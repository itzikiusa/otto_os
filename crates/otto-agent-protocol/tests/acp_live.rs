//! LIVE: the INSTALLED ACP CLIs (gemini, agy, grok, opencode, muse) through
//! the real `AgentRuntime` and each one's real `AcpFlavor` — handshake with
//! a probe MCP server in `mcpServers`, one turn that runs a command (a
//! permission round-trip under approval-required) and calls the probe's tool,
//! one turn that is cancelled, then close + reopen (`session/load`). Every
//! test is `#[ignore]`: real turns spend the person's quota and need the CLI
//! signed in. Run one with
//! `cargo test -p otto-agent-protocol --test acp_live <name> -- --ignored --nocapture`.
//! Each prints its capability dump; docs/features/acp-capabilities.md records
//! the last run per provider.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use otto_agent_protocol::acp::AcpDriver;
use otto_agent_protocol::{
    AgentRuntime, DriverFactory, LaunchSpec, McpDelivery, McpServerDef, RuntimeConfig, RuntimeHost,
    StructuredDriver,
};
use otto_core::agent_runtime::{
    ApprovalDecision, CreateRuntimeSessionReq, ProviderSessionStatus, RuntimeAccessMode,
    RuntimeEvent, RuntimeEventBody, RuntimeProvider, RuntimeRequestStatus, RuntimeSession,
    TurnItemKind, TurnTerminalStatus,
};
use otto_core::domain::Workspace;
use otto_core::{Id, Result};
use otto_state::{AgentRuntimeRepo, DbPool};

const WAIT: Duration = Duration::from_secs(240);

/// A stdio MCP server with one tool, `probe_echo`, answering with
/// `PROBE-OK tag=<PROBE_TAG>` (proves the server and its env arrived).
const PROBE: &str = r#"import json, os, sys
for line in sys.stdin:
    try:
        m = json.loads(line)
    except ValueError:
        continue
    if "id" not in m:
        continue
    meth, r = m.get("method"), None
    if meth == "initialize":
        r = {"protocolVersion": m.get("params", {}).get("protocolVersion", "2025-06-18"), "capabilities": {"tools": {}}, "serverInfo": {"name": "probe", "version": "1"}}
    elif meth == "tools/list":
        r = {"tools": [{"name": "probe_echo", "description": "Returns the probe tag.", "inputSchema": {"type": "object", "properties": {}}}]}
    elif meth == "tools/call":
        r = {"content": [{"type": "text", "text": "PROBE-OK tag=" + os.environ.get("PROBE_TAG", "none")}]}
    out = {"jsonrpc": "2.0", "id": m["id"]}
    if r is None:
        out["error"] = {"code": -32601, "message": "no " + str(meth)}
    else:
        out["result"] = r
    sys.stdout.write(json.dumps(out) + "\n")
    sys.stdout.flush()
"#;

struct Host {
    root: String,
    probe: McpServerDef,
    events: Mutex<Vec<Arc<RuntimeEvent>>>,
    notify: tokio::sync::Notify,
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
        spec: &mut LaunchSpec,
        _mcp: McpDelivery,
    ) -> Result<Vec<McpServerDef>> {
        spec.program = resolve(&spec.program);
        Ok(vec![self.probe.clone()])
    }
    fn publish(&self, _s: &RuntimeSession, event: Arc<RuntimeEvent>) {
        self.events.lock().unwrap().push(event);
        self.notify.notify_waiters();
    }
}

/// The program on PATH or in the daemon's extra tool dirs (ottod's
/// `augment_path`: `~/.local/bin`, `~/.opencode/bin`, …).
fn resolve(program: &str) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = std::env::var("PATH").unwrap_or_default();
    let extra = [".local/bin", ".opencode/bin", ".grok/bin"].map(|d| format!("{home}/{d}"));
    path.split(':')
        .map(str::to_string)
        .chain(extra)
        .map(|d| std::path::Path::new(&d).join(program))
        .find(|p| p.is_file())
        .map_or_else(|| program.to_string(), |p| p.display().to_string())
}

impl Host {
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

    /// Every item's final text (assistant messages, tool output), joined.
    fn transcript(&self) -> String {
        let mut by_id = std::collections::BTreeMap::new();
        for e in self.all() {
            if let RuntimeEventBody::TurnItemUpdated { turn_item } = e.body {
                by_id.insert(turn_item.id.clone(), turn_item);
            }
        }
        by_id
            .values()
            .map(|i| match &i.kind {
                TurnItemKind::AssistantMessage { text, .. } => text.clone(),
                TurnItemKind::UserMessage { .. } => String::new(),
                _ => serde_json::to_string(&i.kind).unwrap_or_default(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

fn terminal(e: &RuntimeEvent) -> Option<TurnTerminalStatus> {
    match &e.body {
        RuntimeEventBody::TurnTerminal { status, .. } => Some(*status),
        _ => None,
    }
}

fn is_ready(e: &RuntimeEvent) -> bool {
    matches!(&e.body, RuntimeEventBody::ProviderSessionUpdated { provider_session }
        if provider_session.status == ProviderSessionStatus::Ready)
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
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('u1', 'u', 'x', 'U', 0, ?)",
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('w1', 'W', '/tmp', ?)",
    ] {
        sqlx::query(q).bind(&now).execute(&pool).await.unwrap();
    }
    pool.into()
}

struct Live {
    rt: AgentRuntime,
    host: Arc<Host>,
    s: RuntimeSession,
    _dir: tempfile::TempDir,
}

/// Start `provider` in a temp folder (one README.md) under `access`.
async fn start(
    provider: RuntimeProvider,
    make: fn() -> AcpDriver,
    access: RuntimeAccessMode,
) -> Live {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap().display().to_string();
    std::fs::write(dir.path().join("README.md"), "hello probe\n").unwrap();
    std::fs::write(dir.path().join("probe.py"), PROBE).unwrap();
    let host = Arc::new(Host {
        root: root.clone(),
        probe: McpServerDef {
            name: "probe".into(),
            command: "/usr/bin/python3".into(),
            args: vec![format!("{root}/probe.py")],
            env: vec![("PROBE_TAG".into(), format!("{}-live", provider.as_str()))],
        },
        events: Mutex::default(),
        notify: tokio::sync::Notify::new(),
    });
    let drivers: Arc<dyn DriverFactory> = Arc::new(
        move |_p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> { Some(Box::new(make())) },
    );
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool().await),
        Arc::clone(&host) as Arc<dyn RuntimeHost>,
        drivers,
        RuntimeConfig {
            attachments_dir: dir.path().join("attachments"),
            ..RuntimeConfig::default()
        },
    );
    let s = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                provider: Some(provider),
                cwd: Some(root),
                access_mode: Some(access),
                ..CreateRuntimeSessionReq::default()
            },
        )
        .await
        .unwrap();
    Live {
        rt,
        host,
        s,
        _dir: dir,
    }
}

impl Live {
    async fn ready(&self, n: usize) -> RuntimeSession {
        self.host.wait_n(is_ready, n).await;
        let s = self.rt.get(&self.s.id).await.unwrap();
        println!(
            "capabilities: {}",
            serde_json::to_string(&s.capabilities).unwrap()
        );
        s
    }

    /// Prompt, accepting every request, to the `n`th turn end. Returns
    /// (status, requests answered).
    async fn turn(&self, text: &str, n: usize) -> (TurnTerminalStatus, usize) {
        self.rt.prompt(&self.s.id, text).await.unwrap();
        let mut answered = std::collections::HashSet::new();
        loop {
            let done = self.host.all().iter().filter_map(terminal).nth(n - 1);
            if let Some(status) = done {
                return (status, answered.len());
            }
            for e in self.host.all() {
                if let RuntimeEventBody::RuntimeRequestUpdated { runtime_request } = e.body {
                    if runtime_request.status == RuntimeRequestStatus::Pending
                        && answered.insert(runtime_request.id.clone())
                    {
                        println!("request: {:?}", runtime_request.kind);
                        self.rt
                            .respond(&self.s.id, &runtime_request.id, ApprovalDecision::Accept)
                            .await
                            .unwrap();
                    }
                }
            }
            let notified = self.host.notify.notified();
            let _ = tokio::time::timeout(Duration::from_millis(500), notified).await;
        }
    }

    /// The full check: turn 1 (command + probe tool), turn 2 cancelled,
    /// close + reopen resumes the agent's own session.
    async fn check(self, expect_ask: bool, expect_probe: bool) {
        self.ready(1).await;
        self.command_and_probe(expect_ask, expect_probe).await;
        self.cancel_and_resume(1).await;
    }

    async fn command_and_probe(&self, expect_ask: bool, expect_probe: bool) {
        let (status, asked) = self
            .turn(
                "Do exactly this and nothing else: run the shell command \
                 `echo otto-live-42 > otto-live.txt && cat otto-live.txt` once, then call the MCP \
                 tool probe_echo once, then reply with one short line containing both outputs.",
                1,
            )
            .await;
        let text = self.host.transcript();
        println!("turn 1: {status:?}, {asked} request(s) answered\n{text}");
        assert_eq!(status, TurnTerminalStatus::Completed);
        assert!(text.contains("otto-live-42"), "the command ran");
        assert_eq!(asked > 0, expect_ask, "a permission round-trip");
        assert_eq!(
            text.contains("PROBE-OK"),
            expect_probe,
            "the probe MCP tool answered"
        );
    }

    /// A turn cancelled mid-stream, then close + reopen: `session/load` of
    /// the agent's own session, no fresh boundary. `done` = turns before.
    async fn cancel_and_resume(&self, done: usize) {
        let native = self.rt.get(&self.s.id).await.unwrap().native_thread_id;
        let native = native.expect("the agent named its session");
        self.rt
            .prompt(
                &self.s.id,
                "Write the numbers from 1 to 400, one per line, nothing else.",
            )
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_secs(4)).await;
        assert!(self.rt.interrupt(&self.s.id).await.unwrap());
        let fin = self.host.wait_n(|e| terminal(e).is_some(), done + 1).await;
        println!("turn 2: {:?}", terminal(&fin));
        assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Interrupted));

        self.rt.close(&self.s.id).await.unwrap();
        self.rt.reopen(&self.s.id).await.unwrap();
        let s = self.host.wait_n(is_ready, 2).await;
        let _ = s;
        let s = self.rt.get(&self.s.id).await.unwrap();
        assert_eq!(
            s.native_thread_id.as_deref(),
            Some(native.as_str()),
            "session/load resumed it"
        );
        let lost = self.host.all().iter().any(|e| {
            matches!(&e.body,
            RuntimeEventBody::TurnItemUpdated { turn_item }
                if matches!(turn_item.kind, TurnItemKind::SessionBoundary))
        });
        assert!(!lost, "no fresh-session boundary after reopen");
        self.rt.close(&self.s.id).await.unwrap();
    }
}

#[tokio::test]
#[ignore = "needs the real agy CLI, signed in; spends 2 turns"]
async fn agy_live() {
    start(RuntimeProvider::Agy, || AcpDriver::agy("agy"), ASK)
        .await
        .check(true, true)
        .await;
}

const ASK: RuntimeAccessMode = RuntimeAccessMode::ApprovalRequired;

#[tokio::test]
#[ignore = "needs the real grok CLI, signed in; spends 2 turns"]
/// Full access: on a machine whose Claude Code settings say
/// `permissions.defaultMode: "dontAsk"` (Grok reads them) Grok 1.0.92 denies
/// every unapproved tool in agent mode without a `session/request_permission`
/// — the driver's notice covers that (unit test
/// `grok_denied_without_asking_says_so_once`).
async fn grok_live() {
    start(
        RuntimeProvider::Grok,
        AcpDriver::grok,
        RuntimeAccessMode::FullAccess,
    )
    .await
    .check(false, true)
    .await;
}

#[tokio::test]
#[ignore = "needs the real opencode CLI; spends 2 turns"]
async fn opencode_live() {
    start(RuntimeProvider::Opencode, AcpDriver::opencode, ASK)
        .await
        .check(true, true)
        .await;
}

#[tokio::test]
#[ignore = "needs the real muse CLI, signed in; spends 2 turns"]
async fn muse_live() {
    start(RuntimeProvider::Muse, || AcpDriver::muse("muse"), ASK)
        .await
        .check(false, true)
        .await;
}

/// Muse 1.4.25 under `promptUnmatched` (live): the mode is applied
/// (`current_mode_update`) yet a workspace shell write runs without a
/// request — its built-in rules cover it. This half spends one turn.
#[tokio::test]
#[ignore = "needs the real muse CLI, signed in; spends 1 turn"]
async fn muse_live_cancel_and_resume() {
    let live = start(RuntimeProvider::Muse, || AcpDriver::muse("muse"), ASK).await;
    live.ready(1).await;
    live.cancel_and_resume(0).await;
}

/// Gemini CLI 0.43 refuses `session/new` for a personal Google account
/// ("This client is no longer supported for Gemini Code Assist for
/// individuals"): the thread must carry the CLI's own words, not a generic
/// sign-in failure. No turn is spent.
#[tokio::test]
#[ignore = "needs the real gemini CLI"]
async fn gemini_live_handshake() {
    let live = start(RuntimeProvider::Gemini, || AcpDriver::gemini("gemini"), ASK).await;
    let _ = live.rt.prompt(&live.s.id, "hi").await;
    let fin = live.host.wait_n(|e| terminal(e).is_some(), 1).await;
    let RuntimeEventBody::TurnTerminal {
        status, failure, ..
    } = fin.body
    else {
        unreachable!()
    };
    println!("gemini: {status:?} {failure:?}");
    match failure {
        Some(f) if f.message.contains("no longer supported") => {}
        // A signed-in account the CLI still serves: a normal turn.
        None => assert_eq!(status, TurnTerminalStatus::Completed),
        Some(f) => panic!("unexpected failure: {f:?}"),
    }
    live.rt.close(&live.s.id).await.unwrap();
}

/// No turn spent: handshake, the agent's slash commands reach the thread
/// (`commands.updated`), close + reopen resumes the same session (with
/// `session/resume` where advertised). Safe to run any time.
/// `same`: the agent keeps an un-prompted session (Muse 1.4.25 does not:
/// `session/resume` answers "session … not found", so it starts fresh).
async fn handshake(provider: RuntimeProvider, make: fn() -> AcpDriver, same: bool) {
    let live = start(provider, make, ASK).await;
    let s = live.ready(1).await;
    let native = s.native_thread_id.expect("the agent named its session");
    let commands = live
        .host
        .wait_n(
            |e| matches!(&e.body, RuntimeEventBody::CommandsUpdated { commands } if !commands.is_empty()),
            1,
        )
        .await;
    if let RuntimeEventBody::CommandsUpdated { commands } = commands.body {
        let names: Vec<_> = commands.iter().map(|c| c.name.as_str()).take(8).collect();
        println!("commands: {} ({names:?}…)", commands.len());
    }
    live.rt.close(&live.s.id).await.unwrap();
    live.rt.reopen(&live.s.id).await.unwrap();
    live.host.wait_n(is_ready, 2).await;
    let s = live.rt.get(&live.s.id).await.unwrap();
    assert_eq!(s.native_thread_id.as_deref() == Some(native.as_str()), same);
    let lost = live.host.all().iter().any(|e| {
        matches!(&e.body,
        RuntimeEventBody::TurnItemUpdated { turn_item }
            if matches!(turn_item.kind, TurnItemKind::SessionBoundary))
    });
    assert_eq!(
        lost, !same,
        "a fresh session says so, a resumed one does not"
    );
    live.rt.close(&live.s.id).await.unwrap();
}

#[tokio::test]
#[ignore = "needs the real agy CLI; no turn"]
async fn agy_live_handshake() {
    handshake(RuntimeProvider::Agy, || AcpDriver::agy("agy"), true).await;
}

#[tokio::test]
#[ignore = "needs the real grok CLI; no turn"]
async fn grok_live_handshake() {
    handshake(RuntimeProvider::Grok, AcpDriver::grok, true).await;
}

#[tokio::test]
#[ignore = "needs the real opencode CLI; no turn"]
async fn opencode_live_handshake() {
    handshake(RuntimeProvider::Opencode, AcpDriver::opencode, true).await;
}

#[tokio::test]
#[ignore = "needs the real muse CLI; no turn"]
async fn muse_live_handshake() {
    handshake(RuntimeProvider::Muse, || AcpDriver::muse("muse"), false).await;
}

/// Slash commands: the text `/compact` goes as the prompt and the agent
/// runs its own command (one short turn).
#[tokio::test]
#[ignore = "needs the real agy CLI, signed in; spends 1 turn"]
async fn agy_live_slash_command() {
    let live = start(RuntimeProvider::Agy, || AcpDriver::agy("agy"), ASK).await;
    live.ready(1).await;
    let (status, _) = live.turn("/compact", 1).await;
    println!("/compact: {status:?}\n{}", live.host.transcript());
    assert_eq!(status, TurnTerminalStatus::Completed);
    live.rt.close(&live.s.id).await.unwrap();
}

/// Otto serves grok's `fs/*` and `terminal/*` (full access: grok's own
/// policy denies tools here otherwise): the command's output streams into
/// its tool row, a write lands in the folder as a file-change row. 2 turns.
#[tokio::test]
#[ignore = "needs the real grok CLI, signed in; spends 2 turns"]
async fn grok_live_client_fs_and_terminal() {
    let live = start(
        RuntimeProvider::Grok,
        AcpDriver::grok,
        RuntimeAccessMode::FullAccess,
    )
    .await;
    live.ready(1).await;
    let (status, _) = live
        .turn("Read README.md, then run the shell command `echo otto-term-7` once. Reply with one line: the file's text and the command's output.", 1)
        .await;
    let streamed: String = live
        .host
        .all()
        .iter()
        .filter_map(|e| match &e.body {
            RuntimeEventBody::CommandOutputDelta { chunk, .. } => Some(chunk.clone()),
            _ => None,
        })
        .collect();
    println!(
        "turn 1: {status:?}; streamed {streamed:?}\n{}",
        live.host.transcript()
    );
    assert_eq!(status, TurnTerminalStatus::Completed);
    assert!(
        streamed.contains("otto-term-7"),
        "terminal output streamed through Otto"
    );
    let (status, _) = live
        .turn(
            "Create the file note.txt containing exactly the text: hi from grok",
            2,
        )
        .await;
    let note = std::fs::read_to_string(std::path::Path::new(&live.host.root).join("note.txt"));
    let changed = live.host.transcript().contains("note.txt");
    println!("turn 2: {status:?}; note.txt = {note:?}; change row: {changed}");
    assert_eq!(status, TurnTerminalStatus::Completed);
    assert_eq!(note.as_deref().ok().map(str::trim), Some("hi from grok"));
    live.rt.close(&live.s.id).await.unwrap();
}
