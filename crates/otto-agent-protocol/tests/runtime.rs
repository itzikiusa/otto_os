//! End to end: the real `AgentRuntime` + the real drivers over stdio — the
//! ACP driver against `otto-fake-agent`, the Codex driver against
//! `otto-fake-codex` (a fake `codex app-server`) — with a real (in-memory)
//! SQLite store.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use otto_agent_protocol::acp::AcpDriver;
use otto_agent_protocol::codex::CodexDriver;
use otto_agent_protocol::ring::RingLimits;
use otto_agent_protocol::{
    AgentRuntime, DriverFactory, LaunchSpec, RuntimeConfig, RuntimeHost, StructuredDriver,
};
use otto_core::agent_runtime::{
    ApprovalDecision, CreateRuntimeSessionReq, ProviderSessionStatus, ProviderTurnStatus,
    RuntimeEvent, RuntimeEventBody, RuntimeProvider, RuntimeReplay, RuntimeRequestStatus,
    RuntimeSession, RuntimeSessionStatus, TurnItem, TurnItemKind, TurnItemStatus,
    TurnTerminalStatus,
};
use otto_core::domain::Workspace;
use otto_core::{Error, Id, Result};
use otto_state::{AgentRuntimeRepo, DbPool};

const FAKE: &str = env!("CARGO_BIN_EXE_otto-fake-agent");
const FAKE_CODEX: &str = env!("CARGO_BIN_EXE_otto-fake-codex");
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
    ) -> Result<()> {
        if self.refuse {
            return Err(Error::Invalid(
                "refusing to start an agent session here".into(),
            ));
        }
        // Prove the runtime spawns the PREPARED spec (the safety pipeline's).
        spec.env.push(("OTTO_PREPARED".into(), "1".into()));
        self.prepared.lock().unwrap().push(spec.clone());
        Ok(())
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
            RuntimeProvider::Codex => Box::new(CodexDriver::with_program(FAKE_CODEX)),
            _ => Box::new(AcpDriver::new(p, FAKE, Vec::new(), Vec::new())),
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
    // `streaming: true`, then settled.
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
        vec![
            ("echo: ".into(), true),
            ("echo: hello".into(), true),
            ("echo: hello".into(), false)
        ]
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
    rt.shutdown();
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
    assert_eq!(spec.args, ["app-server"]);

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
    rt.shutdown();

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
