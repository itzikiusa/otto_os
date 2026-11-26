//! `SlackAdapter::edit_formatted` / `upload_with_caption` and the attachment
//! list against a loopback Web API fixture (DR-025, DR-109, E6.12).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{Form, Json};
use axum::routing::post;
use serde_json::{json, Value};

use super::*;
use crate::adapter::edit_needs_new_post;

/// Serve `router` on an ephemeral loopback port; returns its origin.
async fn fixture(router: axum::Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    format!("http://{addr}")
}

/// A `chat.update` that records its bodies and answers `reply`.
async fn update_fixture(reply: Value) -> (String, Arc<Mutex<Vec<Value>>>) {
    let seen = Arc::new(Mutex::new(vec![]));
    let s = seen.clone();
    let base = fixture(axum::Router::new().route(
        "/chat.update",
        post(move |Json(body): Json<Value>| {
            let s = s.clone();
            let reply = reply.clone();
            async move {
                s.lock().unwrap().push(body);
                Json(reply)
            }
        }),
    ))
    .await;
    (base, seen)
}

#[tokio::test]
async fn edit_formatted_sends_mrkdwn_cut_after_conversion() {
    let (base, seen) = update_fixture(json!({"ok": true})).await;
    let a = SlackAdapter::with_base("xoxb-t", &base);
    a.edit_formatted("C1", "1.2", "**PR Reviewer** · working")
        .await
        .unwrap();
    let long = format!("**head**\n{}", "x".repeat(5_000));
    a.edit_formatted("C1", "1.2", &long).await.unwrap();
    a.edit("C1", "1.2", "**plain**").await.unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0]["text"], "*PR Reviewer* · working");
    assert_eq!(seen[0]["mrkdwn"], true);
    assert_eq!(seen[0]["ts"], "1.2");
    let cut = seen[1]["text"].as_str().unwrap();
    assert_eq!(cut.chars().count(), EDIT_MAX_CHARS);
    assert!(cut.starts_with("*head*") && cut.ends_with('…'));
    assert_eq!(seen[2]["text"], "**plain**", "plain edit is unchanged");
    assert!(seen[2].get("mrkdwn").is_none());
}

#[tokio::test]
async fn edit_errors_that_mean_post_again() {
    for (code, repost) in [
        ("message_not_found", true),
        ("cant_update_message", true),
        ("edit_window_closed", true),
        ("channel_not_found", false),
        ("msg_too_long", false),
    ] {
        let (base, _) = update_fixture(json!({"ok": false, "error": code})).await;
        let err = SlackAdapter::with_base("xoxb-t", &base)
            .edit_formatted("C1", "1.2", "x")
            .await
            .unwrap_err();
        assert_eq!(edit_needs_new_post(&err), repost, "{code}: {err}");
    }
}

#[tokio::test]
async fn upload_with_caption_sets_initial_comment_and_plain_upload_does_not() {
    let completes: Arc<Mutex<Vec<HashMap<String, String>>>> = Arc::default();
    let c = completes.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let put_url = format!("{base}/put");
    let router = axum::Router::new()
        .route(
            "/files.getUploadURLExternal",
            post(move || {
                let put_url = put_url.clone();
                async move { Json(json!({"ok": true, "upload_url": put_url, "file_id": "F1"})) }
            }),
        )
        .route("/put", post(|| async { "OK" }))
        .route(
            "/files.completeUploadExternal",
            post(move |Form(form): Form<HashMap<String, String>>| {
                let c = c.clone();
                async move {
                    c.lock().unwrap().push(form);
                    Json(json!({"ok": true}))
                }
            }),
        );
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    let a = SlackAdapter::with_base("xoxb-t", &base);
    a.upload_with_caption(
        "C1",
        Some("1.2"),
        "step3.md",
        b"# body",
        "✅ **Checkout** · 14s",
    )
    .await
    .unwrap();
    a.upload("C1", Some("1.2"), "step4.md", b"# body")
        .await
        .unwrap();
    let completes = completes.lock().unwrap();
    assert_eq!(completes[0]["initial_comment"], "✅ *Checkout* · 14s");
    assert_eq!(completes[0]["thread_ts"], "1.2");
    assert!(!completes[1].contains_key("initial_comment"));
}

#[tokio::test]
async fn attachments_come_back_as_data_beside_the_note() {
    let id = format!("F{}", otto_core::new_id());
    let base = fixture(
        axum::Router::new()
            .route("/files/ok.txt", axum::routing::get(|| async { "lorem" }))
            .route(
                "/files/gone.pdf",
                axum::routing::get(|| async { axum::http::StatusCode::NOT_FOUND }),
            ),
    )
    .await;
    let event = json!({
        "type": "message", "channel": "C1", "user": "U1", "ts": "1.1",
        "text": "Action: Workflow",
        "files": [
            {"id": id, "name": "../ok.txt", "mimetype": "text/plain", "size": 5,
             "url_private_download": format!("{base}/files/ok.txt")},
            {"id": "F-gone", "name": "gone.pdf", "mimetype": "application/pdf",
             "url_private_download": format!("{base}/files/gone.pdf")},
        ],
    });
    let inbound = Inbound {
        workspace_id: "w".into(),
        chat: "C1".into(),
        thread: None,
        user: "U1".into(),
        user_name: None,
        text: "Action: Workflow".into(),
        edited: false,
        attachments: vec![],
    };
    let got = with_attachments(&event, inbound, "xoxb-t").await.unwrap();
    assert!(got.text.contains("• ../ok.txt (text/plain) — saved to: "));
    let [ok, gone] = got.attachments.as_slice() else {
        panic!("two files: {:?}", got.attachments)
    };
    assert_eq!(
        (ok.name.as_str(), ok.mime.as_str(), ok.size_bytes),
        ("../ok.txt", "text/plain", 5)
    );
    let path = ok.path.as_ref().unwrap();
    assert!(got.text.contains(&path.display().to_string()));
    assert_eq!(std::fs::read_to_string(path).unwrap(), "lorem");
    let _ = std::fs::remove_file(path);
    assert!(gone.path.is_none());
    assert!(gone
        .error
        .as_deref()
        .unwrap()
        .contains("could not download"));
}
