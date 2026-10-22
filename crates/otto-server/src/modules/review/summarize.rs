//! The review summarizer: `summarize_and_persist` (managed summarizer with the
//! deterministic fallback).

use super::*;

/// How often a running summarizer re-reads the durable review status when no
/// `ReviewChanged` arrives (the flagless cancel paths: restart, a cancel that
/// landed before this attempt registered).
const SUMMARIZER_CANCEL_RECHECK: Duration = Duration::from_secs(5);

/// The draft loop reads the durable review status once per this many parsed
/// comments; the in-memory cancel flag is checked for every one.
const CANCEL_CHECK_BATCH: usize = 25;

/// Run the summarizer over the reviewers' findings and persist the result:
/// the summarizer live-state row (last element), the draft comments, and the
/// promoted workflow findings (+ resolve_absent + proof pack).
///
/// Guard rails: a summarizer error falls back to the deterministic Rust-side
/// summary, and — CRITICALLY — so does a summarizer reply that parses to ZERO
/// comments while the reviewers produced findings. Without that second guard a
/// confused summarizer answering `[]` silently threw away a whole run's work
/// (45 findings → "done", 0 comments, no error).
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
    // Hold THIS attempt's flag. A retry may replace the registry entry while
    // a cancelled attempt is still unwinding; it must stay cancelled.
    let cancel_flag = ctx.review_cancels.lock().ok().map(|mut map| {
        map.entry(review_id.clone())
            .or_insert_with(|| Arc::new(std::sync::atomic::AtomicBool::new(false)))
            .clone()
    });
    // The in-memory flag is the fast path (the cancel endpoint always trips it
    // first); the durable status only covers flagless paths, so it is read
    // sparingly — see `cancelled` and the draft loop below.
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
    agent_states[summarizer_idx].status = "running".to_string();
    agent_states[summarizer_idx].session_id = None;
    agent_states[summarizer_idx].fallback = false;
    agent_states[summarizer_idx].note = "Starting summarizer".into();
    agent_states[summarizer_idx].provider = if summarizer_cfg.provider.trim().is_empty() {
        "claude".into()
    } else {
        summarizer_cfg.provider.trim().into()
    };
    agent_states[summarizer_idx].model = summarizer_cfg.model.trim().into();
    ctx.reviews_store
        .set_agent_at(review_id, summarizer_idx, &agent_states[summarizer_idx])
        .await?;

    // A finding with an empty body carries nothing reviewable — feeding husks
    // to the summarizer just teaches it to answer `[]`. Drop them up front.
    let agent_findings: Vec<Vec<otto_core::domain::ReviewFinding>> = agent_findings
        .iter()
        .map(|fs| {
            fs.iter()
                .filter(|f| !f.body.trim().is_empty())
                .cloned()
                .collect()
        })
        .collect();
    let agent_findings = agent_findings.as_slice();
    let total_findings: usize = agent_findings.iter().map(|f| f.len()).sum();
    let batches = agent_findings
        .iter()
        .enumerate()
        .map(|(i, f)| {
            format!(
                "Batch {}:\n{}",
                i + 1,
                serde_json::to_string(&label_findings_with_lens(f))
                    .unwrap_or_else(|_| "[]".to_string())
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    // Context first: it frames how to judge every batch below it.
    let summarizer_prompt = format!("{}{}\n\n{}", context, summarizer_cfg.prompt, batches);

    // The summarizer used to get a flat 120s regardless of how much it had to
    // consolidate. That was survivable while it was capped at 20 items; with the
    // cap gone it must dedupe and re-emit EVERY finding, and a 180-file review
    // (12 agents, 36k-line diff) blew straight through it — dropping to the
    // deterministic fallback, whose own cap then produced exactly 20 findings,
    // all `high`. The run looked clean; it was truncated twice over. Scale the
    // budget with the work, capped so a wedged summarizer still fails rather
    // than hanging. The budget also covers the CLI's cold start and the paste,
    // and at ~1s/finding on a 2-minute floor 9 of 10 runs on one day hit the
    // absolute deadline and fell back to the deterministic summary (which only
    // dedupes exact text): ~3s per finding on a 5-minute floor, capped at 30.
    let summarizer_timeout =
        Duration::from_secs((300 + 3 * total_findings as u64).clamp(300, 1_800));
    tracing::info!(
        review = %review_id,
        "running summarizer agent ({total_findings} findings in, {}s budget)",
        summarizer_timeout.as_secs()
    );
    let mut summary_fallback = false;
    // Every provider uses a managed session. A fresh attempt owns its output
    // directory, so retries cannot observe an earlier attempt's completion.
    let run_text = async {
        let mut attempt = crate::review_summarizer::Attempt::new(
            &summarizer_cfg.provider,
            &summarizer_cfg.model,
        )?;
        attempt.meta["review_id"] = serde_json::json!(review_id);
        let prompt = attempt.prompt(&summarizer_prompt);
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
                crate::review_session::stop_review_sessions(&ctx.manager, &[sid]).await;
            },
        )
        .await
    }
    .await;
    // A cancellation is terminal, not a provider failure eligible for fallback.
    if is_cancelled().await {
        return Ok(());
    }
    let summary_text = match run_text {
        Ok(t) => t,
        Err(e) => {
            // Deterministic floor (design 2026-07-04 §C): dedupe + rank the raw
            // per-agent batches Rust-side instead of the old raw concatenation,
            // and badge the run so the UI can say so.
            tracing::warn!(review = %review_id, "summarizer failed: {e}; deterministic fallback");
            summary_fallback = true;
            crate::review_fallback::deterministic_summary(agent_findings)
        }
    };

    // 5. Parse the final JSON robustly.
    let mut parsed = parse_draft_comments(review_id, &summary_text);

    // Guard: reviewers found things but the summary came back empty — that is
    // a summarizer failure, NOT "no issues". Use the deterministic summary so
    // the run's work is never silently discarded.
    if parsed.is_empty() && total_findings > 0 && !summary_fallback {
        tracing::warn!(
            review = %review_id,
            "summarizer returned 0 comments for {total_findings} findings; deterministic fallback"
        );
        summary_fallback = true;
        let fb = crate::review_fallback::deterministic_summary(agent_findings);
        parsed = parse_draft_comments(review_id, &fb);
    }

    let sum_count = parsed.len();
    agent_states[summarizer_idx].status = "done".to_string();
    agent_states[summarizer_idx].comment_count = sum_count as u32;
    agent_states[summarizer_idx].fallback = summary_fallback;
    agent_states[summarizer_idx].note = format!(
        "{}{} final comment{}",
        if summary_fallback {
            "deterministic fallback — "
        } else {
            ""
        },
        sum_count,
        if sum_count == 1 { "" } else { "s" }
    );
    ctx.reviews_store
        .set_agent_at(review_id, summarizer_idx, &agent_states[summarizer_idx])
        .await?;

    // 6. Persist draft comments AND promote each into a tracked workflow Finding
    //    (the board + Proof Pack read these). The summarized comment is the
    //    PR-posting artifact; the Finding is the canonical workflow record. We
    //    also track seen fingerprints to drive the cross-run DETECTION lifecycle
    //    (absent findings flip `state`→resolved via resolve_absent below — the
    //    engine owns the `state` axis; the workflow `status` is untouched).
    tracing::info!(review = %review_id, "storing {} draft comments", parsed.len());
    // PR #0 is the local-review sentinel → dedup within the review, not across PRs.
    let pr_opt = if pr_number == 0 {
        None
    } else {
        Some(pr_number)
    };
    let mut seen_fingerprints: Vec<String> = Vec::new();
    // Comments the user already decided on (or that are on the PR) — a re-run
    // must not re-draft them. Empty on a first run.
    let kept_comments: Vec<ReviewComment> = ctx
        .reviews_store
        .get_review(review_id)
        .await
        .map(|r| {
            r.comments
                .into_iter()
                .filter(|k| k.state != CommentState::Draft || k.posted)
                .collect()
        })
        .unwrap_or_default();
    // File contents read for fingerprint anchoring, one read per path.
    let mut anchor_files: std::collections::HashMap<String, Option<Vec<String>>> =
        std::collections::HashMap::new();
    for (n, c) in parsed.into_iter().enumerate() {
        // The flag per comment, the durable status once per batch: the loop
        // inserts comments, so a full review load per comment was O(N²).
        let cancelled = if n % CANCEL_CHECK_BATCH == 0 {
            is_cancelled().await
        } else {
            flag_tripped()
        };
        if cancelled {
            return Ok(());
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
            (Some(p), Some(l)) => anchored_line_text(repo_path, p, l, &mut anchor_files).await,
            _ => None,
        };
        let anchor =
            otto_state::review_findings::finding_anchor(line_text.as_deref(), &title, &c.body);
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
        // Track for the cross-run detection lifecycle (resolve_absent below).
        seen_fingerprints.push(fp);
    }

    // Findings present in a prior run but absent now flip open→resolved (the
    // "verification" leg: a re-run that no longer surfaces a finding resolves it).
    //
    // Only a run that really re-evaluated the code may resolve anything:
    // - a PARTIAL run (a reviewer errored / was cut off, or the deterministic
    //   fallback stood in for the summarizer) resolves nothing — a missing lens
    //   is not evidence its findings were fixed;
    // - a LOCAL run (the shared `pr_number = 0` sentinel) resolves only findings
    //   in files its diff touched — one branch's review used to resolve every
    //   open local finding in the repo (3128/3804 rows);
    // - a PR run keeps the whole-PR scope (every run reviews the same change).
    if is_cancelled().await {
        return Ok(());
    }
    let seen_refs: Vec<&str> = seen_fingerprints.iter().map(|s| s.as_str()).collect();
    let complete = review_run_complete(&agent_states[..summarizer_idx], summary_fallback);
    let local_scope: Option<Vec<String>> = if pr_number == 0 {
        match ctx.reviews_store.get_diff(review_id).await {
            Ok(Some(diff)) => Some(diff_file_paths(&diff)),
            _ => None,
        }
    } else {
        None
    };
    if let Some(why) = resolve_blocked {
        tracing::info!(review = %review_id, "{why} — not resolving absent findings");
    } else if !complete {
        tracing::info!(review = %review_id, "partial review run — not resolving absent findings");
    } else if pr_number == 0 && local_scope.is_none() {
        tracing::info!(review = %review_id, "local review without a stored diff — not resolving absent findings");
    } else {
        let scope_refs: Option<Vec<&str>> = local_scope
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_str()).collect());
        // Local runs share `pr_number = 0` across branches: only this run's
        // source branch's findings are its to resolve (S2-05).
        let branch = if pr_number == 0 {
            ctx.reviews_store
                .get_run_context(review_id)
                .await
                .ok()
                .flatten()
                .and_then(|c| c.source_branch)
        } else {
            None
        };
        if pr_number == 0 && branch.is_none() {
            // No recorded branch (detached HEAD, or a pre-context review):
            // which branch's findings these are is unknown — resolve nothing.
            tracing::info!(review = %review_id, "local review without a source branch — not resolving absent findings");
        } else if let Err(e) = ctx
            .findings_store
            .resolve_absent_scoped(
                &workspace.id,
                repo_id,
                pr_number,
                &seen_refs,
                review_id,
                scope_refs.as_deref(),
                branch.as_deref(),
            )
            .await
        {
            tracing::warn!(review = %review_id, "resolve_absent failed: {e}");
        }
    }

    // 7. Assemble the proof pack for this review from the now-persisted findings.
    assemble_review_proof(ctx, review_id, &workspace.id).await;

    Ok(())
}
