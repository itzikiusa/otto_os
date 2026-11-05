use super::*;
use serde_json::json;

pub(crate) mod mock {
    //! A loopback mock of the Slack Web API: `POST /api/{method}` answers
    //! the canned body for `method` (else `{ok:false,error:"unknown_method"}`)
    //! and records `(method, form params, bearer token)`.
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    use axum::extract::{Form, Path, State};
    use axum::http::HeaderMap;
    use axum::response::IntoResponse;
    use axum::routing::post;
    use axum::{Json, Router};
    use serde_json::{json, Value};

    pub type Calls = Arc<Mutex<Vec<(String, HashMap<String, String>, String)>>>;

    #[derive(Clone)]
    struct St {
        replies: Arc<HashMap<String, Value>>,
        scopes: String,
        calls: Calls,
    }

    async fn handle(
        State(st): State<St>,
        Path(method): Path<String>,
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> impl IntoResponse {
        let bearer = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .trim_start_matches("Bearer ")
            .to_string();
        st.calls
            .lock()
            .unwrap()
            .push((method.clone(), form, bearer));
        let body = st
            .replies
            .get(&method)
            .cloned()
            .unwrap_or_else(|| json!({"ok": false, "error": "unknown_method"}));
        ([("x-oauth-scopes", st.scopes.clone())], Json(body))
    }

    /// Serve the mock; returns its API base and the call log.
    pub async fn serve(replies: Vec<(&str, Value)>, scopes: &str) -> (String, Calls) {
        let calls: Calls = Default::default();
        let st = St {
            replies: Arc::new(
                replies
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v))
                    .collect(),
            ),
            scopes: scopes.to_string(),
            calls: calls.clone(),
        };
        let app = Router::new()
            .route("/api/{method}", post(handle))
            .with_state(st);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}/api"), calls)
    }

    pub fn methods(calls: &Calls) -> Vec<String> {
        calls.lock().unwrap().iter().map(|c| c.0.clone()).collect()
    }

    pub fn params(calls: &Calls, method: &str) -> HashMap<String, String> {
        calls
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|c| c.0 == method)
            .map(|c| c.1.clone())
            .unwrap_or_default()
    }
}

fn users_list() -> Value {
    json!({"ok": true, "members": [
        {"id": "U1", "name": "dana", "profile": {"display_name": "Dana"}},
        {"id": "U2", "name": "grok", "is_bot": true, "profile": {"real_name": "Grok"}}
    ]})
}

#[test]
fn the_unread_cursor_sits_one_microsecond_before_the_message() {
    assert_eq!(
        ts_just_before("1730800000.123456").as_deref(),
        Some("1730800000.123455")
    );
    // Borrow across the second boundary, keep six digits.
    assert_eq!(
        ts_just_before("1730800000.000000").as_deref(),
        Some("1730799999.999999")
    );
    assert_eq!(
        ts_just_before("1730800000.000010").as_deref(),
        Some("1730800000.000009")
    );
    assert_eq!(ts_just_before("nope"), None);
    assert_eq!(ts_just_before("1.1234567"), None);
    assert!(valid_ts("1730800000.123456") && valid_ts("1730800000"));
    assert!(!valid_ts("") && !valid_ts("12a.4"));
}

#[test]
fn times_parse_as_ts_relative_rfc3339_or_day() {
    let now = Utc.with_ymd_and_hms(2026, 11, 5, 12, 0, 0).unwrap();
    let base = now.timestamp();
    assert_eq!(
        parse_when("1730800000.123456", now).unwrap(),
        "1730800000.123456"
    );
    assert_eq!(
        parse_when("2h", now).unwrap(),
        format!("{}.000000", base - 7200)
    );
    assert_eq!(
        parse_when("1d", now).unwrap(),
        format!("{}.000000", base - 86400)
    );
    assert_eq!(
        parse_when("2026-11-05T11:00:00Z", now).unwrap(),
        format!("{}.000000", base - 3600)
    );
    assert_eq!(
        parse_when("2026-11-05", now).unwrap(),
        format!("{}.000000", base - 12 * 3600)
    );
    assert!(parse_when("yesterday", now).is_err());
    // The search modifier is exclusive + day-granular: the day before.
    assert_eq!(search_after(&format!("{base}.000000")), "after:2026-11-04");
    assert_eq!(ts_time("1730800000.5"), "2024-11-05T09:46:40Z");
}

#[test]
fn targets_classify_ids_handles_and_names() {
    assert_eq!(
        classify_target("C0123ABCD"),
        Target::Conversation("C0123ABCD")
    );
    assert_eq!(
        classify_target("D0123ABCD"),
        Target::Conversation("D0123ABCD")
    );
    assert_eq!(classify_target("U0123ABCD"), Target::Member("U0123ABCD"));
    assert_eq!(classify_target("@U0123ABCD"), Target::Member("U0123ABCD"));
    assert_eq!(classify_target("@grok"), Target::Handle("grok"));
    assert_eq!(classify_target("#general"), Target::Name("general"));
    // A lower-case word is a channel name, not an id.
    assert_eq!(classify_target("general"), Target::Name("general"));
}

#[test]
fn messages_resolve_mentions_and_keep_authors() {
    let names = Names::from_users(&crate::slack_directory::parse_users(&users_list()));
    let m = parse_message(
        &json!({"ts": "1730800000.000100", "user": "U1",
            "text": "hey <@U2> see <https://x.dev|docs> &amp; <#C1|ops>",
            "thread_ts": "1730800000.000100", "reply_count": 2,
            "reactions": [{"name": "eyes", "count": 1}],
            "files": [{"name": "log.txt"}]}),
        &names,
    );
    assert_eq!(m.user_name.as_deref(), Some("Dana"));
    assert_eq!(m.text, "hey @Grok see docs (https://x.dev) & #ops");
    assert_eq!(m.reply_count, Some(2));
    assert_eq!(m.reactions[0].name, "eyes");
    assert_eq!(m.files, ["log.txt"]);
    // A bot post with no user still has an author.
    let b = parse_message(
        &json!({"ts": "1.000001", "bot_profile": {"name": "Grok"}, "text": "<@U9>"}),
        &names,
    );
    assert_eq!(b.user_name.as_deref(), Some("Grok"));
    assert_eq!(b.text, "@U9");
    // Long text is capped.
    let long = "x".repeat(MAX_TEXT_CHARS + 10);
    let l = parse_message(&json!({"ts": "1.0", "text": long}), &names);
    assert_eq!(l.text.chars().count(), MAX_TEXT_CHARS + 1);
}

#[test]
fn conversations_and_hits_name_dms_by_person() {
    let names = Names::from_users(&crate::slack_directory::parse_users(&users_list()));
    let im = parse_conversation(&json!({"id": "D1", "is_im": true, "user": "U2"}), &names).unwrap();
    assert_eq!((im.kind, im.name.as_str()), ("im", "Grok"));
    let pv = parse_conversation(
        &json!({"id": "G1", "name": "ops", "is_private": true, "is_member": true}),
        &names,
    )
    .unwrap();
    assert_eq!((pv.kind, pv.is_member), ("private", Some(true)));
    let h = parse_hit(
        &json!({"channel": {"id": "D1", "is_im": true, "name": "U2"}, "ts": "1.000002",
            "user": "U1", "text": "ping", "permalink": "https://x/p1"}),
        &names,
    )
    .unwrap();
    assert_eq!(
        (h.kind, h.channel_name.as_str(), h.user_name.as_deref()),
        ("im", "Grok", Some("Dana"))
    );
}

#[test]
fn slack_errors_map_to_otto_statuses_and_name_the_scope() {
    let e = decode_body(
        "conversations.history",
        json!({"ok": false, "error": "missing_scope", "needed": "im:history"}),
    )
    .unwrap_err();
    assert_eq!(
        e.to_string(),
        "slack conversations.history: missing_scope (the token needs the `im:history` scope)"
    );
    assert!(matches!(e.into_otto(), otto_core::Error::Upstream(_)));
    let nf = SlackError::new("chat.postMessage", "channel_not_found");
    assert!(matches!(nf.into_otto(), otto_core::Error::NotFound(_)));
    let bad = SlackError::new("reactions.add", "already_reacted");
    assert!(matches!(bad.into_otto(), otto_core::Error::Invalid(_)));
    // Debug never prints the token.
    let api = SlackApi::new("xoxp-secret-token");
    assert!(!format!("{api:?}").contains("xoxp"));
}

#[tokio::test]
async fn post_then_mark_unread_round_trips_through_the_web_api() {
    let (base, calls) = mock::serve(
        vec![
            ("users.list", users_list()),
            (
                "chat.postMessage",
                json!({"ok": true, "channel": "C9", "ts": "1730800000.000200"}),
            ),
            (
                "chat.getPermalink",
                json!({"ok": true, "permalink": "https://acme.slack.com/p1"}),
            ),
            ("conversations.mark", json!({"ok": true})),
        ],
        "chat:write,channels:write",
    )
    .await;
    let api = SlackApi::with_base("xoxp-1", &base);
    let posted = api
        .post_message("C9", "Daily summary", Some("1730800000.000100"))
        .await
        .unwrap();
    assert_eq!(posted.ts, "1730800000.000200");
    assert_eq!(
        posted.permalink.as_deref(),
        Some("https://acme.slack.com/p1")
    );
    let p = mock::params(&calls, "chat.postMessage");
    assert_eq!(p["text"], "Daily summary");
    assert_eq!(p["thread_ts"], "1730800000.000100");
    let cursor = ts_just_before(&posted.ts).unwrap();
    api.mark("C9", &cursor).await.unwrap();
    assert_eq!(
        mock::params(&calls, "conversations.mark")["ts"],
        "1730800000.000199"
    );
    // The token rides the Authorization header, never the form.
    for (_, form, bearer) in calls.lock().unwrap().iter() {
        assert_eq!(bearer, "xoxp-1");
        assert!(!form.values().any(|v| v.contains("xoxp")));
    }
}

#[tokio::test]
async fn history_is_returned_oldest_first_and_threads_use_replies() {
    let (base, calls) = mock::serve(
        vec![
            ("users.list", users_list()),
            (
                "conversations.history",
                json!({"ok": true, "has_more": true,
                    "messages": [{"ts": "3.000000", "user": "U1", "text": "c"},
                                 {"ts": "1.000000", "user": "U2", "text": "a"}],
                    "response_metadata": {"next_cursor": "dXNlcjpV"}}),
            ),
            (
                "conversations.replies",
                json!({"ok": true, "messages": [{"ts": "1.000000", "text": "root"}]}),
            ),
        ],
        "",
    )
    .await;
    let api = SlackApi::with_base("xoxb-1", &base);
    let page = api
        .messages(&HistoryReq {
            channel: "C1".into(),
            oldest: Some("0.5".into()),
            limit: 2,
            ..Default::default()
        })
        .await
        .unwrap();
    let ts: Vec<_> = page.messages.iter().map(|m| m.ts.as_str()).collect();
    assert_eq!(ts, ["1.000000", "3.000000"]);
    assert_eq!(page.next_cursor.as_deref(), Some("dXNlcjpV"));
    assert!(page.has_more);
    assert_eq!(page.messages[0].user_name.as_deref(), Some("Grok"));
    assert_eq!(
        mock::params(&calls, "conversations.history")["oldest"],
        "0.5"
    );
    api.messages(&HistoryReq {
        channel: "C1".into(),
        thread_ts: Some("1.000000".into()),
        limit: 10,
        ..Default::default()
    })
    .await
    .unwrap();
    assert_eq!(
        mock::params(&calls, "conversations.replies")["ts"],
        "1.000000"
    );
}

#[tokio::test]
async fn names_resolve_to_conversations_and_handles_open_dms() {
    let (base, calls) = mock::serve(
        vec![
            ("users.list", users_list()),
            (
                "users.conversations",
                json!({"ok": true, "channels": [
                    {"id": "C1", "name": "general", "is_member": true},
                    {"id": "D7", "is_im": true, "user": "U2"}]}),
            ),
            (
                "conversations.open",
                json!({"ok": true, "channel": {"id": "D7"}}),
            ),
        ],
        "",
    )
    .await;
    let api = SlackApi::with_base("xoxp-2", &base);
    assert_eq!(api.resolve_channel(true, "#general").await.unwrap(), "C1");
    assert_eq!(api.resolve_channel(true, "@grok").await.unwrap(), "D7");
    assert_eq!(mock::params(&calls, "conversations.open")["users"], "U2");
    let e = api.resolve_channel(true, "#nope").await.unwrap_err();
    assert_eq!(e.code, "channel_not_found");
    // As the person, the listing is users.conversations incl. DMs.
    let list = api
        .conversations(true, None, Some("grok"), 10)
        .await
        .unwrap();
    assert_eq!(list.conversations.len(), 1);
    assert_eq!(list.conversations[0].kind, "im");
    assert!(mock::params(&calls, "users.conversations")["types"].contains("im"));
}

#[tokio::test]
async fn auth_test_reads_identity_and_granted_scopes() {
    let (base, _) = mock::serve(
        vec![(
            "auth.test",
            json!({"ok": true, "user_id": "U1", "user": "dana", "team": "Acme",
                "team_id": "T1", "url": "https://acme.slack.com/"}),
        )],
        "search:read, im:history",
    )
    .await;
    let id = SlackApi::with_base("xoxp-3", &base)
        .auth_test()
        .await
        .unwrap();
    assert_eq!((id.team_id.as_str(), id.user.as_str()), ("T1", "dana"));
    assert_eq!(id.scopes, ["search:read", "im:history"]);
    assert!(id.bot_id.is_none());
}
