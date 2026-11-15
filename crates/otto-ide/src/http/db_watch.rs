//! Axum handlers for IDE Live Database Watch & Component-Test Watcher
//! (`/api/v1/ide/db-watch/detect` and `/api/v1/ide/db-watch/snapshot`).

use axum::extract::State;
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::{Error, Id};

use super::{require_root_user, ApiResult};
use crate::db_watch::detect::detect_project_db_access;
use crate::db_watch::diff::compute_snapshot_diff;
use crate::db_watch::{
    build_default_watch_query, clamp_watch_limit, DbWatchQueryExec, DbWatchQueryTarget,
    IdeDbEngine, IdeDbWatchDetectReq, IdeDbWatchDetectResp, IdeDbWatchSnapshotItem,
    IdeDbWatchSnapshotReq, IdeDbWatchSnapshotResp,
};
use crate::target::ResolvedTarget;
use crate::types::{IdeTargetKind, IdeTargetSpec};
use crate::IdeCtx;

pub(super) async fn db_watch_detect<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeDbWatchDetectReq>,
) -> ApiResult<Json<IdeDbWatchDetectResp>> {
    let ssh_conn_id = req
        .connection_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(Id::from);
    let spec = IdeTargetSpec {
        kind: if ssh_conn_id.is_some() {
            IdeTargetKind::Ssh
        } else {
            IdeTargetKind::Local
        },
        workspace_id: None,
        root: req.root,
        connection_id: ssh_conn_id,
        additional_roots: req.additional_roots,
        project_folders: Vec::new(),
    };
    let resolved = ctx.ide().targets.resolve(&ctx, &user, &spec).await?;
    let saved_connections = ctx.list_saved_db_targets(&user).await?;

    let has_postgres = saved_connections
        .iter()
        .any(|c| c.engine == IdeDbEngine::Postgres);
    let has_mysql = saved_connections
        .iter()
        .any(|c| c.engine == IdeDbEngine::Mysql);
    let default_sql = if has_postgres && !has_mysql {
        IdeDbEngine::Postgres
    } else {
        IdeDbEngine::Mysql
    };

    let (detected_objects, ephemeral_targets) = match resolved {
        ResolvedTarget::Local {
            root,
            additional_roots,
            ..
        } => {
            let active_file = req.active_file.clone();
            let detected = tokio::task::spawn_blocking(move || {
                detect_project_db_access(
                    &root,
                    &additional_roots,
                    active_file.as_deref(),
                    default_sql,
                )
            })
            .await
            .map_err(|e| Error::Internal(format!("db watch detect task failed: {e}")))?;

            let ephemeral = if otto_core::auth::root_authority(&user) {
                let seed_pids = ctx.ide().active_local_pids();
                ctx.ide().db_watch.discover_local(&seed_pids).await
            } else {
                Vec::new()
            };
            (detected, ephemeral)
        }
        ResolvedTarget::Ssh { .. } => (Vec::new(), Vec::new()),
    };

    Ok(Json(IdeDbWatchDetectResp {
        detected_objects,
        saved_connections,
        ephemeral_targets,
    }))
}

pub(super) async fn db_watch_snapshot<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeDbWatchSnapshotReq>,
) -> ApiResult<Json<IdeDbWatchSnapshotResp>> {
    let limit = clamp_watch_limit(req.limit);
    let (eph_label, target) = match req.target_kind.trim() {
        "saved" => {
            let raw_id = req
                .connection_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    Error::Invalid("connection_id is required for saved target".into())
                })?;
            (
                None,
                DbWatchQueryTarget::Saved {
                    connection_id: Id::from(raw_id),
                },
            )
        }
        "ephemeral" => {
            require_root_user(&user)?;
            let eph_id = req
                .ephemeral_id
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    Error::Invalid("ephemeral_id is required for ephemeral target".into())
                })?;
            let (label, resolved) = ctx.ide().db_watch.resolve_target(eph_id, req.engine)?;
            (Some(label), resolved)
        }
        other => {
            return Err(Error::Invalid(format!(
                "unsupported target_kind '{other}' (expected 'saved' or 'ephemeral')"
            ))
            .into());
        }
    };

    let custom = req
        .custom_query
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let (statement, is_custom_query) = match custom {
        Some(q) => (q.to_string(), true),
        None => (
            build_default_watch_query(
                req.engine,
                req.database.as_deref(),
                &req.object_name,
                limit,
            )?,
            false,
        ),
    };

    let outcome = ctx
        .execute_db_watch_query(
            &user,
            DbWatchQueryExec {
                target,
                engine: req.engine,
                database: req.database,
                object_name: req.object_name.clone(),
                statement: statement.clone(),
                is_custom_query,
                limit,
            },
        )
        .await?;

    let snapshot = IdeDbWatchSnapshotItem {
        columns: outcome.columns,
        rows: outcome.rows,
    };
    let diff = req
        .previous
        .as_ref()
        .map(|prev| compute_snapshot_diff(prev, &snapshot));

    Ok(Json(IdeDbWatchSnapshotResp {
        engine: req.engine,
        target_label: eph_label.unwrap_or(outcome.target_label),
        object_name: req.object_name,
        executed_query: statement,
        captured_at_ms: chrono::Utc::now().timestamp_millis(),
        duration_ms: outcome.duration_ms,
        snapshot,
        diff,
    }))
}
