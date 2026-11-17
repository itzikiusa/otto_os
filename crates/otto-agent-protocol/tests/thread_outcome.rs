//! Several threads working together, end to end over the fake CLIs: the
//! row's `last_turn_status` / `last_error_class` (live and after a restart),
//! the waiting notice once per episode, and the agent-to-agent loop guard.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use otto_agent_protocol::acp::AcpDriver;
use otto_agent_protocol::codex::CodexDriver;
use otto_agent_protocol::runtime::{AGENT_HOPS_MAX, PAUSED_ERROR, PAUSED_NOTICE, PAUSED_QUEUED};
use otto_agent_protocol::{
    AgentRuntime, DriverFactory, LaunchSpec, McpDelivery, McpServerDef, RuntimeConfig, RuntimeHost,
    StructuredDriver,
};
use otto_core::agent_runtime::{
    ApprovalDecision, CreateRuntimeSessionReq, OpenPeerReq, RuntimeEvent, RuntimeEventBody,
    RuntimeProvider, RuntimeRequestStatus, RuntimeSession, TurnItemKind, TurnOutcome,
};
use otto_core::domain::Workspace;
use otto_core::{Error, Id, Result};
use otto_state::{AgentRuntimeRepo, DbPool};

macro_rules! fake_bin {
    ($name:literal) => {
        std::env::var(concat!("NEXTEST_BIN_EXE_", $name))
            .unwrap_or_else(|_| env!(concat!("CARGO_BIN_EXE_", $name)).to_string())
    };
}
const WAIT: Duration = Duration::from_secs(15);

struct Host {
    root: String,
    events: Mutex<Vec<Arc<RuntimeEvent>>>,
    /// Every `attention` call: (session, what).
    attention: Mutex<Vec<(Id, Option<String>)>>,
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
        _spec: &mut LaunchSpec,
        _mcp: McpDelivery,
    ) -> Result<Vec<McpServerDef>> {
        Ok(Vec::new())
    }
    fn publish(&self, _s: &RuntimeSession, event: Arc<RuntimeEvent>) {
        self.events.lock().unwrap().push(event);
        self.notify.notify_waiters();
    }
    fn attention(&self, s: &RuntimeSession, what: Option<&str>) {
        self.attention
            .lock()
            .unwrap()
            .push((s.id.clone(), what.map(str::to_string)));
    }
}

impl Host {
    fn new(root: &str) -> Arc<Self> {
        Arc::new(Self {
            root: root.into(),
            events: Mutex::default(),
            attention: Mutex::default(),
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

    fn calls(&self, id: &Id) -> Vec<Option<String>> {
        self.attention
            .lock()
            .unwrap()
            .iter()
            .filter(|(s, _)| s == id)
            .map(|(_, w)| w.clone())
            .collect()
    }

    fn pending_requests(&self, id: &Id) -> Vec<Id> {
        let mut latest = std::collections::BTreeMap::new();
        for e in self.all().into_iter().filter(|e| &e.session_id == id) {
            if let RuntimeEventBody::RuntimeRequestUpdated { runtime_request } = e.body {
                latest.insert(runtime_request.id.clone(), runtime_request.status);
            }
        }
        latest
            .into_iter()
            .filter(|(_, s)| *s == RuntimeRequestStatus::Pending)
            .map(|(id, _)| id)
            .collect()
    }
}

fn terminal_of(id: &Id) -> impl Fn(&RuntimeEvent) -> bool + '_ {
    move |e| &e.session_id == id && matches!(e.body, RuntimeEventBody::TurnTerminal { .. })
}

fn pending_of(id: &Id) -> impl Fn(&RuntimeEvent) -> bool + '_ {
    move |e| {
        &e.session_id == id
            && matches!(&e.body, RuntimeEventBody::RuntimeRequestUpdated { runtime_request }
                if runtime_request.status == RuntimeRequestStatus::Pending)
    }
}

fn drivers() -> Arc<dyn DriverFactory> {
    Arc::new(|p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
        Some(match p {
            RuntimeProvider::Codex => {
                Box::new(CodexDriver::with_program(fake_bin!("otto-fake-codex")))
            }
            _ => Box::new(AcpDriver::gemini(&fake_bin!("otto-fake-agent"))),
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

fn runtime(pool: &DbPool, host: &Arc<Host>) -> AgentRuntime {
    AgentRuntime::new(
        AgentRuntimeRepo::new(pool.clone()),
        Arc::clone(host) as Arc<dyn RuntimeHost>,
        drivers(),
        RuntimeConfig {
            attachments_dir: std::path::Path::new(&host.root).join("attachments"),
            ..RuntimeConfig::default()
        },
    )
}

async fn create(
    rt: &AgentRuntime,
    cwd: &str,
    provider: RuntimeProvider,
    meta: Option<serde_json::Value>,
) -> RuntimeSession {
    rt.create(
        &"u1".into(),
        &"w1".into(),
        CreateRuntimeSessionReq {
            title_auto: false,
            provider: Some(provider),
            cwd: Some(cwd.into()),
            title: Some("T".into()),
            model: None,
            access_mode: None,
            meta,
        },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn the_row_says_how_the_last_turn_ended_live_and_after_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = Host::new(cwd);
    let rt = runtime(&pool, &host);
    let s = create(&rt, cwd, RuntimeProvider::Codex, None).await;
    let row = |s: &RuntimeSession| (s.last_turn_status, s.last_error_class.clone());
    assert_eq!(
        row(&rt.get(&s.id).await.unwrap()),
        (None, None),
        "no turn yet"
    );

    rt.prompt(&s.id, "hello").await.unwrap();
    host.wait_n(terminal_of(&s.id), 1).await;
    assert_eq!(
        row(&rt.get(&s.id).await.unwrap()),
        (Some(TurnOutcome::Completed), None)
    );

    // A structured usage-limit failure (Codex `codexErrorInfo`).
    rt.prompt(&s.id, "limit").await.unwrap();
    host.wait_n(terminal_of(&s.id), 2).await;
    let failed = rt.get(&s.id).await.unwrap();
    assert_eq!(
        row(&failed),
        (Some(TurnOutcome::Failed), Some("usage_limit".into()))
    );
    let listed = rt.list(&"w1".into(), None).await.unwrap();
    assert_eq!(
        row(listed.iter().find(|r| r.id == s.id).unwrap()),
        row(&failed),
        "the list carries it too"
    );
    let wire = serde_json::to_value(&failed).unwrap();
    assert_eq!(wire["last_turn_status"], "failed");
    assert_eq!(wire["last_error_class"], "usage_limit");

    // While the next turn runs the row shows the attempt under way…
    rt.prompt(&s.id, "hang").await.unwrap();
    assert_eq!(row(&rt.get(&s.id).await.unwrap()), (None, None));
    // …and a Stop ends it `interrupted`.
    rt.interrupt(&s.id).await.unwrap();
    host.wait_n(terminal_of(&s.id), 3).await;
    assert_eq!(
        row(&rt.get(&s.id).await.unwrap()),
        (Some(TurnOutcome::Interrupted), None)
    );

    // A second thread whose last turn failed, then a cold daemon: the store
    // derives both rows from the persisted latest turn (and its error item).
    let f = create(&rt, cwd, RuntimeProvider::Codex, None).await;
    rt.prompt(&f.id, "limit").await.unwrap();
    host.wait_n(terminal_of(&f.id), 1).await;
    rt.flush(&s.id).await;
    rt.flush(&f.id).await;
    let cold = runtime(&pool, &Host::new(cwd));
    assert_eq!(
        row(&cold.get(&s.id).await.unwrap()),
        (Some(TurnOutcome::Interrupted), None)
    );
    assert_eq!(
        row(&cold.get(&f.id).await.unwrap()),
        (Some(TurnOutcome::Failed), Some("usage_limit".into()))
    );
    let cold_list = cold.list(&"w1".into(), None).await.unwrap();
    assert_eq!(
        row(cold_list.iter().find(|r| r.id == f.id).unwrap()),
        (Some(TurnOutcome::Failed), Some("usage_limit".into()))
    );
    rt.close(&s.id).await.unwrap();
    rt.close(&f.id).await.unwrap();
}

#[tokio::test]
async fn a_waiting_thread_signals_once_per_episode_and_settles_when_answered() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = Host::new(cwd);
    let rt = runtime(&pool, &host);
    let s = create(&rt, cwd, RuntimeProvider::Gemini, None).await;

    // Two approvals at once: ONE episode.
    rt.prompt(&s.id, "ask2").await.unwrap();
    host.wait_n(pending_of(&s.id), 2).await;
    let calls = host.calls(&s.id);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(
        calls[0].as_deref().unwrap().starts_with("wants approval"),
        "{calls:?}"
    );
    let pending = host.pending_requests(&s.id);
    assert_eq!(pending.len(), 2);
    rt.respond(&s.id, &pending[0], ApprovalDecision::Accept)
        .await
        .unwrap();
    assert_eq!(host.calls(&s.id).len(), 1, "the second still waits");
    rt.respond(&s.id, &pending[1], ApprovalDecision::Accept)
        .await
        .unwrap();
    host.wait_n(terminal_of(&s.id), 1).await;
    assert_eq!(host.calls(&s.id)[1], None, "answered: settled");

    // The next approval is a new episode.
    rt.prompt(&s.id, "ask").await.unwrap();
    host.wait_n(pending_of(&s.id), 3).await;
    assert_eq!(host.calls(&s.id).len(), 3);
    assert!(host.calls(&s.id)[2].is_some());
    rt.close(&s.id).await.unwrap();
    assert_eq!(host.calls(&s.id).len(), 4, "closing ends the episode");
    assert_eq!(host.calls(&s.id)[3], None);

    // A background thread (a workflow step) waits unattended: no notice.
    let bg = create(
        &rt,
        cwd,
        RuntimeProvider::Gemini,
        Some(serde_json::json!({"source": "workflow"})),
    )
    .await;
    rt.prompt(&bg.id, "ask").await.unwrap();
    host.wait_n(pending_of(&bg.id), 1).await;
    assert!(host.calls(&bg.id).is_empty());
    rt.close(&bg.id).await.unwrap();
}

/// Wait until thread `id` has no turn running.
async fn settle(rt: &AgentRuntime, id: &Id) {
    let r = rt.latest_reply(id, WAIT).await.unwrap();
    assert!(!r.busy, "{id} still busy");
}

#[tokio::test]
async fn agent_messages_pause_a_linked_group_and_a_person_resumes_it() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    let pool = pool().await;
    let host = Host::new(cwd);
    let rt = runtime(&pool, &host);
    let lead = create(&rt, cwd, RuntimeProvider::Gemini, None).await;
    let open = |prompt: Option<&str>| OpenPeerReq {
        provider: RuntimeProvider::Gemini,
        prompt: prompt.map(str::to_string),
        title: None,
        model: None,
        effort: None,
        cwd: None,
        meta: None,
    };
    // Hop 1: an agent's opening prompt; hop 2: the child's reply to it,
    // queued for (and sent to) the lead's agent.
    let asked = rt.open_peer(&lead.id, open(Some("hi"))).await.unwrap();
    host.wait_n(terminal_of(&asked.id), 1).await;
    host.wait_n(terminal_of(&lead.id), 1).await;
    // A plain linked peer (no `child_of`: its replies wake nobody), opened
    // without a prompt: not a message.
    let other = rt
        .open_peer(
            &lead.id,
            OpenPeerReq {
                meta: Some(serde_json::json!({"note": "plain peer"})),
                ..open(None)
            },
        )
        .await
        .unwrap();
    // Hops 3..=15.
    for _ in 2..AGENT_HOPS_MAX - 1 {
        rt.message_peer(&lead.id, &other.id, "ping").await.unwrap();
        settle(&rt, &other.id).await;
    }
    // Hop 16: the lead's agent asks the child again. Its reply is hop 17:
    // held in the lead's queue with the pause as its error, not sent.
    rt.message_peer(&lead.id, &asked.id, "once more")
        .await
        .unwrap();
    host.wait_n(
        |e| {
            e.session_id == lead.id
                && matches!(&e.body, RuntimeEventBody::QueueUpdated { queued_prompts }
                    if queued_prompts.iter().any(|q| q.error.as_deref() == Some(PAUSED_QUEUED)))
        },
        1,
    )
    .await;
    // Every further agent message is refused, with one notice in the lead.
    for _ in 0..2 {
        let err = rt
            .message_peer(&lead.id, &other.id, "ping")
            .await
            .unwrap_err();
        assert!(
            matches!(&err, Error::Conflict(m) if m == PAUSED_ERROR),
            "{err:?}"
        );
    }
    assert!(matches!(
        rt.open_peer(&other.id, open(Some("go"))).await,
        Err(Error::Conflict(_))
    ));
    let notices = host
        .all()
        .into_iter()
        .filter(|e| e.session_id == lead.id)
        .filter(|e| {
            matches!(&e.body, RuntimeEventBody::TurnItemUpdated { turn_item }
            if matches!(&turn_item.kind, TurnItemKind::SystemNotice { message, .. }
                if message == PAUSED_NOTICE))
        })
        .count();
    assert_eq!(notices, 1, "one notice per pause");

    // A person's message in any member resumes the group.
    rt.person_prompted(&other.id).await;
    settle(&rt, &other.id).await;
    rt.message_peer(&lead.id, &other.id, "ping").await.unwrap();
    settle(&rt, &other.id).await;
    // A person's own `…/message` never counts and resumes too.
    rt.message_peer_by(&lead.id, &other.id, "from me", true)
        .await
        .unwrap();
    for s in [&lead, &asked, &other] {
        rt.close(&s.id).await.unwrap();
    }
}
