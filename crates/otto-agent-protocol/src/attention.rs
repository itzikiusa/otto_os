//! The daemon's half of [`RuntimeHost::attention`]: the persistent "Agent
//! needs attention" notice a structured thread raises when it starts waiting
//! on its person — the same notice a terminal agent raises (`otto-server`
//! `routes/activity.rs` `notify_waiting`): kind `session`, severity `warn`,
//! source key `session:<id>:waiting` (re-raising refreshes it in place,
//! unread again), gated by Settings › Notifications "session events" (read
//! per notice, so the switch applies live). The bell lists it and the app
//! raises the native banner for it (warn).
//!
//! Two differences, both because a thread is not a terminal session: the
//! notice belongs to the thread's owner (runtime threads are owner-only),
//! and its action opens the thread in Threads (`open_route` `threads/<id>`,
//! which the UI routes as it is; `open_session` looks the id up among
//! terminal sessions and would say "Session unavailable"). When the episode
//! ends (answered, withdrawn, stopped) the notice is marked read, so the bell
//! does not keep asking for something already answered.
//!
//! Notices are written in order by one worker (a settle never overtakes the
//! raise it settles).
//!
//! [`RuntimeHost::attention`]: crate::host::RuntimeHost::attention

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use otto_core::agent_runtime::RuntimeSession;
use otto_core::domain::{Notice, NoticeAction, NoticeKind, NoticeSeverity};
use otto_core::event::Event;
use otto_core::Id;
use otto_state::{NewNotice, NoticeAccess, NotificationsRepo};
use tokio::sync::mpsc;

/// Where notices and their read-state changes are published (the daemon bus).
pub type Publish = Arc<dyn Fn(Event) + Send + Sync>;

/// Characters of what the agent asks the notice body quotes (the PTY path's clip).
const WHAT_CHARS: usize = 160;
pub const NOTICE_TITLE: &str = "Agent needs attention";

enum Op {
    Raise(NewNotice, Id),
    Settle { session: Id, owner: Id },
}

pub struct AttentionNotices {
    repo: NotificationsRepo,
    publish: Publish,
    tx: Mutex<Option<mpsc::UnboundedSender<Op>>>,
}

impl AttentionNotices {
    pub fn new(repo: NotificationsRepo, publish: Publish) -> Self {
        Self {
            repo,
            publish,
            tx: Mutex::new(None),
        }
    }

    /// Raise (`Some(what)`) or settle (`None`) `s`'s notice. Never blocks.
    pub fn signal(&self, s: &RuntimeSession, what: Option<&str>) {
        let op = match what {
            Some(what) => Op::Raise(waiting_notice(s, what), s.id.clone()),
            None => Op::Settle {
                session: s.id.clone(),
                owner: s.created_by.clone(),
            },
        };
        let mut tx = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        if tx.as_ref().is_none_or(|t| t.is_closed()) {
            let Ok(rt) = tokio::runtime::Handle::try_current() else {
                return;
            };
            let (t, rx) = mpsc::unbounded_channel();
            rt.spawn(worker(self.repo.clone(), Arc::clone(&self.publish), rx));
            *tx = Some(t);
        }
        if let Some(t) = tx.as_ref() {
            let _ = t.send(op);
        }
    }
}

/// The notice for thread `s` waiting on `what` ("wants approval: …").
pub fn waiting_notice(s: &RuntimeSession, what: &str) -> NewNotice {
    let what = what.trim();
    let what = if what.chars().count() > WHAT_CHARS {
        format!("{}…", what.chars().take(WHAT_CHARS).collect::<String>())
    } else {
        what.to_string()
    };
    NewNotice {
        kind: NoticeKind::Session,
        severity: NoticeSeverity::Warn,
        title: NOTICE_TITLE.to_string(),
        body: format!("{} · {what}", s.title),
        source_key: Some(format!("session:{}:waiting", s.id)),
        action: Some(NoticeAction::OpenRoute {
            route: format!("threads/{}", s.id),
            workspace_id: Some(s.workspace_id.clone()),
        }),
        user_id: Some(s.created_by.clone()),
    }
}

async fn worker(repo: NotificationsRepo, publish: Publish, mut rx: mpsc::UnboundedReceiver<Op>) {
    // Thread → its unread waiting notice (raised this daemon life).
    let mut open: HashMap<Id, Id> = HashMap::new();
    while let Some(op) = rx.recv().await {
        match op {
            Op::Raise(new, session) => {
                let user_id = new.user_id.clone();
                if let Some(notice) = raise(&repo, new).await {
                    open.insert(session, notice.id.clone());
                    publish(Event::Notification { notice, user_id });
                }
            }
            Op::Settle { session, owner } => {
                let Some(id) = open.remove(&session) else {
                    continue;
                };
                match repo
                    .mark_read(&id, &NoticeAccess::User(owner.clone()))
                    .await
                {
                    Ok(()) => publish(Event::NotificationsChanged { user_id: owner }),
                    Err(e) => tracing::warn!(%session, "thread notice: mark read: {e}"),
                }
            }
        }
    }
}

/// Persist `new` unless session notices are switched off.
async fn raise(repo: &NotificationsRepo, new: NewNotice) -> Option<Notice> {
    match repo.get_settings().await {
        Ok(s) if !s.session_events => return None,
        Ok(_) => {}
        Err(e) => {
            tracing::warn!("thread notice: read settings: {e}");
            return None;
        }
    }
    repo.create(new)
        .await
        .map_err(|e| tracing::warn!("thread notice: create: {e}"))
        .ok()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    async fn pool() -> otto_state::DbPool {
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
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) \
             VALUES ('u1', 'u', 'x', 'U', 0, ?)",
        )
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
        pool.into()
    }

    fn thread() -> RuntimeSession {
        serde_json::from_value(serde_json::json!({
            "id": "rt1", "workspace_id": "w1", "provider": "codex", "title": "Fix the build",
            "cwd": "/w", "status": "awaiting_permission", "access_mode": "approval-required",
            "created_by": "u1", "created_at": chrono::Utc::now(),
            "last_active_at": chrono::Utc::now(), "last_sequence": 0
        }))
        .unwrap()
    }

    /// Waits until `n` events were published.
    async fn published(events: &Mutex<Vec<Event>>, n: usize) -> Vec<Event> {
        for _ in 0..200 {
            if events.lock().unwrap().len() >= n {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        events.lock().unwrap().clone()
    }

    #[test]
    fn the_notice_opens_the_thread_and_belongs_to_its_owner() {
        let n = waiting_notice(&thread(), "wants approval: rm -rf target");
        assert_eq!(n.title, "Agent needs attention");
        assert_eq!(n.body, "Fix the build · wants approval: rm -rf target");
        assert_eq!(n.severity, NoticeSeverity::Warn);
        assert_eq!(n.source_key.as_deref(), Some("session:rt1:waiting"));
        assert_eq!(n.user_id.as_deref(), Some("u1"));
        assert!(matches!(
            &n.action,
            Some(NoticeAction::OpenRoute { route, workspace_id })
                if route == "threads/rt1" && workspace_id.as_deref() == Some("w1")
        ));
        let long = waiting_notice(&thread(), &"x".repeat(400));
        assert!(
            long.body.chars().count() < 200,
            "clipped like the PTY notice"
        );
    }

    #[tokio::test]
    async fn raised_once_per_episode_read_when_answered_and_raised_again_unread() {
        let repo = NotificationsRepo::new(pool().await);
        let events: Arc<Mutex<Vec<Event>>> = Arc::default();
        let sink = Arc::clone(&events);
        let notices = AttentionNotices::new(
            repo.clone(),
            Arc::new(move |e| sink.lock().unwrap().push(e)),
        );
        let s = thread();
        notices.signal(&s, Some("wants approval: cargo test"));
        let evs = published(&events, 1).await;
        assert!(matches!(&evs[0], Event::Notification { notice, user_id }
            if !notice.read && user_id.as_deref() == Some("u1")));
        notices.signal(&s, None);
        let evs = published(&events, 2).await;
        assert!(matches!(&evs[1], Event::NotificationsChanged { user_id } if user_id == "u1"));
        let all = repo.list(10, &NoticeAccess::All).await.unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].read, "answered: the notice is read");
        // The next episode refreshes the SAME notice, unread again.
        notices.signal(&s, Some("asks: which branch?"));
        published(&events, 3).await;
        let all = repo.list(10, &NoticeAccess::All).await.unwrap();
        assert_eq!(all.len(), 1);
        assert!(!all[0].read);
        assert!(all[0].body.ends_with("asks: which branch?"));
        notices.signal(&s, None);
        published(&events, 4).await;
        // A second settle (nothing raised since) publishes nothing.
        notices.signal(&s, None);
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(events.lock().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn the_session_events_switch_silences_it() {
        let repo = NotificationsRepo::new(pool().await);
        let mut settings = repo.get_settings().await.unwrap();
        settings.session_events = false;
        repo.put_settings(&settings).await.unwrap();
        let events: Arc<Mutex<Vec<Event>>> = Arc::default();
        let sink = Arc::clone(&events);
        let notices = AttentionNotices::new(
            repo.clone(),
            Arc::new(move |e| sink.lock().unwrap().push(e)),
        );
        notices.signal(&thread(), Some("wants approval: ls"));
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(events.lock().unwrap().is_empty());
        assert!(repo.list(10, &NoticeAccess::All).await.unwrap().is_empty());
    }
}
