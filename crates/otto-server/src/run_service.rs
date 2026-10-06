//! Run with Otto — the launch funnel + lifecycle actions.
//!
//! Every surface (Slack/Telegram trigger, webhook, REST, UI) calls
//! [`launch`]; that is the "one button". [`approve`]/[`cancel`]/[`open_pr`] are the
//! lifecycle actions the approval gate and the UI drive. All of these are pure
//! orchestration over `RunsRepo` + the engine — no transport concerns.

use chrono::Utc;

use otto_core::api::{CreatePrReq, PrSummary};
use otto_core::run::{
    open_pr_block_reason, parse_decision, parse_source_ref, ApprovalDecision, ApproveRunReq,
    LaunchRunReq, OpenPrBlock, OttoRun, RunOrigin, RunStatus, SourceKind,
};
use otto_core::{Error, Id, Result};
use otto_state::runs::{NewRun, NewRunEvent, RunPatch};

use crate::run_engine;
use crate::state::ServerCtx;

/// Where a launch came from (the chat coordinates for thread replies).
#[derive(Clone, Debug, Default)]
pub struct LaunchOrigin {
    pub kind_chat: Option<String>,
    pub thread: Option<String>,
    pub user: Option<String>,
    pub callback_url: Option<String>,
}

/// Create a run and kick the engine. Returns the queued run immediately (the
/// pipeline runs in the background).
pub async fn launch(
    ctx: &ServerCtx,
    workspace_id: &Id,
    created_by: &str,
    origin: RunOrigin,
    origin_meta: LaunchOrigin,
    req: LaunchRunReq,
) -> Result<OttoRun> {
    let (kind, source_ref, url) = determine_source(&req, origin, &origin_meta)?;
    // An explicit repo must belong to THIS workspace (the caller's role was
    // checked here, not on the repo's): refuse at launch, before any run row,
    // worktree or agent exists. `resolve_repo` re-checks at the stage.
    if let Some(rid) = req.repo_id.as_deref().filter(|s| !s.is_empty()) {
        check_repo_in_workspace(ctx, workspace_id, rid).await?;
    }

    // Channel runs seed their goal/body from the trigger message.
    let (goal, context_summary) = if kind == SourceKind::Channel {
        let seed = req.seed_text.clone().unwrap_or_default();
        let goal = if seed.trim().is_empty() {
            "Handle this request.".to_string()
        } else {
            seed.clone()
        };
        (goal, Some(seed))
    } else {
        (String::new(), None)
    };

    let title = req
        .title
        .clone()
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| default_title(kind, &source_ref));

    // Resolve the run's agent provider through the configured default: explicit
    // request → workspace default → global default → "claude".
    let provider = ctx
        .resolve_provider_for_ws(workspace_id, req.provider.as_deref())
        .await?;

    let run = ctx
        .runs
        .create(NewRun {
            workspace_id: workspace_id.clone(),
            title,
            source_kind: kind,
            source_ref,
            source_url: url,
            goal,
            mode: req.mode.unwrap_or_default(),
            provider,
            model: req
                .model
                .as_deref()
                .map(str::trim)
                .filter(|m| !m.is_empty())
                .unwrap_or_default()
                .to_string(),
            repo_id: req.repo_id.clone().filter(|s| !s.is_empty()),
            origin_kind: origin,
            origin_chat: origin_meta.kind_chat.clone(),
            origin_thread: origin_meta.thread.clone(),
            origin_user: origin_meta.user.clone(),
            callback_url: origin_meta.callback_url.clone(),
            auto_open_pr: req.auto_open_pr.unwrap_or(false),
            context_summary,
            created_by: created_by.to_string(),
        })
        .await?;

    run_engine::project(ctx, &run).await;
    let _ = ctx
        .runs
        .add_event(NewRunEvent {
            run_id: run.id.clone(),
            workspace_id: run.workspace_id.clone(),
            kind: "note".to_string(),
            status: Some(RunStatus::Queued.as_str().to_string()),
            message: format!("Queued via {}", origin.as_str()),
            detail: None,
        })
        .await;

    let ctx2 = ctx.clone();
    let rid = run.id.clone();
    tokio::spawn(async move {
        run_engine::advance(&ctx2, rid).await;
    });
    Ok(run)
}

/// Resolve `(kind, ref, url?)` from an explicit kind, a recognizable URL/key, or a
/// channel seed.
fn determine_source(
    req: &LaunchRunReq,
    origin: RunOrigin,
    origin_meta: &LaunchOrigin,
) -> Result<(SourceKind, String, Option<String>)> {
    if let (Some(kind), Some(r)) = (req.source_kind, req.source_ref.as_deref()) {
        if !r.trim().is_empty() {
            return Ok((kind, r.trim().to_string(), req.url.clone()));
        }
    }
    // Try to auto-detect from a URL or a free-text ref.
    let probe = req
        .url
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or(req.source_ref.as_deref())
        .unwrap_or("");
    if let Some(detected) = parse_source_ref(probe) {
        return Ok(detected);
    }
    // Free text (from any surface — a chat reply, a webhook, or the UI launcher's
    // "describe what you want" box) becomes a channel run.
    if req
        .seed_text
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
    {
        let handle = origin_meta
            .thread
            .clone()
            .or_else(|| origin_meta.kind_chat.clone())
            .unwrap_or_else(|| origin.as_str().to_string());
        return Ok((SourceKind::Channel, format!("thread:{handle}"), None));
    }
    Err(Error::Invalid(
        "could not determine the source — pass source_kind + source_ref, a recognizable URL/key \
         (Jira key, GitHub/Confluence URL, finding:/story:/test:/report:<id>), or seed_text"
            .to_string(),
    ))
}

fn default_title(kind: SourceKind, source_ref: &str) -> String {
    format!("Run: {} {source_ref}", kind.as_str())
}

/// Approve or reject a run at the gate. Approve → resume into `DraftingPr`;
/// reject → `Rejected`.
pub async fn approve(
    ctx: &ServerCtx,
    run_id: &Id,
    req: &ApproveRunReq,
    approver: &str,
) -> Result<OttoRun> {
    let run = ctx.runs.get(run_id).await?;
    if run.status != RunStatus::AwaitingApproval {
        return Err(Error::Invalid("run is not awaiting approval".into()));
    }
    let now = Utc::now().to_rfc3339();
    match parse_decision(&req.decision) {
        Some(ApprovalDecision::Approve) => {
            if !ctx
                .runs
                .set_status_cas(run_id, RunStatus::AwaitingApproval, RunStatus::DraftingPr)
                .await?
            {
                return Err(Error::Conflict("run already moved".into()));
            }
            ctx.runs
                .set_fields(
                    run_id,
                    &RunPatch {
                        approval_decision: Some("approved".into()),
                        approved_by: Some(approver.to_string()),
                        approved_at: Some(now),
                        ..Default::default()
                    },
                )
                .await?;
            log_approval(ctx, &run, "approved", req.note.as_deref()).await;
            let ctx2 = ctx.clone();
            let rid = run_id.clone();
            tokio::spawn(async move {
                run_engine::resume_after_approval(&ctx2, rid).await;
            });
        }
        Some(ApprovalDecision::Reject) => {
            if !ctx
                .runs
                .set_status_cas(run_id, RunStatus::AwaitingApproval, RunStatus::Rejected)
                .await?
            {
                return Err(Error::Conflict("run already moved".into()));
            }
            ctx.runs
                .set_fields(
                    run_id,
                    &RunPatch {
                        approval_decision: Some("rejected".into()),
                        approved_by: Some(approver.to_string()),
                        approved_at: Some(now),
                        ..Default::default()
                    },
                )
                .await?;
            log_approval(ctx, &run, "rejected", req.note.as_deref()).await;
            if let Ok(fresh) = ctx.runs.get(run_id).await {
                run_engine::project(ctx, &fresh).await;
                crate::run_callback::deliver(&ctx.runs, &fresh).await;
            }
            crate::run_workspace::remove_worktree(ctx, &run).await;
        }
        None => {
            return Err(Error::Invalid(
                "decision must be 'approve' or 'reject'".into(),
            ))
        }
    }
    ctx.runs.get(run_id).await
}

async fn log_approval(ctx: &ServerCtx, run: &OttoRun, decision: &str, note: Option<&str>) {
    let _ = ctx
        .runs
        .add_event(NewRunEvent {
            run_id: run.id.clone(),
            workspace_id: run.workspace_id.clone(),
            kind: "approval".to_string(),
            status: Some(run.status.as_str().to_string()),
            message: match note {
                Some(n) if !n.trim().is_empty() => format!("Run {decision}: {n}"),
                _ => format!("Run {decision}"),
            },
            detail: None,
        })
        .await;
}

/// `NotFound` unless repo `repo_id` exists AND belongs to `workspace_id` — a
/// repo of another workspace is reported exactly like a missing one.
pub(crate) async fn check_repo_in_workspace(
    ctx: &ServerCtx,
    workspace_id: &Id,
    repo_id: &str,
) -> Result<()> {
    match ctx.git_store.get_repo(&repo_id.to_string()).await {
        Ok(r) if r.workspace_id == *workspace_id => Ok(()),
        Ok(_) | Err(Error::NotFound(_)) => Err(Error::NotFound("repo".into())),
        Err(e) => Err(e),
    }
}

/// Cancel a non-terminal run. Forceful (not a CAS) — the engine's next CAS then
/// no-ops, and the worktree is cleaned up.
pub async fn cancel(ctx: &ServerCtx, run_id: &Id) -> Result<OttoRun> {
    let run = ctx.runs.get(run_id).await?;
    if run.status.is_terminal() {
        return Ok(run);
    }
    ctx.runs.set_status(run_id, RunStatus::Cancelled).await?;
    let _ = ctx
        .runs
        .add_event(NewRunEvent {
            run_id: run.id.clone(),
            workspace_id: run.workspace_id.clone(),
            kind: "note".to_string(),
            status: Some(RunStatus::Cancelled.as_str().to_string()),
            message: "Run cancelled".to_string(),
            detail: None,
        })
        .await;
    // Stop the work, not just the status: drop the in-flight stage (kills the
    // agent), cancel the review it started and stop its goal loop. The
    // worktree goes only after the stage has stopped, so nothing is left
    // running in a deleted directory.
    ctx.runs_engine.cancel(run_id);
    if !ctx
        .runs_engine
        .wait_idle(run_id, std::time::Duration::from_secs(10))
        .await
    {
        tracing::warn!(run = %run_id, "run stage still in flight 10s after cancel");
    }
    // Re-read AFTER the stage stopped: a review started in the last moments of
    // the stage is recorded on the run before its reviewers spawn, and only a
    // read taken now is guaranteed to see it (S2-302).
    let fresh = ctx.runs.get(run_id).await.unwrap_or_else(|_| run.clone());
    if let Some(review_id) = fresh.review_id.as_ref() {
        if let Ok(review) = ctx.reviews_store.get_review(review_id).await {
            crate::modules::cancel_running_review(ctx, &review, &run.workspace_id).await;
        }
    }
    if let Some(loop_id) = fresh.goal_loop_id.as_ref() {
        if let Err(e) = crate::goal_loop::stop_loop(ctx, loop_id).await {
            tracing::warn!(run = %run_id, "stop goal loop on cancel: {e}");
        }
    }
    stop_run_sessions(ctx, &run.workspace_id, &run.id).await;
    run_engine::project(ctx, &fresh).await;
    crate::run_callback::deliver(&ctx.runs, &fresh).await;
    crate::run_workspace::remove_worktree(ctx, &run).await;
    ctx.runs.get(run_id).await
}

/// The `meta.source` values of the manager-owned sessions a run stage creates:
/// a non-claude execute turn and the PR-draft turn. Claude execute runs on an
/// orchestrator PTY that dies with the dropped stage future; these do NOT — a
/// dropped `run_session_turn` only releases its turn hold.
const RUN_SESSION_SOURCES: &[&str] = &["run_with_otto", "pr-draft"];

/// Kill every live session the run's stages started (`meta.run_id == run.id`),
/// so a cancelled codex/agy run stops editing (and spending) in a worktree that
/// is about to be removed (S2-302 / S15-306). Interactive sessions are never
/// touched: only the two stage sources above carry a `run_id`.
pub(crate) async fn stop_run_sessions(ctx: &ServerCtx, workspace_id: &Id, run_id: &Id) -> usize {
    let sessions = match ctx
        .manager
        .list_live_by_meta(workspace_id, None, "run_id", run_id.as_str())
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(run = %run_id, "list run sessions on cancel: {e}");
            return 0;
        }
    };
    let mut stopped = 0;
    for s in sessions {
        let source = s.meta.get("source").and_then(|v| v.as_str()).unwrap_or("");
        if !RUN_SESSION_SOURCES.contains(&source) {
            continue;
        }
        match ctx.manager.kill_session(&s.id).await {
            Ok(()) => stopped += 1,
            Err(e) => tracing::warn!(run = %run_id, session = %s.id, "kill run session: {e}"),
        }
    }
    stopped
}

/// Open the actual PR from a completed, approved run. Requires the proof pack to
/// be passed/waived (mirrors the `gate_pr` posture) — an outward action.
pub async fn open_pr(ctx: &ServerCtx, run_id: &Id) -> Result<PrSummary> {
    let run = ctx.runs.get(run_id).await?;
    // The single outward-facing gate: approved AND proof passed/waived AND a
    // draft + repo to point at. Pure decision in otto-core; mapped to the same
    // transport errors as before (proof → Conflict, the rest → Invalid).
    if let Some(block) = open_pr_block_reason(
        run.approval_decision.as_deref(),
        run.proof_status.as_deref(),
        run.pr_draft_json.is_some(),
        run.repo_id.is_some(),
    ) {
        let msg = block.message().to_string();
        return Err(match block {
            OpenPrBlock::ProofNotPassed => Error::Conflict(msg),
            _ => Error::Invalid(msg),
        });
    }
    let draft_json = run
        .pr_draft_json
        .as_deref()
        .ok_or_else(|| Error::Invalid("run has no PR draft".into()))?;
    let draft: otto_core::api::DraftPrResp =
        serde_json::from_str(draft_json).map_err(|e| Error::Internal(format!("bad draft: {e}")))?;
    let repo_id = run
        .repo_id
        .as_deref()
        .ok_or_else(|| Error::Invalid("run has no repo".into()))?;
    let repo = ctx.git_store.get_repo(&repo_id.to_string()).await?;
    let (provider, remote) = crate::run_sources::provider_for_repo(ctx, &repo).await?;

    // Push the branch first so the PR has a head to point at.
    if let Some(wt) = run.worktree_path.as_deref() {
        let git = otto_git::LocalGit::new(wt);
        // FIRST check before the PR: a stalled agent can leave its work
        // uncommitted — the push is then a no-op and the provider rejects the
        // PR ("no changes to be pulled"). Run worktrees only (this path is
        // always a dedicated worktree, never the user's main checkout).
        if let Ok(Some(_)) = git
            .commit_all_if_dirty("chore: commit run changes left before PR")
            .await
        {
            let _ = ctx
                .runs
                .add_event(NewRunEvent {
                    run_id: run.id.clone(),
                    workspace_id: run.workspace_id.clone(),
                    kind: "note".to_string(),
                    status: Some(run.status.as_str().to_string()),
                    message: "committed leftover worktree changes before PR".to_string(),
                    detail: None,
                })
                .await;
        }
        let token = match repo.git_account_id.as_ref() {
            Some(aid) => match ctx.git_store.get_account(aid).await {
                Ok(acc) => otto_core::secrets::get_async(&ctx.secrets, &acc.token_ref)
                    .await
                    .ok()
                    .flatten(),
                Err(_) => None,
            },
            None => None,
        };
        let _ = git.push(token).await;
    }

    let create = CreatePrReq {
        title: draft.title,
        description: draft.description,
        source_branch: draft.source_branch,
        target_branch: draft.target_branch,
        proof_pack_id: run.proof_pack_id.clone(),
        allow_unproven: None,
        draft: None,
        reviewers: None,
    };
    let pr = provider.create_pr(&remote, &create).await?;
    ctx.runs
        .set_fields(
            run_id,
            &RunPatch {
                pr_url: Some(pr.url.clone()),
                ..Default::default()
            },
        )
        .await?;
    let _ = ctx
        .runs
        .add_event(NewRunEvent {
            run_id: run.id.clone(),
            workspace_id: run.workspace_id.clone(),
            kind: "note".to_string(),
            status: Some(run.status.as_str().to_string()),
            message: format!("PR opened: {}", pr.url),
            detail: None,
        })
        .await;
    Ok(pr)
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_state::sessions::NewSession;

    /// S2-302 / S15-306: a cancel kills the run's manager-owned stage sessions
    /// (a codex execute turn, the PR-draft turn) — a dropped stage future does
    /// not — and leaves every other session alone, including one that happens
    /// to carry the same `run_id` under another source (a workflow step).
    #[tokio::test]
    async fn cancel_stops_only_the_runs_own_stage_sessions() {
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = crate::test_support::mem_pool().await;
        let ctx = ServerCtx::for_tests(&pool, tmp.path().to_path_buf()).await;
        let users = otto_state::UsersRepo::new(pool.clone());
        let u = users.create("u", "x", "U", false).await.unwrap();
        let root = tmp.path().to_string_lossy().to_string();
        let ws = ctx.workspaces.create("A", &root, &u.id).await.unwrap();
        let repo = otto_state::SessionsRepo::new(pool.clone());
        let mk = |meta: serde_json::Value| NewSession {
            workspace_id: ws.id.clone(),
            kind: otto_core::domain::SessionKind::Agent,
            provider: "codex".into(),
            title: "t".into(),
            cwd: root.clone(),
            provider_session_id: None,
            connection_id: None,
            created_by: u.id.clone(),
            meta,
        };
        let run_id: Id = "run-1".into();
        let exec = repo
            .create(mk(serde_json::json!({"source": "run_with_otto", "run_id": "run-1"})))
            .await
            .unwrap();
        let draft = repo
            .create(mk(serde_json::json!({"source": "pr-draft", "run_id": "run-1"})))
            .await
            .unwrap();
        let other_run = repo
            .create(mk(serde_json::json!({"source": "run_with_otto", "run_id": "run-2"})))
            .await
            .unwrap();
        let workflow = repo
            .create(mk(serde_json::json!({"source": "workflow", "run_id": "run-1"})))
            .await
            .unwrap();
        let interactive = repo.create(mk(serde_json::json!({}))).await.unwrap();

        assert_eq!(stop_run_sessions(&ctx, &ws.id, &run_id).await, 2);
        let status = |id: Id| {
            let repo = repo.clone();
            async move { repo.get(&id).await.unwrap().status }
        };
        use otto_core::domain::SessionStatus;
        assert_eq!(status(exec.id).await, SessionStatus::Exited);
        assert_eq!(status(draft.id).await, SessionStatus::Exited);
        assert_ne!(status(other_run.id).await, SessionStatus::Exited);
        assert_ne!(status(workflow.id).await, SessionStatus::Exited);
        assert_ne!(status(interactive.id).await, SessionStatus::Exited);
    }
}
