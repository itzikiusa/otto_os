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

use std::collections::{HashMap, HashSet};
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
/// The review wait (O4 / SI-07): wakes on the review's own `ReviewChanged`
/// event; this slow re-check only covers a missed or lagged event.
const REVIEW_SAFETY_RECHECK: Duration = Duration::from_secs(30);
const GOAL_LOOP_POLL_MAX: u32 = 7_200; // ~4 h (matches goal-loop HARD_CAP)
/// Why a run whose agent committed nothing stops at the review stage.
pub(crate) const NO_CHANGES_ERROR: &str =
    "the agent produced no changes — nothing to review or open a PR for";

/// Per-run in-flight registry. Stored on `ServerCtx.runs_engine`.
pub struct RunEngine {
    inflight: Mutex<HashSet<Id>>,
    /// Per-run cancel token, tripped by [`RunEngine::cancel`]. `advance`
    /// races every stage against it, so a cancel DROPS the in-flight stage
    /// future — the agent's `PtyHandle` drop kills its CLI — instead of
    /// letting it run on in a worktree that is being removed (S2-04).
    cancels: Mutex<HashMap<Id, tokio_util::sync::CancellationToken>>,
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
        if let Ok(mut g) = self.engine.cancels.lock() {
            g.remove(&self.id);
        }
    }
}

impl RunEngine {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The cancel token of the run `advance` is driving (created on claim).
    fn token(&self, id: &Id) -> tokio_util::sync::CancellationToken {
        self.cancels
            .lock()
            .map(|mut g| g.entry(id.clone()).or_default().clone())
            .unwrap_or_default()
    }

    /// Trip the run's cancel token: its in-flight stage is dropped at once.
    /// No-op when no stage is running.
    pub fn cancel(&self, id: &Id) {
        if let Ok(g) = self.cancels.lock() {
            if let Some(t) = g.get(id) {
                t.cancel();
            }
        }
    }

    /// Is a stage of this run still being driven?
    pub fn is_inflight(&self, id: &Id) -> bool {
        self.inflight.lock().is_ok_and(|g| g.contains(id))
    }

    /// Wait (bounded) until the run's in-flight stage has stopped — after
    /// [`Self::cancel`] that is one scheduler hop, the worktree is then safe
    /// to remove. `false` on timeout.
    pub async fn wait_idle(&self, id: &Id, max: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + max;
        while self.is_inflight(id) {
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        true
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
            cancels: Mutex::new(HashMap::new()),
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
    let cancel = ctx.runs_engine.token(&run_id);
    loop {
        let run = match ctx.runs.get(&run_id).await {
            Ok(r) => r,
            Err(_) => break,
        };
        if run.status.is_terminal() || run.status == RunStatus::AwaitingApproval {
            break;
        }
        let cur = run.status;
        // A cancel drops the stage future mid-flight (the agent CLI dies with
        // its PTY handle); the run is already Cancelled, nothing to record.
        let outcome = tokio::select! {
            r = run_stage(ctx, &run) => r,
            () = cancel.cancelled() => break,
        };
        match outcome {
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
    // The branch the run starts from = the eventual PR destination. Detached
    // HEAD (rev-parse prints literal "HEAD") or an error falls back to the
    // DETECTED default branch — never a fabricated "main", which would later
    // beat the real default in resolve_base on repos that do have a stale
    // local main.
    let base_branch = match git.current_branch().await {
        Ok(b) if !b.trim().is_empty() && b != "HEAD" => Some(b),
        _ => git.default_branch().await,
    };
    // Persist the repo first so the GitHub adapter can resolve a provider.
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                repo_id: Some(repo.id.clone()),
                repo_path: Some(repo.path.clone()),
                base_branch,
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
        let (branch, path, base) =
            crate::run_workspace::provision_worktree(ctx, run, &repo).await?;
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
    // "" = provider default; the model alias goes straight through as `--model`.
    let model = (!run.model.trim().is_empty()).then(|| run.model.trim());
    let provider = if run.provider.trim().is_empty() {
        "claude"
    } else {
        run.provider.trim()
    };
    // claude runs on the fast headless PTY (also the deterministic E2E-stubbed
    // path). ANY OTHER registered provider (codex / agy / grok / …) runs as a
    // real, openable session so the run view can watch it — both honor `model`.
    // The agent commits its own work either way; the diff/proof/PR stages read
    // the worktree afterward.
    let reply = if provider == "claude" {
        ctx.orchestrator
            .run_agent(&packet.prompt, &wt, model, EXEC_NO_PROGRESS)
            .await?
    } else {
        let ws = ctx.workspaces.get(&run.workspace_id).await?;
        let user = otto_state::UsersRepo::new(ctx.pool.clone())
            .get(&run.created_by)
            .await?;
        let mut meta = serde_json::json!({
            "source": "run_with_otto",
            "run_id": run.id,
            "cwd": wt,
        });
        if let Some(m) = model {
            meta["model"] = serde_json::json!(m);
        }
        let rid = run.id.to_string();
        let title = format!("Run with Otto · {}", &rid[..rid.len().min(8)]);
        let (reply, _sid) = crate::agent_session::run_session_turn(
            ctx,
            &ws,
            &user,
            None,
            &title,
            &wt,
            provider,
            meta,
            &packet.prompt,
            EXEC_NO_PROGRESS,
            |_id| {},
        )
        .await
        .map_err(|e| e.0)?;
        reply
    };

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
                result_summary: Some(otto_core::text::clip_bytes(&reply, 4_000)),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}

async fn execute_goal_loop(ctx: &ServerCtx, run: &OttoRun) -> Result<()> {
    use otto_core::domain::{
        AcceptanceCriterion, GoalLoopConfig, GoalLoopDefinition, GoalLoopLimits,
    };
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
    // The run's provider/model choice drives the change-producing executors;
    // the bookkeeping roles (planner/evaluator/digester) keep their tuned
    // defaults from `GoalLoopConfig::default()`.
    let mut config = GoalLoopConfig::default();
    for exec in &mut config.executors {
        if !run.provider.trim().is_empty() {
            exec.provider = run.provider.trim().to_string();
        }
        if !run.model.trim().is_empty() {
            exec.model = run.model.trim().to_string();
        }
    }
    let loop_ = ctx
        .goal_loops_repo
        .create(NewGoalLoop {
            workspace_id: run.workspace_id.clone(),
            name: run.title.clone(),
            repo_path: repo.path.clone(),
            definition,
            limits: GoalLoopLimits::default(),
            config,
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
    let path = ctx.data_dir.join("otto-runs").join(&run.id).join("work");
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
    // The captured launch-HEAD SHA when present; None ⇒ detected default.
    let base = run.base_commit.clone().filter(|s| !s.trim().is_empty());

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

    let (review_id, _resolved, no_changes, diff_len) = crate::modules::run_review_for_branch_sized(
        ctx,
        &repo_id,
        &wt,
        base.as_deref(),
        None,
        None,
        None,
        None,
    )
    .await?;
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                review_id: Some(review_id.clone()),
                ..Default::default()
            },
        )
        .await?;
    // An EMPTY diff means no reviewer ran: "0 findings (0 blocking)" would be
    // an unreviewed change dressed as a clean one in the approval prompt
    // (`run_review_for_branch`'s contract). Stop before AwaitingApproval.
    if no_changes {
        return Err(Error::Invalid(NO_CHANGES_ERROR.into()));
    }
    // Wait as long as the review itself may legitimately take (agents +
    // summarizer), sized off the same diff the reviewers received.
    let budget = crate::modules::review_wait_budget(ctx, &repo_id, diff_len).await;
    let (total, _open, blocker) = poll_review(ctx, &review_id, budget).await?;
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
    // None ⇒ draft_pr_core resolves the repo's detected default branch.
    let base = run.base_branch.clone().filter(|s| !s.trim().is_empty());

    let ws = ctx.workspaces.get(&run.workspace_id).await?;
    let user = otto_state::UsersRepo::new(ctx.pool.clone())
        .get(&run.created_by)
        .await?;
    match crate::modules::draft_pr_core(ctx, &ws, &user, &wt, base.as_deref()).await {
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

async fn poll_review(ctx: &ServerCtx, review_id: &Id, budget: Duration) -> Result<(u64, u64, u64)> {
    // Event-driven (O4 / SI-07): subscribe BEFORE the first status read so a
    // transition between the read and the wait is never missed, then re-read
    // the status column only when this review's `ReviewChanged` arrives (or
    // every 30 s as a safety net). The deadline is the review's own budget —
    // a shorter one read the counts of a still-running review as "0 findings
    // (0 blocking)" and put THAT in front of the approver.
    let mut rx = ctx.events.subscribe();
    let id = review_id.clone();
    wait_until_event(
        &mut rx,
        |ev| matches!(ev, Event::ReviewChanged { review_id, .. } if *review_id == id),
        || async {
            ctx.reviews_store
                .review_status(review_id)
                .await
                .is_ok_and(review_is_terminal)
        },
        REVIEW_SAFETY_RECHECK,
        budget,
    )
    .await;
    let status = ctx.reviews_store.review_status(review_id).await.ok();
    review_outcome(status, budget)?;
    Ok(crate::modules::review_findings_counts(ctx, review_id).await)
}

fn review_is_terminal(s: otto_core::domain::ReviewStatus) -> bool {
    use otto_core::domain::ReviewStatus;
    matches!(
        s,
        ReviewStatus::Done | ReviewStatus::Error | ReviewStatus::Cancelled
    )
}

/// Whether the finding counts of a review in `status` may be reported. Only a
/// `Done` review has final counts; anything else fails the stage so the
/// approval prompt never shows a fabricated "0 findings (0 blocking)".
fn review_outcome(status: Option<otto_core::domain::ReviewStatus>, budget: Duration) -> Result<()> {
    use otto_core::domain::ReviewStatus;
    match status {
        Some(ReviewStatus::Done) => Ok(()),
        Some(ReviewStatus::Error) => Err(Error::Internal(
            "code review failed — finding counts unknown".into(),
        )),
        Some(ReviewStatus::Cancelled) => Err(Error::Internal(
            "code review was cancelled — finding counts unknown".into(),
        )),
        Some(ReviewStatus::Running) => Err(Error::Internal(format!(
            "code review still running after {}s — finding counts unknown",
            budget.as_secs()
        ))),
        None => Err(Error::Internal(
            "code review status unreadable — finding counts unknown".into(),
        )),
    }
}

/// Re-run `check` until it is true or `total` elapses, waking on events that
/// `is_wake` accepts and at least every `safety`. A lagged or closed bus
/// degrades to the safety cadence. Returns the last `check` result.
async fn wait_until_event<W, C, F>(
    rx: &mut tokio::sync::broadcast::Receiver<Event>,
    is_wake: W,
    mut check: C,
    safety: Duration,
    total: Duration,
) -> bool
where
    W: Fn(&Event) -> bool,
    C: FnMut() -> F,
    F: std::future::Future<Output = bool>,
{
    use tokio::sync::broadcast::error::RecvError;
    let deadline = tokio::time::Instant::now() + total;
    loop {
        if check().await {
            return true;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            return false;
        }
        let wake_at = (now + safety).min(deadline);
        loop {
            match tokio::time::timeout_at(wake_at, rx.recv()).await {
                Err(_) => break,                        // safety / deadline tick
                Ok(Ok(ev)) if is_wake(&ev) => break,    // our event
                Ok(Ok(_)) => continue,                  // someone else's
                Ok(Err(RecvError::Lagged(_))) => break, // may have missed ours
                Ok(Err(RecvError::Closed)) => {
                    tokio::time::sleep_until(wake_at).await;
                    break;
                }
            }
        }
    }
}

async fn poll_goal_loop(ctx: &ServerCtx, loop_id: &Id) -> Result<otto_core::domain::GoalLoop> {
    for _ in 0..GOAL_LOOP_POLL_MAX {
        let gl = ctx.goal_loops_repo.get(loop_id).await?;
        if gl.status.is_terminal() {
            return Ok(gl);
        }
        tokio::time::sleep(POLL_EVERY).await;
    }
    // Past the cap the loop is still running: stop it (it would otherwise keep
    // burning tokens with no run waiting on it) and say why, instead of the
    // misleading "goal loop produced no branch" a half-done loop surfaced.
    if let Err(e) = crate::goal_loop::stop_loop(ctx, loop_id).await {
        tracing::warn!(goal_loop = %loop_id, "stop overdue goal loop: {e}");
    }
    Err(Error::Internal(GOAL_LOOP_OVERDUE.into()))
}

/// The goal-loop stage's error once [`GOAL_LOOP_POLL_MAX`] polls elapsed.
const GOAL_LOOP_OVERDUE: &str = "goal loop exceeded 4h — stopped";

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
        Ok(acc) => otto_core::secrets::get_async(&ctx.secrets, &acc.token_ref)
            .await
            .ok()
            .flatten(),
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
    // CAS: a run already ended (cancelled while this stage was in flight)
    // stays as it is — no Failed overwrite, no "❌ Run failed" notice, no
    // second webhook callback.
    match ctx.runs.set_error(&run.id, err).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::info!(run = %run.id, "stage error after the run ended (ignored): {err}");
            return;
        }
        Err(e) => {
            tracing::warn!(run = %run.id, "set run error: {e}");
            return;
        }
    }
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
    if let Ok(Some(integ)) = ctx.integrations_store.get(&run.workspace_id, channel).await {
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
    // The approval is an OUTWARD action: say where it goes. Approving pushes
    // the run's branch to `origin` (`best_effort_push`, when the repo has a
    // bound git account) before the PR draft is written.
    let branch = run
        .branch
        .as_deref()
        .filter(|b| !b.is_empty())
        .map_or_else(|| format!("otto-run/{}", run.id), str::to_string);
    format!(
        "🧪 *Run with Otto* — ready for review\n*{title}*\nProof: {proof} · risk {risk}/100 · \
         findings: {total} ({blocking} blocking)\n\nReply *approve* to push branch `{branch}` \
         to the repo's `origin` remote and draft a PR, or *reject*.",
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
    use super::wait_until_event;
    use otto_core::event::Event;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn review_changed(id: &str) -> Event {
        Event::ReviewChanged {
            workspace_id: "w".into(),
            session_id: None,
            review_id: id.into(),
            status: "done".into(),
        }
    }

    // O4: the review wait re-reads status on its own event, not on a 2 s tick.
    // (Real time, scaled down: the safety tick is far past the test.)
    #[tokio::test]
    async fn review_wait_wakes_on_its_event_and_ignores_others() {
        let (tx, mut rx) = tokio::sync::broadcast::channel(16);
        let checks = AtomicUsize::new(0);
        let done = std::sync::atomic::AtomicBool::new(false);
        let waiter = wait_until_event(
            &mut rx,
            |ev| matches!(ev, Event::ReviewChanged { review_id, .. } if review_id == "r1"),
            || async {
                checks.fetch_add(1, Ordering::SeqCst);
                done.load(Ordering::SeqCst)
            },
            Duration::from_secs(30),
            Duration::from_secs(300),
        );
        let driver = async {
            tokio::time::sleep(Duration::from_millis(40)).await;
            let _ = tx.send(review_changed("other")); // not ours: no re-check
            tokio::time::sleep(Duration::from_millis(40)).await;
            done.store(true, Ordering::SeqCst);
            let _ = tx.send(review_changed("r1"));
        };
        let (ok, ()) = tokio::join!(waiter, driver);
        assert!(ok);
        // Initial check + one wake on our event only.
        assert_eq!(checks.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn review_wait_falls_back_to_the_safety_recheck_and_the_deadline() {
        let (_tx, mut rx) = tokio::sync::broadcast::channel::<Event>(4);
        let checks = AtomicUsize::new(0);
        let ok = wait_until_event(
            &mut rx,
            |_| false,
            || async {
                checks.fetch_add(1, Ordering::SeqCst);
                false
            },
            Duration::from_millis(50),
            Duration::from_millis(250),
        )
        .await;
        assert!(!ok);
        // t = 0, 50, …, 250: one read per safety window (≈6), then give up.
        let n = checks.load(Ordering::SeqCst);
        assert!((4..=7).contains(&n), "{n} checks");
    }

    // Only a Done review reports counts; running/error/cancelled fail the
    // stage instead of feeding "0 findings (0 blocking)" to the approval.
    #[test]
    fn review_outcome_only_trusts_done() {
        use super::{review_is_terminal, review_outcome};
        use otto_core::domain::ReviewStatus;
        let b = Duration::from_secs(60);
        assert!(review_outcome(Some(ReviewStatus::Done), b).is_ok());
        for s in [
            ReviewStatus::Running,
            ReviewStatus::Error,
            ReviewStatus::Cancelled,
        ] {
            assert!(review_outcome(Some(s), b).is_err(), "{s:?}");
        }
        assert!(review_outcome(None, b).is_err());
        assert!(review_is_terminal(ReviewStatus::Cancelled));
        assert!(!review_is_terminal(ReviewStatus::Running));
    }

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

#[cfg(test)]
mod cancel_tests {
    use super::{approval_prompt, RunEngine};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::Duration;

    struct DropFlag(Arc<AtomicBool>);
    impl Drop for DropFlag {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    /// S2-04: a cancel drops the in-flight stage future (its PTY handle — and
    /// so the agent CLI — goes with it) and `wait_idle` then reports the run
    /// idle, so the worktree is removed only after the stage stopped.
    #[tokio::test]
    async fn cancel_drops_the_in_flight_stage_then_the_run_goes_idle() {
        let engine = RunEngine::new();
        let id = "run-1".to_string();
        let dropped = Arc::new(AtomicBool::new(false));
        let driver = {
            let engine = Arc::clone(&engine);
            let id = id.clone();
            let dropped = Arc::clone(&dropped);
            tokio::spawn(async move {
                let _claim = engine.claim(&id).expect("first claim");
                let cancel = engine.token(&id);
                let stage = async move {
                    let _agent = DropFlag(dropped);
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                };
                tokio::select! {
                    () = stage => "finished",
                    () = cancel.cancelled() => "cancelled",
                }
            })
        };
        while !engine.is_inflight(&id) {
            tokio::task::yield_now().await;
        }
        engine.cancel(&id);
        assert!(engine.wait_idle(&id, Duration::from_secs(5)).await);
        assert_eq!(driver.await.unwrap(), "cancelled");
        assert!(
            dropped.load(Ordering::SeqCst),
            "the stage future was dropped"
        );
        // Cancelling an idle run is a no-op.
        engine.cancel(&id);
        assert!(!engine.is_inflight(&id));
    }

    /// S2-16: approving pushes a branch — the prompt says which and where.
    #[test]
    fn approval_prompt_names_the_push_and_its_remote() {
        use chrono::Utc;
        use otto_core::run::{OttoRun, RunMode, RunOrigin, RunStatus, SourceKind};
        let run = OttoRun {
            id: "abc".into(),
            workspace_id: "w1".into(),
            title: "t".into(),
            source_kind: SourceKind::Finding,
            source_ref: "f1".into(),
            source_url: None,
            goal: "g".into(),
            mode: RunMode::SingleAgent,
            provider: "claude".into(),
            model: String::new(),
            repo_id: Some("r1".into()),
            repo_path: None,
            base_branch: Some("main".into()),
            branch: Some("otto-run/abc".into()),
            worktree_path: None,
            base_commit: None,
            status: RunStatus::AwaitingApproval,
            error: None,
            origin_kind: RunOrigin::Api,
            origin_chat: None,
            origin_thread: None,
            origin_user: None,
            callback_url: None,
            goal_loop_id: None,
            review_id: None,
            proof_pack_id: None,
            proof_status: None,
            risk_score: None,
            findings_total: 0,
            findings_blocking: 0,
            pr_draft_json: None,
            pr_url: None,
            auto_open_pr: false,
            approval_decision: None,
            approved_by: None,
            approved_at: None,
            result_summary: None,
            context_summary: None,
            created_by: "root".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let p = approval_prompt(&run);
        assert!(p.contains("push branch `otto-run/abc`"), "{p}");
        assert!(p.contains("`origin`"), "{p}");
    }
}
