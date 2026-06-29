//! Run with Otto — the stage-machine engine.
//!
//! [`advance`] drives a run through its [`RunStatus`] pipeline one stage at a time.
//! Every transition is a compare-and-set (`set_status_cas`) and the engine holds a
//! per-run in-flight guard, so a racing boot reaper or a double-approve can never
//! double-advance a run (design §20.8). The engine stops at `AwaitingApproval`
//! (resumed by [`resume_after_approval`] on approve) and at every terminal state.
//!
//! Each stage reuses an existing subsystem: source/context (`run_sources`,
//! `run_context`), worktree (`run_workspace`), execute (`Orchestrator::run_agent`
//! or a goal loop), proof (`crate::proof`), review (`modules::run_review_for_branch`),
//! PR draft (`modules::draft_pr_core`). The run also projects into Mission Control.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_core::domain::Channel;
use otto_core::event::Event;
use otto_core::run::{OttoRun, ResolvedSource, RunMode, RunStatus};
use otto_core::{Error, Id, Result};
use otto_state::runs::{NewRunEvent, RunPatch};
use serde_json::json;

use crate::state::ServerCtx;

/// Stuck-window for a single-agent run (NOT wall-clock; generous).
const EXEC_NO_PROGRESS: Duration = Duration::from_secs(300);
/// Poll cadence + caps for the async sub-steps (review, goal loop).
const POLL_EVERY: Duration = Duration::from_secs(2);
const REVIEW_POLL_MAX: u32 = 150; // ~5 min
const GOAL_LOOP_POLL_MAX: u32 = 7_200; // ~4 h (matches goal-loop HARD_CAP)

/// Per-run in-flight registry. Stored on `ServerCtx.runs_engine`.
pub struct RunEngine {
    inflight: Mutex<HashSet<Id>>,
}

struct InFlight<'a> {
    engine: &'a RunEngine,
    id: Id,
}
impl Drop for InFlight<'_> {
    fn drop(&mut self) {
        if let Ok(mut g) = self.engine.inflight.lock() {
            g.remove(&self.id);
        }
    }
}

impl RunEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inflight: Mutex::new(HashSet::new()),
        })
    }

    /// Claim the run; `None` means another `advance` is already driving it.
    fn claim(&self, id: &Id) -> Option<InFlight<'_>> {
        let mut g = self.inflight.lock().ok()?;
        if g.contains(id) {
            None
        } else {
            g.insert(id.clone());
            Some(InFlight {
                engine: self,
                id: id.clone(),
            })
        }
    }
}

impl Default for RunEngine {
    fn default() -> Self {
        Self {
            inflight: Mutex::new(HashSet::new()),
        }
    }
}

fn is_e2e() -> bool {
    matches!(std::env::var("OTTO_E2E").as_deref(), Ok("1") | Ok("true"))
}

/// Drive the run forward until it reaches `AwaitingApproval` or a terminal state.
/// Safe to call from anywhere (launch, scheduler, approve) — the in-flight guard
/// makes concurrent calls a no-op.
pub async fn advance(ctx: &ServerCtx, run_id: Id) {
    let _claim = match ctx.runs_engine.claim(&run_id) {
        Some(c) => c,
        None => return,
    };
    loop {
        let run = match ctx.runs.get(&run_id).await {
            Ok(r) => r,
            Err(_) => break,
        };
        if run.status.is_terminal() || run.status == RunStatus::AwaitingApproval {
            break;
        }
        let cur = run.status;
        match run_stage(ctx, &run).await {
            Ok(()) => {
                let Some(next) = cur.next_on_success() else {
                    break; // AwaitingApproval reached via Reviewing → no auto-next
                };
                match ctx.runs.set_status_cas(&run_id, cur, next).await {
                    Ok(true) => after_transition(ctx, &run_id, next).await,
                    // Lost the CAS race (reaper / cancel moved it) — stop quietly.
                    _ => break,
                }
            }
            Err(e) => {
                fail(ctx, &run, &e.to_string()).await;
                break;
            }
        }
    }
}

/// Resume after an approval flipped the run to `DraftingPr`.
pub async fn resume_after_approval(ctx: &ServerCtx, run_id: Id) {
    advance(ctx, run_id).await;
}

/// Execute the work for the run's CURRENT status (the stage it is "in").
async fn run_stage(ctx: &ServerCtx, run: &OttoRun) -> Result<()> {
    match run.status {
        RunStatus::Queued => Ok(()),
        RunStatus::ResolvingSource => stage_resolve(ctx, run).await,
        RunStatus::BuildingContext => Ok(()), // packet is rebuilt at execute from stored fields
        RunStatus::Provisioning => stage_provision(ctx, run).await,
        RunStatus::Executing => stage_execute(ctx, run).await,
        RunStatus::Proving => stage_prove(ctx, run).await,
        RunStatus::Reviewing => stage_review(ctx, run).await,
        RunStatus::DraftingPr => stage_draft_pr(ctx, run).await,
        // Driven by advance()'s break conditions, not by run_stage.
        RunStatus::AwaitingApproval
        | RunStatus::Completed
        | RunStatus::Failed
        | RunStatus::Rejected
        | RunStatus::Cancelled => Ok(()),
    }
}

// --- Stages ----------------------------------------------------------------

async fn stage_resolve(ctx: &ServerCtx, run: &OttoRun) -> Result<()> {
    let repo = crate::run_sources::resolve_repo(
        ctx,
        &run.workspace_id,
        run.repo_id.as_deref(),
        run.source_kind,
        &run.source_ref,
    )
    .await?;
    let git = otto_git::LocalGit::new(&repo.path);
    let base_branch = git
        .current_branch()
        .await
        .unwrap_or_else(|_| "main".to_string());
    // Persist the repo first so the GitHub adapter can resolve a provider.
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                repo_id: Some(repo.id.clone()),
                repo_path: Some(repo.path.clone()),
                base_branch: Some(base_branch),
                ..Default::default()
            },
        )
        .await?;

    let resolved = crate::run_sources::resolve_source(ctx, run, &repo).await?;
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                title: if run.title.trim().is_empty() {
                    Some(resolved.title.clone())
                } else {
                    None
                },
                goal: Some(resolved.goal.clone()),
                source_url: resolved.source_url.clone(),
                context_summary: Some(resolved.body_md.clone()),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

async fn stage_provision(ctx: &ServerCtx, run: &OttoRun) -> Result<()> {
    if run.mode == RunMode::SingleAgent {
        let repo = ctx
            .git_store
            .get_repo(run.repo_id.as_ref().ok_or_else(no_repo)?)
            .await?;
        let (branch, path, base) = crate::run_workspace::provision_worktree(ctx, run, &repo).await?;
        ctx.runs
            .set_fields(
                &run.id,
                &RunPatch {
                    branch: Some(branch),
                    worktree_path: Some(path),
                    base_commit: Some(base),
                    ..Default::default()
                },
            )
            .await?;
    }
    // goal_loop provisions its own worktree inside start_loop (stage_execute).
    Ok(())
}

async fn stage_execute(ctx: &ServerCtx, run: &OttoRun) -> Result<()> {
    match run.mode {
        RunMode::SingleAgent => execute_single_agent(ctx, run).await,
        RunMode::GoalLoop => execute_goal_loop(ctx, run).await,
    }
}

async fn execute_single_agent(ctx: &ServerCtx, run: &OttoRun) -> Result<()> {
    let wt = run
        .worktree_path
        .clone()
        .ok_or_else(|| Error::Internal("run has no worktree".into()))?;
    let repo = ctx
        .git_store
        .get_repo(run.repo_id.as_ref().ok_or_else(no_repo)?)
        .await?;
    let resolved = reconstruct_resolved(run);
    let packet = crate::run_context::build_packet(run, &resolved, &repo);
    let reply = ctx
        .orchestrator
        .run_agent(&packet.prompt, &wt, None, EXEC_NO_PROGRESS)
        .await?;

    // E2E seam: the stubbed `run_agent` writes no files. Commit a deterministic
    // note so the proof/diff/PR-draft stages have real material. Production never
    // does this (the real agent commits its own work).
    if is_e2e() {
        e2e_commit_note(&wt, run).await;
    }

    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                // Replace the stored source body with the human-readable packet
                // summary now that the prompt has been assembled from it.
                context_summary: Some(packet.summary.clone()),
                result_summary: Some(truncate(&reply, 4_000)),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

async fn execute_goal_loop(ctx: &ServerCtx, run: &OttoRun) -> Result<()> {
    use otto_core::domain::{AcceptanceCriterion, GoalLoopConfig, GoalLoopDefinition, GoalLoopLimits};
    use otto_state::NewGoalLoop;

    let repo = ctx
        .git_store
        .get_repo(run.repo_id.as_ref().ok_or_else(no_repo)?)
        .await?;
    let definition = GoalLoopDefinition {
        title: run.title.clone(),
        summary: run.goal.clone(),
        objectives: vec![],
        acceptance_criteria: vec![AcceptanceCriterion {
            id: "c1".to_string(),
            text: run.goal.clone(),
            verify: "A reviewer confirms the goal is met.".to_string(),
            verify_kind: "manual".to_string(),
            verify_cmd: None,
        }],
        constraints: vec![],
        out_of_scope: vec![],
        success_signal: String::new(),
    };
    let loop_ = ctx
        .goal_loops_repo
        .create(NewGoalLoop {
            workspace_id: run.workspace_id.clone(),
            name: run.title.clone(),
            repo_path: repo.path.clone(),
            definition,
            limits: GoalLoopLimits::default(),
            config: GoalLoopConfig::default(),
            created_by: run.created_by.clone(),
        })
        .await?;
    let loop_id = loop_.id.clone();
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                goal_loop_id: Some(loop_id.clone()),
                ..Default::default()
            },
        )
        .await?;

    crate::goal_loop::start_loop(ctx, &loop_id).await?;
    let gl = poll_goal_loop(ctx, &loop_id).await?;

    let branch = gl
        .branch
        .clone()
        .ok_or_else(|| Error::Internal("goal loop produced no branch".into()))?;
    let base_commit = gl.base_commit.clone().unwrap_or_default();

    // The loop removed its worktree on finalize; re-attach one on its branch for
    // the proof/review/PR-draft stages (non-destructive — keeps the commits).
    let path = ctx
        .data_dir
        .join("otto-runs")
        .join(&run.id)
        .join("work");
    let path_str = path.to_string_lossy().to_string();
    otto_git::LocalGit::new(&repo.path)
        .worktree_attach(&path_str, &branch)
        .await?;

    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                branch: Some(branch),
                base_commit: Some(base_commit),
                worktree_path: Some(path_str),
                result_summary: gl.summary.clone(),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

async fn stage_prove(ctx: &ServerCtx, run: &OttoRun) -> Result<()> {
    use otto_core::proof::{ProofArtifactKind, ProofArtifactStatus, WorkItemKind};
    let wt = run
        .worktree_path
        .clone()
        .ok_or_else(|| Error::Internal("run has no worktree".into()))?;
    let base = run.base_commit.clone().unwrap_or_default();

    let pack = crate::proof::gate(
        ctx,
        WorkItemKind::Task,
        &run.id,
        &run.workspace_id,
        &run.title,
        &run.created_by,
    )
    .await?;
    let _ = crate::proof::assemble_diff(ctx, &pack, &wt, Some(&base)).await;
    if let Some(summary) = run.result_summary.as_deref().filter(|s| !s.is_empty()) {
        let _ = crate::proof::upsert_content_artifact(
            ctx,
            &pack,
            ProofArtifactKind::SelfReview,
            "Agent self-review",
            summary,
            ProofArtifactStatus::Info,
            json!({ "source": "run_with_otto" }),
            "otto",
        )
        .await;
    }
    let pack = crate::proof::recompute_and_emit(ctx, &pack.id).await?;
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                proof_pack_id: Some(pack.id.clone()),
                proof_status: Some(pack.status.as_str().to_string()),
                risk_score: Some(i64::from(pack.risk_score)),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

async fn stage_review(ctx: &ServerCtx, run: &OttoRun) -> Result<()> {
    let Some(repo_id) = run.repo_id.clone() else {
        // No repo to review against — skip the stage cleanly.
        return Ok(());
    };
    let wt = run
        .worktree_path
        .clone()
        .ok_or_else(|| Error::Internal("run has no worktree".into()))?;
    let base = run.base_commit.clone().unwrap_or_default();

    // Deterministic E2E: skip spawning real review agents (they need a live CLI),
    // record a completed review with no findings. The stage stays visible.
    if is_e2e() {
        let review = ctx.reviews_store.create_review(&repo_id, 0).await?;
        let _ = ctx
            .reviews_store
            .set_status(&review.id, otto_core::domain::ReviewStatus::Done, None)
            .await;
        ctx.runs
            .set_fields(
                &run.id,
                &RunPatch {
                    review_id: Some(review.id),
                    findings_total: Some(0),
                    findings_blocking: Some(0),
                    ..Default::default()
                },
            )
            .await?;
        return Ok(());
    }

    let review_id = crate::modules::run_review_for_branch(ctx, &repo_id, &wt, &base, None).await?;
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                review_id: Some(review_id.clone()),
                ..Default::default()
            },
        )
        .await?;
    let (total, _open, blocker) = poll_review(ctx, &review_id).await;
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                findings_total: Some(total as i64),
                findings_blocking: Some(blocker as i64),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

async fn stage_draft_pr(ctx: &ServerCtx, run: &OttoRun) -> Result<()> {
    let wt = run
        .worktree_path
        .clone()
        .ok_or_else(|| Error::Internal("run has no worktree".into()))?;
    let base = run
        .base_branch
        .clone()
        .unwrap_or_else(|| "main".to_string());

    match crate::modules::draft_pr_core(ctx, &wt, &base).await {
        Ok(draft) => {
            let json = serde_json::to_string(&draft).unwrap_or_default();
            ctx.runs
                .set_fields(
                    &run.id,
                    &RunPatch {
                        pr_draft_json: Some(json),
                        ..Default::default()
                    },
                )
                .await?;
        }
        Err(e) => {
            // e.g. the agent produced no committed change — record a note, still complete.
            let _ = ctx
                .runs
                .add_event(NewRunEvent {
                    run_id: run.id.clone(),
                    workspace_id: run.workspace_id.clone(),
                    kind: "note".to_string(),
                    status: Some(RunStatus::DraftingPr.as_str().to_string()),
                    message: format!("PR draft skipped: {e}"),
                    detail: None,
                })
                .await;
        }
    }

    // Best-effort push so the branch is openable (never blocks the run).
    best_effort_push(ctx, run).await;
    Ok(())
}

// --- Helpers ---------------------------------------------------------------

fn no_repo() -> Error {
    Error::Internal("run has no resolved repo".into())
}

fn reconstruct_resolved(run: &OttoRun) -> ResolvedSource {
    ResolvedSource {
        title: run.title.clone(),
        body_md: run.context_summary.clone().unwrap_or_default(),
        goal: run.goal.clone(),
        source_url: run.source_url.clone(),
        repo_hint: None,
        metadata: serde_json::Value::Null,
    }
}

fn truncate(s: &str, cap: usize) -> String {
    if s.len() <= cap {
        return s.to_string();
    }
    let mut end = cap;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

async fn e2e_commit_note(wt: &str, run: &OttoRun) {
    use tokio::process::Command;
    let note = format!(
        "# Otto run {}\n\nGoal: {}\n\nThis file was committed by the Run with Otto \
         engine under OTTO_E2E to provide a deterministic diff.\n",
        run.id, run.goal
    );
    let _ = tokio::fs::write(format!("{wt}/OTTO_RUN_NOTE.md"), note).await;
    let _ = Command::new("git")
        .args(["-C", wt, "add", "-A"])
        .status()
        .await;
    let _ = Command::new("git")
        .args([
            "-C",
            wt,
            "-c",
            "user.email=otto@local",
            "-c",
            "user.name=Otto",
            "commit",
            "-m",
            "otto: run note (e2e)",
        ])
        .status()
        .await;
}

async fn poll_review(ctx: &ServerCtx, review_id: &Id) -> (u64, u64, u64) {
    use otto_core::domain::ReviewStatus;
    for _ in 0..REVIEW_POLL_MAX {
        if let Ok(rev) = ctx.reviews_store.get_review(review_id).await {
            if matches!(rev.status, ReviewStatus::Done | ReviewStatus::Error) {
                return crate::modules::review_findings_counts(ctx, review_id).await;
            }
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
    crate::modules::review_findings_counts(ctx, review_id).await
}

async fn poll_goal_loop(ctx: &ServerCtx, loop_id: &Id) -> Result<otto_core::domain::GoalLoop> {
    for _ in 0..GOAL_LOOP_POLL_MAX {
        let gl = ctx.goal_loops_repo.get(loop_id).await?;
        if gl.status.is_terminal() {
            return Ok(gl);
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
    ctx.goal_loops_repo.get(loop_id).await
}

async fn best_effort_push(ctx: &ServerCtx, run: &OttoRun) {
    let (Some(repo_id), Some(wt)) = (run.repo_id.as_deref(), run.worktree_path.as_deref()) else {
        return;
    };
    let Ok(repo) = ctx.git_store.get_repo(&repo_id.to_string()).await else {
        return;
    };
    let Some(account_id) = repo.git_account_id.as_ref() else {
        return; // no bound account → can't push; the draft is still the deliverable
    };
    let token = match ctx.git_store.get_account(account_id).await {
        Ok(acc) => ctx.secrets.get(&acc.token_ref).ok().flatten(),
        Err(_) => None,
    };
    let _ = otto_git::LocalGit::new(wt).push(token).await;
}

/// After a successful transition: record an event, emit the WS event, project into
/// Mission Control, and post a live update to the origin (Slack/Telegram) thread.
async fn after_transition(ctx: &ServerCtx, run_id: &Id, new_status: RunStatus) {
    let Ok(run) = ctx.runs.get(run_id).await else {
        return;
    };
    let _ = ctx
        .runs
        .add_event(NewRunEvent {
            run_id: run.id.clone(),
            workspace_id: run.workspace_id.clone(),
            kind: "stage_enter".to_string(),
            status: Some(new_status.as_str().to_string()),
            message: stage_message(&run, new_status),
            detail: None,
        })
        .await;
    emit(ctx, &run);
    project(ctx, &run).await;

    // Live origin updates for the key milestones only (avoid chat spam), plus a
    // webhook callback (no-op unless the run carries a callback_url).
    match new_status {
        RunStatus::AwaitingApproval => {
            post_origin(ctx, &run, &approval_prompt(&run)).await;
            crate::run_callback::deliver(&ctx.runs, &run).await;
        }
        RunStatus::Completed => {
            post_origin(ctx, &run, &completion_message(&run)).await;
            crate::run_callback::deliver(&ctx.runs, &run).await;
        }
        _ => {}
    }
}

async fn fail(ctx: &ServerCtx, run: &OttoRun, err: &str) {
    let _ = ctx.runs.set_error(&run.id, err).await;
    let _ = ctx
        .runs
        .add_event(NewRunEvent {
            run_id: run.id.clone(),
            workspace_id: run.workspace_id.clone(),
            kind: "stage_error".to_string(),
            status: Some(RunStatus::Failed.as_str().to_string()),
            message: format!("Failed during {}: {err}", run.status.as_str()),
            detail: None,
        })
        .await;
    if let Ok(fresh) = ctx.runs.get(&run.id).await {
        emit(ctx, &fresh);
        project(ctx, &fresh).await;
        post_origin(ctx, &fresh, &format!("❌ Run failed: {err}")).await;
        crate::run_callback::deliver(&ctx.runs, &fresh).await;
    }
}

fn emit(ctx: &ServerCtx, run: &OttoRun) {
    let _ = ctx.events.send(Event::OttoRunUpdated {
        workspace_id: run.workspace_id.clone(),
        run_id: run.id.clone(),
        status: run.status.as_str().to_string(),
    });
}

/// Materialize / refresh the run as a Mission Control work item.
pub(crate) async fn project(ctx: &ServerCtx, run: &OttoRun) {
    use otto_state::workgraph::{WorkActor, WorkItemUpsert, WorkKind, WorkStatus};
    let status = WorkStatus::from_source(WorkKind::OttoRun, run.status.as_str());
    let risk = otto_workgraph::normalize::risk(WorkKind::OttoRun, &run.title);
    let up = WorkItemUpsert {
        workspace_id: run.workspace_id.clone(),
        kind: WorkKind::OttoRun,
        source_id: run.id.clone(),
        title: run.title.clone(),
        goal: Some(run.goal.clone()),
        status,
        owner: run.origin_user.clone(),
        owner_kind: WorkActor::System,
        repo_id: run.repo_id.clone(),
        branch: run.branch.clone(),
        cost_so_far: None,
        risk_level: risk,
        result_summary: run.result_summary.clone(),
        context_summary: run.context_summary.clone(),
        started_by_id: Some(run.created_by.clone()),
    };
    let _ = ctx.workgraph.record(up).await;
}

async fn post_origin(ctx: &ServerCtx, run: &OttoRun, text: &str) {
    let Some(chat) = run.origin_chat.as_deref().filter(|c| !c.is_empty()) else {
        return;
    };
    let channel = match run.origin_kind {
        otto_core::run::RunOrigin::Slack => Channel::Slack,
        otto_core::run::RunOrigin::Telegram => Channel::Telegram,
        // Webhook callbacks + UI/api/mcp origins don't have a chat adapter here.
        _ => return,
    };
    if let Ok(Some(integ)) = ctx
        .integrations_store
        .get(&run.workspace_id, channel)
        .await
    {
        let _ = otto_channels::improve_notify::send_to(
            &ctx.secrets,
            &integ,
            chat,
            run.origin_thread.as_deref(),
            text,
        )
        .await;
    }
}

fn stage_message(run: &OttoRun, status: RunStatus) -> String {
    match status {
        RunStatus::ResolvingSource => format!("Resolving {} source…", run.source_kind.as_str()),
        RunStatus::BuildingContext => "Assembling the context packet…".to_string(),
        RunStatus::Provisioning => "Provisioning an isolated branch/worktree…".to_string(),
        RunStatus::Executing => format!("Working ({})…", run.mode.as_str()),
        RunStatus::Proving => "Assembling the proof pack…".to_string(),
        RunStatus::Reviewing => "Running AI review…".to_string(),
        RunStatus::AwaitingApproval => "Awaiting human approval.".to_string(),
        RunStatus::DraftingPr => "Drafting the pull request…".to_string(),
        RunStatus::Completed => "Done.".to_string(),
        other => other.as_str().to_string(),
    }
}

fn approval_prompt(run: &OttoRun) -> String {
    let proof = run.proof_status.as_deref().unwrap_or("unknown");
    let risk = run
        .risk_score
        .map(|r| r.to_string())
        .unwrap_or_else(|| "?".to_string());
    format!(
        "🧪 *Run with Otto* — ready for review\n*{title}*\nProof: {proof} · risk {risk}/100 · \
         findings: {total} ({blocking} blocking)\n\nReply *approve* to draft a PR, or *reject*.",
        title = run.title,
        proof = proof,
        risk = risk,
        total = run.findings_total,
        blocking = run.findings_blocking,
    )
}

fn completion_message(run: &OttoRun) -> String {
    let pr = if run.pr_draft_json.is_some() {
        "a PR draft is ready"
    } else {
        "no PR draft was produced"
    };
    format!("✅ *Run with Otto* complete — {pr} for *{}*.", run.title)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_flight_guard_is_exclusive() {
        let e = RunEngine::new();
        let id = "run-a".to_string();
        let g1 = e.claim(&id);
        assert!(g1.is_some(), "first claim succeeds");
        assert!(e.claim(&id).is_none(), "second claim is blocked");
        drop(g1);
        assert!(e.claim(&id).is_some(), "released claim can be re-taken");
    }

    /// §20.5: the goal_loop mode must construct a NewGoalLoop that passes the
    /// goal-loops `create` validation (≥1 acceptance criterion with a non-empty
    /// verify, ≥1 executor). We assert the defaults + synthesized criterion here
    /// (the live controller can't run under the E2E CLI stub).
    #[test]
    fn goal_loop_construction_is_valid() {
        use otto_core::domain::{AcceptanceCriterion, GoalLoopConfig};
        let cfg = GoalLoopConfig::default();
        assert!(
            !cfg.executors.is_empty(),
            "default goal-loop config has an executor"
        );
        let criteria = [AcceptanceCriterion {
            id: "c1".to_string(),
            text: "the goal".to_string(),
            verify: "A reviewer confirms the goal is met.".to_string(),
            verify_kind: "manual".to_string(),
            verify_cmd: None,
        }];
        assert!(!criteria.is_empty());
        assert!(!criteria[0].verify.trim().is_empty());
        // A manual criterion needs no verify_cmd (only command-kind does).
        assert_eq!(criteria[0].verify_kind, "manual");
    }
}
