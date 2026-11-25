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
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ensure_read_only, force_read_only_execution, mcp_query_timeout_ms, DbViewerService};
use crate::driver::Driver;
use crate::types::{
    Engine, NodeKind, NodePath, QueryRequest, QueryResult, ResolvedConfig, SchemaNode, TlsConfig,
};

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
    /// The real endpoint is on this Mac: a loopback host and no SSH tunnel.
    pub local: bool,
}

/// Resolved target endpoint for an IDE DB Watch snapshot query.
///
/// [`WatchExecReq`] / [`WatchExecOutcome`] mirror `otto_ide::db_watch`'s
/// `DbWatchQueryExec` / `DbWatchExecOutcome` field for field: the host
/// converts one into the other through serde (the two crates share no types).
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

/// What a [`WatchExecReq`] asks for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WatchOp {
    /// Run `statement` (rows → `columns` / `rows`).
    #[default]
    Read,
    /// The target's watchable objects (names → `objects`).
    ListObjects,
    /// `object_name`'s primary key and row estimate.
    Describe,
}

/// Read-only snapshot query request executed by [`DbViewerService::execute_watch_query`].
#[derive(Clone, Serialize, Deserialize)]
pub struct WatchExecReq {
    #[serde(rename = "target")]
    pub endpoint: WatchEndpoint,
    pub engine: Engine,
    pub database: Option<String>,
    pub object_name: String,
    pub statement: String,
    pub limit: usize,
    #[serde(default)]
    pub op: WatchOp,
}

/// Snapshot / listing / describe result returned to `otto-ide`. Rows are RAW:
/// the watch diffs them and masks only what an agent reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WatchExecOutcome {
    pub target_label: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub duration_ms: u64,
    /// `ListObjects`: names; `Describe`: unused.
    pub objects: Vec<String>,
    /// `ListObjects`: the database / schema the names belong to.
    pub database: Option<String>,
    /// `Describe`: the primary key (`[]` = none) and the engine's row estimate.
    pub primary_key: Vec<String>,
    pub row_estimate: Option<i64>,
}

/// Most objects one listing returns.
const WATCH_LIST_CAP: usize = 500;

/// Where a watch op runs: through the service (a saved connection: its pool,
/// tunnel, access checks) or straight on a driver (a local ephemeral target).
enum Via<'a> {
    Saved { id: &'a Id, user: &'a Id },
    Direct(Arc<dyn Driver>, Box<ResolvedConfig>),
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
                let host = if parsed.config.host.is_empty() {
                    "127.0.0.1".to_string()
                } else {
                    parsed.config.host
                };
                out.push(SavedWatchTarget {
                    connection_id: conn.id.to_string(),
                    name: conn.name,
                    engine,
                    local: parsed.ssh.is_none() && is_loopback(&host),
                    host,
                    port: parsed.config.port,
                    database: parsed.config.database,
                    environment: conn.environment.as_str().to_string(),
                });
            }
        }
        Ok(out)
    }

    /// Run one strictly read-only IDE DB Watch op against a saved or local
    /// loopback ephemeral target. Reads never write a DB Explorer history row
    /// and come back unmasked (the watch masks what an agent reads).
    pub async fn execute_watch_query(
        &self,
        roles: &Arc<dyn RoleChecker>,
        user: &User,
        req: WatchExecReq,
    ) -> Result<WatchExecOutcome> {
        let started = Instant::now();
        let limit = req.limit.clamp(1, 150);
        let (via, label, default_db) = match &req.endpoint {
            WatchEndpoint::Saved { connection_id } => {
                let conn = self
                    .authorize_saved_watch(roles, user, connection_id)
                    .await?;
                let engine = Engine::from_kind(conn.kind).ok_or_else(|| {
                    Error::Invalid(format!(
                        "connection '{}' is not a queryable database",
                        conn.name
                    ))
                })?;
                if engine != req.engine {
                    return Err(Error::Invalid(format!(
                        "connection '{}' uses engine '{}', not '{}'",
                        conn.name,
                        engine.as_str(),
                        req.engine.as_str()
                    )));
                }
                let default_db = crate::config::parse(&conn, None)?.config.database;
                let via = Via::Saved {
                    id: connection_id,
                    user: &user.id,
                };
                (via, conn.name, default_db)
            }
            WatchEndpoint::EphemeralLoopback {
                port,
                user: db_user,
                password,
                database,
            } => {
                if !otto_core::auth::root_authority(user) {
                    return Err(otto_core::auth::root_refusal());
                }
                let mut cfg = ResolvedConfig {
                    lifecycle: None,
                    engine: req.engine,
                    host: "127.0.0.1".into(),
                    port: *port,
                    user: db_user.clone(),
                    password: password.clone(),
                    database: database.clone(),
                    tls: TlsConfig::default(),
                    params: serde_json::json!({}),
                };
                // Postgres: the watch's `database` is the schema; the
                // container's own database stays the connection's.
                if req.engine != Engine::Postgres {
                    if let Some(db) = trimmed(req.database.as_deref()) {
                        cfg.database = Some(db.to_string());
                    }
                }
                force_read_only_execution(&mut cfg.params);
                let label = format!("{}@127.0.0.1:{port}", req.engine.as_str());
                let default_db = database.clone();
                (
                    Via::Direct(self.registry.get(req.engine), Box::new(cfg)),
                    label,
                    default_db,
                )
            }
        };
        // Postgres: `database` is a schema (default `public`), never the
        // connection's own database name.
        let database = trimmed(req.database.as_deref())
            .map(str::to_string)
            .or(default_db.filter(|_| req.engine != Engine::Postgres));
        let mut out = match req.op {
            WatchOp::ListObjects => {
                let (db, objects) = self.watch_list(&via, req.engine, database).await?;
                WatchExecOutcome {
                    objects,
                    database: db,
                    ..Default::default()
                }
            }
            WatchOp::Describe => {
                self.watch_describe(&via, req.engine, database.as_deref(), &req.object_name)
                    .await?
            }
            WatchOp::Read => self.watch_read(&via, &req, database, limit).await?,
        };
        out.target_label = label;
        out.duration_ms = out.duration_ms.max(started.elapsed().as_millis() as u64);
        Ok(out)
    }

    /// Database grant + connection access for a saved watch target.
    async fn authorize_saved_watch(
        &self,
        roles: &Arc<dyn RoleChecker>,
        user: &User,
        connection_id: &Id,
    ) -> Result<Connection> {
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
        let conn = self.connections.get(connection_id).await?;
        self.check_watch_conn_access(roles, user, &conn, owner_private)
            .await?;
        Ok(conn)
    }

    async fn watch_read(
        &self,
        via: &Via<'_>,
        req: &WatchExecReq,
        database: Option<String>,
        limit: usize,
    ) -> Result<WatchExecOutcome> {
        ensure_read_only(req.engine, &req.statement)?;
        // Redis items read by key pattern (the statement is only the label).
        if req.engine == Engine::Redis {
            let node = redis_node(database.as_deref());
            let (columns, rows) = match via {
                Via::Saved { id, user } => {
                    // Authorize BEFORE touching the server (the key scan).
                    self.authorize(id, user, Some(&node), "db_browse").await?;
                    let child = crate::access::child(Some(&node));
                    let mut r = self
                        .resolve(id, user, child.as_deref(), "db_browse")
                        .await?;
                    force_read_only_execution(&mut r.config.params);
                    // Values are data: the native privilege ceiling applies,
                    // and a dead pooled session is retried like other reads.
                    self.verify_native(id, user, &r).await?;
                    r.read(|| {
                        enrich_redis_prefix(&*r.driver, &r.config, &node, &req.object_name, limit)
                    })
                    .await?
                }
                Via::Direct(driver, cfg) => {
                    enrich_redis_prefix(&**driver, cfg, &node, &req.object_name, limit).await?
                }
            };
            return Ok(WatchExecOutcome {
                columns,
                rows,
                ..Default::default()
            });
        }
        let node = database.map(|d| format!("db:{d}"));
        let qreq = QueryRequest {
            statement: req.statement.clone(),
            max_rows: Some(limit),
            node,
            // MySQL carries its cap IN the statement (`MAX_EXECUTION_TIME`):
            // a client-side deadline retires the pooled session after every
            // read (`governed_read`), a fresh TCP/TLS/SSH connect per table
            // per stop.
            timeout_ms: (req.engine != Engine::Mysql).then(|| mcp_query_timeout_ms(Some(10_000))),
            read_only: true,
            ..QueryRequest::default()
        };
        let res: QueryResult = match via {
            // Native read-only, never a history row (`record: false`).
            Via::Saved { id, user } => self.run_with(id, user, &qreq, true, false).await?,
            Via::Direct(driver, cfg) => driver.run(cfg, &qreq).await?,
        };
        Ok(WatchExecOutcome {
            columns: res.columns.into_iter().map(|c| c.name).collect(),
            rows: res.rows,
            duration_ms: res.stats.duration_ms,
            ..Default::default()
        })
    }

    async fn watch_children(&self, via: &Via<'_>, path: &str) -> Result<Vec<SchemaNode>> {
        match via {
            Via::Saved { id, user } => self.schema_children(id, user, path, None).await,
            Via::Direct(driver, cfg) => {
                driver
                    .schema_children(cfg, &NodePath::parse(path), None)
                    .await
            }
        }
    }

    /// The target's watchable objects: MySQL / ClickHouse tables of the
    /// database, Postgres tables of the schema (default `public`), Mongo
    /// collections, Redis first-segment key prefixes (`session:`) that exist now.
    async fn watch_list(
        &self,
        via: &Via<'_>,
        engine: Engine,
        database: Option<String>,
    ) -> Result<(Option<String>, Vec<String>)> {
        let db = match engine {
            Engine::Postgres => Some(database.unwrap_or_else(|| "public".into())),
            Engine::Redis => database,
            _ => Some(
                database
                    .ok_or_else(|| Error::Invalid("choose a database to list its tables".into()))?,
            ),
        };
        let path = match engine {
            Engine::Mysql | Engine::Postgres => {
                format!("db:{}/folder:tables", db.as_deref().unwrap_or_default())
            }
            Engine::Mongodb | Engine::Clickhouse => {
                format!("db:{}", db.as_deref().unwrap_or_default())
            }
            Engine::Redis => redis_node(db.as_deref()),
        };
        let mut names: Vec<String> = self
            .watch_children(via, &path)
            .await?
            .into_iter()
            .filter_map(|n| match n.kind {
                NodeKind::Table | NodeKind::Collection => Some(n.label),
                // The namespace node reads `auth:*`; a watch item is the prefix.
                NodeKind::KeyNamespace => Some(format!(
                    "{}:",
                    n.label.trim_end_matches('*').trim_end_matches(':')
                )),
                _ => None,
            })
            .collect();
        names.sort();
        names.dedup();
        names.truncate(WATCH_LIST_CAP + 1);
        Ok((db, names))
    }

    /// `object`'s primary key and the engine's row estimate (Redis: `key`).
    async fn watch_describe(
        &self,
        via: &Via<'_>,
        engine: Engine,
        database: Option<&str>,
        object: &str,
    ) -> Result<WatchExecOutcome> {
        if engine == Engine::Redis {
            return Ok(WatchExecOutcome {
                primary_key: vec!["key".into()],
                ..Default::default()
            });
        }
        let (db, name) = match object.split_once('.') {
            Some((db, name)) => (Some(db), name),
            None => (database, object),
        };
        let db = match (engine, db) {
            (_, Some(db)) => db,
            (Engine::Postgres, None) => "public",
            _ => return Err(Error::Invalid("choose a database for this table".into())),
        };
        let leaf = if engine == Engine::Mongodb {
            "coll"
        } else {
            "table"
        };
        let path = format!("db:{db}/{leaf}:{name}");
        let detail = match via {
            Via::Saved { id, user } => self.object_detail(id, user, &path, true).await?,
            Via::Direct(driver, cfg) => {
                driver
                    .object_detail_with_opts(cfg, &NodePath::parse(&path), true)
                    .await?
            }
        };
        let primary_key = match engine {
            Engine::Mongodb => vec!["_id".into()],
            _ => detail.primary_key,
        };
        Ok(WatchExecOutcome {
            primary_key,
            row_estimate: detail.row_count,
            ..Default::default()
        })
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

/// Rows `key, type, value, ttl` of the keys matching `object` (a prefix or
/// a glob), pipelined by the driver.
async fn enrich_redis_prefix(
    driver: &dyn Driver,
    cfg: &ResolvedConfig,
    redis_node: &str,
    object_name: &str,
    limit: usize,
) -> Result<(Vec<String>, Vec<Vec<Value>>)> {
    let cap = limit.min(REDIS_WATCH_ENRICH_CAP);
    let rows = driver
        .watch_keys(cfg, &NodePath::parse(redis_node), object_name, cap)
        .await?;
    Ok((
        vec!["key".into(), "type".into(), "value".into(), "ttl".into()],
        rows,
    ))
}

fn trimmed(raw: Option<&str>) -> Option<&str> {
    raw.map(str::trim).filter(|s| !s.is_empty())
}

/// `kdb:<n>` for a Redis keyspace given as `n` or `kdb:n` (default 0).
fn redis_node(database: Option<&str>) -> String {
    match trimmed(database) {
        Some(d) if d.starts_with("kdb:") => d.to_string(),
        Some(d) => format!("kdb:{d}"),
        None => "kdb:0".into(),
    }
}

/// A host name that can only mean this Mac.
fn is_loopback(host: &str) -> bool {
    let h = host.trim().trim_start_matches('[').trim_end_matches(']');
    h.eq_ignore_ascii_case("localhost")
        || h.parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_loopback_host_is_local() {
        for h in ["127.0.0.1", "localhost", "::1", "[::1]", "127.0.0.2"] {
            assert!(is_loopback(h), "{h}");
        }
        for h in ["10.0.0.5", "db.internal", "0.0.0.0", "192.168.1.2"] {
            assert!(!is_loopback(h), "{h}");
        }
    }

    #[test]
    fn a_mysql_watch_read_with_its_time_cap_hint_is_read_only() {
        ensure_read_only(
            Engine::Mysql,
            "SELECT /*+ MAX_EXECUTION_TIME(10000) */ * FROM `pr_bo_1`.`session_info` ORDER BY `id` DESC LIMIT 150",
        )
        .expect("watch read passes the classifier");
    }

    #[test]
    fn watch_types_round_trip_through_their_wire_shape() {
        let req: WatchExecReq = serde_json::from_value(serde_json::json!({
            "target": {"ephemeral_loopback": {"port": 1, "user": null, "password": null, "database": null}},
            "engine": "mysql", "database": null, "object_name": "t", "statement": "SELECT 1",
            "limit": 5, "op": "describe"
        }))
        .unwrap();
        assert_eq!(req.op, WatchOp::Describe);
    }
}
