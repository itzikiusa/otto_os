//! RT: unattended turns settle on events (runtime.md §1.4). Every case runs
//! a real `AgentRuntime` on the scripted driver or the fake CLIs.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use otto_agent_protocol::claude::ClaudeDriver;
use otto_agent_protocol::codex::CodexDriver;
use otto_agent_protocol::fake_script::Config;
use otto_agent_protocol::scripted::ScriptedDriver;
use otto_agent_protocol::{
    AgentRuntime, DriverFactory, Ended, LaunchSpec, McpDelivery, McpServerDef, RuntimeConfig,
    RuntimeHost, StructuredDriver, TurnOpts,
};
use otto_agent_run::step::StepEvent;
use otto_core::agent_runtime::{
    ApprovalDecision, Attention, AttentionKind, CreateRuntimeSessionReq, OpenPeerReq,
    ProviderTurnStatus, QuestionAnswer, RuntimeEvent, RuntimeEventBody, RuntimePromptReq,
    RuntimeProvider, RuntimeRequestStatus, RuntimeSession,
};
use otto_core::cancel_signal::CancelSignal;
use otto_core::domain::Workspace;
use otto_core::{Id, Result};
use otto_state::{AgentRuntimeRepo, DbPool};
use serde_json::json;

macro_rules! fake_bin {
    ($name:literal) => {
        std::env::var(concat!("NEXTEST_BIN_EXE_", $name))
            .unwrap_or_else(|_| env!(concat!("CARGO_BIN_EXE_", $name)).to_string())
    };
}

struct Host {
    root: PathBuf,
    env: Vec<(String, String)>,
    events: Mutex<Vec<Arc<RuntimeEvent>>>,
}

#[async_trait::async_trait]
impl RuntimeHost for Host {
    async fn workspace(&self, id: &Id) -> Result<Workspace> {
        Ok(Workspace {
            id: id.clone(),
            name: "W".into(),
            root_path: self.root.display().to_string(),
            settings: json!({}),
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
        spec: &mut LaunchSpec,
        _mcp: McpDelivery,
    ) -> Result<Vec<McpServerDef>> {
        spec.env.extend(self.env.iter().cloned());
        Ok(Vec::new())
    }
    fn publish(&self, _s: &RuntimeSession, e: Arc<RuntimeEvent>) {
        self.events.lock().unwrap().push(e);
    }
}

async fn pool() -> DbPool {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let now = Utc::now().to_rfc3339();
    for q in [
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) \
         VALUES ('u1', 'u', 'x', 'U', 0, ?)",
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('w1', 'W', '/tmp', ?)",
    ] {
        sqlx::query(q).bind(&now).execute(&pool).await.unwrap();
    }
    pool.into()
}

struct Rig {
    rt: AgentRuntime,
    pool: DbPool,
    host: Arc<Host>,
    drivers: Arc<dyn DriverFactory>,
    _dir: tempfile::TempDir,
    root: PathBuf,
}

impl Rig {
    /// `binaries`: the fake claude / codex CLIs; else the scripted driver.
    async fn new(binaries: bool) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let path = |n: &str| root.join(n).display().to_string();
        let host = Arc::new(Host {
            root: root.clone(),
            env: vec![
                ("OTTO_FAKE_STATE_DIR".into(), path("state")),
                ("CLAUDE_CODE_TMPDIR".into(), path("claude-tmp")),
            ],
            events: Mutex::default(),
        });
        let drivers: Arc<dyn DriverFactory> = if binaries {
            Arc::new(|p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
                Some(match p {
                    RuntimeProvider::Codex => {
                        Box::new(CodexDriver::with_program(fake_bin!("otto-fake-codex")))
                    }
                    _ => Box::new(ClaudeDriver::with_program(fake_bin!("otto-fake-claude"))),
                })
            })
        } else {
            ScriptedDriver::factory(Config {
                provider: String::new(),
                time_scale: 1.0,
                scripts_dir: None,
                step_persona: false,
                review_dir: None,
                state_dir: root.join("state"),
                unsupported: &[],
                log: None,
            })
        };
        let pool = pool().await;
        let rt = Self::runtime(&pool, &host, &drivers, &root);
        Self {
            rt,
            pool,
            host,
            drivers,
            _dir: dir,
            root,
        }
    }

    fn runtime(
        pool: &DbPool,
        host: &Arc<Host>,
        drivers: &Arc<dyn DriverFactory>,
        root: &std::path::Path,
    ) -> AgentRuntime {
        AgentRuntime::new(
            AgentRuntimeRepo::new(pool.clone()),
            Arc::clone(host) as Arc<dyn RuntimeHost>,
            Arc::clone(drivers),
            RuntimeConfig {
                attachments_dir: root.join("attachments"),
                ..RuntimeConfig::default()
            },
        )
    }

    async fn thread(&self, provider: RuntimeProvider) -> Id {
        let req = CreateRuntimeSessionReq {
            title_auto: false,
            provider: Some(provider),
            cwd: Some(self.root.display().to_string()),
            title: None,
            model: None,
            access_mode: None,
            meta: Some(json!({"source": "workflow", "run_id": "r1"})),
        };
        self.rt
            .create(&"u1".into(), &"w1".into(), req)
            .await
            .unwrap()
            .id
    }
}

fn script(verbs: &[&str]) -> RuntimePromptReq {
    let text = verbs
        .iter()
        .map(|v| format!("OTTO_FAKE: {v}"))
        .collect::<Vec<_>>()
        .join("\n");
    RuntimePromptReq::of(&text)
}

/// The events a wait reported, for asserting order.
#[derive(Clone, Default)]
struct Seen(Arc<Mutex<Vec<StepEvent>>>);

impl Seen {
    fn on(&self) -> impl FnMut(StepEvent) + Send + 'static {
        let s = self.0.clone();
        move |e| s.lock().unwrap().push(e)
    }
    fn all(&self) -> Vec<StepEvent> {
        self.0.lock().unwrap().clone()
    }
}

async fn need(rx: &mut tokio::sync::mpsc::UnboundedReceiver<StepEvent>) -> Attention {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Some(StepEvent::NeedsYou(a))) => return a,
            Ok(Some(_)) => continue,
            other => panic!("no NeedsYou: {other:?}"),
        }
    }
}

fn opts() -> TurnOpts {
    TurnOpts::new(CancelSignal::new())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_finished_turn_settles_at_once_with_its_answer_and_usage() {
    let rig = Rig::new(false).await;
    let id = rig.thread(RuntimeProvider::Claude).await;
    let seen = Seen::default();
    let out = rig
        .rt
        .run_turn(
            &id,
            &script(&["usage 120 30", "reply all good"]),
            opts(),
            seen.on(),
        )
        .await
        .unwrap();
    let returned = Utc::now();
    assert_eq!(out.ended, Ended::Settled);
    assert_eq!(out.turn_status, Some(ProviderTurnStatus::Completed));
    assert_eq!((out.turns, out.start_ordinal), (1, 0));
    assert!(out.answer.contains("all good"), "{}", out.answer);
    assert_eq!((out.usage.input_tokens, out.usage.output_tokens), (120, 30));
    assert_eq!(seen.all().first(), Some(&StepEvent::Working));
    // Settled on the terminal event, not after a poll or a stability sleep.
    let ended = rig
        .host
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e.body, RuntimeEventBody::TurnTerminal { .. }))
        .map(|e| e.ts)
        .max()
        .unwrap();
    let lag = (returned - ended).to_std().unwrap_or_default();
    assert!(
        lag < Duration::from_millis(250),
        "settled {lag:?} after the turn ended"
    );
    // A second turn counts only itself.
    let out = rig
        .rt
        .run_turn(&id, &script(&["reply again"]), opts(), |_| {})
        .await
        .unwrap();
    assert_eq!(
        (out.turns, out.start_ordinal, out.usage.input_tokens),
        (1, 1, 0)
    );
    rig.rt.shutdown().await;
}

/// Background work keeps the step open; the agent's own follow-up turn is
/// part of it (claude, codex and the scripted driver all start one).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_work_and_the_agents_own_turn_belong_to_the_step() {
    for (label, binaries, p) in [
        ("claude", true, RuntimeProvider::Claude),
        ("codex", true, RuntimeProvider::Codex),
        ("scripted", false, RuntimeProvider::Codex),
    ] {
        let rig = Rig::new(binaries).await;
        let id = rig.thread(p).await;
        let seen = Seen::default();
        let started = Instant::now();
        let out = rig
            .rt
            .run_turn(
                &id,
                &script(&["reply started", "background 800", "reply all done"]),
                opts(),
                seen.on(),
            )
            .await
            .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(800), "{label}");
        assert_eq!(out.ended, Ended::Settled, "{label}");
        assert_eq!(out.turns, 2, "{label}: the agent's own turn counts");
        assert!(out.answer.contains("all done"), "{label}: {}", out.answer);
        assert!(
            seen.all().contains(&StepEvent::Finishing),
            "{label}: {:?}",
            seen.all()
        );
        rig.rt.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_question_and_an_approval_need_you_then_resume_with_the_answer() {
    let rig = Rig::new(false).await;
    let id = rig.thread(RuntimeProvider::Claude).await;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let rt = rig.rt.clone();
    let tid = id.clone();
    let wait = tokio::spawn(async move {
        rt.run_turn(
            &tid,
            &script(&["ask Which branch?", "approve make clean", "reply finished"]),
            opts(),
            move |e| {
                let _ = tx.send(e);
            },
        )
        .await
    });
    let q = need(&mut rx).await;
    assert_eq!(q.kind, AttentionKind::Question);
    assert!(q.text.contains("Which branch?"), "{}", q.text);
    assert_eq!(q.thread_id.as_ref(), Some(&id));
    let r = rig
        .host
        .events
        .lock()
        .unwrap()
        .iter()
        .find_map(|e| match &e.body {
            RuntimeEventBody::RuntimeRequestUpdated { runtime_request: r }
                if r.status == RuntimeRequestStatus::Pending =>
            {
                Some(r.clone())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(q.request_id.as_ref(), Some(&r.id));
    let answer = vec![QuestionAnswer::Text {
        question_id: r.questions[0].id.clone(),
        text: "main".into(),
    }];
    rig.rt.respond_question(&id, &r.id, answer).await.unwrap();
    let a = need(&mut rx).await;
    assert_eq!(a.kind, AttentionKind::Approval);
    assert!(a.text.starts_with("wants approval"), "{}", a.text);
    rig.rt
        .respond(
            &id,
            a.request_id.as_ref().unwrap(),
            ApprovalDecision::Accept,
        )
        .await
        .unwrap();
    let out = wait.await.unwrap().unwrap();
    assert_eq!(out.ended, Ended::Settled);
    assert!(out.answer.contains("finished"), "{}", out.answer);
    let mut resumed = Vec::new();
    while let Ok(e) = rx.try_recv() {
        if let StepEvent::Resumed { answer, .. } = e {
            resumed.push(answer.unwrap_or_default());
        }
    }
    assert!(
        resumed.iter().any(|a| a.contains("Which branch?: main")),
        "{resumed:?}"
    );
    assert!(resumed.iter().any(|a| a == "approved"), "{resumed:?}");
    rig.rt.shutdown().await;
}

/// The budget runs while a person is asked; when it passes the wait ends
/// as a deadline that names what it waited on. No progress never fires
/// while a request waits.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_deadline_names_the_wait_and_no_progress_spares_a_pending_request() {
    let rig = Rig::new(false).await;
    let id = rig.thread(RuntimeProvider::Claude).await;
    let mut o = opts();
    o.deadline = Some(Duration::from_millis(1500));
    o.no_progress = Some(Duration::from_millis(300));
    let out = rig
        .rt
        .run_turn(&id, &script(&["approve deploy prod"]), o, |_| {})
        .await
        .unwrap();
    assert_eq!(out.ended, Ended::Deadline);
    assert!(
        out.waited_on
            .as_deref()
            .is_some_and(|w| w.contains("deploy prod")),
        "{:?}",
        out.waited_on
    );
    let mut o = opts();
    o.no_progress = Some(Duration::from_millis(400));
    let out = rig
        .rt
        .run_turn(&id, &script(&["sleep 5000"]), o, |_| {})
        .await
        .unwrap();
    assert_eq!(out.ended, Ended::NoProgress);
    rig.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cancel_ends_the_wait_and_stops_the_turn() {
    let rig = Rig::new(false).await;
    let id = rig.thread(RuntimeProvider::Claude).await;
    let o = opts();
    let cancel = o.cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        cancel.cancel();
    });
    let started = Instant::now();
    let out = rig
        .rt
        .run_turn(&id, &script(&["sleep 20000"]), o, |_| {})
        .await
        .unwrap();
    assert_eq!(out.ended, Ended::Cancelled);
    assert!(started.elapsed() < Duration::from_secs(5));
    rig.rt.shutdown().await;
}

/// A step's agent opens a sub-thread, which opens its own: the step is not
/// settled while the grandchild works, and the child's answer (queued on
/// the step after a gap) wakes the step for one more turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sub_threads_and_their_sub_threads_keep_the_step_open() {
    let rig = Rig::new(false).await;
    let step = rig.thread(RuntimeProvider::Claude).await;
    rig.rt
        .run_turn(&step, &script(&["reply ready"]), opts(), |_| {})
        .await
        .unwrap();
    let open = |prompt: &str| OpenPeerReq {
        provider: RuntimeProvider::Codex,
        prompt: Some(prompt.into()),
        title: None,
        model: None,
        effort: None,
        cwd: None,
        meta: None,
    };
    let child = rig
        .rt
        .open_peer(
            &step,
            open("OTTO_FAKE: sleep 300\nOTTO_FAKE: reply child up"),
        )
        .await
        .unwrap();
    let started = Instant::now();
    rig.rt
        .open_peer(
            &child.id,
            open("OTTO_FAKE: sleep 1500\nOTTO_FAKE: reply grandchild done"),
        )
        .await
        .unwrap();
    let out = rig
        .rt
        .await_settled(&step, 1, opts(), |_| {})
        .await
        .unwrap();
    assert!(
        started.elapsed() >= Duration::from_millis(1500),
        "settled at {:?}, before the grandchild finished",
        started.elapsed()
    );
    assert_eq!(out.ended, Ended::Settled);
    assert!(out.turns >= 1, "the child's answer woke the step");
    assert!(out.sub_threads.len() >= 2, "{:?}", out.sub_threads);
    rig.rt.shutdown().await;
}

/// An unheld thread whose turn a restart cut off: the wait ends as a
/// server restart at once — never as a cancel or a stall.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_restart_that_cut_the_turn_ends_the_wait_as_server_restart() {
    let rig = Rig::new(false).await;
    let id = rig.thread(RuntimeProvider::Claude).await;
    rig.rt
        .prompt_with(&id, &script(&["sleep 20000"]))
        .await
        .unwrap();
    rig.rt.shutdown().await;
    let rt = Rig::runtime(&rig.pool, &rig.host, &rig.drivers, &rig.root);
    rt.recover().await.unwrap();
    let started = Instant::now();
    let out = rt.await_settled(&id, 0, opts(), |_| {}).await.unwrap();
    assert_eq!(out.ended, Ended::ServerRestart);
    assert!(started.elapsed() < Duration::from_secs(2));
    // Consumed once: the next wait is an ordinary one.
    let out = rt
        .run_turn(&id, &script(&["reply back"]), opts(), |_| {})
        .await
        .unwrap();
    assert_eq!(out.ended, Ended::Settled);
    rt.shutdown().await;
}
