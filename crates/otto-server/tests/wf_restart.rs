//! Restart in the middle of each node kind on the new engine (test plan §5.1):
//! the engine is dropped as a process death would drop it and a fresh
//! context boots on the same database and data dir through the daemon's own
//! recovery (`boot::recover_before_serve`: runtime adoption, then the run
//! reconciler). Agents are the in-memory fakes (otto_agent_protocol::scripted).
//!
//! Two ways a node is caught mid-flight:
//! * **left behind** — for every catalog kind, the row a dead daemon leaves
//!   (run running, the node running since before the boot) is written and
//!   the new life boots on it; the outcome must be the kind's row in
//!   `wf_lifecycle::RESTART`, the one table of restart behaviour (G1);
//! * **live** — the run really executes and the engine task is aborted while
//!   the node works: an agent step with its thread gone (runtime shut down)
//!   or still held (the runtime survives the engine), and a delay step.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use otto_agent_protocol::{scripted, AgentRuntime, RuntimeConfig, SessionManagerHost};
use otto_core::workflows::{NodeStatus, RunScope, RunStatus, WorkflowGraph, WorkflowRun};
use otto_server::ServerCtx;
use otto_state::{DbPool, WorkflowsRepo};
use serde_json::{json, Value};

use crate::wf_lifecycle::{Restart, RESTART};

const WS: &str = "ws-restart";

pub(crate) struct Rig {
    _tmp: tempfile::TempDir,
    root: PathBuf,
    rec: Arc<otto_channels::test_util::RecordingAdapter>,
}

impl Rig {
    pub(crate) async fn new() -> Rig {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        for d in ["data", "scripts", "fake-state"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let pool = otto_state::open(&root.join("data/otto.db")).await.unwrap();
        let now = "2026-01-01T00:00:00Z";
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('root', 'root', 'x', 'Root', 1, ?)")
            .bind(now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at) VALUES (?, 'Restart', ?, '{}', 0, ?)")
            .bind(WS).bind(root.to_string_lossy()).bind(now).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, 'root', 'admin')")
            .bind(WS).execute(&pool).await.unwrap();
        let rec = otto_channels::test_util::RecordingAdapter::new(
            otto_core::domain::Channel::Slack,
            WS,
            "chat-restart",
            None,
        );
        pool.close().await;
        Rig {
            _tmp: tmp,
            root,
            rec,
        }
    }

    fn fake_log(&self) -> PathBuf {
        self.root.join("fake-turns.jsonl")
    }

    /// One daemon life: the same database file and data dir, a fresh
    /// context and (unless `keep` hands over a surviving one) a fresh runtime.
    pub(crate) async fn life(&self, keep: Option<AgentRuntime>) -> ServerCtx {
        let pool = otto_state::open(&self.root.join("data/otto.db"))
            .await
            .unwrap();
        let mut ctx = ServerCtx::for_tests(&pool, self.root.join("data")).await;
        ctx.agent_runtime = keep.unwrap_or_else(|| runtime(&pool, &ctx, &self.root));
        ctx.outbound = self.rec.factory();
        let key = format!("chan-bot-{WS}-slack");
        ctx.secrets.put(&key, "restart-token").unwrap();
        ctx.integrations_store
            .upsert(
                &WS.into(),
                otto_core::domain::Channel::Slack,
                true,
                Some(key),
                None,
                "user-restart",
                false,
                "",
                "chat-restart",
                "",
            )
            .await
            .unwrap();
        otto_state::SettingsRepo::new(pool.clone())
            .put("agent_admission_enabled", &json!(false))
            .await
            .unwrap();
        ctx
    }
}

fn runtime(pool: &DbPool, ctx: &ServerCtx, root: &Path) -> AgentRuntime {
    let host = SessionManagerHost {
        manager: Arc::clone(&ctx.manager),
        workspaces: otto_state::WorkspacesRepo::new(pool.clone()),
        publish: Box::new(|_| {}),
        notices: otto_agent_protocol::attention::AttentionNotices::new(
            otto_state::NotificationsRepo::new(pool.clone()),
            Arc::new(|_| {}),
        ),
    };
    let s = |p: &str| root.join(p).to_string_lossy().into_owned();
    let env = vec![
        ("OTTO_FAKE_SCRIPTS".to_string(), s("scripts")),
        ("OTTO_FAKE_STATE_DIR".to_string(), s("fake-state")),
        ("OTTO_FAKE_LOG".to_string(), s("fake-turns.jsonl")),
        ("OTTO_FAKE_DEFAULT".to_string(), "step".to_string()),
    ];
    AgentRuntime::new(
        otto_state::AgentRuntimeRepo::new(pool.clone()),
        Arc::new(host),
        scripted::drivers(),
        RuntimeConfig {
            attachments_dir: root.join("attachments"),
            transport: Some(scripted::transport(env)),
            ..RuntimeConfig::default()
        },
    )
}

fn graph(kind: &str, params: Value) -> WorkflowGraph {
    // Edges from variables: otto-core's background-source guard reads every
    // literal string under a "source" key as a session source.
    let edge = |id: &str, from: &str, to: &str| json!({"id": id, "source": from, "target": to});
    serde_json::from_value(json!({
        "nodes": [
            {"id": "n1", "kind": "manual_trigger", "name": "Start 1"},
            {"id": "n2", "kind": kind, "name": "Step 2", "params": params},
            {"id": "n3", "kind": "log", "name": "Log 3"},
        ],
        "edges": [edge("e1", "n1", "n2"), edge("e2", "n2", "n3")],
    }))
    .unwrap()
}

/// Params that let a re-run kind do its work here (a kind without a row
/// re-runs on empty params; its own failure then is not a restart verdict).
fn params(kind: &str) -> Value {
    match kind {
        "agent_prompt" => {
            json!({"provider": "claude", "prompt": "Do the step.\nOTTO_FAKE: @script"})
        }
        "delay" => json!({"ms": 50}),
        "transform" => json!({"json": {"ok": true}}),
        "condition" => json!({"expr": "true"}),
        "script" => json!({"command": "true"}),
        _ => json!({}),
    }
}

pub(crate) async fn create(
    ctx: &ServerCtx,
    graph: &WorkflowGraph,
) -> (otto_core::workflows::Workflow, otto_core::Id) {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let wf = repo
        .create(&WS.into(), "Restart", "", "", graph, &"root".into())
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &WS.into(), &json!({}), Some(&"root".into()))
        .await
        .unwrap();
    (wf, run.id)
}

/// The row a dead daemon leaves: running, n1 done, n2 running since a minute ago.
async fn leave_behind(ctx: &ServerCtx, run_id: &otto_core::Id, approval: bool) {
    let ago = Utc::now() - chrono::Duration::seconds(60);
    let nodes = json!([
        {"node_id": "n1", "status": "success", "output": {}, "started_at": ago, "duration_ms": 1, "step_no": 1},
        {"node_id": "n2", "status": "running", "started_at": ago, "step_no": 2},
        {"node_id": "n3", "status": "pending"},
    ]);
    sqlx::query("UPDATE workflow_runs SET status = 'running', started_at = ?, nodes_json = ?, waiting_approval = ?, approval_node_id = ? WHERE id = ?")
        .bind(ago)
        .bind(nodes.to_string())
        .bind(approval)
        .bind(approval.then_some("n2"))
        .bind(run_id)
        .execute(&ctx.pool)
        .await
        .unwrap();
}

async fn boot(ctx: &ServerCtx) {
    otto_server::boot::recover_before_serve(ctx, &mut otto_server::boot::BootPhases::start())
        .await
        .unwrap();
}

/// Wait until the run settles, or parks at an approval when `parked` says so.
pub(crate) async fn settled(ctx: &ServerCtx, run_id: &otto_core::Id, parked: bool) -> WorkflowRun {
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let run = repo.get_run(run_id).await.unwrap();
        let done = matches!(
            run.status,
            RunStatus::Success | RunStatus::Error | RunStatus::Canceled
        ) && !otto_server::workflow_engine::driver_alive(run_id);
        if done || (parked && run.waiting_approval) {
            return run;
        }
        assert!(
            Instant::now() < deadline,
            "{run_id} did not settle: {:?} {:?}",
            run.status,
            run.nodes
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

pub(crate) fn node<'a>(run: &'a WorkflowRun, id: &str) -> &'a otto_core::workflows::NodeRunState {
    run.nodes
        .iter()
        .find(|n| n.node_id == id)
        .unwrap_or_else(|| panic!("no {id}: {:?} {:?} {:?}", run.status, run.error, run.nodes))
}

/// Every catalog kind, left behind mid-flight by a dead daemon, comes back as
/// its `RESTART` row says: re-entered (the node runs again after the boot,
/// never failed as "outcome unknown"), or failed "outcome unknown" with
/// nothing after it run. An approval re-parks without spending a resume.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn every_kind_left_behind_mid_flight_comes_back_as_its_row_says() {
    let mut seen = Vec::new();
    for (kind, want) in RESTART {
        let rig = Rig::new().await;
        let ctx = rig.life(None).await;
        let (_wf, run_id) = create(&ctx, &graph(kind, params(kind))).await;
        let approval = *kind == "human_approval";
        leave_behind(&ctx, &run_id, approval).await;
        let booted = Utc::now();
        boot(&ctx).await;
        let run = settled(&ctx, &run_id, approval).await;
        // Re-entry validates the graph again: a kind this harness gives no
        // working config (a DB connection, a repo, …) is refused there —
        // re-entered, not a restart verdict.
        let refused = run
            .error
            .as_deref()
            .is_some_and(|e| e.starts_with("invalid:"));
        if refused {
            seen.push((
                *kind,
                Restart::Resume,
                if *want == Restart::FailOutcomeUnknown {
                    *want
                } else {
                    Restart::Resume
                },
                "refused at re-entry validation".into(),
                run.error.clone(),
            ));
            ctx.agent_runtime.shutdown().await;
            continue;
        }
        let n2 = node(&run, "n2");
        let unknown = n2
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("outcome unknown");
        let got = if approval {
            assert!(run.waiting_approval, "{kind}: re-parked");
            assert_eq!(
                run.resume_attempts, 0,
                "{kind}: a parked approval spends no resume"
            );
            Restart::Resume
        } else if unknown {
            assert_eq!(
                node(&run, "n3").status,
                NodeStatus::Skipped,
                "{kind}: nothing after it ran"
            );
            assert_eq!(run.status, RunStatus::Error, "{kind}");
            Restart::FailOutcomeUnknown
        } else {
            assert!(
                n2.started_at.is_some_and(|t| t >= booted),
                "{kind}: re-entered after the boot: {n2:?} {:?}",
                run.error
            );
            Restart::Resume
        };
        // A loop left without a checkpoint has nothing to resume from.
        let want = if *want == Restart::ResumeFromCheckpoint {
            Restart::FailOutcomeUnknown
        } else {
            *want
        };
        seen.push((
            *kind,
            got,
            want,
            format!("{:?}", run.status),
            n2.error.clone(),
        ));
        ctx.agent_runtime.shutdown().await;
    }
    for (kind, got, _, status, err) in &seen {
        let err: String = err.as_deref().unwrap_or("").chars().take(90).collect();
        println!("| {kind} | {got:?} | {status} | {err} |");
    }
    let wrong: Vec<_> = seen
        .iter()
        .filter(|(_, got, want, ..)| got != want)
        .collect();
    assert!(
        wrong.is_empty(),
        "kinds whose restart differs from their row: {wrong:#?}"
    );
    // Re-entered kinds this harness can run to the end succeed.
    for (kind, _, _, status, err) in &seen {
        if [
            "manual_trigger",
            "agent_prompt",
            "delay",
            "log",
            "transform",
            "condition",
        ]
        .contains(kind)
        {
            assert_eq!(status, "Success", "{kind}: {err:?}");
        }
    }
}

/// The fake turn log's turns per session.
fn turns(rig: &Rig) -> Vec<(String, u64)> {
    std::fs::read_to_string(rig.fake_log())
        .unwrap_or_default()
        .lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .map(|v| {
            (
                v["session"].as_str().unwrap_or_default().to_string(),
                v["turn"].as_u64().unwrap_or(0),
            )
        })
        .collect()
}

/// A run really executing, its engine task aborted once n2 runs and (for an
/// agent step) its thread is recorded.
async fn start_and_cut(
    _rig: &Rig,
    ctx: &ServerCtx,
    graph: WorkflowGraph,
) -> (otto_core::Id, WorkflowRun) {
    let (wf, run_id) = create(ctx, &graph).await;
    let ws = ctx.workspaces.get(&WS.into()).await.unwrap();
    let task = tokio::spawn(otto_server::workflow_engine::run_workflow(
        ctx.clone(),
        ws,
        wf,
        run_id.clone(),
        json!({}),
        RunScope::default(),
        None,
    ));
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let deadline = Instant::now() + Duration::from_secs(30);
    let before = loop {
        let run = repo.get_run(&run_id).await.unwrap();
        let agent = graph.nodes[1].kind == "agent_prompt";
        let threads: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_runtime_sessions")
            .fetch_one(&ctx.pool)
            .await
            .unwrap();
        let running = run
            .nodes
            .iter()
            .any(|n| n.node_id == "n2" && n.status == NodeStatus::Running);
        if running && (!agent || threads > 0) {
            // Let the prompt reach the fake before the cut.
            tokio::time::sleep(Duration::from_millis(500)).await;
            break repo.get_run(&run_id).await.unwrap();
        }
        assert!(Instant::now() < deadline, "n2 never ran: {:?}", run.nodes);
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    task.abort();
    let _ = task.await;
    (run_id, before)
}

/// Run `f` on a multi-thread runtime whose workers have `stack` bytes.
fn on_workers<F: std::future::Future<Output = ()>>(stack: usize, f: F) {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_stack_size(stack)
        .enable_all()
        .build()
        .unwrap()
        .block_on(f);
}

/// Tokio's default worker stack, the daemon's: a debug build of the resumed
/// agent step overflowed it until the deep awaits were boxed in their own
/// frames (CR1-002, `otto_core::stack`).
const DEFAULT_STACK: usize = 2 * 1024 * 1024;

/// An agent step mid-turn whose thread died with the daemon: after the boot
/// it continues on the SAME thread — one more turn there (the handoff nudge),
/// no second thread — and the step and run succeed with one resume spent.
#[test]
fn an_agent_step_whose_thread_died_continues_on_the_same_thread() {
    on_workers(DEFAULT_STACK, thread_died());
}

async fn thread_died() {
    let rig = Rig::new().await;
    std::fs::write(
        rig.root.join("scripts/step-2.txt"),
        "sleep 4000\nwrite $HANDOFF @filler:400\nreply done\n",
    )
    .unwrap();
    let ctx = rig.life(None).await;
    let (run_id, before) =
        start_and_cut(&rig, &ctx, graph("agent_prompt", params("agent_prompt"))).await;
    let thread: String = sqlx::query_scalar("SELECT id FROM agent_runtime_sessions")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(
        node(&before, "n2").status,
        NodeStatus::Running,
        "cut mid-turn"
    );
    ctx.agent_runtime.shutdown().await; // the process dies: no holder survives
    drop(ctx);
    let ctx = rig.life(None).await;
    boot(&ctx).await;
    let run = settled(&ctx, &run_id, false).await;
    let n2 = node(&run, "n2");
    assert_eq!(
        run.status,
        RunStatus::Success,
        "{:?} {:?}",
        run.error,
        n2.error
    );
    assert_eq!(
        n2.thread.as_ref().map(|t| t.thread_id.clone()),
        Some(thread.clone()),
        "same thread"
    );
    assert_eq!(run.resume_attempts, 1);
    let threads: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agent_runtime_sessions")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(threads, 1, "no second thread");
    // The cut turn ended as a server restart; the one turn sent after it on
    // the same thread is the handoff nudge (the fake echoes an unscripted
    // prompt), and the step file falls back to the engine's rendering.
    let reply = n2
        .output
        .as_ref()
        .and_then(|o| o["reply"].as_str())
        .unwrap_or_default();
    assert!(
        reply.starts_with("echo: You did not write ") && reply.contains("step2-step-2.md"),
        "{reply}"
    );
    assert!(
        turns(&rig).is_empty(),
        "the cut scripted turn never finished: {:?}",
        turns(&rig)
    );
    ctx.agent_runtime.shutdown().await;
}

/// An agent step mid-turn whose thread outlives the engine (a held thread):
/// the new life awaits it — the turn finishes once, nothing is prompted again.
#[test]
fn an_agent_step_whose_thread_survived_is_awaited_not_prompted_again() {
    on_workers(DEFAULT_STACK, thread_survived());
}

async fn thread_survived() {
    let rig = Rig::new().await;
    std::fs::write(
        rig.root.join("scripts/step-2.txt"),
        "sleep 4000\nwrite $HANDOFF @filler:400\nreply done\n",
    )
    .unwrap();
    let ctx = rig.life(None).await;
    let (run_id, before) =
        start_and_cut(&rig, &ctx, graph("agent_prompt", params("agent_prompt"))).await;
    let thread: String = sqlx::query_scalar("SELECT id FROM agent_runtime_sessions")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    assert_eq!(
        node(&before, "n2").status,
        NodeStatus::Running,
        "cut mid-turn"
    );
    let kept = ctx.agent_runtime.clone();
    drop(ctx);
    let ctx = rig.life(Some(kept)).await;
    boot(&ctx).await;
    let run = settled(&ctx, &run_id, false).await;
    let n2 = node(&run, "n2");
    assert_eq!(
        run.status,
        RunStatus::Success,
        "{:?} {:?}",
        run.error,
        n2.error
    );
    assert_eq!(
        n2.thread.as_ref().map(|t| t.thread_id.clone()),
        Some(thread)
    );
    assert_eq!(
        turns(&rig).len(),
        1,
        "one turn, never prompted again: {:?}",
        turns(&rig)
    );
    ctx.agent_runtime.shutdown().await;
}

/// A delay cut mid-wait re-runs after the boot and the run finishes.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_delay_cut_mid_wait_runs_again_and_the_run_finishes() {
    let rig = Rig::new().await;
    let ctx = rig.life(None).await;
    let (run_id, _) = start_and_cut(&rig, &ctx, graph("delay", json!({"ms": 3000}))).await;
    drop(ctx);
    let ctx = rig.life(None).await;
    let booted = Utc::now();
    boot(&ctx).await;
    let run = settled(&ctx, &run_id, false).await;
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    assert!(node(&run, "n2").started_at.is_some_and(|t| t >= booted));
    assert_eq!(node(&run, "n3").status, NodeStatus::Success);
}
