//! `ServerCtx::resolve_provider` — the one provider precedence chain:
//! requested → workspace default → global default → "claude".
use otto_core::Id;
use otto_server::test_support::mem_pool;
use otto_server::ServerCtx;
use otto_state::{SettingsRepo, DEFAULT_PROVIDER_KEY};
use serde_json::json;

async fn seed_ws(pool: &otto_state::DbPool, id: &str, settings: serde_json::Value) {
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at)
         VALUES (?, 'ws', '/tmp', ?, 0, '2026-01-01T00:00:00Z')",
    )
    .bind(id)
    .bind(settings.to_string())
    .execute(pool)
    .await
    .expect("seed workspace");
}

#[tokio::test]
async fn precedence_is_requested_then_workspace_then_global_then_claude() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
    seed_ws(&pool, "ws-set", json!({ "default_provider": "agy" })).await;
    seed_ws(&pool, "ws-unset", json!({})).await;
    let ws_set = ctx.workspaces.get(&Id::from("ws-set")).await.unwrap();
    let ws_unset = ctx.workspaces.get(&Id::from("ws-unset")).await.unwrap();

    // Nothing configured anywhere → built-in fallback.
    assert_eq!(ctx.resolve_provider(None, None).await.unwrap(), "claude");
    assert_eq!(
        ctx.resolve_provider(Some(&ws_unset), Some("  "))
            .await
            .unwrap(),
        "claude",
        "a blank request is 'unset', not a provider name"
    );

    SettingsRepo::new(pool.clone())
        .put(DEFAULT_PROVIDER_KEY, &json!("codex"))
        .await
        .unwrap();
    // Global default applies when the workspace has none (or there is no workspace).
    assert_eq!(ctx.resolve_provider(None, None).await.unwrap(), "codex");
    assert_eq!(
        ctx.resolve_provider(Some(&ws_unset), None).await.unwrap(),
        "codex"
    );
    // Workspace default beats the global default.
    assert_eq!(
        ctx.resolve_provider(Some(&ws_set), None).await.unwrap(),
        "agy"
    );
    assert_eq!(
        ctx.resolve_provider_for_ws(&Id::from("ws-set"), None)
            .await
            .unwrap(),
        "agy"
    );
    // An explicit request beats everything (trimmed).
    assert_eq!(
        ctx.resolve_provider(Some(&ws_set), Some(" shell "))
            .await
            .unwrap(),
        "shell"
    );
}

/// Sweep 2026-10-11 prefs S-5: only workflows skipped a provider excluded in
/// Settings; insights, canvas, DB assist, product and scheduled tasks could
/// still spawn a disabled default.
#[tokio::test]
async fn a_disabled_default_is_never_resolved() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
    seed_ws(&pool, "ws-codex", json!({ "default_provider": "codex" })).await;
    let ws = ctx.workspaces.get(&Id::from("ws-codex")).await.unwrap();
    SettingsRepo::new(pool.clone())
        .put(DEFAULT_PROVIDER_KEY, &json!("agy"))
        .await
        .unwrap();
    ctx.manager.providers().set_disabled(&["codex"]);
    assert_eq!(
        ctx.resolve_provider(Some(&ws), None).await.unwrap(),
        "agy",
        "a disabled workspace default falls through to the global one"
    );
    ctx.manager
        .providers()
        .set_disabled(&["codex", "agy", "claude"]);
    let picked = ctx.resolve_provider(Some(&ws), None).await.unwrap();
    assert!(
        !["codex", "agy", "claude"].contains(&picked.as_str()),
        "never a disabled provider, even the claude fallback: {picked}"
    );
    // An explicit request is the person's choice and is not filtered.
    assert_eq!(
        ctx.resolve_provider(Some(&ws), Some("codex"))
            .await
            .unwrap(),
        "codex"
    );
}

#[tokio::test]
async fn unknown_workspace_and_db_errors_surface_instead_of_falling_back() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
    assert!(ctx
        .resolve_provider_for_ws(&Id::from("missing"), None)
        .await
        .is_err());
    // An explicit request never needs the DB, even for an unknown workspace.
    assert_eq!(
        ctx.resolve_provider_for_ws(&Id::from("missing"), Some("codex"))
            .await
            .unwrap(),
        "codex"
    );
    // A broken settings store is an error, not a silent "claude".
    sqlx::query("DROP TABLE settings")
        .execute(&pool)
        .await
        .unwrap();
    assert!(ctx.resolve_provider(None, None).await.is_err());
    assert_eq!(
        ctx.resolve_provider_or_fallback(None, None, "test").await,
        "claude"
    );
}
