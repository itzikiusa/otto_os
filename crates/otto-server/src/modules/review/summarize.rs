//! The review summarizer: `summarize_and_persist`, a thin adapter over the
//! shared summarizer step (`otto_agent_run::fanout::summarize`) — it builds
//! the managed summarizer turn ([`SummarizerTurn`]) and the deterministic
//! fallback, then persists the result: the summarizer row, the draft comments
//! and promoted findings ([`persist_comments`]), and the prior findings this
//! run confirmed fixed (`otto_review::resolution` — never by absence).

use super::*;
use otto_review::prompts::*;
use otto_review::summarizer::DEADLINE_EXCEEDED;
use tokio::time::Instant;

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
    cfg: &ReviewConfig,
    review_deadline: Option<std::time::Instant>,
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
    let cancel_guard = ctx.review_cancels.join(review_id);
    let cancel_flag = Some(Arc::clone(cancel_guard.flag()));
    let flag_tripped = || {
        cancel_flag
            .as_ref()
            .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
    };
    let is_cancelled = || async { flag_tripped() || review_is_cancelled(ctx, review_id).await };
    let (summarizer_cfg, threads) = (&cfg.summarizer, review_threads(ctx, Some(cfg)).await);
    let mut agent_states = ctx.reviews_store.get_review(review_id).await?.agents;
    let summarizer_idx = agent_states
        .len()
        .checked_sub(1)
        .ok_or_else(|| Error::Invalid("review has no summarizer row".into()))?;
    let held_summarizer =
        held_summarizer_thread(ctx, threads.is_some(), &agent_states[summarizer_idx]).await;
    if held_summarizer.is_none() {
        mark_summarizer_starting(&mut agent_states[summarizer_idx], summarizer_cfg);
        ctx.reviews_store
            .set_agent_at(review_id, summarizer_idx, &agent_states[summarizer_idx])
            .await?;
    }

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

    let now = Instant::now();
    let remaining = review_deadline.map(|at| Instant::from_std(at).saturating_duration_since(now));
    let budget = summarizer_budget(total_findings, remaining);
    tracing::info!(
        review = %review_id,
        "running summarizer agent ({total_findings} input findings, {}s shared budget)",
        budget.as_secs()
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
        deadline: now + budget,
        total_findings,
        cancel_flag: cancel_flag.as_ref(),
        row: summarizer_idx,
        threads: threads.as_ref(),
    };
    // A start/delivery failure gets a fresh attempt (fresh handoff dir) within
    // the same overall budget; the verifier used to get exactly one.
    let states = &mut agent_states;
    let summary = otto_agent_run::fanout::summarize(
        &batches_in,
        |_| async move {
            let mut result = if let Some((sid, path)) = held_summarizer {
                match turn.resume_held(&mut *states, &sid, &path).await {
                    Ok(comments) => return Ok(comments),
                    Err(e) if flag_tripped() || Instant::now() >= turn.deadline => return Err(e),
                    Err(e) => {
                        tracing::warn!(review = %review_id, "held summarizer failed ({e}); starting fresh");
                        mark_summarizer_starting(&mut states[summarizer_idx], summarizer_cfg);
                        let _ = ctx
                            .reviews_store
                            .set_agent_at(review_id, summarizer_idx, &states[summarizer_idx])
                            .await;
                        turn.run(&mut *states).await
                    }
                }
            } else {
                turn.run(&mut *states).await
            };
            for _ in 1..otto_review::verification::SUMMARIZER_ATTEMPTS {
                match &result {
                    Err(e) if otto_review::verification::summarizer_retryable(e) && !flag_tripped() => {
                        let retry_at = Instant::now() + otto_review::verification::SUMMARIZER_RETRY_BACKOFF;
                        if retry_at >= turn.deadline {
                            break;
                        }
                        tracing::warn!(review = %review_id, "summarizer did not start ({e}); retrying");
                        tokio::time::sleep_until(retry_at).await;
                        mark_summarizer_starting(&mut states[summarizer_idx], summarizer_cfg);
                        result = turn.run(&mut *states).await;
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
        otto_review::verification::set_notes(&mut plan, &settled.notes);
        ctx.reviews_store.set_scope_plan(review_id, &plan).await?;
    }
    let incomplete = otto_review::verification::incomplete_reason(
        &settled,
        fallback_cause.as_deref(),
        resolve_blocked,
        &otto_review::restart::with_failure_reasons(&agent_states[..summarizer_idx]),
    );
    let (parsed, verified) = (settled.comments, cause.is_none() && !settled.incomplete);
    mark_summarizer_done(
        &mut agent_states[summarizer_idx],
        fallback_cause.as_deref(),
        parsed.len(),
    );
    ctx.reviews_store
        .set_agent_at(review_id, summarizer_idx, &agent_states[summarizer_idx])
        .await?;

    let Some(seen) = persist_comments(
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
    let stores = (
        &ctx.reviews_store,
        &ctx.findings_store,
        &ctx.finding_events_store,
    );
    otto_review::resolution::close_superseded_unverified(stores, review_id, &seen, verified).await;
    // Not a verdict (status `error`, the findings stay stored): the message
    // names the failed step for the review panel and the workflow step.
    incomplete.map_or(Ok(()), |why| Err(Error::Invalid(why)))
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
    //    says why); seen fingerprints disambiguate same-line findings and tell
    //    a re-run which earlier unverified findings it superseded.
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

        let stores = (&ctx.findings_store, &ctx.finding_events_store);
        let at = (
            review_id,
            workspace.id.as_str(),
            repo_id,
            pr_number,
            repo_path,
        );
        let fp = otto_review::promote::promote_finding(
            stores,
            at,
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

async fn held_summarizer_thread(
    ctx: &ServerCtx,
    has_threads: bool,
    row: &otto_core::domain::ReviewAgentState,
) -> Option<(Id, std::path::PathBuf)> {
    if !has_threads || !row.thread || row.status != "running" {
        return None;
    }
    let tid: Id = row.session_id.as_deref()?.into();
    let s = ctx.agent_runtime.get(&tid).await.ok()?;
    if s.status == otto_core::agent_runtime::RuntimeSessionStatus::Closed {
        return None;
    }
    let p = s.meta.as_ref()?.get("result_path")?.as_str()?;
    (!p.is_empty()).then(|| (tid, std::path::PathBuf::from(p)))
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
    deadline: Instant,
    total_findings: usize,
    cancel_flag: Option<&'a Arc<std::sync::atomic::AtomicBool>>,
    /// The summarizer's agent row (always the last).
    row: usize,
    threads: Option<&'a otto_review::thread_run::Threads>,
}

impl SummarizerTurn<'_> {
    fn parse_output(
        &self,
        agent_states: &mut [otto_core::domain::ReviewAgentState],
        text: &str,
    ) -> std::result::Result<Vec<otto_review::engine::DraftComment>, String> {
        let parsed = parse_draft_comments(self.review_id, text);
        apply_lens_score(&mut agent_states[self.row], text);
        if parsed.is_empty() && self.total_findings > 0 {
            return Err(format!(
                "returned no comments for {} findings",
                self.total_findings
            ));
        }
        Ok(parsed)
    }

    async fn run(
        &self,
        agent_states: &mut [otto_core::domain::ReviewAgentState],
    ) -> std::result::Result<Vec<otto_review::engine::DraftComment>, String> {
        let text = self
            .drive(agent_states)
            .await
            .map_err(|e| otto_core::text::clip_chars(e.trim(), 200))?;
        self.parse_output(agent_states, &text)
    }

    /// Await a summarizer thread that survived a daemon restart in its holder.
    async fn resume_held(
        &self,
        agent_states: &mut [otto_core::domain::ReviewAgentState],
        thread_id: &Id,
        result_path: &std::path::Path,
    ) -> std::result::Result<Vec<otto_review::engine::DraftComment>, String> {
        let Some(threads) = self.threads else {
            return Err("thread runner unavailable".into());
        };
        if Instant::now() >= self.deadline {
            return Err(DEADLINE_EXCEEDED.into());
        }
        let flag_tripped = || {
            self.cancel_flag
                .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
        };
        let cancelled = otto_review::summarizer::until_cancelled(
            self.ctx.events.subscribe(),
            self.review_id,
            flag_tripped,
            || review_is_cancelled(self.ctx, self.review_id),
        );
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        let _ = ready_tx.send(thread_id.to_string());
        let brief = (
            self.prompt,
            self.cfg,
            self.workspace,
            self.user,
            self.repo_path,
        );
        let turn = threads.resume_summarizer_turn(thread_id, result_path, brief, self.cancel_flag);
        let stop = |sid: String| async move { threads.close(&sid).await };
        let res = crate::review_summarizer::drive(
            turn,
            ready_rx,
            self.deadline,
            cancelled,
            |_| async { Ok(()) },
            stop,
        )
        .await;
        if let Some(dir) = result_path.parent().filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("otto-summary-"))
        }) {
            let _ = tokio::fs::remove_dir_all(dir).await;
        }
        let text = res.map_err(|e| otto_core::text::clip_chars(e.trim(), 200))?;
        self.parse_output(agent_states, &text)
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
        let (summarizer_cfg, summarizer_prompt, deadline, summarizer_idx) =
            (self.cfg, self.prompt, self.deadline, self.row);
        if Instant::now() >= deadline {
            return Err(DEADLINE_EXCEEDED.into());
        }
        let flag_tripped = || {
            self.cancel_flag
                .is_some_and(|f| f.load(std::sync::atomic::Ordering::SeqCst))
        };
        let mut attempt = crate::review_summarizer::Attempt::new(
            &summarizer_cfg.provider,
            &summarizer_cfg.model,
        )?;
        attempt.meta["review_id"] = serde_json::json!(review_id);
        // Event- and flag-driven (no 100 ms DB poll of the whole review).
        let cancelled = otto_review::summarizer::until_cancelled(
            ctx.events.subscribe(),
            review_id,
            flag_tripped,
            || review_is_cancelled(ctx, review_id),
        );
        let publish = move |sid: String| async move {
            let row = &mut agent_states[summarizer_idx];
            (row.session_id, row.thread) = (Some(sid.clone()), self.threads.is_some());
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
        };
        let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
        if let Some(threads) = self.threads {
            // Keep the handoff directory across a daemon restart while the held
            // summarizer CLI is still running; remove it once drive finishes here.
            attempt.persist_for_restart();
            attempt.meta["result_path"] =
                serde_json::json!(attempt.result_path().to_string_lossy());
            // The runner takes the machine place itself.
            let brief = (
                summarizer_prompt,
                summarizer_cfg,
                workspace,
                user,
                repo_path,
            );
            let turn = threads.summarizer_turn(&attempt, brief, ready_tx, self.cancel_flag);
            let stop = |sid: String| async move { threads.close(&sid).await };
            let res =
                crate::review_summarizer::drive(turn, ready_rx, deadline, cancelled, publish, stop)
                    .await;
            attempt.cleanup();
            return res;
        }
        // Admission shares the verifier's deadline, just as on threads.
        tokio::pin!(cancelled);
        let admission = tokio::select! {
            biased;
            _ = &mut cancelled => return Err("review cancelled".into()),
            _ = tokio::time::sleep_until(deadline) => return Err(DEADLINE_EXCEEDED.into()),
            a = ctx.manager.admit_background("review_summarizer") => a,
        };
        admission.stamp(&mut attempt.meta);
        let prompt = attempt.prompt(summarizer_prompt);
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
            deadline.saturating_duration_since(Instant::now()),
            crate::agent_session::TurnOpts {
                cancel: self.cancel_flag.cloned(),
                done_file: Some(attempt.result_path()),
                done_file_validator: Some(crate::review_summarizer::valid_result),
                ..Default::default()
            },
            |id| {
                let _ = ready_tx.send(id.clone());
            },
        );
        crate::review_summarizer::drive(
            async {
                turn.await
                    .map(|(text, _)| text)
                    .map_err(|e| e.0.to_string())
            },
            ready_rx,
            deadline,
            cancelled,
            publish,
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
