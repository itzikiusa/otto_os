//! Axum handlers for IDE Live Database Watch & Component-Test Watcher
//! (`/api/v1/ide/db-watch/detect` and the watch engine: `…/objects` and
//! `…/watches`; engine in `db_watch/watch.rs`).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::{Extension, Json};
use otto_core::auth::{AuthUser, BoxFuture};
use otto_core::domain::User;
use otto_core::event::Event;
use otto_core::{Error, Id, Result};

use super::{require_root_user, ApiResult};
use crate::db_watch::detect::detect_project_db_access;
use crate::db_watch::watch::{bind_debug, bind_run, Checkpoint, Watch, WatchHost, WatchTarget};
use crate::db_watch::{
    DbWatchExecOutcome, DbWatchOp, DbWatchQueryExec, DbWatchQueryTarget, IdeDbEngine, IdeDbWatch,
    IdeDbWatchBind, IdeDbWatchCheckpointReason, IdeDbWatchDetectReq, IdeDbWatchDetectResp,
    IdeDbWatchListQuery, IdeDbWatchListResp, IdeDbWatchObject, IdeDbWatchObjectsReq,
    IdeDbWatchObjectsResp, IdeDbWatchReadQuery, IdeDbWatchReadResp, IdeDbWatchSnapshotNowReq,
    IdeDbWatchStartReq, IdeDbWatchTargetInfo, IdeDbWatchTargetRef, IdeDbWatchUpdateReq,
    IdeDetectedDbObject, IdeEphemeralSource,
};
use crate::target::ResolvedTarget;
use crate::types::{IdeOkResp, IdeTargetKind, IdeTargetSpec};
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
        root: req.root.clone(),
        connection_id: ssh_conn_id,
        additional_roots: req.additional_roots.clone(),
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

    // The paused session's frames (top first) rank the code path.
    let stack_files: Vec<String> = match req.debug_session_id.as_deref() {
        Some(id) => {
            ctx.ide().authorize_debug_session(&user, id)?;
            let snap = ctx.ide().dap.get_session(id)?;
            snap.stack_frames
                .into_iter()
                .filter_map(|f| f.path)
                .collect()
        }
        None => Vec::new(),
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
                    &stack_files,
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

    let mut detected_objects: Vec<IdeDetectedDbObject> = detected_objects
        .into_iter()
        .map(|access| IdeDetectedDbObject {
            access,
            verified: None,
        })
        .collect();
    if let Some(target_ref) = &req.target {
        let target = resolve_watch_target(&ctx, &user, target_ref).await?;
        let listed = list_objects(&ctx, &user, &target, req.database.clone()).await?;
        for obj in &mut detected_objects {
            let a = &obj.access;
            // The SQL engine of a name is a guess (DAOs mix MySQL and
            // ClickHouse): check every SQL name against a SQL target.
            let sql = |e: IdeDbEngine| !matches!(e, IdeDbEngine::Mongodb | IdeDbEngine::Redis);
            let comparable =
                a.engine == target.info.engine || (sql(a.engine) && sql(target.info.engine));
            if comparable
                && a.engine != IdeDbEngine::Redis
                && target.info.engine != IdeDbEngine::Redis
            {
                let name = a.object_name.trim();
                obj.verified = Some(listed.objects.iter().any(|o| o.eq_ignore_ascii_case(name)));
            }
        }
    }

    Ok(Json(IdeDbWatchDetectResp {
        detected_objects,
        saved_connections,
        ephemeral_targets,
    }))
}

// ── Watch engine routes ─────────────────────────────────────────────────────

/// The daemon side of [`WatchHost`]: the read-only executor + the event bus.
struct CtxHost<S>(S);

impl<S: IdeCtx> WatchHost for CtxHost<S> {
    fn exec<'a>(
        &'a self,
        user: &'a User,
        exec: DbWatchQueryExec,
    ) -> BoxFuture<'a, Result<DbWatchExecOutcome>> {
        self.0.execute_db_watch_query(user, exec)
    }

    fn emit(&self, event: Event) {
        if let Some(tx) = self.0.events() {
            let _ = tx.send(event);
        }
    }
}

fn host<S: IdeCtx>(ctx: &S) -> Arc<dyn WatchHost> {
    Arc::new(CtxHost(ctx.clone()))
}

/// A watch reads only a LOCAL project root the caller may open.
async fn authorize_root<S: IdeCtx>(ctx: &S, user: &User, root: &str) -> Result<()> {
    let spec = IdeTargetSpec::local(root);
    match ctx.ide().targets.resolve(ctx, user, &spec).await? {
        ResolvedTarget::Local { .. } => Ok(()),
        ResolvedTarget::Ssh { .. } => {
            Err(Error::Invalid("DB Watch works on local projects".into()))
        }
    }
}

/// Saved: one the caller can see. Ephemeral / auto: a person with root, and
/// `auto` never picks a saved connection.
async fn resolve_watch_target<S: IdeCtx>(
    ctx: &S,
    user: &User,
    target: &IdeDbWatchTargetRef,
) -> Result<WatchTarget> {
    let ephemeral = |t: crate::db_watch::IdeEphemeralDbTarget| -> Result<WatchTarget> {
        let (label, exec) = ctx
            .ide()
            .db_watch
            .resolve_target(&t.ephemeral_id, t.engine)?;
        Ok(WatchTarget {
            info: IdeDbWatchTargetInfo {
                kind: "ephemeral".into(),
                connection_id: None,
                ephemeral_id: Some(t.ephemeral_id),
                engine: t.engine,
                label,
                environment: None,
                local: true,
            },
            exec,
        })
    };
    match target {
        IdeDbWatchTargetRef::Saved { connection_id } => {
            let saved = ctx
                .list_saved_db_targets(user)
                .await?
                .into_iter()
                .find(|t| t.connection_id == *connection_id)
                .ok_or_else(|| Error::NotFound(format!("connection '{connection_id}'")))?;
            Ok(WatchTarget {
                info: IdeDbWatchTargetInfo {
                    kind: "saved".into(),
                    connection_id: Some(saved.connection_id.clone()),
                    ephemeral_id: None,
                    engine: saved.engine,
                    label: saved.name,
                    environment: saved.environment,
                    local: saved.local,
                },
                exec: DbWatchQueryTarget::Saved {
                    connection_id: Id::from(saved.connection_id.as_str()),
                },
            })
        }
        IdeDbWatchTargetRef::Ephemeral { ephemeral_id } => {
            require_root_user(user)?;
            let t = ctx
                .ide()
                .db_watch
                .public_target(ephemeral_id)
                .ok_or_else(|| {
                    Error::NotFound(format!(
                        "the local database '{ephemeral_id}' is no longer running"
                    ))
                })?;
            ephemeral(t)
        }
        IdeDbWatchTargetRef::Auto { engine } => {
            require_root_user(user)?;
            let seed = ctx.ide().active_local_pids();
            let mut found: Vec<_> = ctx
                .ide()
                .db_watch
                .discover_local(&seed)
                .await
                .into_iter()
                .filter(|t| t.engine == *engine)
                .collect();
            found.sort_by_key(|t| match t.source {
                IdeEphemeralSource::Testcontainers => 0,
                IdeEphemeralSource::Process => 1,
                IdeEphemeralSource::Container => 2,
            });
            let t = found.into_iter().next().ok_or_else(|| {
                Error::NotFound(format!(
                    "no local {} database is running: start the test or the debug session first",
                    engine.as_str()
                ))
            })?;
            ephemeral(t)
        }
    }
}

/// A prod-tagged target needs a PERSON's `confirm_production` each time.
fn check_production(target: &WatchTarget, confirm: bool) -> Result<()> {
    let prod = target.info.environment.as_deref() == Some("prod");
    if prod && !(confirm && !otto_core::auth::request_is_agent()) {
        return Err(Error::Forbidden(format!(
            "'{}' is a production database: a person signed in to Otto must confirm watching it",
            target.info.label
        )));
    }
    Ok(())
}

async fn list_objects<S: IdeCtx>(
    ctx: &S,
    user: &User,
    target: &WatchTarget,
    database: Option<String>,
) -> Result<DbWatchExecOutcome> {
    ctx.execute_db_watch_query(
        user,
        DbWatchQueryExec {
            target: target.exec.clone(),
            engine: target.info.engine,
            database,
            object_name: String::new(),
            statement: String::new(),
            limit: 1,
            op: DbWatchOp::ListObjects,
        },
    )
    .await
}

pub(super) async fn db_watch_objects<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeDbWatchObjectsReq>,
) -> ApiResult<Json<IdeDbWatchObjectsResp>> {
    authorize_root(&ctx, &user, &req.root).await?;
    let target = resolve_watch_target(&ctx, &user, &req.target).await?;
    let mut out = list_objects(&ctx, &user, &target, req.database).await?;
    let truncated = out.objects.len() > 500;
    out.objects.truncate(500);
    let kind = target.info.engine.object_kind();
    Ok(Json(IdeDbWatchObjectsResp {
        target: target.info,
        database: out.database,
        objects: out
            .objects
            .into_iter()
            .map(|name| IdeDbWatchObject { name, kind })
            .collect(),
        truncated,
    }))
}

/// Bind `watch` to the caller's debug session or run.
fn bind<S: IdeCtx>(
    ctx: &S,
    user: &User,
    watch: &Arc<Watch>,
    bind: Option<IdeDbWatchBind>,
) -> Result<()> {
    let Some(b) = bind.filter(|b| b.debug_session_id.is_some() || b.run_id.is_some()) else {
        watch.set_binding(None, None);
        return Ok(());
    };
    let task = if let Some(id) = b.debug_session_id.clone() {
        ctx.ide().authorize_debug_session(user, &id)?;
        let stops = ctx.ide().dap.watch_stops(&id)?;
        let c = ctx.clone();
        bind_debug(watch, host(ctx), stops, move || {
            c.ide()
                .dap
                .get_session(&id)
                .map(|s| (s.paused_file, s.paused_line))
                .unwrap_or_default()
        })
    } else {
        let id = b.run_id.clone().unwrap_or_default();
        let (initial, events) = ctx.ide().environments.subscribe_run_for_user(user, &id)?;
        let (c, u) = (ctx.clone(), user.clone());
        bind_run(watch, host(ctx), initial, events, move || {
            c.ide()
                .environments
                .get_run_for_user(&u, &id, Some(0))
                .ok()
                .map(|r| r.status)
        })
    };
    watch.set_binding(Some(b), Some(task));
    Ok(())
}

pub(super) async fn db_watch_start<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Json(req): Json<IdeDbWatchStartReq>,
) -> ApiResult<Json<IdeDbWatch>> {
    authorize_root(&ctx, &user, &req.root).await?;
    let target = resolve_watch_target(&ctx, &user, &req.target).await?;
    check_production(&target, req.confirm_production)?;
    let watch = Watch::new(&user, &req.root, target, req.database, req.items)?;
    ctx.ide().watches.insert(Arc::clone(&watch));
    let h = host(&ctx);
    watch
        .checkpoint(&h, Checkpoint::new(IdeDbWatchCheckpointReason::Start), true)
        .await;
    if let Err(e) = bind(&ctx, &user, &watch, req.bind) {
        let _ = ctx.ide().watches.remove(&user, &watch.id);
        return Err(e.into());
    }
    Ok(Json(watch.view()))
}

pub(super) async fn db_watch_list<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Query(q): Query<IdeDbWatchListQuery>,
) -> ApiResult<Json<IdeDbWatchListResp>> {
    Ok(Json(IdeDbWatchListResp {
        watches: ctx.ide().watches.list(&user, q.root.as_deref()),
    }))
}

fn flag(v: &Option<String>) -> bool {
    matches!(v.as_deref(), Some("1" | "true"))
}

pub(super) async fn db_watch_read<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Query(q): Query<IdeDbWatchReadQuery>,
) -> ApiResult<Json<IdeDbWatchReadResp>> {
    let watch = ctx.ide().watches.get(&user, &id)?;
    let for_agent = flag(&q.for_agent) || otto_core::auth::request_is_agent();
    Ok(Json(watch.read(q.compare, flag(&q.rows), for_agent)))
}

pub(super) async fn db_watch_snapshot_now<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeDbWatchSnapshotNowReq>,
) -> ApiResult<Json<IdeDbWatchReadResp>> {
    let watch = ctx.ide().watches.get(&user, &id)?;
    let ck = Checkpoint {
        label: req.label.map(|l| l.chars().take(80).collect()),
        ..Checkpoint::new(IdeDbWatchCheckpointReason::Manual)
    };
    watch.checkpoint(&host(&ctx), ck, true).await;
    let for_agent = req.for_agent || otto_core::auth::request_is_agent();
    Ok(Json(watch.read(req.compare, false, for_agent)))
}

pub(super) async fn db_watch_update<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
    Json(req): Json<IdeDbWatchUpdateReq>,
) -> ApiResult<Json<IdeDbWatch>> {
    let watch = ctx.ide().watches.get(&user, &id)?;
    check_production(&watch.target(), req.confirm_production)?;
    if let Some(items) = req.items {
        watch.set_items(items).await?;
        watch.baseline_new_items(&host(&ctx)).await;
    }
    if req.bind.is_some() {
        bind(&ctx, &user, &watch, req.bind)?;
    }
    Ok(Json(watch.view()))
}

pub(super) async fn db_watch_delete<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(id): Path<String>,
) -> ApiResult<Json<IdeOkResp>> {
    ctx.ide().watches.remove(&user, &id)?;
    Ok(Json(IdeOkResp { ok: true }))
}
