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
            provider: req
                .provider
                .filter(|p| !p.trim().is_empty())
                .unwrap_or_else(|| "claude".to_string()),
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
    if req.seed_text.as_deref().is_some_and(|s| !s.trim().is_empty()) {
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
                crate::run_callback::deliver(ctx, &fresh).await;
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
    if let Ok(fresh) = ctx.runs.get(run_id).await {
        run_engine::project(ctx, &fresh).await;
        crate::run_callback::deliver(ctx, &fresh).await;
    }
    crate::run_workspace::remove_worktree(ctx, &run).await;
    ctx.runs.get(run_id).await
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
        let token = match repo.git_account_id.as_ref() {
            Some(aid) => match ctx.git_store.get_account(aid).await {
                Ok(acc) => ctx.secrets.get(&acc.token_ref).ok().flatten(),
                Err(_) => None,
            },
            None => None,
        };
        let _ = otto_git::LocalGit::new(wt).push(token).await;
    }

    let create = CreatePrReq {
        title: draft.title,
        description: draft.description,
        source_branch: draft.source_branch,
        target_branch: draft.target_branch,
        proof_pack_id: run.proof_pack_id.clone(),
        allow_unproven: None,
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
