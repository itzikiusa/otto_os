//! The review summarizer: `summarize_and_persist`, a thin adapter over the
//! shared summarizer step (`otto_agent_run::fanout::summarize`) — it builds
//! the managed summarizer turn ([`SummarizerTurn`]) and the deterministic
//! fallback, then persists the result: the summarizer row, the draft comments
//! and promoted findings ([`persist_comments`]), and the prior findings this
//! run confirmed fixed (`otto_review::resolution` — never by absence).

use super::*;
use otto_review::prompts::*;

/// How often a running summarizer re-reads the durable review status when no
/// `ReviewChanged` arrives (the flagless cancel paths: restart, a cancel that
/// landed before this attempt registered).
const SUMMARIZER_CANCEL_RECHECK: Duration = Duration::from_secs(5);

/// The draft loop reads the durable review status once per this many parsed
/// comments; the in-memory cancel flag is checked for every one.
const CANCEL_CHECK_BATCH: usize = 25;

/// Independently verify every raw claim before promoting comments. Incomplete
/// evidence or provider failure keeps the review retryable, never clean.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn summarize_and_persist(
    ctx: &ServerCtx,
    review_id: &Id,
    repo_path: &str,
    workspace: &Workspace,
    user: &otto_core::domain::User,
    repo_id: &Id,
    pr_number: u64,
    summarizer_cfg: &ReviewAgentCfg,
    agent_findings: &[Vec<otto_core::domain::ReviewFinding>],
    // What the caller already knew about the change (ticket + prior-step
    // briefs), pre-rendered. The summarizer is the step that applies the scope
    // gate and decides what survives, so it needs this MORE than the reviewers
    // do: without it, "documented, intentional, already-ticketed behavior" is
    // indistinguishable from a defect this change introduced, and it keeps it.
    context: &str,
    // `Some(reason)` ⇒ this run cannot vouch for what it did NOT see (partial
    // diff, wrong checkout, a retry anchoring outside the reviewed tree), so
    // absent findings are left alone. See [`resolve_blocker`].
    resolve_blocked: Option<&str>,
) -> Result<()> {
    // Reclaim the live states (updated by the reviewer tasks) and mark the
    // summarizer (always the LAST row) running.
    if review_is_cancelled(ctx, review_id).await {
        return Ok(());
    }
    let cancel_flag = attempt_cancel_flag(ctx, review_id);
    let flag_tripped = || {
        cancel_flag
            .as_ref()
            .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
    };
    let is_cancelled = || async { flag_tripped() || review_is_cancelled(ctx, review_id).await };
    let mut agent_states = ctx.reviews_store.get_review(review_id).await?.agents;
    let summarizer_idx = agent_states
        .len()
        .checked_sub(1)
        .ok_or_else(|| Error::Invalid("review has no summarizer row".into()))?;
    mark_summarizer_starting(&mut agent_states[summarizer_idx], summarizer_cfg);
    ctx.reviews_store
        .set_agent_at(review_id, summarizer_idx, &agent_states[summarizer_idx])
        .await?;

    // A finding with an empty body carries nothing reviewable — feeding husks
    // to the summarizer just teaches it to answer `[]`. Drop them up front.
    let mut agent_findings: Vec<Vec<otto_core::domain::ReviewFinding>> = agent_findings
        .iter()
        .map(|fs| {
            fs.iter()
                .filter(|f| !f.body.trim().is_empty())
                .cloned()
                .collect()
        })
        .collect();
    // Prior findings go in LAST, one claim each (`resolution::confirmed_fixed`).
    let (plan, prior_batch) = (
        ctx.reviews_store.get_review(review_id).await?.scope_plan,
        agent_findings.len(),
    );
    if let Some(plan) = &plan {
        let prior = otto_review::history::prior_claims(plan, &ctx.findings_store).await?;
        if !prior.is_empty() {
            agent_findings.push(prior);
        }
    }
    let agent_findings = agent_findings.as_slice();
    let total_findings: usize = agent_findings.iter().map(|f| f.len()).sum();
    let context = context.to_string() + &lens_scores_block(&agent_states[..summarizer_idx]);
    let summarizer_prompt = summarizer_prompt(&context, summarizer_cfg, agent_findings);

    let summarizer_timeout = summarizer_budget(total_findings);
    tracing::info!(
        review = %review_id,
        "running summarizer agent ({total_findings} findings in, {}s budget)",
        summarizer_timeout.as_secs()
    );
    // The shared summarizer step (`otto_agent_run::fanout::summarize`): the
    // managed turn, else the deterministic floor (design 2026-07-04 §C: dedupe
    // + rank the raw per-agent batches Rust-side, and badge the run so the UI
    // can say so). Reviewer slots are the agent rows before the summarizer's.
    let batches_in: Vec<(usize, &[_])> = agent_findings
        .iter()
        .map(Vec::as_slice)
        .enumerate()
        .collect();
    let turn = SummarizerTurn {
        ctx,
        review_id,
        repo_path,
        workspace,
        user,
        cfg: summarizer_cfg,
        prompt: &summarizer_prompt,
        timeout: summarizer_timeout,
        total_findings,
        cancel_flag: cancel_flag.as_ref(),
        row: summarizer_idx,
    };
    // A start/delivery failure gets a fresh attempt (fresh handoff dir) within
    // the same overall budget; the verifier used to get exactly one.
    let (deadline, states) = (
        std::time::Instant::now() + summarizer_timeout,
        &mut agent_states,
    );
    let summary = otto_agent_run::fanout::summarize(
        &batches_in,
        |_| async move {
            let mut result = turn.run(&mut *states).await;
            for _ in 1..otto_review::verification::SUMMARIZER_ATTEMPTS {
                match &result {
                    Err(e) if otto_review::verification::summarizer_retryable(e) && !flag_tripped() => {
                        tracing::warn!(review = %review_id, "summarizer did not start ({e}); retrying");
                        tokio::time::sleep(otto_review::verification::SUMMARIZER_RETRY_BACKOFF).await;
                        let timeout = deadline.saturating_duration_since(std::time::Instant::now());
                        mark_summarizer_starting(&mut states[summarizer_idx], summarizer_cfg);
                        result = SummarizerTurn { timeout, ..turn }.run(&mut *states).await;
                    }
                    _ => break,
                }
            }
            result
        },
        |_| deterministic_comments(review_id, agent_findings),
        |_, _| std::future::ready(()),
    )
    .await;
    // A cancellation is terminal, not a provider failure eligible for fallback.
    if is_cancelled().await {
        return Ok(());
    }
    let fallback_cause = summary.error;
    if let Some(e) = &fallback_cause {
        tracing::warn!(review = %review_id, "summarizer failed: {e}; reviewer findings kept unverified");
    }
    // Verified comments plus every claim the verifier did not account for,
    // stored marked unverified — never dropped (see `otto_review::verification`).
    let (cause, blocked) = (fallback_cause.as_deref(), resolve_blocked);
    let fixed = otto_review::resolution::confirmed_fixed(
        plan.as_ref(),
        prior_batch,
        &summary.value,
        cause,
        blocked,
    );
    let settled = otto_review::verification::settle(summary.value, agent_findings, cause);
    if let Some(mut plan) = ctx.reviews_store.get_review(review_id).await?.scope_plan {
        plan.verification_notes = settled.notes.clone();
        ctx.reviews_store.set_scope_plan(review_id, &plan).await?;
    }
    let incomplete = otto_review::verification::incomplete_reason(
        &settled,
        fallback_cause.as_deref(),
        resolve_blocked,
        &otto_review::restart::with_failure_reasons(&agent_states[..summarizer_idx]),
    );
    let parsed = settled.comments;
    mark_summarizer_done(
        &mut agent_states[summarizer_idx],
        fallback_cause.as_deref(),
        parsed.len(),
    );
    ctx.reviews_store
        .set_agent_at(review_id, summarizer_idx, &agent_states[summarizer_idx])
        .await?;

    let Some(_seen) = persist_comments(
        ctx,
        review_id,
        repo_path,
        workspace,
        repo_id,
        pr_number,
        parsed,
        &flag_tripped,
    )
    .await?
    else {
        return Ok(());
    };
    if is_cancelled().await {
        return Ok(());
    }
    otto_review::resolution::resolve_confirmed(&ctx.findings_store, review_id, &fixed).await;
    // Not a verdict (status `error`, the findings stay stored): the message
    // names the failed step for the review panel and the workflow step.
    incomplete.map_or(Ok(()), |why| Err(Error::Invalid(why)))
}

/// THIS attempt's cancel flag (registered on first use). Held for the whole
/// attempt: a retry may replace the registry entry while a cancelled attempt
/// is still unwinding; it must stay cancelled. The in-memory flag is the fast
/// path (the cancel endpoint always trips it first); the durable status only
/// covers flagless paths, so it is read sparingly (the turn's cancel watch
/// and the draft loop).
fn attempt_cancel_flag(
    ctx: &ServerCtx,
    review_id: &Id,
) -> Option<Arc<std::sync::atomic::AtomicBool>> {
    ctx.review_cancels.lock().ok().map(|mut map| {
        map.entry(review_id.clone())
            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicBool::new(false)))
            .clone()
    })
}

/// Persist the summary's draft comments AND promote each into a tracked
/// workflow Finding. Returns the fingerprints seen, or `None` when the review
/// was cancelled mid-way.
#[allow(clippy::too_many_arguments)]
async fn persist_comments(
    ctx: &ServerCtx,
    review_id: &Id,
    repo_path: &str,
    workspace: &Workspace,
    repo_id: &Id,
    pr_number: u64,
    parsed: Vec<otto_review::engine::DraftComment>,
    flag_tripped: &(dyn Fn() -> bool + Sync),
) -> Result<Option<Vec<String>>> {
    // 6. Persist draft comments AND promote each into a tracked workflow Finding
    //    (the board + Proof Pack read these). The summarized comment is the
    //    PR-posting artifact; the Finding is the canonical workflow record. A
    //    finding absent from this run is NOT resolved (`otto_review::resolution`
    //    says why); seen fingerprints only disambiguate same-line findings.
    tracing::info!(review = %review_id, "storing {} draft comments", parsed.len());
    let mut seen_fingerprints: Vec<String> = Vec::new();
    // Comments the user already decided on (or that are on the PR) — a re-run
    // must not re-draft them. Empty on a first run.
    // A failed read fails the run (retryable), never "nothing kept": that
    // re-drafted every approved / posted comment (error-handling-5-10).
    let kept_comments: Vec<ReviewComment> = (ctx.reviews_store.get_review(review_id).await?)
        .comments
        .into_iter()
        .filter(|k| k.state != CommentState::Draft || k.posted)
        .collect();
    // File contents read for fingerprint anchoring, one read per path.
    let mut anchor_files: std::collections::HashMap<String, Option<Vec<String>>> =
        std::collections::HashMap::new();
    for (n, c) in parsed.into_iter().enumerate() {
        // The flag per comment, the durable status once per batch: the loop
        // inserts comments, so a full review load per comment was O(N²).
        let cancelled = if n % CANCEL_CHECK_BATCH == 0 {
            flag_tripped() || review_is_cancelled(ctx, review_id).await
        } else {
            flag_tripped()
        };
        if cancelled {
            return Ok(None);
        }
        let sev = CommentSeverity::normalize(&c.severity);
        // A summarizer RE-RUN (retry_summarizer) replaces only the drafts; a
        // comment the user already approved/declined (or that is on the PR)
        // must not come back as a fresh draft — approving that copy posted a
        // duplicate. Reuse the decided comment instead.
        let comment_id = match kept_comments.iter().find(|k| same_draft_comment(k, &c)) {
            Some(k) => k.id.clone(),
            None => {
                ctx.reviews_store
                    .add_comment(review_id, c.path.as_deref(), c.line, sev, &c.body)
                    .await?
                    .id
            }
        };

        let fp = promote_finding(
            ctx,
            review_id,
            repo_path,
            workspace,
            repo_id,
            pr_number,
            &c,
            &comment_id,
            &seen_fingerprints,
            &mut anchor_files,
        )
        .await;
        seen_fingerprints.push(fp);
    }
    Ok(Some(seen_fingerprints))
}

/// Promote one draft comment into a tracked workflow Finding (created →
/// audit event linking the comment). Returns its fingerprint.
#[allow(clippy::too_many_arguments)]
async fn promote_finding(
    ctx: &ServerCtx,
    review_id: &Id,
    repo_path: &str,
    workspace: &Workspace,
    repo_id: &Id,
    pr_number: u64,
    c: &otto_review::engine::DraftComment,
    comment_id: &Id,
    seen_fingerprints: &[String],
    anchor_files: &mut std::collections::HashMap<String, Option<Vec<String>>>,
) -> String {
    // PR #0 is the local-review sentinel → dedup within the review, not across PRs.
    let pr_opt = if pr_number == 0 {
        None
    } else {
        Some(pr_number)
    };
    // Derive the enriched fields from the comment when the summarizer didn't
    // emit them (back-compat; §15.9).
    let title = c
        .title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| c.body.lines().next().unwrap_or("").trim().to_string());
    let evidence = c
        .evidence
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| c.body.clone());
    let reasoning = c.reasoning.clone().unwrap_or_default();
    // Stable (v2) fingerprint anchored on the flagged code line's text (or
    // the title), NOT the summarizer's re-worded body — see
    // `compute_finding_fingerprint`. The legacy body hash rides along so
    // rows stored before v2 are recognized and re-keyed, not duplicated.
    let legacy_fp = otto_state::review_findings::compute_fingerprint(
        repo_id,
        pr_number,
        c.path.as_deref(),
        c.category.as_deref(),
        &c.body,
    );
    let line_text = match (c.path.as_deref(), c.line) {
        (Some(p), Some(l)) => anchored_line_text(repo_path, p, l, anchor_files).await,
        _ => None,
    };
    let anchor = otto_state::review_findings::finding_anchor(line_text.as_deref(), &title, &c.body);
    let mut fp = otto_state::review_findings::compute_finding_fingerprint(
        repo_id,
        pr_number,
        c.path.as_deref(),
        c.category.as_deref(),
        &anchor,
    );
    if seen_fingerprints.contains(&fp) {
        // A second, distinct finding on the same line/category in this run:
        // disambiguate by title so it is not folded into the first one.
        let titled = format!(
            "{anchor}|{}",
            otto_state::review_findings::normalize_anchor_text(&title)
        );
        fp = otto_state::review_findings::compute_finding_fingerprint(
            repo_id,
            pr_number,
            c.path.as_deref(),
            c.category.as_deref(),
            &titled,
        );
    }
    // Anchor the finding to a sane (line, line_end) span — an end without a
    // start, or an inverted/degenerate range, collapses appropriately.
    let (line, line_end) = otto_core::finding::normalize_line_range(c.line, c.line_end);
    let nf = otto_state::NewFinding {
        review_id,
        workspace_id: &workspace.id,
        repo_id,
        pr_number: pr_opt,
        path: c.path.as_deref(),
        line: line.map(|l| l as i64),
        line_end: line_end.map(|l| l as i64),
        severity: &c.severity,
        category: c.category.as_deref(),
        title: &title,
        body: &c.body,
        evidence: &evidence,
        agent_reasoning_summary: &reasoning,
        suggested_fix: c.suggested_fix.as_deref(),
        produced_by_agent: Some("review"),
        reviewer: "review",
        fingerprint: &fp,
        run_id: review_id,
    };
    match ctx
        .findings_store
        .upsert_tracked(&nf, Some(&legacy_fp))
        .await
    {
        Ok((f, created)) => {
            if created {
                // Anchor the audit trail + link the originating comment id.
                let _ = ctx
                    .finding_events_store
                    .append(
                        &f.id,
                        &f.workspace_id,
                        "created",
                        "review",
                        None,
                        Some("open"),
                        serde_json::json!({ "comment_id": comment_id }),
                    )
                    .await;
            }
        }
        Err(e) => tracing::warn!(review = %review_id, "persist finding failed: {e}"),
    }
    fp
}

/// One managed summarizer turn — the `summarizer` step [`summarize_and_persist`]
/// hands to `otto_agent_run::fanout::summarize`. It fails (so the runner
/// takes the deterministic fallback) on a provider error AND on a reply that
/// parses to ZERO comments while the reviewers produced findings.
struct SummarizerTurn<'a> {
    ctx: &'a ServerCtx,
    review_id: &'a Id,
    repo_path: &'a str,
    workspace: &'a Workspace,
    user: &'a otto_core::domain::User,
    cfg: &'a ReviewAgentCfg,
    prompt: &'a str,
    timeout: Duration,
    total_findings: usize,
    cancel_flag: Option<&'a Arc<std::sync::atomic::AtomicBool>>,
    /// The summarizer's agent row (always the last).
    row: usize,
}

impl SummarizerTurn<'_> {
    async fn run(
        &self,
        agent_states: &mut [otto_core::domain::ReviewAgentState],
    ) -> std::result::Result<Vec<otto_review::engine::DraftComment>, String> {
        let text = self
            .drive(agent_states)
            .await
            .map_err(|e| otto_core::text::clip_chars(e.trim(), 200))?;
        let parsed = parse_draft_comments(self.review_id, &text);
        apply_lens_score(&mut agent_states[self.row], &text);
        if parsed.is_empty() && self.total_findings > 0 {
            return Err(format!(
                "returned no comments for {} findings",
                self.total_findings
            ));
        }
        Ok(parsed)
    }

    /// Every provider uses a managed session. A fresh attempt owns its output
    /// directory, so retries cannot observe an earlier attempt's completion.
    async fn drive(
        &self,
        agent_states: &mut [otto_core::domain::ReviewAgentState],
    ) -> std::result::Result<String, String> {
        let (ctx, review_id, workspace, user, repo_path) = (
            self.ctx,
            self.review_id,
            self.workspace,
            self.user,
            self.repo_path,
        );
        let (summarizer_cfg, summarizer_prompt, summarizer_timeout, summarizer_idx) =
            (self.cfg, self.prompt, self.timeout, self.row);
        let flag_tripped = || {
            self.cancel_flag
                .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
        };
        let mut attempt = crate::review_summarizer::Attempt::new(
            &summarizer_cfg.provider,
            &summarizer_cfg.model,
        )?;
        attempt.meta["review_id"] = serde_json::json!(review_id);
        // Queue for a machine slot BEFORE the summarizer's hard budget starts.
        let admission = ctx.manager.admit_background("review_summarizer").await;
        admission.stamp(&mut attempt.meta);
        let prompt = attempt.prompt(summarizer_prompt);
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let turn = crate::agent_session::run_session_turn_with(
            ctx,
            workspace,
            user,
            None,
            "Review summarizer",
            repo_path,
            &attempt.provider,
            attempt.meta.clone(),
            &prompt,
            summarizer_timeout,
            crate::agent_session::TurnOpts {
                done_file: Some(attempt.result_path()),
                done_file_validator: Some(crate::review_summarizer::valid_result),
                ..Default::default()
            },
            |id| {
                let _ = ready_tx.send(id.clone());
            },
        );
        // Event- and flag-driven: the cheap in-memory flag every 100 ms, the
        // one-column status on this review's `ReviewChanged`, and a slow safety
        // recheck. A 100 ms DB poll used to load the whole review (agents JSON
        // plus every comment) 3,000–18,000 times per summarizer turn.
        let cancelled = async {
            let mut events = ctx.events.subscribe();
            let mut events_open = true;
            let mut flag_tick = tokio::time::interval(Duration::from_millis(100));
            let mut recheck = tokio::time::interval(SUMMARIZER_CANCEL_RECHECK);
            loop {
                let check_db = tokio::select! {
                    _ = flag_tick.tick() => false,
                    _ = recheck.tick() => true,
                    ev = events.recv(), if events_open => match ev {
                        Ok(Event::ReviewChanged { review_id: rid, .. }) => rid == *review_id,
                        Ok(_) => false,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            events_open = false;
                            false
                        }
                    },
                };
                if flag_tripped() || (check_db && review_is_cancelled(ctx, review_id).await) {
                    break;
                }
            }
        };
        crate::review_summarizer::drive(
            async {
                turn.await
                    .map(|(text, _)| text)
                    .map_err(|e| e.0.to_string())
            },
            ready_rx,
            summarizer_timeout,
            cancelled,
            |sid| async {
                agent_states[summarizer_idx].session_id = Some(sid.clone());
                ctx.reviews_store
                    .set_agent_at(review_id, summarizer_idx, &agent_states[summarizer_idx])
                    .await
                    .map_err(|e| e.to_string())?;
                let _ = ctx.events.send(Event::ReviewChanged {
                    workspace_id: workspace.id.clone(),
                    session_id: Some(sid),
                    review_id: review_id.clone(),
                    status: "running".into(),
                });
                Ok(())
            },
            |sid| async move {
                otto_review::session::stop_review_sessions(&ctx.manager, &[sid]).await;
            },
        )
        .await
    }
}

#[cfg(test)]
mod note_tests {
    /// error-handling-5-05: the cause reaches the stored note.
    #[test]
    fn fallback_note_carries_the_cause() {
        assert_eq!(super::fallback_note(None, 1), "1 final comment");
        let n = super::fallback_note(Some("returned no comments for 4 findings"), 4);
        assert!(n.starts_with("deterministic fallback (summarizer: returned no comments"));
        assert!(n.ends_with("— 4 final comments"), "{n}");
    }
}
