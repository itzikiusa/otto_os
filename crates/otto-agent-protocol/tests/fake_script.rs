//! RT (test plan §1, §11.1): every `OTTO_FAKE:` verb on every fake agent,
//! through the real `AgentRuntime` and the real drivers — `otto-fake-claude`,
//! `otto-fake-codex`, `otto-fake-agent` as each ACP flavour (gemini, agy,
//! grok, opencode, muse), and — in memory, with no process — the same fake programs behind the runtime's `ChildTransport` seam (`scripted`). A verb a fake cannot act out must answer
//! "unsupported: <verb>"; the shared parser and the prompt finders are pinned
//! on the engine's real prompt shapes.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use otto_agent_protocol::acp::AcpDriver;
use otto_agent_protocol::claude::ClaudeDriver;
use otto_agent_protocol::codex::CodexDriver;
use otto_agent_protocol::fake_script::{self, Act, Config, FailClass};
use otto_agent_protocol::scripted;
use otto_agent_protocol::{
    AgentRuntime, DriverFactory, LaunchSpec, McpDelivery, McpServerDef, RuntimeConfig, RuntimeHost,
    StructuredDriver,
};
use otto_core::agent_runtime::{
    ApprovalDecision, CreateRuntimeSessionReq, QuestionAnswer, RequestKind, RuntimeEvent,
    RuntimeEventBody, RuntimeProvider, RuntimeRequestStatus, RuntimeSession, TurnItemKind,
    TurnOrigin, TurnTerminalStatus,
};
use otto_core::domain::Workspace;
use otto_core::{Id, Result};
use otto_state::{AgentRuntimeRepo, DbPool};
use serde_json::{json, Value};

macro_rules! fake_bin {
    ($name:literal) => {
        std::env::var(concat!("NEXTEST_BIN_EXE_", $name))
            .unwrap_or_else(|_| env!(concat!("CARGO_BIN_EXE_", $name)).to_string())
    };
}

const WAIT: Duration = Duration::from_secs(20);

/// Every runtime driver's fake (each ACP flavour through `otto-fake-agent`).
const FAKES: [RuntimeProvider; 7] = [
    RuntimeProvider::Claude,
    RuntimeProvider::Codex,
    RuntimeProvider::Gemini,
    RuntimeProvider::Agy,
    RuntimeProvider::Grok,
    RuntimeProvider::Opencode,
    RuntimeProvider::Muse,
];

const ACP: [RuntimeProvider; 5] = [
    RuntimeProvider::Gemini,
    RuntimeProvider::Agy,
    RuntimeProvider::Grok,
    RuntimeProvider::Opencode,
    RuntimeProvider::Muse,
];

/// The stub Otto MCP server `child` / `finish-child` talk to: it logs each
/// `tools/call` line to `$STUB_LOG` and answers with thread `child-1`.
const STUB_MCP: &str = r#"#!/bin/sh
read -r l
echo '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{}}}'
read -r l
read -r l
printf '%s\n' "$l" >> "$STUB_LOG"
echo '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"{\"peer_session_id\":\"child-1\"}"}]}}'
"#;

struct Host {
    root: PathBuf,
    env: Vec<(String, String)>,
    events: Mutex<Vec<Arc<RuntimeEvent>>>,
    notify: tokio::sync::Notify,
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
        s: &RuntimeSession,
        spec: &mut LaunchSpec,
        mcp: McpDelivery,
    ) -> Result<Vec<McpServerDef>> {
        spec.env.extend(self.env.iter().cloned());
        let (stub, log) = (
            self.root.join("stub-mcp.sh"),
            self.root.join("mcp-calls.log"),
        );
        if s.provider == RuntimeProvider::Claude {
            let config = self.root.join("mcp.json");
            let doc = json!({"mcpServers": {"otto": {"command": stub, "env": {"STUB_LOG": log}}}});
            std::fs::write(&config, doc.to_string()).unwrap();
            spec.args
                .extend(["--mcp-config".into(), config.display().to_string()]);
        }
        Ok(match mcp {
            McpDelivery::Protocol => vec![McpServerDef {
                name: "otto".into(),
                command: stub.display().to_string(),
                args: Vec::new(),
                env: vec![("STUB_LOG".into(), log.display().to_string())],
            }],
            McpDelivery::Launcher => Vec::new(),
        })
    }
    fn publish(&self, _s: &RuntimeSession, event: Arc<RuntimeEvent>) {
        self.events.lock().unwrap().push(event);
        self.notify.notify_waiters();
    }
}

fn binaries() -> Arc<dyn DriverFactory> {
    Arc::new(|p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
        let acp = fake_bin!("otto-fake-agent");
        Some(match p {
            RuntimeProvider::Claude => {
                Box::new(ClaudeDriver::with_program(fake_bin!("otto-fake-claude")))
            }
            RuntimeProvider::Codex => {
                Box::new(CodexDriver::with_program(fake_bin!("otto-fake-codex")))
            }
            RuntimeProvider::Gemini => Box::new(AcpDriver::gemini(&acp)),
            RuntimeProvider::Agy => Box::new(AcpDriver::agy(&acp)),
            RuntimeProvider::Muse => Box::new(AcpDriver::muse(&acp)),
            _ => Box::new(AcpDriver::new(p, acp, Vec::new())),
        })
    })
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

/// One runtime (the fake binaries, or the scripted driver) on its own temp
/// dir: a run context folder, a handoff path, the fakes' turn log and state.
struct Rig {
    rt: AgentRuntime,
    host: Arc<Host>,
    _dir: tempfile::TempDir,
    root: PathBuf,
    ctx: PathBuf,
}

impl Rig {
    async fn new(scripted: bool) -> Self {
        Self::with(scripted, &[]).await
    }

    /// With extra environment for the fake binaries.
    async fn with(scripted: bool, extra: &[(&str, &str)]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let ctx = root.join("ctx");
        std::fs::create_dir_all(&ctx).unwrap();
        let stub = root.join("stub-mcp.sh");
        std::fs::write(&stub, STUB_MCP).unwrap();
        std::fs::set_permissions(&stub, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let path = |n: &str| root.join(n).display().to_string();
        let env = vec![
            ("OTTO_FAKE_LOG".into(), path("turns.log")),
            ("OTTO_FAKE_ARGV_LOG".into(), path("argv.log")),
            ("OTTO_FAKE_STATE_DIR".into(), path("state")),
            ("OTTO_FAKE_SCRIPTS".into(), path("scripts")),
            ("CLAUDE_CODE_TMPDIR".into(), path("claude-tmp")),
        ];
        let env = env
            .into_iter()
            .chain(extra.iter().map(|(k, v)| (k.to_string(), v.to_string())))
            .collect();
        let host = Arc::new(Host {
            root: root.clone(),
            env,
            events: Mutex::default(),
            notify: tokio::sync::Notify::new(),
        });
        // In memory: the same fake programs and the real drivers, no process.
        let (drivers, transport) = if scripted {
            (scripted::drivers(), Some(scripted::transport(Vec::new())))
        } else {
            (binaries(), None)
        };
        let rt = AgentRuntime::new(
            AgentRuntimeRepo::new(pool().await),
            Arc::clone(&host) as Arc<dyn RuntimeHost>,
            drivers,
            RuntimeConfig {
                attachments_dir: root.join("attachments"),
                transport,
                ..RuntimeConfig::default()
            },
        );
        Self {
            rt,
            host,
            _dir: dir,
            root,
            ctx,
        }
    }

    async fn session(&self, provider: RuntimeProvider) -> Id {
        let req = CreateRuntimeSessionReq {
            title_auto: false,
            provider: Some(provider),
            cwd: Some(self.root.display().to_string()),
            title: None,
            model: None,
            access_mode: None,
            meta: None,
        };
        self.rt
            .create(&"u1".into(), &"w1".into(), req)
            .await
            .unwrap()
            .id
    }

    fn handoff(&self, slug: &str) -> PathBuf {
        self.ctx.join(format!("step2-{slug}.md"))
    }

    /// A workflow step prompt (the engine's preamble shape) with these
    /// `OTTO_FAKE:` lines.
    fn step(&self, slug: &str, verbs: &[&str]) -> String {
        let mut p = format!(
            "[workflow context]\nContext directory: {ctx}\nRead, in order:\n- run-brief.md — the run's mission.\n\
             - Prior step summaries: step1-gather.md\nRead the files you need before starting.\n\n\
             [your handoff — required]\nWhen finished, write a complete summary of what you did to: {ctx}/step2-{slug}.md\n\
             (the engine treats this file as your done signal)\n\nDo the step.\n",
            ctx = self.ctx.display()
        );
        for v in verbs {
            p.push_str(&format!("OTTO_FAKE: {v}\n"));
        }
        p
    }

    fn events(&self, id: &Id) -> Vec<RuntimeEvent> {
        self.host
            .events
            .lock()
            .unwrap()
            .iter()
            .filter(|e| &e.session_id == id)
            .map(|e| (**e).clone())
            .collect()
    }

    async fn wait_n(
        &self,
        id: &Id,
        n: usize,
        what: &str,
        pred: impl Fn(&RuntimeEvent) -> bool,
    ) -> RuntimeEvent {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let notified = self.host.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let hits: Vec<RuntimeEvent> = self.events(id).into_iter().filter(|e| pred(e)).collect();
            if hits.len() >= n {
                return hits[n - 1].clone();
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                panic!(
                    "timed out waiting for {what} #{n}; text so far: {:?}",
                    self.text(id)
                );
            }
        }
    }

    fn terminals(&self, id: &Id) -> usize {
        self.events(id)
            .iter()
            .filter(|e| terminal(e).is_some())
            .count()
    }

    /// Prompt and wait for that turn's end: (status, failure message).
    async fn turn(&self, id: &Id, prompt: &str) -> (TurnTerminalStatus, String) {
        let n = self.terminals(id) + 1;
        self.rt.prompt(id, prompt).await.unwrap();
        self.next_terminal(id, n).await
    }

    async fn next_terminal(&self, id: &Id, n: usize) -> (TurnTerminalStatus, String) {
        let e = self
            .wait_n(id, n, "a turn end", |e| terminal(e).is_some())
            .await;
        let RuntimeEventBody::TurnTerminal {
            status, failure, ..
        } = e.body
        else {
            unreachable!()
        };
        (status, failure.map(|f| f.message).unwrap_or_default())
    }

    /// Everything the agent said in this session, in order.
    fn text(&self, id: &Id) -> String {
        let mut by_id = std::collections::BTreeMap::new();
        for e in self.events(id) {
            if let RuntimeEventBody::TurnItemUpdated { turn_item } = e.body {
                if let TurnItemKind::AssistantMessage { text, .. } = &turn_item.kind {
                    by_id.insert(
                        turn_item.id.clone(),
                        (turn_item.ordinal, turn_item.turn_id.clone(), text.clone()),
                    );
                }
            }
        }
        let mut v: Vec<_> = by_id.into_values().collect();
        v.sort_by(|a, b| (&a.1, a.0).cmp(&(&b.1, b.0)));
        v.into_iter()
            .map(|(_, _, t)| t)
            .collect::<Vec<_>>()
            .join("|")
    }

    fn log(&self, name: &str) -> Vec<Value> {
        std::fs::read_to_string(self.root.join(name))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// The n-th request of this session to show as pending.
    async fn pending(&self, id: &Id, n: usize) -> otto_core::agent_runtime::RuntimeRequest {
        let e = self
            .wait_n(id, n, "a pending request", |e| {
                matches!(&e.body, RuntimeEventBody::RuntimeRequestUpdated { runtime_request: r }
                    if r.status == RuntimeRequestStatus::Pending)
            })
            .await;
        let RuntimeEventBody::RuntimeRequestUpdated { runtime_request: r } = e.body else {
            unreachable!()
        };
        r
    }
}

fn terminal(e: &RuntimeEvent) -> Option<TurnTerminalStatus> {
    match &e.body {
        RuntimeEventBody::TurnTerminal { status, .. } => Some(*status),
        _ => None,
    }
}

/// Every fake, then the scripted driver (as Claude): `(label, scripted, provider)`.
fn every_driver() -> Vec<(String, bool, RuntimeProvider)> {
    let mut v: Vec<_> = FAKES
        .iter()
        .map(|p| (p.as_str().to_string(), false, *p))
        .collect();
    for p in [
        RuntimeProvider::Claude,
        RuntimeProvider::Codex,
        RuntimeProvider::Gemini,
    ] {
        v.push((format!("in-memory {}", p.as_str()), true, p));
    }
    v
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn files_memory_usage_and_replies_act_out_on_every_fake() {
    for (label, scripted, p) in every_driver() {
        let rig = Rig::new(scripted).await;
        std::fs::write(rig.ctx.join("run-brief.md"), "brief").unwrap();
        let id = rig.session(p).await;
        let prompt = rig.step(
            "gather",
            &[
                "write $CTX/notes/a.txt hello there",
                "expect $CTX/notes/a.txt",
                "expect $CTX/run-brief.md",
                "sleep 30",
                "stream 40 40",
                "usage 1200 300",
                "rss 1",
                "remember tok-42",
                "reply wrote it",
                "write $HANDOFF @filler:50",
            ],
        );
        let (status, why) = rig.turn(&id, &prompt).await;
        assert_eq!(status, TurnTerminalStatus::Completed, "{label}: {why}");
        assert_eq!(
            std::fs::read_to_string(rig.ctx.join("notes/a.txt")).unwrap(),
            "hello there",
            "{label}"
        );
        assert_eq!(
            std::fs::read_to_string(rig.handoff("gather"))
                .unwrap()
                .len(),
            50,
            "{label}"
        );
        let said = rig.text(&id);
        assert!(said.contains("wrote it"), "{label}: {said}");
        assert!(
            said.matches("the ").count() >= 10,
            "{label}: ten streamed chunks: {said}"
        );
        // The memory is the provider session's: the next turn recalls it.
        let (status, why) = rig.turn(&id, "OTTO_FAKE: recall").await;
        assert_eq!(status, TurnTerminalStatus::Completed, "{label}: {why}");
        assert!(
            rig.text(&id).contains("recall: tok-42"),
            "{label}: {}",
            rig.text(&id)
        );
        // The turn log: one record per turn, the verbs in order.
        let turns = rig.log("turns.log");
        assert_eq!(turns.len(), 2, "{label}: {turns:?}");
        let verbs: Vec<&str> = turns[0]["verbs"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert_eq!(&verbs[..3], ["write", "expect", "expect"], "{label}");
        assert!(
            verbs.contains(&"usage") && verbs.contains(&"remember"),
            "{label}: {verbs:?}"
        );
        assert!(
            turns[0]["t_end"].as_u64() >= turns[0]["t_start"].as_u64(),
            "{label}"
        );
        rig.rt.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_missing_file_and_each_failure_class_end_the_turn_as_that_class() {
    for (label, scripted, p) in every_driver() {
        let rig = Rig::new(scripted).await;
        let id = rig.session(p).await;
        let (status, why) = rig
            .turn(
                &id,
                &rig.step("check", &["expect $CTX/nope.md", "reply never"]),
            )
            .await;
        assert_eq!(status, TurnTerminalStatus::Failed, "{label}");
        assert!(why.contains("missing $CTX/nope.md"), "{label}: {why}");
        assert!(
            !rig.text(&id).contains("never"),
            "{label}: the script stopped"
        );
        for class in ["provider", "auth", "usage_limit"] {
            let (status, _) = rig.turn(&id, &format!("OTTO_FAKE: fail {class}")).await;
            assert_eq!(status, TurnTerminalStatus::Failed, "{label} {class}");
            let s = rig.rt.get(&id).await.unwrap();
            assert_eq!(
                s.last_error_class.as_deref(),
                Some(class),
                "{label} {class}"
            );
        }
        rig.rt.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fail_attempts_fails_the_first_turns_of_one_handoff_then_lets_it_through() {
    for (label, scripted, p) in every_driver() {
        let rig = Rig::new(scripted).await;
        let id = rig.session(p).await;
        let prompt = rig.step("flaky", &["fail-attempts 2 auth", "write $HANDOFF ok"]);
        for attempt in 1..=2 {
            let (status, _) = rig.turn(&id, &prompt).await;
            assert_eq!(
                status,
                TurnTerminalStatus::Failed,
                "{label} attempt {attempt}"
            );
            assert!(!rig.handoff("flaky").exists(), "{label}");
        }
        let (status, why) = rig.turn(&id, &prompt).await;
        assert_eq!(status, TurnTerminalStatus::Completed, "{label}: {why}");
        assert_eq!(
            std::fs::read_to_string(rig.handoff("flaky")).unwrap(),
            "ok",
            "{label}"
        );
        rig.rt.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn crash_ends_the_turn_and_hang_waits_for_the_interrupt() {
    for (label, scripted, p) in every_driver() {
        let rig = Rig::new(scripted).await;
        let id = rig.session(p).await;
        let (status, _) = rig.turn(&id, "OTTO_FAKE: fail crash").await;
        assert_ne!(
            status,
            TurnTerminalStatus::Completed,
            "{label}: a dead agent's turn fails"
        );
        let n = rig.terminals(&id) + 1;
        rig.rt
            .prompt(&id, "OTTO_FAKE: reply up\nOTTO_FAKE: fail hang")
            .await
            .unwrap();
        rig.wait_n(&id, 1, "the reply before the hang", |e| {
            matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
                if matches!(&turn_item.kind, TurnItemKind::AssistantMessage { text, .. } if text.contains("up")))
        })
        .await;
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(
            rig.terminals(&id),
            n - 1,
            "{label}: a hang does not end on its own"
        );
        rig.rt.interrupt(&id).await.unwrap();
        let (status, _) = rig.next_terminal(&id, n).await;
        assert!(
            matches!(
                status,
                TurnTerminalStatus::Interrupted | TurnTerminalStatus::Cancelled
            ),
            "{label}: {status:?}"
        );
        rig.rt.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn questions_and_approvals_wait_for_the_person_where_the_provider_has_them() {
    // pi has neither (its own test: `verbs_a_provider_cannot_act_out_answer_unsupported`).
    for (label, scripted, p) in every_driver().into_iter().filter(|d| d.0 != "pi") {
        let rig = Rig::new(scripted).await;
        let id = rig.session(p).await;
        let n = rig.terminals(&id) + 1;
        rig.rt
            .prompt(&id, "OTTO_FAKE: ask Which branch?\nOTTO_FAKE: approve make clean\nOTTO_FAKE: approve rm -rf build")
            .await
            .unwrap();
        let r = rig.pending(&id, 1).await;
        assert!(
            matches!(r.kind, RequestKind::Question | RequestKind::McpElicitation),
            "{label}: {:?}",
            r.kind
        );
        let q = r
            .questions
            .first()
            .unwrap_or_else(|| panic!("{label}: the question's form"));
        assert!(
            q.prompt.contains("Which branch?")
                || r.question_title.as_deref() == Some("Which branch?"),
            "{label}: {q:?}"
        );
        let answer = vec![QuestionAnswer::Text {
            question_id: q.id.clone(),
            text: "main".into(),
        }];
        rig.rt
            .respond_question(&id, &r.id, answer)
            .await
            .unwrap_or_else(|e| panic!("{label}: {e}"));
        let r = rig.pending(&id, 2).await;
        assert!(
            matches!(r.kind, RequestKind::Command | RequestKind::Permission),
            "{label}: {:?}",
            r.kind
        );
        rig.rt
            .respond(&id, &r.id, ApprovalDecision::Accept)
            .await
            .unwrap();
        let r = rig.pending(&id, 3).await;
        rig.rt
            .respond(&id, &r.id, ApprovalDecision::Decline)
            .await
            .unwrap();
        let (status, why) = rig.next_terminal(&id, n).await;
        assert_eq!(status, TurnTerminalStatus::Completed, "{label}: {why}");
        let said = rig.text(&id);
        assert!(said.contains("answer: main"), "{label}: {said}");
        assert!(said.contains("approved: make clean"), "{label}: {said}");
        assert!(said.contains("denied: rm -rf build"), "{label}: {said}");
        rig.rt.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn verbs_a_provider_cannot_act_out_answer_unsupported() {
    let cases: [(RuntimeProvider, &[&str]); 2] = [
        (
            RuntimeProvider::Codex,
            &["child claude reply hi", "finish-child"],
        ),
        (RuntimeProvider::Gemini, &["background 10"]),
    ];
    let rig = Rig::new(false).await;
    for (p, verbs) in cases {
        for verb in verbs {
            let id = rig.session(p).await;
            let (status, _) = rig
                .turn(&id, &format!("OTTO_FAKE: {verb}\nOTTO_FAKE: reply after"))
                .await;
            assert_eq!(status, TurnTerminalStatus::Completed, "{p:?} {verb}");
            let name = verb.split_whitespace().next().unwrap();
            let said = rig.text(&id);
            assert!(
                said.contains(&format!("unsupported: {name}")),
                "{p:?} {verb}: {said}"
            );
            assert!(
                !said.contains("after"),
                "{p:?} {verb}: the script stops there"
            );
        }
    }
    let id = rig.session(RuntimeProvider::Claude).await;
    rig.turn(&id, "OTTO_FAKE: dance now").await;
    assert!(
        rig.text(&id).contains("unsupported: dance now"),
        "{}",
        rig.text(&id)
    );
    rig.rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn background_work_ends_the_turn_and_the_agents_own_turn_runs_the_rest() {
    let drivers = [
        ("claude", false, RuntimeProvider::Claude),
        ("codex", false, RuntimeProvider::Codex),
        ("in-memory claude", true, RuntimeProvider::Claude),
        ("in-memory codex", true, RuntimeProvider::Codex),
    ];
    for (label, scripted, p) in drivers {
        let rig = Rig::new(scripted).await;
        let id = rig.session(p).await;
        let prompt = rig.step(
            "later",
            &[
                "reply started",
                "background 600",
                "write $HANDOFF from the follow-up",
                "reply all done",
            ],
        );
        let (status, why) = rig.turn(&id, &prompt).await;
        assert_eq!(status, TurnTerminalStatus::Completed, "{label}: {why}");
        assert!(
            !rig.handoff("later").exists(),
            "{label}: the rest waits for the work"
        );
        rig.wait_n(&id, 1, "background work listed", |e| {
            matches!(&e.body, RuntimeEventBody::ProviderSessionUpdated { provider_session }
                if !provider_session.background_tasks.is_empty())
        })
        .await;
        let (status, why) = rig.next_terminal(&id, 2).await;
        assert_eq!(status, TurnTerminalStatus::Completed, "{label}: {why}");
        assert_eq!(
            std::fs::read_to_string(rig.handoff("later")).unwrap(),
            "from the follow-up",
            "{label}"
        );
        let own = rig
            .events(&id)
            .into_iter()
            .filter_map(|e| match e.body {
                RuntimeEventBody::ProviderTurnUpdated { provider_turn } => Some(provider_turn),
                _ => None,
            })
            .any(|t| t.origin == TurnOrigin::Agent);
        assert!(own, "{label}: the follow-up is the agent's own turn");
        assert!(
            rig.text(&id).contains("all done"),
            "{label}: {}",
            rig.text(&id)
        );
        rig.rt.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_step_opens_and_removes_a_sub_thread_through_the_otto_mcp_server() {
    let mut drivers = vec![RuntimeProvider::Claude];
    drivers.extend(ACP);
    for p in drivers {
        let rig = Rig::new(false).await;
        let id = rig.session(p).await;
        let (status, why) = rig
            .turn(
                &id,
                "OTTO_FAKE: child codex reply hi;;write out.txt x\nOTTO_FAKE: finish-child",
            )
            .await;
        assert_eq!(status, TurnTerminalStatus::Completed, "{p:?}: {why}");
        let said = rig.text(&id);
        assert!(
            said.contains("opened child-1") && said.contains("closed child-1"),
            "{p:?}: {said}"
        );
        let calls: Vec<Value> = std::fs::read_to_string(rig.root.join("mcp-calls.log"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(calls.len(), 2, "{p:?}: {calls:?}");
        assert_eq!(calls[0]["params"]["name"], "otto_thread_open", "{p:?}");
        assert_eq!(calls[0]["params"]["arguments"]["provider"], "codex");
        assert_eq!(
            calls[0]["params"]["arguments"]["prompt"],
            "OTTO_FAKE: reply hi\nOTTO_FAKE: write out.txt x"
        );
        assert_eq!(calls[1]["params"]["name"], "otto_thread_close");
        assert_eq!(
            calls[1]["params"]["arguments"],
            json!({"peer_session_id": "child-1", "remove": true})
        );
        rig.rt.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_step_script_is_looked_up_by_its_handoff_slug_on_every_fake() {
    for (label, scripted, p) in every_driver() {
        let rig = Rig::new(scripted).await;
        std::fs::create_dir_all(rig.root.join("scripts")).unwrap();
        std::fs::write(
            rig.root.join("scripts/review-gate.txt"),
            "# the gate's script\nreply gate says hi\nOTTO_FAKE: write $HANDOFF gated\n",
        )
        .unwrap();
        let id = rig.session(p).await;
        let (status, why) = rig.turn(&id, &rig.step("review-gate", &["@script"])).await;
        assert_eq!(status, TurnTerminalStatus::Completed, "{label}: {why}");
        assert_eq!(
            std::fs::read_to_string(rig.handoff("review-gate")).unwrap(),
            "gated",
            "{label}"
        );
        assert!(rig.text(&id).contains("gate says hi"), "{label}");
        // A step with no script file acts as the step persona.
        let (status, why) = rig.turn(&id, &rig.step("other", &["@script"])).await;
        assert_eq!(
            status,
            TurnTerminalStatus::Failed,
            "{label}: run-brief.md is missing: {why}"
        );
        std::fs::write(rig.ctx.join("run-brief.md"), "b").unwrap();
        std::fs::write(rig.ctx.join("step1-gather.md"), "g").unwrap();
        let (status, why) = rig.turn(&id, &rig.step("other", &["@script"])).await;
        assert_eq!(status, TurnTerminalStatus::Completed, "{label}: {why}");
        assert_eq!(
            std::fs::read_to_string(rig.handoff("other")).unwrap().len(),
            400,
            "{label}"
        );
        rig.rt.shutdown().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_prompt_without_a_sentinel_keeps_each_fakes_exact_text_behaviour() {
    for (label, scripted, p) in every_driver() {
        let rig = Rig::new(scripted).await;
        let id = rig.session(p).await;
        // A workflow preamble alone (no persona configured) is not a script.
        let (status, _) = rig.turn(&id, &rig.step("plain", &[])).await;
        assert_eq!(status, TurnTerminalStatus::Completed, "{label}");
        assert!(
            rig.text(&id).contains("echo: "),
            "{label}: {}",
            rig.text(&id)
        );
        assert!(!rig.handoff("plain").exists(), "{label}");
        rig.rt.shutdown().await;
    }
}

/// The in-memory transport runs the fake program itself: a path no OS could
/// start still gets a working agent, and the fake sees its own in-memory pid.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn in_memory_children_run_the_fake_programs_without_a_process() {
    let rig = Rig::new(true).await;
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool().await),
        Arc::clone(&rig.host) as Arc<dyn RuntimeHost>,
        Arc::new(|_p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
            Some(Box::new(CodexDriver::with_program(
                "/nonexistent/otto-fake-codex",
            )))
        }),
        RuntimeConfig {
            attachments_dir: rig.root.join("attachments"),
            transport: Some(scripted::transport(vec![(
                "OTTO_FAKE_LOG".into(),
                rig.root.join("mem.log").display().to_string(),
            )])),
            ..RuntimeConfig::default()
        },
    );
    let req = CreateRuntimeSessionReq {
        title_auto: false,
        provider: Some(RuntimeProvider::Codex),
        cwd: Some(rig.root.display().to_string()),
        title: None,
        model: None,
        access_mode: None,
        meta: None,
    };
    let id = rt.create(&"u1".into(), &"w1".into(), req).await.unwrap().id;
    let n = rig.terminals(&id) + 1;
    rt.prompt(&id, "OTTO_FAKE: reply in memory").await.unwrap();
    let (status, why) = rig.next_terminal(&id, n).await;
    assert_eq!(status, TurnTerminalStatus::Completed, "{why}");
    assert!(rig.text(&id).contains("in memory"));
    // The spawn's own env (the host's) wins over the transport's.
    let turns = rig.log("turns.log");
    assert_eq!(turns.len(), 1, "{turns:?}");
    assert!(
        turns[0]["pid"].as_u64().unwrap() >= 1_000_000,
        "an in-memory pid: {turns:?}"
    );
    assert!(rig.log("mem.log").is_empty());
    rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_time_scale_shrinks_every_wait_and_each_start_is_logged() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        provider: "claude".into(),
        time_scale: 0.001,
        scripts_dir: None,
        step_persona: false,
        review_dir: None,
        state_dir: dir.path().into(),
        unsupported: &[],
        log: None,
    };
    let script = "OTTO_FAKE: sleep 600000\nOTTO_FAKE: stream 10 20000\nOTTO_FAKE: background 10000";
    let mut t = fake_script::Turn::start(script, dir.path(), "s", 1, &cfg).unwrap();
    assert_eq!(
        t.next_step(),
        fake_script::Step::Wait(Duration::from_millis(600))
    );
    assert_eq!(
        t.next_step(),
        fake_script::Step::Wait(Duration::from_millis(2)),
        "stream: 10 chunks"
    );
    // Through a fake binary: an hour's sleep at 0.0001 is 360 ms.
    let rig = Rig::with(false, &[("OTTO_FAKE_TIME_SCALE", "0.0001")]).await;
    for p in [RuntimeProvider::Codex, RuntimeProvider::Gemini] {
        let id = rig.session(p).await;
        let started = std::time::Instant::now();
        let (status, _) = rig
            .turn(&id, "OTTO_FAKE: sleep 3600000\nOTTO_FAKE: reply woke")
            .await;
        assert_eq!(status, TurnTerminalStatus::Completed, "{p:?}");
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{p:?}: {:?}",
            started.elapsed()
        );
        assert!(rig.text(&id).contains("woke"), "{p:?}");
    }
    let starts = rig.log("argv.log");
    for name in ["otto-fake-codex", "otto-fake-agent"] {
        assert!(
            starts
                .iter()
                .any(|l| l["argv"][0].as_str().is_some_and(|a| a.ends_with(name))),
            "{name}: {starts:?}"
        );
    }
    rig.rt.shutdown().await;
}

/// The finders on the engine's real prompt shapes.
#[test]
fn the_finders_read_the_engines_real_preamble_and_review_prompts() {
    let ctx = "/data/workflow-context/run-1";
    let preamble = otto_workflows::context::agent_preamble(
        ctx,
        true,
        false,
        true,
        &["step1-gather-info.md".into()],
        "step2-review-gate-iter3.md",
    );
    let prompt = format!("{preamble}Check repos.json and prs.json; ignore other.txt.\n");
    assert_eq!(fake_script::context_dir(&prompt).as_deref(), Some(ctx));
    let handoff = fake_script::handoff_path(&prompt).unwrap();
    assert_eq!(handoff, format!("{ctx}/step2-review-gate-iter3.md"));
    assert_eq!(fake_script::handoff_slug(&handoff), "review-gate");
    let files = fake_script::contract_files(&prompt);
    for f in [
        "instructions.md",
        "run-brief.md",
        "repos.json",
        "step1-gather-info.md",
        "prs.json",
    ] {
        assert!(files.iter().any(|x| x == f), "{f} in {files:?}");
    }
    assert!(!files.iter().any(|x| x == "other.txt"), "{files:?}");

    let dir = tempfile::tempdir().unwrap();
    let review = dir.path().join("review");
    std::fs::create_dir_all(&review).unwrap();
    std::fs::write(
        review.join("reviewer-security-review-claude.json"),
        r#"[{"body":"x"}]"#,
    )
    .unwrap();
    let cfg = Config {
        provider: "claude".into(),
        time_scale: 1.0,
        scripts_dir: None,
        step_persona: false,
        review_dir: Some(review.clone()),
        state_dir: dir.path().into(),
        unsupported: &[],
        log: None,
    };
    let out = dir.path().join("otto-review-R-0.json");
    let base = otto_review::engine::compose_review_lens_prompt(
        "security-review",
        "Method.",
        "Review the diff.",
    );
    let reviewer =
        otto_review::session::typed_reviewer_prompt(&base, &out.display().to_string(), None);
    let mut t =
        fake_script::Turn::start(&reviewer, dir.path(), "s", 1, &cfg).expect("the review persona");
    assert_eq!(
        t.next_step(),
        fake_script::Step::Say("reviewer done".into())
    );
    assert_eq!(std::fs::read_to_string(&out).unwrap(), r#"[{"body":"x"}]"#);
    // No file for the role: an empty valid answer.
    std::fs::remove_file(&out).unwrap();
    let plain =
        otto_review::session::typed_reviewer_prompt("Review it.", &out.display().to_string(), None);
    let mut t = fake_script::Turn::start(&plain, dir.path(), "s", 1, &cfg).unwrap();
    t.next_step();
    assert_eq!(std::fs::read_to_string(&out).unwrap(), "[]");
    // `<role>.pre`: its verbs run first (the reviewer's work time), then the result.
    std::fs::write(review.join("reviewer.pre"), "sleep 1500\n").unwrap();
    let mut t = fake_script::Turn::start(&plain, dir.path(), "s", 1, &cfg).unwrap();
    assert_eq!(
        t.next_step(),
        fake_script::Step::Wait(Duration::from_millis(1500))
    );
    assert_eq!(
        t.next_step(),
        fake_script::Step::Say("reviewer done".into())
    );
    std::fs::remove_file(review.join("reviewer.pre")).unwrap();
    // Without the persona the same prompt is not scripted.
    let off = Config {
        review_dir: None,
        ..cfg
    };
    assert!(fake_script::Turn::start(&plain, dir.path(), "s", 1, &off).is_none());

    // OTTO_FAKE_DEFAULT=step: a step prompt with no sentinel expects the
    // context files it names (not its own handoff), then writes the handoff.
    let step_on = Config {
        step_persona: true,
        ..off
    };
    let mut t =
        fake_script::Turn::start(&prompt, dir.path(), "s", 1, &step_on).expect("the step persona");
    assert_eq!(
        t.next_step(),
        fake_script::Step::Fail(FailClass::Provider, "missing $CTX/instructions.md".into()),
        "the real preamble's run dir does not exist here"
    );
    let ctx_dir = dir.path().join("ctx");
    std::fs::create_dir_all(&ctx_dir).unwrap();
    for f in [
        "instructions.md",
        "run-brief.md",
        "repos.json",
        "prs.json",
        "step1-gather-info.md",
    ] {
        std::fs::write(ctx_dir.join(f), "x").unwrap();
    }
    let preamble = otto_workflows::context::agent_preamble(
        &ctx_dir.display().to_string(),
        true,
        false,
        true,
        &["step1-gather-info.md".into()],
        "step2-review-gate.md",
    );
    let real = format!("{preamble}Use prs.json.");
    let mut t = fake_script::Turn::start(&real, dir.path(), "s", 1, &step_on).unwrap();
    assert_eq!(t.next_step(), fake_script::Step::Say("step done".into()));
    assert_eq!(
        std::fs::read_to_string(ctx_dir.join("step2-review-gate.md"))
            .unwrap()
            .len(),
        400
    );
    let off = Config {
        step_persona: false,
        ..step_on
    };
    assert!(
        fake_script::Turn::start(&real, dir.path(), "s", 1, &off).is_none(),
        "off by default"
    );
}

#[test]
fn the_parser_reads_every_verb_and_flags_the_rest() {
    let p = |l: &str| fake_script::parse_act(l);
    assert_eq!(
        p("write $HANDOFF two words"),
        Act::Write {
            path: "$HANDOFF".into(),
            text: "two words".into()
        }
    );
    assert_eq!(p("expect $CTX/a.md"), Act::Expect("$CTX/a.md".into()));
    assert_eq!(p("sleep 25"), Act::Sleep(25));
    assert_eq!(
        p("stream 2048 1000"),
        Act::Stream {
            bytes: 2048,
            ms: 1000
        }
    );
    assert_eq!(p("reply a b"), Act::Reply("a b".into()));
    assert_eq!(p("ask Which one?"), Act::Ask("Which one?".into()));
    assert_eq!(p("approve make"), Act::Approve("make".into()));
    assert_eq!(p("fail usage_limit"), Act::Fail(FailClass::UsageLimit));
    assert_eq!(p("fail hang"), Act::Fail(FailClass::Hang));
    assert_eq!(
        p("fail-attempts 2 provider"),
        Act::FailAttempts {
            n: 2,
            class: FailClass::Provider
        }
    );
    assert_eq!(p("background 500"), Act::Background(500));
    assert_eq!(
        p("child codex reply hi;;sleep 5"),
        Act::Child {
            provider: "codex".into(),
            scripts: vec!["reply hi".into(), "sleep 5".into()]
        }
    );
    assert_eq!(p("finish-child"), Act::FinishChild);
    assert_eq!(
        p("usage 1200 300"),
        Act::Usage {
            input: 1200,
            output: 300
        }
    );
    assert_eq!(p("remember t1"), Act::Remember("t1".into()));
    assert_eq!(p("recall"), Act::Recall);
    assert_eq!(p("rss 150"), Act::Rss(150));
    assert_eq!(p("@script"), Act::Script);
    for bad in [
        "sleep soon",
        "fail loudly",
        "stream 10",
        "dance",
        "fail-attempts x auth",
    ] {
        assert!(matches!(p(bad), Act::Unsupported(_)), "{bad}");
    }
    assert_eq!(fake_script::parse("no sentinel here"), None);
    assert_eq!(
        fake_script::parse("intro\n  OTTO_FAKE: sleep 1\nmiddle\nOTTO_FAKE: recall\n"),
        Some(vec![Act::Sleep(1), Act::Recall])
    );
}
