//! Settings export/import and state backup/restore (C3).
//!
//! Four root-only endpoints:
//!
//!   GET  /settings/export  — full settings JSON with secrets EXCLUDED (redacted)
//!   POST /settings/import  — apply a subset of non-secret settings
//!   GET  /state/backup     — portable non-secret snapshot (settings + manifest)
//!   POST /state/restore    — restore non-secret settings from a backup
//!
//! Secrets never leave this trust boundary. Every mutation and the export reads
//! append an audit row via `record_action`.
//!
//! The `record_action` helper at the bottom of this module is re-used by
//! `settings.rs` and `auth_routes.rs` for the few additional high-value sites
//! we wire without touching cross-cutting call sites.

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use otto_state::{NewAuditEntry, SettingsRepo};

use crate::auth::{require_root, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// Route assembly
// ---------------------------------------------------------------------------

pub fn backup_routes() -> Router<ServerCtx> {
    Router::new()
        .route("/settings/export", get(export_settings))
        .route("/settings/import", post(import_settings))
        .route("/state/backup", get(state_backup))
        .route("/state/restore", post(state_restore))
}

// ---------------------------------------------------------------------------
// Secret-key filter
//
// Top-level setting keys whose names match any of these patterns are excluded
// from exports wholesale. Value-level heuristics in `otto_core::redact` catch
// embedded secrets in non-excluded values; this list handles settings whose
// values are Keychain refs or raw credentials that the heuristic might miss.
// ---------------------------------------------------------------------------

/// Returns `true` when a top-level settings key should be withheld from exports.
/// Import uses the same filter so a re-ingested file can never smuggle secrets in.
pub fn is_secret_key(key: &str) -> bool {
    let k = key.to_lowercase();
    k.contains("password")
        || k.contains("passwd")
        || k.contains("secret")
        || k.contains("token")
        || k.contains("api_key")
        || k.contains("apikey")
        || k.contains("access_key")
        || k.contains("private_key")
        || k.contains("credential")
        || k.contains("keychain")
        || k.contains("keyref")
        || k.contains("auth_token")
}

/// Run a redaction pass on a raw settings map and return the scrubbed output
/// plus the list of top-level keys that were excluded entirely.
fn scrub_settings(raw: Map<String, Value>) -> (Map<String, Value>, Vec<String>) {
    let mut out = Map::with_capacity(raw.len());
    let mut excluded = Vec::new();
    for (k, v) in raw {
        if is_secret_key(&k) {
            excluded.push(k);
            continue;
        }
        // Value-level: strip embedded JWTs, Bearer tokens, PEM blocks, emails
        // that might appear inside config blobs such as `pr_review`.
        let redacted = otto_core::redact::redact_json(&v);
        out.insert(k, redacted.value);
    }
    excluded.sort();
    (out, excluded)
}

// ---------------------------------------------------------------------------
// GET /settings/export
// ---------------------------------------------------------------------------

/// Response body for `GET /settings/export`.
#[derive(Debug, Serialize, Deserialize)]
pub struct SettingsExportResp {
    /// Scrubbed settings (no secrets, no Keychain refs).
    pub settings: Map<String, Value>,
    /// Top-level keys excluded because they matched the secret filter.
    pub excluded_keys: Vec<String>,
    /// Format version; bump when the shape changes.
    pub export_format: u32,
}

/// `GET /api/v1/settings/export` — daemon settings JSON with secrets excluded.
///
/// Root only. The response is safe to download and store outside the trust
/// boundary. An audit row is written (action `settings.export`).
pub async fn export_settings(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SettingsExportResp>> {
    require_root(&user)?;

    let raw = SettingsRepo::new(ctx.pool.clone()).all().await?;
    let (settings, excluded_keys) = scrub_settings(raw);

    record_action(
        &ctx,
        &user.id,
        "settings.export",
        None,
        Some(serde_json::json!({ "excluded_key_count": excluded_keys.len() })),
    )
    .await;

    Ok(Json(SettingsExportResp {
        settings,
        excluded_keys,
        export_format: 1,
    }))
}

// ---------------------------------------------------------------------------
// POST /settings/import
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct SettingsImportReq {
    /// Settings map to merge. Secret-keyed entries are rejected silently.
    pub settings: Map<String, Value>,
}

/// `POST /api/v1/settings/import` — merge non-secret settings.
///
/// Root only. Secret-keyed entries are rejected; Keychain refs are never
/// imported. Returns the full settings after the merge. Providers are reloaded
/// immediately if `providers` was in the payload, matching `PUT /settings`.
pub async fn import_settings(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SettingsImportReq>,
) -> ApiResult<Json<Map<String, Value>>> {
    require_root(&user)?;

    let repo = SettingsRepo::new(ctx.pool.clone());
    let mut accepted: Vec<String> = Vec::new();
    let mut rejected: Vec<String> = Vec::new();

    for (key, value) in &req.settings {
        if is_secret_key(key) {
            rejected.push(key.clone());
            continue;
        }
        repo.put(key, value).await?;
        accepted.push(key.clone());

        // Reload providers immediately — mirrors `PUT /settings`.
        if key == "providers" {
            ctx.manager.providers().reload(Some(value));
        }
    }

    accepted.sort();
    rejected.sort();

    if !rejected.is_empty() {
        tracing::info!(
            user = %user.id,
            "settings.import: rejected secret-keyed entries: {:?}",
            rejected
        );
    }

    record_action(
        &ctx,
        &user.id,
        "settings.import",
        None,
        Some(serde_json::json!({
            "accepted_keys": accepted,
            "rejected_secret_keys": rejected,
        })),
    )
    .await;

    Ok(Json(repo.all().await?))
}

// ---------------------------------------------------------------------------
// GET /state/backup
// ---------------------------------------------------------------------------

/// Non-sensitive live-state manifest included in the backup.
#[derive(Debug, Serialize, Deserialize)]
pub struct StateManifest {
    /// Non-sensitive display names of active (non-archived) workspaces.
    pub workspace_names: Vec<String>,
    /// Total non-archived workspace count.
    pub workspace_count: usize,
    /// Highest applied migration version number.
    pub migration_level: String,
    /// Daemon version at snapshot time.
    pub daemon_version: String,
    /// RFC3339 timestamp.
    pub snapshot_at: String,
}

/// Response body for `GET /state/backup`.
#[derive(Debug, Serialize, Deserialize)]
pub struct StateBackupResp {
    /// Scrubbed settings (same filtering as export).
    pub settings: Map<String, Value>,
    /// Top-level setting keys excluded by the secret filter.
    pub excluded_keys: Vec<String>,
    /// Live-state manifest.
    pub manifest: StateManifest,
    /// Format version of the backup envelope.
    pub backup_format: u32,
}

/// `GET /api/v1/state/backup` — portable non-secret snapshot. Root only.
///
/// Includes: scrubbed settings + a manifest (workspace names/counts, migration
/// level, daemon version, timestamp). Does NOT include raw DB rows, session data,
/// PTY output, Keychain secrets, or any file from disk.
pub async fn state_backup(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<StateBackupResp>> {
    require_root(&user)?;

    // Settings (scrubbed).
    let raw = SettingsRepo::new(ctx.pool.clone()).all().await?;
    let (settings, excluded_keys) = scrub_settings(raw);

    // Workspace manifest: non-archived names + count (via direct SQL on ctx.pool
    // — avoids needing WorkspacesRepo::list which isn't yet publicly surfaced).
    let ws_rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT name, archived FROM workspaces ORDER BY created_at ASC",
    )
    .fetch_all(&ctx.pool)
    .await
    .map_err(|e| {
        ApiError(otto_core::Error::Internal(format!("list workspaces: {e}")))
    })?;

    let workspace_names: Vec<String> = ws_rows
        .iter()
        .filter(|(_, archived)| *archived == 0)
        .map(|(name, _)| name.clone())
        .collect();
    let workspace_count = workspace_names.len();

    // Highest applied migration version from sqlx's internal tracking table.
    let migration_level: String = sqlx::query_scalar::<_, i64>(
        "SELECT COALESCE(MAX(version), 0) FROM _sqlx_migrations WHERE success = 1",
    )
    .fetch_one(&ctx.pool)
    .await
    .map(|v| v.to_string())
    .unwrap_or_else(|_| "unknown".to_string());

    let snapshot_at = chrono::Utc::now().to_rfc3339();

    record_action(
        &ctx,
        &user.id,
        "state.backup",
        None,
        Some(serde_json::json!({
            "snapshot_at": snapshot_at,
            "workspace_count": workspace_count,
        })),
    )
    .await;

    Ok(Json(StateBackupResp {
        settings,
        excluded_keys,
        manifest: StateManifest {
            workspace_names,
            workspace_count,
            migration_level,
            daemon_version: ctx.version.clone(),
            snapshot_at,
        },
        backup_format: 1,
    }))
}

// ---------------------------------------------------------------------------
// POST /state/restore
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct StateRestoreReq {
    /// A backup as produced by `GET /state/backup`.
    pub backup: StateBackupResp,
    /// Must be `true` — explicit confirmation guard (mirrors the DB write-gate).
    #[serde(default)]
    pub confirm: bool,
}

/// `POST /api/v1/state/restore` — restore non-secret settings from a backup.
///
/// Root only. Requires `{ "confirm": true }`. Merges `backup.settings` into
/// the live settings store exactly like `POST /settings/import`. Does NOT wipe
/// the DB, drop tables, delete sessions or workspaces, or overwrite secrets.
pub async fn state_restore(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<StateRestoreReq>,
) -> ApiResult<StatusCode> {
    require_root(&user)?;

    if !req.confirm {
        return Err(ApiError(otto_core::Error::Invalid(
            "state restore requires { \"confirm\": true }".into(),
        )));
    }

    let repo = SettingsRepo::new(ctx.pool.clone());
    let mut accepted: Vec<String> = Vec::new();

    for (key, value) in &req.backup.settings {
        if is_secret_key(key) {
            // Belt-and-suspenders: the backup should already be scrubbed.
            tracing::warn!(key = %key, "state.restore: rejected secret-keyed entry in backup");
            continue;
        }
        repo.put(key, value).await?;
        accepted.push(key.clone());

        if key == "providers" {
            ctx.manager.providers().reload(Some(value));
        }
    }

    accepted.sort();

    record_action(
        &ctx,
        &user.id,
        "state.restore",
        None,
        Some(serde_json::json!({
            "accepted_keys": accepted,
            "backup_format": req.backup.backup_format,
            "backup_daemon_version": req.backup.manifest.daemon_version,
            "backup_snapshot_at": req.backup.manifest.snapshot_at,
        })),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Shared action-audit helper
// ---------------------------------------------------------------------------

/// Write one security audit row. Best-effort — failures are logged by
/// [`ServerCtx::audit`] and never propagate to the caller.
///
/// Imported by `settings.rs` and `auth_routes.rs` as
/// `crate::routes::backup::record_action` for the few additional sites we wire
/// without touching cross-cutting call sites (git push, channel send, etc. —
/// those are left as a documented follow-up).
pub async fn record_action(
    ctx: &ServerCtx,
    user_id: &str,
    action: &str,
    target: Option<&str>,
    detail: Option<Value>,
) {
    ctx.audit(NewAuditEntry {
        user_id: Some(user_id.to_string()),
        action: action.to_string(),
        target: target.map(str::to_string),
        detail,
        ip: None,
    })
    .await;
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn secret_key_filter() {
        assert!(is_secret_key("github_token"));
        assert!(is_secret_key("api_key"));
        assert!(is_secret_key("password"));
        assert!(is_secret_key("client_secret"));
        assert!(is_secret_key("KEYCHAIN_REF"));
        assert!(!is_secret_key("default_provider"));
        assert!(!is_secret_key("pr_review"));
        assert!(!is_secret_key("network_listener"));
    }

    #[test]
    fn scrub_removes_secret_keys_and_redacts_values() {
        let raw: Map<String, Value> = serde_json::from_value(json!({
            "default_provider": "claude",
            "github_token": "ghp_supersecret",
            "pr_review": {
                "agents": [],
                "password": "also_secret"
            }
        }))
        .unwrap();

        let (scrubbed, excluded) = scrub_settings(raw);

        // Secret key is excluded at the top level.
        assert!(scrubbed.get("github_token").is_none());
        assert_eq!(excluded, vec!["github_token"]);

        // Non-secret key survives.
        assert_eq!(scrubbed["default_provider"], json!("claude"));

        // Nested secret sub-key is redacted by the redact_json pass.
        let pr = scrubbed["pr_review"].as_object().unwrap();
        assert_eq!(pr["password"], json!("[redacted]"));
    }
}
