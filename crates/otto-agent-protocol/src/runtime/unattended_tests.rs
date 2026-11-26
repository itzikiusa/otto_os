//! The settle rule's look at a step's family (DR-016): what a sub-thread
//! below the step does counts as the step's own, a closed one included.

use otto_core::agent_runtime::{CreateRuntimeSessionReq, OpenPeerReq, RuntimeProvider};
use otto_core::domain::Workspace;
use otto_state::{AgentRuntimeRepo, DbPool};

use super::*;
use crate::driver::{LaunchSpec, McpDelivery, McpServerDef};

struct Host(String);

#[async_trait::async_trait]
impl RuntimeHost for Host {
    async fn workspace(&self, id: &Id) -> Result<Workspace> {
        Ok(Workspace {
            id: id.clone(),
            name: "W".into(),
            root_path: self.0.clone(),
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
    for q in [
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) \
         VALUES ('u1', 'u', 'x', 'U', 0, ?)",
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('w1', 'W', '/tmp', ?)",
    ] {
        sqlx::query(q).bind(&now).execute(&pool).await.unwrap();
    }
    pool.into()
}

fn open() -> OpenPeerReq {
    OpenPeerReq {
        provider: RuntimeProvider::Claude,
        prompt: None,
        title: None,
        model: None,
        effort: None,
        cwd: None,
        meta: None,
    }
}

/// A step's thread and one sub-thread its agent opened, on the fakes.
async fn family(dir: &tempfile::TempDir) -> (AgentRuntime, Id, Id) {
    let root = dir.path().display().to_string();
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool().await),
        Arc::new(Host(root.clone())),
        crate::scripted::drivers(),
        RuntimeConfig {
            attachments_dir: dir.path().join("attachments"),
            transport: Some(crate::scripted::transport(Vec::new())),
            ..RuntimeConfig::default()
        },
    );
    let req = CreateRuntimeSessionReq {
        title_auto: false,
        provider: Some(RuntimeProvider::Claude),
        cwd: Some(root),
        title: None,
        model: None,
        access_mode: None,
        meta: Some(serde_json::json!({"source": "workflow", "run_id": "r1"})),
    };
    let step = rt.create(&"u1".into(), &"w1".into(), req).await.unwrap().id;
    let sub = rt.open_peer(&step, open()).await.unwrap().id;
    (rt, step, sub)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sub_threads_events_and_auto_turn_grace_are_the_steps() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, step, sub) = family(&dir).await;
    let root = rt.live(&step).await.unwrap();
    // Later than any event the fakes' own start-up may still bring; such an
    // event resets it, so set it again until one look sees it.
    let later = Instant::now() + Duration::from_secs(3600);
    let b = rt.live(&sub).await.unwrap();
    let mut look = Look::default();
    for _ in 0..20 {
        {
            let mut st = b.lock();
            st.last_activity = later;
            st.background_ended_idle_at = Some(later);
        }
        look = rt.look(&step, &root, 0);
        if look.last_activity == Some(later) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(
        look.last_activity,
        Some(later),
        "no_progress counts the sub-thread"
    );
    assert!(
        !look.settled(later),
        "its own follow-up turn may still start"
    );
    assert!(look.settled(later + AUTO_TURN_GRACE));
    rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reply_the_loop_guard_holds_below_the_step_ends_it_as_loop_guard() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, step, sub) = family(&dir).await;
    rt.live(&sub)
        .await
        .unwrap()
        .lock()
        .proj
        .queue
        .push(QueuedRecord {
            id: "q1".into(),
            created_at: Utc::now(),
            req: Arc::new(RuntimePromptReq::of("the grandchild's reply")),
            wake: None,
            error: Some(loop_guard::PAUSED_QUEUED.into()),
        });
    let root = rt.live(&step).await.unwrap();
    let look = rt.look(&step, &root, 0);
    assert!(look.held_wake, "the held reply below the step is seen");
    assert!(!look.settled(Instant::now() + AUTO_TURN_GRACE));
    let out = rt
        .await_settled(&step, 0, TurnOpts::new(CancelSignal::new()), |_| {})
        .await
        .unwrap();
    assert_eq!(out.ended, Ended::LoopGuard);
    rt.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_sub_thread_does_not_hide_a_busy_one_below_it() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, step, sub) = family(&dir).await;
    let grandchild = rt.open_peer(&sub, open()).await.unwrap().id;
    rt.close(&sub).await.unwrap();
    let c = rt.live(&grandchild).await.unwrap();
    c.lock().starting = 1;
    let root = rt.live(&step).await.unwrap();
    let look = rt.look(&step, &root, 0);
    assert!(look.descendants_busy, "the grandchild still works");
    assert!(look.descendants.iter().any(|(id, _)| *id == grandchild));
    c.lock().starting = 0;
    rt.shutdown().await;
}

async fn settle(rt: &AgentRuntime, step: &Id, opts: TurnOpts) -> Ended {
    tokio::time::timeout(
        Duration::from_secs(5),
        rt.await_settled(step, 0, opts, |_| {}),
    )
    .await
    .expect("the wait ends")
    .unwrap()
    .ended
}

fn opts() -> TurnOpts {
    TurnOpts::new(CancelSignal::new())
}

/// CR2-001: a sub-thread's background work ended long ago with no turn
/// after it; its past grace must not hide a limit that is due.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_expired_auto_turn_grace_does_not_hide_a_due_limit() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, step, sub) = family(&dir).await;
    let b = rt.live(&sub).await.unwrap();
    b.lock().background_ended_idle_at = Some(Instant::now() - 2 * AUTO_TURN_GRACE);
    b.lock().starting = 1;
    let mut o = opts();
    o.deadline = Some(Duration::from_millis(200));
    assert_eq!(settle(&rt, &step, o).await, Ended::Deadline);
    let mut o = opts();
    o.no_progress = Some(Duration::from_millis(200));
    assert_eq!(settle(&rt, &step, o).await, Ended::NoProgress);
    b.lock().starting = 0;
    rt.shutdown().await;
}

/// CR1-030: a held reply ends the wait only once nothing else is about to
/// start — not while a sibling's reply is announced, nor within a
/// sub-thread's auto-turn grace.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_reply_waits_for_an_announced_wake_and_the_auto_turn_grace() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, step, sub) = family(&dir).await;
    let b = rt.live(&sub).await.unwrap();
    b.lock().proj.queue.push(QueuedRecord {
        id: "q1".into(),
        created_at: Utc::now(),
        req: Arc::new(RuntimePromptReq::of("the held reply")),
        wake: None,
        error: Some(loop_guard::PAUSED_QUEUED.into()),
    });
    let root = rt.live(&step).await.unwrap();
    let due = || {
        let mut o = opts();
        o.deadline = Some(Duration::from_millis(300));
        o
    };
    root.pending_wakes().push("sibling".into());
    assert_eq!(settle(&rt, &step, due()).await, Ended::Deadline);
    root.pending_wakes().clear();
    b.lock().background_ended_idle_at = Some(Instant::now());
    assert_eq!(settle(&rt, &step, due()).await, Ended::Deadline);
    b.lock().background_ended_idle_at = None;
    assert_eq!(settle(&rt, &step, due()).await, Ended::LoopGuard);
    rt.shutdown().await;
}

/// CR1-030: a sub-thread's background work pauses no-progress as the
/// step's own does, and counts toward the background limit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sub_threads_background_work_pauses_no_progress_and_meets_the_limit() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, step, sub) = family(&dir).await;
    let b = rt.live(&sub).await.unwrap();
    b.lock().session.background_tasks = vec![otto_core::agent_runtime::BackgroundTask {
        id: "t1".into(),
        kind: "command".into(),
        label: "npm run dev".into(),
        started_at: Utc::now(),
        item_id: None,
        due_at: None,
        stopping: false,
        stop_error: None,
    }];
    let mut o = opts();
    o.no_progress = Some(Duration::from_millis(100));
    o.deadline = Some(Duration::from_millis(600));
    assert_eq!(settle(&rt, &step, o).await, Ended::Deadline);
    let mut o = opts();
    o.background_limit = Duration::from_millis(200);
    assert_eq!(settle(&rt, &step, o).await, Ended::BackgroundLimit);
    rt.shutdown().await;
}

/// CR1-031: the family runs through a closed sub-thread to the open one
/// below it, asked from the step or from a sub-thread.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_family_reaches_below_a_closed_sub_thread_from_any_member() {
    let dir = tempfile::tempdir().unwrap();
    let (rt, step, sub) = family(&dir).await;
    let grandchild = rt.open_peer(&sub, open()).await.unwrap().id;
    assert_eq!(rt.family(&sub).await, [sub.clone(), grandchild.clone()]);
    rt.close(&sub).await.unwrap();
    assert_eq!(rt.family(&step).await, [step.clone(), grandchild.clone()]);
    rt.shutdown().await;
}
