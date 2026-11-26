//! RT: a held step thread keeps working through a daemon restart, and the
//! step runner's `resume` settles it as if nothing happened (runtime.md
//! §1.9, DR-006): usage from the step's first turn, a request already
//! pending reported at once.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use otto_agent_protocol::claude::ClaudeDriver;
use otto_agent_protocol::holder::HolderLauncher;
use otto_agent_protocol::step_runner::{NoStepHost, ThreadStepRunner};
use otto_agent_protocol::{
    AgentRuntime, DriverFactory, LaunchSpec, McpDelivery, McpServerDef, RuntimeConfig, RuntimeHost,
    StructuredDriver,
};
use otto_agent_run::step::{StepEvent, StepRunner, StepTurn};
use otto_core::agent_runtime::{AttentionKind, RuntimeEvent, RuntimeProvider, RuntimeSession};
use otto_core::cancel_signal::CancelSignal;
use otto_core::domain::Workspace;
use otto_core::{Id, Result};
use otto_state::{AgentRuntimeRepo, DbPool};

macro_rules! fake_bin {
    ($name:literal) => {
        std::env::var(concat!("NEXTEST_BIN_EXE_", $name))
            .unwrap_or_else(|_| env!(concat!("CARGO_BIN_EXE_", $name)).to_string())
    };
}

struct Host {
    root: String,
}

#[async_trait::async_trait]
impl RuntimeHost for Host {
    async fn workspace(&self, id: &Id) -> Result<Workspace> {
        Ok(Workspace {
            id: id.clone(),
            name: "W".into(),
            root_path: self.root.clone(),
            settings: serde_json::json!({}),
            archived: false,
            created_at: Utc::now(),
        })
    }
    async fn sandboxed(&self, _s: &RuntimeSession) -> bool {
        false
    }
    async fn prepare_spawn(
        &self,
        _ws: &Workspace,
        _s: &RuntimeSession,
        _spec: &mut LaunchSpec,
        _mcp: McpDelivery,
    ) -> Result<Vec<McpServerDef>> {
        Ok(Vec::new())
    }
    fn publish(&self, _s: &RuntimeSession, _e: Arc<RuntimeEvent>) {}
}

async fn open_db(path: &Path) -> DbPool {
    let pool = otto_state::open(path).await.unwrap();
    let now = Utc::now().to_rfc3339();
    for q in [
        "INSERT OR IGNORE INTO users (id, username, password_hash, display_name, is_root, created_at) \
         VALUES ('u1', 'u', 'x', 'U', 0, ?)",
        "INSERT OR IGNORE INTO workspaces (id, name, root_path, created_at) VALUES ('w1', 'W', '/tmp', ?)",
    ] {
        sqlx::query(q).bind(&now).execute(&pool).await.unwrap();
    }
    pool
}

fn held_runtime(pool: &DbPool, root: &Path) -> AgentRuntime {
    let drivers: Arc<dyn DriverFactory> =
        Arc::new(|p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
            (p == RuntimeProvider::Claude).then(|| {
                Box::new(ClaudeDriver::with_program(fake_bin!("otto-fake-claude")))
                    as Box<dyn StructuredDriver>
            })
        });
    AgentRuntime::new(
        AgentRuntimeRepo::new(pool.clone()),
        Arc::new(Host {
            root: root.display().to_string(),
        }),
        drivers,
        RuntimeConfig {
            attachments_dir: root.join("attachments"),
            holders_dir: Some(root.join("holders")),
            holder_launcher: Some(HolderLauncher::for_bin(fake_bin!("otto-runtime-holder"))),
            ..RuntimeConfig::default()
        },
    )
}

fn turn(root: &Path, prompt: &str) -> StepTurn {
    StepTurn {
        user_id: "u1".into(),
        workspace_id: "w1".into(),
        provider: "claude".into(),
        model: None,
        effort: None,
        cwd: root.to_path_buf(),
        title: "WF t · step · run abc123".into(),
        meta: serde_json::json!({"source": "workflow", "run_id": "r1"}),
        prompt: prompt.into(),
        continue_thread: None,
        handoff: None,
        read_only: false,
        timeout: None,
        no_progress: None,
    }
}

/// Start a step on generation 1 and restart the daemon once it runs.
async fn start_then_restart(
    dir: &Path,
    prompt: &str,
    ready: impl Fn(&StepEvent) -> bool,
) -> (AgentRuntime, Id, u32) {
    let db = dir.join("state.db");
    let rt1 = held_runtime(&open_db(&db).await, dir);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let runner = ThreadStepRunner::new(rt1.clone(), Arc::new(NoStepHost));
    let t = turn(dir, prompt);
    let task = tokio::spawn(async move { runner.run(t, tx, CancelSignal::new()).await });
    let mut started = None;
    while let Some(e) = tokio::time::timeout(Duration::from_secs(15), rx.recv())
        .await
        .unwrap()
    {
        if let StepEvent::Started {
            thread_id,
            start_ordinal,
        } = &e
        {
            started = Some((thread_id.clone(), *start_ordinal));
        }
        if ready(&e) {
            break;
        }
    }
    let (id, start) = started.unwrap();
    rt1.shutdown_for_restart().await;
    task.abort();
    drop(rt1);
    let rt2 = held_runtime(&open_db(&db).await, dir);
    let adopted = rt2.recover_and_adopt().await.unwrap();
    assert!(adopted.contains(&id), "the step thread was held");
    (rt2, id, start)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_held_step_finishes_through_a_restart_with_its_whole_usage() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let prompt = "OTTO_FAKE: usage 7 3\nOTTO_FAKE: sleep 1500\nOTTO_FAKE: reply held done";
    let (rt, id, start) = start_then_restart(&root, prompt, |e| *e == StepEvent::Working).await;
    let runner = ThreadStepRunner::new(rt.clone(), Arc::new(NoStepHost));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let out = runner
        .resume(&id, start, &turn(&root, prompt), tx, CancelSignal::new())
        .await
        .unwrap();
    assert_eq!(out.error, None, "{:?}", out.error);
    assert_eq!(out.reply, "held done");
    assert_eq!(out.turns, 1, "no restart turn: the agent never stopped");
    assert_eq!((out.usage.input_tokens, out.usage.output_tokens), (7, 3));
    rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_held_step_waiting_on_approval_needs_you_at_once_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let prompt = "OTTO_FAKE: approve make release\nOTTO_FAKE: reply released";
    let (rt, id, start) =
        start_then_restart(&root, prompt, |e| matches!(e, StepEvent::NeedsYou(_))).await;
    let runner = ThreadStepRunner::new(rt.clone(), Arc::new(NoStepHost));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let t = turn(&root, prompt);
    let (r2, id2) = (rt.clone(), id.clone());
    let wait = tokio::spawn(async move {
        ThreadStepRunner::new(r2, Arc::new(NoStepHost))
            .resume(&id2, start, &t, tx, CancelSignal::new())
            .await
    });
    let a = loop {
        match tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .unwrap()
        {
            Some(StepEvent::NeedsYou(a)) => break a,
            Some(_) => continue,
            None => panic!("no NeedsYou"),
        }
    };
    assert_eq!(a.kind, AttentionKind::Approval);
    rt.respond(
        &id,
        a.request_id.as_ref().unwrap(),
        otto_core::agent_runtime::ApprovalDecision::Accept,
    )
    .await
    .unwrap();
    let out = wait.await.unwrap().unwrap();
    assert_eq!(out.error, None, "{:?}", out.error);
    assert!(out.reply.ends_with("released"), "{}", out.reply);
    drop(runner);
    rt.shutdown().await;
}
