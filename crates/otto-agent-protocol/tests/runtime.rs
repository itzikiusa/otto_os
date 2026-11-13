//! End to end: the real `AgentRuntime` + the real drivers over stdio — the
//! ACP driver against `otto-fake-agent`, the Codex driver against
//! `otto-fake-codex` (a fake `codex app-server`) — with a real (in-memory)
//! SQLite store.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use otto_agent_protocol::acp::AcpDriver;
use otto_agent_protocol::claude::ClaudeDriver;
use otto_agent_protocol::codex::CodexDriver;
use otto_agent_protocol::ring::RingLimits;
use otto_agent_protocol::{
    AgentRuntime, DriverFactory, LaunchSpec, McpDelivery, McpServerDef, RuntimeConfig, RuntimeHost,
    StructuredDriver,
};
use otto_core::agent_runtime::{
    ApprovalDecision, Attachment, AttachmentKind, CancelReason, CreateRuntimeSessionReq,
    PromptFile, PromptImage, ProviderSessionStatus, ProviderTurnStatus, RuntimeAccessMode,
    RuntimeEvent, RuntimeEventBody, RuntimePromptReq, RuntimeProvider, RuntimeReplay,
    RuntimeRequestStatus, RuntimeSession, RuntimeSessionStatus, TurnItem, TurnItemKind,
    TurnItemStatus, TurnTerminalStatus,
};
use otto_core::domain::Workspace;
use otto_core::{Error, Id, Result};
use otto_state::{AgentRuntimeRepo, DbPool};

/// A fake binary's path. `CARGO_BIN_EXE_*` is the BUILD machine's path, frozen
/// at compile time; a nextest archive run (CI shards) unpacks the binaries
/// elsewhere and exports the real location as `NEXTEST_BIN_EXE_*`.
macro_rules! fake_bin {
    ($name:literal) => {
        std::env::var(concat!("NEXTEST_BIN_EXE_", $name))
            .unwrap_or_else(|_| env!(concat!("CARGO_BIN_EXE_", $name)).to_string())
    };
}
const WAIT: Duration = Duration::from_secs(15);

struct TestHost {
    root: String,
    refuse: bool,
    /// Refuse only this provider's spawns (a council seat that cannot start).
    refuse_provider: Mutex<Option<RuntimeProvider>>,
    prepared: Mutex<Vec<LaunchSpec>>,
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
        if self.refuse || *self.refuse_provider.lock().unwrap() == Some(s.provider) {
            return Err(Error::Invalid(
                "refusing to start an agent session here".into(),
            ));
        }
        // Prove the runtime spawns the PREPARED spec (the safety pipeline's).
        spec.env.push(("OTTO_PREPARED".into(), "1".into()));
        // otto-context's agy bundle (env, not `--add-dir`).
        if s.provider == RuntimeProvider::Agy {
            spec.env.push((
                "OTTO_AGY_CONTEXT_DIR".into(),
                format!("{}/bundle", self.root),
            ));
        }
        self.prepared.lock().unwrap().push(spec.clone());
        Ok(Vec::new())
    }
    fn publish(&self, _s: &RuntimeSession, event: Arc<RuntimeEvent>) {
        self.events.lock().unwrap().push(event);
        self.notify.notify_waiters();
    }
}

impl TestHost {
    fn new(root: &str, refuse: bool) -> Arc<Self> {
        Arc::new(Self {
            root: root.into(),
            refuse,
            refuse_provider: Mutex::default(),
            prepared: Mutex::default(),
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
            notified.as_mut().enable(); // registered before the check
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

    /// The latest state of every turn item, by ordinal.
    fn items(&self) -> Vec<TurnItem> {
        let mut by_id = std::collections::BTreeMap::new();
        for e in self.all() {
            if let RuntimeEventBody::TurnItemUpdated { turn_item } = e.body {
                by_id.insert(turn_item.id.clone(), turn_item);
            }
        }
        let mut v: Vec<TurnItem> = by_id.into_values().collect();
        v.sort_by_key(|i| i.ordinal);
        v
    }
}

fn drivers() -> Arc<dyn DriverFactory> {
    Arc::new(|p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
        Some(match p {
            RuntimeProvider::Codex => {
                Box::new(CodexDriver::with_program(fake_bin!("otto-fake-codex")))
            }
            RuntimeProvider::Agy => Box::new(AcpDriver::agy(&fake_bin!("otto-fake-agent"))),
            RuntimeProvider::Gemini => Box::new(AcpDriver::gemini(&fake_bin!("otto-fake-agent"))),
            // A signed-out ACP agent.
            RuntimeProvider::Grok => Box::new(AcpDriver::new(
                p,
                fake_bin!("otto-fake-agent"),
                vec!["--refuse-session".into()],
            )),
            RuntimeProvider::Claude => {
                Box::new(ClaudeDriver::with_program(fake_bin!("otto-fake-claude")))
            }
            _ => Box::new(AcpDriver::new(p, fake_bin!("otto-fake-agent"), Vec::new())),
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

fn runtime(pool: &DbPool, host: &Arc<TestHost>, config: RuntimeConfig) -> AgentRuntime {
    let config = RuntimeConfig {
        attachments_dir: std::path::Path::new(&host.root).join("attachments"),
        ..config
    };
    AgentRuntime::new(
        AgentRuntimeRepo::new(pool.clone()),
        Arc::clone(host) as Arc<dyn RuntimeHost>,
        drivers(),
        config,
    )
}

async fn create(rt: &AgentRuntime, cwd: &str) -> RuntimeSession {
    create_as(rt, cwd, RuntimeProvider::Gemini).await
}

async fn create_as(rt: &AgentRuntime, cwd: &str, provider: RuntimeProvider) -> RuntimeSession {
    rt.create(
        &"u1".into(),
        &"w1".into(),
        CreateRuntimeSessionReq {
            provider: Some(provider),
            cwd: Some(cwd.into()),
            title: None,
            model: None,
            access_mode: None,
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

fn is_ready(e: &RuntimeEvent) -> bool {
    matches!(&e.body, RuntimeEventBody::ProviderSessionUpdated { provider_session }
        if provider_session.status == ProviderSessionStatus::Ready)
}

fn is_type(t: &'static str) -> impl Fn(&RuntimeEvent) -> bool {
    move |e| e.body.type_name() == t
}

#[tokio::test]
async fn echo_turn_streams_entity_upserts_in_sequence_order() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    host.wait(is_ready).await;
    let s = rt.get(&s.id).await.unwrap();
    assert_eq!(s.status, RuntimeSessionStatus::Idle);
    assert!(s.native_thread_id.as_deref().unwrap().starts_with("fake-"));
    assert_eq!(
        host.prepared.lock().unwrap()[0].env.last().unwrap().0,
        "OTTO_PREPARED"
    );

    let r = rt.prompt(&s.id, "hello").await.unwrap();
    assert!(
        matches!(rt.prompt(&s.id, "again").await, Err(Error::Conflict(_))),
        "one turn at a time"
    );
    let fin = host.wait(is_terminal).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Completed));

    let evs = host.all();
    assert!(
        evs.windows(2).all(|w| w[1].sequence == w[0].sequence + 1),
        "dense, increasing"
    );
    // Streaming: the assistant item is re-emitted whole with growing text,
    // `streaming: true`, then settled. The first chunk goes out at once; the
    // second lands inside the 50 ms coalescing window and is superseded by
    // the settled item (T3 Code coalesces the same way).
    let texts: Vec<(String, bool)> = evs
        .iter()
        .filter_map(|e| match &e.body {
            RuntimeEventBody::TurnItemUpdated { turn_item } => match &turn_item.kind {
                TurnItemKind::AssistantMessage {
                    text, streaming, ..
                } => Some((text.clone(), *streaming)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(
        texts,
        vec![("echo: ".into(), true), ("echo: hello".into(), false)]
    );
    let items = host.items();
    let kinds: Vec<&str> = items
        .iter()
        .map(|i| match i.kind {
            TurnItemKind::UserMessage { .. } => "user",
            TurnItemKind::Reasoning { .. } => "reasoning",
            TurnItemKind::AssistantMessage { .. } => "assistant",
            _ => "other",
        })
        .collect();
    assert_eq!(kinds, vec!["user", "reasoning", "assistant"]);
    assert!(items
        .iter()
        .all(|i| i.turn_id.as_deref() == Some(r.turn_id.as_str())));
    assert!(items
        .iter()
        .all(|i| i.status == TurnItemStatus::Completed && !i.is_streaming()));

    // Replay: exact events after any sequence…
    let RuntimeReplay::Events {
        events,
        last_sequence,
    } = rt.replay(&s.id, evs[3].sequence).await.unwrap()
    else {
        panic!("expected events")
    };
    assert_eq!(events, evs[4..].to_vec());
    assert_eq!(last_sequence, evs.last().unwrap().sequence);
    // …or nothing when up to date.
    assert!(
        matches!(rt.replay(&s.id, last_sequence).await.unwrap(), RuntimeReplay::Events { events, .. } if events.is_empty())
    );
    assert_eq!(
        rt.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::Idle
    );
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn approval_round_trip_validates_decisions_and_resolves_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    rt.prompt(&s.id, "ask").await.unwrap();
    let req = host.wait(is_type("runtime_request.updated")).await;
    let RuntimeEventBody::RuntimeRequestUpdated { runtime_request } = req.body else {
        unreachable!()
    };
    assert_eq!(runtime_request.status, RuntimeRequestStatus::Pending);
    assert_eq!(
        rt.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::AwaitingPermission
    );
    assert!(matches!(
        rt.respond(&s.id, "nope", ApprovalDecision::Accept).await,
        Err(Error::NotFound(_))
    ));
    assert!(
        matches!(
            rt.respond(&s.id, &runtime_request.id, ApprovalDecision::AcceptAlways)
                .await,
            Err(Error::Invalid(_))
        ),
        "not offered"
    );
    rt.respond(
        &s.id,
        &runtime_request.id,
        ApprovalDecision::AcceptForSession,
    )
    .await
    .unwrap();
    let fin = host.wait(is_terminal).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Completed));
    let resolved = host
        .all()
        .into_iter()
        .rev()
        .find_map(|e| match e.body {
            RuntimeEventBody::RuntimeRequestUpdated { runtime_request } => Some(runtime_request),
            _ => None,
        })
        .unwrap();
    assert_eq!(resolved.status, RuntimeRequestStatus::Resolved);
    assert_eq!(resolved.decision, Some(ApprovalDecision::AcceptForSession));
    let items = host.items();
    assert!(items.iter().any(
        |i| matches!(&i.kind, TurnItemKind::CommandExecution { input, output: Some(o), .. }
        if input == "ls" && o == "a.txt\n")
            && i.status == TurnItemStatus::Completed
    ));
    assert!(items
        .iter()
        .any(|i| matches!(i.kind, TurnItemKind::ApprovalRequest { .. })
            && i.status == TurnItemStatus::Completed));
    assert!(rt
        .respond(&s.id, &runtime_request.id, ApprovalDecision::Accept)
        .await
        .is_err());
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn interrupt_ends_the_turn_and_cancels_pending_requests() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, dir.path().to_str().unwrap()).await;

    rt.prompt(&s.id, "hang").await.unwrap();
    host.wait(is_type("turn_item.updated")).await;
    host.wait_n(is_type("turn_item.updated"), 2).await; // user + streaming chunk
    assert!(rt.interrupt(&s.id).await.unwrap());
    let fin = host.wait(is_terminal).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Interrupted));
    assert!(
        !rt.interrupt(&s.id).await.unwrap(),
        "nothing left to interrupt"
    );
    assert!(
        host.items()
            .iter()
            .all(|i| !i.status.is_open() && !i.is_streaming()),
        "open items settled"
    );

    rt.prompt(&s.id, "ask").await.unwrap();
    host.wait(is_type("runtime_request.updated")).await;
    rt.interrupt(&s.id).await.unwrap();
    let fin = host.wait_n(is_terminal, 2).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Interrupted));
    assert!(host.all().iter().any(|e| matches!(&e.body,
        RuntimeEventBody::RuntimeRequestUpdated { runtime_request } if runtime_request.status == RuntimeRequestStatus::Cancelled)));
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn ignored_interrupt_kills_the_group_and_the_next_prompt_resumes_via_load() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(
        &pool,
        &host,
        RuntimeConfig {
            interrupt_grace: Duration::from_millis(300),
            ..RuntimeConfig::default()
        },
    );
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    host.wait(is_ready).await;
    let native = rt.get(&s.id).await.unwrap().native_thread_id.unwrap();

    rt.prompt(&s.id, "hang-hard").await.unwrap();
    host.wait_n(is_type("turn_item.updated"), 2).await;
    rt.interrupt(&s.id).await.unwrap();
    let fin = host.wait(is_terminal).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Interrupted));
    host.wait(|e| {
        matches!(&e.body, RuntimeEventBody::ProviderSessionUpdated { provider_session }
        if provider_session.status == ProviderSessionStatus::Stopped)
    })
    .await;
    assert_eq!(
        rt.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::Dormant
    );

    // Respawn on demand: session/load of the same ACP session; its replayed
    // history ("(history)") never reaches the timeline.
    rt.prompt(&s.id, "back").await.unwrap();
    host.wait_n(is_terminal, 2).await;
    assert_eq!(
        rt.get(&s.id).await.unwrap().native_thread_id.unwrap(),
        native
    );
    assert!(host.items().iter().all(|i| !matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if text.contains("history"))));
    assert_eq!(
        host.prepared.lock().unwrap().len(),
        2,
        "safety prep per spawn"
    );
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn crash_mid_turn_fails_it_as_transport_error_with_stderr() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    rt.prompt(&s.id, "garbage").await.unwrap();
    host.wait(is_terminal).await;
    assert!(host.items().iter().any(|i| matches!(&i.kind, TurnItemKind::Error { failure } if failure.message.contains("not json"))));

    rt.prompt(&s.id, "crash").await.unwrap();
    let fin = host.wait_n(is_terminal, 2).await;
    let RuntimeEventBody::TurnTerminal {
        status,
        failure: Some(f),
        ..
    } = &fin.body
    else {
        panic!("{fin:?}")
    };
    assert_eq!(*status, TurnTerminalStatus::Failed);
    assert_eq!(f.class, "transport_error");
    assert!(
        f.message.contains("code 3") && f.message.contains("crashing on purpose"),
        "{}",
        f.message
    );
    assert!(host.items().iter().any(
        |i| matches!(&i.kind, TurnItemKind::Error { failure } if failure.class == "transport_error")
    ));
    host.wait(|e| {
        matches!(&e.body, RuntimeEventBody::ProviderSessionUpdated { provider_session }
        if provider_session.status == ProviderSessionStatus::Stopped)
    })
    .await;
    assert_eq!(
        rt.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::Dormant
    );
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn plans_and_refused_client_methods() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    rt.prompt(&s.id, "plan").await.unwrap();
    let plan = host.wait(is_type("plan.updated")).await;
    let RuntimeEventBody::PlanUpdated { plan } = plan.body else {
        unreachable!()
    };
    assert_eq!(plan.steps.len(), 2);
    host.wait(is_terminal).await;
    rt.prompt(&s.id, "fs").await.unwrap();
    host.wait_n(is_terminal, 2).await;
    assert!(host.items().iter().any(|i| matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if text == "fs refused -32601")));
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn refused_spawn_fails_the_create_and_leaves_no_row() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), true);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let err = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                provider: Some(RuntimeProvider::Gemini),
                ..CreateRuntimeSessionReq::default()
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Invalid(m) if m.contains("refusing")));
    assert!(rt.list(&"w1".into(), None).await.unwrap().is_empty());
    assert!(host.all().is_empty(), "nothing spawned");
}

#[tokio::test]
async fn snapshot_past_replay_limits_and_recovery_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let small = RuntimeConfig {
        ring: RingLimits {
            max_events: 8,
            max_bytes: usize::MAX,
        },
        // Every chunk its own event, to overflow the ring.
        stream_coalesce: Duration::ZERO,
        ..RuntimeConfig::default()
    };
    let rt = runtime(&pool, &host, small.clone());
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    let first = rt.prompt(&s.id, "big:20").await.unwrap();
    host.wait(is_terminal).await;

    // Out of the ring → a snapshot of every entity instead.
    let RuntimeReplay::Snapshot {
        snapshot_sequence,
        snapshot,
    } = rt.replay(&s.id, first.sequence).await.unwrap()
    else {
        panic!("expected a snapshot")
    };
    assert_eq!(snapshot_sequence, host.all().last().unwrap().sequence);
    assert_eq!(snapshot.turns.len(), 1);
    assert_eq!(snapshot.turns[0].status, ProviderTurnStatus::Completed);
    assert!(snapshot.turn_items.iter().any(|i| matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: false, .. } if text == &"x".repeat(20))));

    // Leave a turn waiting on a request, then "restart": a fresh runtime on the same DB.
    rt.prompt(&s.id, "ask").await.unwrap();
    host.wait(is_type("runtime_request.updated")).await;
    rt.flush(&s.id).await;
    rt.shutdown().await;
    let seen = host.all().last().unwrap().sequence;

    let host2 = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt2 = runtime(&pool, &host2, small);
    assert_eq!(rt2.recover().await.unwrap(), 1);
    let RuntimeReplay::Snapshot {
        snapshot_sequence,
        snapshot,
    } = rt2.replay(&s.id, 0).await.unwrap()
    else {
        panic!("a fresh daemon has no ring: snapshot")
    };
    assert!(
        snapshot_sequence > seen,
        "recovery writes above anything a client saw"
    );
    let statuses: Vec<ProviderTurnStatus> = snapshot.turns.iter().map(|t| t.status).collect();
    assert_eq!(
        statuses,
        vec![
            ProviderTurnStatus::Completed,
            ProviderTurnStatus::Interrupted
        ]
    );
    assert!(snapshot
        .runtime_requests
        .iter()
        .all(|r| r.status == RuntimeRequestStatus::Expired));
    assert!(snapshot.turn_items.iter().all(|i| !i.status.is_open()));
    assert_eq!(
        snapshot.provider_session.unwrap().status,
        ProviderSessionStatus::Stopped
    );
    assert_eq!(
        rt2.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::Dormant
    );

    // The next life resumes the ACP session and sequences above the floor.
    rt2.prompt(&s.id, "hi").await.unwrap();
    host2.wait(is_terminal).await;
    assert!(host2.all()[0].sequence > snapshot_sequence);
    rt2.close(&s.id).await.unwrap();
}

// ---- per-turn model / effort / images ----------------------------------

fn last_assistant_text(host: &TestHost) -> String {
    host.items()
        .iter()
        .rev()
        .find_map(|i| match &i.kind {
            TurnItemKind::AssistantMessage { text, .. } => Some(text.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn codex_takes_model_effort_and_images_per_turn_without_a_respawn() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, dir.path().to_str().unwrap(), RuntimeProvider::Codex).await;
    let req = RuntimePromptReq {
        text: "look".into(),
        model: Some("m2".into()),
        effort: Some("high".into()),
        images: vec![PromptImage {
            mime: "image/png".into(),
            data_base64: "iVBORw0KGgo=".into(),
        }],
        ..RuntimePromptReq::default()
    };
    rt.prompt_with(&s.id, &req).await.unwrap();
    let fin = host.wait(is_terminal).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Completed));
    assert_eq!(last_assistant_text(&host), "echo: look [m2 high 1]");
    assert_eq!(host.prepared.lock().unwrap().len(), 1, "switched live");
    assert_eq!(rt.get(&s.id).await.unwrap().model.as_deref(), Some("m2"));
    let bad = RuntimePromptReq {
        images: vec![PromptImage {
            mime: "text/plain".into(),
            data_base64: "eA==".into(),
        }],
        ..RuntimePromptReq::of("x")
    };
    assert!(matches!(
        rt.prompt_with(&s.id, &bad).await,
        Err(Error::Invalid(_))
    ));
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn acp_model_switch_respawns_and_resumes_and_unsupported_input_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    rt.prompt(&s.id, "hi").await.unwrap();
    host.wait(is_terminal).await;
    let native = rt.get(&s.id).await.unwrap().native_thread_id;

    let switch = RuntimePromptReq {
        model: Some("m2".into()),
        ..RuntimePromptReq::of("again")
    };
    rt.prompt_with(&s.id, &switch).await.unwrap();
    let fin = host.wait_n(is_terminal, 2).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Completed));
    let spawns = host.prepared.lock().unwrap().clone();
    assert_eq!(spawns.len(), 2, "no live switch: a respawn");
    assert!(spawns[1].args.windows(2).any(|w| w == ["--model", "m2"]));
    let now = rt.get(&s.id).await.unwrap();
    assert_eq!(now.model.as_deref(), Some("m2"));
    assert_eq!(
        now.native_thread_id, native,
        "the session was loaded, not replaced"
    );

    // the same model again: no respawn
    rt.prompt_with(&s.id, &switch).await.unwrap();
    host.wait_n(is_terminal, 3).await;
    assert_eq!(host.prepared.lock().unwrap().len(), 2);

    let image = RuntimePromptReq {
        images: vec![PromptImage {
            mime: "image/png".into(),
            data_base64: "iVBORw0KGgo=".into(),
        }],
        ..RuntimePromptReq::of("look")
    };
    let effort = RuntimePromptReq {
        effort: Some("high".into()),
        ..RuntimePromptReq::of("think")
    };
    for req in [image, effort] {
        match rt.prompt_with(&s.id, &req).await {
            Err(Error::Invalid(m)) => assert!(m.starts_with("gemini does not"), "{m}"),
            other => panic!("{other:?}"),
        }
    }
    rt.close(&s.id).await.unwrap();
}

// ---- Antigravity (`agy acp`, fake ACP agent) -------------------------

#[tokio::test]
async fn agy_runs_on_acp_with_its_modes_model_and_context_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap();
    let skill = dir.path().join("bundle/.agents/skills/triage");
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(dir.path().join("bundle/AGENTS.md"), "Be terse.\n").unwrap();
    std::fs::write(
        skill.join("SKILL.md"),
        "---\nname: triage\ndescription: Sort bugs\n---\nbody",
    )
    .unwrap();
    let pool = pool().await;
    let host = TestHost::new(root, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                provider: Some(RuntimeProvider::Agy),
                cwd: Some(root.into()),
                model: Some("gemini-4-argon-high".into()),
                access_mode: Some(RuntimeAccessMode::AutoAcceptEdits),
                ..CreateRuntimeSessionReq::default()
            },
        )
        .await
        .unwrap();
    let say = |text: &str, n: usize| {
        let (rt, host, id, text) = (&rt, &host, s.id.clone(), text.to_string());
        async move {
            rt.prompt(&id, &text).await.unwrap();
            host.wait_n(is_terminal, n).await;
            last_assistant_text(host)
        }
    };
    // `agy acp`, no mode / model flag: both go over the protocol.
    assert_eq!(say("mode", 1).await, "mode: accept-edits");
    let spawn = host.prepared.lock().unwrap()[0].clone();
    assert_eq!(spawn.args, ["acp"]);
    assert_eq!(say("model", 2).await, "model: gemini-4-argon-high");
    assert!(host.all().iter().any(|e| matches!(&e.body,
        RuntimeEventBody::ProviderTurnUpdated { provider_turn }
            if provider_turn.turn_token_usage.as_ref().is_some_and(|u| u.input_tokens == 7
                && u.output_tokens == 3))));
    // The bundle rides the system-prompt append, not a workspace root.
    let system = say("system", 3).await;
    assert!(system.contains("Be terse."), "{system}");
    assert!(
        system.contains(&format!(
            "- triage: Sort bugs ({}/SKILL.md)",
            skill.display()
        )),
        "{system}"
    );

    // Full access is a launch flag: entering and leaving it respawns and
    // loads the session (which re-sends the bundle).
    let full = RuntimePromptReq {
        access_mode: Some(RuntimeAccessMode::FullAccess),
        ..RuntimePromptReq::of("system")
    };
    rt.prompt_with(&s.id, &full).await.unwrap();
    host.wait_n(is_terminal, 4).await;
    assert!(last_assistant_text(&host).contains("Be terse."));
    let spawns = host.prepared.lock().unwrap().clone();
    assert_eq!(spawns.len(), 2);
    assert_eq!(spawns[1].args, ["acp", "--dangerously-skip-permissions"]);
    let back = RuntimePromptReq {
        access_mode: Some(RuntimeAccessMode::ApprovalRequired),
        ..RuntimePromptReq::of("mode")
    };
    rt.prompt_with(&s.id, &back).await.unwrap();
    host.wait_n(is_terminal, 5).await;
    assert_eq!(last_assistant_text(&host), "mode: default");
    let spawns = host.prepared.lock().unwrap().clone();
    assert_eq!(spawns.len(), 3);
    assert_eq!(spawns[2].args, ["acp"]);

    // A tool that needs approval asks (agy acp can), and cancel interrupts.
    rt.prompt(&s.id, "ask").await.unwrap();
    host.wait(is_type("runtime_request.updated")).await;
    assert!(rt.interrupt(&s.id).await.unwrap());
    let fin = host.wait_n(is_terminal, 6).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Interrupted));
    rt.close(&s.id).await.unwrap();
}

// ---- Codex app-server (fake) ------------------------------------------

#[tokio::test]
async fn codex_streams_deltas_runs_an_approved_command_and_ends_the_turn() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, dir.path().to_str().unwrap(), RuntimeProvider::Codex).await;
    host.wait(is_ready).await;
    let s = rt.get(&s.id).await.unwrap();
    assert!(s
        .native_thread_id
        .as_deref()
        .unwrap()
        .starts_with("fake-th-"));
    assert_eq!(s.model.as_deref(), Some("fake-model"));
    let spec = host.prepared.lock().unwrap()[0].clone();
    assert_eq!(spec.tail_args, ["app-server"], "context flags go before it");

    // prompt → streamed deltas → turn end
    rt.prompt(&s.id, "hello").await.unwrap();
    let fin = host.wait(is_terminal).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Completed));
    let texts: Vec<(String, bool)> = host
        .all()
        .iter()
        .filter_map(|e| match &e.body {
            RuntimeEventBody::TurnItemUpdated { turn_item } => match &turn_item.kind {
                TurnItemKind::AssistantMessage {
                    text, streaming, ..
                } => Some((text.clone(), *streaming)),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(
        texts.last().unwrap(),
        &("echo: hello".to_string(), false),
        "{texts:?}"
    );
    assert!(texts.contains(&("echo: ".to_string(), true)), "{texts:?}");
    assert!(host.all().iter().any(|e| matches!(&e.body,
        RuntimeEventBody::ProviderTurnUpdated { provider_turn }
            if provider_turn.turn_token_usage.as_ref().is_some_and(|u| u.input_tokens == 5))));

    // tool call → permission round-trip → command output → turn end
    rt.prompt(&s.id, "ask").await.unwrap();
    let req = host.wait(is_type("runtime_request.updated")).await;
    let RuntimeEventBody::RuntimeRequestUpdated { runtime_request } = req.body else {
        unreachable!()
    };
    assert_eq!(
        rt.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::AwaitingPermission
    );
    rt.respond(&s.id, &runtime_request.id, ApprovalDecision::Accept)
        .await
        .unwrap();
    let fin = host.wait_n(is_terminal, 2).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Completed));
    assert!(host.all().iter().any(|e| matches!(&e.body,
        RuntimeEventBody::CommandOutputDelta { chunk, .. } if chunk == "cleaned\n")));
    let items = host.items();
    assert!(items.iter().any(|i| matches!(&i.kind,
        TurnItemKind::CommandExecution { input, exit_code: Some(0), .. } if input == "make clean")
        && i.status == TurnItemStatus::Completed));
    assert!(items.iter().any(|i| matches!(&i.kind,
        TurnItemKind::ApprovalRequest { reason: Some(r), .. } if r == "clean the build")
        && i.status == TurnItemStatus::Completed));
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn codex_interrupt_cancels_the_pending_approval_and_ends_interrupted() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, dir.path().to_str().unwrap(), RuntimeProvider::Codex).await;

    rt.prompt(&s.id, "hang").await.unwrap();
    host.wait_n(is_type("turn_item.updated"), 2).await; // user + delta
    assert!(rt.interrupt(&s.id).await.unwrap());
    let fin = host.wait(is_terminal).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Interrupted));
    assert!(host
        .items()
        .iter()
        .all(|i| !i.status.is_open() && !i.is_streaming()));

    rt.prompt(&s.id, "ask").await.unwrap();
    host.wait(is_type("runtime_request.updated")).await;
    rt.interrupt(&s.id).await.unwrap();
    let fin = host.wait_n(is_terminal, 2).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Interrupted));
    assert!(host.all().iter().any(|e| matches!(&e.body,
        RuntimeEventBody::RuntimeRequestUpdated { runtime_request }
            if runtime_request.status == RuntimeRequestStatus::Cancelled)));
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn codex_resumes_its_thread_after_a_daemon_restart() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, dir.path().to_str().unwrap(), RuntimeProvider::Codex).await;
    host.wait(is_ready).await;
    let native = rt.get(&s.id).await.unwrap().native_thread_id.unwrap();
    // A turn in flight when the daemon goes away.
    rt.prompt(&s.id, "hang").await.unwrap();
    host.wait_n(is_type("turn_item.updated"), 2).await;
    rt.flush(&s.id).await;
    rt.shutdown().await;

    let host2 = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt2 = runtime(&pool, &host2, RuntimeConfig::default());
    assert_eq!(rt2.recover().await.unwrap(), 1);
    let RuntimeReplay::Snapshot { snapshot, .. } = rt2.replay(&s.id, 0).await.unwrap() else {
        panic!("snapshot")
    };
    assert_eq!(
        snapshot.turns.iter().map(|t| t.status).collect::<Vec<_>>(),
        vec![ProviderTurnStatus::Interrupted],
        "the in-flight turn ends interrupted"
    );
    // The next prompt respawns `codex app-server` and resumes the same thread.
    rt2.prompt(&s.id, "again").await.unwrap();
    let fin = host2.wait(is_terminal).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Completed));
    assert_eq!(
        rt2.get(&s.id).await.unwrap().native_thread_id.unwrap(),
        native
    );
    rt2.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn streaming_chunks_coalesce_inside_the_window_and_flush_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    rt.prompt(&s.id, "big:200").await.unwrap();
    host.wait(is_terminal).await;
    let streamed: Vec<String> = host
        .all()
        .iter()
        .filter_map(|e| match &e.body {
            RuntimeEventBody::TurnItemUpdated { turn_item } => match &turn_item.kind {
                TurnItemKind::AssistantMessage { text, .. } => Some(text.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert!(
        streamed.len() < 200,
        "coalesced: {} upserts",
        streamed.len()
    );
    assert_eq!(streamed.last().unwrap(), &"x".repeat(200));
    // Text only grows: no held update is lost or reordered.
    assert!(streamed.windows(2).all(|w| w[1].starts_with(w[0].as_str())));
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn refused_handshake_fails_the_turn_stops_the_child_and_retries_on_the_next_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, dir.path().to_str().unwrap(), RuntimeProvider::Grok).await;
    // The refusal lands either before the prompt (400 with the reason) or
    // after it (the turn fails with it); never a prompt queued forever.
    let mut turns = 0;
    for n in 1..=2 {
        match rt.prompt(&s.id, "hi").await {
            Ok(_) => {
                turns += 1;
                let fin = host.wait_n(is_terminal, turns).await;
                assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Failed));
            }
            Err(Error::Invalid(m)) => assert!(m.contains("Authentication required"), "{m}"),
            Err(e) => panic!("{e:?}"),
        }
        host.wait_n(
            |e| {
                matches!(&e.body, RuntimeEventBody::ProviderSessionUpdated { provider_session }
                if provider_session.last_error.as_deref() == Some("Authentication required"))
            },
            n,
        )
        .await;
        assert_eq!(
            rt.get(&s.id).await.unwrap().status,
            RuntimeSessionStatus::Dormant
        );
    }
    assert!(
        host.prepared.lock().unwrap().len() >= 2,
        "each prompt respawns"
    );
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn rename_updates_the_live_entry_and_the_row_also_when_closed() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    assert!(matches!(
        rt.rename(&s.id, "  ").await,
        Err(Error::Invalid(_))
    ));
    assert_eq!(
        rt.rename(&s.id, " Fix the build ").await.unwrap().title,
        "Fix the build"
    );
    rt.close(&s.id).await.unwrap();
    rt.rename(&s.id, "Closed one").await.unwrap();
    let listed = rt.list(&"w1".into(), None).await.unwrap();
    assert_eq!(listed[0].title, "Closed one");
    // A fresh daemon life reads it from the row.
    let rt2 = runtime(
        &pool,
        &TestHost::new(dir.path().to_str().unwrap(), false),
        RuntimeConfig::default(),
    );
    assert_eq!(rt2.get(&s.id).await.unwrap().title, "Closed one");
}

// ---- access modes, parallel approvals, cancel reasons, attachments ------

fn requests(host: &TestHost) -> Vec<otto_core::agent_runtime::RuntimeRequest> {
    let mut by_id = std::collections::BTreeMap::new();
    for e in host.all() {
        if let RuntimeEventBody::RuntimeRequestUpdated { runtime_request } = e.body {
            by_id.insert(runtime_request.id.clone(), runtime_request);
        }
    }
    by_id.into_values().collect()
}

fn is_pending_request(e: &RuntimeEvent) -> bool {
    matches!(&e.body, RuntimeEventBody::RuntimeRequestUpdated { runtime_request }
        if runtime_request.status == RuntimeRequestStatus::Pending)
}

#[tokio::test]
async fn parallel_approvals_are_live_together_and_answer_out_of_order() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    let turn = rt.prompt(&s.id, "ask2").await.unwrap().turn_id;
    host.wait_n(is_pending_request, 2).await;
    let pending: Vec<_> = requests(&host)
        .into_iter()
        .filter(|r| r.status == RuntimeRequestStatus::Pending)
        .map(|r| r.id)
        .collect();
    assert_eq!(pending.len(), 2, "both live at once");
    let request = |native: &str| {
        pending
            .iter()
            .find(|id| id.ends_with(&format!(":{native}")))
            .unwrap()
            .clone()
    };
    for c in ["c1", "c2"] {
        let want = format!("{turn}:tool:{c}");
        assert!(host.items().iter().any(|i| matches!(&i.kind,
            TurnItemKind::ApprovalRequest { tool_item_id: Some(t), .. } if *t == want)));
    }
    // The second first: nothing serializes them.
    rt.respond(&s.id, &request("acp-p-c2"), ApprovalDecision::Accept)
        .await
        .unwrap();
    assert_eq!(
        rt.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::AwaitingPermission,
        "c1 still waits"
    );
    rt.respond(&s.id, &request("acp-p-c1"), ApprovalDecision::Decline)
        .await
        .unwrap();
    let fin = host.wait(is_terminal).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Completed));
    let items = host.items();
    let tool = |c: &str| {
        let id = format!("{turn}:tool:{c}");
        items.iter().find(|i| i.id == id).unwrap().clone()
    };
    assert_eq!(
        (tool("c2").status, tool("c2").error),
        (TurnItemStatus::Completed, None)
    );
    assert_eq!(
        (tool("c1").status, tool("c1").error.as_deref()),
        (TurnItemStatus::Failed, Some("denied by the user"))
    );
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn cancelled_requests_say_why() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    for (n, reason) in [(1, CancelReason::Interrupt), (2, CancelReason::Closed)] {
        rt.prompt(&s.id, "ask").await.unwrap();
        host.wait_n(is_pending_request, n).await;
        if reason == CancelReason::Interrupt {
            rt.interrupt(&s.id).await.unwrap();
            host.wait(is_terminal).await;
        } else {
            rt.close(&s.id).await.unwrap();
        }
        let suffix = format!(":acp-perm-{n}");
        let req = requests(&host)
            .into_iter()
            .find(|r| r.id.ends_with(&suffix))
            .unwrap();
        let id = req.id.clone();
        assert_eq!(
            (req.status, req.cancel_reason),
            (RuntimeRequestStatus::Cancelled, Some(reason))
        );
        assert!(host.items().iter().any(|i| matches!(&i.kind,
            TurnItemKind::ApprovalRequest { request_id, cancel_reason: Some(r), .. }
                if *request_id == id && *r == reason)
            && i.status == TurnItemStatus::Cancelled));
    }
}

#[tokio::test]
async fn access_mode_sticks_and_switches_live_or_by_respawn() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, dir.path().to_str().unwrap()).await;
    assert_eq!(s.access_mode, RuntimeAccessMode::ApprovalRequired);
    let mode = |m| RuntimePromptReq {
        access_mode: m,
        ..RuntimePromptReq::of("mode")
    };
    // Advertised by the agent: `session/set_mode` on the live process.
    host.wait(is_ready).await;
    rt.prompt_with(&s.id, &mode(Some(RuntimeAccessMode::AutoAcceptEdits)))
        .await
        .unwrap();
    host.wait(is_terminal).await;
    assert_eq!(last_assistant_text(&host), "mode: autoEdit");
    assert_eq!(host.prepared.lock().unwrap().len(), 1, "switched live");
    // It sticks.
    rt.prompt_with(&s.id, &mode(None)).await.unwrap();
    host.wait_n(is_terminal, 2).await;
    assert_eq!(last_assistant_text(&host), "mode: autoEdit");
    // Not advertised: a respawn in the new mode that loads the session.
    rt.prompt_with(&s.id, &mode(Some(RuntimeAccessMode::FullAccess)))
        .await
        .unwrap();
    host.wait_n(is_terminal, 3).await;
    assert_eq!(last_assistant_text(&host), "mode: yolo");
    let spawns = host.prepared.lock().unwrap().clone();
    assert_eq!(spawns.len(), 2);
    assert!(spawns[1]
        .args
        .windows(2)
        .any(|w| w == ["--approval-mode", "yolo"]));
    assert_eq!(
        rt.get(&s.id).await.unwrap().access_mode,
        RuntimeAccessMode::FullAccess
    );
    rt.flush(&s.id).await;
    let row = AgentRuntimeRepo::new(pool.clone())
        .get(&s.id)
        .await
        .unwrap();
    assert_eq!(row.session.access_mode, RuntimeAccessMode::FullAccess);
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn attachments_are_stored_served_and_refused_where_unsupported() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("attachments");
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let png = PromptImage {
        mime: "image/png".into(),
        data_base64: "iVBORw0KGgo=".into(),
    };
    let pdf = PromptFile {
        mime: "application/pdf".into(),
        name: "spec.pdf".into(),
        data_base64: "JVBERi0=".into(),
    };
    let user_attachments = |host: &TestHost| -> Vec<Attachment> {
        host.items()
            .iter()
            .rev()
            .find_map(|i| match &i.kind {
                TurnItemKind::UserMessage { attachments, .. } => Some(attachments.clone()),
                _ => None,
            })
            .unwrap()
    };

    // Gemini (the fake) takes embedded PDFs but no images.
    let g = create(&rt, dir.path().to_str().unwrap()).await;
    let with_pdf = RuntimePromptReq {
        files: vec![pdf.clone()],
        ..RuntimePromptReq::of("read")
    };
    rt.prompt_with(&g.id, &with_pdf).await.unwrap();
    host.wait(is_terminal).await;
    let a = user_attachments(&host);
    assert_eq!(a.len(), 1);
    assert_eq!(
        (a[0].kind, a[0].name.as_deref()),
        (AttachmentKind::File, Some("spec.pdf"))
    );
    let (mime, bytes) = rt.attachment(&g.id, &a[0].id).await.unwrap();
    assert_eq!(
        (mime.as_str(), bytes.as_slice()),
        ("application/pdf", &b"%PDF-"[..])
    );
    let refused = RuntimePromptReq {
        images: vec![png.clone()],
        ..RuntimePromptReq::of("look")
    };
    assert!(matches!(
        rt.prompt_with(&g.id, &refused).await,
        Err(Error::Invalid(_))
    ));
    let mut stored = tokio::fs::read_dir(store.join(&g.id)).await.unwrap();
    assert!(stored.next_entry().await.unwrap().is_some());
    assert!(
        stored.next_entry().await.unwrap().is_none(),
        "a refused prompt leaves no file"
    );
    for bad in ["nope", "../../etc/hosts"] {
        assert!(matches!(
            rt.attachment(&g.id, bad).await,
            Err(Error::NotFound(_))
        ));
    }
    let not_b64 = RuntimePromptReq {
        files: vec![PromptFile {
            data_base64: "!!".into(),
            ..pdf.clone()
        }],
        ..RuntimePromptReq::of("x")
    };
    assert!(matches!(
        rt.prompt_with(&g.id, &not_b64).await,
        Err(Error::Invalid(_))
    ));
    let not_pdf = RuntimePromptReq {
        files: vec![PromptFile {
            mime: "text/plain".into(),
            ..pdf.clone()
        }],
        ..RuntimePromptReq::of("x")
    };
    assert!(matches!(
        rt.prompt_with(&g.id, &not_pdf).await,
        Err(Error::Invalid(_))
    ));
    rt.close(&g.id).await.unwrap();

    // Codex: an image is served back for its thumbnail; a PDF is refused;
    // a failed command carries its output as the error.
    let c = create_as(&rt, dir.path().to_str().unwrap(), RuntimeProvider::Codex).await;
    rt.prompt_with(&c.id, &refused).await.unwrap();
    host.wait_n(is_terminal, 2).await;
    let a = user_attachments(&host);
    assert_eq!(a[0].kind, AttachmentKind::Image);
    let (mime, bytes) = rt.attachment(&c.id, &a[0].id).await.unwrap();
    assert_eq!((mime.as_str(), &bytes[..4]), ("image/png", &b"\x89PNG"[..]));
    match rt.prompt_with(&c.id, &with_pdf).await {
        Err(Error::Invalid(m)) => assert!(m.contains("Codex"), "{m}"),
        other => panic!("{other:?}"),
    }
    rt.prompt(&c.id, "fail").await.unwrap();
    host.wait_n(is_terminal, 3).await;
    assert!(host
        .items()
        .iter()
        .any(|i| i.status == TurnItemStatus::Failed
            && i.error.as_deref() == Some("boom")
            && matches!(
                &i.kind,
                TurnItemKind::CommandExecution {
                    exit_code: Some(1),
                    ..
                }
            )));
    rt.close(&c.id).await.unwrap();

    // Boot recovery drops the folders of sessions that no longer exist.
    std::fs::create_dir_all(store.join("ghost")).unwrap();
    rt.recover().await.unwrap();
    assert!(!store.join("ghost").exists());
    assert!(store.join(&g.id).exists());
}

/// A daemon-driven thread (a personal agent's): its server-owned meta is
/// stored, survives a reload and reaches the spawn; a thread it opens may get
/// its own folder and meta but stays read-only when the opener is; the run's
/// result is the latest turn's assistant text.
#[tokio::test]
async fn daemon_threads_carry_meta_and_report_their_latest_reply() {
    use otto_core::agent_runtime::OpenPeerReq;
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let own = dir.path().join("specialist");
    std::fs::create_dir_all(&own).unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let lead = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                provider: Some(RuntimeProvider::Gemini),
                cwd: Some(cwd.into()),
                meta: Some(serde_json::json!({"personal_agent": "main", "read_only": true})),
                ..CreateRuntimeSessionReq::default()
            },
        )
        .await
        .unwrap();
    assert!(lead.meta_flag("read_only"));
    let stored = AgentRuntimeRepo::new(pool.clone())
        .get(&lead.id)
        .await
        .unwrap();
    assert_eq!(stored.session.meta, lead.meta);

    rt.prompt(&lead.id, "status please").await.unwrap();
    let reply = rt.latest_reply(&lead.id, WAIT).await.unwrap();
    assert!(!reply.busy);
    assert_eq!(reply.turn_status.as_deref(), Some("completed"));
    assert!(reply.text.contains("status please"), "{}", reply.text);

    let peer = rt
        .open_peer(
            &lead.id,
            OpenPeerReq {
                provider: RuntimeProvider::Gemini,
                prompt: None,
                title: Some("Agent: Specialist".into()),
                model: None,
                effort: None,
                cwd: Some(own.to_string_lossy().into_owned()),
                meta: Some(serde_json::json!({"personal_agent": "spec"})),
            },
        )
        .await
        .unwrap();
    assert_eq!(peer.cwd, own.to_string_lossy());
    assert_eq!(peer.parent_id.as_ref(), Some(&lead.id));
    let meta = peer.meta.clone().unwrap();
    assert_eq!(meta["personal_agent"], "spec");
    assert_eq!(meta["read_only"], true, "never wider than the opener");
    // A request body can never carry meta (daemon-only).
    let parsed: CreateRuntimeSessionReq = serde_json::from_value(
        serde_json::json!({"provider": "gemini", "meta": {"read_only": false}}),
    )
    .unwrap();
    assert!(parsed.meta.is_none());
}

/// Linked threads: a thread opens another (same owner/workspace/cwd/access,
/// grouped under the root), messages it with a framed prompt, and reads its
/// reply; an unrelated thread is not reachable.
#[tokio::test]
async fn linked_threads_open_message_and_read_within_their_group() {
    use otto_core::agent_runtime::OpenPeerReq;
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let lead = create(&rt, cwd).await;
    let other = create(&rt, cwd).await;
    let worker = rt
        .open_peer(
            &lead.id,
            OpenPeerReq {
                provider: RuntimeProvider::Gemini,
                prompt: Some("you are my worker".into()),
                title: Some("worker".into()),
                model: None,
                effort: None,
                cwd: None,
                meta: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(worker.parent_id.as_ref(), Some(&lead.id));
    assert_eq!(worker.access_mode, lead.access_mode);
    // The opening prompt ran, framed with the opener.
    let first = rt.peer_reply(&lead.id, &worker.id, WAIT).await.unwrap();
    assert!(!first.busy);
    assert!(first.text.contains("Opened by thread"), "{}", first.text);
    assert!(first.text.contains("you are my worker"), "{}", first.text);

    let ids = |v: Vec<otto_core::agent_runtime::PeerInfo>| {
        let mut v: Vec<String> = v.into_iter().map(|p| p.id.to_string()).collect();
        v.sort();
        v
    };
    let mut want = vec![lead.id.to_string(), worker.id.to_string()];
    want.sort();
    assert_eq!(ids(rt.peers(&lead.id).await.unwrap()), want);
    assert_eq!(
        ids(rt.peers(&worker.id).await.unwrap()),
        want,
        "same group from either side"
    );

    // The worker's answer to what the lead asked comes back to the lead's
    // agent as its next turn ("Completion wakes this thread"), and the lead's
    // timeline notes it.
    host.wait(|e| {
        e.session_id == lead.id
            && matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
                if matches!(&turn_item.kind, TurnItemKind::UserMessage { text, .. }
                    if text.starts_with("[Reply from thread \"worker\"")))
    })
    .await;
    let woke = rt.latest_reply(&lead.id, WAIT).await.unwrap();
    assert!(woke.text.contains("you are my worker"), "{}", woke.text);
    let marker = format!("[otto-thread:{}:finished]", worker.id);
    host.wait(|e| {
        e.session_id == lead.id
            && matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
                if matches!(&turn_item.kind, TurnItemKind::SystemNotice { message } if message.ends_with(&marker)))
    })
    .await;
    assert_eq!(
        worker.meta.as_ref().unwrap()["child_of"],
        serde_json::json!({"id": lead.id, "by": "agent"})
    );

    // The worker answers the lead.
    rt.message_peer(&worker.id, &lead.id, "done").await.unwrap();
    let back = rt.peer_reply(&worker.id, &lead.id, WAIT).await.unwrap();
    assert!(back.text.contains("Message from thread"), "{}", back.text);
    assert!(back.text.contains("done"), "{}", back.text);

    assert!(matches!(
        rt.message_peer(&lead.id, &other.id, "x").await,
        Err(Error::NotFound(_))
    ));
    assert!(matches!(
        rt.message_peer(&lead.id, &lead.id, "x").await,
        Err(Error::Invalid(_))
    ));

    // Closing: never the lead (a person's), never outside the group; the
    // lead closes its worker, which then reads as closed.
    assert!(matches!(
        rt.close_peer(&worker.id, &lead.id).await,
        Err(Error::Forbidden(_))
    ));
    assert!(matches!(
        rt.close_peer(&lead.id, &other.id).await,
        Err(Error::NotFound(_))
    ));
    let closed = rt.close_peer(&lead.id, &worker.id).await.unwrap();
    assert_eq!(
        closed.status,
        otto_core::agent_runtime::RuntimeSessionStatus::Closed
    );
}

/// A person asks other agents from a thread: the children are linked to it,
/// get the message as typed (no "Opened by" frame), the parent's timeline
/// names them and notes each finished turn — and the parent's AGENT is not
/// woken (a person asked, a person reads).
#[tokio::test]
async fn a_person_asks_other_agents_from_a_thread() {
    use otto_core::agent_runtime::{AskChildReq, AskChildTarget};
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let lead = create(&rt, cwd).await;
    let target = |p| AskChildTarget {
        provider: p,
        model: None,
        effort: None,
    };

    // Never more access than the parent's.
    let wider = rt
        .ask_children(
            &lead.id,
            AskChildReq {
                prompt: "check it".into(),
                targets: vec![target(RuntimeProvider::Gemini)],
                access_mode: Some(RuntimeAccessMode::FullAccess),
                ..AskChildReq::default()
            },
        )
        .await
        .expect_err("wider than the parent");
    assert!(matches!(wider, Error::Invalid(_)), "{wider:?}");

    let kids = rt
        .ask_children(
            &lead.id,
            AskChildReq {
                prompt: "Why does CI fail?".into(),
                targets: vec![
                    target(RuntimeProvider::Gemini),
                    target(RuntimeProvider::Opencode),
                ],
                ..AskChildReq::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(kids.len(), 2);
    for k in &kids {
        assert_eq!(k.parent_id.as_ref(), Some(&lead.id));
        assert_eq!(k.cwd, lead.cwd);
        assert_eq!(k.access_mode, lead.access_mode);
        assert_eq!(
            k.meta.as_ref().unwrap()["child_of"],
            serde_json::json!({"id": lead.id, "by": "person"})
        );
        let reply = rt.latest_reply(&k.id, WAIT).await.unwrap();
        assert!(reply.text.contains("Why does CI fail?"), "{}", reply.text);
        assert!(!reply.text.contains("Opened by"), "{}", reply.text);
    }
    let started = format!("[otto-thread:{},{}:started]", kids[0].id, kids[1].id);
    for k in &kids {
        let done = format!("[otto-thread:{}:finished]", k.id);
        host.wait(|e| {
            e.session_id == lead.id
                && matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
                    if matches!(&turn_item.kind, TurnItemKind::SystemNotice { message } if message.ends_with(&done)))
        })
        .await;
    }
    assert!(
        notices(&host).iter().any(|n| n.ends_with(&started)),
        "{:?}",
        notices(&host)
    );
    // No wake for the lead's agent.
    assert!(user_texts(&host)
        .iter()
        .all(|t| !t.starts_with("[Reply from thread")));

    // Nothing to ask, or too many at once, is refused before anything starts.
    let none = rt
        .ask_children(
            &lead.id,
            AskChildReq {
                prompt: " ".into(),
                targets: vec![target(RuntimeProvider::Gemini)],
                ..AskChildReq::default()
            },
        )
        .await
        .expect_err("empty");
    assert!(matches!(none, Error::Invalid(_)), "{none:?}");
    let five = rt
        .ask_children(
            &lead.id,
            AskChildReq {
                prompt: "x".into(),
                targets: (0..5).map(|_| target(RuntimeProvider::Gemini)).collect(),
                ..AskChildReq::default()
            },
        )
        .await
        .expect_err("five");
    assert!(matches!(five, Error::Invalid(_)), "{five:?}");
}

// ---- Claude (otto-fake-claude): background work, own turns, queue, reopen ----

fn turns(host: &TestHost) -> Vec<otto_core::agent_runtime::ProviderTurn> {
    let mut by_id = std::collections::BTreeMap::new();
    for e in host.all() {
        if let RuntimeEventBody::ProviderTurnUpdated { provider_turn } = e.body {
            by_id.insert(provider_turn.id.clone(), provider_turn);
        }
    }
    let mut v: Vec<_> = by_id.into_values().collect();
    v.sort_by_key(|t| t.ordinal);
    v
}

fn is_agent_turn_running(e: &RuntimeEvent) -> bool {
    matches!(&e.body, RuntimeEventBody::ProviderTurnUpdated { provider_turn }
        if provider_turn.origin == otto_core::agent_runtime::TurnOrigin::Agent
            && provider_turn.status == ProviderTurnStatus::Running)
}

fn user_texts(host: &TestHost) -> Vec<String> {
    host.items()
        .into_iter()
        .filter_map(|i| match i.kind {
            TurnItemKind::UserMessage { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn notices(host: &TestHost) -> Vec<String> {
    host.items()
        .into_iter()
        .filter_map(|i| match i.kind {
            TurnItemKind::SystemNotice { message } => Some(message),
            _ => None,
        })
        .collect()
}

fn queued(mode: otto_core::agent_runtime::PromptDelivery, text: &str) -> RuntimePromptReq {
    RuntimePromptReq {
        delivery: Some(mode),
        ..RuntimePromptReq::of(text)
    }
}

#[tokio::test]
async fn claude_background_work_keeps_the_thread_busy_until_its_own_turn_ends() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "bg 1").await.unwrap();
    host.wait(is_terminal).await;
    // The person's turn ended, the background command did not.
    let r = rt.latest_reply(&s.id, Duration::ZERO).await.unwrap();
    assert!(r.busy, "pending background work is busy");
    assert_eq!(r.background_tasks, 1);
    assert_eq!(r.text, "Waiting for it.");
    let live = rt.get(&s.id).await.unwrap();
    assert_eq!(live.background_tasks.len(), 1);
    assert_eq!(live.background_tasks[0].kind, "command");
    // It finishes; the agent continues on its own, and the reply waits for that.
    let r = rt.latest_reply(&s.id, WAIT).await.unwrap();
    assert!(!r.busy && r.background_tasks == 0);
    assert_eq!(r.text, "Background done: ok");
    assert_eq!(r.turn_status.as_deref(), Some("completed"));
    let t = turns(&host);
    assert_eq!(t.len(), 2);
    assert_eq!(t[1].origin, otto_core::agent_runtime::TurnOrigin::Agent);
    assert_eq!(t[1].status, ProviderTurnStatus::Completed);
    assert_eq!(
        user_texts(&host),
        vec!["bg 1".to_string()],
        "no user message"
    );
    assert!(notices(&host).contains(&"Background command finished: sleep 300".to_string()));
    // The session event carried the list, then cleared it.
    let lists: Vec<usize> = host
        .all()
        .into_iter()
        .filter_map(|e| match e.body {
            RuntimeEventBody::ProviderSessionUpdated { provider_session } => {
                Some(provider_session.background_tasks.len())
            }
            _ => None,
        })
        .collect();
    assert!(lists.contains(&1) && lists.last() == Some(&0), "{lists:?}");
    assert!(rt.get(&s.id).await.unwrap().background_tasks.is_empty());
}

#[tokio::test]
async fn claude_background_subagent_card_outlives_its_turn() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "agent 1").await.unwrap();
    host.wait(is_terminal).await;
    let card = |host: &TestHost| {
        host.items()
            .into_iter()
            .find(|i| matches!(i.kind, TurnItemKind::Subagent { .. }))
            .unwrap()
    };
    let c = card(&host);
    assert_eq!(
        c.status,
        TurnItemStatus::Running,
        "the turn end left it running"
    );
    let children: Vec<TurnItem> = host
        .items()
        .into_iter()
        .filter(|i| i.parent_item_id.as_ref() == Some(&c.id))
        .collect();
    assert_eq!(children.len(), 2, "its text and its Read");
    let r = rt.latest_reply(&s.id, WAIT).await.unwrap();
    assert_eq!(
        r.text, "Agent reported",
        "the subagent's own text is not the answer"
    );
    let c = card(&host);
    assert_eq!(c.status, TurnItemStatus::Completed);
    assert!(
        matches!(&c.kind, TurnItemKind::Subagent { output: Some(o), .. } if o == "Found 2 open PRs")
    );
}

#[tokio::test]
async fn a_message_sent_during_the_agents_own_turn_waits_in_the_queue() {
    use otto_core::agent_runtime::PromptDelivery;
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "bg 0.2").await.unwrap();
    host.wait(is_agent_turn_running).await;
    // Never interleaved: a plain prompt is refused, a queued one waits.
    assert!(matches!(
        rt.prompt(&s.id, "now").await,
        Err(Error::Conflict(_))
    ));
    let r = rt
        .prompt_with(&s.id, &queued(PromptDelivery::Queue, "after"))
        .await
        .unwrap();
    assert!(r.queued_id.is_some() && r.turn_id.is_empty());
    let q = host
        .wait(|e| matches!(&e.body, RuntimeEventBody::QueueUpdated { queued_prompts } if queued_prompts.len() == 1))
        .await;
    let RuntimeEventBody::QueueUpdated { queued_prompts } = q.body else {
        unreachable!()
    };
    assert_eq!(queued_prompts[0].text, "after");
    host.wait_n(is_terminal, 3).await;
    assert_eq!(
        user_texts(&host),
        vec!["bg 0.2".to_string(), "after".into()]
    );
    assert_eq!(last_assistant_text(&host), "echo: after");
    host.wait(|e| matches!(&e.body, RuntimeEventBody::QueueUpdated { queued_prompts } if queued_prompts.is_empty()))
        .await;
    let t = turns(&host);
    assert_eq!(t.len(), 3);
    assert_eq!(t[2].origin, otto_core::agent_runtime::TurnOrigin::User);
}

#[tokio::test]
async fn steer_lands_in_the_running_turn_and_interrupt_sends_next() {
    use otto_core::agent_runtime::PromptDelivery;
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    let first = rt.prompt(&s.id, "slow 3").await.unwrap();
    host.wait(|e| matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
        if matches!(&turn_item.kind, TurnItemKind::AssistantMessage { text, streaming: false, .. } if text == "Working…")))
        .await;
    let r = rt
        .prompt_with(&s.id, &queued(PromptDelivery::Steer, "more"))
        .await
        .unwrap();
    assert!(r.steered);
    assert_eq!(r.turn_id, first.turn_id);
    host.wait(|e| matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
        if matches!(&turn_item.kind, TurnItemKind::AssistantMessage { text, .. } if text == "steered: more")))
        .await;
    // Stop & send now: the turn stops, the message goes next.
    let r = rt
        .prompt_with(&s.id, &queued(PromptDelivery::Interrupt, "next"))
        .await
        .unwrap();
    assert!(r.queued_id.is_some());
    let stopped = host.wait(is_terminal).await;
    assert_eq!(terminal(&stopped), Some(TurnTerminalStatus::Interrupted));
    let done = host.wait_n(is_terminal, 2).await;
    assert_eq!(terminal(&done), Some(TurnTerminalStatus::Completed));
    assert_eq!(last_assistant_text(&host), "echo: next");
    assert_eq!(turns(&host).len(), 2, "the steer did not start a turn");
    let steered = host
        .items()
        .into_iter()
        .find(|i| matches!(&i.kind, TurnItemKind::UserMessage { text, .. } if text == "more"))
        .unwrap();
    assert_eq!(steered.turn_id.as_ref(), Some(&first.turn_id));
}

#[tokio::test]
async fn a_queued_message_survives_a_daemon_restart() {
    use otto_core::agent_runtime::PromptDelivery;
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "slow 30").await.unwrap();
    // The CLI is up (its resume key is known) and working.
    host.wait(|e| {
        matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
        if matches!(turn_item.kind, TurnItemKind::AssistantMessage { .. }))
    })
    .await;
    rt.prompt_with(&s.id, &queued(PromptDelivery::Queue, "later"))
        .await
        .unwrap();
    rt.shutdown().await;
    drop(rt);
    let host2 = TestHost::new(cwd, false);
    let rt2 = runtime(&pool, &host2, RuntimeConfig::default());
    rt2.recover().await.unwrap();
    // The queue drains after the restart: the interrupted turn, then "later".
    host2.wait(is_terminal).await;
    assert_eq!(user_texts(&host2), vec!["later".to_string()]);
    assert_eq!(last_assistant_text(&host2), "echo: later");
    let prepared = host2.prepared.lock().unwrap().clone();
    assert!(
        prepared[0].args.iter().any(|a| a == "--resume"),
        "resumed: {:?}",
        prepared.iter().map(|p| p.args.clone()).collect::<Vec<_>>()
    );
}

// ---- threads inside a thread across a daemon restart -----------------------

fn worker_req(title: &str, prompt: Option<&str>) -> otto_core::agent_runtime::OpenPeerReq {
    otto_core::agent_runtime::OpenPeerReq {
        provider: RuntimeProvider::Gemini,
        prompt: prompt.map(str::to_string),
        title: Some(title.into()),
        model: None,
        effort: None,
        cwd: None,
        meta: None,
    }
}

/// `id`'s queue as the latest `queue.updated` of `host` shows it.
fn queue_texts(host: &TestHost, id: &Id) -> Option<Vec<String>> {
    host.all().into_iter().rev().find_map(|e| match e.body {
        RuntimeEventBody::QueueUpdated { queued_prompts } if &e.session_id == id => {
            Some(queued_prompts.into_iter().map(|q| q.text).collect())
        }
        _ => None,
    })
}

/// A Claude lead busy with a long turn while each worker its agent opened
/// answers: the replies wait in the lead's queue, oldest first. Then the
/// daemon stops.
async fn lead_with_queued_replies(
    pool: &DbPool,
    cwd: &str,
    workers: &[&str],
    person_queues: Option<&str>,
) -> RuntimeSession {
    use otto_core::agent_runtime::PromptDelivery;
    let host = TestHost::new(cwd, false);
    let rt = runtime(pool, &host, RuntimeConfig::default());
    let lead = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&lead.id, "slow 30").await.unwrap();
    host.wait(|e| {
        e.session_id == lead.id
            && matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
                if matches!(&turn_item.kind, TurnItemKind::AssistantMessage { text, streaming: false, .. } if text == "Working…"))
    })
    .await;
    for (n, title) in workers.iter().enumerate() {
        rt.open_peer(&lead.id, worker_req(title, Some("check the tests")))
            .await
            .unwrap();
        let lead_id = lead.id.clone();
        host.wait(move |e| {
            e.session_id == lead_id
                && matches!(&e.body, RuntimeEventBody::QueueUpdated { queued_prompts }
                    if queued_prompts.len() == n + 1)
        })
        .await;
    }
    if let Some(text) = person_queues {
        rt.prompt_with(&lead.id, &queued(PromptDelivery::Queue, text))
            .await
            .unwrap();
    }
    let q = queue_texts(&host, &lead.id).unwrap();
    for (title, text) in workers.iter().zip(&q) {
        assert!(
            text.starts_with(&format!("[Reply from thread \"{title}\"")),
            "{q:?}"
        );
    }
    rt.flush(&lead.id).await;
    rt.shutdown().await;
    lead
}

/// A queue holding only child threads' replies does not respawn its (now
/// dormant) parent at boot: an agent with access must not start on its own.
#[tokio::test]
async fn restart_does_not_respawn_a_parent_for_a_queued_child_reply() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let lead = lead_with_queued_replies(&pool, cwd, &["worker"], None).await;

    let host2 = TestHost::new(cwd, false);
    let rt2 = runtime(&pool, &host2, RuntimeConfig::default());
    rt2.recover().await.unwrap();
    // Proves an absence (no spawn follows the recovery).
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert!(
        host2.prepared.lock().unwrap().is_empty(),
        "no agent was started"
    );
    assert_eq!(
        rt2.get(&lead.id).await.unwrap().status,
        RuntimeSessionStatus::Dormant
    );
    let RuntimeReplay::Snapshot { snapshot, .. } = rt2.replay(&lead.id, 0).await.unwrap() else {
        panic!("a fresh daemon has no ring: snapshot")
    };
    assert_eq!(snapshot.queued_prompts.len(), 1, "the reply still waits");
    assert!(snapshot.queued_prompts[0]
        .text
        .starts_with("[Reply from thread \"worker\""));
    assert!(user_texts(&host2).is_empty(), "nothing was sent");
}

/// A message the PERSON queued still goes out after a restart, and a child's
/// reply queued before it goes first, each once.
#[tokio::test]
async fn a_person_queued_message_still_resumes_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let lead = lead_with_queued_replies(&pool, cwd, &["worker"], Some("later")).await;

    let host2 = TestHost::new(cwd, false);
    let rt2 = runtime(&pool, &host2, RuntimeConfig::default());
    rt2.recover().await.unwrap();
    host2.wait_n(is_terminal, 2).await;
    let sent = user_texts(&host2);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(
        sent[0].starts_with("[Reply from thread \"worker\""),
        "{sent:?}"
    );
    assert_eq!(sent[1], "later");
    assert_eq!(last_assistant_text(&host2), "echo: later");
    host2
        .wait(|e| {
            e.session_id == lead.id
                && matches!(&e.body, RuntimeEventBody::QueueUpdated { queued_prompts } if queued_prompts.is_empty())
        })
        .await;
    let prepared = host2.prepared.lock().unwrap().clone();
    assert_eq!(prepared.len(), 1, "one respawn");
    assert!(prepared[0].args.iter().any(|a| a == "--resume"));
    rt2.shutdown().await;
}

/// Revive sends what waited for the person: every child's reply, in the
/// order they arrived, one turn each.
#[tokio::test]
async fn queued_child_replies_are_delivered_when_the_parent_is_revived() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let lead = lead_with_queued_replies(&pool, cwd, &["worker 1", "worker 2"], None).await;

    let host2 = TestHost::new(cwd, false);
    let rt2 = runtime(&pool, &host2, RuntimeConfig::default());
    rt2.recover().await.unwrap();
    assert!(host2.prepared.lock().unwrap().is_empty());
    rt2.reopen(&lead.id).await.unwrap();
    host2.wait_n(is_terminal, 2).await;
    let sent = user_texts(&host2);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert!(
        sent[0].starts_with("[Reply from thread \"worker 1\""),
        "{sent:?}"
    );
    assert!(
        sent[1].starts_with("[Reply from thread \"worker 2\""),
        "{sent:?}"
    );
    host2
        .wait(|e| {
            e.session_id == lead.id
                && matches!(&e.body, RuntimeEventBody::QueueUpdated { queued_prompts } if queued_prompts.is_empty())
        })
        .await;
    assert_eq!(host2.prepared.lock().unwrap().len(), 1, "revived once");
    rt2.shutdown().await;
}

/// A child whose turn the restart cut off tells its parent — the card's
/// `stopped` marker and, as its agent asked, one queued reply saying so —
/// without respawning the parent, and only once.
#[tokio::test]
async fn a_child_interrupted_by_restart_reports_to_its_parent() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let lead = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    let worker = rt
        .open_peer(&lead.id, worker_req("worker", None))
        .await
        .unwrap();
    rt.flush(&lead.id).await;
    rt.flush(&worker.id).await;
    rt.shutdown().await;
    drop(rt);

    // What the dead daemon left behind: the worker mid-turn on what the
    // lead's agent asked, with part of an answer.
    let repo = AgentRuntimeRepo::new(pool.clone());
    let rec = repo.get(&worker.id).await.unwrap();
    let base = rec.seq_reserved.max(rec.session.last_sequence);
    let now = Utc::now();
    let turn = otto_core::agent_runtime::ProviderTurn {
        id: "t-cut".into(),
        ordinal: 1,
        status: ProviderTurnStatus::Running,
        started_at: Some(now),
        completed_at: None,
        turn_token_usage: None,
        origin: otto_core::agent_runtime::TurnOrigin::User,
    };
    let in_turn = |id: &str, ordinal: u32, status: TurnItemStatus, kind: TurnItemKind| {
        let mut i = otto_agent_protocol::driver::item(id.into(), status, None, kind);
        i.turn_id = Some("t-cut".into());
        i.ordinal = ordinal;
        i
    };
    let asked = format!(
        "[Opened by thread \"{}\" (claude, id {}). Answer it with the otto_thread_message tool \
         (session_id {}); see every linked thread with otto_thread_list.]\n\nlook into it",
        lead.title, lead.id, lead.id
    );
    let user = in_turn(
        "t-cut:user",
        1,
        TurnItemStatus::Completed,
        TurnItemKind::UserMessage {
            text: asked,
            attachments: Vec::new(),
        },
    );
    let partial = in_turn(
        "t-cut:a",
        2,
        TurnItemStatus::Running,
        TurnItemKind::AssistantMessage {
            message_id: "m1".into(),
            text: "Half way there".into(),
            streaming: false,
        },
    );
    let row = |kind: &str, id: &str, seq: u64, v: serde_json::Value| otto_state::RuntimeEntityRow {
        kind: kind.into(),
        entity_id: id.into(),
        sequence: seq,
        payload: v,
    };
    repo.upsert_entities(
        &worker.id,
        &[
            row(
                "turn",
                "t-cut",
                base + 1,
                serde_json::to_value(&turn).unwrap(),
            ),
            row(
                "turn_item",
                "t-cut:user",
                base + 2,
                serde_json::to_value(&user).unwrap(),
            ),
            row(
                "turn_item",
                "t-cut:a",
                base + 3,
                serde_json::to_value(&partial).unwrap(),
            ),
        ],
    )
    .await
    .unwrap();

    let host2 = TestHost::new(cwd, false);
    let rt2 = runtime(&pool, &host2, RuntimeConfig::default());
    rt2.recover().await.unwrap();
    let marker = format!("[otto-thread:{}:stopped]", worker.id);
    let lead_notices = |host: &TestHost| -> Vec<String> {
        host.all()
            .into_iter()
            .filter(|e| e.session_id == lead.id)
            .filter_map(|e| match e.body {
                RuntimeEventBody::TurnItemUpdated { turn_item } => match turn_item.kind {
                    TurnItemKind::SystemNotice { message } => Some(message),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    };
    let got = lead_notices(&host2);
    assert_eq!(
        got,
        vec![format!(
            "Gemini was interrupted when Otto restarted: Half way there {marker}"
        )]
    );
    let q = queue_texts(&host2, &lead.id).expect("a reply was queued");
    assert_eq!(q.len(), 1, "{q:?}");
    assert!(q[0].starts_with("[Reply from thread \"worker\""), "{q:?}");
    assert!(
        q[0].contains("was interrupted when Otto restarted"),
        "{q:?}"
    );
    assert!(q[0].ends_with("Half way there"), "{q:?}");
    // The worker's own card reads its turn as interrupted, not working.
    let w = rt2.latest_reply(&worker.id, Duration::ZERO).await.unwrap();
    assert!(!w.busy);
    assert_eq!(w.turn_status.as_deref(), Some("interrupted"));
    // Proves an absence (no spawn follows the report).
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        host2.prepared.lock().unwrap().is_empty(),
        "nobody respawned"
    );
    rt2.flush(&lead.id).await;
    rt2.shutdown().await;
    drop(rt2);

    // The next restart has nothing new to say.
    let host3 = TestHost::new(cwd, false);
    let rt3 = runtime(&pool, &host3, RuntimeConfig::default());
    rt3.recover().await.unwrap();
    assert!(
        lead_notices(&host3).is_empty(),
        "{:?}",
        lead_notices(&host3)
    );
    let RuntimeReplay::Snapshot { snapshot, .. } = rt3.replay(&lead.id, 0).await.unwrap() else {
        panic!("a fresh daemon has no ring: snapshot")
    };
    assert_eq!(snapshot.queued_prompts.len(), 1, "queued once");
}

/// Closing a parent does not stop its children (Threads keeps them, marked
/// detached, with their answers); what they finish afterwards is not written
/// into the closed parent.
#[tokio::test]
async fn closing_a_parent_leaves_its_children_running_and_tells_it_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let lead = create(&rt, cwd).await;
    let mut req = worker_req("worker", None);
    req.provider = RuntimeProvider::Claude;
    let worker = rt.open_peer(&lead.id, req).await.unwrap();
    rt.prompt(&worker.id, "slow 1").await.unwrap();
    rt.close(&lead.id).await.unwrap();
    let w = rt.latest_reply(&worker.id, WAIT).await.unwrap();
    assert_eq!(
        w.turn_status.as_deref(),
        Some("completed"),
        "it ran to its end"
    );
    // Proves an absence (the turn-end hook runs spawned).
    tokio::time::sleep(Duration::from_millis(300)).await;
    let finished = format!("[otto-thread:{}:finished]", worker.id);
    assert!(
        !host.all().iter().any(|e| e.session_id == lead.id
            && matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
                if matches!(&turn_item.kind, TurnItemKind::SystemNotice { message } if message.ends_with(&finished)))),
        "nothing lands in a closed parent"
    );
    assert!(queue_texts(&host, &lead.id).unwrap_or_default().is_empty());
    rt.close(&worker.id).await.unwrap();
}

#[tokio::test]
async fn reopen_resumes_the_thread_or_starts_fresh_with_a_notice() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "hi").await.unwrap();
    host.wait(is_terminal).await;
    let native = rt.get(&s.id).await.unwrap().native_thread_id.unwrap();
    rt.close(&s.id).await.unwrap();
    assert!(matches!(
        rt.prompt(&s.id, "x").await,
        Err(Error::Conflict(_))
    ));
    assert!(matches!(
        rt.notice(&s.id, "x").await,
        Err(Error::Conflict(_))
    ));
    let spawns = || host.prepared.lock().unwrap().len();
    let before = spawns();
    // Two reopens at once: one agent process.
    let (a, b) = tokio::join!(rt.reopen(&s.id), rt.reopen(&s.id));
    assert_ne!(a.unwrap().status, RuntimeSessionStatus::Closed);
    assert_ne!(b.unwrap().status, RuntimeSessionStatus::Closed);
    assert_eq!(spawns(), before + 1, "a single runner");
    let last = host.prepared.lock().unwrap().last().cloned().unwrap();
    let r = last.args.iter().position(|a| a == "--resume").unwrap();
    assert_eq!(last.args[r + 1], native);
    let stored = AgentRuntimeRepo::new(pool.clone())
        .get(&s.id)
        .await
        .unwrap();
    assert!(!stored.closed, "persisted open");
    rt.prompt(&s.id, "again").await.unwrap();
    host.wait_n(is_terminal, 2).await;
    assert_eq!(last_assistant_text(&host), "echo: again");
    // The provider lost the conversation: a fresh thread, and the timeline says so.
    rt.close(&s.id).await.unwrap();
    tokio::fs::remove_dir_all(dir.path().join(".fake-claude"))
        .await
        .unwrap();
    let reopened = rt.reopen(&s.id).await.unwrap();
    assert_ne!(reopened.status, RuntimeSessionStatus::Closed);
    assert!(
        notices(&host)
            .iter()
            .any(|n| n.contains("could not be resumed")),
        "{:?}",
        notices(&host)
    );
    let last = host.prepared.lock().unwrap().last().cloned().unwrap();
    assert!(last.args.iter().any(|a| a == "--session-id"));
    let fresh = rt.get(&s.id).await.unwrap().native_thread_id.unwrap();
    assert_ne!(fresh, native);
    rt.flush(&s.id).await;
    let stored = AgentRuntimeRepo::new(pool.clone())
        .get(&s.id)
        .await
        .unwrap();
    assert_eq!(
        stored.session.native_thread_id.as_deref(),
        Some(fresh.as_str())
    );
    rt.prompt(&s.id, "fresh").await.unwrap();
    host.wait_n(is_terminal, 3).await;
    assert_eq!(last_assistant_text(&host), "echo: fresh");
}

/// T3 Code #34912: a Stop that lands while a prompt is still starting (its
/// agent spawning) stops that turn once it runs, instead of finding nothing
/// to stop and leaving the turn running.
#[tokio::test]
async fn a_stop_landing_while_a_prompt_starts_stops_its_turn() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    let is_starting = |e: &RuntimeEvent| {
        matches!(&e.body, RuntimeEventBody::ProviderSessionUpdated { provider_session }
            if provider_session.status == ProviderSessionStatus::Starting)
    };
    // `silent` never ends on its own: only a Stop ends this turn.
    let (sent, stopped) = tokio::join!(rt.prompt(&s.id, "silent"), async {
        host.wait(is_starting).await;
        rt.interrupt(&s.id).await
    });
    let turn_id = sent.unwrap().turn_id;
    assert!(stopped.unwrap(), "the Stop was taken");
    let end = host.wait(is_terminal).await;
    assert_eq!(terminal(&end), Some(TurnTerminalStatus::Interrupted));
    assert!(
        matches!(&end.body, RuntimeEventBody::TurnTerminal { turn_id: t, .. } if *t == turn_id)
    );
    // The Stop is spent: the next prompt runs normally.
    rt.prompt(&s.id, "after").await.unwrap();
    host.wait_n(is_terminal, 2).await;
    assert_eq!(last_assistant_text(&host), "echo: after");
}

/// T3 Code #34505: a job the Claude CLI started in a process group of its
/// own outlives the group kill; closing the session stops it too.
#[tokio::test]
async fn closing_a_claude_session_stops_jobs_it_moved_out_of_its_group() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "escape").await.unwrap();
    host.wait(is_terminal).await;
    let pid: i32 = std::fs::read_to_string(dir.path().join(".fake-claude/escaped.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    // SAFETY: signal 0 only checks that the pid exists.
    let alive = || unsafe { libc::kill(pid, 0) } == 0;
    assert!(alive(), "the job runs");
    // Let a poll record the tree, then close.
    tokio::time::sleep(Duration::from_millis(2500)).await;
    rt.close(&s.id).await.unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    while alive() {
        assert!(
            std::time::Instant::now() < deadline,
            "the escaped job {pid} still runs"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// T3 Code #34762: a resume refused for a passing reason keeps the
/// provider's conversation — Otto resumes once more instead of starting a
/// fresh thread the agent remembers nothing of.
#[tokio::test]
async fn a_resume_refused_for_a_passing_reason_keeps_the_conversation() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "hi").await.unwrap();
    host.wait(is_terminal).await;
    let native = rt.get(&s.id).await.unwrap().native_thread_id.unwrap();
    rt.close(&s.id).await.unwrap();
    // The next resume fails once with a connection error.
    std::fs::write(
        dir.path()
            .join(".fake-claude")
            .join(format!("{native}.flaky")),
        b"",
    )
    .unwrap();
    let before = host.prepared.lock().unwrap().len();
    let reopened = rt.reopen(&s.id).await.unwrap();
    assert_ne!(reopened.status, RuntimeSessionStatus::Closed);
    let prepared = host.prepared.lock().unwrap().clone();
    assert_eq!(prepared.len(), before + 2, "resumed twice");
    for p in &prepared[before..] {
        let r = p.args.iter().position(|a| a == "--resume").unwrap();
        assert_eq!(p.args[r + 1], native);
    }
    assert!(
        !notices(&host)
            .iter()
            .any(|n| n.contains("could not be resumed")),
        "{:?}",
        notices(&host)
    );
    assert_eq!(
        rt.get(&s.id).await.unwrap().native_thread_id.as_deref(),
        Some(native.as_str())
    );
    rt.prompt(&s.id, "still there").await.unwrap();
    host.wait_n(is_terminal, 2).await;
    assert_eq!(last_assistant_text(&host), "echo: still there");
}

#[tokio::test]
async fn revive_dormant_thread_resumes_without_sending_and_keeps_live_runner() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "keep this history").await.unwrap();
    host.wait(is_terminal).await;
    rt.flush(&s.id).await;
    let native = rt.get(&s.id).await.unwrap().native_thread_id.unwrap();
    let before = AgentRuntimeRepo::new(pool.clone())
        .entities(&s.id)
        .await
        .unwrap();
    rt.shutdown().await;

    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    assert_eq!(
        rt.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::Dormant
    );
    let (a, b) = tokio::join!(rt.reopen(&s.id), rt.reopen(&s.id));
    assert_eq!(a.unwrap().id, s.id);
    assert_eq!(b.unwrap().id, s.id);
    assert_eq!(
        host.prepared.lock().unwrap().len(),
        1,
        "revival starts one runner"
    );
    host.wait(is_ready).await;
    assert_eq!(
        rt.get(&s.id).await.unwrap().native_thread_id.as_deref(),
        Some(native.as_str())
    );
    assert!(
        !host.all().iter().any(is_terminal),
        "revival sends no prompt"
    );
    rt.reopen(&s.id).await.unwrap();
    assert_eq!(
        host.prepared.lock().unwrap().len(),
        1,
        "a live runner is preserved"
    );
    rt.flush(&s.id).await;
    let after = AgentRuntimeRepo::new(pool.clone())
        .entities(&s.id)
        .await
        .unwrap();
    assert!(before.iter().any(|row| row.kind == "turn_item"));
    for item in before.iter().filter(|row| row.kind == "turn_item") {
        assert!(
            after
                .iter()
                .any(|row| row.entity_id == item.entity_id && row.payload == item.payload),
            "history item is preserved"
        );
    }
    rt.prompt(&s.id, "continue").await.unwrap();
    host.wait(is_terminal).await;
    assert_eq!(last_assistant_text(&host), "echo: continue");
    rt.shutdown().await;
}

/// After a restart the chat is dormant; a prompt respawns the agent with
/// `--resume`. If the provider lost (or never saved) that conversation, the
/// prompt still gets an answer on a fresh thread instead of failing forever.
#[tokio::test]
async fn a_prompt_to_a_dormant_thread_the_provider_lost_starts_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let s = {
        let rt = runtime(&pool, &host, RuntimeConfig::default());
        let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
        rt.prompt(&s.id, "hi").await.unwrap();
        host.wait(is_terminal).await;
        rt.flush(&s.id).await;
        rt.shutdown().await;
        s
    };
    let native = AgentRuntimeRepo::new(pool.clone())
        .get(&s.id)
        .await
        .unwrap()
        .session
        .native_thread_id
        .unwrap();
    tokio::fs::remove_dir_all(dir.path().join(".fake-claude"))
        .await
        .unwrap();
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    rt.prompt(&s.id, "still there?").await.unwrap();
    host.wait(is_terminal).await;
    assert_eq!(last_assistant_text(&host), "echo: still there?");
    assert!(notices(&host)
        .iter()
        .any(|n| n.contains("could not be resumed")));
    let fresh = rt.get(&s.id).await.unwrap().native_thread_id.unwrap();
    assert_ne!(fresh, native);
}

#[tokio::test]
async fn delete_removes_the_thread_and_its_history() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "hi").await.unwrap();
    host.wait(is_terminal).await;
    rt.delete(&s.id).await.unwrap();
    assert!(matches!(rt.get(&s.id).await, Err(Error::NotFound(_))));
    let repo = AgentRuntimeRepo::new(pool.clone());
    assert!(matches!(repo.get(&s.id).await, Err(Error::NotFound(_))));
    assert!(repo.entities(&s.id).await.unwrap().is_empty());
    assert!(rt.list(&"w1".into(), None).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_silent_turn_is_stopped_as_stalled_but_background_work_is_no_stall() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(
        &pool,
        &host,
        RuntimeConfig {
            stall_after: Duration::from_millis(1200),
            stall_tool_after: Duration::from_millis(1200),
            interrupt_grace: Duration::from_secs(5),
            ..RuntimeConfig::default()
        },
    );
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    host.wait(is_ready).await;
    // Let the child pump park between turns. No later provider output may be
    // needed to arm the watchdog when the next prompt is completely silent.
    tokio::time::sleep(Duration::from_millis(100)).await;
    rt.prompt(&s.id, "silent").await.unwrap();
    let t = host.wait(is_terminal).await;
    assert_eq!(terminal(&t), Some(TurnTerminalStatus::Interrupted));
    assert!(
        matches!(&t.body, RuntimeEventBody::TurnTerminal { reason: Some(r), .. } if r == "stalled")
    );
    assert!(notices(&host)
        .iter()
        .any(|n| n.starts_with("Stopped: the agent showed no progress")));
    // Two seconds of background work between turns: nothing is stopped.
    rt.prompt(&s.id, "bg 2").await.unwrap();
    let r = rt.latest_reply(&s.id, WAIT).await.unwrap();
    assert!(!r.busy);
    assert_eq!(r.text, "Background done: ok");
    let ends: Vec<TurnTerminalStatus> = host.all().iter().filter_map(terminal).collect();
    assert_eq!(
        ends,
        vec![
            TurnTerminalStatus::Interrupted,
            TurnTerminalStatus::Completed,
            TurnTerminalStatus::Completed
        ]
    );
}

// Explicit fresh context never creates a second Otto thread or rewrites its history.
#[tokio::test]
async fn fresh_context_keeps_history_and_changes_native_session_only_on_send() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "before").await.unwrap();
    host.wait(is_terminal).await;
    let old = rt.get(&s.id).await.unwrap().native_thread_id;
    // A closed thread starts fresh directly, without first resuming the old agent.
    rt.close(&s.id).await.unwrap();
    let sent = rt
        .prompt_with(
            &s.id,
            &RuntimePromptReq {
                text: "after".into(),
                fresh_context: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    host.wait_n(is_terminal, 2).await;
    let current = rt.get(&s.id).await.unwrap();
    assert_eq!(current.id, s.id);
    assert_eq!(current.cwd, s.cwd);
    assert_eq!(current.workspace_id, s.workspace_id);
    assert_ne!(current.native_thread_id, old);
    let spawns = host.prepared.lock().unwrap().clone();
    assert_eq!(spawns.len(), 2);
    assert!(!spawns[1].args.iter().any(|a| a == "--resume"));
    let items = host.items();
    let boundaries: Vec<_> = items
        .iter()
        .filter(|i| matches!(i.kind, TurnItemKind::SessionBoundary))
        .collect();
    assert_eq!(boundaries.len(), 1);
    assert_eq!(boundaries[0].turn_id.as_ref(), Some(&sent.turn_id));
    for text in ["before", "after"] {
        assert!(items
            .iter()
            .any(|i| matches!(&i.kind, TurnItemKind::UserMessage { text: t, .. } if t == text)));
    }
    rt.flush(&s.id).await;
    let stored = AgentRuntimeRepo::new(pool.clone())
        .get(&s.id)
        .await
        .unwrap();
    assert_eq!(stored.session.native_thread_id, current.native_thread_id);
    assert!(!stored.closed);
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn fresh_context_refuses_pending_and_queued_work_and_scopes_reused_request_ids() {
    use otto_core::agent_runtime::PromptDelivery;
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, cwd).await;
    rt.prompt(&s.id, "ask").await.unwrap();
    let event = host.wait(is_pending_request).await;
    let RuntimeEventBody::RuntimeRequestUpdated {
        runtime_request: first,
    } = event.body
    else {
        panic!()
    };
    let queued = rt
        .prompt_with(&s.id, &queued(PromptDelivery::Queue, "queued"))
        .await
        .unwrap();
    let fresh = RuntimePromptReq {
        text: "ask".into(),
        fresh_context: true,
        ..Default::default()
    };
    assert!(matches!(
        rt.prompt_with(&s.id, &fresh).await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(host.prepared.lock().unwrap().len(), 1);
    assert!(!host
        .items()
        .iter()
        .any(|i| matches!(i.kind, TurnItemKind::SessionBoundary)));
    rt.unqueue(&s.id, queued.queued_id.as_ref().unwrap())
        .await
        .unwrap();
    rt.respond(&s.id, &first.id, ApprovalDecision::Accept)
        .await
        .unwrap();
    host.wait(is_terminal).await;
    rt.prompt_with(&s.id, &fresh).await.unwrap();
    let event = host.wait_n(is_pending_request, 2).await;
    let RuntimeEventBody::RuntimeRequestUpdated {
        runtime_request: second,
    } = event.body
    else {
        panic!()
    };
    assert_ne!(
        first.id, second.id,
        "both providers use native perm-1, UI IDs must differ"
    );
    assert!(rt
        .respond(&s.id, &first.id, ApprovalDecision::Accept)
        .await
        .is_err());
    let all = requests(&host);
    assert!(all
        .iter()
        .any(|r| r.id == first.id && r.status == RuntimeRequestStatus::Resolved));
    assert!(all
        .iter()
        .any(|r| r.id == second.id && r.status == RuntimeRequestStatus::Pending));
    rt.respond(&s.id, &second.id, ApprovalDecision::Decline)
        .await
        .unwrap();
    host.wait_n(is_terminal, 2).await;
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn rejected_fresh_prompt_preserves_old_native_identity_without_a_boundary() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Codex).await;
    rt.prompt(&s.id, "before").await.unwrap();
    host.wait(is_terminal).await;
    let old = rt.get(&s.id).await.unwrap().native_thread_id;
    let rejected = RuntimePromptReq {
        text: "unsupported file".into(),
        fresh_context: true,
        files: vec![PromptFile {
            mime: "application/pdf".into(),
            name: "test.pdf".into(),
            data_base64: "YQ==".into(),
        }],
        ..Default::default()
    };
    assert!(rt.prompt_with(&s.id, &rejected).await.is_err());
    assert_eq!(rt.get(&s.id).await.unwrap().native_thread_id, old);
    assert!(!host
        .items()
        .iter()
        .any(|i| matches!(i.kind, TurnItemKind::SessionBoundary)));
    rt.flush(&s.id).await;
    assert_eq!(
        AgentRuntimeRepo::new(pool.clone())
            .get(&s.id)
            .await
            .unwrap()
            .session
            .native_thread_id,
        old
    );
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn fresh_context_refuses_background_work_after_the_foreground_turn_ends() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "bg 10").await.unwrap();
    host.wait(is_terminal).await;
    assert!(!rt.get(&s.id).await.unwrap().background_tasks.is_empty());
    let fresh = RuntimePromptReq {
        fresh_context: true,
        ..RuntimePromptReq::of("after")
    };
    assert!(matches!(
        rt.prompt_with(&s.id, &fresh).await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(host.prepared.lock().unwrap().len(), 1);
    assert!(!rt.get(&s.id).await.unwrap().background_tasks.is_empty());
    rt.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn fresh_context_refuses_closed_threads_with_queued_prompts() {
    use otto_core::agent_runtime::PromptDelivery;
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create(&rt, cwd).await;
    rt.prompt(&s.id, "ask").await.unwrap();
    host.wait(is_pending_request).await;
    let q = rt
        .prompt_with(&s.id, &queued(PromptDelivery::Queue, "later"))
        .await
        .unwrap();
    rt.close(&s.id).await.unwrap();
    let fresh = RuntimePromptReq {
        fresh_context: true,
        ..RuntimePromptReq::of("after")
    };
    assert!(matches!(
        rt.prompt_with(&s.id, &fresh).await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(
        rt.get(&s.id).await.unwrap().status,
        RuntimeSessionStatus::Closed
    );
    assert_eq!(host.prepared.lock().unwrap().len(), 1);
    assert!(rt
        .unqueue(&s.id, q.queued_id.as_ref().unwrap())
        .await
        .unwrap());
}

#[tokio::test]
async fn fresh_context_failed_spawn_preserves_closed_history_and_resume_identity() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, cwd, RuntimeProvider::Claude).await;
    rt.prompt(&s.id, "before").await.unwrap();
    host.wait(is_terminal).await;
    rt.close(&s.id).await.unwrap();
    rt.flush(&s.id).await;
    let old = rt.get(&s.id).await.unwrap().native_thread_id;
    // A new daemon whose spawn safety pipeline rejects the launch.
    let refusing = TestHost::new(cwd, true);
    let rt = runtime(&pool, &refusing, RuntimeConfig::default());
    rt.recover().await.unwrap();
    let fresh = RuntimePromptReq {
        fresh_context: true,
        ..RuntimePromptReq::of("after")
    };
    assert!(rt.prompt_with(&s.id, &fresh).await.is_err());
    let current = rt.get(&s.id).await.unwrap();
    assert_eq!(current.status, RuntimeSessionStatus::Closed);
    assert_eq!(current.native_thread_id, old);
    assert!(!refusing
        .items()
        .iter()
        .any(|i| matches!(i.kind, TurnItemKind::SessionBoundary)));
    rt.flush(&s.id).await;
    let stored = AgentRuntimeRepo::new(pool.clone())
        .get(&s.id)
        .await
        .unwrap();
    assert_eq!(stored.session.native_thread_id, old);
    assert!(stored.closed);
}

#[tokio::test]
async fn codex_questions_validate_answers_and_replay_without_tool_rows() {
    use otto_core::agent_runtime::{QuestionAnswer, RequestKind};
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, dir.path().to_str().unwrap(), RuntimeProvider::Codex).await;
    host.wait(is_ready).await;
    rt.prompt(&s.id, "question form").await.unwrap();
    let event = host.wait(is_type("runtime_request.updated")).await;
    let RuntimeEventBody::RuntimeRequestUpdated {
        runtime_request: request,
    } = event.body
    else {
        panic!("missing request")
    };
    assert_eq!(request.kind, RequestKind::Question);
    assert_eq!(request.questions.len(), 2);
    assert!(rt
        .respond(&s.id, &request.id, ApprovalDecision::Accept)
        .await
        .is_err());
    assert!(rt
        .respond_question(&s.id, &request.id, vec![])
        .await
        .is_err());
    assert_eq!(
        requests(&host).last().unwrap().status,
        RuntimeRequestStatus::Pending
    );
    let answers = vec![
        QuestionAnswer::Single {
            question_id: "strategy_native".into(),
            value: "Careful".into(),
        },
        QuestionAnswer::Text {
            question_id: "note_native".into(),
            text: "Keep the API stable".into(),
        },
    ];
    rt.respond_question(&s.id, &request.id, answers.clone())
        .await
        .unwrap();
    host.wait(is_terminal).await;
    let resolved = requests(&host)
        .into_iter()
        .find(|r| r.status == RuntimeRequestStatus::Resolved)
        .unwrap();
    assert_eq!(resolved.answers, Some(answers.clone()));
    assert!(rt
        .respond_question(&s.id, &request.id, answers)
        .await
        .is_err());
    assert!(host
        .items()
        .iter()
        .all(|i| !matches!(i.kind, TurnItemKind::ApprovalRequest { .. })));
    assert!(host.items().iter().any(|i| matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if text.contains("strategy_native") && text.contains("Keep the API stable"))));
    // Skip settles only the question; it does not create an approval decision.
    rt.prompt(&s.id, "question form").await.unwrap();
    let event = host.wait_n(is_pending_request, 2).await;
    let RuntimeEventBody::RuntimeRequestUpdated {
        runtime_request: next,
    } = event.body
    else {
        panic!("missing second question")
    };
    rt.dismiss_question(
        &s.id,
        &next.id,
        otto_core::agent_runtime::QuestionDismissAction::Decline,
    )
    .await
    .unwrap();
    host.wait_n(is_terminal, 2).await;
    let dismissed = requests(&host)
        .into_iter()
        .find(|r| r.id == next.id)
        .unwrap();
    assert_eq!(dismissed.status, RuntimeRequestStatus::Cancelled);
    assert!(dismissed.decision.is_none());
    // Full request data survives replay and restart; the live answer authority does not.
    rt.prompt(&s.id, "question form").await.unwrap();
    let pending = host.wait_n(is_pending_request, 3).await;
    let RuntimeEventBody::RuntimeRequestUpdated {
        runtime_request: pending,
    } = pending.body
    else {
        panic!("missing pending question")
    };
    rt.flush(&s.id).await;
    rt.shutdown().await;
    let host2 = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt2 = runtime(&pool, &host2, RuntimeConfig::default());
    rt2.recover().await.unwrap();
    let RuntimeReplay::Snapshot { snapshot, .. } = rt2.replay(&s.id, 0).await.unwrap() else {
        panic!("restart must replay a snapshot")
    };
    let expired = snapshot
        .runtime_requests
        .iter()
        .find(|r| r.id == pending.id)
        .unwrap();
    assert_eq!(expired.questions, pending.questions);
    assert_eq!(expired.status, RuntimeRequestStatus::Expired);
    assert!(matches!(
        expired.response_capability,
        otto_core::agent_runtime::ResponseCapability::NotResumable { .. }
    ));
    assert!(snapshot
        .runtime_requests
        .iter()
        .any(|r| r.id == request.id && r.answers == resolved.answers));
    assert!(rt2
        .respond_question(&s.id, &pending.id, vec![])
        .await
        .is_err());
    rt2.close(&s.id).await.unwrap();
}

#[tokio::test]
async fn fresh_context_waits_for_questions_and_old_cards_cannot_answer_new_process() {
    use otto_core::agent_runtime::{QuestionAnswer, QuestionDismissAction};
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, dir.path().to_str().unwrap(), RuntimeProvider::Codex).await;
    host.wait(is_ready).await;
    rt.prompt(&s.id, "question form").await.unwrap();
    let event = host.wait(is_pending_request).await;
    let RuntimeEventBody::RuntimeRequestUpdated {
        runtime_request: first,
    } = event.body
    else {
        panic!("question missing")
    };
    let fresh = RuntimePromptReq {
        fresh_context: true,
        ..RuntimePromptReq::of("question form")
    };
    assert!(rt.prompt_with(&s.id, &fresh).await.is_err());
    let answers = vec![
        QuestionAnswer::Single {
            question_id: "strategy_native".into(),
            value: "Careful".into(),
        },
        QuestionAnswer::Text {
            question_id: "note_native".into(),
            text: "Keep the public API".into(),
        },
    ];
    rt.respond_question(&s.id, &first.id, answers.clone())
        .await
        .unwrap();
    host.wait(is_terminal).await;
    rt.prompt_with(&s.id, &fresh).await.unwrap();
    let event = host.wait_n(is_pending_request, 2).await;
    let RuntimeEventBody::RuntimeRequestUpdated {
        runtime_request: second,
    } = event.body
    else {
        panic!("replacement question missing")
    };
    assert_ne!(
        first.id, second.id,
        "new process reuses its native RPC ID but not its public request ID"
    );
    assert!(rt
        .respond_question(&s.id, &first.id, answers.clone())
        .await
        .is_err());
    assert!(rt
        .dismiss_question(&s.id, &first.id, QuestionDismissAction::Decline)
        .await
        .is_err());
    rt.dismiss_question(&s.id, &second.id, QuestionDismissAction::Decline)
        .await
        .unwrap();
    host.wait_n(is_terminal, 2).await;
    assert_eq!(
        host.items()
            .iter()
            .filter(|i| matches!(i.kind, TurnItemKind::SessionBoundary))
            .count(),
        1
    );
    rt.flush(&s.id).await;
    rt.shutdown().await;
    let host2 = TestHost::new(dir.path().to_str().unwrap(), false);
    let recovered = runtime(&pool, &host2, RuntimeConfig::default());
    recovered.recover().await.unwrap();
    let RuntimeReplay::Snapshot { snapshot, .. } = recovered.replay(&s.id, 0).await.unwrap() else {
        panic!("snapshot missing")
    };
    let old = snapshot
        .runtime_requests
        .iter()
        .find(|r| r.id == first.id)
        .unwrap();
    assert_eq!(old.answers, Some(answers));
    assert_eq!(old.status, RuntimeRequestStatus::Resolved);
    assert_eq!(
        snapshot
            .runtime_requests
            .iter()
            .find(|r| r.id == second.id)
            .unwrap()
            .status,
        RuntimeRequestStatus::Cancelled
    );
    recovered.close(&s.id).await.unwrap();
}

/// Embedded threads (a PR's Ask / Explain / Fix, a connection's diagnosis):
/// the create body's `source` / `origin` land in the server-owned meta, the
/// page finds the thread again by them (closed ones too), a thread it opens
/// stays grouped with it, and the auto-close pass closes only the idle ones —
/// never one with a running turn or a pending approval, never a plain thread.
#[tokio::test]
async fn embedded_threads_are_found_by_origin_and_auto_close_only_when_idle() {
    use chrono::TimeDelta;
    use otto_core::agent_runtime::{CreateRuntimeSessionBody, OpenPeerReq};
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let embedded = |origin_id: &str| {
        serde_json::from_value::<CreateRuntimeSessionBody>(serde_json::json!({
            "provider": "gemini",
            "cwd": cwd,
            "source": "pr-agent",
            "origin": {"kind": "pr", "id": origin_id, "label": "Fix the parser"},
            "meta": {"read_only": true},
        }))
        .unwrap()
        .into_req()
        .unwrap()
    };
    let (u, w): (Id, Id) = ("u1".into(), "w1".into());
    let idle = rt.create(&u, &w, embedded("repo#7")).await.unwrap();
    let asking = rt.create(&u, &w, embedded("repo#7")).await.unwrap();
    let working = rt.create(&u, &w, embedded("repo#7")).await.unwrap();
    let other_pr = rt.create(&u, &w, embedded("repo#8")).await.unwrap();
    let plain = create(&rt, cwd).await;
    let meta = idle.meta.clone().unwrap();
    assert_eq!(meta["source"], "pr-agent");
    assert_eq!(meta["origin"]["id"], "repo#7");
    assert_eq!(meta["origin"]["label"], "Fix the parser");
    assert!(meta.get("read_only").is_none(), "a body's meta never lands");
    assert_eq!(rt.get(&idle.id).await.unwrap().meta, idle.meta);

    let found = rt
        .list_filtered(&w, Some(&u), Some("pr-agent"), Some("repo#7"))
        .await
        .unwrap();
    let mut found_ids: Vec<Id> = found.iter().map(|s| s.id.clone()).collect();
    found_ids.sort();
    let mut want = vec![idle.id.clone(), asking.id.clone(), working.id.clone()];
    want.sort();
    assert_eq!(found_ids, want);
    assert!(found.iter().all(|s| s.meta.as_ref() == Some(&meta)));

    rt.prompt(&asking.id, "ask").await.unwrap();
    host.wait(|e| e.session_id == asking.id && e.body.type_name() == "runtime_request.updated")
        .await;
    rt.prompt(&working.id, "hang").await.unwrap();

    // Not idle long enough yet, or switched off: nothing closes.
    assert!(rt
        .auto_close_idle_embedded(12, Utc::now())
        .await
        .unwrap()
        .is_empty());
    let later = Utc::now() + TimeDelta::hours(13);
    assert!(rt
        .auto_close_idle_embedded(0, later)
        .await
        .unwrap()
        .is_empty());
    let mut closed = rt.auto_close_idle_embedded(12, later).await.unwrap();
    closed.sort();
    let mut want = vec![idle.id.clone(), other_pr.id.clone()];
    want.sort();
    assert_eq!(closed, want, "only the idle embedded threads");
    rt.flush(&idle.id).await;
    assert_eq!(
        rt.get(&idle.id).await.unwrap().status,
        RuntimeSessionStatus::Closed
    );
    assert_eq!(
        rt.get(&asking.id).await.unwrap().status,
        RuntimeSessionStatus::AwaitingPermission,
        "the person's pending answer is kept"
    );
    for s in [&working, &plain] {
        assert_ne!(
            rt.get(&s.id).await.unwrap().status,
            RuntimeSessionStatus::Closed
        );
    }
    // Closed, not deleted: still found by the page.
    let found = rt
        .list_filtered(&w, Some(&u), Some("pr-agent"), Some("repo#7"))
        .await
        .unwrap();
    assert_eq!(found.len(), 3);
    assert!(found
        .iter()
        .any(|s| s.id == idle.id && s.status == RuntimeSessionStatus::Closed));

    // A thread opened from an embedded one stays grouped with it.
    let peer = rt
        .open_peer(
            &asking.id,
            OpenPeerReq {
                provider: RuntimeProvider::Gemini,
                prompt: None,
                title: None,
                model: None,
                effort: None,
                cwd: None,
                meta: None,
            },
        )
        .await
        .unwrap();
    let peer_meta = peer.meta.clone().unwrap();
    assert_eq!(peer_meta["source"], meta["source"]);
    assert_eq!(peer_meta["origin"], meta["origin"]);
    assert_eq!(
        rt.list_filtered(&w, Some(&u), None, Some("repo#7"))
            .await
            .unwrap()
            .len(),
        4
    );

    for s in [&idle, &asking, &working, &plain] {
        rt.close(&s.id).await.unwrap();
    }
    rt.close(&peer.id).await.unwrap();
}

// ---- council: one prompt to several agents, then a judge ----

fn seat(
    provider: RuntimeProvider,
    effort: Option<&str>,
) -> otto_core::agent_runtime::CouncilSeatReq {
    otto_core::agent_runtime::CouncilSeatReq {
        provider,
        model: None,
        effort: effort.map(str::to_string),
    }
}

fn council_req(
    prompt: &str,
    seats: Vec<otto_core::agent_runtime::CouncilSeatReq>,
) -> otto_core::agent_runtime::CreateCouncilReq {
    otto_core::agent_runtime::CreateCouncilReq {
        prompt: prompt.into(),
        title: None,
        cwd: None,
        seats,
        images: Vec::new(),
    }
}

/// A council is one linked group (seat 1 the root), every seat read-only and
/// approval-required, each sent the prompt (the same provider may sit
/// twice); the judge is linked to seat 1, reads every answer and gets the
/// daemon's own prompt without the "Opened by" header.
#[tokio::test]
async fn council_seats_form_one_read_only_group_and_the_judge_reads_every_answer() {
    use otto_core::agent_runtime::{CouncilRole, SynthesizeCouncilReq};
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let seats = rt
        .create_council(
            &"u1".into(),
            &"w1".into(),
            council_req(
                "which cache?",
                vec![
                    seat(RuntimeProvider::Gemini, None),
                    seat(RuntimeProvider::Gemini, None),
                    seat(RuntimeProvider::Codex, Some("low")),
                ],
            ),
        )
        .await
        .unwrap();
    assert_eq!(seats.len(), 3);
    let root = &seats[0];
    assert!(root.parent_id.is_none());
    let council = root.council().unwrap();
    for (i, s) in seats.iter().enumerate() {
        let c = s.council().unwrap();
        assert_eq!(
            (c.id.as_str(), c.seat, c.role),
            (council.id.as_str(), i as u32 + 1, CouncilRole::Seat)
        );
        assert!(s.meta_flag("read_only"), "seat {} must only read", i + 1);
        assert_eq!(s.access_mode, RuntimeAccessMode::ApprovalRequired);
        if i > 0 {
            assert_eq!(s.parent_id.as_ref(), Some(&root.id));
        }
    }
    assert_eq!(seats[1].title, "which cache? · gemini");
    for s in &seats[..2] {
        let r = rt.latest_reply(&s.id, WAIT).await.unwrap();
        assert!(!r.busy);
        assert!(r.text.contains("which cache?"), "{}", r.text);
    }
    let codex = rt.latest_reply(&seats[2].id, WAIT).await.unwrap();
    assert!(
        codex.text.contains(" low 0]"),
        "the seat's effort rode its turn: {}",
        codex.text
    );

    let judge = rt
        .synthesize_council(
            &seats[2].id,
            SynthesizeCouncilReq {
                provider: RuntimeProvider::Gemini,
                model: None,
                effort: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(judge.parent_id.as_ref(), Some(&root.id));
    let jc = judge.council().unwrap();
    assert_eq!(
        (jc.id, jc.seat, jc.role),
        (council.id.clone(), 0, CouncilRole::Synthesis)
    );
    assert!(judge.meta_flag("read_only"));
    assert_eq!(judge.title, "which cache? · synthesis");
    let verdict = rt.latest_reply(&judge.id, WAIT).await.unwrap();
    assert!(
        !verdict.text.contains("Opened by thread"),
        "{}",
        verdict.text
    );
    for want in [
        "## The question",
        "which cache?",
        "## Seat 1 — gemini",
        "## Seat 3 — codex",
        "disagree",
    ] {
        assert!(verdict.text.contains(want), "{want}: {}", verdict.text);
    }

    // A thread outside any council has nothing to synthesize.
    let plain = create(&rt, cwd).await;
    assert!(matches!(
        rt.synthesize_council(
            &plain.id,
            SynthesizeCouncilReq {
                provider: RuntimeProvider::Gemini,
                model: None,
                effort: None,
            },
        )
        .await,
        Err(Error::Invalid(_))
    ));
}

/// 2–4 seats and a prompt, else 400; a seat that cannot start removes the
/// seats already made (all or none) and the error names it.
#[tokio::test]
async fn council_is_created_all_or_none() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let (u, w): (Id, Id) = ("u1".into(), "w1".into());
    let g = || seat(RuntimeProvider::Gemini, None);
    for bad in [
        council_req("q", vec![g()]),
        council_req("q", vec![g(), g(), g(), g(), g()]),
        council_req("   ", vec![g(), g()]),
    ] {
        assert!(matches!(
            rt.create_council(&u, &w, bad).await,
            Err(Error::Invalid(_))
        ));
    }
    assert!(
        rt.list(&w, None).await.unwrap().is_empty(),
        "nothing spawned"
    );

    *host.refuse_provider.lock().unwrap() = Some(RuntimeProvider::Codex);
    let err = rt
        .create_council(
            &u,
            &w,
            council_req("q", vec![g(), g(), seat(RuntimeProvider::Codex, None)]),
        )
        .await
        .expect_err("seat 3 cannot start");
    assert!(
        matches!(&err, Error::Invalid(m) if m.starts_with("seat 3 (codex)")),
        "{err:?}"
    );
    assert!(
        rt.list(&w, None).await.unwrap().is_empty(),
        "the two seats already made are gone"
    );
}

/// The judge waits for every seat (409 while one works) and counts against
/// the group's open-thread cap like any linked thread.
#[tokio::test]
async fn synthesis_waits_for_every_seat_and_respects_the_group_cap() {
    use otto_core::agent_runtime::{OpenPeerReq, SynthesizeCouncilReq};
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let g = || seat(RuntimeProvider::Gemini, None);
    let seats = rt
        .create_council(
            &"u1".into(),
            &"w1".into(),
            council_req("hang", vec![g(), g()]),
        )
        .await
        .unwrap();
    let judge = || SynthesizeCouncilReq {
        provider: RuntimeProvider::Gemini,
        model: None,
        effort: None,
    };
    let busy = rt
        .synthesize_council(&seats[0].id, judge())
        .await
        .expect_err("seats are still in their turn");
    assert!(
        matches!(&busy, Error::Conflict(m) if m.contains("is still working")),
        "{busy:?}"
    );
    // Both seats have started answering ("working") before they are stopped,
    // so each keeps a partial answer for the judge.
    for s in &seats {
        host.wait(|e| {
            e.session_id == s.id
                && matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
                    if matches!(&turn_item.kind, TurnItemKind::AssistantMessage { text, .. } if text == "working"))
        })
        .await;
    }
    for s in &seats {
        assert!(rt.interrupt(&s.id).await.unwrap());
        assert!(!rt.latest_reply(&s.id, WAIT).await.unwrap().busy);
    }

    // Fill the group to its cap (seat 1 + 8 linked threads).
    let mut extra = Vec::new();
    for i in 0..7 {
        let p = rt
            .open_peer(
                &seats[0].id,
                OpenPeerReq {
                    provider: RuntimeProvider::Gemini,
                    prompt: None,
                    title: Some(format!("extra {i}")),
                    model: None,
                    effort: None,
                    cwd: None,
                    meta: None,
                },
            )
            .await
            .unwrap();
        extra.push(p);
    }
    // The cap is checked before the answers are read.
    let full = rt
        .synthesize_council(&seats[1].id, judge())
        .await
        .expect_err("the group is full");
    assert!(
        matches!(&full, Error::Conflict(m) if m.contains("open linked threads")),
        "{full:?}"
    );
    rt.close(&extra[0].id).await.unwrap();
    let j = rt.synthesize_council(&seats[1].id, judge()).await.unwrap();
    let verdict = rt.latest_reply(&j.id, WAIT).await.unwrap();
    assert!(
        verdict.text.contains("working"),
        "the interrupted seats' partial answers: {}",
        verdict.text
    );
}

// ---- council rounds: follow-ups, per-round synthesis, restart ----

fn follow_up(
    text: &str,
    seats: &[u32],
) -> otto_agent_protocol::runtime::council::CouncilFollowUpReq {
    otto_agent_protocol::runtime::council::CouncilFollowUpReq {
        text: text.into(),
        seats: seats.to_vec(),
    }
}

fn gemini_judge() -> otto_core::agent_runtime::SynthesizeCouncilReq {
    otto_core::agent_runtime::SynthesizeCouncilReq {
        provider: RuntimeProvider::Gemini,
        model: None,
        effort: None,
    }
}

/// A follow-up goes to every seat (or only the named one) as its own
/// thread's next turn; the judge reads the CURRENT round only, with the
/// council's first question, and is recorded under that round.
#[tokio::test]
async fn council_follow_ups_reach_every_seat_or_one_and_the_judge_reads_the_current_round() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let g = || seat(RuntimeProvider::Gemini, None);
    let seats = rt
        .create_council(
            &"u1".into(),
            &"w1".into(),
            council_req("which cache?", vec![g(), g()]),
        )
        .await
        .unwrap();
    for s in &seats {
        assert!(!rt.latest_reply(&s.id, WAIT).await.unwrap().busy);
    }
    let one = rt.council_rounds(&seats[1].id).await.unwrap();
    assert_eq!(one.max_rounds, 10);
    assert_eq!(one.rounds.len(), 1);
    assert_eq!(
        (
            one.rounds[0].n,
            one.rounds[0].text.as_str(),
            one.rounds[0].seats.clone()
        ),
        (1, "which cache?", vec![1, 2])
    );

    // Round 2 to every seat (no seats named), sent through seat 2.
    let two = rt
        .follow_up_council(&seats[1].id, follow_up("why redis?", &[]))
        .await
        .unwrap();
    assert_eq!(two.rounds.len(), 2);
    assert_eq!(two.rounds[1].seats, vec![1, 2]);
    assert!(two.rounds[1].refused.is_empty());
    for s in &seats {
        let r = rt.latest_reply(&s.id, WAIT).await.unwrap();
        assert!(r.text.contains("why redis?"), "{}", r.text);
    }
    // Round 3 to seat 2 only: seat 1 keeps its round-2 answer.
    let three = rt
        .follow_up_council(&seats[0].id, follow_up("only you", &[2]))
        .await
        .unwrap();
    assert_eq!(three.rounds[2].seats, vec![2]);
    assert!(rt
        .latest_reply(&seats[1].id, WAIT)
        .await
        .unwrap()
        .text
        .contains("only you"));
    assert!(rt
        .latest_reply(&seats[0].id, WAIT)
        .await
        .unwrap()
        .text
        .contains("why redis?"));

    let judge = rt
        .synthesize_council(&seats[0].id, gemini_judge())
        .await
        .unwrap();
    assert_eq!(judge.title, "which cache? · synthesis of round 3");
    let verdict = rt.latest_reply(&judge.id, WAIT).await.unwrap();
    for want in [
        "## The council's first question\n\nwhich cache?",
        "## The question (round 3)\n\nonly you",
        "## Seat 2 — gemini",
    ] {
        assert!(verdict.text.contains(want), "{want}: {}", verdict.text);
    }
    assert!(
        !verdict.text.contains("## Seat 1 —"),
        "seat 1 sat out round 3: {}",
        verdict.text
    );
    let after = rt.council_rounds(&judge.id).await.unwrap();
    assert_eq!(after.rounds[2].syntheses, vec![judge.id.clone()]);
    assert!(after.rounds[1].syntheses.is_empty());

    for bad in [follow_up("x", &[7]), follow_up("   ", &[])] {
        assert!(matches!(
            rt.follow_up_council(&seats[0].id, bad).await,
            Err(Error::Invalid(_))
        ));
    }
    let plain = create(&rt, cwd).await;
    assert!(matches!(
        rt.follow_up_council(&plain.id, follow_up("x", &[])).await,
        Err(Error::Invalid(_))
    ));
}

/// A closed seat is listed as refused for the round and the judge names it
/// as missing; a round no seat takes is not recorded; ten rounds is the cap.
#[tokio::test]
async fn a_round_names_the_seat_it_never_reached_and_stops_at_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let g = || seat(RuntimeProvider::Gemini, None);
    let seats = rt
        .create_council(
            &"u1".into(),
            &"w1".into(),
            council_req("q1", vec![g(), g()]),
        )
        .await
        .unwrap();
    for s in &seats {
        assert!(!rt.latest_reply(&s.id, WAIT).await.unwrap().busy);
    }
    rt.close(&seats[1].id).await.unwrap();
    let two = rt
        .follow_up_council(&seats[0].id, follow_up("q2", &[]))
        .await
        .unwrap();
    let r2 = &two.rounds[1];
    assert_eq!(r2.seats, vec![1, 2]);
    assert_eq!(r2.refused.len(), 1);
    assert_eq!(r2.refused[0].seat, 2);
    assert!(r2.refused[0].reason.contains("closed"), "{:?}", r2.refused);
    assert!(rt
        .latest_reply(&seats[0].id, WAIT)
        .await
        .unwrap()
        .text
        .contains("q2"));
    let judge = rt
        .synthesize_council(&seats[0].id, gemini_judge())
        .await
        .unwrap();
    let verdict = rt.latest_reply(&judge.id, WAIT).await.unwrap();
    assert!(
        verdict
            .text
            .contains("No answer in this round from: seat 2 (the question never reached it"),
        "{}",
        verdict.text
    );

    // Only the closed seat: nothing takes it, nothing is recorded.
    let none = rt
        .follow_up_council(&seats[0].id, follow_up("q3", &[2]))
        .await
        .expect_err("seat 2 is closed");
    assert!(
        matches!(&none, Error::Conflict(m) if m.contains("no seat took the follow-up")),
        "{none:?}"
    );
    assert_eq!(
        rt.council_rounds(&seats[0].id).await.unwrap().rounds.len(),
        2
    );

    // Up to ten rounds (queued behind each other at seat 1), then 409.
    for i in 3..=10 {
        rt.follow_up_council(&seats[0].id, follow_up(&format!("r{i}"), &[1]))
            .await
            .unwrap();
    }
    let cap = rt
        .follow_up_council(&seats[0].id, follow_up("r11", &[1]))
        .await
        .expect_err("the cap");
    assert!(
        matches!(&cap, Error::Conflict(m) if m.contains("10 rounds")),
        "{cap:?}"
    );
    // The queued rounds drain in order; the last one is answered last.
    let deadline = std::time::Instant::now() + WAIT;
    while !rt
        .latest_reply(&seats[0].id, WAIT)
        .await
        .unwrap()
        .text
        .contains("r10")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "round 10 never answered"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The rounds live on seat 1 and survive a daemon restart. What counts as a
/// seat's answer in a round: any assistant text it has on record there,
/// whole or cut off. A restart loses text still streaming (it is never
/// persisted mid-stream), so a seat cut mid-answer has none; a seat that
/// finished before the restart keeps its answer and counts. A round no seat
/// answered is refused (409, nothing opened) and the person asks again.
#[tokio::test]
async fn council_rounds_survive_a_restart_and_a_cut_round_can_be_asked_again() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = TestHost::new(cwd, false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let g = || seat(RuntimeProvider::Gemini, None);
    // Council A: both seats hang on round 2. Council B: the Claude seat
    // answers "hang" at once (its fake echoes it), the Gemini seat hangs.
    let a = rt
        .create_council(
            &"u1".into(),
            &"w1".into(),
            council_req("q1", vec![g(), g()]),
        )
        .await
        .unwrap();
    let b = rt
        .create_council(
            &"u1".into(),
            &"w1".into(),
            council_req("q1", vec![g(), seat(RuntimeProvider::Claude, None)]),
        )
        .await
        .unwrap();
    for s in a.iter().chain(&b) {
        assert!(!rt.latest_reply(&s.id, WAIT).await.unwrap().busy);
    }
    for c in [&a, &b] {
        rt.follow_up_council(&c[0].id, follow_up("hang", &[]))
            .await
            .unwrap();
    }
    for s in [&a[0], &a[1], &b[0]] {
        host.wait(|e| {
            e.session_id == s.id
                && matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
                    if matches!(&turn_item.kind, TurnItemKind::AssistantMessage { text, .. } if text == "working"))
        })
        .await;
    }
    let done = rt.latest_reply(&b[1].id, WAIT).await.unwrap();
    assert!(
        !done.busy && done.text.contains("echo: hang"),
        "{}",
        done.text
    );
    for s in a.iter().chain(&b) {
        rt.flush(&s.id).await;
    }
    rt.shutdown().await;
    drop(rt);

    let host2 = TestHost::new(cwd, false);
    let rt2 = runtime(&pool, &host2, RuntimeConfig::default());
    rt2.recover().await.unwrap();
    let rounds = rt2.council_rounds(&a[1].id).await.unwrap();
    assert_eq!(rounds.rounds.len(), 2);
    assert_eq!(rounds.rounds[1].text, "hang");

    // A: no seat answered round 2 — refused, with the reason, nothing opened.
    let none = rt2
        .synthesize_council(&a[0].id, gemini_judge())
        .await
        .expect_err("round 2 has no answer");
    assert!(
        matches!(&none, Error::Conflict(m) if m == "no seat has answered round 2 yet — nothing to synthesize"),
        "{none:?}"
    );
    assert!(rt2.council_rounds(&a[0].id).await.unwrap().rounds[1]
        .syntheses
        .is_empty());
    // Asked again (a new round): answered, then it synthesizes.
    let three = rt2
        .follow_up_council(&a[0].id, follow_up("again", &[]))
        .await
        .unwrap();
    assert_eq!(three.rounds.len(), 3);
    for s in &a {
        assert!(rt2
            .latest_reply(&s.id, WAIT)
            .await
            .unwrap()
            .text
            .contains("again"));
    }
    let judge = rt2
        .synthesize_council(&a[0].id, gemini_judge())
        .await
        .unwrap();
    assert!(rt2
        .latest_reply(&judge.id, WAIT)
        .await
        .unwrap()
        .text
        .contains("## The question (round 3)"));

    // B: the seat that finished before the restart counts; the cut one is
    // named as missing, never dropped.
    let judge = rt2
        .synthesize_council(&b[0].id, gemini_judge())
        .await
        .unwrap();
    let verdict = rt2.latest_reply(&judge.id, WAIT).await.unwrap();
    for want in [
        "## The question (round 2)",
        "No answer in this round from: seat 1 (",
        "## Seat 2 — claude",
        "echo: hang",
    ] {
        assert!(verdict.text.contains(want), "{want}: {}", verdict.text);
    }
}
