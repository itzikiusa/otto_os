//! The swarm result router (`otto_swarm::runtime::engine::route_result_inner`)
//! decides what happens to a task after every swarm turn: the operator's move
//! wins, a pause refunds the attempt, a Stop parks the task, failures retry up
//! to the ceiling, managed ICs propose through their manager, and handoff
//! chains are capped (test-quality-2-01). Each case seeds a swarm on a real
//! `ServerCtx::for_tests` (the runtime's `SwarmHost`) and drives the router
//! with a hand-built `SwarmTurnResult`, then asserts the task rows and the
//! board posts.
//!
//! The `*_submit_*` cases pin concurrency-async-2-02: every paste → Enter
//! submitter (swarm turns, handover briefs, delegation answers) holds the
//! session's submit lock, so a concurrent `submit_text` on the same live
//! session never interleaves into one merged prompt.

use std::time::Duration;

use otto_core::domain::{SessionKind, User, Workspace};
use otto_server::ServerCtx;
use otto_state::swarm::NewTask;
use otto_state::{
    NewAgent, NewProject, NewRun, NewSwarm, SwarmAgent, SwarmRun, SwarmTask, TaskPatch,
};
use otto_swarm::runtime::engine::{route_result_inner, submit_run_prompt};
use otto_swarm::runtime::run::{SwarmTurnResult, TurnHandoff, TurnSubtask};
use serde_json::{json, Value};

struct World {
    ctx: ServerCtx,
    owner: User,
    ws: Workspace,
    swarm: String,
    project: String,
    manager: SwarmAgent,
    ic: SwarmAgent,
    tmp: tempfile::TempDir,
}

async fn world() -> World {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = otto_server::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().to_path_buf()).await;
    let owner = otto_state::UsersRepo::new(pool.clone())
        .create("owner", "x", "Owner", false)
        .await
        .unwrap();
    let ws = ctx
        .workspaces
        .create("W", &tmp.path().to_string_lossy(), &owner.id)
        .await
        .unwrap();
    let repo = &ctx.swarm_repo;
    let swarm = repo
        .create_swarm(NewSwarm {
            workspace_id: ws.id.clone(),
            name: "s".into(),
            description: String::new(),
            preset_slug: None,
            config: json!({}),
            max_total_runs: None,
            max_cost_usd: None,
            max_runtime_secs: None,
            max_attempts: None,
            created_by: owner.id.clone(),
        })
        .await
        .unwrap();
    let mk_agent = |name: &str, title: &str, reports_to: Option<String>| NewAgent {
        swarm_id: swarm.id.clone(),
        workspace_id: ws.id.clone(),
        name: name.into(),
        title: title.into(),
        reports_to,
        provider: "claude".into(),
        model: None,
        soul_name: None,
        soul_md: None,
        specialization: String::new(),
        scope_md: String::new(),
        skills: json!([]),
        schedule: None,
        cwd_mode: None,
        avatar: String::new(),
        order_idx: 0,
        created_by: owner.id.clone(),
    };
    let manager = repo
        .create_agent(mk_agent("lead", "Lead", None))
        .await
        .unwrap();
    let ic = repo
        .create_agent(mk_agent("dev", "Developer", Some(manager.id.clone())))
        .await
        .unwrap();
    let project = repo
        .create_project(NewProject {
            swarm_id: swarm.id.clone(),
            workspace_id: ws.id.clone(),
            name: "p".into(),
            description: String::new(),
            repo_path: None,
            goal_md: None,
            story_id: None,
            order_idx: 0,
            created_by: owner.id.clone(),
        })
        .await
        .unwrap();
    World {
        ctx,
        owner,
        ws,
        swarm: swarm.id,
        project: project.id,
        manager,
        ic,
        tmp,
    }
}

impl World {
    /// An `in_progress` task assigned to `agent`, carrying `labels`.
    async fn task(&self, title: &str, agent: &SwarmAgent, labels: Value) -> SwarmTask {
        self.ctx
            .swarm_repo
            .create_task(NewTask {
                project_id: self.project.clone(),
                swarm_id: self.swarm.clone(),
                workspace_id: self.ws.id.clone(),
                title: title.into(),
                description: String::new(),
                assignee_agent_id: Some(agent.id.clone()),
                status: "in_progress".into(),
                priority: "medium".into(),
                parent_task_id: None,
                depends_on: json!([]),
                labels,
                order_idx: 0,
                created_by: self.owner.id.clone(),
            })
            .await
            .unwrap()
    }

    /// A queued (= active) `task` run of `task` by `agent`.
    async fn run(&self, task: &SwarmTask, agent: &SwarmAgent) -> SwarmRun {
        self.ctx
            .swarm_repo
            .create_run(NewRun {
                swarm_id: self.swarm.clone(),
                workspace_id: self.ws.id.clone(),
                project_id: Some(self.project.clone()),
                task_id: Some(task.id.clone()),
                agent_id: agent.id.clone(),
                kind: "task".into(),
                trigger: "manual".into(),
            })
            .await
            .unwrap()
    }

    async fn route(&self, run: &SwarmRun, task: &SwarmTask, result: Option<SwarmTurnResult>) {
        route_result_inner(&self.ctx.swarm_rt(), run, task, result).await;
    }

    async fn status(&self, task: &SwarmTask) -> String {
        self.ctx.swarm_repo.get_task(&task.id).await.unwrap().status
    }

    /// Board posts on `task` of `kind`.
    async fn posts(&self, task: &SwarmTask, kind: &str) -> Vec<String> {
        self.ctx
            .swarm_repo
            .list_board(&self.swarm, None, Some(&task.id), 100)
            .await
            .unwrap()
            .into_iter()
            .filter(|m| m.kind == kind)
            .map(|m| m.body)
            .collect()
    }

    async fn tasks(&self) -> Vec<SwarmTask> {
        self.ctx.swarm_repo.list_tasks(&self.project).await.unwrap()
    }
}

fn done(summary: &str) -> SwarmTurnResult {
    SwarmTurnResult {
        status: "done".into(),
        summary: summary.into(),
        ..Default::default()
    }
}

fn labels(task: &SwarmTask) -> Vec<String> {
    task.labels
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|l| l.as_str().map(str::to_string))
        .collect()
}

/// Case 1: the operator cancelled the card mid-turn — their decision wins.
#[tokio::test]
async fn operator_cancel_mid_turn_is_left_as_set() {
    let w = world().await;
    let task = w.task("Cancel me", &w.ic, json!([])).await;
    let run = w.run(&task, &w.ic).await;
    w.ctx
        .swarm_repo
        .update_task(
            &task.id,
            TaskPatch {
                status: Some("cancelled".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    w.route(&run, &task, Some(done("all good"))).await;
    assert_eq!(w.status(&task).await, "cancelled", "no resurrection");
    let posts = w.posts(&task, "status").await;
    assert_eq!(posts.len(), 1, "one note, no retry post: {posts:?}");
    assert!(posts[0].contains("left as you set it"), "{posts:?}");
    assert_eq!(w.tasks().await.len(), 1, "no follow-up task");
}

/// Case 2: a PAUSE refunds the attempt and parks the task as `todo`; a plain
/// operator Stop parks it as `blocked` (re-queueing made Stop a restart).
#[tokio::test]
async fn paused_stop_requeues_but_plain_stop_blocks() {
    let w = world().await;
    let repo = &w.ctx.swarm_repo;

    let paused = w.task("Paused", &w.ic, json!([])).await;
    repo.bump_task_attempt(&paused.id).await.unwrap();
    let run = w.run(&paused, &w.ic).await;
    repo.stop_active_runs_with_reason(&w.swarm, "paused")
        .await
        .unwrap();
    w.route(&run, &paused, None).await;
    let row = repo.get_task(&paused.id).await.unwrap();
    assert_eq!(row.status, "todo", "a pause parks for the resume");
    assert_eq!(row.attempts, 0, "the interrupted turn is refunded");

    let stopped = w.task("Stopped", &w.ic, json!([])).await;
    repo.bump_task_attempt(&stopped.id).await.unwrap();
    let run = w.run(&stopped, &w.ic).await;
    repo.stop_active_runs_with_reason(&w.swarm, "stopped by operator")
        .await
        .unwrap();
    w.route(&run, &stopped, None).await;
    let row = repo.get_task(&stopped.id).await.unwrap();
    assert_eq!(row.status, "blocked", "a Stop parks the task");
    assert_eq!(row.attempts, 1, "a Stop is not refunded");
    let posts = w.posts(&stopped, "status").await;
    assert!(
        posts.iter().any(|p| p.contains("parked as blocked")),
        "{posts:?}"
    );
}

/// Case 3: a failed turn retries below the attempt ceiling and blocks at it.
#[tokio::test]
async fn failed_turn_retries_below_ceiling_and_blocks_at_it() {
    let w = world().await;
    let repo = &w.ctx.swarm_repo;
    let max = repo.get_swarm(&w.swarm).await.unwrap().max_attempts;
    assert!(max > 1, "default ceiling allows a retry");

    let below = w.task("Below", &w.ic, json!([])).await;
    repo.bump_task_attempt(&below.id).await.unwrap();
    let run = w.run(&below, &w.ic).await;
    w.route(&run, &below, None).await;
    assert_eq!(w.status(&below).await, "todo");
    let posts = w.posts(&below, "status").await;
    assert!(posts.iter().any(|p| p.contains("will retry")), "{posts:?}");
    assert!(w.posts(&below, "escalation").await.is_empty());

    let at = w.task("At ceiling", &w.ic, json!([])).await;
    for _ in 0..max {
        repo.bump_task_attempt(&at.id).await.unwrap();
    }
    let run = w.run(&at, &w.ic).await;
    w.route(&run, &at, None).await;
    assert_eq!(w.status(&at).await, "blocked");
    let escalations = w.posts(&at, "escalation").await;
    assert_eq!(escalations.len(), 1, "{escalations:?}");
    assert!(escalations[0].contains("attempt(s)"), "{escalations:?}");
}

/// Case 4: a managed IC's subtasks become ONE triage task for its manager,
/// labelled `hops:1` — never direct board growth.
#[tokio::test]
async fn managed_ic_subtasks_become_one_manager_triage_task() {
    let w = world().await;
    let task = w.task("Build login", &w.ic, json!([])).await;
    let run = w.run(&task, &w.ic).await;
    let sub = |t: &str| TurnSubtask {
        title: t.into(),
        description: format!("{t} details"),
        ..Default::default()
    };
    let result = SwarmTurnResult {
        status: "blocked".into(),
        subtasks: vec![sub("Add OAuth"), sub("Add 2FA")],
        ..Default::default()
    };
    w.route(&run, &task, Some(result)).await;
    let created: Vec<SwarmTask> = w
        .tasks()
        .await
        .into_iter()
        .filter(|t| t.id != task.id)
        .collect();
    assert_eq!(created.len(), 1, "one triage task, not one per subtask");
    let triage = &created[0];
    assert_eq!(
        triage.assignee_agent_id.as_deref(),
        Some(w.manager.id.as_str())
    );
    let l = labels(triage);
    assert!(l.contains(&"hops:1".to_string()), "{l:?}");
    assert!(l.contains(&"proposal".to_string()), "{l:?}");
    assert!(triage.description.contains("Add OAuth") && triage.description.contains("Add 2FA"));
    assert_eq!(w.status(&task).await, "blocked");
}

/// Case 5: a handoff from a `hops:3` task is past MAX_HANDOFF_HOPS (3): it
/// escalates to a human and creates nothing.
#[tokio::test]
async fn handoff_past_hop_cap_escalates_without_a_task() {
    let w = world().await;
    let task = w.task("Deep chain", &w.ic, json!(["hops:3"])).await;
    let run = w.run(&task, &w.ic).await;
    let result = SwarmTurnResult {
        status: "blocked".into(),
        handoffs: vec![TurnHandoff {
            to_role: "QA".into(),
            brief: "please test".into(),
        }],
        ..Default::default()
    };
    w.route(&run, &task, Some(result)).await;
    assert_eq!(w.tasks().await.len(), 1, "no new task past the cap");
    let escalations = w.posts(&task, "escalation").await;
    assert_eq!(escalations.len(), 1, "{escalations:?}");
    assert!(
        escalations[0].contains("exceeded 3 hops"),
        "{escalations:?}"
    );
}

/// Case 6: the same handoff raised twice is deduped against the open task.
#[tokio::test]
async fn duplicate_handoff_is_deduped() {
    let w = world().await;
    let repo = &w.ctx.swarm_repo;
    let task = w.task("Ship it", &w.ic, json!([])).await;
    let handoff = || SwarmTurnResult {
        status: "blocked".into(),
        handoffs: vec![TurnHandoff {
            to_role: "QA".into(),
            brief: "please test the release".into(),
        }],
        ..Default::default()
    };
    let run = w.run(&task, &w.ic).await;
    w.route(&run, &task, Some(handoff())).await;
    assert_eq!(w.tasks().await.len(), 2, "first handoff creates its triage");
    // Back in progress for a second turn that repeats the same handoff.
    repo.update_task(
        &task.id,
        TaskPatch {
            status: Some("in_progress".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let run = w.run(&task, &w.ic).await;
    w.route(&run, &task, Some(handoff())).await;
    let followups: Vec<SwarmTask> = w
        .tasks()
        .await
        .into_iter()
        .filter(|t| t.id != task.id)
        .collect();
    assert_eq!(followups.len(), 1, "the repeat is deduped");
    assert!(labels(&followups[0]).contains(&"hops:1".to_string()));
}

// ---------------------------------------------------------------------------
// concurrency-async-2-02: paste → Enter submitters hold the submit lock
// ---------------------------------------------------------------------------

/// A live session whose PTY runs `cat > sink`, so the test sees exactly what
/// reached the terminal; each `\r` (mapped to `\n`) ends one submission.
async fn cat_session(w: &World) -> (String, std::path::PathBuf) {
    let sink = w.tmp.path().join(format!("pty-{}", uuid::Uuid::new_v4()));
    let session = w
        .ctx
        .manager
        .create(
            &w.ws,
            &w.owner.id,
            otto_core::api::CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some("shell".into()),
                title: Some("submit lock".into()),
                cwd: None,
                connection_id: None,
                meta: None,
                model: None,
            },
            Some(otto_pty::CommandSpec {
                program: "/bin/sh".into(),
                args: vec!["-c".into(), format!("exec cat > '{}'", sink.display())],
                cwd: Some(w.tmp.path().to_string_lossy().into()),
                env: vec![],
            }),
        )
        .await
        .unwrap();
    (session.id, sink)
}

/// The submissions that reached the PTY, sorted (order between the two
/// submitters is not part of the contract — only that neither interleaves).
async fn submissions(sink: &std::path::Path, want: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for _ in 0..80 {
        let got = std::fs::read_to_string(sink).unwrap_or_default();
        lines = got
            .split(['\r', '\n'])
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if lines.len() >= want {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    lines.sort();
    lines
}

#[tokio::test]
async fn swarm_turn_submit_does_not_interleave_with_submit_text() {
    let w = world().await;
    let (sid, sink) = cat_session(&w).await;
    let task = w.task("Brief", &w.ic, json!([])).await;
    let run = w.run(&task, &w.ic).await;
    let rt = w.ctx.swarm_rt();
    let (swarm_ok, other) = tokio::join!(
        submit_run_prompt(
            &rt,
            &w.swarm,
            &run.id,
            &sid,
            b"\x1b[200~from-swarm\x1b[201~",
            false
        ),
        w.ctx.manager.submit_text(&sid, "from-broadcast"),
    );
    assert!(swarm_ok);
    other.unwrap();
    assert_eq!(
        submissions(&sink, 2).await,
        vec![
            "\x1b[200~from-broadcast\x1b[201~".to_string(),
            "\x1b[200~from-swarm\x1b[201~".to_string(),
        ],
        "each submission must be its own paste + Enter"
    );
    let _ = w.ctx.manager.kill_session(&sid).await;
}

#[tokio::test]
async fn handover_submit_does_not_interleave_with_submit_text() {
    let w = world().await;
    let (sid, sink) = cat_session(&w).await;
    let (handover_ok, other) = tokio::join!(
        otto_server::routes::handover::inject_handover_prompt(
            &w.ctx.manager,
            &sid,
            "from-handover"
        ),
        w.ctx.manager.submit_text(&sid, "from-broadcast"),
    );
    assert!(handover_ok);
    other.unwrap();
    assert_eq!(
        submissions(&sink, 2).await,
        vec![
            "\x1b[200~from-broadcast\x1b[201~".to_string(),
            "\x1b[200~from-handover\x1b[201~".to_string(),
        ],
        "each submission must be its own paste + Enter"
    );
    let _ = w.ctx.manager.kill_session(&sid).await;
}

#[tokio::test]
async fn delegation_answer_submit_does_not_interleave_with_submit_text() {
    let w = world().await;
    let (sid, sink) = cat_session(&w).await;
    let agents = otto_state::PersonalAgentsRepo::new(w.ctx.pool.clone());
    let agent = agents
        .create(otto_state::personal_agents::NewPersonalAgent::defaults(
            w.ws.id.clone(),
            "pa".into(),
        ))
        .await
        .unwrap();
    let run = agents
        .create_run(otto_state::personal_agents::NewAgentRun {
            agent_id: agent.id.clone(),
            schedule_id: None,
            workspace_id: w.ws.id.clone(),
            trigger: "manual".into(),
        })
        .await
        .unwrap();
    agents.set_run_session(&run.id, &sid).await.unwrap();
    let (answer_ok, other) = tokio::join!(
        otto_assistant::assistant::tasks::deliver_delegation_answer(
            &w.ctx,
            Some(run.id.as_str()),
            "from-assistant"
        ),
        w.ctx.manager.submit_text(&sid, "from-broadcast"),
    );
    assert!(answer_ok);
    other.unwrap();
    assert_eq!(
        submissions(&sink, 2).await,
        vec![
            "\x1b[200~from-assistant\x1b[201~".to_string(),
            "\x1b[200~from-broadcast\x1b[201~".to_string(),
        ],
        "each submission must be its own paste + Enter"
    );
    let _ = w.ctx.manager.kill_session(&sid).await;
}
