//! A chat trigger from a sender the integration does not admit.
//!
//! The channel bridge drops a message from anyone outside the integration's
//! `allowed_users` list before any routing — on purpose, and without a reply
//! (answering would let anyone probe the bot). The cost was a trigger that
//! "does nothing" with no trace in the app: the owner's known incident. The
//! bridge now hands such a message to
//! [`WorkflowChatTrigger::note_rejected_sender`](otto_channels::workflow_trigger::WorkflowChatTrigger::note_rejected_sender),
//! and this module records it where the owner looks — the matching
//! trigger's status line ("Last attempt rejected: sender … is not on the
//! integration's allowed users") plus, the first time a day per trigger, a
//! System notice in the bell. It only MATCHES the message (the same binding
//! and name rules `try_start` uses); it never starts, replies or reads further.
//! A message that is no workflow trigger at all is ignored.

use chrono::{DateTime, Utc};
use otto_core::domain::{NoticeAction, NoticeKind, NoticeSeverity};
use otto_core::event::Event;
use otto_state::{NewNotice, NotificationsRepo, TriggersRepo, WorkflowTrigger, WorkflowsRepo};
use serde_json::Value;

use crate::chat::{
    binding_candidates, binding_matches, is_own_ack, mentions_someone, parse_run_command,
    parse_workflow_command,
};
use crate::chat_resolve::{resolve_name, NameMatch};
use crate::trigger_outcome::{stamp, TriggerOutcome};
use crate::WorkflowCtx;

/// The stamped reason. Starts with [`REASON_PREFIX`] so a repeat the same day
/// is recognised (no second bell notice).
pub fn sender_reason(user: &str) -> String {
    format!("{REASON_PREFIX}{user} is not on the integration's allowed users")
}

const REASON_PREFIX: &str = "sender ";

/// Whether this rejection is the first of its kind on `trigger` today (UTC):
/// the trigger's PREVIOUS stamp was not already a not-allowed-sender
/// rejection from the same day. Read before the new stamp lands.
pub fn first_today(previous: &WorkflowTrigger, now: DateTime<Utc>) -> bool {
    let same_kind = previous.last_outcome.as_deref() == Some("rejected")
        && previous
            .last_detail
            .as_deref()
            .is_some_and(|d| d.starts_with(REASON_PREFIX) && d.contains("allowed users"));
    let same_day = previous
        .last_fired_at
        .is_some_and(|t| t.date_naive() == now.date_naive());
    !(same_kind && same_day)
}

/// The workflow a typed name would have started in `workspace_id`, if any:
/// an `Action: Workflow` command, or a `run <name>:` / `workflow <name>:`
/// command whose name resolves (exact, then normalised — `try_start`'s rule).
async fn named_workflow(repo: &WorkflowsRepo, workspace_id: &str, text: &str) -> Option<String> {
    let typed = match parse_workflow_command(text) {
        Some(cmd) => cmd.name,
        None => parse_run_command(text)?.0,
    };
    let ws = workspace_id.to_string();
    if let Ok(Some(wf)) = repo.find_by_name(&typed, &ws).await {
        return Some(wf.id);
    }
    match resolve_name(&typed, &repo.list_names(&ws).await.ok()?) {
        NameMatch::One(id, _) => Some(id),
        _ => None,
    }
}

/// Record a dropped message from a not-admitted sender against the trigger(s)
/// it was meant for. Best-effort; never replies.
pub async fn note<C: WorkflowCtx>(
    ctx: &C,
    workspace_id: &str,
    channel: &str,
    chat: &str,
    thread: Option<&str>,
    user: &str,
    text: &str,
) {
    if is_own_ack(text) || text.trim().is_empty() {
        return;
    }
    let repo = WorkflowsRepo::new(ctx.pool().clone());
    let Ok(chat_triggers) = TriggersRepo::new(ctx.pool().clone())
        .list_enabled_by_kind("chat")
        .await
    else {
        return;
    };
    let mut hits: Vec<&WorkflowTrigger> = Vec::new();
    // (1) The binding a message here would have fired: the first candidate
    // owned by the receiving workspace, as in `try_start`.
    let has_mention = mentions_someone(text);
    for t in binding_candidates(&chat_triggers, channel, chat, thread, has_mention) {
        if repo
            .get(&t.workflow_id)
            .await
            .is_ok_and(|w| w.workspace_id == workspace_id)
        {
            hits.push(t);
            break;
        }
    }
    // (2) A workflow named explicitly: its chat bindings on THIS chat (any
    // thread, mention or not) — the row the owner reads for this channel.
    let named = named_workflow(&repo, workspace_id, text).await;
    if let Some(wf_id) = &named {
        for t in chat_triggers.iter().filter(|t| &t.workflow_id == wf_id) {
            if binding_matches(&without_thread(&t.spec), channel, chat, None, true)
                && !hits.iter().any(|h| h.id == t.id)
            {
                hits.push(t);
            }
        }
    }
    if hits.is_empty() {
        if let Some(wf_id) = named {
            tracing::warn!(
                workspace = workspace_id, channel, chat, user, workflow_id = %wf_id,
                "workflow chat: explicit trigger dropped — sender is not on the integration's allowed users"
            );
        }
        return;
    }
    let reason = sender_reason(user);
    let now = Utc::now();
    for t in hits {
        tracing::warn!(
            workspace = workspace_id, channel, chat, user, trigger_id = %t.id,
            "workflow chat: trigger message dropped — sender is not on the integration's allowed users"
        );
        let first = first_today(t, now);
        stamp(ctx.pool(), &t.id, &TriggerOutcome::Rejected(reason.clone())).await;
        if first {
            notify(ctx, workspace_id, channel, user, t, &repo).await;
        }
    }
}

/// `spec` with its pinned thread removed — a named trigger in another thread
/// of the bound chat still belongs to that binding's row.
fn without_thread(spec: &Value) -> Value {
    let mut s = spec.clone();
    if let Some(o) = s.as_object_mut() {
        o.remove("thread");
    }
    s
}

/// The once-a-day System notice (one row per trigger, refreshed in place via
/// its source key). Says who and where — never the message text.
async fn notify<C: WorkflowCtx>(
    ctx: &C,
    workspace_id: &str,
    channel: &str,
    user: &str,
    trigger: &WorkflowTrigger,
    repo: &WorkflowsRepo,
) {
    let name = repo
        .get(&trigger.workflow_id)
        .await
        .map(|w| w.name)
        .unwrap_or_else(|_| "a workflow".into());
    let new = NewNotice {
        kind: NoticeKind::System,
        severity: NoticeSeverity::Warn,
        title: format!("Workflow trigger blocked: {name}"),
        body: format!(
            "A {channel} message from {user} would have started this workflow, but the sender is \
             not on the integration's allowed users. Add them in Settings → Channels if they \
             should be able to."
        ),
        source_key: Some(format!("workflow-trigger-sender:{}", trigger.id)),
        action: Some(NoticeAction::OpenRoute {
            route: format!("workflows/{}", trigger.workflow_id),
            workspace_id: Some(workspace_id.to_string()),
        }),
        user_id: None,
    };
    match NotificationsRepo::new(ctx.pool().clone()).create(new).await {
        Ok(notice) => {
            let _ = ctx.events().send(Event::Notification {
                notice,
                user_id: None,
            });
        }
        Err(e) => tracing::warn!(trigger_id = %trigger.id, "trigger notice: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn trigger(outcome: Option<&str>, detail: Option<&str>, at: Option<&str>) -> WorkflowTrigger {
        WorkflowTrigger {
            id: "t1".into(),
            workflow_id: "wf1".into(),
            kind: "chat".into(),
            spec: json!({"channel": "slack", "chat": "C1"}),
            enabled: true,
            admission_generation: 0,
            created_at: Utc::now(),
            armed_at: None,
            last_fired_at: at.map(|s| s.parse().unwrap()),
            last_outcome: outcome.map(str::to_string),
            last_detail: detail.map(str::to_string),
        }
    }

    #[test]
    fn the_reason_names_the_sender_and_the_allowlist() {
        assert_eq!(
            sender_reason("U123"),
            "sender U123 is not on the integration's allowed users"
        );
    }

    #[test]
    fn a_bell_notice_only_for_the_first_such_rejection_of_the_day() {
        let now: DateTime<Utc> = "2026-11-12T15:00:00Z".parse().unwrap();
        // Never fired, or fired fine: first.
        assert!(first_today(&trigger(None, None, None), now));
        assert!(first_today(
            &trigger(Some("started"), None, Some("2026-11-12T09:00:00Z")),
            now
        ));
        // Already rejected for a sender today: not again.
        let same = sender_reason("U9");
        assert!(!first_today(
            &trigger(Some("rejected"), Some(&same), Some("2026-11-12T01:00:00Z")),
            now
        ));
        // Yesterday's, or another rejection reason today: first.
        assert!(first_today(
            &trigger(Some("rejected"), Some(&same), Some("2026-11-11T23:59:00Z")),
            now
        ));
        assert!(first_today(
            &trigger(
                Some("rejected"),
                Some("previous run of this workflow still active"),
                Some("2026-11-12T10:00:00Z")
            ),
            now
        ));
    }

    #[derive(Clone)]
    struct Ctx {
        pool: otto_state::DbPool,
        events: tokio::sync::broadcast::Sender<Event>,
        workspaces: otto_state::WorkspacesRepo,
        issues: otto_state::IssuesRepo,
        secrets: std::sync::Arc<dyn otto_core::secrets::SecretStore>,
    }

    struct NoSecrets;
    impl otto_core::secrets::SecretStore for NoSecrets {
        fn put(&self, _k: &str, _v: &str) -> otto_core::Result<()> {
            Ok(())
        }
        fn get(&self, _k: &str) -> otto_core::Result<Option<String>> {
            Ok(None)
        }
        fn delete(&self, _k: &str) -> otto_core::Result<()> {
            Ok(())
        }
    }

    impl WorkflowCtx for Ctx {
        fn pool(&self) -> &otto_state::DbPool {
            &self.pool
        }
        fn events(&self) -> &tokio::sync::broadcast::Sender<Event> {
            &self.events
        }
        fn workspaces(&self) -> &otto_state::WorkspacesRepo {
            &self.workspaces
        }
        fn issues_store(&self) -> &otto_state::IssuesRepo {
            &self.issues
        }
        fn secrets(&self) -> &std::sync::Arc<dyn otto_core::secrets::SecretStore> {
            &self.secrets
        }
        fn spawn_run(
            &self,
            _ws: otto_core::domain::Workspace,
            _wf: otto_core::workflows::Workflow,
            _run_id: otto_core::Id,
            _input: Value,
            _scope: otto_core::workflows::RunScope,
        ) {
            panic!("a dropped sender must never start a run");
        }
        fn request_skip_current(&self, _run_id: &str, _node_id: &str) {}
        async fn check_run_location(
            &self,
            _ws: &otto_core::domain::Workspace,
            _input: &Value,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    /// A real (temp-file) state DB with one workspace, one workflow "PR
    /// Reviewer" and a chat binding `spec`. Returns (dir, ctx, ws id, trigger id).
    async fn fixture(spec: Value) -> (tempfile::TempDir, Ctx, String, String) {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("state.sqlite"))
            .await
            .unwrap();
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("owner", "", "Owner", true)
            .await
            .unwrap();
        let workspaces = otto_state::WorkspacesRepo::new(pool.clone());
        let ws = workspaces.create("WS", "/tmp", &user.id).await.unwrap();
        let wf = WorkflowsRepo::new(pool.clone())
            .create(
                &ws.id,
                "PR Reviewer",
                "",
                "",
                &otto_core::workflows::WorkflowGraph::default(),
                &user.id,
            )
            .await
            .unwrap();
        let t = TriggersRepo::new(pool.clone())
            .create(otto_state::NewWorkflowTrigger {
                workflow_id: wf.id.clone(),
                kind: "chat".into(),
                spec,
                enabled: true,
            })
            .await
            .unwrap();
        let ctx = Ctx {
            events: tokio::sync::broadcast::channel(16).0,
            workspaces,
            issues: otto_state::IssuesRepo::new(pool.clone()),
            secrets: std::sync::Arc::new(NoSecrets),
            pool,
        };
        (dir, ctx, ws.id, t.id)
    }

    fn bound() -> Value {
        json!({"channel": "slack", "chat": "C1"})
    }

    #[tokio::test]
    async fn a_dropped_sender_on_a_bound_channel_stamps_the_binding_once_a_day_noticed() {
        let (_dir, ctx, ws, trigger_id) = fixture(bound()).await;
        let mut bell = ctx.events.subscribe();
        note(
            &ctx,
            &ws,
            "slack",
            "C1",
            Some("1.1"),
            "U9",
            "please review PR 12",
        )
        .await;
        let t = TriggersRepo::new(ctx.pool.clone())
            .get(&trigger_id)
            .await
            .unwrap();
        assert_eq!(t.last_outcome.as_deref(), Some("rejected"));
        assert_eq!(t.last_detail.as_deref(), Some(sender_reason("U9").as_str()));
        assert!(matches!(bell.try_recv(), Ok(Event::Notification { .. })));
        // The same sender again today: stamped, but no second bell notice.
        note(&ctx, &ws, "slack", "C1", Some("1.2"), "U9", "again").await;
        assert!(bell.try_recv().is_err(), "one notice per trigger per day");
    }

    #[tokio::test]
    async fn an_unrelated_chat_message_stamps_nothing() {
        let (_dir, ctx, ws, trigger_id) = fixture(bound()).await;
        let mut bell = ctx.events.subscribe();
        // Another channel, no binding, no workflow named.
        note(&ctx, &ws, "slack", "C2", None, "U9", "lunch?").await;
        // Another workspace's message on the bound channel.
        note(&ctx, "other-ws", "slack", "C1", None, "U9", "hello").await;
        let t = TriggersRepo::new(ctx.pool.clone())
            .get(&trigger_id)
            .await
            .unwrap();
        assert!(t.last_outcome.is_none() && t.last_fired_at.is_none());
        assert!(bell.try_recv().is_err());
    }

    #[tokio::test]
    async fn an_explicit_trigger_is_recorded_on_its_binding_even_off_the_binding_rules() {
        // A mention-only binding pinned to one thread: a plain message in
        // another thread would not fire it, but naming the workflow would
        // start it — so that attempt is recorded on the binding's row.
        let spec = json!({"channel": "slack", "chat": "C1", "thread": "1.1", "mention_only": true});
        let (_dir, ctx, ws, trigger_id) = fixture(spec).await;
        let repo = TriggersRepo::new(ctx.pool.clone());
        note(&ctx, &ws, "slack", "C1", Some("9.9"), "U7", "nice weather").await;
        assert!(repo.get(&trigger_id).await.unwrap().last_outcome.is_none());
        note(
            &ctx,
            &ws,
            "slack",
            "C1",
            Some("9.9"),
            "U7",
            "workflow PR Reviewer: go",
        )
        .await;
        let t = repo.get(&trigger_id).await.unwrap();
        assert_eq!(t.last_detail.as_deref(), Some(sender_reason("U7").as_str()));
    }

    #[test]
    fn a_named_trigger_in_another_thread_still_belongs_to_the_binding() {
        let spec = json!({"channel": "slack", "chat": "C1", "thread": "111.1"});
        assert!(!binding_matches(&spec, "slack", "C1", Some("222.2"), true));
        assert!(binding_matches(
            &without_thread(&spec),
            "slack",
            "C1",
            None,
            true
        ));
    }
}
