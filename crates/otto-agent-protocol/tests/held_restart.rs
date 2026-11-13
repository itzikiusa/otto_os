//! Threads survive a daemon restart: the real runtime + drivers against the
//! fake agents, each child run inside a stdio holder (this test binary is the
//! holder executable, see `holder_entry`). A "restart" = `shutdown()` (or,
//! for a crash, dropping the whole tokio runtime) and a NEW `AgentRuntime` on
//! the same DB whose `recover()` adopts the holders. Every provider family:
//! Claude (stream-json), Codex (app-server), ACP (Gemini and Antigravity
//! flavours over `otto-fake-agent`).

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use otto_agent_protocol::acp::AcpDriver;
use otto_agent_protocol::claude::ClaudeDriver;
use otto_agent_protocol::codex::CodexDriver;
use otto_agent_protocol::{
    AgentRuntime, DriverFactory, LaunchSpec, McpDelivery, McpServerDef, RuntimeConfig, RuntimeHost,
    StructuredDriver,
};
use otto_core::agent_runtime::{
    ApprovalDecision, CreateRuntimeSessionReq, PromptDelivery, RuntimeAccessMode, RuntimeEvent,
    RuntimeEventBody, RuntimePromptReq, RuntimeProvider, RuntimeReplay, RuntimeRequestStatus,
    RuntimeSession, RuntimeSessionStatus, TurnItemKind, TurnTerminalStatus,
};
use otto_core::domain::Workspace;
use otto_core::{Id, Result};
use otto_pty::holder::{HolderConfig, HolderLauncher};
use otto_state::{AgentRuntimeRepo, DbPool};

macro_rules! fake_bin {
    ($name:literal) => {
        std::env::var(concat!("NEXTEST_BIN_EXE_", $name))
            .unwrap_or_else(|_| env!(concat!("CARGO_BIN_EXE_", $name)).to_string())
    };
}
const WAIT: Duration = Duration::from_secs(20);

/// Holder entry point: only does anything in a process the launcher started.
#[test]
fn holder_entry() {
    otto_pty::holder::run_if_holder_process();
}

struct TestHost {
    root: String,
    holders: HolderConfig,
    spawns: Mutex<usize>,
    events: Mutex<Vec<Arc<RuntimeEvent>>>,
    notify: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl RuntimeHost for TestHost {
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
        s: &RuntimeSession,
        spec: &mut LaunchSpec,
        _mcp: McpDelivery,
    ) -> Result<Vec<McpServerDef>> {
        if s.provider == RuntimeProvider::Agy {
            spec.env.push((
                "OTTO_AGY_CONTEXT_DIR".into(),
                format!("{}/bundle", self.root),
            ));
        }
        *self.spawns.lock().unwrap() += 1;
        Ok(Vec::new())
    }
    fn publish(&self, _s: &RuntimeSession, event: Arc<RuntimeEvent>) {
        self.events.lock().unwrap().push(event);
        self.notify.notify_waiters();
    }
    fn stdio_holders(&self) -> Option<HolderConfig> {
        Some(self.holders.clone())
    }
}

impl TestHost {
    fn new(root: &Path, holders: &Path) -> Arc<Self> {
        let launcher = HolderLauncher::current_exe(vec![
            "holder_entry".into(),
            "--exact".into(),
            "--nocapture".into(),
            "--test-threads=1".into(),
        ])
        .unwrap();
        Arc::new(Self {
            root: root.to_str().unwrap().into(),
            holders: HolderConfig::new(holders, launcher),
            spawns: Mutex::default(),
            events: Mutex::default(),
            notify: tokio::sync::Notify::new(),
        })
    }

    fn all(&self) -> Vec<RuntimeEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| (**e).clone())
            .collect()
    }

    async fn wait_n(&self, pred: impl Fn(&RuntimeEvent) -> bool, n: usize) -> RuntimeEvent {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let hits: Vec<RuntimeEvent> = self.all().into_iter().filter(|e| pred(e)).collect();
            if hits.len() >= n {
                return hits[n - 1].clone();
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                panic!("timed out waiting for {n}; events: {:#?}", self.all());
            }
        }
    }

    async fn wait(&self, pred: impl Fn(&RuntimeEvent) -> bool) -> RuntimeEvent {
        self.wait_n(pred, 1).await
    }

    fn spawns(&self) -> usize {
        *self.spawns.lock().unwrap()
    }

    /// Every settled assistant text, in event order.
    fn answers(&self) -> Vec<String> {
        self.all()
            .into_iter()
            .filter_map(|e| match e.body {
                RuntimeEventBody::TurnItemUpdated { turn_item } => match turn_item.kind {
                    TurnItemKind::AssistantMessage {
                        text,
                        streaming: false,
                        ..
                    } => Some(text),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    /// Every assistant text this daemon showed is a prefix of `whole`: no
    /// chunk was shown twice or skipped.
    fn assert_prefixes(&self, whole: &str, what: &str) {
        for e in self.all() {
            if let RuntimeEventBody::TurnItemUpdated { turn_item } = e.body {
                if let TurnItemKind::AssistantMessage { text, .. } = turn_item.kind {
                    if text.starts_with("c0 ") {
                        assert!(whole.starts_with(&text), "{what}: shown {text:?}");
                    }
                }
            }
        }
    }
}

fn drivers() -> Arc<dyn DriverFactory> {
    Arc::new(|p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
        Some(match p {
            RuntimeProvider::Codex => {
                Box::new(CodexDriver::with_program(fake_bin!("otto-fake-codex")))
            }
            RuntimeProvider::Agy => Box::new(AcpDriver::agy(&fake_bin!("otto-fake-agent"))),
            RuntimeProvider::Claude => {
                Box::new(ClaudeDriver::with_program(fake_bin!("otto-fake-claude")))
            }
            _ => Box::new(AcpDriver::gemini(&fake_bin!("otto-fake-agent"))),
        })
    })
}

async fn pool() -> DbPool {
    pool_with(sqlx::sqlite::SqliteConnectOptions::new().in_memory(true)).await
}

/// A file-backed store: an in-memory one lives in its single connection,
/// which a "crashed" runtime may take down with it.
async fn file_pool(path: &Path) -> DbPool {
    pool_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true),
    )
    .await
}

async fn pool_with(opts: sqlx::sqlite::SqliteConnectOptions) -> DbPool {
    let opts = opts.foreign_keys(true);
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
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) \
         VALUES ('u1', 'u', 'x', 'U', 0, ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('w1', 'W', '/tmp', ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    pool.into()
}

fn runtime(pool: &DbPool, host: &Arc<TestHost>) -> AgentRuntime {
    AgentRuntime::new(
        AgentRuntimeRepo::new(pool.clone()),
        Arc::clone(host) as Arc<dyn RuntimeHost>,
        drivers(),
        RuntimeConfig {
            attachments_dir: Path::new(&host.root).join("attachments"),
            ..RuntimeConfig::default()
        },
    )
}

/// A work dir + a short holder dir (unix socket paths are short).
struct Dirs {
    work: tempfile::TempDir,
    holders: tempfile::TempDir,
}

impl Dirs {
    fn new() -> Self {
        Self {
            work: tempfile::tempdir().unwrap(),
            holders: tempfile::Builder::new()
                .prefix("rh")
                .tempdir_in("/tmp")
                .unwrap(),
        }
    }

    fn host(&self) -> Arc<TestHost> {
        TestHost::new(self.work.path(), &self.holders.path().join("h"))
    }

    fn cwd(&self) -> &str {
        self.work.path().to_str().unwrap()
    }
}

async fn create(rt: &AgentRuntime, cwd: &str, provider: RuntimeProvider) -> RuntimeSession {
    create_with(rt, cwd, provider, None).await
}

async fn create_with(
    rt: &AgentRuntime,
    cwd: &str,
    provider: RuntimeProvider,
    access_mode: Option<RuntimeAccessMode>,
) -> RuntimeSession {
    rt.create(
        &"u1".into(),
        &"w1".into(),
        CreateRuntimeSessionReq {
            title_auto: false,
            provider: Some(provider),
            cwd: Some(cwd.into()),
            title: None,
            model: None,
            access_mode,
            meta: None,
        },
    )
    .await
    .unwrap()
}

fn terminal(e: &RuntimeEvent) -> Option<TurnTerminalStatus> {
    match &e.body {
        RuntimeEventBody::TurnTerminal { status, .. } => Some(*status),
        _ => None,
    }
}

fn is_terminal(e: &RuntimeEvent) -> bool {
    terminal(e).is_some()
}

/// A streaming assistant upsert whose text already holds `needle`.
fn streamed(needle: &'static str) -> impl Fn(&RuntimeEvent) -> bool {
    move |e| {
        matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
            if matches!(&turn_item.kind, TurnItemKind::AssistantMessage { text, .. } if text.contains(needle)))
    }
}

fn stream_prompt(provider: RuntimeProvider, n: usize, ms: u64) -> String {
    if provider == RuntimeProvider::Claude {
        format!("stream {n} {ms}")
    } else {
        format!("stream:{n}:{ms}")
    }
}

fn expected(n: usize) -> String {
    (0..n).map(|i| format!("c{i} ")).collect()
}

const PROVIDERS: [RuntimeProvider; 4] = [
    RuntimeProvider::Claude,
    RuntimeProvider::Codex,
    RuntimeProvider::Gemini,
    RuntimeProvider::Agy,
];

/// A turn streaming when the daemon restarts finishes in the new daemon:
/// no respawn, no "interrupted", every chunk exactly once.
async fn mid_turn(provider: RuntimeProvider) {
    let d = Dirs::new();
    let pool = pool().await;
    let host = d.host();
    let rt = runtime(&pool, &host);
    let s = create(&rt, d.cwd(), provider).await;
    let n = 40;
    rt.prompt(&s.id, &stream_prompt(provider, n, 40))
        .await
        .unwrap();
    host.wait(streamed("c3 ")).await;
    rt.shutdown().await;
    drop(rt);
    assert!(
        host.all().iter().all(|e| !is_terminal(e)),
        "{provider:?}: the turn did not end in the old daemon"
    );

    let host2 = d.host();
    let rt2 = runtime(&pool, &host2);
    rt2.recover().await.unwrap();
    assert_eq!(
        rt2.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::Working,
        "{provider:?}: still working after the restart"
    );
    let t = host2.wait(is_terminal).await;
    assert_eq!(
        terminal(&t),
        Some(TurnTerminalStatus::Completed),
        "{provider:?}"
    );
    assert_eq!(
        host2.answers().last().cloned(),
        Some(expected(n)),
        "{provider:?}: the whole answer, each chunk once"
    );
    host.assert_prefixes(&expected(n), &format!("{provider:?} before"));
    host2.assert_prefixes(&expected(n), &format!("{provider:?} after"));
    assert_eq!(host2.spawns(), 0, "{provider:?}: adopted, not respawned");
    // It is the same live process: the next prompt needs no spawn either.
    rt2.prompt(&s.id, "again").await.unwrap();
    host2.wait_n(is_terminal, 2).await;
    assert_eq!(host2.spawns(), 0, "{provider:?}");
    rt2.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn a_streaming_turn_continues_across_a_restart_for_every_provider() {
    for p in PROVIDERS {
        mid_turn(p).await;
    }
}

/// A pending approval stays answerable in the new daemon.
async fn pending_approval(provider: RuntimeProvider) {
    let d = Dirs::new();
    let pool = pool().await;
    let host = d.host();
    let rt = runtime(&pool, &host);
    let s = create_with(
        &rt,
        d.cwd(),
        provider,
        Some(RuntimeAccessMode::ApprovalRequired),
    )
    .await;
    rt.prompt(&s.id, "ask").await.unwrap();
    host.wait(|e| {
        matches!(&e.body, RuntimeEventBody::RuntimeRequestUpdated { runtime_request }
        if runtime_request.status == RuntimeRequestStatus::Pending)
    })
    .await;
    rt.shutdown().await;
    drop(rt);

    let host2 = d.host();
    let rt2 = runtime(&pool, &host2);
    rt2.recover().await.unwrap();
    assert_eq!(
        rt2.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::AwaitingPermission,
        "{provider:?}"
    );
    let RuntimeReplay::Snapshot { snapshot, .. } = rt2.replay(&s.id, 0).await.unwrap() else {
        panic!("a fresh daemon replays a snapshot")
    };
    let pending: Vec<_> = snapshot
        .runtime_requests
        .iter()
        .filter(|r| r.status == RuntimeRequestStatus::Pending)
        .collect();
    assert_eq!(pending.len(), 1, "{provider:?}: still pending, not expired");
    rt2.respond(&s.id, &pending[0].id, ApprovalDecision::Accept)
        .await
        .unwrap();
    let t = host2.wait(is_terminal).await;
    assert_eq!(
        terminal(&t),
        Some(TurnTerminalStatus::Completed),
        "{provider:?}"
    );
    assert!(
        host2
            .answers()
            .iter()
            .any(|a| a.contains("allowed") || a == "done"),
        "{provider:?}: {:?}",
        host2.answers()
    );
    assert_eq!(host2.spawns(), 0, "{provider:?}");
    rt2.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn a_pending_approval_stays_answerable_across_a_restart() {
    for p in PROVIDERS {
        pending_approval(p).await;
    }
}

/// A message queued behind the running turn goes out after it, once.
async fn queued_follow_up(provider: RuntimeProvider) {
    let d = Dirs::new();
    let pool = pool().await;
    let host = d.host();
    let rt = runtime(&pool, &host);
    let s = create(&rt, d.cwd(), provider).await;
    rt.prompt(&s.id, &stream_prompt(provider, 30, 40))
        .await
        .unwrap();
    host.wait(streamed("c2 ")).await;
    let r = rt
        .prompt_with(
            &s.id,
            &RuntimePromptReq {
                delivery: Some(PromptDelivery::Queue),
                ..RuntimePromptReq::of("after")
            },
        )
        .await
        .unwrap();
    assert!(r.queued_id.is_some());
    rt.shutdown().await;
    drop(rt);

    let host2 = d.host();
    let rt2 = runtime(&pool, &host2);
    rt2.recover().await.unwrap();
    host2.wait_n(is_terminal, 2).await;
    let answers = host2.answers();
    assert_eq!(
        answers.iter().filter(|a| **a == expected(30)).count(),
        1,
        "{provider:?}: {answers:?}"
    );
    assert!(
        answers.last().is_some_and(|a| a.contains("after")),
        "{provider:?}: the queued message went out after the turn: {answers:?}"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        host2.all().iter().filter(|e| is_terminal(e)).count(),
        2,
        "{provider:?}: sent once"
    );
    assert_eq!(host2.spawns(), 0, "{provider:?}");
    rt2.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn a_queued_follow_up_goes_out_after_the_adopted_turn() {
    for p in PROVIDERS {
        queued_follow_up(p).await;
    }
}

/// An idle thread is simply alive after the restart: no Revive, no respawn.
#[tokio::test]
async fn an_idle_thread_needs_no_revive() {
    for provider in PROVIDERS {
        let d = Dirs::new();
        let pool = pool().await;
        let host = d.host();
        let rt = runtime(&pool, &host);
        let s = create(&rt, d.cwd(), provider).await;
        rt.prompt(&s.id, "hello").await.unwrap();
        host.wait(is_terminal).await;
        // Past the trailing checkpoint of the turn's last lines.
        tokio::time::sleep(Duration::from_millis(800)).await;
        rt.flush(&s.id).await;
        let repo = AgentRuntimeRepo::new(pool.clone());
        let active = repo.get(&s.id).await.unwrap().session.last_active_at;
        rt.shutdown().await;
        drop(rt);

        let host2 = d.host();
        let rt2 = runtime(&pool, &host2);
        rt2.recover().await.unwrap();
        assert_eq!(
            repo.get(&s.id).await.unwrap().session.last_active_at,
            active,
            "{provider:?}: a restart is not activity"
        );
        assert_eq!(
            rt2.get(&s.id).await.unwrap().status,
            RuntimeSessionStatus::Idle,
            "{provider:?}"
        );
        rt2.prompt(&s.id, "again").await.unwrap();
        let t = host2.wait(is_terminal).await;
        assert_eq!(
            terminal(&t),
            Some(TurnTerminalStatus::Completed),
            "{provider:?}"
        );
        assert!(
            host2.answers().last().is_some_and(|a| a.contains("again")),
            "{provider:?}: {:?}",
            host2.answers()
        );
        assert_eq!(host2.spawns(), 0, "{provider:?}");
        rt2.close(&s.id).await.unwrap();
    }
}

/// Claude's background subagent keeps running across the restart, and its
/// own follow-up turn arrives in the new daemon.
#[tokio::test]
async fn a_background_subagent_survives_a_restart() {
    let d = Dirs::new();
    let pool = pool().await;
    let host = d.host();
    let rt = runtime(&pool, &host);
    let s = create(&rt, d.cwd(), RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "agent 1.5").await.unwrap();
    host.wait(is_terminal).await;
    assert!(!rt.get(&s.id).await.unwrap().background_tasks.is_empty());
    rt.shutdown().await;
    drop(rt);

    let host2 = d.host();
    let rt2 = runtime(&pool, &host2);
    rt2.recover().await.unwrap();
    assert!(
        !rt2.get(&s.id).await.unwrap().background_tasks.is_empty(),
        "the background work is still pending"
    );
    host2.wait(is_terminal).await;
    assert!(
        host2.answers().iter().any(|a| a == "Agent reported"),
        "{:?}",
        host2.answers()
    );
    assert_eq!(host2.spawns(), 0);
    rt2.close(&s.id).await.unwrap();
}

/// A crash (no shutdown at all): the new daemon replays from the last
/// checkpoint; the answer still arrives whole and only once.
#[test]
fn a_streaming_turn_survives_a_daemon_crash() {
    let d = Dirs::new();
    let main = tokio::runtime::Runtime::new().unwrap();
    let pool = main.block_on(file_pool(&d.work.path().join("state.db")));
    for provider in PROVIDERS {
        let host = d.host();
        // The "daemon" lives on its own tokio runtime; dropping that runtime
        // mid-stream stops every task at once, as a crash would.
        let crashed = tokio::runtime::Runtime::new().unwrap();
        let s = crashed.block_on(async {
            let rt = runtime(&pool, &host);
            let s = create(&rt, d.cwd(), provider).await;
            rt.prompt(&s.id, &stream_prompt(provider, 40, 40))
                .await
                .unwrap();
            host.wait(streamed("c10 ")).await;
            s
        });
        drop(crashed);
        main.block_on(async {
            let host2 = d.host();
            let rt2 = runtime(&pool, &host2);
            rt2.recover().await.unwrap();
            let t = host2.wait(is_terminal).await;
            assert_eq!(
                terminal(&t),
                Some(TurnTerminalStatus::Completed),
                "{provider:?}"
            );
            assert_eq!(
                host2.answers().last().cloned(),
                Some(expected(40)),
                "{provider:?}"
            );
            host2.assert_prefixes(&expected(40), &format!("{provider:?} after a crash"));
            assert_eq!(host2.spawns(), 0, "{provider:?}");
            rt2.close(&s.id).await.unwrap();
        });
    }
}

/// A holder that is gone by the time the new daemon starts: the old
/// recovery (turn interrupted, the next message resumes).
#[tokio::test]
async fn an_unadoptable_holder_falls_back_to_resume() {
    let d = Dirs::new();
    let pool = pool().await;
    let host = d.host();
    let rt = runtime(&pool, &host);
    let s = create(&rt, d.cwd(), RuntimeProvider::Gemini).await;
    rt.prompt(&s.id, "stream:40:40").await.unwrap();
    host.wait(streamed("c2 ")).await;
    rt.shutdown().await;
    drop(rt);
    for socket in host.holders.sockets() {
        let s = socket.clone();
        tokio::task::spawn_blocking(move || otto_pty::holder::terminate(&s))
            .await
            .unwrap()
            .unwrap();
    }
    tokio::time::sleep(Duration::from_secs(3)).await;

    let host2 = d.host();
    let rt2 = runtime(&pool, &host2);
    rt2.recover().await.unwrap();
    assert_eq!(
        rt2.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::Dormant
    );
    rt2.prompt(&s.id, "hi").await.unwrap();
    let t = host2.wait(is_terminal).await;
    assert_eq!(terminal(&t), Some(TurnTerminalStatus::Completed));
    assert_eq!(host2.spawns(), 1, "resumed by a respawn");
    rt2.close(&s.id).await.unwrap();
}
