//! Trust & Safety Center: the security audit log + a derived posture snapshot
//! (both root only). The audit log is written best-effort by `ServerCtx::audit`
//! at sensitive sites; this module only reads it. The posture summary derives
//! from the live listener, saved settings and the auth store — no new tables.

use axum::extract::{Query, State};
use axum::Json;
use otto_core::api::{
    evaluate_security_posture, AuditLogQuery, AuditLogResp, SecurityPostureFacts,
    SecurityPostureResp,
};
use otto_rbac::AuthRepo;
use otto_state::{AuditRepo, SettingsRepo};
use serde_json::Value;

use crate::auth::{require_root, CurrentUser};
use crate::error::ApiResult;
use crate::state::ServerCtx;

/// `GET /api/v1/audit-log?from=&to=&action=&user_id=&limit=&offset=` (root only)
/// — a filtered, paged page of audit entries (newest first) plus the total
/// matching the filters.
pub async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<AuditLogQuery>,
) -> ApiResult<Json<AuditLogResp>> {
    require_root(&user)?;
    let repo = AuditRepo::new(ctx.pool.clone());
    let entries = repo.list(&q).await?;
    let total = repo.count(&q).await?;
    Ok(Json(AuditLogResp { entries, total }))
}

/// `GET /api/v1/security-posture` (root only) — a snapshot the Trust & Safety
/// Center renders: network-listener state + bind port, loopback-only status,
/// and the count of active API tokens. Saved settings identify pending restarts.
pub async fn posture(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SecurityPostureResp>> {
    require_root(&user)?;

    // The network listener is stored as `{ "enabled": bool, "port": u16 }` (see
    // ottod's boot). These are desired settings, not proof of a live socket.
    let listener = SettingsRepo::new(ctx.pool.clone())
        .get("network_listener")
        .await?;
    let configured_enabled = listener
        .as_ref()
        .and_then(|v| v.get("enabled").and_then(Value::as_bool))
        .unwrap_or(false);
    let configured_port = listener
        .as_ref()
        .and_then(|v| v.get("port").and_then(Value::as_u64))
        .and_then(|p| u16::try_from(p).ok());
    let network_listener_port = crate::transport::network_listener_port();
    let network_listener = network_listener_port.is_some();
    let desired_port = configured_port.or_else(|| ctx.base_url.rsplit(':').next()?.parse().ok());
    let network_listener_restart_required = configured_enabled != network_listener
        || (configured_enabled && desired_port != network_listener_port);

    let active_api_tokens = AuthRepo::new(ctx.pool.clone())
        .count_active_api_tokens()
        .await?;

    let now = chrono::Utc::now().to_rfc3339();
    let active_shares: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM auth_sessions WHERE kind = 'share' AND expires_at > ?",
    )
    .bind(&now)
    .fetch_one(&ctx.pool)
    .await
    .unwrap_or(0);
    let active_phone_devices: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM phone_devices WHERE expires_at > ? AND idle_expires_at > ?",
    )
    .bind(&now)
    .bind(&now)
    .fetch_one(&ctx.pool)
    .await
    .unwrap_or(0);
    let page_count: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(&ctx.pool)
        .await
        .unwrap_or(0);
    let freelist_count: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(&ctx.pool)
        .await
        .unwrap_or(0);
    let journal_mode: Option<String> = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(&ctx.pool)
        .await
        .ok();
    let enabled_plugins = ctx.plugins.repo().list_enabled().await.unwrap_or_default();
    let plugin_sources: Vec<(String, String)> = enabled_plugins
        .iter()
        .map(|p| (p.slug.clone(), p.source.clone()))
        .collect();
    let data_dir = ctx.data_dir.clone();
    let fs_facts =
        crate::offload::blocking(move || probe_fs_posture(&data_dir, &plugin_sources)).await;
    let facts = SecurityPostureFacts {
        has_encrypted_secrets: fs_facts.has_encrypted_secrets,
        has_plaintext_secrets: fs_facts.has_plaintext_secrets,
        orphaned_secret_files: fs_facts.orphaned_secret_files,
        db_mode: fs_facts.db_mode,
        tls_dir_mode: fs_facts.tls_dir_mode,
        tls_key_mode: fs_facts.tls_key_mode,
        network_listener,
        network_listener_port,
        allowed_hosts_env: std::env::var("OTTO_ALLOWED_HOSTS").ok(),
        active_shares,
        active_phone_devices,
        active_api_tokens,
        enabled_plugins: enabled_plugins.len(),
        unrestricted_plugins: fs_facts.unrestricted_plugins,
        jira_cred_plugins: fs_facts.jira_cred_plugins,
        wal_bytes: fs_facts.wal_bytes,
        page_count,
        freelist_count,
        journal_mode,
    };
    let (overall, checks) = evaluate_security_posture(&facts);

    Ok(Json(SecurityPostureResp {
        network_listener,
        network_listener_port,
        loopback_only: !network_listener,
        network_listener_restart_required,
        active_api_tokens,
        overall,
        checked_at: now,
        checks,
    }))
}

#[derive(Default)]
struct FsPostureProbe {
    has_encrypted_secrets: bool,
    has_plaintext_secrets: bool,
    orphaned_secret_files: usize,
    db_mode: Option<u32>,
    tls_dir_mode: Option<u32>,
    tls_key_mode: Option<u32>,
    wal_bytes: u64,
    unrestricted_plugins: Vec<String>,
    jira_cred_plugins: Vec<String>,
}

#[allow(clippy::disallowed_methods)] // runs inside crate::offload::blocking
fn probe_fs_posture(
    data_dir: &std::path::Path,
    plugin_sources: &[(String, String)],
) -> FsPostureProbe {
    #[cfg(unix)]
    let mode_of = |p: &std::path::Path| -> Option<u32> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .ok()
            .map(|m| m.permissions().mode() & 0o777)
    };
    #[cfg(not(unix))]
    let mode_of = |_p: &std::path::Path| -> Option<u32> { None };

    let orphaned_secret_files = std::fs::read_dir(data_dir)
        .ok()
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name()
                        .to_str()
                        .is_some_and(|n| n.starts_with("secrets.enc.orphaned-"))
                })
                .count()
        })
        .unwrap_or(0);
    let mut unrestricted_plugins = Vec::new();
    let mut jira_cred_plugins = Vec::new();
    for (slug, src) in plugin_sources {
        let manifest_path = std::path::Path::new(src).join("otto-plugin.json");
        let caps = std::fs::read_to_string(&manifest_path)
            .ok()
            .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            .and_then(|v| v.get("capabilities").cloned())
            .and_then(|c| serde_json::from_value::<Vec<String>>(c).ok());
        match caps {
            None => unrestricted_plugins.push(slug.clone()),
            Some(list) => {
                if list.iter().any(|c| c == "jira:credentials") {
                    jira_cred_plugins.push(slug.clone());
                }
            }
        }
    }
    FsPostureProbe {
        has_encrypted_secrets: data_dir.join("secrets.enc").exists(),
        has_plaintext_secrets: data_dir.join("secrets.json").exists(),
        orphaned_secret_files,
        db_mode: mode_of(&data_dir.join("otto.db")),
        tls_dir_mode: mode_of(&data_dir.join("tls")),
        tls_key_mode: mode_of(&data_dir.join("tls").join("key.pem")),
        wal_bytes: std::fs::metadata(data_dir.join("otto.db-wal"))
            .map(|m| m.len())
            .unwrap_or(0),
        unrestricted_plugins,
        jira_cred_plugins,
    }
}
