//! Leader goal-verification controller (design §3).
//!
//! After a dev finishes a task, the leader verifies each goal **sequentially**
//! (one per iteration): pass → next; a blocking miss → a fix request to the dev
//! with all the findings → re-verify (more thoroughly) up to `max_retries`; then a
//! near miss settles as a (non-blocking) **warning** and a far miss as **unmet**
//! ("could not be achieved"). When every goal is resolved with no blocking unmet,
//! the leader merges the task's worktree branch into the integration branch.
//!
//! The pure engine ([`run_verification`]) is decoupled from I/O via [`VerifyOps`]
//! so the loop logic is unit-testable with scripted verdicts. [`SwarmVerifyOps`]
//! wires it to the running daemon (leader/dev turns, merge, board, persistence).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use async_trait::async_trait;
use otto_core::domain::{User, Workspace};
use otto_core::event::Event;
use otto_state::{GoalPatch, SwarmAgent, SwarmGoal, SwarmProject, SwarmTask, TaskPatch};
use serde_json::{json, Value};

use crate::swarm_agent_run::CancelState;
use crate::state::ServerCtx;

// ===========================================================================
// Pure engine
// ===========================================================================

/// The leader's structured judgment of one goal (parsed from its verify turn).
#[derive(Debug, Clone, Default)]
pub struct Verdict {
    /// Did the goal's PRIMARY objective get achieved? (Nits → still `true`.)
    pub target_met: bool,
    /// Only meaningful when `!target_met`: is the miss bad (block) or close (warn)?
    pub blocker: bool,
    pub severity: String,
    pub measured: Option<String>,
    pub summary: String,
    pub findings: Vec<Value>,
}

impl Verdict {
    /// Parse a verdict from a leader reply (expects one JSON object).
    pub fn parse(reply: &str) -> Option<Verdict> {
        let v = otto_swarm::recruiter::extract_json(reply)?;
        Some(Verdict {
            target_met: v.get("target_met").and_then(|b| b.as_bool()).unwrap_or(false),
            blocker: v.get("blocker").and_then(|b| b.as_bool()).unwrap_or(true),
            severity: v.get("severity").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            measured: v.get("measured").and_then(|s| s.as_str()).map(str::to_string),
            summary: v.get("summary").and_then(|s| s.as_str()).unwrap_or("").to_string(),
            findings: v.get("findings").and_then(|f| f.as_array()).cloned().unwrap_or_default(),
        })
    }
    fn to_json(&self) -> Value {
        json!({
            "target_met": self.target_met, "blocker": self.blocker, "severity": self.severity,
            "measured": self.measured, "summary": self.summary, "findings": self.findings,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Pass,
    Warned,
    RequestFix,
    Unmet,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixOutcome {
    Completed,
    Failed,
}

/// The single decision table (design §3.3; review Gap A + M5 + M6).
/// `iterations` = fix attempts taken so far on this goal.
pub fn classify(goal: &SwarmGoal, v: &Verdict, iterations: i64) -> Decision {
    if v.target_met {
        return Decision::Pass; // includes pass-with-nits — terminal success
    }
    if !goal.blocking {
        return Decision::Warned; // advisory goal: never fix-loop, never block merge
    }
    if iterations < goal.max_retries {
        return Decision::RequestFix; // blocking + unmet: push the dev toward the target
    }
    // Retries exhausted: a near miss warns (non-blocking), a far miss blocks.
    if v.blocker {
        Decision::Unmet
    } else {
        Decision::Warned
    }
}

/// Outcome of one goal in a verification run (for the summary + final task status).
#[derive(Debug, Clone)]
pub struct GoalResult {
    pub goal_id: String,
    pub status: String,
}

#[derive(Debug, Clone, Default)]
pub struct VerifySummary {
    pub cancelled: bool,
    /// A blocking goal ended `unmet` → the worktree must NOT be merged.
    pub blocked: bool,
    pub results: Vec<GoalResult>,
    pub merge_status: Option<String>,
}

/// I/O the engine performs, injected so the loop is unit-testable.
#[async_trait]
pub trait VerifyOps: Send + Sync {
    /// Run the leader verify turn for `goal` at the given scrutiny level.
    /// `None` = the turn failed to produce a verdict.
    async fn verify_goal(&self, goal: &SwarmGoal, scrutiny: u32) -> Option<Verdict>;
    /// Run the dev fix turn with all the findings; blocks until it completes.
    async fn request_fix(&self, goal: &SwarmGoal, v: &Verdict) -> FixOutcome;
    /// Merge the task's worktree branch into the integration branch. Returns a
    /// status string ("merged" | "conflicts" | "up_to_date" | "error" | "skipped").
    async fn merge_back(&self) -> String;
    /// Persist a goal's terminal status + iterations + last verdict.
    async fn record(&self, goal: &SwarmGoal, status: &str, iterations: i64, v: Option<&Verdict>);
    /// Persist just the iteration counter (after a completed fix).
    async fn record_iterations(&self, goal: &SwarmGoal, iterations: i64);
    /// Post the verdict to the board feed.
    async fn post_verdict(&self, goal: &SwarmGoal, v: &Verdict);
    /// Escalate a goal that could not be achieved (feed + channel).
    async fn escalate(&self, goal: &SwarmGoal, reason: &str);
    /// Abort/stop requested?
    fn cancelled(&self) -> bool;
    /// Swarm over budget / paused?
    async fn over_budget(&self) -> bool;
}

/// The pure sequential per-goal loop.
pub async fn run_verification(goals: Vec<SwarmGoal>, ops: &dyn VerifyOps) -> VerifySummary {
    let mut summary = VerifySummary::default();
    for goal in &goals {
        // Resume from persisted iterations so a restart continues mid-sequence.
        let mut iterations = goal.iterations;
        let mut scrutiny = (iterations + 1) as u32;
        let status: &str = loop {
            if ops.cancelled() {
                summary.cancelled = true;
                return summary;
            }
            let Some(v) = ops.verify_goal(goal, scrutiny).await else {
                ops.record(goal, "error", iterations, None).await;
                break "error";
            };
            ops.post_verdict(goal, &v).await;
            match classify(goal, &v, iterations) {
                Decision::Pass => {
                    ops.record(goal, "passed", iterations, Some(&v)).await;
                    break "passed";
                }
                Decision::Warned => {
                    ops.record(goal, "warned", iterations, Some(&v)).await;
                    break "warned";
                }
                Decision::Unmet => {
                    ops.record(goal, "unmet", iterations, Some(&v)).await;
                    ops.escalate(goal, "goal could not be achieved").await;
                    summary.blocked = true;
                    break "unmet";
                }
                Decision::RequestFix => {
                    // Once merge is already impossible, verify-once-for-the-report but
                    // don't spend fix budget on further blocking goals (review m5).
                    if summary.blocked {
                        ops.record(goal, "unmet", iterations, Some(&v)).await;
                        break "unmet";
                    }
                    if ops.over_budget().await {
                        ops.record(goal, "unmet", iterations, Some(&v)).await;
                        ops.escalate(goal, "budget exhausted before the goal was met").await;
                        summary.blocked = true;
                        break "unmet";
                    }
                    match ops.request_fix(goal, &v).await {
                        FixOutcome::Completed => {
                            iterations += 1;
                            scrutiny += 1;
                            ops.record_iterations(goal, iterations).await;
                        }
                        FixOutcome::Failed => {
                            ops.record(goal, "unmet", iterations, Some(&v)).await;
                            ops.escalate(goal, "could not run the fix turn").await;
                            summary.blocked = true;
                            break "unmet";
                        }
                    }
                }
            }
        };
        summary.results.push(GoalResult { goal_id: goal.id.clone(), status: status.to_string() });
    }
    // Merge only when no blocking goal is unmet.
    if !summary.cancelled && !summary.blocked {
        summary.merge_status = Some(ops.merge_back().await);
    }
    summary
}

// ===========================================================================
// Controller registry (cancel + dev-agent lock + double-spawn guard)
// ===========================================================================

struct Handle {
    cancel: CancelState,
    dev_agent_id: String,
    swarm_id: String,
}

static REGISTRY: OnceLock<Mutex<HashMap<String, Handle>>> = OnceLock::new();

fn registry() -> &'static Mutex<HashMap<String, Handle>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// True if `agent_id` is the dev whose branch is currently under verification —
/// the coordinator must not start another of its tasks (would pollute the branch
/// the leader is verifying / about to merge; review B1).
pub fn agent_under_verification(agent_id: &str) -> bool {
    registry()
        .lock()
        .unwrap()
        .values()
        .any(|h| h.dev_agent_id == agent_id)
}

/// True if a verification controller is already running for `task_id`.
pub fn is_verifying(task_id: &str) -> bool {
    registry().lock().unwrap().contains_key(task_id)
}

/// Stop every in-flight verification for a swarm (Abort): flag + kill sessions.
pub async fn stop_swarm(ctx: &ServerCtx, swarm_id: &str) {
    let handles: Vec<CancelState> = {
        let map = registry().lock().unwrap();
        map.values().filter(|h| h.swarm_id == swarm_id).map(|h| h.cancel.clone()).collect()
    };
    for cs in handles {
        cs.signal();
        cs.kill_tracked(ctx).await;
    }
}

/// Stop a single task's verification.
pub async fn stop_task(ctx: &ServerCtx, task_id: &str) {
    let cs = registry().lock().unwrap().get(task_id).map(|h| h.cancel.clone());
    if let Some(cs) = cs {
        cs.signal();
        cs.kill_tracked(ctx).await;
    }
}

/// RAII guard: removes the registry entry when the controller exits (incl. panic).
struct RegistryGuard(String);
impl Drop for RegistryGuard {
    fn drop(&mut self) {
        registry().lock().unwrap().remove(&self.0);
    }
}

// ===========================================================================
// Goal-set assembly
// ===========================================================================

/// Build the task's goal set: its explicit goals, plus task-scoped COPIES of the
/// project goals and the swarm standing goals (created once, idempotent by title)
/// so each is tracked + resumable per task. Returns the ordered goal rows.
pub async fn assemble_task_goals(ctx: &ServerCtx, task: &SwarmTask) -> Vec<SwarmGoal> {
    let repo = &ctx.swarm_repo;
    let mut existing = repo.list_goals_for_task(&task.id).await.unwrap_or_default();
    let have: std::collections::HashSet<String> =
        existing.iter().map(|g| g.title.to_lowercase()).collect();

    // Lazily seed swarm standing goals if none exist yet (also covers old swarms).
    ensure_standing_goals(ctx, &task.swarm_id, &task.workspace_id, &task.created_by).await;

    let mut templates: Vec<SwarmGoal> = Vec::new();
    templates.extend(repo.list_goals_for_project(&task.project_id).await.unwrap_or_default());
    templates.extend(repo.list_standing_goals(&task.swarm_id).await.unwrap_or_default());

    for t in templates {
        if have.contains(&t.title.to_lowercase()) {
            continue;
        }
        if let Ok(g) = repo
            .create_goal(otto_state::NewGoal {
                swarm_id: task.swarm_id.clone(),
                workspace_id: task.workspace_id.clone(),
                project_id: Some(task.project_id.clone()),
                task_id: Some(task.id.clone()),
                kind: t.kind.clone(),
                title: t.title.clone(),
                description: t.description.clone(),
                metric: t.metric.clone(),
                comparator: t.comparator.clone(),
                target_value: t.target_value,
                block_value: t.block_value,
                verify_cmd: t.verify_cmd.clone(),
                max_retries: t.max_retries,
                blocking: t.blocking,
                order_idx: t.order_idx + 1000, // standing/project goals after explicit ones
                created_by: task.created_by.clone(),
            })
            .await
        {
            existing.push(g);
        }
    }
    // Order: explicit (kind='explicit') first by order_idx, then the rest.
    existing.sort_by(|a, b| {
        let ka = (a.kind != "explicit", a.order_idx, a.created_at);
        let kb = (b.kind != "explicit", b.order_idx, b.created_at);
        ka.cmp(&kb)
    });
    existing
}

/// Default standing (always-applied) goals — the "unwritten goals" every code
/// task should follow. Seeded once per swarm.
pub const STANDING_GOALS: &[(&str, &str)] = &[
    ("Reuse existing flows", "Reuse existing flows/utilities instead of generating new ones; prefer extending what's already there."),
    ("No code duplication", "No copy-pasted/duplicated logic; shared logic is factored into one place."),
    ("Follow coding standards", "Follow the project's coding standards, conventions and patterns (match the surrounding code)."),
    ("Use the assigned skill set", "Apply the assigned skills (especially must-use ones) where they're relevant."),
];

/// Ensure the swarm has its standing-goal templates (idempotent).
pub async fn ensure_standing_goals(ctx: &ServerCtx, swarm_id: &str, workspace_id: &str, created_by: &str) {
    let repo = &ctx.swarm_repo;
    let existing = repo.list_standing_goals(&swarm_id.to_string()).await.unwrap_or_default();
    if !existing.is_empty() {
        return;
    }
    for (i, (title, desc)) in STANDING_GOALS.iter().enumerate() {
        let _ = repo
            .create_goal(otto_state::NewGoal {
                swarm_id: swarm_id.to_string(),
                workspace_id: workspace_id.to_string(),
                project_id: None,
                task_id: None,
                kind: "standing".into(),
                title: title.to_string(),
                description: desc.to_string(),
                metric: None,
                comparator: None,
                target_value: None,
                block_value: None,
                verify_cmd: None,
                max_retries: 3,
                blocking: true,
                order_idx: i as i64,
                created_by: created_by.to_string(),
            })
            .await;
    }
}

/// Does this task have any goals to verify (explicit, project, or standing)?
pub async fn task_has_goals(ctx: &ServerCtx, task: &SwarmTask) -> bool {
    let repo = &ctx.swarm_repo;
    if !repo.list_goals_for_task(&task.id).await.unwrap_or_default().is_empty() {
        return true;
    }
    if !repo.list_goals_for_project(&task.project_id).await.unwrap_or_default().is_empty() {
        return true;
    }
    !repo.list_standing_goals(&task.swarm_id).await.unwrap_or_default().is_empty()
}

// ===========================================================================
// Leader resolution
// ===========================================================================

/// Resolve the leader who verifies the dev's work: the dev's manager (walk
/// `reports_to` up to an agent that has reports), else the swarm root, else the
/// dev itself (self-verification — acceptable; review m3).
pub async fn resolve_leader(ctx: &ServerCtx, swarm_id: &str, dev_agent_id: &str) -> Option<SwarmAgent> {
    let agents = ctx.swarm_repo.list_agents(&swarm_id.to_string()).await.ok()?;
    let by_id = |id: &str| agents.iter().find(|a| a.id == id).cloned();
    let has_reports = |id: &str| agents.iter().any(|a| a.reports_to.as_deref() == Some(id));

    // The dev's direct manager, if any.
    if let Some(mgr_id) = by_id(dev_agent_id).and_then(|a| a.reports_to) {
        if let Some(mgr) = by_id(&mgr_id) {
            return Some(mgr);
        }
    }
    // Fallback: an agent that has reports (a leader), else the root, else the dev
    // itself (self-verification — acceptable when the dev is the top-level agent).
    agents
        .iter()
        .find(|a| has_reports(&a.id) && a.id != dev_agent_id)
        .cloned()
        .or_else(|| agents.iter().find(|a| a.reports_to.is_none()).cloned())
        .or_else(|| by_id(dev_agent_id))
}

// ===========================================================================
// Real ops wiring
// ===========================================================================

pub struct SwarmVerifyOps {
    ctx: ServerCtx,
    ws: Workspace,
    user: User,
    swarm: otto_state::Swarm,
    project: SwarmProject,
    task: SwarmTask,
    leader: SwarmAgent,
    dev: SwarmAgent,
    /// The dev's worktree (where verify + fix turns run).
    cwd: String,
    /// The dev's task branch (merged on success).
    agent_branch: String,
    /// The integration branch (base for diffs + merge target).
    integration_branch: String,
    cancel: CancelState,
}

#[async_trait]
impl VerifyOps for SwarmVerifyOps {
    async fn verify_goal(&self, goal: &SwarmGoal, scrutiny: u32) -> Option<Verdict> {
        let prompt = verify_prompt(goal, &self.integration_branch, scrutiny);
        let title = format!("Verify: {} · {}", clip(&goal.title, 40), clip(&self.task.title, 30));
        let (raw, _rid) = crate::swarm_agent_run::run_swarm_agent(
            &self.ctx,
            &self.ws,
            &self.user,
            &self.swarm.id,
            Some(&self.project.id),
            Some(&self.task.id),
            &self.leader.id,
            &self.leader.provider,
            "verify",
            &title,
            &self.cwd,
            &prompt,
            |t| otto_swarm::recruiter::extract_json(t).is_some(),
            &self.cancel,
        )
        .await;
        raw.and_then(|r| Verdict::parse(&r))
    }

    async fn request_fix(&self, goal: &SwarmGoal, v: &Verdict) -> FixOutcome {
        let prompt = fix_prompt(goal, v);
        let title = format!("Fix: {} · {}", clip(&goal.title, 40), clip(&self.task.title, 30));
        let (raw, _rid) = crate::swarm_agent_run::run_swarm_agent(
            &self.ctx,
            &self.ws,
            &self.user,
            &self.swarm.id,
            Some(&self.project.id),
            Some(&self.task.id),
            &self.dev.id,
            &self.dev.provider,
            "fix",
            &title,
            &self.cwd,
            &prompt,
            |t| otto_swarm::recruiter::extract_json(t).is_some(),
            &self.cancel,
        )
        .await;
        if raw.is_some() {
            FixOutcome::Completed
        } else {
            FixOutcome::Failed
        }
    }

    async fn merge_back(&self) -> String {
        let outcome =
            crate::swarm_merge::merge_task_branch(&self.ctx, &self.swarm, &self.project, &self.agent_branch).await;
        let body = match outcome.status.as_str() {
            "merged" => format!("✅ merged `{}` → `{}`.", self.agent_branch, outcome.integration_branch),
            "up_to_date" => format!("`{}` already integrated into `{}`.", self.agent_branch, outcome.integration_branch),
            "conflicts" => format!(
                "❌ merge conflict merging `{}` → `{}` on {} file(s): {}. A fix task was created.",
                self.agent_branch,
                outcome.integration_branch,
                outcome.conflicted_files.len(),
                outcome.conflicted_files.iter().take(8).map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ")
            ),
            _ => format!("⚠️ merge of `{}` did not complete: {}", self.agent_branch, outcome.note.clone().unwrap_or_default()),
        };
        let kind = if outcome.status == "conflicts" || outcome.status == "error" { "concern" } else { "merge" };
        crate::swarm_runtime::system_post_meta(
            &self.ctx,
            &self.swarm.id,
            Some(&self.project.id),
            Some(&self.task.id),
            kind,
            &body,
            json!({ "event": "merge", "status": outcome.status, "branch": self.agent_branch,
                    "integration_branch": outcome.integration_branch, "files": outcome.conflicted_files }),
        )
        .await;
        // On conflict, create a fix task for the dev to resolve.
        if outcome.status == "conflicts" {
            let _ = self
                .ctx
                .swarm_repo
                .create_task(otto_state::swarm::NewTask {
                    project_id: self.project.id.clone(),
                    swarm_id: self.swarm.id.clone(),
                    workspace_id: self.swarm.workspace_id.clone(),
                    title: format!("Resolve merge conflict: {}", clip(&self.task.title, 50)),
                    description: format!(
                        "Merging `{}` into `{}` conflicts on: {}. Resolve the conflicts so the work can integrate.",
                        self.agent_branch,
                        outcome.integration_branch,
                        outcome.conflicted_files.join(", ")
                    ),
                    assignee_agent_id: Some(self.dev.id.clone()),
                    status: "todo".into(),
                    priority: "high".into(),
                    parent_task_id: Some(self.task.id.clone()),
                    depends_on: json!([]),
                    labels: json!(["merge-conflict"]),
                    order_idx: 0,
                    created_by: self.task.created_by.clone(),
                })
                .await;
        }
        outcome.status
    }

    async fn record(&self, goal: &SwarmGoal, status: &str, iterations: i64, v: Option<&Verdict>) {
        let _ = self
            .ctx
            .swarm_repo
            .update_goal(
                &goal.id,
                GoalPatch {
                    status: Some(status.to_string()),
                    iterations: Some(iterations),
                    verdict: v.map(|v| Some(v.to_json())),
                    ..Default::default()
                },
            )
            .await;
        self.emit_goal(&goal.id).await;
    }

    async fn record_iterations(&self, goal: &SwarmGoal, iterations: i64) {
        let _ = self
            .ctx
            .swarm_repo
            .update_goal(
                &goal.id,
                GoalPatch { status: Some("verifying".into()), iterations: Some(iterations), ..Default::default() },
            )
            .await;
        self.emit_goal(&goal.id).await;
    }

    async fn post_verdict(&self, goal: &SwarmGoal, v: &Verdict) {
        let icon = if v.target_met { "🔎✅" } else if v.blocker { "🔎❌" } else { "🔎⚠️" };
        let measured = v.measured.as_deref().map(|m| format!(" (measured: {m})")).unwrap_or_default();
        let body = format!("{icon} Goal “{}”{}: {}", goal.title, measured, clip(&v.summary, 240));
        crate::swarm_runtime::system_post_meta(
            &self.ctx,
            &self.swarm.id,
            Some(&self.project.id),
            Some(&self.task.id),
            "verify",
            &body,
            json!({ "event": "verify", "goal_id": goal.id, "verdict": v.to_json() }),
        )
        .await;
    }

    async fn escalate(&self, goal: &SwarmGoal, reason: &str) {
        let body = format!("🚫 Goal “{}” {} (after {} attempt(s)).", goal.title, reason, goal.max_retries);
        crate::swarm_runtime::system_post_meta(
            &self.ctx,
            &self.swarm.id,
            Some(&self.project.id),
            Some(&self.task.id),
            "escalation",
            &body,
            json!({ "event": "goal_unmet", "goal_id": goal.id, "reason": reason }),
        )
        .await;
        let _ = self.ctx.events.send(Event::Notice {
            level: "warn".into(),
            title: "Swarm goal could not be achieved".into(),
            body: clip(&body, 160),
        });
        // Reply back to the originating channel, if the project was channel-launched.
        crate::swarm_channels::notify_origin(&self.ctx, &self.project, &body).await;
    }

    fn cancelled(&self) -> bool {
        self.cancel.cancelled()
    }

    async fn over_budget(&self) -> bool {
        crate::swarm_runtime::is_over_budget(&self.ctx, &self.swarm.id).await
    }
}

impl SwarmVerifyOps {
    async fn emit_goal(&self, goal_id: &str) {
        if let Ok(g) = self.ctx.swarm_repo.get_goal(&goal_id.to_string()).await {
            let _ = self.ctx.events.send(Event::SwarmGoalUpdated {
                workspace_id: self.swarm.workspace_id.clone(),
                swarm_id: self.swarm.id.clone(),
                task_id: Some(self.task.id.clone()),
                goal: serde_json::to_value(&g).unwrap_or_default(),
            });
        }
    }
}

// ===========================================================================
// Controller entry point + recovery
// ===========================================================================

/// Start verifying a task's goals (idempotent — a no-op if one is already running).
/// Spawns a background controller. `dev_agent_id` is the agent that did the work.
pub fn start_verification(ctx: &ServerCtx, task: SwarmTask, dev_agent_id: String) {
    // Test-and-set: register a handle, or bail if one already runs for this task.
    let cancel = CancelState::detached();
    {
        let mut map = registry().lock().unwrap();
        if map.contains_key(&task.id) {
            return;
        }
        map.insert(
            task.id.clone(),
            Handle { cancel: cancel.clone(), dev_agent_id: dev_agent_id.clone(), swarm_id: task.swarm_id.clone() },
        );
    }
    let ctx = ctx.clone();
    tokio::spawn(async move {
        let _guard = RegistryGuard(task.id.clone());
        if let Err(e) = run_controller(&ctx, &task, &dev_agent_id, cancel).await {
            tracing::warn!(task = %task.id, "swarm verification controller error: {e}");
            // Don't strand the task in `verifying`.
            let _ = ctx
                .swarm_repo
                .update_task(&task.id, TaskPatch { status: Some("blocked".into()), ..Default::default() })
                .await;
            crate::swarm_runtime::emit_task_pub(&ctx, &task.id).await;
        }
    });
}

async fn run_controller(
    ctx: &ServerCtx,
    task: &SwarmTask,
    dev_agent_id: &str,
    cancel: CancelState,
) -> otto_core::Result<()> {
    let repo = &ctx.swarm_repo;
    let swarm = repo.get_swarm(&task.swarm_id).await?;
    let project = repo.get_project(&task.project_id).await?;
    let ws = ctx.workspaces.get(&swarm.workspace_id).await?;
    let dev = repo.get_agent(&dev_agent_id.to_string()).await?;
    let leader = resolve_leader(ctx, &swarm.id, dev_agent_id).await.unwrap_or_else(|| dev.clone());

    // The dev's worktree + branch (already created during the dev's turn).
    let cwd_info = crate::swarm_workspace::ensure_cwd_info(ctx, &swarm, &dev, Some(&project)).await?;
    let (agent_branch, integration_branch) = match (cwd_info.branch.clone(), cwd_info.integration_branch.clone()) {
        (Some(b), Some(i)) => (b, i),
        _ => {
            // No worktree (scratch/repo mode) — nothing to verify-and-merge; just complete.
            repo.update_task(&task.id, TaskPatch { status: Some("done".into()), ..Default::default() }).await?;
            crate::swarm_runtime::emit_task_pub(ctx, &task.id).await;
            return Ok(());
        }
    };

    let goals = assemble_task_goals(ctx, task).await;
    if goals.is_empty() {
        repo.update_task(&task.id, TaskPatch { status: Some("done".into()), ..Default::default() }).await?;
        crate::swarm_runtime::emit_task_pub(ctx, &task.id).await;
        return Ok(());
    }

    let user = User {
        id: swarm.created_by.clone(),
        username: "swarm".into(),
        display_name: "Swarm".into(),
        is_root: false,
        disabled: false,
        created_at: chrono::Utc::now(),
    };
    let ops = SwarmVerifyOps {
        ctx: ctx.clone(),
        ws,
        user,
        swarm: swarm.clone(),
        project: project.clone(),
        task: task.clone(),
        leader,
        dev,
        cwd: cwd_info.path.clone(),
        agent_branch,
        integration_branch,
        cancel,
    };

    let summary = run_verification(goals, &ops).await;

    // Final task status + summary post.
    let final_status = if summary.cancelled {
        // Leave as-is on cancel; abort path handles status.
        return Ok(());
    } else if summary.blocked {
        "blocked"
    } else {
        "done"
    };
    repo.update_task(&task.id, TaskPatch { status: Some(final_status.into()), ..Default::default() }).await?;
    crate::swarm_runtime::emit_task_pub(ctx, &task.id).await;

    let passed = summary.results.iter().filter(|r| r.status == "passed").count();
    let warned = summary.results.iter().filter(|r| r.status == "warned").count();
    let unmet = summary.results.iter().filter(|r| r.status == "unmet").count();
    let merge_note = summary
        .merge_status
        .as_deref()
        .map(|s| format!(" · merge: {s}"))
        .unwrap_or_default();
    let body = format!(
        "Goal verification for “{}” complete: {passed} passed, {warned} warned, {unmet} unmet{merge_note}.",
        task.title
    );
    crate::swarm_runtime::system_post_meta(
        ctx,
        &swarm.id,
        Some(&task.project_id),
        Some(&task.id),
        "status",
        &body,
        json!({ "event": "verification_complete", "passed": passed, "warned": warned, "unmet": unmet,
                "merge": summary.merge_status }),
    )
    .await;
    crate::swarm_channels::notify_origin(ctx, &project, &body).await;
    Ok(())
}

/// On coordinator (re)start: re-spawn controllers for tasks left in `verifying`
/// (e.g. after a daemon restart) so they aren't stranded (review B2). The trigger
/// persists the dev as the task's `assignee_agent_id`, so recovery reads it there.
pub async fn recover(ctx: &ServerCtx, swarm_id: &str) {
    let tasks = ctx.swarm_repo.list_tasks_for_swarm(&swarm_id.to_string()).await.unwrap_or_default();
    for task in tasks {
        if task.status != "verifying" || is_verifying(&task.id) {
            continue;
        }
        let Some(dev_agent_id) = task.assignee_agent_id.clone() else {
            tracing::warn!(task = %task.id, "swarm: stranded verifying task has no assignee; marking blocked");
            let _ = ctx
                .swarm_repo
                .update_task(&task.id, TaskPatch { status: Some("blocked".into()), ..Default::default() })
                .await;
            continue;
        };
        tracing::info!(task = %task.id, "swarm: recovering stranded verification");
        start_verification(ctx, task, dev_agent_id);
    }
}

// ===========================================================================
// Prompts
// ===========================================================================

fn verify_prompt(goal: &SwarmGoal, base_branch: &str, scrutiny: u32) -> String {
    let mut s = String::new();
    s.push_str("You are the LEADER verifying ONE goal of a teammate's completed work, in their git worktree (your CWD).\n\n");
    s.push_str(&format!("## Goal\n**{}**\n{}\n\n", goal.title, goal.description));
    if let Some(metric) = &goal.metric {
        s.push_str(&format!("Metric: `{metric}`"));
        if let Some(t) = goal.target_value {
            s.push_str(&format!(" — target {} {}", goal.comparator.clone().unwrap_or_else(|| "lte".into()), t));
        }
        if let Some(b) = goal.block_value {
            s.push_str(&format!(" (block threshold: {b})"));
        }
        s.push('\n');
    }
    if let Some(cmd) = &goal.verify_cmd {
        s.push_str(&format!("Ground-truth by RUNNING this command in the CWD and reading its output/timing:\n```\n{cmd}\n```\n"));
    }
    s.push_str(&format!(
        "\n## How to inspect\nReview the diff of this work: `git diff {base_branch}...HEAD` and read the changed files.\n"
    ));
    if scrutiny > 1 {
        s.push_str(&format!(
            "\n⚠️ A PRIOR pass on THIS goal FAILED (this is attempt #{scrutiny}). Re-examine the WHOLE goal from \
             scratch — re-run the verify command, re-read the full diff, and look for ANYTHING the earlier pass \
             missed, not only the previously-reported items.\n"
        ));
    }
    s.push_str(
        "\n## What matters\nJudge the goal's PRIMARY objective. Focus on IMPORTANT problems — correctness, the \
         metric/target, real duplication, broken standards. Petty/stylistic nits (a variable you'd name \
         differently) are NEVER blockers: if the only issues are nits, set `target_met:true` and list them as notes. \
         For a metric goal: if measured is within the target → `target_met:true`; close-but-over (between target and \
         the block threshold) → `target_met:false, blocker:false` (a warning to push toward); far over (past the \
         block threshold) → `target_met:false, blocker:true`. Example: target \"under 2 min\" — a 2.5 min result is \
         a warning, 7 min is a blocker. You decide.\n",
    );
    s.push_str(
        "\n## Output\nReply with EXACTLY ONE ```json block and nothing else:\n```json\n\
         {\"target_met\": true|false, \"blocker\": true|false, \"severity\": \"none|minor|major|blocker\", \
         \"measured\": \"<value or null>\", \"summary\": \"one line\", \
         \"findings\": [{\"severity\":\"...\",\"title\":\"...\",\"detail\":\"...\",\"file\":\"path:line\",\"fix\":\"what to change\"}]}\n```\n",
    );
    s
}

fn fix_prompt(goal: &SwarmGoal, v: &Verdict) -> String {
    let mut s = String::new();
    s.push_str("Your work did NOT yet meet a goal. Fix it in THIS worktree (your CWD), commit your changes, then report done.\n\n");
    s.push_str(&format!("## Goal not met\n**{}**\n{}\n\n", goal.title, goal.description));
    if !v.summary.is_empty() {
        s.push_str(&format!("Leader's summary: {}\n\n", v.summary));
    }
    if !v.findings.is_empty() {
        s.push_str("## Issues to fix (all of them)\n");
        for (i, f) in v.findings.iter().enumerate() {
            let title = f.get("title").and_then(|x| x.as_str()).unwrap_or("");
            let detail = f.get("detail").and_then(|x| x.as_str()).unwrap_or("");
            let file = f.get("file").and_then(|x| x.as_str()).unwrap_or("");
            let fix = f.get("fix").and_then(|x| x.as_str()).unwrap_or("");
            s.push_str(&format!("{}. {} — {} {}\n   fix: {}\n", i + 1, title, detail, if file.is_empty() { String::new() } else { format!("({file})") }, fix));
        }
        s.push('\n');
    }
    s.push_str(
        "Make the real change (don't just claim it). Reuse existing flows, avoid duplication, follow the project's \
         standards. When finished, COMMIT in this worktree and reply with EXACTLY ONE ```json block:\n\
         ```json\n{\"done\": true, \"summary\": \"what you changed\"}\n```\n",
    );
    s
}

pub(crate) fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let t: String = s.chars().take(n).collect();
        format!("{t}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Mutex as StdMutex;

    fn goal(id: &str, blocking: bool, max_retries: i64) -> SwarmGoal {
        let now = chrono::Utc::now();
        SwarmGoal {
            id: id.into(),
            swarm_id: "s".into(),
            workspace_id: "w".into(),
            project_id: Some("p".into()),
            task_id: Some("t".into()),
            kind: "explicit".into(),
            title: id.into(),
            description: String::new(),
            metric: None,
            comparator: None,
            target_value: None,
            block_value: None,
            verify_cmd: None,
            max_retries,
            blocking,
            status: "pending".into(),
            verdict: None,
            iterations: 0,
            order_idx: 0,
            created_by: "u".into(),
            created_at: now,
            updated_at: now,
        }
    }

    fn vd(target_met: bool, blocker: bool) -> Verdict {
        Verdict { target_met, blocker, ..Default::default() }
    }

    /// Scripted ops: per-goal queue of verdicts, a fixed fix outcome, and a log of
    /// what the engine drove.
    struct MockOps {
        verdicts: StdMutex<std::collections::HashMap<String, VecDeque<Verdict>>>,
        fix: FixOutcome,
        cancel_after_first_verify: bool,
        cancel: AtomicBool,
        over_budget: bool,
        log: StdMutex<Vec<String>>,
        scrutiny: StdMutex<Vec<(String, u32)>>,
    }
    impl MockOps {
        fn new(fix: FixOutcome) -> Self {
            Self {
                verdicts: StdMutex::new(std::collections::HashMap::new()),
                fix,
                cancel_after_first_verify: false,
                cancel: AtomicBool::new(false),
                over_budget: false,
                log: StdMutex::new(Vec::new()),
                scrutiny: StdMutex::new(Vec::new()),
            }
        }
        fn script(self, id: &str, vds: Vec<Verdict>) -> Self {
            self.verdicts.lock().unwrap().insert(id.into(), vds.into());
            self
        }
        fn logged(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }
    #[async_trait]
    impl VerifyOps for MockOps {
        async fn verify_goal(&self, g: &SwarmGoal, scrutiny: u32) -> Option<Verdict> {
            self.scrutiny.lock().unwrap().push((g.id.clone(), scrutiny));
            self.log.lock().unwrap().push(format!("verify:{}:{}", g.id, scrutiny));
            if self.cancel_after_first_verify {
                self.cancel.store(true, Ordering::Relaxed);
            }
            let mut q = self.verdicts.lock().unwrap();
            q.get_mut(&g.id).and_then(|d| d.pop_front())
        }
        async fn request_fix(&self, g: &SwarmGoal, _v: &Verdict) -> FixOutcome {
            self.log.lock().unwrap().push(format!("fix:{}", g.id));
            self.fix
        }
        async fn merge_back(&self) -> String {
            self.log.lock().unwrap().push("merge".into());
            "merged".into()
        }
        async fn record(&self, g: &SwarmGoal, status: &str, iterations: i64, _v: Option<&Verdict>) {
            self.log.lock().unwrap().push(format!("record:{}:{}:{}", g.id, status, iterations));
        }
        async fn record_iterations(&self, g: &SwarmGoal, iterations: i64) {
            self.log.lock().unwrap().push(format!("iter:{}:{}", g.id, iterations));
        }
        async fn post_verdict(&self, _g: &SwarmGoal, _v: &Verdict) {}
        async fn escalate(&self, g: &SwarmGoal, reason: &str) {
            self.log.lock().unwrap().push(format!("escalate:{}:{}", g.id, reason));
        }
        fn cancelled(&self) -> bool {
            self.cancel.load(Ordering::Relaxed)
        }
        async fn over_budget(&self) -> bool {
            self.over_budget
        }
    }

    #[test]
    fn classify_table() {
        // pass (incl. nits → target_met=true)
        assert_eq!(classify(&goal("g", true, 3), &vd(true, true), 0), Decision::Pass);
        // advisory + unmet → warned (no fix loop)
        assert_eq!(classify(&goal("g", false, 3), &vd(false, true), 0), Decision::Warned);
        // blocking + unmet, retries left → request fix
        assert_eq!(classify(&goal("g", true, 3), &vd(false, true), 0), Decision::RequestFix);
        // retries exhausted: near miss (blocker=false) → warned
        assert_eq!(classify(&goal("g", true, 2), &vd(false, false), 2), Decision::Warned);
        // retries exhausted: far miss (blocker=true) → unmet
        assert_eq!(classify(&goal("g", true, 2), &vd(false, true), 2), Decision::Unmet);
    }

    #[tokio::test]
    async fn pass_first_then_merge() {
        let ops = MockOps::new(FixOutcome::Completed).script("g1", vec![vd(true, true)]);
        let s = run_verification(vec![goal("g1", true, 3)], &ops).await;
        assert!(!s.blocked);
        assert_eq!(s.merge_status.as_deref(), Some("merged"));
        let log = ops.logged();
        assert_eq!(log, vec!["verify:g1:1", "record:g1:passed:0", "merge"]);
    }

    #[tokio::test]
    async fn near_miss_warns_only_after_retries_then_merges() {
        // Gap A: a near miss is pushed via retries; warns (non-blocking) only at exhaustion.
        let ops = MockOps::new(FixOutcome::Completed)
            .script("g1", vec![vd(false, false), vd(false, false), vd(false, false)]);
        let s = run_verification(vec![goal("g1", true, 2)], &ops).await;
        assert!(!s.blocked, "near miss must NOT block the merge");
        assert_eq!(s.merge_status.as_deref(), Some("merged"));
        // 3 verifications (scrutiny 1,2,3), 2 fixes, final = warned.
        assert_eq!(
            ops.scrutiny.lock().unwrap().clone(),
            vec![("g1".into(), 1), ("g1".into(), 2), ("g1".into(), 3)]
        );
        let log = ops.logged();
        assert_eq!(log.iter().filter(|l| l.starts_with("fix:")).count(), 2);
        assert!(log.contains(&"record:g1:warned:2".to_string()));
    }

    #[tokio::test]
    async fn far_miss_blocks_merge_and_escalates() {
        let ops = MockOps::new(FixOutcome::Completed)
            .script("g1", vec![vd(false, true), vd(false, true)]);
        let s = run_verification(vec![goal("g1", true, 1)], &ops).await;
        assert!(s.blocked);
        assert!(s.merge_status.is_none(), "blocking unmet must skip the merge");
        let log = ops.logged();
        assert!(log.contains(&"record:g1:unmet:1".to_string()));
        assert!(log.iter().any(|l| l.starts_with("escalate:g1:goal could not be achieved")));
    }

    #[tokio::test]
    async fn advisory_warns_without_fix_loop() {
        let ops = MockOps::new(FixOutcome::Completed).script("g1", vec![vd(false, true)]);
        let s = run_verification(vec![goal("g1", false, 3)], &ops).await;
        assert!(!s.blocked);
        let log = ops.logged();
        assert!(log.iter().all(|l| !l.starts_with("fix:")), "advisory goals never fix-loop");
        assert!(log.contains(&"record:g1:warned:0".to_string()));
    }

    #[tokio::test]
    async fn sequential_later_goal_runs_after_earlier_passes() {
        let ops = MockOps::new(FixOutcome::Completed)
            .script("g1", vec![vd(true, true)])
            .script("g2", vec![vd(true, true)]);
        let s = run_verification(vec![goal("g1", true, 3), goal("g2", true, 3)], &ops).await;
        assert!(!s.blocked);
        let log = ops.logged();
        let v1 = log.iter().position(|l| l == "verify:g1:1").unwrap();
        let v2 = log.iter().position(|l| l == "verify:g2:1").unwrap();
        assert!(v1 < v2, "goals verify in order");
    }

    #[tokio::test]
    async fn failed_fix_turn_is_distinct_and_does_not_burn_retries() {
        let ops = MockOps::new(FixOutcome::Failed).script("g1", vec![vd(false, true)]);
        let s = run_verification(vec![goal("g1", true, 5)], &ops).await;
        assert!(s.blocked);
        let log = ops.logged();
        // Exactly one verify + one (failed) fix, then unmet with the distinct reason.
        assert_eq!(log.iter().filter(|l| l.starts_with("verify:")).count(), 1);
        assert_eq!(log.iter().filter(|l| l.starts_with("fix:")).count(), 1);
        assert!(log.iter().any(|l| l.contains("could not run the fix turn")));
    }

    #[tokio::test]
    async fn over_budget_stops_with_escalation() {
        let mut ops = MockOps::new(FixOutcome::Completed).script("g1", vec![vd(false, true)]);
        ops.over_budget = true;
        let s = run_verification(vec![goal("g1", true, 3)], &ops).await;
        assert!(s.blocked);
        let log = ops.logged();
        assert!(log.iter().all(|l| !l.starts_with("fix:")), "no fix when over budget");
        assert!(log.iter().any(|l| l.contains("budget exhausted")));
    }

    #[tokio::test]
    async fn cancel_short_circuits_before_merge() {
        let mut ops = MockOps::new(FixOutcome::Completed)
            .script("g1", vec![vd(false, true)])
            .script("g2", vec![vd(true, true)]);
        ops.cancel_after_first_verify = true;
        let s = run_verification(vec![goal("g1", true, 3), goal("g2", true, 3)], &ops).await;
        assert!(s.cancelled);
        assert!(s.merge_status.is_none());
        let log = ops.logged();
        assert!(!log.contains(&"verify:g2:1".to_string()), "cancel stops before later goals");
        assert!(!log.contains(&"merge".to_string()));
    }
}
