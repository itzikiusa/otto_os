//! The bridge drops a message from a sender the integration's `allowed_users`
//! list does not admit — and now tells the workflow trigger about it, so a
//! chat trigger from them no longer "does nothing" without a trace. Exactly
//! once per fresh not-admitted message; never for an admitted sender, never
//! for an edit; and never a reply to the stranger.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use otto_channels::adapter::{Adapter, Inbound};
use otto_channels::admission::{Admission, InboundAdmission};
use otto_channels::bridge::Bridge;
use otto_channels::mirror::Mirror;
use otto_channels::workflow_trigger::{WorkflowChatAck, WorkflowChatTrigger};
use otto_core::domain::{Channel, Integration};
use otto_sessions::SessionManager;
use otto_state::{SettingsRepo, WorkspacesRepo};

#[derive(Default)]
struct Calls {
    noted: AtomicUsize,
    started: AtomicUsize,
    sent: AtomicUsize,
}

struct Trigger(Arc<Calls>);

#[async_trait]
impl WorkflowChatTrigger for Trigger {
    async fn try_start(
        &self,
        _workspace_id: &str,
        _channel: &str,
        _chat: &str,
        _thread: Option<&str>,
        _user: &str,
        _text: &str,
    ) -> Option<WorkflowChatAck> {
        self.0.started.fetch_add(1, Ordering::SeqCst);
        // Claim the message, so an admitted one stops here (no session).
        Some(WorkflowChatAck {
            reply: "started".into(),
        })
    }

    async fn note_rejected_sender(
        &self,
        _workspace_id: &str,
        channel: &str,
        chat: &str,
        _thread: Option<&str>,
        user: &str,
        text: &str,
    ) {
        assert_eq!(
            (channel, chat, user, text),
            ("slack", "C1", "U2", "run PR Reviewer: go")
        );
        self.0.noted.fetch_add(1, Ordering::SeqCst);
    }
}

struct CountingAdapter(Arc<Calls>);

#[async_trait]
impl Adapter for CountingAdapter {
    async fn send(&self, _c: &str, _t: Option<&str>, _x: &str) -> anyhow::Result<String> {
        self.0.sent.fetch_add(1, Ordering::SeqCst);
        Ok("m1".into())
    }
    async fn edit(&self, _c: &str, _m: &str, _x: &str) -> anyhow::Result<()> {
        Ok(())
    }
    fn channel(&self) -> Channel {
        Channel::Slack
    }
}

fn integration(workspace_id: &str) -> Integration {
    Integration {
        workspace_id: workspace_id.into(),
        channel: Channel::Slack,
        enabled: true,
        allowed_users: "U1".into(),
        open_to_all: false,
        agent_reply: true,
        reply_instructions: String::new(),
        channel_id: String::new(),
        preferred_cli: String::new(),
        has_bot_token: true,
        has_app_token: true,
        updated_at: chrono::Utc::now(),
    }
}

fn inbound(workspace_id: &str, user: &str, edited: bool) -> Inbound {
    Inbound {
        workspace_id: workspace_id.into(),
        chat: "C1".into(),
        thread: Some("1.1".into()),
        user: user.into(),
        user_name: None,
        text: "run PR Reviewer: go".into(),
        edited,
    }
}

#[tokio::test]
async fn the_hook_fires_once_for_a_not_admitted_sender_and_never_for_an_admitted_one() {
    let dir = tempfile_dir();
    let pool = otto_state::db::open(&dir.join("state.sqlite"))
        .await
        .unwrap();
    let user = otto_state::UsersRepo::new(pool.clone())
        .create("owner", "", "Owner", true)
        .await
        .unwrap();
    let ws = WorkspacesRepo::new(pool.clone())
        .create("WS", "/tmp", &user.id)
        .await
        .unwrap();
    let (events, _) = tokio::sync::broadcast::channel(16);
    let manager = Arc::new(SessionManager::new(
        otto_state::SessionsRepo::new(pool.clone()),
        events,
        otto_sessions::providers::ProviderRegistry::new(None),
    ));
    let calls = Arc::new(Calls::default());
    let bridge = Bridge::new_with_workflow_trigger(
        manager.clone(),
        WorkspacesRepo::new(pool.clone()),
        SettingsRepo::new(pool.clone()),
        Mirror::new(manager.clone()),
        "root".into(),
        Some(Arc::new(Trigger(calls.clone()))),
        InboundAdmission::default(),
    );
    let integ = integration(&ws.id);
    let adapter = || Arc::new(CountingAdapter(calls.clone())) as Arc<dyn Adapter>;
    let permit = |n: usize| match bridge.admission.admit("slack", "C1", Some(&n.to_string())) {
        Admission::Work(p) => p,
        _ => panic!("capacity available"),
    };

    // Not admitted: dropped, hook told once, nothing sent, nothing started.
    bridge
        .handle(&integ, adapter(), inbound(&ws.id, "U2", false), permit(1))
        .await;
    assert_eq!(calls.noted.load(Ordering::SeqCst), 1);
    assert_eq!(calls.started.load(Ordering::SeqCst), 0);
    assert_eq!(
        calls.sent.load(Ordering::SeqCst),
        0,
        "a stranger never gets a reply"
    );

    // An edit of that message re-fires nothing, the hook included.
    bridge
        .handle(&integ, adapter(), inbound(&ws.id, "U2", true), permit(2))
        .await;
    assert_eq!(calls.noted.load(Ordering::SeqCst), 1);

    // Admitted: routed to the trigger as before, the hook never called.
    bridge
        .handle(&integ, adapter(), inbound(&ws.id, "U1", false), permit(3))
        .await;
    assert_eq!(calls.noted.load(Ordering::SeqCst), 1);
    assert_eq!(calls.started.load(Ordering::SeqCst), 1);
    assert_eq!(calls.sent.load(Ordering::SeqCst), 1);

    drop((bridge, manager, pool));
    let _ = std::fs::remove_dir_all(dir);
}

fn tempfile_dir() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("otto-bridge-rejected-{}", otto_core::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
