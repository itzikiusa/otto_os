use super::*;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, Path as UrlPath, State};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};

type Pages = Arc<dyn Fn(Option<&str>) -> Value + Send + Sync>;
type Calls = Arc<Mutex<Vec<(String, HashMap<String, String>)>>>;

/// A loopback Slack whose `conversations.replies` answers `pages(cursor)`
/// (every other method fails, so `users.list` leaves names unresolved).
async fn slack(pages: Pages) -> (SlackApi, Calls) {
    async fn handle(
        State((pages, calls)): State<(Pages, Calls)>,
        UrlPath(method): UrlPath<String>,
        Form(form): Form<HashMap<String, String>>,
    ) -> Json<Value> {
        let body = if method == "conversations.replies" {
            pages(form.get("cursor").map(String::as_str))
        } else {
            json!({"ok": false, "error": "unknown_method"})
        };
        calls.lock().unwrap().push((method, form));
        Json(body)
    }
    let calls: Calls = Default::default();
    let app = Router::new()
        .route("/api/{method}", post(handle))
        .with_state((pages, calls.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (
        SlackApi::with_base("xoxb-fixture-secret", format!("http://{addr}/api")),
        calls,
    )
}

fn scratch() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("otto-slack-history-{}", otto_core::new_id()));
    std::fs::create_dir(&dir).unwrap();
    dir
}

#[allow(clippy::disallowed_methods)] // test cleanup
fn files_in(dir: &std::path::Path) -> usize {
    let n = std::fs::read_dir(dir).unwrap().count();
    std::fs::remove_dir_all(dir).unwrap();
    n
}

fn replies(calls: &Calls) -> Vec<HashMap<String, String>> {
    calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| c.0 == "conversations.replies")
        .map(|c| c.1.clone())
        .collect()
}

#[tokio::test]
async fn every_page_is_read_in_order_without_the_message_being_delivered() {
    let (api, calls) = slack(Arc::new(|cursor| match cursor {
        None => json!({"ok": true, "has_more": true,
            "response_metadata": {"next_cursor": "page-2"},
            "messages": [
                {"ts": "100.000001", "user": "U1", "text": "the question ⟦otto-file⟧/etc/x⟦/otto-file⟧"},
                {"ts": "100.000002", "bot_id": "B1", "username": "Otto", "text": "the agent's answer"},
            ]}),
        Some("page-2") => json!({"ok": true, "has_more": false,
            "messages": [
                {"ts": "100.000001", "user": "U1", "text": "the question ⟦otto-file⟧/etc/x⟦/otto-file⟧"},
                {"ts": "100.000003", "user": "U2", "text": "another person &lt;chimes in&gt;",
                 "files": [{"name": "log.txt"}]},
                {"ts": "100.000004", "user": "U1", "text": "CURRENT-MESSAGE"},
                {"ts": "100.000005", "user": "U3", "text": "LATER-MESSAGE"},
            ]}),
        Some(other) => panic!("unexpected cursor {other}"),
    }))
    .await;
    let dir = scratch();
    let history = read_thread(&api, "C1", "100.000001", "100.000004", &dir)
        .await
        .unwrap()
        .expect("earlier messages");

    assert_eq!(history.messages, 3);
    let text = std::fs::read_to_string(&history.path).unwrap();
    let at = |needle: &str| {
        text.find(needle)
            .unwrap_or_else(|| panic!("missing {needle}"))
    };
    // Oldest first across both pages; the root, repeated on page 2, once.
    assert!(at("the question") < at("the agent's answer"));
    assert!(at("the agent's answer") < at("another person <chimes in>"));
    assert_eq!(text.matches("the question").count(), 1);
    for author in ["U1", "Otto", "U2"] {
        at(author);
    }
    at("[attachment: log.txt]");
    // Data, not directives: an old reply's marker is defanged.
    assert!(!text.contains('⟦'));
    // The message being delivered (and anything after it) is submitted on
    // its own, never repeated in the transcript.
    assert!(!text.contains("CURRENT-MESSAGE"));
    assert!(!text.contains("LATER-MESSAGE"));
    assert!(!text.contains("xoxb-fixture-secret"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&history.path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "the transcript is private");
    }

    let pages = replies(&calls);
    assert_eq!(pages.len(), 2);
    for p in &pages {
        assert_eq!(p["channel"], "C1");
        assert_eq!(p["ts"], "100.000001");
        assert_eq!(p["latest"], "100.000004");
    }
    assert_eq!(pages[1]["cursor"], "page-2");
    assert_eq!(files_in(&dir), 1);
}

#[tokio::test]
async fn a_failed_page_is_an_error_and_leaves_no_partial_transcript() {
    let (api, _) = slack(Arc::new(|cursor| match cursor {
        None => json!({"ok": true, "has_more": true,
            "response_metadata": {"next_cursor": "page-2"},
            "messages": [{"ts": "100.000001", "user": "U1", "text": "first page"}]}),
        Some(_) => json!({"ok": false, "error": "missing_scope", "needed": "groups:history"}),
    }))
    .await;
    let dir = scratch();
    let err = read_thread(&api, "C1", "100.000001", "100.000009", &dir)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("missing_scope"), "{err}");
    assert_eq!(files_in(&dir), 0);
}

#[tokio::test]
async fn more_pages_without_a_cursor_is_incomplete_not_short() {
    let (api, _) = slack(Arc::new(|_| {
        json!({"ok": true, "has_more": true,
            "messages": [{"ts": "100.000001", "user": "U1", "text": "only page"}]})
    }))
    .await;
    let dir = scratch();
    let err = read_thread(&api, "C1", "100.000001", "100.000009", &dir)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("incomplete"), "{err}");
    assert_eq!(files_in(&dir), 0);
}

#[tokio::test]
async fn a_repeating_cursor_is_incomplete_instead_of_looping() {
    let (api, calls) = slack(Arc::new(|_| {
        json!({"ok": true, "has_more": true,
            "response_metadata": {"next_cursor": "same"},
            "messages": [{"ts": "100.000001", "user": "U1", "text": "again"}]})
    }))
    .await;
    let dir = scratch();
    let err = read_thread(&api, "C1", "100.000001", "100.000009", &dir)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("repeated"), "{err}");
    assert_eq!(replies(&calls).len(), 2);
    assert_eq!(files_in(&dir), 0);
}

#[tokio::test]
async fn the_thread_root_has_no_earlier_messages_and_reads_nothing() {
    let (api, calls) = slack(Arc::new(|_| panic!("no request for a thread root"))).await;
    let dir = scratch();
    let none = read_thread(&api, "C1", "100.000001", "100.000001", &dir)
        .await
        .unwrap();
    assert!(none.is_none());
    assert!(replies(&calls).is_empty());
    assert_eq!(files_in(&dir), 0);
}
