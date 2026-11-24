//! A long, real-shaped Claude session opens in the conversation view (the
//! 0.5.0 Chat switch answered 500 "transcript resource limit exceeded" on
//! the owner's own daily session). Generated, not copied: 60+ MB, 20k+
//! records, multi-hundred-KB tool results, images, sub-agents, compactions
//! and a final turn still running — see `long_session.rs`.
use axum::body::Body;
use axum::http::Request;
use axum::Router;
use otto_core::domain::SessionKind;
use otto_server::ServerCtx;
use otto_state::{DbPool, NewSession, SessionsRepo};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use tower::ServiceExt;

#[path = "../../otto-transcript/tests/support/long_session.rs"]
mod long_session;

async fn mem_pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

async fn get(app: &Router, path: &str, token: &str) -> (u16, Value) {
    let req = Request::builder()
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status().as_u16();
    let bytes = axum::body::to_bytes(resp.into_body(), 16 << 20)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

fn tool_ids(page: &Value) -> Vec<String> {
    page["turns"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|t| t["blocks"].as_array().cloned().unwrap_or_default())
        .filter(|b| b["kind"] == "tool_call")
        .map(|b| b["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn a_long_session_opens_on_its_recent_turns_and_pages_back() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dir = tmp.path().join("projects").join("-work-repo");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("5e55e55e-0000-4000-8000-000000000001.jsonl");
    let long = long_session::write_long_claude_session(&path, 60 << 20, 20_000);
    assert!(long.bytes >= 60 << 20 && long.records >= 20_000);
    eprintln!("generated {} bytes, {} records", long.bytes, long.records);

    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let owner = otto_state::UsersRepo::new(pool.clone())
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    let token = otto_rbac::AuthRepo::new(pool.clone())
        .issue(&owner.id)
        .await
        .unwrap();
    let workspace = ctx
        .workspaces
        .create("Long", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    let session = SessionsRepo::new(pool.clone())
        .create(NewSession {
            workspace_id: workspace.id.clone(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "long".into(),
            cwd: "/work/repo".into(),
            provider_session_id: Some("5e55e55e-0000-4000-8000-000000000001".into()),
            connection_id: None,
            created_by: owner.id.clone(),
            meta: Value::Null,
        })
        .await
        .unwrap();
    sqlx::query("UPDATE sessions SET transcript_path = ? WHERE id = ?")
        .bind(path.to_string_lossy().as_ref())
        .bind(&session.id)
        .execute(&pool)
        .await
        .unwrap();
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    let base = format!("/api/v1/sessions/{}/transcript", session.id);

    // The newest page: the still-running final turn's last tool calls.
    let (status, page) = get(&app, &format!("{base}?limit=60"), &token).await;
    assert_eq!(status, 200, "the conversation opens: {page}");
    assert!(page["has_earlier"].as_bool().unwrap());
    let ids = tool_ids(&page);
    let last = format!("toolu-{}-r119", long.exchanges);
    assert_eq!(ids.last(), Some(&last), "ends on the newest tool call");

    // Older turns page in by cursor until the final turn's prompt shows up,
    // every page strictly older than the one before and none overlapping.
    let mut seen: std::collections::HashSet<String> = page["turns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    let mut cursor = page["cursor"].as_str().unwrap().to_string();
    let mut found_prompt = false;
    for _ in 0..6 {
        let (status, older) = get(&app, &format!("{base}?limit=60&before={cursor}"), &token).await;
        assert_eq!(status, 200, "an older page opens: {older}");
        for t in older["turns"].as_array().unwrap() {
            assert!(
                seen.insert(t["id"].as_str().unwrap().to_string()),
                "turn paged twice: {}",
                t["id"]
            );
            found_prompt |= t["blocks"].to_string().contains(&long.last_prompt);
        }
        let next = older["cursor"].as_str().unwrap().to_string();
        assert!(next.parse::<usize>().unwrap() < cursor.parse::<usize>().unwrap());
        cursor = next;
        if found_prompt {
            break;
        }
    }
    assert!(found_prompt, "the final turn's prompt is a few pages back");

    // The 2 MB tool result: shortened in the page, its block fetchable alone.
    let (status, block) = get(&app, &format!("{base}/tool/{}", long.big_tool_id), &token).await;
    assert_eq!(status, 200, "the big tool call opens: {block}");
    assert_eq!(block["result"]["truncated"], true);

    // What is still refused says so in words with its own status, never 500:
    // a sub-agent transcript holding one record over the 16 MiB record cap.
    let subagents = path.with_extension("").join("subagents");
    let huge = format!(
        "{{\"type\":\"user\",\"message\":{{\"content\":\"{}\"}}}}\n",
        "z".repeat(17 << 20)
    );
    std::fs::write(subagents.join("agent-huge.jsonl"), huge).unwrap();
    let (status, body) = get(&app, &format!("{base}?sub=huge"), &token).await;
    assert_eq!(status, 413, "{body}");
    assert_eq!(body["code"], "payload_too_large");
    assert!(
        body["message"]
            .as_str()
            .unwrap()
            .contains("too large to show here"),
        "{body}"
    );
}
