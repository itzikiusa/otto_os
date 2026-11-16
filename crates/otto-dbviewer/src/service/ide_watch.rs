//! Read-only IDE Live Database Watch execution & connection discovery helpers.
//!
//! Reuses `DbViewerService`'s connection resolution, SSH tunnel cache, RBAC
//! (`Database:View` + workspace `Viewer` + enforced `ResourceAccess` +
//! `connections.owner_private`), `ensure_read_only` statement classifier, and
//! native read-only transaction wrappers (`START TRANSACTION READ ONLY`,
//! `BEGIN READ ONLY`, ClickHouse `readonly=2`) without pulling DB driver crates
//! into `otto-ide`.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use otto_core::auth::RoleChecker;
use otto_core::domain::{Capability, Connection, Feature, User, WorkspaceRole};
use otto_core::{Error, Id, Result};
use otto_state::{capability_for_role, GrantsRepo, SettingsRepo, WorkspacesRepo};
use serde_json::Value;

use super::{ensure_read_only, force_read_only_execution, mcp_query_timeout_ms, DbViewerService};
use crate::driver::Driver;
use crate::types::{Engine, NodeKind, NodePath, QueryRequest, ResolvedConfig, TlsConfig};

/// Max Redis keys enriched with full `TYPE` + `TTL` + preview per watch snapshot.
const REDIS_WATCH_ENRICH_CAP: usize = 50;

/// Metadata for a saved Otto DB connection visible to the IDE Database Watcher.
#[derive(Debug, Clone)]
pub struct SavedWatchTarget {
    pub connection_id: String,
    pub name: String,
    pub engine: Engine,
    pub host: String,
    pub port: u16,
    pub database: Option<String>,
    pub environment: String,
}

/// Resolved target endpoint for an IDE DB Watch snapshot query.
#[derive(Clone)]
pub enum WatchEndpoint {
    Saved {
        connection_id: Id,
    },
    EphemeralLoopback {
        port: u16,
        user: Option<String>,
        password: Option<String>,
        database: Option<String>,
    },
}

/// Read-only snapshot query request executed by [`DbViewerService::execute_watch_query`].
#[derive(Clone)]
pub struct WatchExecReq {
    pub endpoint: WatchEndpoint,
    pub engine: Engine,
    pub database: Option<String>,
    pub object_name: String,
    pub statement: String,
    pub is_custom_query: bool,
    pub limit: usize,
}

/// Tabular snapshot result returned to `otto-ide`.
#[derive(Debug, Clone)]
pub struct WatchExecOutcome {
    pub target_label: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub duration_ms: u64,
}

impl DbViewerService {
    /// List saved browsable DB connections visible to `user` across their workspaces + global.
    pub async fn list_saved_watch_targets(
        &self,
        roles: &Arc<dyn RoleChecker>,
        user: &User,
    ) -> Result<Vec<SavedWatchTarget>> {
        let pool = self.connections.pool();
        if !user.is_root
            && GrantsRepo::new(pool.clone())
                .check_global(
                    user,
                    Feature::Database,
                    Capability::View,
                    "Database view grant required",
                )
                .await
                .is_err()
        {
            return Ok(Vec::new());
        }

        let owner_private = SettingsRepo::new(pool.clone())
            .get("connections.owner_private")
            .await?
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let ws_repo = WorkspacesRepo::new(pool);
        let mut ws_ids: Vec<Id> = if user.is_root {
            ws_repo
                .list_all()
                .await?
                .into_iter()
                .map(|w| w.id)
                .collect()
        } else {
            ws_repo
                .list_for_user(&user.id)
                .await?
                .into_iter()
                .map(|(w, _)| w.id)
                .collect()
        };
        if ws_ids.is_empty() {
            // Still surface global (`workspace_id IS NULL`) connections.
            ws_ids.push(Id::default());
        }

        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for ws_id in ws_ids {
            for conn in self.connections.list_visible(&ws_id).await? {
                if !seen.insert(conn.id.clone()) {
                    continue;
                }
                let Some(engine) = Engine::from_kind(conn.kind) else {
                    continue;
                };
                if self
                    .check_watch_conn_access(roles, user, &conn, owner_private)
                    .await
                    .is_err()
                {
                    continue;
                }
                let Ok(parsed) = crate::config::parse(&conn, None) else {
                    continue;
                };
                out.push(SavedWatchTarget {
                    connection_id: conn.id.to_string(),
                    name: conn.name,
                    engine,
                    host: if parsed.config.host.is_empty() {
                        "127.0.0.1".into()
                    } else {
                        parsed.config.host
                    },
                    port: parsed.config.port,
                    database: parsed.config.database,
                    environment: conn.environment.as_str().to_string(),
                });
            }
        }
        Ok(out)
    }

    /// Execute a strictly read-only IDE DB Watch snapshot against a saved or
    /// local loopback ephemeral target.
    pub async fn execute_watch_query(
        &self,
        roles: &Arc<dyn RoleChecker>,
        user: &User,
        req: WatchExecReq,
    ) -> Result<WatchExecOutcome> {
        let started = Instant::now();
        let limit = req.limit.clamp(1, 150);
        match req.endpoint {
            WatchEndpoint::Saved { connection_id } => {
                let pool = self.connections.pool();
                if !user.is_root {
                    GrantsRepo::new(pool.clone())
                        .check_global(
                            user,
                            Feature::Database,
                            Capability::View,
                            "saved database watch requires the Database 'view' grant",
                        )
                        .await?;
                }
                let owner_private = SettingsRepo::new(pool)
                    .get("connections.owner_private")
                    .await?
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let conn = self.connections.get(&connection_id).await?;
                self.check_watch_conn_access(roles, user, &conn, owner_private)
                    .await?;
                let conn_engine = Engine::from_kind(conn.kind).ok_or_else(|| {
                    Error::Invalid(format!(
                        "connection '{}' is not a queryable database",
                        conn.name
                    ))
                })?;
                if conn_engine != req.engine {
                    return Err(Error::Invalid(format!(
                        "connection '{}' uses engine '{}', not '{}'",
                        conn.name,
                        conn_engine.as_str(),
                        req.engine.as_str()
                    )));
                }
                ensure_read_only(conn_engine, &req.statement)?;

                if conn_engine == Engine::Redis && !req.is_custom_query {
                    let redis_node = req
                        .database
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|d| {
                            if d.starts_with("kdb:") {
                                d.to_string()
                            } else {
                                format!("kdb:{d}")
                            }
                        })
                        .unwrap_or_else(|| "kdb:0".to_string());
                    let child = crate::access::child(Some(&redis_node));
                    let mut r = self
                        .resolve(&connection_id, &user.id, child.as_deref(), "db_browse")
                        .await?;
                    force_read_only_execution(&mut r.config.params);
                    let (columns, mut rows) = enrich_redis_prefix(
                        &*r.driver,
                        &r.config,
                        &redis_node,
                        &req.object_name,
                        limit,
                    )
                    .await?;
                    self.authorize(&connection_id, &user.id, Some(&redis_node), "db_browse")
                        .await?;
                    redact_rows(&mut rows);
                    return Ok(WatchExecOutcome {
                        target_label: conn.name,
                        columns,
                        rows,
                        duration_ms: started.elapsed().as_millis() as u64,
                    });
                }

                let node = match conn_engine {
                    Engine::Redis => Some("kdb:0".to_string()),
                    _ => req
                        .database
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(|d| format!("db:{d}")),
                };
                let qreq = QueryRequest {
                    statement: req.statement,
                    max_rows: Some(limit),
                    node,
                    timeout_ms: Some(mcp_query_timeout_ms(Some(10_000))),
                    ..QueryRequest::default()
                };
                let res = self.run_read_only(&connection_id, &user.id, &qreq).await?;
                Ok(WatchExecOutcome {
                    target_label: conn.name,
                    columns: res.columns.into_iter().map(|c| c.name).collect(),
                    rows: res.rows,
                    duration_ms: res
                        .stats
                        .duration_ms
                        .max(started.elapsed().as_millis() as u64),
                })
            }
            WatchEndpoint::EphemeralLoopback {
                port,
                user: db_user,
                password,
                database: target_db,
            } => {
                if !otto_core::auth::root_authority(user) {
                    return Err(otto_core::auth::root_refusal());
                }
                ensure_read_only(req.engine, &req.statement)?;
                let effective_db = req
                    .database
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .or_else(|| {
                        target_db
                            .as_deref()
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                    });
                let mut cfg = ResolvedConfig {
                    lifecycle: None,
                    engine: req.engine,
                    host: "127.0.0.1".into(),
                    port,
                    user: db_user,
                    password,
                    database: effective_db.clone(),
                    tls: TlsConfig::default(),
                    params: serde_json::json!({}),
                };
                force_read_only_execution(&mut cfg.params);
                let driver = self.registry.get(req.engine);
                let target_label = format!("{}@127.0.0.1:{port}", req.engine.as_str());

                if req.engine == Engine::Redis && !req.is_custom_query {
                    let redis_node = effective_db
                        .as_deref()
                        .map(|d| {
                            if d.starts_with("kdb:") {
                                d.to_string()
                            } else {
                                format!("kdb:{d}")
                            }
                        })
                        .unwrap_or_else(|| "kdb:0".to_string());
                    let (columns, rows) =
                        enrich_redis_prefix(&*driver, &cfg, &redis_node, &req.object_name, limit)
                            .await?;
                    return Ok(WatchExecOutcome {
                        target_label,
                        columns,
                        rows,
                        duration_ms: started.elapsed().as_millis() as u64,
                    });
                }

                let node = match req.engine {
                    Engine::Redis => Some("kdb:0".to_string()),
                    _ => effective_db,
                };
                let qreq = QueryRequest {
                    statement: req.statement,
                    max_rows: Some(limit),
                    node,
                    timeout_ms: Some(mcp_query_timeout_ms(Some(10_000))),
                    read_only: true,
                    ..QueryRequest::default()
                };
                let res = driver.run(&cfg, &qreq).await?;
                Ok(WatchExecOutcome {
                    target_label,
                    columns: res.columns.into_iter().map(|c| c.name).collect(),
                    rows: res.rows,
                    duration_ms: res
                        .stats
                        .duration_ms
                        .max(started.elapsed().as_millis() as u64),
                })
            }
        }
    }

    async fn check_watch_conn_access(
        &self,
        roles: &Arc<dyn RoleChecker>,
        user: &User,
        conn: &Connection,
        owner_private: bool,
    ) -> Result<()> {
        let pool = self.connections.pool();
        let enforced = crate::access::policy(&pool, &conn.id).await?.mode
            == otto_core::access::AccessMode::Enforced;
        if !enforced && owner_private && !user.is_root && conn.created_by != user.id {
            return Err(Error::NotFound("connection".into()));
        }
        if enforced {
            let current = crate::access::current_user(&pool, conn, &user.id).await?;
            if !otto_rbac::resource_access::ResourceAccess::new(pool.clone())
                .evaluate(&current, &crate::access::target(&conn.id, None), "discover")
                .await?
                .allowed
            {
                return Err(Error::NotFound("connection".into()));
            }
        }
        match &conn.workspace_id {
            Some(ws) => roles.check(user, ws, WorkspaceRole::Viewer).await,
            None => {
                let need = capability_for_role(WorkspaceRole::Viewer);
                GrantsRepo::new(pool)
                    .check_global(
                        user,
                        Feature::Database,
                        need,
                        &format!(
                            "global connections need the Database '{}' grant",
                            need.as_str()
                        ),
                    )
                    .await
            }
        }
    }
}

async fn enrich_redis_prefix(
    driver: &dyn Driver,
    cfg: &ResolvedConfig,
    redis_node: &str,
    object_name: &str,
    limit: usize,
) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let cleaned = object_name.trim().trim_end_matches('*');
    let parent = NodePath::parse(redis_node);
    let nodes = driver.schema_children(cfg, &parent, Some(cleaned)).await?;
    let cap = limit.min(REDIS_WATCH_ENRICH_CAP);
    let mut rows = Vec::new();
    for node in nodes
        .into_iter()
        .filter(|n| n.kind == NodeKind::Key)
        .take(cap)
    {
        let key_path = NodePath::parse(&node.id);
        match driver.object_detail(cfg, &key_path).await {
            Ok(detail) => {
                let ty = detail
                    .extra
                    .get("type")
                    .cloned()
                    .unwrap_or_else(|| Value::String(node.detail.unwrap_or_default()));
                let preview = detail.extra.get("preview").cloned().unwrap_or(Value::Null);
                let ttl = detail.extra.get("ttl").cloned().unwrap_or(Value::Null);
                rows.push(vec![Value::String(node.label), ty, preview, ttl]);
            }
            Err(_) => {
                rows.push(vec![
                    Value::String(node.label),
                    Value::String(node.detail.unwrap_or_else(|| "unknown".into())),
                    Value::Null,
                    Value::Null,
                ]);
            }
        }
    }
    Ok((
        vec!["key".into(), "type".into(), "value".into(), "ttl".into()],
        rows,
    ))
}

fn redact_rows(rows: &mut [Vec<Value>]) {
    for row in rows {
        for cell in row.iter_mut() {
            *cell = otto_core::redact::redact_json(cell).value;
        }
    }
}
