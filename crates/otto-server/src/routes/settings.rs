//! Endpoints #57-58: daemon settings (root only). Shape: flat JSON object
//! `{ "<key>": <value_json>, ... }`.

use axum::extract::State;
use axum::Json;
use otto_state::{NewAuditEntry, SettingsRepo};
use serde_json::{Map, Value};

use crate::auth::{require_root, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// `GET /api/v1/settings`
pub async fn get_all(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Map<String, Value>>> {
    require_root(&user)?;
    Ok(Json(SettingsRepo::new(ctx.pool.clone()).all().await?))
}

/// `PUT /api/v1/settings` — upserts every key in the body, returns the full
/// settings object.
pub async fn put_all(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<Map<String, Value>>,
) -> ApiResult<Json<Map<String, Value>>> {
    require_root(&user)?;
    let repo = SettingsRepo::new(ctx.pool.clone());
    for (key, value) in &body {
        repo.put(key, value).await?;
    }
    // Custom providers apply immediately — no daemon restart needed.
    if let Some(value) = body.get("providers") {
        ctx.manager.providers().reload(Some(value));
    }
    // Excluded providers apply immediately: hide them from every picker without a
    // restart. A JSON array of names; anything else clears the exclusion.
    if let Some(value) = body.get("disabled_providers") {
        let names: Vec<String> = serde_json::from_value(value.clone()).unwrap_or_default();
        ctx.manager.providers().set_disabled(&names);
    }
    // The skip-permissions opt-out applies immediately too: rebuild the registry
    // against the current `providers` override with the new mode. New spawns pick
    // it up; running sessions are untouched.
    if let Some(value) = body.get("agent_skip_permissions") {
        let skip = value.as_bool().unwrap_or(true);
        let overrides = repo.get("providers").await.ok().flatten();
        ctx.manager
            .providers()
            .set_skip_permissions(skip, overrides.as_ref());
        ctx.audit(NewAuditEntry {
            user_id: Some(user.id.clone()),
            action: "agent_skip_permissions.toggle".into(),
            target: Some(if skip { "on" } else { "off" }.into()),
            detail: None,
            ip: None,
        })
        .await;
    }

    // Audit the change. The list of changed keys is the durable record; secret
    // values are deliberately NOT captured. The network listener is a setting,
    // so its toggle flows through here — give it a dedicated, easily-filtered
    // entry (with the new enabled/port) on top of the generic settings change.
    if let Some(listener) = body.get("network_listener") {
        let enabled = listener
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        ctx.audit(NewAuditEntry {
            user_id: Some(user.id.clone()),
            action: "network_listener.toggle".into(),
            target: Some(if enabled { "on" } else { "off" }.into()),
            detail: Some(listener.clone()),
            ip: None,
        })
        .await;
    }
    let mut keys: Vec<&String> = body.keys().collect();
    keys.sort();
    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "settings.change".into(),
        target: Some(
            keys.iter()
                .map(|k| k.as_str())
                .collect::<Vec<_>>()
                .join(","),
        ),
        detail: Some(serde_json::json!({ "keys": keys })),
        ip: None,
    })
    .await;

    Ok(Json(repo.all().await?))
}

// --- Database maintenance (14-daemon-perf P3) -------------------------------

/// Response of `GET /admin/db/stats`.
#[derive(serde::Serialize)]
pub struct DbStatsResp {
    pub size_bytes: i64,
    pub free_bytes: i64,
    /// 0 = none, 1 = full, 2 = incremental (compacted).
    pub auto_vacuum: i64,
    /// A compaction is running right now.
    pub compacting: bool,
}

/// Body of `POST /admin/db/compact`: must carry `confirm: true` — the UI
/// sends it only after a `confirmer.ask` that says the DB locks meanwhile.
#[derive(serde::Deserialize)]
pub struct CompactReq {
    #[serde(default)]
    pub confirm: bool,
}

/// One compaction at a time: a second request while one runs gets 409.
static COMPACTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `GET /admin/db/stats` — `otto.db` size, reclaimable free space and whether
/// it was already compacted (root only).
pub async fn db_stats(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<DbStatsResp>> {
    require_root(&user)?;
    let s = otto_state::maintenance::stats(&ctx.pool).await?;
    Ok(Json(DbStatsResp {
        size_bytes: s.size_bytes(),
        free_bytes: s.free_bytes(),
        auto_vacuum: s.auto_vacuum,
        compacting: COMPACTING.load(std::sync::atomic::Ordering::SeqCst),
    }))
}

/// `POST /admin/db/compact` — the one-time `auto_vacuum=INCREMENTAL` +
/// `VACUUM` rewrite (root only, explicit `confirm: true`, audited). Holds the
/// SQLite write lock for the duration; afterwards the hourly maintenance pass
/// reclaims free pages incrementally. Never run automatically.
pub async fn db_compact(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<CompactReq>,
) -> ApiResult<Json<otto_state::maintenance::CompactReport>> {
    require_root(&user)?;
    if !body.confirm {
        return Err(
            otto_core::Error::Invalid("compact requires an explicit confirm: true".into()).into(),
        );
    }
    use std::sync::atomic::Ordering;
    if COMPACTING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(otto_core::Error::Conflict("a compaction is already running".into()).into());
    }
    // Cleared on drop, so a cancelled request (client gone) can't wedge it.
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            COMPACTING.store(false, std::sync::atomic::Ordering::SeqCst);
        }
    }
    let _reset = Reset;
    let report = otto_state::maintenance::compact(&ctx.pool).await?;
    tracing::info!(
        "db compact: {} → {} bytes in {} ms",
        report.before_bytes,
        report.after_bytes,
        report.duration_ms
    );
    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: "db.compact".into(),
        target: None,
        detail: serde_json::to_value(report).ok(),
        ip: None,
    })
    .await;
    Ok(Json(report))
}
