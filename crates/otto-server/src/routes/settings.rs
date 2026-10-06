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
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(body): Json<Map<String, Value>>,
) -> ApiResult<Json<Map<String, Value>>> {
    require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
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
    /// An offline compaction will run at the next daemon start (requested via
    /// `at: "next_restart"`, or automatic for a large fragmented file).
    pub compaction_scheduled: bool,
    /// Rough duration of that offline compaction (ms) — it delays that start
    /// by about this much and stalls no write.
    pub estimated_offline_ms: u64,
}

/// Body of `POST /admin/db/compact`: must carry `confirm: true` — the UI
/// sends it only after a `confirmer.ask` that says the DB locks meanwhile.
#[derive(serde::Deserialize)]
pub struct CompactReq {
    #[serde(default)]
    pub confirm: bool,
    /// `now` (default): in-place rewrite, writes wait meanwhile.
    /// `next_restart`: offline copy-and-swap before the pool opens at the next
    /// start (no write stall). `cancel`: withdraw a `next_restart` request.
    #[serde(default)]
    pub at: CompactAt,
}

#[derive(serde::Deserialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CompactAt {
    #[default]
    Now,
    NextRestart,
    Cancel,
}

/// Response of `POST /admin/db/compact` with `at: "next_restart" | "cancel"`.
#[derive(serde::Serialize)]
pub struct CompactScheduled {
    pub compaction_scheduled: bool,
    pub estimated_offline_ms: u64,
}

fn db_file(ctx: &ServerCtx) -> std::path::PathBuf {
    ctx.data_dir.join("otto.db")
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
        compaction_scheduled: otto_state::maintenance::compaction_scheduled(&db_file(&ctx), &s),
        estimated_offline_ms: otto_state::maintenance::estimate_offline_compact_ms(&s),
    }))
}

/// `POST /admin/db/compact` — the one-time `auto_vacuum=INCREMENTAL` +
/// `VACUUM` rewrite (root only, explicit `confirm: true`, audited). Holds the
/// SQLite write lock for the duration; afterwards the hourly maintenance pass
/// reclaims free pages incrementally. Never run automatically.
pub async fn db_compact(
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(body): Json<CompactReq>,
) -> ApiResult<axum::response::Response> {
    use axum::response::IntoResponse;
    require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    if !body.confirm {
        return Err(
            otto_core::Error::Invalid("compact requires an explicit confirm: true".into()).into(),
        );
    }
    if body.at != CompactAt::Now {
        // A marker file next to otto.db; the work happens at the next start.
        let db = db_file(&ctx);
        let next = body.at == CompactAt::NextRestart;
        {
            let db = db.clone();
            tokio::task::spawn_blocking(move || {
                if next {
                    otto_state::maintenance::request_compaction(&db)
                } else {
                    otto_state::maintenance::cancel_compaction_request(&db);
                    Ok(())
                }
            })
            .await
            .map_err(|e| otto_core::Error::Internal(format!("compaction request task: {e}")))??;
        }
        let s = otto_state::maintenance::stats(&ctx.pool).await?;
        ctx.audit(NewAuditEntry {
            user_id: Some(user.id.clone()),
            action: if next {
                "db.compact_scheduled".into()
            } else {
                "db.compact_cancelled".into()
            },
            target: None,
            detail: None,
            ip: None,
        })
        .await;
        return Ok(Json(CompactScheduled {
            compaction_scheduled: otto_state::maintenance::compaction_scheduled(&db, &s),
            estimated_offline_ms: otto_state::maintenance::estimate_offline_compact_ms(&s),
        })
        .into_response());
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
    Ok(Json(report).into_response())
}

// --- Secret store status + "Secure secrets…" (p-daemon SEC-1) ---------------

fn secrets_control() -> ApiResult<std::sync::Arc<otto_keychain::SecretsControl>> {
    otto_keychain::control::global().ok_or_else(|| {
        otto_core::Error::NotFound("this daemon has no managed secret store".into()).into()
    })
}

/// `GET /admin/secrets/status` — active secret backend, whether plaintext
/// `secrets.json` is in use (entry COUNT only) and the master-key state
/// (`locked` while a Keychain prompt waits). Root only. Never returns values
/// or key names.
pub async fn secrets_status(
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<otto_keychain::SecretsStatus>> {
    require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    let c = secrets_control()?;
    // Counting entries reads the plaintext file — keep it off the worker.
    let st = tokio::task::spawn_blocking(move || c.status())
        .await
        .map_err(|e| otto_core::Error::Internal(format!("secrets status task: {e}")))?;
    Ok(Json(st))
}

/// Response of `POST /admin/secrets/reset-store`.
#[derive(serde::Serialize)]
pub struct ResetSecretStoreResp {
    /// Where the unreadable `secrets.enc` was moved (kept, never deleted);
    /// `null` when there was no file.
    pub set_aside: Option<String>,
}

/// `POST /admin/secrets/reset-store` — recovery for an orphaned encrypted
/// store (S7-305): its Keychain master key is gone (or the file no longer
/// decrypts), so every secret save 409s. Moves `secrets.enc` aside to
/// `secrets.enc.orphaned-<secs>` (restoring the old key makes it readable
/// again) so new secrets can be saved. Root + human + explicit `confirm`;
/// audited. 409 when the store is readable or not encrypted.
pub async fn secrets_reset_store(
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(body): Json<SecureSecretsReq>,
) -> ApiResult<Json<ResetSecretStoreResp>> {
    require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    if !body.confirm {
        return Err(otto_core::Error::Invalid(
            "resetting the secret store requires an explicit confirm: true".into(),
        )
        .into());
    }
    let c = secrets_control()?;
    let res = tokio::task::spawn_blocking(move || c.reset_store())
        .await
        .map_err(|e| otto_core::Error::Internal(format!("secrets reset task: {e}")))?;
    let set_aside = res
        .as_ref()
        .ok()
        .and_then(|p| p.as_ref())
        .map(|p| p.to_string_lossy().into_owned());
    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: if res.is_ok() {
            "secrets.reset_store".into()
        } else {
            "secrets.reset_store_failed".into()
        },
        target: None,
        detail: Some(match &res {
            Ok(_) => serde_json::json!({ "set_aside": set_aside }),
            Err(e) => serde_json::json!({ "error": e.to_string() }),
        }),
        ip: None,
    })
    .await;
    res?;
    Ok(Json(ResetSecretStoreResp { set_aside }))
}

/// Body of `POST /admin/secrets/secure`: `confirm: true` is required — the UI
/// sends it only after a `confirmer.ask` that explains the Keychain prompt.
#[derive(serde::Deserialize)]
pub struct SecureSecretsReq {
    #[serde(default)]
    pub confirm: bool,
}

/// `POST /admin/secrets/secure` — migrate plaintext `secrets.json` into the
/// encrypted store (master key in the Keychain): every entry is verified to
/// read back before the plaintext is wiped and deleted; an encrypted backup is
/// kept until then. Root only, explicit `confirm`, audited (counts only).
/// NEVER run automatically — the Keychain may prompt, and an unattended boot
/// must not block on that. 409 if already encrypted / running / the Keychain
/// is locked (`502`, nothing changed).
pub async fn secrets_secure(
    State(ctx): State<ServerCtx>,
    auth: crate::auth::CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(body): Json<SecureSecretsReq>,
) -> ApiResult<Json<otto_keychain::control::MigrationReport>> {
    require_root(&user)?;
    crate::auth::require_human(&auth.0)?;
    if !body.confirm {
        return Err(otto_core::Error::Invalid(
            "securing secrets requires an explicit confirm: true".into(),
        )
        .into());
    }
    let c = secrets_control()?;
    let res = tokio::task::spawn_blocking(move || c.migrate_to_encrypted())
        .await
        .map_err(|e| otto_core::Error::Internal(format!("secrets migration task: {e}")))?;
    let detail = match &res {
        Ok(r) => serde_json::to_value(r).ok(),
        Err(e) => Some(serde_json::json!({ "error": e.to_string() })),
    };
    ctx.audit(NewAuditEntry {
        user_id: Some(user.id.clone()),
        action: if res.is_ok() {
            "secrets.secure".into()
        } else {
            "secrets.secure_failed".into()
        },
        target: None,
        detail,
        ip: None,
    })
    .await;
    Ok(Json(res?))
}

#[cfg(test)]
mod secrets_reset_tests {
    use super::*;
    use otto_core::auth::AuthContext;

    fn person(is_root: bool) -> otto_core::domain::User {
        otto_core::domain::User {
            id: "u-reset".into(),
            username: "u-reset".into(),
            display_name: "u-reset".into(),
            is_root,
            disabled: false,
            created_at: chrono::Utc::now(),
        }
    }

    fn auth(user: &otto_core::domain::User, agent: bool) -> crate::auth::CurrentAuthContext {
        crate::auth::CurrentAuthContext(AuthContext {
            real_user: user.clone(),
            effective_user: user.clone(),
            scope: None,
            mcp_only: false,
            mcp_scope: None,
            mcp_internal: false,
            mcp_session_id: None,
            managed_session_id: agent.then(|| "agent-session".into()),
        })
    }

    /// S7-305: the reset-store recovery is root + a person's own credential:
    /// a non-root user and a root-owned AGENT token are both refused (403)
    /// before the secret store is touched.
    #[tokio::test]
    async fn reset_store_refuses_non_root_and_agent_tokens() {
        let pool = otto_state::db::test_pool().await;
        let tmp = tempfile::tempdir().unwrap();
        let ctx = ServerCtx::for_tests(&pool, tmp.path()).await;
        for (user, agent) in [(person(false), false), (person(true), true)] {
            let err = secrets_reset_store(
                State(ctx.clone()),
                auth(&user, agent),
                CurrentUser(user.clone()),
                Json(SecureSecretsReq { confirm: true }),
            )
            .await
            .map(|_| ())
            .unwrap_err();
            assert!(
                matches!(err.0, otto_core::Error::Forbidden(_)),
                "root={} agent={agent}: {:?}",
                user.is_root,
                err.0
            );
        }
        // A person's root credential without `confirm` is a 400, never a reset.
        let root = person(true);
        let err = secrets_reset_store(
            State(ctx.clone()),
            auth(&root, false),
            CurrentUser(root.clone()),
            Json(SecureSecretsReq { confirm: false }),
        )
        .await
        .map(|_| ())
        .unwrap_err();
        assert!(matches!(err.0, otto_core::Error::Invalid(_)), "{:?}", err.0);
    }
}
