//! End to end: the real `AgentRuntime` + the real drivers over stdio — the
//! ACP driver against `otto-fake-agent`, the Codex driver against
//! `otto-fake-codex` (a fake `codex app-server`), the agy driver against
//! `otto-fake-agy` — with a real (in-memory) SQLite store.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use otto_agent_protocol::acp::AcpDriver;
use otto_agent_protocol::agy::AgyDriver;
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
        _s: &RuntimeSession,
        spec: &mut LaunchSpec,
        _mcp: McpDelivery,
    ) -> Result<Vec<McpServerDef>> {
        if self.refuse {
            return Err(Error::Invalid(
                "refusing to start an agent session here".into(),
            ));
        }
        // Prove the runtime spawns the PREPARED spec (the safety pipeline's).
        spec.env.push(("OTTO_PREPARED".into(), "1".into()));
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
            RuntimeProvider::Agy => Box::new(AgyDriver::with_program(&fake_bin!("otto-fake-agy"))),
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

// ---- Antigravity (fake agy) ------------------------------------------

#[tokio::test]
async fn agy_streams_reports_denials_and_resumes_after_an_interrupt() {
    let dir = tempfile::tempdir().unwrap();
    let pool = pool().await;
    let host = TestHost::new(dir.path().to_str().unwrap(), false);
    let rt = runtime(&pool, &host, RuntimeConfig::default());
    let s = create_as(&rt, dir.path().to_str().unwrap(), RuntimeProvider::Agy).await;

    rt.prompt(&s.id, "hello").await.unwrap();
    let fin = host.wait(is_terminal).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Completed));
    assert!(host.items().iter().any(|i| matches!(&i.kind,
        TurnItemKind::AssistantMessage { text, streaming: false, .. } if text == "echo: hello")));
    assert!(host.all().iter().any(|e| matches!(&e.body,
        RuntimeEventBody::ProviderTurnUpdated { provider_turn }
            if provider_turn.turn_token_usage.as_ref().is_some_and(|u| u.input_tokens == 5))));
    let native = rt.get(&s.id).await.unwrap().native_thread_id.unwrap();
    assert!(native.starts_with("fake-conv-"), "{native}");

    // headless agy denies what needs approval: the tool + a notice, no request
    rt.prompt(&s.id, "deny").await.unwrap();
    let fin = host.wait_n(is_terminal, 2).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Completed));
    let items = host.items();
    assert!(items.iter().any(|i| matches!(&i.kind,
        TurnItemKind::CommandExecution { input, .. } if input == "echo hi > x.txt")));
    assert!(items.iter().any(|i| matches!(&i.kind,
        TurnItemKind::SystemNotice { message } if message.contains("RunCommand"))));
    assert!(!host.all().iter().any(is_type("runtime_request.updated")));

    rt.prompt(&s.id, "fail").await.unwrap();
    let fin = host.wait_n(is_terminal, 3).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Failed));

    // SIGINT ends the turn interrupted and agy exits; the next prompt
    // respawns it on the same conversation.
    rt.prompt(&s.id, "hang").await.unwrap();
    host.wait(|e| {
        matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
        if matches!(&turn_item.kind, TurnItemKind::AssistantMessage { text, .. } if text == "1\n"))
    })
    .await;
    assert!(rt.interrupt(&s.id).await.unwrap());
    let fin = host.wait_n(is_terminal, 4).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Interrupted));
    host.wait(|e| {
        matches!(&e.body, RuntimeEventBody::ProviderSessionUpdated { provider_session }
        if provider_session.status == ProviderSessionStatus::Stopped)
    })
    .await;
    rt.prompt(&s.id, "again").await.unwrap();
    let fin = host.wait_n(is_terminal, 5).await;
    assert_eq!(terminal(&fin), Some(TurnTerminalStatus::Completed));
    let respawn = host.prepared.lock().unwrap()[1].clone();
    assert!(respawn
        .args
        .windows(2)
        .any(|w| w == ["--conversation", native.as_str()]));
    assert_eq!(rt.get(&s.id).await.unwrap().native_thread_id, Some(native));
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

    // Created in a mode: agy launches with its `--mode`.
    let agy = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                provider: Some(RuntimeProvider::Agy),
                cwd: Some(dir.path().to_str().unwrap().into()),
                access_mode: Some(RuntimeAccessMode::AutoAcceptEdits),
                ..CreateRuntimeSessionReq::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(agy.access_mode, RuntimeAccessMode::AutoAcceptEdits);
    let spawn = host.prepared.lock().unwrap().last().unwrap().clone();
    assert!(spawn
        .args
        .windows(2)
        .any(|w| w == ["--mode", "accept-edits"]));
    rt.close(&agy.id).await.unwrap();
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
