//! RT: unattended turns settle on events (runtime.md §1.4). Every case runs
//! a real `AgentRuntime` on the scripted driver or the fake CLIs.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::Utc;
use otto_agent_protocol::claude::ClaudeDriver;
use otto_agent_protocol::codex::CodexDriver;
use otto_agent_protocol::process::ChildTransport;
use otto_agent_protocol::scripted;
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
    /// Each spawn's pre-admitted key (`admit_spawn`).
    spawns: Mutex<Vec<Option<String>>>,
    /// Each registered process's exit watch (`register_runtime`).
    registered: Mutex<Vec<tokio::sync::watch::Receiver<Option<i32>>>>,
    critical: std::sync::atomic::AtomicBool,
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
    fn admit_spawn(&self, _s: &RuntimeSession, key: Option<String>) -> Box<dyn Send> {
        self.spawns.lock().unwrap().push(key);
        Box::new(())
    }
    async fn register_runtime(
        &self,
        _s: &RuntimeSession,
        _pid: Option<u32>,
        exit: tokio::sync::watch::Receiver<Option<i32>>,
    ) {
        self.registered.lock().unwrap().push(exit);
    }
    async fn memory_critical(&self) -> bool {
        self.critical.load(std::sync::atomic::Ordering::SeqCst)
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
    transport: Option<Arc<dyn ChildTransport>>,
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
                ("OTTO_FAKE_ARGV_LOG".into(), path("argv.log")),
            ],
            events: Mutex::default(),
            spawns: Mutex::default(),
            registered: Mutex::default(),
            critical: Default::default(),
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
            scripted::drivers()
        };
        let transport = (!binaries).then(|| scripted::transport(Vec::new()));
        let pool = pool().await;
        let rt = Self::runtime(&pool, &host, &drivers, &transport, &root);
        Self {
            rt,
            pool,
            host,
            drivers,
            transport,
            _dir: dir,
            root,
        }
    }

    fn runtime(
        pool: &DbPool,
        host: &Arc<Host>,
        drivers: &Arc<dyn DriverFactory>,
        transport: &Option<Arc<dyn ChildTransport>>,
        root: &std::path::Path,
    ) -> AgentRuntime {
        AgentRuntime::new(
            AgentRuntimeRepo::new(pool.clone()),
            Arc::clone(host) as Arc<dyn RuntimeHost>,
            Arc::clone(drivers),
            RuntimeConfig {
                attachments_dir: root.join("attachments"),
                transport: transport.clone(),
                ..RuntimeConfig::default()
            },
        )
    }

    /// A second runtime on the same store (a daemon restart).
    fn again(&self) -> AgentRuntime {
        Self::runtime(
            &self.pool,
            &self.host,
            &self.drivers,
            &self.transport,
            &self.root,
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
    need_seen(rx, &mut Vec::new()).await
}

/// [`need`], keeping every other event it passes over in `seen`.
async fn need_seen(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<StepEvent>,
    seen: &mut Vec<StepEvent>,
) -> Attention {
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Some(StepEvent::NeedsYou(a))) => return a,
            Ok(Some(e)) => seen.push(e),
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
    rig.rt.note_resolver(&id, &r.id, "u1").await;
    rig.rt.respond_question(&id, &r.id, answer).await.unwrap();
    let mut seen = Vec::new();
    let a = need_seen(&mut rx, &mut seen).await;
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
    let mut by = Vec::new();
    while let Ok(e) = rx.try_recv() {
        seen.push(e);
    }
    for e in seen {
        if let StepEvent::Resumed {
            answer, by: who, ..
        } = e
        {
            resumed.push(answer.unwrap_or_default());
            by.push(who);
        }
    }
    assert_eq!(
        by,
        [Some("u1".to_string()), None],
        "the answering person, then nobody named"
    );
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
    let rt = rig.again();
    rt.recover().await.unwrap();
    // A second restart before the step resumes still knows (the mark is
    // on the row, not in memory).
    rt.shutdown().await;
    let rt = rig.again();
    rt.recover().await.unwrap();
    let started = Instant::now();
    let out = rt.await_settled(&id, 0, opts(), |_| {}).await.unwrap();
    assert_eq!(out.ended, Ended::ServerRestart);
    assert!(started.elapsed() < Duration::from_secs(2));
    // The next turn is not the cut one: an ordinary wait.
    let out = rt
        .run_turn(&id, &script(&["reply back"]), opts(), |_| {})
        .await
        .unwrap();
    assert_eq!(out.ended, Ended::Settled);
    rt.shutdown().await;
}

// ---- ThreadStepRunner (runtime.md §1.5–§1.9) --------------------------------

use otto_agent_protocol::step_runner::{
    NoStepHost, Place, StepHost, ThreadStepRunner, CONTINUE_AFTER_LIMIT,
};
use otto_agent_run::step::{StepErrorClass, StepOutcome, StepRunner, StepTurn};

impl Rig {
    fn runner(&self) -> ThreadStepRunner {
        ThreadStepRunner::new(self.rt.clone(), Arc::new(NoStepHost))
    }

    fn turn(&self, provider: &str, prompt: &str) -> StepTurn {
        StepTurn {
            user_id: "u1".into(),
            workspace_id: "w1".into(),
            provider: provider.into(),
            model: None,
            effort: None,
            cwd: self.root.clone(),
            title: "WF test · step · run abc123".into(),
            meta: json!({"source": "workflow", "run_id": "r1", "node_id": "n1"}),
            prompt: prompt.into(),
            continue_thread: None,
            handoff: None,
            read_only: false,
            timeout: None,
            no_progress: None,
        }
    }

    fn starts(&self) -> usize {
        std::fs::read_to_string(self.root.join("argv.log"))
            .unwrap_or_default()
            .lines()
            .filter(|l| l.contains("\"argv\""))
            .count()
    }
}

fn fake(verbs: &[&str]) -> String {
    verbs.iter().map(|v| format!("OTTO_FAKE: {v}\n")).collect()
}

async fn run(runner: &ThreadStepRunner, t: StepTurn) -> (StepOutcome, Vec<StepEvent>) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let out = runner.run(t, tx, CancelSignal::new()).await.unwrap();
    let mut evs = Vec::new();
    while let Ok(e) = rx.try_recv() {
        evs.push(e);
    }
    (out, evs)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_step_runs_in_order_and_a_missing_handoff_gets_one_nudge() {
    let rig = Rig::new(false).await;
    let runner = rig.runner();
    let handoff = rig.root.join("step1-a.md");
    let mut t = rig.turn("claude", &fake(&["usage 10 5", "reply looked around"]));
    t.handoff = Some(handoff.clone());
    let (out, evs) = run(&runner, t.clone()).await;
    assert!(
        matches!(
            &evs[0],
            StepEvent::Started {
                start_ordinal: 0,
                ..
            }
        ),
        "{evs:?}"
    );
    assert_eq!(evs[1], StepEvent::Working);
    assert!(evs.contains(&StepEvent::Finishing), "{evs:?}");
    assert_eq!(out.error, None);
    assert_eq!(out.turns, 2, "the step's turn and exactly one nudge");
    assert!(!out.handoff_written);
    assert!(
        out.reply.starts_with("echo: You did not write"),
        "{}",
        out.reply
    );
    assert!(out.usage.input_tokens >= 10, "{:?}", out.usage);
    let digest = out.digest.unwrap();
    assert_eq!(digest.turns, 2);
    assert!(serde_json::to_vec(&digest).unwrap().len() <= 8 * 1024);
    // Written in the first turn: no nudge.
    t.prompt = fake(&[
        &format!("write {} the summary", handoff.display()),
        "reply done",
    ]);
    let (out, _) = run(&runner, t).await;
    assert_eq!((out.turns, out.handoff_written), (1, true));
    assert_eq!(out.reply, "done");
    rig.rt.shutdown().await;
}

/// `continue_from`: the same warm process, a closed thread reopened, a
/// released one respawned with resume; a deleted one or another provider
/// is refused with `spawn`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn continuing_an_earlier_steps_thread() {
    let rig = Rig::new(true).await;
    let runner = rig.runner();
    let (first, _) = run(
        &runner,
        rig.turn("claude", &fake(&["remember blue", "reply one"])),
    )
    .await;
    assert_eq!(first.error, None);
    let starts = rig.starts();
    let mut next = rig.turn("claude", &fake(&["recall", "reply two"]));
    next.continue_thread = Some(first.thread_id.clone());
    let (out, evs) = run(&runner, next.clone()).await;
    assert_eq!(out.thread_id, first.thread_id);
    assert!(
        matches!(
            &evs[0],
            StepEvent::Started {
                start_ordinal: 1,
                ..
            }
        ),
        "{evs:?}"
    );
    assert_eq!(out.turns, 1);
    assert_eq!(rig.starts(), starts, "the warm process, no second spawn");
    // Released between steps: the next turn respawns, resuming the conversation.
    runner.release_process(&first.thread_id).await;
    let (out, _) = run(&runner, next.clone()).await;
    assert_eq!(out.error, None, "{:?}", out.error);
    assert_eq!(rig.starts(), starts + 1);
    // Closed but retained: reopened without a spawn; the turn's own spawn is
    // the only one (DR-115.1).
    runner
        .close(&first.thread_id, Utc::now() + chrono::TimeDelta::hours(1))
        .await;
    let before = rig.starts();
    let (out, _) = run(&runner, next.clone()).await;
    assert_eq!(out.error, None, "{:?}", out.error);
    assert_eq!(rig.starts(), before + 1, "exactly one spawn");
    // Another provider.
    let mut other = next.clone();
    other.provider = "codex".into();
    let (out, _) = run(&runner, other).await;
    assert_eq!(out.error.unwrap().class, StepErrorClass::Spawn);
    // Deleted by retention.
    runner.delete(&first.thread_id).await;
    let (out, _) = run(&runner, next).await;
    let e = out.error.unwrap();
    assert_eq!(e.class, StepErrorClass::Spawn);
    assert!(e.message.contains("deleted by retention"), "{}", e.message);
    rig.rt.shutdown().await;
}

/// Counts its admissions; a usage limit resets at `.0`.
struct ResetsSoon(chrono::DateTime<Utc>, std::sync::atomic::AtomicUsize);

#[async_trait::async_trait]
impl StepHost for ResetsSoon {
    async fn admit(
        &self,
        _: &str,
        _: &str,
        on_wait: &(dyn Fn(otto_core::agent_runtime::WaitReason) + Send + Sync),
    ) -> Place {
        self.1.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        on_wait(otto_core::agent_runtime::WaitReason {
            code: "slots".into(),
            text: "Waiting for a free agent slot — 1 ahead in line".into(),
            since: Utc::now(),
            detail: None,
        });
        Place {
            key: None,
            guard: Box::new(()),
        }
    }
    async fn usage_resets_at(
        &self,
        _: &str,
        _: &serde_json::Value,
    ) -> Option<chrono::DateTime<Utc>> {
        Some(self.0)
    }
}

/// A plan limit whose reset Otto knows parks the step and continues the same
/// thread at the reset; without one the step fails as `usage_limit`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_usage_limit_with_a_known_reset_waits_and_continues_the_same_thread() {
    let rig = Rig::new(false).await;
    let t = rig.turn("claude", &fake(&["fail usage_limit_no_reset"]));
    let (out, _) = run(&rig.runner(), t.clone()).await;
    assert_eq!(out.error.unwrap().class, StepErrorClass::UsageLimit);
    let at = Utc::now() + chrono::TimeDelta::seconds(2);
    let host = Arc::new(ResetsSoon(at, Default::default()));
    let runner = ThreadStepRunner::new(rig.rt.clone(), Arc::clone(&host) as Arc<dyn StepHost>);
    let started = Instant::now();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(async move { runner.run(t, tx, CancelSignal::new()).await });
    let mut evs = Vec::new();
    let mut parked_without_process = None;
    while let Some(e) = rx.recv().await {
        if let StepEvent::NeedsYou(a) = &e {
            if a.kind == AttentionKind::UsageLimit {
                // DR-115.4: parked = no process, no machine place.
                tokio::time::sleep(Duration::from_millis(200)).await;
                let id = a.thread_id.clone().unwrap();
                parked_without_process = Some(!rig.rt.has_process(&id).await.unwrap());
            }
        }
        evs.push(e);
    }
    let out = task.await.unwrap().unwrap();
    assert!(started.elapsed() >= Duration::from_millis(1500));
    assert!(
        matches!(&evs[0], StepEvent::Waiting(w) if w.code == "slots"),
        "{evs:?}"
    );
    let limit = evs.iter().find_map(|e| match e {
        StepEvent::NeedsYou(a) if a.kind == AttentionKind::UsageLimit => Some(a.clone()),
        _ => None,
    });
    assert_eq!(limit.unwrap().resets_at, Some(at));
    assert_eq!(
        parked_without_process,
        Some(true),
        "the process is released at park"
    );
    assert_eq!(
        host.1.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "admitted for the create and again at the wake"
    );
    assert!(evs.iter().any(|e| matches!(e, StepEvent::Resumed { .. })));
    assert_eq!(out.error, None, "{:?}", out.error);
    assert_eq!(out.turns, 2);
    assert_eq!(out.reply, format!("echo: {CONTINUE_AFTER_LIMIT}"));
    rig.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_step_timeout_while_an_approval_waits_names_it() {
    let rig = Rig::new(false).await;
    let mut t = rig.turn("claude", &fake(&["approve git push --force"]));
    t.timeout = Some(Duration::from_millis(800));
    let (out, evs) = run(&rig.runner(), t).await;
    assert!(evs.iter().any(|e| matches!(e, StepEvent::NeedsYou(_))));
    let e = out.error.unwrap();
    assert_eq!(e.class, StepErrorClass::Timeout);
    assert!(e.message.contains("git push --force"), "{}", e.message);
    rig.rt.shutdown().await;
}

/// Close stamps `retain_until` on the step and its sub-threads; the run's
/// end moves it; the sweep deletes closed threads past it and nothing else.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retention_stamps_closes_and_sweeps_only_what_is_due() {
    let rig = Rig::new(false).await;
    let runner = rig.runner();
    let (step, _) = run(&runner, rig.turn("claude", &fake(&["reply ok"]))).await;
    let open = OpenPeerReq {
        provider: RuntimeProvider::Codex,
        prompt: Some("OTTO_FAKE: reply sub".into()),
        title: None,
        model: None,
        effort: None,
        cwd: None,
        meta: None,
    };
    let sub = rig.rt.open_peer(&step.thread_id, open).await.unwrap();
    let person = rig.thread(RuntimeProvider::Claude).await;
    rig.rt.close(&person).await.unwrap();
    let soon = Utc::now() + chrono::TimeDelta::minutes(5);
    runner.close(&step.thread_id, soon).await;
    for id in [&step.thread_id, &sub.id] {
        let s = rig.rt.get(id).await.unwrap();
        assert_eq!(
            s.status,
            otto_core::agent_runtime::RuntimeSessionStatus::Closed,
            "{id}"
        );
        assert_eq!(
            s.meta.unwrap()["retain_until"],
            otto_agent_protocol::runtime::retain_stamp(soon)
        );
    }
    assert_eq!(rig.rt.sweep_retained(Utc::now()).await.unwrap(), 0);
    // The run failed: kept longer. Unknown ids are skipped.
    let later = soon + chrono::TimeDelta::days(7);
    runner
        .set_retain_until(&[step.thread_id.clone(), "gone".into()], later)
        .await;
    assert_eq!(
        rig.rt
            .sweep_retained(soon + chrono::TimeDelta::hours(1))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        rig.rt
            .sweep_retained(later + chrono::TimeDelta::seconds(1))
            .await
            .unwrap(),
        2
    );
    assert!(rig.rt.get(&step.thread_id).await.is_err());
    assert!(rig.rt.get(&sub.id).await.is_err());
    assert!(
        rig.rt.get(&person).await.is_ok(),
        "a person's thread has no date"
    );
    rig.rt.shutdown().await;
}

/// After a restart that cut an unheld step's turn, `resume` tells the agent
/// and finishes on the same thread; usage counts every turn of the step.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_after_a_restart_finishes_the_step_on_the_same_thread() {
    let rig = Rig::new(false).await;
    let runner = rig.runner();
    let t = rig.turn("claude", &fake(&["usage 7 3", "sleep 20000"]));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let r2 = rig.runner();
    let t2 = t.clone();
    let task = tokio::spawn(async move { r2.run(t2, tx, CancelSignal::new()).await });
    let (id, start) = loop {
        match rx.recv().await {
            Some(StepEvent::Started {
                thread_id,
                start_ordinal,
            }) => break (thread_id, start_ordinal),
            Some(_) => continue,
            None => panic!("no start"),
        }
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    rig.rt.shutdown().await;
    task.abort();
    drop(runner);
    let rt = rig.again();
    rt.recover().await.unwrap();
    let runner = ThreadStepRunner::new(rt.clone(), Arc::new(NoStepHost));
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let out = runner
        .resume(&id, start, &t, tx, CancelSignal::new())
        .await
        .unwrap();
    assert_eq!(out.error, None, "{:?}", out.error);
    assert_eq!(out.thread_id, id);
    assert_eq!(out.turns, 2, "the cut turn and the restart turn");
    assert!(
        out.reply.starts_with("echo: Otto restarted"),
        "{}",
        out.reply
    );
    rt.shutdown().await;
}

/// A step agent's sub-threads belong to its run: they inherit the run's
/// keys (so no person's list shows them), at most 4 are open at once, and
/// a step cannot ask for a secret nobody is there to type.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_steps_sub_threads_inherit_its_run_and_are_capped() {
    let rig = Rig::new(false).await;
    let step = rig.thread(RuntimeProvider::Claude).await;
    let open = || OpenPeerReq {
        provider: RuntimeProvider::Codex,
        prompt: None,
        title: None,
        model: None,
        effort: None,
        cwd: None,
        meta: None,
    };
    let mut subs = Vec::new();
    for _ in 0..4 {
        subs.push(rig.rt.open_peer(&step, open()).await.unwrap());
    }
    let meta = subs[0].meta.clone().unwrap();
    assert_eq!(
        (meta["source"].as_str(), meta["run_id"].as_str()),
        (Some("workflow"), Some("r1"))
    );
    let fifth = rig.rt.open_peer(&step, open()).await.unwrap_err();
    assert!(fifth.to_string().contains("4 open sub-threads"), "{fifth}");
    let listed = rig.rt.list(&"w1".into(), None).await.unwrap();
    assert!(
        listed.is_empty(),
        "{:?}",
        listed.iter().map(|s| &s.id).collect::<Vec<_>>()
    );
    let (events, _) = tokio::sync::broadcast::channel(4);
    let refused = rig
        .rt
        .request_secret(&events, &json!({"prompt": "the deploy token"}), Some(&step))
        .await
        .unwrap_err();
    assert!(
        refused.to_string().contains("cannot ask for a secret"),
        "{refused}"
    );
    rig.rt.shutdown().await;
}

/// A governed Otto MCP call the step's thread made waits on an approval
/// card: the run hears "Needs you" (via mcp) and the decision.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_pending_mcp_approval_card_needs_you_until_it_is_decided() {
    use otto_agent_protocol::step_runner::DaemonStepHost;
    let rig = Rig::new(false).await;
    let (bus, _keep) = tokio::sync::broadcast::channel(16);
    let repo = otto_state::McpApprovalRepo::new(rig.pool.clone());
    let card = repo
        .create(otto_state::NewApproval {
            workspace_id: Some("w1".into()),
            kind: "tool_call".into(),
            server_id: None,
            server_name: Some("otto".into()),
            tool: Some("otto.comment_pr".into()),
            title: "Comment on PR #12".into(),
            detail: None,
            args_redacted_json: "{}".into(),
            args_hash: None,
            risk_label: None,
            requested_by: Some("agent".into()),
            requested_by_kind: Some("session".into()),
            requested_by_session_id: Some("t1".into()),
            expires_at: None,
        })
        .await
        .unwrap();
    let manager = otto_sessions::SessionManager::new(
        otto_state::SessionsRepo::new(rig.pool.clone()),
        bus.clone(),
        otto_sessions::ProviderRegistry::new(None),
    );
    let host = Arc::new(DaemonStepHost {
        manager: Arc::new(manager),
        approvals: Some((repo.clone(), bus.clone())),
        resets: None,
    });
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let (h, rt) = (Arc::clone(&host), rig.rt.clone());
    let watch = tokio::spawn(async move { h.watch_mcp(&rt, &"t1".into(), tx).await });
    let a = need(&mut rx).await;
    assert_eq!(a.via, Some(otto_core::agent_runtime::AttentionVia::Mcp));
    assert_eq!(a.request_id.as_ref(), Some(&card.id));
    assert_eq!(a.text, "Comment on PR #12");
    repo.decide(&card.id, true, "u1", None).await.unwrap();
    let _ = bus.send(otto_core::event::Event::McpApprovalChanged {
        approval_id: Some(card.id.clone()),
        workspace_id: Some("w1".into()),
        status: "approved".into(),
    });
    let resumed = tokio::time::timeout(Duration::from_secs(5), rx.recv())
        .await
        .unwrap()
        .unwrap();
    let StepEvent::Resumed { request_id, by, .. } = resumed else {
        panic!("{resumed:?}");
    };
    assert_eq!((request_id, by), (Some(card.id.clone()), Some("u1".into())));
    watch.abort();
    rig.rt.shutdown().await;
}

/// A continued step whose provider cannot resume the conversation fails as
/// `provider` — never a fresh agent that remembers nothing; a person's
/// thread keeps today's fresh start.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_refused_resume_fails_the_step_instead_of_starting_fresh() {
    let rig = Rig::new(true).await;
    let runner = rig.runner();
    let (first, _) = run(&runner, rig.turn("claude", &fake(&["reply one"]))).await;
    assert_eq!(first.error, None);
    runner.release_process(&first.thread_id).await;
    tokio::fs::remove_dir_all(rig.root.join(".fake-claude"))
        .await
        .unwrap();
    let mut next = rig.turn("claude", &fake(&["reply two"]));
    next.continue_thread = Some(first.thread_id.clone());
    let (out, _) = run(&runner, next).await;
    let e = out.error.unwrap();
    assert_eq!(e.class, StepErrorClass::Provider);
    assert!(e.message.contains("could not resume"), "{}", e.message);
    // A person's thread: the same refusal starts fresh, as today.
    let person = rig
        .rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                title_auto: false,
                provider: Some(RuntimeProvider::Claude),
                cwd: Some(rig.root.display().to_string()),
                title: None,
                model: None,
                access_mode: None,
                meta: None,
            },
        )
        .await
        .unwrap()
        .id;
    rig.rt
        .run_turn(&person, &script(&["reply hi"]), opts(), |_| {})
        .await
        .unwrap();
    rig.rt.release_process(&person).await.unwrap();
    tokio::fs::remove_dir_all(rig.root.join(".fake-claude"))
        .await
        .unwrap();
    let out = rig
        .rt
        .run_turn(&person, &script(&["reply again"]), opts(), |_| {})
        .await
        .unwrap();
    assert!(out.answer.contains("again"), "{}", out.answer);
    rig.rt.shutdown().await;
}

/// Every runtime spawn asks the host for its machine place with the
/// pre-admitted key (the create's, stripped before the row; a turn's for a
/// respawn) and is counted until its process is gone (DR-009, DR-115.4).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spawns_claim_their_pre_admitted_place_and_stay_counted_while_they_run() {
    let rig = Rig::new(false).await;
    let req = CreateRuntimeSessionReq {
        title_auto: false,
        provider: Some(RuntimeProvider::Claude),
        cwd: Some(rig.root.display().to_string()),
        title: None,
        model: None,
        access_mode: None,
        meta: Some(json!({"source": "workflow", "run_id": "r1", "admission_permit": "k1"})),
    };
    let s = rig
        .rt
        .create(&"u1".into(), &"w1".into(), req)
        .await
        .unwrap();
    assert_eq!(*rig.host.spawns.lock().unwrap(), [Some("k1".to_string())]);
    let stored = rig
        .rt
        .repo()
        .get(&s.id)
        .await
        .unwrap()
        .session
        .meta
        .unwrap();
    assert!(stored.get("admission_permit").is_none(), "{stored}");
    let mut first = rig.host.registered.lock().unwrap()[0].clone();
    assert!(first.borrow().is_none(), "counted while it runs");
    rig.rt.release_process(&s.id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), first.changed())
        .await
        .unwrap()
        .unwrap_err();
    // A respawn takes the place its turn was given.
    let mut o = opts();
    o.permit = Some("k2".into());
    rig.rt
        .run_turn(&s.id, &script(&["reply again"]), o, |_| {})
        .await
        .unwrap();
    assert_eq!(
        *rig.host.spawns.lock().unwrap(),
        [Some("k1".to_string()), Some("k2".to_string())]
    );
    assert_eq!(rig.host.registered.lock().unwrap().len(), 2);
    // Under critical memory pressure a step's agent opens no sub-thread.
    rig.host
        .critical
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let refused = rig
        .rt
        .open_peer(
            &s.id,
            OpenPeerReq {
                provider: RuntimeProvider::Codex,
                prompt: None,
                title: None,
                model: None,
                effort: None,
                cwd: None,
                meta: None,
            },
        )
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("short of memory"), "{refused}");
    rig.rt.shutdown().await;
}

/// A sub-thread the step's agent opened waits on an approval card: the
/// step's run hears "Needs you" for it too, naming the sub-thread.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_sub_threads_pending_mcp_approval_card_needs_you_too() {
    use otto_agent_protocol::step_runner::DaemonStepHost;
    let rig = Rig::new(false).await;
    let step = rig.thread(RuntimeProvider::Claude).await;
    let open = OpenPeerReq {
        provider: RuntimeProvider::Codex,
        prompt: None,
        title: None,
        model: None,
        effort: None,
        cwd: None,
        meta: None,
    };
    let sub = rig.rt.open_peer(&step, open).await.unwrap();
    let (bus, _keep) = tokio::sync::broadcast::channel(16);
    let repo = otto_state::McpApprovalRepo::new(rig.pool.clone());
    let card = repo
        .create(otto_state::NewApproval {
            workspace_id: Some("w1".into()),
            kind: "tool_call".into(),
            server_id: None,
            server_name: Some("otto".into()),
            tool: Some("otto.comment_issue".into()),
            title: "Comment on PROJ-1".into(),
            detail: None,
            args_redacted_json: "{}".into(),
            args_hash: None,
            risk_label: None,
            requested_by: Some("agent".into()),
            requested_by_kind: Some("session".into()),
            requested_by_session_id: Some(sub.id.clone()),
            expires_at: None,
        })
        .await
        .unwrap();
    let manager = otto_sessions::SessionManager::new(
        otto_state::SessionsRepo::new(rig.pool.clone()),
        bus.clone(),
        otto_sessions::ProviderRegistry::new(None),
    );
    let host = DaemonStepHost {
        manager: Arc::new(manager),
        approvals: Some((repo, bus)),
        resets: None,
    };
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let rt = rig.rt.clone();
    let watch = tokio::spawn(async move { host.watch_mcp(&rt, &step, tx).await });
    let a = need(&mut rx).await;
    assert_eq!(a.request_id.as_ref(), Some(&card.id));
    assert_eq!(a.thread_id.as_ref(), Some(&sub.id));
    watch.abort();
    rig.rt.shutdown().await;
}

/// Never admits: the step waits for its machine place until the restart.
struct NeverAdmits(tokio::sync::Notify);

#[async_trait::async_trait]
impl StepHost for NeverAdmits {
    async fn admit(
        &self,
        _: &str,
        _: &str,
        _: &(dyn Fn(otto_core::agent_runtime::WaitReason) + Send + Sync),
    ) -> Place {
        self.0.notify_one();
        std::future::pending().await
    }
}

/// CR1-003: the daemon restarts while a continued step waits for its
/// machine place — after `Started`, before its prompt went out. `resume`
/// sends the step's prompt; it never ends "done" with no turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_after_a_restart_before_the_prompt_went_out_sends_it() {
    let rig = Rig::new(false).await;
    let (first, _) = run(&rig.runner(), rig.turn("claude", &fake(&["reply one"]))).await;
    assert_eq!(first.error, None);
    rig.runner().release_process(&first.thread_id).await;
    let mut t = rig.turn("claude", &fake(&["usage 4 2", "reply two"]));
    t.continue_thread = Some(first.thread_id.clone());
    let host = Arc::new(NeverAdmits(tokio::sync::Notify::new()));
    let runner = ThreadStepRunner::new(rig.rt.clone(), Arc::clone(&host) as Arc<dyn StepHost>);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let t2 = t.clone();
    let task = tokio::spawn(async move { runner.run(t2, tx, CancelSignal::new()).await });
    host.0.notified().await;
    let start = loop {
        match rx.try_recv() {
            Ok(StepEvent::Started { start_ordinal, .. }) => break start_ordinal,
            Ok(_) => continue,
            Err(e) => panic!("no start before the wait: {e:?}"),
        }
    };
    rig.rt.shutdown().await;
    task.abort();
    let rt = rig.again();
    rt.recover().await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let out = ThreadStepRunner::new(rt.clone(), Arc::new(NoStepHost))
        .resume(&first.thread_id, start, &t, tx, CancelSignal::new())
        .await
        .unwrap();
    assert_eq!(out.error, None, "{:?}", out.error);
    assert_eq!(out.turns, 1, "the step's own prompt went out");
    assert_eq!(out.reply, "two");
    assert_eq!((out.usage.input_tokens, out.usage.output_tokens), (4, 2));
    rt.shutdown().await;
}

/// CR1-003: an unheld restart kills the background work a step's finished
/// turn left running: `resume` tells the agent, never "completed".
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_after_a_restart_killed_background_work_tells_the_agent() {
    let rig = Rig::new(false).await;
    let t = rig.turn("claude", &fake(&["background 20000", "reply after it"]));
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let runner = rig.runner();
    let t2 = t.clone();
    let task = tokio::spawn(async move { runner.run(t2, tx, CancelSignal::new()).await });
    let mut started = None;
    loop {
        match tokio::time::timeout(Duration::from_secs(10), rx.recv()).await {
            Ok(Some(StepEvent::Started {
                thread_id,
                start_ordinal,
            })) => started = Some((thread_id, start_ordinal)),
            // The turn ended; its background work still runs.
            Ok(Some(StepEvent::Finishing)) => break,
            Ok(Some(_)) => continue,
            other => panic!("no Finishing: {other:?}"),
        }
    }
    let (id, start) = started.unwrap();
    rig.rt.shutdown().await;
    task.abort();
    let rt = rig.again();
    rt.recover().await.unwrap();
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let out = ThreadStepRunner::new(rt.clone(), Arc::new(NoStepHost))
        .resume(&id, start, &t, tx, CancelSignal::new())
        .await
        .unwrap();
    assert_eq!(out.error, None, "{:?}", out.error);
    assert_eq!(out.turns, 2, "the turn and the restart turn");
    assert!(
        out.reply.starts_with("echo: Otto restarted"),
        "{}",
        out.reply
    );
    rt.shutdown().await;
}

/// CR1-032: an agent removes two answered sub-threads in parallel tool
/// calls: both records stay on the step (their usage is not lost).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_removals_at_once_keep_both_records() {
    let rig = Rig::new(false).await;
    let step = rig.thread(RuntimeProvider::Claude).await;
    let open = |text: &str| OpenPeerReq {
        provider: RuntimeProvider::Claude,
        prompt: Some(format!("OTTO_FAKE: usage 5 1\nOTTO_FAKE: reply {text}")),
        title: None,
        model: None,
        effort: None,
        cwd: None,
        meta: None,
    };
    let c1 = rig.rt.open_peer(&step, open("one")).await.unwrap();
    let c2 = rig.rt.open_peer(&step, open("two")).await.unwrap();
    let out = rig
        .rt
        .await_settled(&step, 0, opts(), |_| {})
        .await
        .unwrap();
    assert_eq!(out.ended, Ended::Settled);
    let (a, b) = tokio::join!(
        rig.rt.close_peer(&step, &c1.id, true),
        rig.rt.close_peer(&step, &c2.id, true),
    );
    a.unwrap();
    b.unwrap();
    let kept = |meta: Option<serde_json::Value>| {
        meta.and_then(|m| m.get("removed_children").cloned())
            .and_then(|v| v.as_array().map(Vec::len))
    };
    assert_eq!(kept(rig.rt.get(&step).await.unwrap().meta), Some(2));
    rig.rt.shutdown().await;
    let rt = rig.again();
    assert_eq!(
        kept(rt.get(&step).await.unwrap().meta),
        Some(2),
        "as stored"
    );
    rt.shutdown().await;
}

/// CR1-053: a usage limit that resets after the step's time runs out ends
/// the step as `timeout` at once, with no "Needs you" for a wait it will
/// never do.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_usage_limit_past_the_steps_time_fails_it_without_needs_you() {
    let rig = Rig::new(false).await;
    let at = Utc::now() + chrono::TimeDelta::hours(3);
    let host = Arc::new(ResetsSoon(at, Default::default()));
    let runner = ThreadStepRunner::new(rig.rt.clone(), host as Arc<dyn StepHost>);
    let mut t = rig.turn("claude", &fake(&["fail usage_limit_no_reset"]));
    t.timeout = Some(Duration::from_secs(20 * 60));
    let (out, evs) = run(&runner, t).await;
    assert_eq!(out.error.unwrap().class, StepErrorClass::Timeout);
    assert!(
        !evs.iter().any(|e| matches!(e, StepEvent::NeedsYou(_))),
        "{evs:?}"
    );
    rig.rt.shutdown().await;
}

/// A step's sub-agents reach the engine as activity rows, as the PTY path's
/// phase feed sends them (the thread path sent none).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_steps_sub_agents_are_reported_as_activity() {
    let rig = Rig::new(false).await;
    let (_, evs) = run(&rig.runner(), rig.turn("claude", "agent 0.5")).await;
    let rows: Vec<_> = evs
        .iter()
        .filter_map(|e| match e {
            StepEvent::Activity(a) => Some(a.clone()),
            _ => None,
        })
        .collect();
    assert!(
        rows.iter()
            .any(|a| a.subagents.iter().any(|s| s.description == "PR sweep")),
        "{rows:?}"
    );
    assert!(rows.iter().any(|a| a.pending_tasks == 1), "{rows:?}");
}
