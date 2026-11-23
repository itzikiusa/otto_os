//! What the person picked is what runs; queued messages are sent exactly
//! once or kept. A scripted `/bin/sh` agent stands in for the CLI and
//! records every launch and every prompt the runtime hands its driver.
use std::sync::Mutex as StdMutex;

use otto_core::agent_runtime::{
    BackgroundTask, RuntimeAccessMode, RuntimeProvider, MODEL_SWITCH_REFUSED,
};
use otto_core::domain::Workspace;
use otto_state::DbPool;

use super::*;
use crate::driver::{DriverError, LaunchSpec, McpDelivery, McpServerDef};

#[derive(Default)]
struct Rec {
    /// `LaunchContext::model` of every spawn.
    launches: Vec<Option<String>>,
    /// Every prompt as the driver got it.
    prompts: Vec<RuntimePromptReq>,
}

struct ScriptDriver {
    rec: Arc<StdMutex<Rec>>,
    live_model: bool,
    /// Never answers its handshake.
    silent: bool,
}

const SCRIPT: &str = r#"
echo READY
while IFS= read -r l; do
  case "$l" in
    EXIT) exit 0 ;;
    SLOW) sleep 0.5; echo DONE ;;
    TITLE*) echo "$l"; echo DONE ;;
    *) echo DONE ;;
  esac
done
"#;

impl StructuredDriver for ScriptDriver {
    fn provider(&self) -> RuntimeProvider {
        RuntimeProvider::Claude
    }

    fn launch(&self, ctx: &LaunchContext) -> LaunchSpec {
        self.rec.lock().unwrap().launches.push(ctx.model.clone());
        let script = if self.silent { "sleep 30" } else { SCRIPT };
        LaunchSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            ..LaunchSpec::default()
        }
    }

    fn start(&mut self, _ctx: &LaunchContext) -> DriverStep {
        DriverStep::default()
    }

    fn prompt(
        &mut self,
        _turn_id: &str,
        req: &RuntimePromptReq,
    ) -> std::result::Result<DriverStep, DriverError> {
        self.rec.lock().unwrap().prompts.push(req.clone());
        if req.text == "FAIL" {
            return Err(DriverError("the agent refused this message".into()));
        }
        let mut step = DriverStep::default();
        if req.model.as_deref() == Some("bad") {
            step.push(DriverEvent::Error(ProviderFailure {
                class: "provider_error".into(),
                message: "no such model".into(),
                code: Some(MODEL_SWITCH_REFUSED.into()),
                retryable: None,
            }));
        }
        step.write(req.text.clone());
        Ok(step)
    }

    fn switches_model_live(&self) -> bool {
        self.live_model
    }

    fn interrupt(&mut self) -> DriverStep {
        DriverStep::default()
    }

    fn respond(
        &mut self,
        request_id: &str,
        _decision: ApprovalDecision,
    ) -> std::result::Result<DriverStep, DriverError> {
        Err(DriverError(format!("unknown request {request_id}")))
    }

    fn on_line(&mut self, line: &str) -> DriverStep {
        let mut step = DriverStep::default();
        match line {
            "READY" => step.push(DriverEvent::Ready {
                native_thread_id: Some("native-1".into()),
                model: None,
            }),
            l if l.starts_with("TITLE ") => step.push(DriverEvent::Title(l[6..].into())),
            "DONE" => step.push(DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                failure: None,
                reason: None,
            }),
            _ => {}
        }
        step
    }
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
    fn publish(&self, _s: &RuntimeSession, _event: Arc<RuntimeEvent>) {}
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

struct Fixture {
    rt: AgentRuntime,
    rec: Arc<StdMutex<Rec>>,
    _dir: tempfile::TempDir,
}

async fn fixture(live_model: bool, silent: bool, config: RuntimeConfig) -> (Fixture, Id) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap().to_string();
    let rec = Arc::new(StdMutex::new(Rec::default()));
    let r = Arc::clone(&rec);
    let drivers: Arc<dyn DriverFactory> = Arc::new(move |_| -> Option<Box<dyn StructuredDriver>> {
        Some(Box::new(ScriptDriver {
            rec: Arc::clone(&r),
            live_model,
            silent,
        }))
    });
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool().await),
        Arc::new(Host { root: root.clone() }) as Arc<dyn RuntimeHost>,
        drivers,
        RuntimeConfig {
            attachments_dir: dir.path().join("att"),
            ..config
        },
    );
    let s = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                provider: Some(RuntimeProvider::Claude),
                cwd: Some(root),
                model: Some("opus".into()),
                access_mode: Some(RuntimeAccessMode::ApprovalRequired),
                ..CreateRuntimeSessionReq::default()
            },
        )
        .await
        .unwrap();
    (Fixture { rt, rec, _dir: dir }, s.id)
}

impl Fixture {
    fn live(&self, id: &Id) -> Arc<Live> {
        Arc::clone(self.rt.inner.live.get(id).unwrap().value())
    }

    /// Wait (≤ 10 s) until `pred` holds on the session state; returns the
    /// entry it watched (held, so it is not evicted under the caller).
    async fn until(&self, id: &Id, what: &str, pred: impl Fn(&State) -> bool) -> Arc<Live> {
        let live = self.live(id);
        for _ in 0..200 {
            if pred(&live.lock()) {
                return live;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("timed out waiting for {what}");
    }

    async fn idle(&self, id: &Id) {
        self.until(id, "the turn to end", |st| {
            st.turn.is_none() && st.starting == 0
        })
        .await;
    }

    async fn send(&self, id: &Id, req: RuntimePromptReq) {
        self.rt.prompt_with(id, &req).await.unwrap();
        self.idle(id).await;
    }

    fn notices(&self, id: &Id, field: &str) -> Vec<String> {
        let live = self.live(id);
        let st = live.lock();
        let mut items: Vec<&TurnItem> = st
            .proj
            .items
            .values()
            .filter(|i| i.title.as_deref() == Some(field))
            .collect();
        items.sort_by_key(|i| i.ordinal);
        items
            .iter()
            .filter_map(|i| match &i.kind {
                TurnItemKind::SystemNotice { message, .. } => Some(message.clone()),
                _ => None,
            })
            .collect()
    }

    fn last_turn(&self, id: &Id) -> ProviderTurn {
        let live = self.live(id);
        let st = live.lock();
        st.proj
            .turns
            .values()
            .max_by_key(|t| t.ordinal)
            .cloned()
            .unwrap()
    }
}

fn with_model(text: &str, model: &str) -> RuntimePromptReq {
    RuntimePromptReq {
        model: Some(model.into()),
        ..RuntimePromptReq::of(text)
    }
}

/// F1: "Default model" (`model: ""`) drops the pin in memory and in the
/// DB, and the agent restarts without `--model`.
#[tokio::test]
async fn default_model_clears_the_pin_and_respawns_without_it() {
    let (f, id) = fixture(true, false, RuntimeConfig::default()).await;
    f.send(&id, RuntimePromptReq::of("hi")).await;
    assert_eq!(f.rec.lock().unwrap().launches, [Some("opus".to_string())]);
    f.send(&id, with_model("again", "")).await;
    f.rt.flush(&id).await;
    assert_eq!(f.rt.get(&id).await.unwrap().model, None);
    assert_eq!(f.rt.repo().get(&id).await.unwrap().session.model, None);
    assert_eq!(
        f.rec.lock().unwrap().launches,
        [Some("opus".to_string()), None],
        "a clear restarts the agent on the provider default"
    );
    let sent = f.rec.lock().unwrap().prompts.last().cloned().unwrap();
    assert_eq!(sent.model, None, "the driver never sees an empty model");
    assert_eq!(f.notices(&id, "Model"), ["Model: opus → default"]);
    assert_eq!(f.last_turn(&id).model, None);
    // Unpinned already: another "" changes nothing (no restart loop).
    f.send(&id, with_model("more", "")).await;
    assert_eq!(f.rec.lock().unwrap().launches.len(), 2);
}

/// F2: an accepted effort is the thread's, persisted and re-sent after the
/// agent respawns, and shown on the session and the turn.
#[tokio::test]
async fn effort_is_stored_and_reapplied_after_a_respawn() {
    let (f, id) = fixture(true, false, RuntimeConfig::default()).await;
    let high = RuntimePromptReq {
        effort: Some("high".into()),
        ..RuntimePromptReq::of("think")
    };
    f.send(&id, high).await;
    f.rt.flush(&id).await;
    assert_eq!(f.rt.get(&id).await.unwrap().effort.as_deref(), Some("high"));
    assert_eq!(
        f.rt.repo()
            .get(&id)
            .await
            .unwrap()
            .session
            .effort
            .as_deref(),
        Some("high")
    );
    assert_eq!(f.notices(&id, "Effort"), ["Effort: default → high"]);
    // The agent exits (an idle suspend, a crash); the next plain message
    // respawns it and still carries the effort.
    f.rt.prompt(&id, "EXIT").await.unwrap();
    f.until(&id, "the agent to exit", |st| {
        st.proc.is_none() && st.turn.is_none()
    })
    .await;
    f.send(&id, RuntimePromptReq::of("next")).await;
    assert_eq!(f.rec.lock().unwrap().launches.len(), 2);
    let sent = f.rec.lock().unwrap().prompts.last().cloned().unwrap();
    assert_eq!(sent.effort.as_deref(), Some("high"));
    assert_eq!(f.last_turn(&id).effort.as_deref(), Some("high"));
    // A bad effort at create time is refused before anything is stored.
    assert!(matches!(
        f.rt.set_effort(&id, Some("ludicrous")).await,
        Err(Error::Invalid(_))
    ));
}

/// F4 + F7: a live switch the agent refuses puts the thread back on the
/// model it runs (memory, DB, the turn's record) and the timeline says so.
#[tokio::test]
async fn a_refused_live_switch_rolls_back_and_is_noted() {
    let (f, id) = fixture(true, false, RuntimeConfig::default()).await;
    f.send(&id, with_model("hi", "sonnet")).await;
    assert_eq!(f.notices(&id, "Model"), ["Model: opus → sonnet"]);
    assert_eq!(f.last_turn(&id).model.as_deref(), Some("sonnet"));
    f.send(&id, with_model("try", "bad")).await;
    f.rt.flush(&id).await;
    assert_eq!(
        f.rt.get(&id).await.unwrap().model.as_deref(),
        Some("sonnet")
    );
    assert_eq!(
        f.rt.repo().get(&id).await.unwrap().session.model.as_deref(),
        Some("sonnet")
    );
    assert_eq!(f.last_turn(&id).model.as_deref(), Some("sonnet"));
    assert_eq!(
        f.notices(&id, "Model"),
        [
            "Model: opus → sonnet",
            "Model: sonnet → bad",
            "Model: bad was refused — still on sonnet"
        ]
    );
    assert_eq!(f.rec.lock().unwrap().launches.len(), 1, "switched live");
}

/// H3 + F7: a switch that restarts the agent settles the old process's
/// background work (it would keep the thread "busy" forever) and notes it.
#[tokio::test]
async fn a_restart_switch_settles_the_old_processes_background_work() {
    let (f, id) = fixture(false, false, RuntimeConfig::default()).await;
    f.send(&id, RuntimePromptReq::of("hi")).await;
    f.live(&id).lock().session.background_tasks = vec![BackgroundTask {
        id: "b1".into(),
        kind: "command".into(),
        label: "npm run dev".into(),
        started_at: Utc::now(),
        item_id: None,
        due_at: None,
        stopping: false,
        stop_error: None,
    }];
    let full = RuntimePromptReq {
        access_mode: Some(RuntimeAccessMode::FullAccess),
        ..with_model("go", "sonnet")
    };
    f.send(&id, full).await;
    assert_eq!(f.rec.lock().unwrap().launches.len(), 2, "a restart");
    assert!(f.live(&id).lock().session.background_tasks.is_empty());
    assert_eq!(
        f.notices(&id, "Access"),
        ["Access: approval-required → full-access"]
    );
    f.rt.flush(&id).await;
    assert_eq!(
        f.rt.repo().get(&id).await.unwrap().session.access_mode,
        RuntimeAccessMode::FullAccess
    );
}

/// H2: a queued message whose delivery fails stays queued with its error
/// (shown, skipped by the drain) until a person retries it.
#[tokio::test]
async fn a_queued_message_that_fails_is_kept_with_its_error() {
    let (f, id) = fixture(true, false, RuntimeConfig::default()).await;
    f.rt.prompt(&id, "SLOW").await.unwrap();
    let fail = RuntimePromptReq {
        delivery: Some(PromptDelivery::Queue),
        ..RuntimePromptReq::of("FAIL")
    };
    let qid =
        f.rt.prompt_with(&id, &fail)
            .await
            .unwrap()
            .queued_id
            .unwrap();
    f.until(&id, "the failed send", |st| {
        st.turn.is_none() && st.proj.queue.iter().any(|q| q.error.is_some())
    })
    .await;
    {
        let live = f.live(&id);
        let st = live.lock();
        assert_eq!(st.proj.queue.len(), 1);
        assert_eq!(st.proj.queue[0].id, qid);
        assert_eq!(st.proj.queue[0].req.text, "FAIL");
        assert!(st.proj.snapshot().queued_prompts[0].error.is_some());
    }
    // Persisted with its error: a restart keeps it.
    f.rt.flush(&id).await;
    let rows = f.rt.repo().entities(&id).await.unwrap();
    let row = rows.iter().find(|r| r.kind == K_QUEUE).unwrap();
    assert!(row.payload[0]["error"].is_string());
    // Edit hands it back exactly once.
    let back = f.rt.unqueue_prompt(&id, &qid).await.unwrap().unwrap();
    assert_eq!(back.text, "FAIL");
    assert!(f.rt.unqueue_prompt(&id, &qid).await.unwrap().is_none());
}

/// H1: Edit / Remove waits for a delivery in flight (the prompt lock), so a
/// message being sent cannot also come back into the composer.
#[tokio::test]
async fn unqueue_waits_for_a_delivery_in_flight() {
    let (f, id) = fixture(true, false, RuntimeConfig::default()).await;
    let live = f.live(&id);
    let held = live.prompt_lock.lock().await;
    let rt = f.rt.clone();
    let id2 = id.clone();
    let edit = tokio::spawn(async move { rt.unqueue_prompt(&id2, "nope").await });
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!edit.is_finished(), "unqueue must wait for the prompt lock");
    drop(held);
    assert!(edit.await.unwrap().unwrap().is_none());
}

/// H8: the queue is capped in count with a clear 4xx.
#[tokio::test]
async fn the_queue_is_capped() {
    let (f, id) = fixture(true, false, RuntimeConfig::default()).await;
    // Keep a turn "running" while we queue.
    f.live(&id).lock().turn = Some(ActiveTurn {
        id: "t-hold".into(),
        origin: TurnOrigin::User,
        interrupting: false,
        stalled: false,
    });
    let queued = RuntimePromptReq {
        delivery: Some(PromptDelivery::Queue),
        ..RuntimePromptReq::of("later")
    };
    for _ in 0..MAX_QUEUED_PROMPTS {
        f.rt.prompt_with(&id, &queued).await.unwrap();
    }
    assert!(matches!(
        f.rt.prompt_with(&id, &queued).await,
        Err(Error::Conflict(_))
    ));
    f.live(&id).lock().turn = None;
}

/// H4: a CLI that never answers its handshake is stopped with a clear error
/// instead of "Starting" forever.
#[tokio::test]
async fn a_silent_handshake_times_out_with_a_clear_error() {
    let config = RuntimeConfig {
        handshake_timeout: Duration::from_millis(300),
        ..RuntimeConfig::default()
    };
    let (f, id) = fixture(true, true, config).await;
    let live = f
        .until(&id, "the handshake timeout", |st| st.proc.is_none())
        .await;
    let st = live.lock();
    let p = st.proj.session.as_ref().unwrap();
    assert_eq!(p.status, ProviderSessionStatus::Stopped);
    let why = p.last_error.as_deref().unwrap();
    assert!(
        why.contains("did not answer within") && why.contains("signed in"),
        "{why}"
    );
}

/// C4: fresh context with only background work left is refused with a 409
/// that names `stop_background`; with it, the work is stopped and settled
/// and the fresh conversation starts.
#[tokio::test]
async fn fresh_context_can_stop_leftover_background_work() {
    let (f, id) = fixture(true, false, RuntimeConfig::default()).await;
    f.send(&id, RuntimePromptReq::of("hi")).await;
    f.live(&id).lock().session.background_tasks = vec![BackgroundTask {
        id: "b1".into(),
        kind: "command".into(),
        label: "npm run dev".into(),
        started_at: Utc::now(),
        item_id: None,
        due_at: None,
        stopping: false,
        stop_error: None,
    }];
    let fresh = RuntimePromptReq {
        fresh_context: true,
        ..RuntimePromptReq::of("start over")
    };
    match f.rt.prompt_with(&id, &fresh).await {
        Err(Error::Conflict(m)) => assert!(m.contains("stop_background"), "{m}"),
        other => panic!("expected a 409, got ok={}", other.is_ok()),
    }
    assert_eq!(f.rec.lock().unwrap().launches.len(), 1, "nothing replaced");
    let stop = RuntimePromptReq {
        stop_background: true,
        ..fresh
    };
    f.send(&id, stop).await;
    assert!(f.live(&id).lock().session.background_tasks.is_empty());
    assert_eq!(f.rec.lock().unwrap().launches.len(), 2, "a fresh agent");
    let live = f.live(&id);
    let st = live.lock();
    assert!(st
        .proj
        .items
        .values()
        .any(|i| matches!(i.kind, TurnItemKind::SessionBoundary { .. })));
}

/// A Claude thread whose switch to Codex never comes up (its handshake
/// never answers; `start_wait` gives up after 600 ms).
async fn silent_codex() -> (Fixture, Id) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_str().unwrap().to_string();
    let rec = Arc::new(StdMutex::new(Rec::default()));
    let r = Arc::clone(&rec);
    // Codex never answers its handshake.
    let drivers: Arc<dyn DriverFactory> = Arc::new(move |p| -> Option<Box<dyn StructuredDriver>> {
        Some(Box::new(ScriptDriver {
            rec: Arc::clone(&r),
            live_model: false,
            silent: p == RuntimeProvider::Codex,
        }))
    });
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool().await),
        Arc::new(Host { root: root.clone() }) as Arc<dyn RuntimeHost>,
        drivers,
        RuntimeConfig {
            attachments_dir: dir.path().join("att"),
            start_wait: Duration::from_millis(600),
            ..RuntimeConfig::default()
        },
    );
    let s = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                provider: Some(RuntimeProvider::Claude),
                cwd: Some(root),
                ..CreateRuntimeSessionReq::default()
            },
        )
        .await
        .unwrap();
    let f = Fixture { rt, rec, _dir: dir };
    (f, s.id)
}

/// Review 1: a switch whose request is dropped mid-handshake (a page reload)
/// still ends: the agent never comes up, so the thread is exactly as before.
#[tokio::test]
async fn a_switch_whose_request_is_dropped_still_ends() {
    let (f, id) = silent_codex().await;
    let s = f.rt.get(&id).await.unwrap();
    f.send(&s.id, RuntimePromptReq::of("hi")).await;
    seed_commands(&f, &s.id);
    let switch = RuntimePromptReq {
        provider: Some("codex".into()),
        ..RuntimePromptReq::of("go")
    };
    let dropped = tokio::time::timeout(
        Duration::from_millis(150),
        f.rt.prompt_to_end(&s.id, &switch),
    );
    assert!(
        dropped.await.is_err(),
        "the request is dropped mid-handshake"
    );
    let live = f
        .until(&s.id, "the switch to end", |st| {
            !st.fresh_starting && st.starting == 0
        })
        .await;
    {
        let st = live.lock();
        assert_eq!(st.session.provider, RuntimeProvider::Claude);
        assert_eq!(st.session.native_thread_id.as_deref(), Some("native-1"));
        assert!(st.handoff.is_none());
        assert_eq!(
            st.proj.commands.len(),
            1,
            "a failed switch keeps the old commands"
        );
        assert!(!st
            .proj
            .items
            .values()
            .any(|i| matches!(i.kind, TurnItemKind::SessionBoundary { .. })));
    }
    f.rt.flush(&s.id).await;
    let stored = f.rt.repo().get(&s.id).await.unwrap().session;
    assert_eq!(stored.provider, RuntimeProvider::Claude);
}

/// A subagent of `of`, opened by its agent.
async fn subagent(f: &Fixture, of: &Id) -> Id {
    let cwd = f.rt.get(of).await.unwrap().cwd;
    f.rt.create(
        &"u1".into(),
        &"w1".into(),
        CreateRuntimeSessionReq {
            provider: Some(RuntimeProvider::Claude),
            cwd: Some(cwd),
            meta: Some(children::child_meta(
                of,
                otto_core::agent_runtime::ChildOpener::Agent,
            )),
            ..CreateRuntimeSessionReq::default()
        },
    )
    .await
    .unwrap()
    .id
}

/// Review 2: Stop pressed in a subagent's own thread quiets only what it
/// delegated to; its own "was stopped" reply still wakes its lead.
#[tokio::test]
async fn stop_on_a_subagent_still_tells_its_lead() {
    let (f, lead) = fixture(false, false, RuntimeConfig::default()).await;
    let sub = subagent(&f, &lead).await;
    let grand = subagent(&f, &sub).await;
    f.rt.stop_family(&sub, false).await.unwrap();
    assert!(
        !family::recently_stopped(&sub),
        "its reply to the lead is kept"
    );
    assert!(family::recently_stopped(&grand));
    assert!(!family::recently_stopped(&lead));
}

/// Review 6: a family stop drops the replies queued for every member, not
/// only for the thread Stop was pressed on.
#[tokio::test]
async fn a_family_stop_drops_every_members_queued_replies() {
    let (f, lead) = fixture(false, false, RuntimeConfig::default()).await;
    let a = subagent(&f, &lead).await;
    let b = subagent(&f, &a).await;
    for (on, from) in [(&lead, &a), (&a, &b)] {
        f.live(on).lock().proj.queue.push(QueuedRecord {
            id: new_id(),
            created_at: Utc::now(),
            req: Arc::new(RuntimePromptReq::of("a reply")),
            wake: Some(children::WakeKey {
                child: from.clone(),
                turn: "t".into(),
            }),
            error: None,
        });
    }
    f.rt.stop_family(&lead, false).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    for id in [&lead, &a] {
        let live = f.live(id);
        let st = live.lock();
        assert!(st.proj.queue.is_empty(), "{id}");
        assert!(st.turn.is_none(), "{id} stays stopped");
    }
}

fn seed_commands(f: &Fixture, id: &Id) {
    f.live(id).lock().proj.commands = vec![otto_core::agent_runtime::ProviderCommand {
        name: "old-skill".into(),
        description: String::new(),
        hint: None,
    }];
}

/// Review 10: after a switch the old agent's slash commands are gone, even
/// when the new agent lists none of its own.
#[tokio::test]
async fn a_switch_drops_the_old_agents_commands() {
    let (f, id) = fixture(false, false, RuntimeConfig::default()).await;
    f.send(&id, RuntimePromptReq::of("hi")).await;
    seed_commands(&f, &id);
    let switch = RuntimePromptReq {
        provider: Some("codex".into()),
        ..RuntimePromptReq::of("go")
    };
    f.send(&id, switch).await;
    let live = f.live(&id);
    let st = live.lock();
    assert_eq!(st.session.provider, RuntimeProvider::Codex);
    assert!(st.proj.commands.is_empty(), "{:?}", st.proj.commands);
}

/// Review 11: a pin made while a switch starts is kept when the switch fails
/// (only the provider's own meta key is put back).
#[tokio::test]
async fn a_failed_switch_keeps_meta_patched_meanwhile() {
    let (f, id) = silent_codex().await;
    f.send(&id, RuntimePromptReq::of("hi")).await;
    let rt = f.rt.clone();
    let sid = id.clone();
    let switch = tokio::spawn(async move {
        let req = RuntimePromptReq {
            provider: Some("codex".into()),
            ..RuntimePromptReq::of("go")
        };
        rt.prompt_to_end(&sid, &req).await
    });
    f.until(&id, "the switch to start", |st| st.fresh_starting)
        .await;
    let pin = serde_json::json!({"thread_pinned": true});
    f.rt.update_thread_meta(&id, pin.as_object().unwrap().clone())
        .await
        .unwrap();
    assert!(switch.await.unwrap().is_err(), "codex never comes up");
    let s = f.rt.get(&id).await.unwrap();
    assert_eq!(s.provider, RuntimeProvider::Claude);
    let meta = s.meta.unwrap_or_default();
    assert_eq!(
        meta.get("thread_pinned"),
        Some(&serde_json::json!(true)),
        "{meta}"
    );
}

/// An agent that names the conversation after its first prompt (Codex does,
/// verbatim) names it after Otto's preamble: that is never the title.
#[tokio::test]
async fn otto_preamble_never_becomes_the_title() {
    let (f, id) = fixture(false, false, RuntimeConfig::default()).await;
    {
        let live = f.live(&id);
        let mut st = live.lock();
        st.session.title = "fix the login bug".into();
        st.session.title_auto = true;
    }
    for named in [
        "TITLE <otto_instructions> ## Otto subagents Ordinary tasks need none",
        "TITLE [Context handoff from Claude Code] You continue a thread",
        "TITLE [Subagent task from thread \"x\"]",
    ] {
        f.send(&id, RuntimePromptReq::of(named)).await;
        assert_eq!(
            f.rt.get(&id).await.unwrap().title,
            "fix the login bug",
            "{named}"
        );
    }
    f.send(&id, RuntimePromptReq::of("TITLE Login bug fix"))
        .await;
    assert_eq!(f.rt.get(&id).await.unwrap().title, "Login bug fix");
}
