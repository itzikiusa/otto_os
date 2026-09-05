//! `DbViewerService` — the orchestration layer behind the HTTP routes.
//!
//! Resolves a stored connection profile + keychain secret into a
//! [`ResolvedConfig`], establishes an SSH tunnel when configured (rewriting the
//! endpoint to the local forward), dispatches to the engine [`Driver`], and
//! records query history.
//!
//! For speed, both layers are cached across calls: each driver keeps a
//! per-`cache_key` connection cache, and this service keeps the SSH tunnel
//! alive in [`DbViewerService::tunnels`] instead of re-spawning an `ssh` child
//! every operation. A cached tunnel keeps a stable local port, which keeps the
//! driver's `cache_key` (host:port-derived) stable, which keeps the driver's
//! cached connection valid — the two caches compose.

mod changes;
mod validation;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::stream::{self, StreamExt as _};

use otto_core::domain::Connection;
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id, Result};
use otto_state::{
    Dashboard, ConnectionsRepo, DbExplorerRepo, HistoryEntry, NewSavedQuery, NewWidget, SavedQuery,
    Widget,
};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::config::{self};
use crate::driver::Driver;
use crate::registry::Registry;
#[cfg(test)]
use crate::types::QueryHandle;
use otto_core::redact;
use otto_ssh::SshTunnel;
use crate::types::{
    statement_is_write, Capabilities, CancelToken, CompletionContext, CompletionResponse,
    DbQueryPlan, Engine, GraphColumn, GraphEdge, GraphTable, NodeKind, NodePath, ObjectDetail,
    ObjectSearchReq, ObjectSearchResult, QueryRequest, QueryResult, QueryStatus,
    ResolvedConfig, SchemaGraph, SchemaNode, TestResult,
};

/// Stable marker prefixed to the write-gate rejection message so the UI can
/// recognise it (and prompt for a typed confirmation) without string-matching
/// prose. The text after it stays human-readable.
pub const WRITE_BLOCKED_PREFIX: &str = "write_blocked: ";

/// Stable marker prefixed to the **MCP read-only** rejection message. Distinct
/// from [`WRITE_BLOCKED_PREFIX`] because there is no confirm path here — the
/// read-only MCP query endpoint refuses writes outright, regardless of the
/// connection's write-guard, so an agent-supplied statement can never mutate data.
pub const MCP_READ_ONLY_PREFIX: &str = "mcp_read_only: ";

/// Hard ceiling on rows a read-only MCP query may return, applied server-side
/// regardless of the request (the `ottod mcp-tools` process caps again before the
/// rows reach the agent transcript). Keeps a runaway `SELECT *` from flooding an
/// agent's context.
pub const MCP_MAX_ROWS: usize = 200;

/// Recursively PII-mask a result's cells in place (via `otto_core::redact`) and
/// flag it `masked`, including every later result set in a multi-statement
/// batch's `more_results` — so an unmasked cell never leaks just because it was
/// produced by the 2nd+ statement.
fn mask_result(res: &mut QueryResult) {
    for row in &mut res.rows {
        for cell in row.iter_mut() {
            *cell = redact::redact_json(cell).value;
        }
    }
    res.masked = true;
    for more in &mut res.more_results {
        mask_result(more);
    }
}

/// The MCP read-only policy gate: decide whether `statement` may run over the
/// read-only MCP query path for `engine`. Pure (no I/O) so it is unit-tested
/// exhaustively without a database. Returns:
/// - `Err(Invalid)` for an empty statement,
/// - `Err(Forbidden)` when the statement is classified as a write/DDL (reusing the
///   conservative [`statement_is_write`] classifier — unknown counts as a write),
/// - `Ok(())` for a recognised read.
pub fn ensure_read_only(engine: Engine, statement: &str) -> Result<()> {
    if statement.trim().is_empty() {
        return Err(Error::Invalid("empty statement".into()));
    }
    if statement_is_write(engine, statement) {
        return Err(Error::Forbidden(format!(
            "{MCP_READ_ONLY_PREFIX}only read-only statements may run over MCP; \
             this statement is classified as a write/DDL"
        )));
    }
    Ok(())
}

/// Evict cached SSH tunnels idle longer than this — dropping them kills the
/// `ssh` child via `SshTunnel::Drop`. Enforced both lazily (on the next
/// `resolve`) and by a background reaper (`reap_idle`) so an idle daemon doesn't
/// keep an orphaned `ssh` child alive indefinitely. Conservative window: a
/// reconnect on the next query after a long idle is transparent to the user.
const TUNNEL_IDLE_TTL: Duration = Duration::from_secs(30 * 60);

/// Hard cap on an import file's size. The whole file is read into RAM and the
/// parse + INSERT-rendering pipeline holds several transformed copies of it, so
/// this bounds worst-case daemon memory for `import_from_path` at a few × this.
const IMPORT_MAX_BYTES: u64 = 100 * 1024 * 1024;

/// Table cap for [`DbViewerService::schema_context`] — the COMPLETE schema fed to
/// the DB Assistant agent. Generous (a model wants the whole picture) but bounded:
/// each table is one `object_detail` round-trip, so a runaway schema with tens of
/// thousands of tables can't stall the assist's seed step.
const SCHEMA_CONTEXT_MAX_TABLES: usize = 300;

/// A cached SSH tunnel kept alive between operations.
struct CachedTunnel {
    tunnel: Arc<SshTunnel>,
    last_used: Instant,
}

/// One in-flight, cancellable query. Holds the engine-native handle slot (filled
/// by the driver once it knows the backend connection id / `query_id`) plus the
/// connection it ran against, so a cancel can re-resolve a *fresh* connection to
/// that same endpoint and issue the engine-native KILL on it.
#[derive(Clone)]
struct InFlightQuery {
    conn_id: Id,
    user_id: Id,
    resolved: Option<Resolved>,
    token: CancelToken,
}

/// How long a finished-but-unclaimed query outcome is parked for re-attach.
const FINISHED_TTL: Duration = Duration::from_secs(10 * 60);
/// Cap on parked outcomes; the oldest is evicted first when full.
const FINISHED_MAX: usize = 50;

/// Outcome of a detached query whose HTTP waiter was gone when it finished
/// (the user navigated away and the browser dropped the request). Parked so the
/// client can re-attach by `query_id` and still see the result.
struct FinishedQuery {
    conn_id: Id,
    outcome: std::result::Result<QueryResult, String>,
    at: Instant,
}

/// Bounded TTL store for [`FinishedQuery`]: expired entries are pruned on every
/// touch, and the oldest entry is evicted beyond [`FINISHED_MAX`]. Row counts
/// are already capped per query (`max_rows`), so worst-case memory is bounded
/// by `FINISHED_MAX × one result page`. Pure map logic so it's unit-testable.
#[derive(Default)]
struct FinishedStore {
    map: HashMap<String, FinishedQuery>,
}

impl FinishedStore {
    fn park(
        &mut self,
        query_id: String,
        conn_id: Id,
        outcome: std::result::Result<QueryResult, String>,
        now: Instant,
    ) {
        self.prune(now);
        if self.map.len() >= FINISHED_MAX {
            if let Some(oldest) = self
                .map
                .iter()
                .min_by_key(|(_, f)| f.at)
                .map(|(k, _)| k.clone())
            {
                self.map.remove(&oldest);
            }
        }
        self.map.insert(
            query_id,
            FinishedQuery {
                conn_id,
                outcome,
                at: now,
            },
        );
    }

    fn prune(&mut self, now: Instant) {
        self.map
            .retain(|_, f| now.duration_since(f.at) < FINISHED_TTL);
    }

    /// The parked outcome for `(query_id, conn_id)`, if present and unexpired.
    /// The connection must match — a status probe must not read another
    /// connection's result through a guessed/reused query id.
    fn get(&mut self, query_id: &str, conn_id: &Id, now: Instant) -> Option<&FinishedQuery> {
        self.prune(now);
        self.map.get(query_id).filter(|f| &f.conn_id == conn_id)
    }
}

#[derive(Clone)]
pub struct DbViewerService {
    connections: ConnectionsRepo,
    secrets: Arc<dyn SecretStore>,
    repo: DbExplorerRepo,
    registry: Registry,
    /// Live SSH tunnels, keyed by connection id, reused across operations.
    tunnels: Arc<Mutex<HashMap<Id, CachedTunnel>>>,
    /// In-flight cancellable queries, keyed by the client's `query_id`. Populated
    /// for the duration of a `run` that carries a `query_id`; a `cancel` request
    /// looks the target up here to issue engine-native cancellation. A plain
    /// `std::sync::Mutex` (held only for the brief map insert/remove/lookup, never
    /// across an await) — distinct from the tokio `Mutex` guarding `tunnels`.
    in_flight: Arc<std::sync::Mutex<HashMap<String, InFlightQuery>>>,
    /// Outcomes of detached queries that finished with no HTTP waiter left,
    /// keyed by `query_id`, kept for [`FINISHED_TTL`] so the client can
    /// re-attach ([`Self::query_status`]) after navigating away.
    finished: Arc<std::sync::Mutex<FinishedStore>>,
    /// The driver cache key each connection last resolved to. Driver caches are
    /// keyed by [`ResolvedConfig::cache_key`], not connection id, so this map is
    /// what lets the service (a) tear a connection's pool down on an explicit
    /// close ([`Self::close_connection`]) and (b) evict the SUPERSEDED pool when
    /// a config change (password edit, tunnel restart → new local port) rekeys a
    /// connection — previously the old pool leaked for the daemon's lifetime.
    /// Plain `std::sync::Mutex`: held only for map ops, never across an await.
    active_keys: Arc<std::sync::Mutex<HashMap<Id, (Engine, String)>>>,
}

/// Removes an in-flight query from the registry when a `run` ends — on success,
/// error, or future-cancellation (the UI dropping the request). RAII so the map
/// never leaks a stale entry that a later cancel could wrongly target.
struct InFlightGuard {
    map: Arc<std::sync::Mutex<HashMap<String, InFlightQuery>>>,
    key: String,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Ok(mut map) = self.map.lock() {
            map.remove(&self.key);
        }
    }
}

/// Decide what (if anything) to cancel for a `(conn_id, query_id)` pair, given a
/// snapshot of the in-flight registry. Returns the engine-native [`QueryHandle`]
/// to KILL, or `None` when there's nothing to do — an unknown/finished query, a
/// query that belongs to a *different* connection (a cancel must not reach across
/// connections), or one whose driver hasn't captured a native handle yet (engine
/// without per-query cancel, or the query hadn't reached the capture point). Pure
/// so the decision logic is unit-testable without live repos.
#[cfg(test)]
fn cancel_handle_for(
    map: &HashMap<String, InFlightQuery>,
    conn_id: &Id,
    query_id: &str,
) -> Option<QueryHandle> {
    let target = map.get(query_id)?;
    if &target.conn_id != conn_id {
        return None;
    }
    target.token.handle()
}

/// What an import produced — returned to the HTTP handler for its final line.
#[derive(Debug, Clone, Copy, Default, serde::Serialize)]
pub struct ImportCounts {
    pub rows: u64,
    pub batches: u64,
}

/// A driver + endpoint resolved for one operation. The `_tunnel` clone, when
/// present, keeps a reference to the (cached) SSH forward alive for at least
/// the lifetime of this struct; the cache holds the other reference, so the
/// tunnel persists after the operation completes.
#[derive(Clone)]
struct Resolved {
    driver: Arc<dyn Driver>,
    config: ResolvedConfig,
    _tunnel: Option<Arc<SshTunnel>>,
}

impl DbViewerService {
    pub fn new(
        connections: ConnectionsRepo,
        secrets: Arc<dyn SecretStore>,
        repo: DbExplorerRepo,
    ) -> Self {
        Self {
            connections,
            secrets,
            repo,
            registry: Registry::new(),
            tunnels: Arc::new(Mutex::new(HashMap::new())),
            in_flight: Arc::new(std::sync::Mutex::new(HashMap::new())),
            finished: Arc::new(std::sync::Mutex::new(FinishedStore::default())),
            active_keys: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Fetch a stored connection profile (for workspace/role resolution).
    pub async fn get_connection(&self, id: &Id) -> Result<Connection> {
        self.connections.get(id).await
    }

    /// Resource gate for trusted adapters. Resolves current feature/workspace,
    /// user/group and direct-rule state on every call.
    pub async fn authorize(&self, conn_id: &Id, user_id: &Id, node: Option<&str>, operation: &str) -> Result<()> {
        let conn = self.connections.get(conn_id).await?;
        let child = crate::access::child(node);
        crate::access::check(&self.connections.pool(), &conn, user_id, child.as_deref(), operation).await
    }

    pub async fn is_enforced(&self, conn_id: &Id) -> Result<bool> {
        Ok(crate::access::policy(&self.connections.pool(), conn_id).await?.mode == otto_core::access::AccessMode::Enforced)
    }

    async fn execution_access(&self, conn_id: &Id, user_id: &Id, req: &QueryRequest) -> Result<()> {
        if !self.is_enforced(conn_id).await? { return Ok(()); }
        let conn = self.connections.get(conn_id).await?;
        let engine = Engine::from_kind(conn.kind).ok_or_else(|| Error::Invalid("not a database".into()))?;
        let operations = crate::access::operations(engine, &req.statement)?;
        for operation in &operations {
            self.authorize(conn_id, user_id, req.node.as_deref(), operation).await?;
        }
        if conn.read_only && operations.iter().any(|op|*op!="db_query") {
            return Err(Error::Forbidden("read-only connection cannot perform database mutations".into()));
        }
        // Production writes must use the independent reviewed-change adapter.
        // Neither root nor confirm_write can turn an approval into an access grant.
        if conn.environment == otto_core::domain::Environment::Prod && operations.iter().any(|op| *op != "db_query") {
            return Err(Error::Forbidden("review_required: production changes require an approved immutable change".into()));
        }
        Ok(())
    }

    async fn verify_native(&self, conn_id: &Id, user_id: &Id, r: &Resolved) -> Result<()> {
        if !self.is_enforced(conn_id).await? { return Ok(()); }
        let conn = self.connections.get(conn_id).await?;
        let user = crate::access::current_user(&self.connections.pool(), &conn, user_id).await?;
        if user.is_root { return Ok(()); }
        let grants = r.driver.native_grants(&r.config).await?;
        for grant in grants {
            let decision = otto_rbac::resource_access::ResourceAccess::new(self.connections.pool())
                .evaluate(&user, &crate::access::target(conn_id, Some(&grant.child)), grant.operation).await?;
            if !decision.allowed {
                return Err(crate::native_access::setup_error("credential privileges exceed the caller's current database/operation scope"));
            }
        }
        Ok(())
    }

    async fn require_local_path_access(&self, conn_id: &Id, user_id: &Id) -> Result<()> {
        if self.is_enforced(conn_id).await? {
            let conn = self.connections.get(conn_id).await?;
            if !crate::access::current_user(&self.connections.pool(), &conn, user_id).await?.is_root {
                return Err(Error::Forbidden("daemon-local file paths require root for governed connections; use browser export".into()));
            }
        }
        Ok(())
    }

    /// Close only the effective caller's governed pools. Shared tunnels remain
    /// until idle; closing one tab must not close another user's database work.
    pub async fn close_for_user(&self, conn_id: &Id, user_id: &Id) -> Result<()> {
        self.authorize(conn_id, user_id, None, "discover").await?;
        if !self.is_enforced(conn_id).await? { return self.close_connection(conn_id).await; }
        let entries = {
            let mut keys = self.active_keys.lock().map_err(|_| Error::Internal("active-keys registry poisoned".into()))?;
            let prefix = format!("{conn_id}\0{user_id}\0");
            let owners: Vec<_> = keys.keys().filter(|key| key.starts_with(&prefix)).cloned().collect();
            owners.into_iter().filter_map(|key| keys.remove(&key)).collect::<Vec<_>>()
        };
        for (engine, key) in entries { self.registry.get(engine).close(&key).await; }
        Ok(())
    }

    async fn current_scope(&self, conn_id: &Id, user_id: &Id, child: Option<&str>, operation: &str) -> Result<Option<String>> {
        let logical = self.connections.get(conn_id).await?;
        let (profile_id, scope) = crate::access::credential_profile(&self.connections.pool(), &logical, user_id, child, operation).await?;
        let Some(scope) = scope else { return Ok(None); };
        let profile = self.connections.get(&profile_id).await?;
        let secret = match &profile.secret_ref { Some(key) => self.secrets.get(key)?, None => None };
        let cfg = config::parse(&profile, secret)?.config;
        Ok(Some(format!("{}:{}:{}:{}", scope, cfg.cache_key(), logical.environment.as_str(), logical.read_only)))
    }

    async fn check_resolved_scope(&self, conn_id: &Id, user_id: &Id, node: Option<&str>, operation: &str, r: &Resolved) -> Result<()> {
        let child = crate::access::child(node);
        let current = self.current_scope(conn_id, user_id, child.as_deref(), operation).await?;
        if current.as_deref() != r.config.params.get("__access_scope").and_then(Value::as_str) {
            return Err(Error::Forbidden("connection authorization or credentials changed during execution".into()));
        }
        Ok(())
    }

    async fn monitor_export<T>(&self, conn_id: &Id, user_id: &Id, statement: &str, node: Option<&str>, r: &Resolved, execution: impl std::future::Future<Output = Result<T>>) -> Result<T> {
        tokio::pin!(execution);
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        loop {
            tokio::select! {
                result = &mut execution => {
                    self.guard_export(conn_id, user_id, statement, node).await?;
                    self.check_resolved_scope(conn_id, user_id, node, "db_export", r).await?;
                    return result;
                }
                _ = interval.tick() => {
                    // Dropping the native stream on revocation stops further
                    // response/file writes; effects already sent are not undone.
                    self.guard_export(conn_id, user_id, statement, node).await?;
                    self.check_resolved_scope(conn_id, user_id, node, "db_export", r).await?;
                }
            }
        }
    }

    /// Static capabilities for the engine behind a connection.
    pub async fn capabilities(&self, conn_id: &Id, user_id: &Id) -> Result<Capabilities> {
        self.authorize(conn_id, user_id, None, "discover").await?;
        let conn = self.connections.get(conn_id).await?;
        let engine = Engine::from_kind(conn.kind).ok_or_else(|| {
            otto_core::Error::Invalid(format!(
                "connection kind '{}' is not a browsable database",
                conn.kind.as_str()
            ))
        })?;
        Ok(self.registry.get(engine).capabilities())
    }

    async fn resolve(&self, conn_id: &Id, user_id: &Id, child: Option<&str>, operation: &str) -> Result<Resolved> {
        let logical = self.connections.get(conn_id).await?;
        let (profile_id, scope) = crate::access::credential_profile(&self.connections.pool(), &logical, user_id, child, operation).await?;
        let conn = if profile_id == *conn_id { logical.clone() } else {
            let profile = self.connections.get(&profile_id).await?;
            let mut logical_params = logical.params.clone();
            let mut profile_params = profile.params.clone();
            for params in [&mut logical_params, &mut profile_params] {
                if let Some(map) = params.as_object_mut() {
                    for field in ["user", "username", "password"] { map.remove(field); }
                }
            }
            if profile.kind != logical.kind || profile_params != logical_params {
                return Err(crate::native_access::setup_error("credential profile must match the logical engine, endpoint, database, TLS and SSH configuration"));
            }
            profile
        };
        let secret = match &conn.secret_ref {
            Some(r) => self.secrets.get(r)?,
            None => None,
        };
        let parsed = config::parse(&conn, secret)?;
        let engine = parsed.config.engine;
        let driver = self.registry.get(engine);
        let mut config = parsed.config;
        if let Some(scope) = scope {
            let scope = format!("{}:{}:{}:{}", scope, config.cache_key(), logical.environment.as_str(), logical.read_only);
            config.params.as_object_mut().ok_or_else(|| Error::Invalid("connection params must be an object".into()))?
                .insert("__access_scope".into(), Value::String(scope));
            if matches!(operation,"db_query"|"db_export") {config.params.as_object_mut().unwrap().insert("__read_only_execution".into(),Value::Bool(true));}
        }
        let tunnel = match parsed.ssh {
            Some(ssh) => Some(self.tunnel_for(&profile_id, &ssh, &config, engine).await?),
            None => None,
        };
        if let Some(t) = &tunnel {
            if engine == Engine::Mongodb {
                // MongoDB tunnels via a dynamic SOCKS5 forward, NOT a local
                // forward: a `mongodb+srv` (Atlas) profile resolves its replica
                // set's real shard hosts at runtime via SRV/SDAM, so there's no
                // single endpoint to rewrite, and Atlas's load balancer routes by
                // SNI. We leave host/port (and any conn_string) untouched and just
                // hand the driver the SOCKS proxy port — it then dials each real
                // host through the bastion with the real SNI preserved.
                let socks_port = t.local_port();
                if let Value::Object(map) = &mut config.params {
                    map.insert("__socks_port".into(), Value::from(socks_port));
                } else {
                    config.params = serde_json::json!({ "__socks_port": socks_port });
                }
            } else {
                // Single-endpoint engines: point the driver at the (cached) local
                // forward, but stash the ORIGINAL host/port in params so
                // TLS-sensitive drivers (e.g. ClickHouse over HTTPS to an
                // SNI-routed managed service like Yandex/clickhouse.cloud) can
                // still present the real hostname for SNI/Host while the TCP goes
                // through the tunnel. Without this we'd send SNI=127.0.0.1 and the
                // managed frontend can't route it → the TLS handshake stalls.
                let real_host = config.host.clone();
                let real_port = config.port;
                config.host = t.local_host().to_string();
                config.port = t.local_port();
                if let Value::Object(map) = &mut config.params {
                    map.insert("__tunnel_host".into(), Value::from(real_host));
                    map.insert("__tunnel_port".into(), Value::from(real_port));
                } else {
                    config.params = serde_json::json!({
                        "__tunnel_host": real_host,
                        "__tunnel_port": real_port,
                    });
                }
            }
        }
        // Track the cache key this connection now resolves to, and EVICT the
        // superseded driver-side handle when the key changed (credential/param
        // edit, or a restarted tunnel minting a new local port / `__socks_port`)
        // — otherwise the old pool, its backend connections, and its reconnect
        // loops are retained for the daemon's lifetime.
        let key = config.cache_key();
        let stale = {
            let mut keys = self
                .active_keys
                .lock()
                .map_err(|_| Error::Internal("active-keys registry poisoned".into()))?;
            let owner = if config.params.get("__access_scope").is_some() {
                format!("{conn_id}\0{user_id}\0{operation}\0{}", child.unwrap_or(""))
            } else { conn_id.clone() };
            match keys.insert(owner, (engine, key.clone())) {
                Some((old_engine, old_key)) if old_key != key => Some((old_engine, old_key)),
                _ => None,
            }
        };
        if let Some((old_engine, old_key)) = stale {
            self.registry.get(old_engine).close(&old_key).await;
        }
        Ok(Resolved {
            driver,
            config,
            _tunnel: tunnel,
        })
    }

    /// Tear down everything the daemon holds for a connection: cancel its
    /// in-flight queries (engine-native, best-effort — this must run FIRST,
    /// while the tunnel/pool are still alive), evict + close the driver's
    /// cached pool/handle, and drop the cached SSH tunnel (killing its `ssh`
    /// child). Idempotent: closing a connection with nothing cached is a no-op
    /// success. Backs `POST /connections/{id}/db/close` — without it, closing a
    /// tab was purely client-side and the warm pool made a "closed" connection
    /// reconnect instantly (and hold backend connections forever).
    pub async fn close_connection(&self, conn_id: &Id) -> Result<()> {
        // Cancel in-flight queries scoped to this connection.
        let queries: Vec<_> = self.in_flight.lock().map(|m| m.values().filter(|q| &q.conn_id == conn_id).cloned().collect()).unwrap_or_default();
        for query in queries {
            if let (Some(handle), Some(r)) = (query.token.handle(), query.resolved) {
                let _ = r.driver.cancel(&r.config, &handle).await;
            }
        }
        // Evict + close the driver-side cached handle for the key this
        // connection last resolved to.
        let entries = {
            let mut keys = self.active_keys.lock().map_err(|_| Error::Internal("active-keys registry poisoned".into()))?;
            let prefix = format!("{conn_id}\0");
            let owners: Vec<_> = keys.keys().filter(|key| *key == conn_id || key.starts_with(&prefix)).cloned().collect();
            owners.into_iter().filter_map(|key| keys.remove(&key)).collect::<Vec<_>>()
        };
        for (engine, key) in entries {
            self.registry.get(engine).close(&key).await;
        }
        // Drop the cached SSH tunnel — `SshTunnel::Drop` kills the ssh child.
        self.tunnels.lock().await.remove(conn_id);
        Ok(())
    }

    /// Get a live SSH tunnel for `conn_id`, reusing a cached one when it's still
    /// alive (and re-pointing at its local port), else opening a fresh tunnel.
    /// Idle tunnels are evicted on the way through, which drops their `ssh`
    /// child. Kept alive in the cache after this returns — the caller's clone is
    /// just an extra reference, not the sole owner.
    /// Evict cached SSH tunnels idle longer than [`TUNNEL_IDLE_TTL`], killing
    /// their `ssh` child (via `SshTunnel::Drop`). The lazy sweep in `tunnel_for`
    /// only fires when a DB op happens, so on an idle daemon a tunnel + its `ssh`
    /// child would otherwise linger indefinitely; a background reaper calls this
    /// on a timer. An in-flight query holds its own `Arc<SshTunnel>` clone (and
    /// refreshes `last_used`), so a live tunnel is never reaped out from under it.
    /// Returns the number of tunnels reaped.
    pub async fn reap_idle(&self) -> usize {
        let now = Instant::now();
        let mut tunnels = self.tunnels.lock().await;
        let before = tunnels.len();
        tunnels.retain(|_, c| now.duration_since(c.last_used) <= TUNNEL_IDLE_TTL);
        before - tunnels.len()
    }

    async fn tunnel_for(
        &self,
        conn_id: &Id,
        ssh: &crate::types::SshTunnelConfig,
        config: &ResolvedConfig,
        engine: Engine,
    ) -> Result<Arc<SshTunnel>> {
        let now = Instant::now();
        let mut tunnels = self.tunnels.lock().await;

        // Evict tunnels idle longer than the TTL (dropping kills their child).
        tunnels.retain(|_, c| now.duration_since(c.last_used) <= TUNNEL_IDLE_TTL);

        // Reuse a still-alive cached tunnel for this connection.
        if let Some(cached) = tunnels.get_mut(conn_id) {
            if cached.tunnel.is_alive() {
                cached.last_used = now;
                return Ok(Arc::clone(&cached.tunnel));
            }
            // Dead tunnel: drop it (kills the stale child) and fall through.
            tunnels.remove(conn_id);
        }

        // Open a fresh tunnel and cache it. MongoDB uses a dynamic SOCKS5 proxy
        // (the driver dials real hosts through it — see `resolve`); the
        // single-endpoint engines use a local forward to the profile's endpoint.
        let tunnel = Arc::new(if engine == Engine::Mongodb {
            SshTunnel::open_socks(ssh).await?
        } else {
            SshTunnel::open(ssh, &config.host, config.port).await?
        });
        tunnels.insert(
            conn_id.clone(),
            CachedTunnel {
                tunnel: Arc::clone(&tunnel),
                last_used: now,
            },
        );
        Ok(tunnel)
    }

    pub async fn test(&self, conn_id: &Id, user_id: &Id) -> Result<TestResult> {
        let child = crate::access::child(None);
        let r = self.resolve(conn_id, user_id, child.as_deref(), "configure").await?;
        r.driver.test(&r.config).await
    }

    /// Probe an UNSAVED connection config — kind + params + optional plaintext
    /// secret — without persisting anything (no DB row, no Keychain write). This
    /// backs the connection form's "Test" button so bad host/creds are caught
    /// BEFORE the profile (and its password) are stored. An SSH tunnel, when
    /// configured, opens ephemerally (never cached) and drops when the probe
    /// returns.
    pub async fn test_config(
        &self,
        kind: otto_core::domain::ConnectionKind,
        params: Value,
        secret: Option<String>,
    ) -> Result<TestResult> {
        use otto_core::domain::Connection;
        // Synthetic profile: config::parse only reads `kind` + `params` (+ the
        // secret we pass directly); everything else is inert placeholder.
        let conn = Connection {
            id: "unsaved-test".to_string(),
            workspace_id: None,
            name: "unsaved-test".to_string(),
            kind,
            params,
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Default::default(),
            read_only: false,
            created_by: "unsaved-test".to_string(),
            created_at: chrono::Utc::now(),
            last_opened_at: None,
            pinned: false,
        };
        let parsed = config::parse(&conn, secret)?;
        let engine = parsed.config.engine;
        let driver = self.registry.get(engine);
        let mut config = parsed.config;
        // Ephemeral tunnel — mirrors `resolve()`'s host/port rewrite, but is
        // held only for this probe (dropping it kills the ssh child).
        let _tunnel = match parsed.ssh {
            Some(ssh) => {
                let tunnel = if engine == Engine::Mongodb {
                    SshTunnel::open_socks(&ssh).await?
                } else {
                    SshTunnel::open(&ssh, &config.host, config.port).await?
                };
                if engine == Engine::Mongodb {
                    let socks_port = tunnel.local_port();
                    if let Value::Object(map) = &mut config.params {
                        map.insert("__socks_port".into(), Value::from(socks_port));
                    } else {
                        config.params = serde_json::json!({ "__socks_port": socks_port });
                    }
                } else {
                    let real_host = config.host.clone();
                    let real_port = config.port;
                    config.host = tunnel.local_host().to_string();
                    config.port = tunnel.local_port();
                    if let Value::Object(map) = &mut config.params {
                        map.insert("__tunnel_host".into(), Value::from(real_host));
                        map.insert("__tunnel_port".into(), Value::from(real_port));
                    } else {
                        config.params = serde_json::json!({
                            "__tunnel_host": real_host,
                            "__tunnel_port": real_port,
                        });
                    }
                }
                Some(tunnel)
            }
            None => None,
        };
        driver.test(&config).await
    }

    pub async fn schema_root(&self, conn_id: &Id, user_id: &Id) -> Result<Vec<SchemaNode>> {
        let child = crate::access::child(None);
        let r = self.resolve(conn_id, user_id, child.as_deref(), "discover").await?;
        let nodes = r.driver.schema_root(&r.config).await?;
        let mut visible = Vec::new();
        for node in nodes {
            if self.authorize(conn_id, user_id, Some(&node.id), "db_browse").await.is_ok() {
                visible.push(node);
            }
        }
        self.authorize(conn_id, user_id, None, "discover").await?;
        Ok(visible)
    }

    pub async fn schema_children(
        &self,
        conn_id: &Id, user_id: &Id,
        path: &str,
        filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        self.schema_children_with_counts(conn_id, user_id, path, filter, false).await
    }

    /// As above, but `counts` asks the driver to fill each node's `detail` with
    /// an engine-native ROW ESTIMATE. Opt-in: gathering that statistic is the
    /// slow part of expanding a database on a large server.
    pub async fn schema_children_with_counts(
        &self,
        conn_id: &Id, user_id: &Id,
        path: &str,
        filter: Option<&str>,
        counts: bool,
    ) -> Result<Vec<SchemaNode>> {
        let child = crate::access::child(Some(path));
        let r = self.resolve(conn_id, user_id, child.as_deref(), "db_browse").await?;
        let node = NodePath::parse(path);
        // An empty/whitespace filter is treated as "no filter".
        let filter = filter.map(str::trim).filter(|s| !s.is_empty());
        let result = r.driver.schema_children_with_counts(&r.config, &node, filter, counts).await?;
        self.authorize(conn_id, user_id, Some(path), "db_browse").await?;
        Ok(result)
    }

    /// Find objects by name without expanding the tree. A blank needle returns
    /// nothing rather than the entire catalog.
    pub async fn search_objects(
        &self,
        conn_id: &Id, user_id: &Id,
        req: &ObjectSearchReq,
    ) -> Result<ObjectSearchResult> {
        if req.q.trim().is_empty() {
            return Ok(ObjectSearchResult { supported: true, ..Default::default() });
        }
        let child = crate::access::child(None);
        let r = self.resolve(conn_id, user_id, child.as_deref(), "discover").await?;
        let mut result = r.driver.search_objects(&r.config, req).await?;
        let mut visible = Vec::new();
        for hit in result.hits {
            if self.authorize(conn_id, user_id, Some(&hit.schema), "db_browse").await.is_ok() { visible.push(hit); }
        }
        result.hits = visible;
        // Counts cannot reveal how many hidden databases were inspected.
        result.scanned = result.hits.iter().map(|h| &h.schema).collect::<std::collections::HashSet<_>>().len();
        self.authorize(conn_id, user_id, None, "discover").await?;
        Ok(result)
    }

    /// Return the full detail for a schema object. When `approx_row_count` is
    /// true, the driver is asked to fill `ObjectDetail::row_count` from an
    /// engine-native estimate (e.g. MySQL `information_schema.table_rows`).
    /// The flag is opt-in because it adds an extra query per call.
    pub async fn object_detail(
        &self,
        conn_id: &Id, user_id: &Id,
        path: &str,
        approx_row_count: bool,
    ) -> Result<ObjectDetail> {
        let child = crate::access::child(Some(path));
        let r = self.resolve(conn_id, user_id, child.as_deref(), "db_browse").await?;
        let node = NodePath::parse(path);
        let mut result = r.driver.object_detail_with_opts(&r.config, &node, approx_row_count).await?;
        if self.is_enforced(conn_id).await? {
            let mut foreign_keys = Vec::new();
            for key in result.foreign_keys {
                if self.authorize(conn_id, user_id, key.ref_schema.as_deref().or(child.as_deref()), "db_browse").await.is_ok() { foreign_keys.push(key); }
            }
            result.foreign_keys = foreign_keys;
            if self.authorize(conn_id, user_id, None, "db_browse").await.is_err() {
                result.ddl = None;
            }
            result.extra = Value::Null;
        }
        self.authorize(conn_id, user_id, Some(path), "db_browse").await?;
        Ok(result)
    }

    /// Production / read-only guardrail. When a connection is `Prod` or
    /// `read_only`, a statement classified as a write/DDL is refused unless the
    /// request carries `confirm_write`. Classification is conservative (unknown
    /// counts as a write). The error is a 409 with a [`WRITE_BLOCKED_PREFIX`]-tagged
    /// message so the UI can detect it and ask for a typed confirmation.
    ///
    /// `req.explain` does NOT exempt a statement: the SQL drivers ignore the flag
    /// and execute by statement text, so `{statement:"DELETE …", explain:true}`
    /// would otherwise bypass the gate and still run the write. A genuine
    /// `EXPLAIN`-prefixed statement classifies as a read via `statement_is_write`
    /// and passes the `is_write` check below; only raw writes carrying
    /// `explain:true` are blocked.
    async fn guard_write(&self, conn_id: &Id, req: &QueryRequest) -> Result<()> {
        if req.confirm_write {
            return Ok(());
        }
        let conn = self.connections.get(conn_id).await?;
        if !conn.is_write_guarded() {
            return Ok(());
        }
        // Non-browsable kinds never reach `run`, but guard defensively: if we
        // can't map to an engine we can't classify, so treat it as a write.
        let is_write = match Engine::from_kind(conn.kind) {
            Some(engine) => statement_is_write(engine, &req.statement),
            None => true,
        };
        if !is_write {
            return Ok(());
        }
        let reason = if conn.environment.is_production() {
            format!("production connection '{}'", conn.name)
        } else {
            format!("read-only connection '{}'", conn.name)
        };
        Err(Error::Conflict(format!(
            "{WRITE_BLOCKED_PREFIX}this is a {reason}; confirm the write to run it"
        )))
    }

    /// Build the relationship graph (ERD) for one schema/database: the tables
    /// (with columns + PK/FK flags) and the foreign-key edges between them.
    ///
    /// Engine-agnostic by construction — it walks the same lazy schema tree the
    /// UI browses (`schema_children` + `object_detail`), so every driver gets it
    /// for free and the FK data flows through the existing `ObjectDetail`
    /// introspection (e.g. MySQL `foreign_keys_of`). Engines without FK metadata
    /// (Redis/Mongo) yield tables with no edges and `relationships = false`.
    ///
    /// `max_tables` caps how many objects are introspected (a full schema can
    /// have thousands of tables, and each is one `object_detail` round-trip);
    /// when the cap clips the list, `truncated` is set so the UI can prompt the
    /// user to pick a subset.
    pub async fn schema_graph(
        &self,
        conn_id: &Id, user_id: &Id,
        schema: &str,
        max_tables: usize,
    ) -> Result<SchemaGraph> {
        let child = crate::access::child(Some(schema));
        let r = self.resolve(conn_id, user_id, child.as_deref(), "db_browse").await?;
        let driver = Arc::clone(&r.driver);
        let cfg = r.config.clone();
        let relationships = driver.capabilities().joins;

        // Redis has no `db:`-rooted tree (its root is `kdb:<n>` keyspaces), so
        // walking it below would error ("expected a keyspace node"). It also has
        // no relationships. Return an empty graph instead of failing the diagram.
        // Mongo (also `relationships = false`) DOES use `db:` nodes, so it keeps
        // walking the tree and yields collection cards with no edges.
        if cfg.engine == Engine::Redis {
            return Ok(SchemaGraph {
                schema: schema.to_string(),
                tables: Vec::new(),
                edges: Vec::new(),
                relationships: false,
                truncated: false,
            });
        }

        // Enumerate the table-like objects under the schema. We list the db
        // node's children and descend one level into any "folder" grouping
        // (MySQL exposes Tables/Views as folders); ClickHouse/Mongo list the
        // tables/collections directly.
        let db_path = NodePath::parse(&format!("db:{schema}"));
        let mut object_nodes: Vec<SchemaNode> = Vec::new();
        let mut total_seen = 0usize;
        for node in driver.schema_children(&cfg, &db_path, None).await? {
            match node.kind {
                NodeKind::Folder => {
                    let child = NodePath::parse(&node.id);
                    for inner in driver.schema_children(&cfg, &child, None).await? {
                        // Only table-like objects belong on the ERD — skip routine
                        // folders (Procedures/Functions), whose children are
                        // stored procedures/functions, not diagrammable tables.
                        if !matches!(
                            inner.kind,
                            NodeKind::Table | NodeKind::View | NodeKind::Collection
                        ) {
                            continue;
                        }
                        total_seen += 1;
                        if object_nodes.len() < max_tables {
                            object_nodes.push(inner);
                        }
                    }
                }
                NodeKind::Table | NodeKind::View | NodeKind::Collection => {
                    total_seen += 1;
                    if object_nodes.len() < max_tables {
                        object_nodes.push(node);
                    }
                }
                _ => {}
            }
        }
        let truncated = total_seen > object_nodes.len();

        // Fetch each object's detail in parallel (capped at 8 concurrent
        // in-flight round-trips) and project it onto the graph shape. A single
        // object that fails to introspect is skipped rather than failing the
        // whole diagram (a dropped/locked table shouldn't blank the canvas).
        //
        // Over an SSH tunnel each `object_detail` is one full RTT; running them
        // sequentially on a schema with 60 tables is 60 serial RTTs. At
        // concurrency-8 that shrinks to ~8 parallel waves → ~8× faster for
        // large schemas while staying well below typical MySQL `max_connections`.
        const GRAPH_CONCURRENCY: usize = 8;
        let schema_str = schema.to_string();
        let detail_results: Vec<(SchemaNode, Option<ObjectDetail>)> = stream::iter(object_nodes)
            .map(|node| {
                let driver = Arc::clone(&driver);
                let cfg = cfg.clone();
                async move {
                    let path = NodePath::parse(&node.id);
                    let detail = driver.object_detail(&cfg, &path).await.ok();
                    (node, detail)
                }
            })
            .buffer_unordered(GRAPH_CONCURRENCY)
            .collect()
            .await;

        let mut tables: Vec<GraphTable> = Vec::with_capacity(detail_results.len());
        let mut edges: Vec<GraphEdge> = Vec::new();
        for (node, maybe_detail) in detail_results {
            let detail = match maybe_detail {
                Some(d) => d,
                None => continue,
            };
            let pk: std::collections::HashSet<&str> =
                detail.primary_key.iter().map(String::as_str).collect();
            let fk_cols: std::collections::HashSet<&str> = detail
                .foreign_keys
                .iter()
                .flat_map(|fk| fk.columns.iter().map(String::as_str))
                .collect();
            let columns = detail
                .columns
                .iter()
                .map(|c| GraphColumn {
                    name: c.name.clone(),
                    data_type: c.data_type.clone(),
                    nullable: c.nullable,
                    primary_key: pk.contains(c.name.as_str()),
                    foreign_key: fk_cols.contains(c.name.as_str()),
                })
                .collect();

            for fk in &detail.foreign_keys {
                edges.push(GraphEdge {
                    name: fk.name.clone(),
                    from_table: detail.name.clone(),
                    from_columns: fk.columns.clone(),
                    // Default a missing ref schema to this object's schema (a
                    // self-schema reference); the UI matches on schema.name.
                    to_schema: fk.ref_schema.clone().unwrap_or_else(|| schema_str.clone()),
                    to_table: fk.ref_table.clone(),
                    to_columns: fk.ref_columns.clone(),
                });
            }

            tables.push(GraphTable {
                id: node.id,
                schema: schema_str.clone(),
                name: detail.name,
                kind: detail.kind,
                columns,
            });
        }

        let mut visible_edges = Vec::new();
        for edge in edges {
            if self.authorize(conn_id, user_id, Some(&edge.to_schema), "db_browse").await.is_ok() { visible_edges.push(edge); }
        }
        self.authorize(conn_id, user_id, Some(schema), "db_browse").await?;
        Ok(SchemaGraph {
            schema: schema.to_string(),
            tables,
            edges: visible_edges,
            relationships,
            truncated,
        })
    }

    /// Run a query/command and record it in history (best-effort).
    ///
    /// `user_id` identifies the caller and is written into `db_query_history.user_id`
    /// (since migration 0042) so history can later be filtered per-user.
    ///
    /// When `req.query_id` is set, the query is registered in the in-flight map
    /// for its duration (via [`InFlightGuard`], which removes it on drop even if
    /// the driver errors or the future is cancelled), so a concurrent
    /// [`Self::cancel`] can issue engine-native cancellation against it. The
    /// driver fills the [`CancelToken`] with its native handle as it starts.
    pub async fn run(&self, conn_id: &Id, user_id: &Id, req: &QueryRequest) -> Result<QueryResult> {
        let req = &QueryRequest { node: crate::access::child(req.node.as_deref()), ..req.clone() };
        self.execution_access(conn_id, user_id, req).await?;
        self.guard_write(conn_id, req).await?;
        let child = crate::access::child(req.node.as_deref());
        let mut r = self.resolve(conn_id, user_id, child.as_deref(), "db_query").await?;
        if self.is_enforced(conn_id).await? && crate::access::operations(r.config.engine,&req.statement)?.iter().any(|op|*op!="db_query") {
            // Direct nonproduction mutations passed their separate resource gate.
            // Approved changes use another resolver operation entirely.
            r.config.params.as_object_mut().unwrap().remove("__read_only_execution");
        }
        self.verify_native(conn_id, user_id, &r).await?;
        let token = CancelToken::new();

        // Without a `query_id` there is nothing to cancel or re-attach to —
        // request-scoped execution (agents over MCP, widgets) stays inline.
        let Some(qid) = req.query_id.clone().filter(|s| !s.is_empty()) else {
            return self.execute_recorded(r, conn_id, user_id, req, &token).await;
        };

        let governed = r.config.params.get("__access_scope").is_some();
        let qid = if governed { format!("{user_id}:{qid}") } else { qid };
        // Register the in-flight query so a cancel can find it. The guard moves
        // into the detached task below, so the entry lives exactly as long as
        // the execution — not as long as the HTTP request.
        if let Ok(mut map) = self.in_flight.lock() {
            if map.contains_key(&qid) { return Err(Error::Conflict("query id is already running".into())); }
            map.insert(
                qid.clone(),
                InFlightQuery {
                    conn_id: conn_id.clone(),
                    user_id: user_id.clone(),
                    resolved: Some(r.clone()),
                    token: token.clone(),
                },
            );
        }
        let guard = InFlightGuard {
            map: Arc::clone(&self.in_flight),
            key: qid.clone(),
        };

        if governed {
            let _guard = guard;
            return self.execute_recorded(r, conn_id, user_id, req, &token).await;
        }
        // Detached execution: the query runs in its own task, so a dropped HTTP
        // request (page navigation, window close, network blip) no longer
        // cancels the database work — only an explicit `cancel` does. The task
        // hands the outcome back over a oneshot; when the waiter is already
        // gone, the outcome is parked in `finished` instead, for the client to
        // collect via [`Self::query_status`]. Outcomes are parked ONLY when
        // nobody was waiting, so the common delivered-response path retains
        // nothing.
        let (out_tx, out_rx) = tokio::sync::oneshot::channel();
        let svc = self.clone();
        let (cid, uid, req_owned) = (conn_id.clone(), user_id.clone(), req.clone());
        tokio::spawn(async move {
            let _guard = guard;
            let result = svc
                .execute_recorded(r, &cid, &uid, &req_owned, &token)
                .await;
            if let Err(result) = out_tx.send(result) {
                let outcome = result.map_err(|e| e.to_string());
                if let Ok(mut store) = svc.finished.lock() {
                    store.park(qid, cid, outcome, Instant::now());
                }
            }
            // `_guard` drops here — AFTER parking — so `query_status` never
            // reports "unknown" in the gap between running and parked.
        });
        out_rx
            .await
            .map_err(|_| Error::Internal("query task dropped its result".into()))?
    }

    /// Drive the resolved driver, apply opt-in masking, and record history —
    /// the shared tail of [`Self::run`] for both inline and detached execution.
    async fn execute_recorded(
        &self,
        r: Resolved,
        conn_id: &Id,
        user_id: &Id,
        req: &QueryRequest,
        token: &CancelToken,
    ) -> Result<QueryResult> {
        let started = Instant::now();
        self.execution_access(conn_id, user_id, req).await?;
        self.check_resolved_scope(conn_id, user_id, req.node.as_deref(), "db_query", &r).await?;
        let execution = r.driver.run_tracked(&r.config, req, token);
        tokio::pin!(execution);
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        let result = loop {
            tokio::select! {
                result = &mut execution => break result,
                _ = interval.tick() => {
                    let eligibility = async {
                        self.execution_access(conn_id, user_id, req).await?;
                        self.check_resolved_scope(conn_id, user_id, req.node.as_deref(), "db_query", &r).await
                    }.await;
                    if let Err(error) = eligibility {
                        if let Some(handle) = token.handle() {
                            let _ = r.driver.cancel(&r.config, &handle).await;
                        }
                        return Err(error);
                    }
                }
            }
        };
        self.execution_access(conn_id, user_id, req).await?;
        self.check_resolved_scope(conn_id, user_id, req.node.as_deref(), "db_query", &r).await?;
        let elapsed = started.elapsed().as_millis() as i64;

        // Apply server-side PII masking when the request opts in. Raw cell values
        // are passed through `otto_core::redact::redact_json` before leaving the
        // server — unmasked data never reaches the client when the flag is set.
        // Multi-statement batches carry later result sets in `more_results`, so
        // mask those too (never leak an unmasked cell just because it was the 2nd
        // statement).
        let result = result.map(|mut res| {
            if req.mask == Some(true) {
                mask_result(&mut res);
            }
            res
        });

        match &result {
            Ok(res) => {
                let _ = self
                    .repo
                    .add_history(
                        conn_id,
                        user_id,
                        &req.statement,
                        true,
                        elapsed,
                        res.stats.row_count as i64,
                        None,
                    )
                    .await;
            }
            Err(e) => {
                let _ = self
                    .repo
                    .add_history(conn_id, user_id, &req.statement, false, elapsed, 0, Some(&e.to_string()))
                    .await;
            }
        }
        result
    }

    /// Re-attach probe for a query previously submitted with `query_id`:
    /// `"running"` while its detached task executes, `"done"` (+ result/error)
    /// while its parked outcome is retained, `"unknown"` otherwise (never seen,
    /// delivered to a live waiter, or expired). Scoped to `conn_id` — a probe
    /// must not read another connection's outcome through a reused id.
    pub async fn query_status(&self, conn_id: &Id, user_id: &Id, query_id: &str) -> Result<QueryStatus> {
        self.authorize(conn_id, user_id, None, "discover").await?;
        if self.is_enforced(conn_id).await? {
            let key = format!("{user_id}:{query_id}");
            let running = self.in_flight.lock().is_ok_and(|m| m.get(&key).is_some_and(|q| &q.conn_id == conn_id));
            return Ok(QueryStatus { status: if running { "running" } else { "unknown" }, result: None, error: None });
        }
        Ok(self.legacy_query_status(conn_id, query_id))
    }

    fn legacy_query_status(&self, conn_id: &Id, query_id: &str) -> QueryStatus {
        let running = self
            .in_flight
            .lock()
            .map(|m| m.get(query_id).is_some_and(|q| &q.conn_id == conn_id))
            .unwrap_or(false);
        if running {
            return QueryStatus {
                status: "running",
                result: None,
                error: None,
            };
        }
        if let Ok(mut store) = self.finished.lock() {
            if let Some(f) = store.get(query_id, conn_id, Instant::now()) {
                return match &f.outcome {
                    Ok(res) => QueryStatus {
                        status: "done",
                        result: Some(res.clone()),
                        error: None,
                    },
                    Err(e) => QueryStatus {
                        status: "done",
                        result: None,
                        error: Some(e.clone()),
                    },
                };
            }
        }
        QueryStatus {
            status: "unknown",
            result: None,
            error: None,
        }
    }

    /// Run a query on behalf of an **agent over MCP**, enforcing read-only
    /// **unconditionally** — independent of the connection's write-guard. The
    /// statement is classified *before* the driver is touched ([`ensure_read_only`]),
    /// so a write/DDL (or a statement against a non-queryable connection kind) is
    /// refused without ever connecting. Rows are hard-capped at [`MCP_MAX_ROWS`] and
    /// PII masking is forced on, since the result lands in an agent's transcript.
    ///
    /// This is the trust boundary for LLM-supplied SQL: unlike [`Self::run`] /
    /// [`Self::guard_write`] (which only block writes on *guarded* connections), no
    /// write can pass here on any connection.
    pub async fn run_read_only(
        &self,
        conn_id: &Id,
        user_id: &Id,
        req: &QueryRequest,
    ) -> Result<QueryResult> {
        let conn = self.connections.get(conn_id).await?;
        let engine = Engine::from_kind(conn.kind).ok_or_else(|| {
            Error::Invalid(format!(
                "connection '{}' is not a queryable database (kind {:?})",
                conn.name, conn.kind
            ))
        })?;
        ensure_read_only(engine, &req.statement)?;
        let max_rows = Some(req.max_rows.unwrap_or(MCP_MAX_ROWS).min(MCP_MAX_ROWS));
        let safe = QueryRequest {
            statement: req.statement.clone(),
            max_rows,
            node: req.node.clone(),
            timeout_ms: req.timeout_ms,
            // Force the safe defaults: never confirm a write, never use the
            // explain/params side-channels, always mask PII for an agent.
            confirm_write: false,
            explain: false,
            params: None,
            query_id: None,
            mask: Some(true),
            offset: None,
        };
        self.run(conn_id, user_id, &safe).await
    }

    /// Export a (potentially huge) **uncapped** read result to a local file, in
    /// the chosen [`ExportFormat`], **streaming** through the driver's native
    /// cursor/stream so daemon memory stays bounded regardless of result size.
    ///
    /// Resolves the connection + SSH tunnel exactly like [`Self::run`] (the
    /// driver receives the resolved endpoint), then dispatches to
    /// [`crate::driver::Driver::export_to_path`]. The write-guard still applies —
    /// a write statement on a guarded connection is refused (export is for reads)
    /// — but here `confirm_write` is implicitly false (no UI confirmation path),
    /// so a guarded write is always blocked. Returns the rows/bytes written and
    /// the wall-clock duration.
    ///
    /// `user_id` is recorded in history like a normal run.
    #[allow(clippy::too_many_arguments)]
    pub async fn export_to_path(
        &self,
        conn_id: &Id,
        user_id: &Id,
        statement: &str,
        node: Option<&str>,
        format: crate::export::ExportFormat,
        max_rows: Option<usize>,
        dest: &std::path::Path,
    ) -> Result<(crate::export::ExportCounts, u64)> {
        self.require_local_path_access(conn_id, user_id).await?;
        // Reuse the write-gate: an export is a read; a write/DDL on a guarded
        // (production / read-only) connection is refused (no confirm path here).
        self.guard_export(conn_id, user_id, statement, node).await?;

        let child = crate::access::child(node);
        let r = self.resolve(conn_id, user_id, child.as_deref(), "db_export").await?;
        self.verify_native(conn_id, user_id, &r).await?;
        let started = Instant::now();
        let result = self.monitor_export(conn_id, user_id, statement, node, &r,
            r.driver.export_to_path(&r.config, statement, node, format, max_rows, dest)).await;
        let elapsed = started.elapsed().as_millis() as u64;

        // Record in history (best-effort), mirroring `run`.
        match &result {
            Ok(counts) => {
                let _ = self
                    .repo
                    .add_history(
                        conn_id,
                        user_id,
                        statement,
                        true,
                        elapsed as i64,
                        counts.rows as i64,
                        None,
                    )
                    .await;
            }
            Err(e) => {
                let _ = self
                    .repo
                    .add_history(conn_id, user_id, statement, false, elapsed as i64, 0, Some(&e.to_string()))
                    .await;
            }
        }
        result.map(|counts| (counts, elapsed))
    }

    /// Streaming variant of [`Self::export_to_path`] that writes through an
    /// arbitrary `w` instead of a local file — the HTTP `/db/export` handler
    /// passes a channel-backed writer so bytes stream straight to the browser
    /// without the daemon ever buffering the full result (the fix for the old
    /// `/db/export` truncation, spec §2.1). Same write-guard, resolve, and
    /// history semantics as `export_to_path`; returns `{rows, bytes}` + duration.
    #[allow(clippy::too_many_arguments)]
    pub async fn export_to_writer(
        &self,
        conn_id: &Id,
        user_id: &Id,
        statement: &str,
        node: Option<&str>,
        format: crate::export::ExportFormat,
        max_rows: Option<usize>,
        w: Box<dyn std::io::Write + Send>,
    ) -> Result<(crate::export::ExportCounts, u64)> {
        self.guard_export(conn_id, user_id, statement, node).await?;

        let child = crate::access::child(node);
        let r = self.resolve(conn_id, user_id, child.as_deref(), "db_export").await?;
        self.verify_native(conn_id, user_id, &r).await?;
        let started = Instant::now();
        let result = self.monitor_export(conn_id, user_id, statement, node, &r,
            r.driver.export_to_writer(&r.config, statement, node, format, max_rows, w)).await;
        let elapsed = started.elapsed().as_millis() as u64;

        // Record in history (best-effort), mirroring `export_to_path`.
        match &result {
            Ok(counts) => {
                let _ = self
                    .repo
                    .add_history(
                        conn_id,
                        user_id,
                        statement,
                        true,
                        elapsed as i64,
                        counts.rows as i64,
                        None,
                    )
                    .await;
            }
            Err(e) => {
                let _ = self
                    .repo
                    .add_history(conn_id, user_id, statement, false, elapsed as i64, 0, Some(&e.to_string()))
                    .await;
            }
        }
        result.map(|counts| (counts, elapsed))
    }

    /// Pre-flight the export write-guard *without* running the export — lets the
    /// streaming `/db/export` handler reject a guarded write with a clean HTTP
    /// error before it has committed to a 200 streaming response. (An export is a
    /// read; a write/DDL on a guarded connection is refused, `confirm_write`
    /// implicitly false — there is no export confirmation path.)
    pub async fn guard_export(
        &self,
        conn_id: &Id, user_id: &Id,
        statement: &str,
        node: Option<&str>,
    ) -> Result<()> {
        let guard_req = QueryRequest {
            statement: statement.to_string(),
            node: node.map(str::to_string),
            ..QueryRequest::default()
        };
        self.authorize(conn_id, user_id, node, "db_export").await?;
        if self.is_enforced(conn_id).await? {
            let conn = self.connections.get(conn_id).await?;
            let engine = Engine::from_kind(conn.kind).ok_or_else(|| Error::Invalid("not a database".into()))?;
            if crate::access::operations(engine, statement)? != vec!["db_query"] {
                return Err(Error::Forbidden("exports must contain only read queries".into()));
            }
        }
        self.execution_access(conn_id, user_id, &guard_req).await?;
        self.guard_write(conn_id, &guard_req).await
    }

    /// Import a local file into an existing table/collection. Parses the file,
    /// then imports per engine — respecting the write guard + `confirm_write` on
    /// every engine:
    /// - **SQL (MySQL/ClickHouse):** builds batched `INSERT`s and runs each batch
    ///   through the guarded `run` path (so guard/history/masking all apply).
    /// - **MongoDB:** guards once (an `insertMany` is a write), coerces CSV/TSV
    ///   string cells to inferred types (Mongo is schemaless), then `insertMany`s
    ///   in batches via the driver.
    ///
    /// Redis has no bulk-load surface (documented out).
    #[allow(clippy::too_many_arguments)]
    pub async fn import_from_path(
        &self,
        conn_id: &Id,
        user_id: &Id,
        local_path: &str,
        format: crate::import::ImportFormat,
        table: &str,
        batch_size: usize,
        confirm_write: bool,
    ) -> Result<ImportCounts> {
        self.require_local_path_access(conn_id, user_id).await?;
        let conn = self.connections.get(conn_id).await?;
        let engine = Engine::from_kind(conn.kind)
            .ok_or_else(|| Error::Invalid("connection is not a browsable database".into()))?;
        if matches!(engine, Engine::Redis) {
            return Err(Error::Invalid(
                "file import is not supported for redis".into(),
            ));
        }

        self.authorize(conn_id, user_id, None, "db_data").await?;
        if self.is_enforced(conn_id).await? && !matches!(engine, Engine::Mysql | Engine::Postgres) {
            return Err(crate::native_access::setup_error("restricted imports are unsupported for this engine"));
        }
        // Hard byte cap BEFORE reading: the request names any daemon-host file,
        // and the parse/render pipeline holds several copies of the data in RAM
        // — an unbounded multi-GB path would OOM the daemon.
        let meta = tokio::fs::metadata(local_path)
            .await
            .map_err(|e| Error::Invalid(format!("read import file: {e}")))?;
        if meta.len() > IMPORT_MAX_BYTES {
            return Err(Error::Invalid(format!(
                "import file is {} bytes — larger than the {} MiB import cap; \
                 split the file or load it with the engine's native bulk loader",
                meta.len(),
                IMPORT_MAX_BYTES / (1024 * 1024)
            )));
        }
        let bytes = tokio::fs::read(local_path)
            .await
            .map_err(|e| Error::Invalid(format!("read import file: {e}")))?;
        let parsed = crate::import::parse_rows(format, &bytes)?;

        match engine {
            // SQL engines import as batched INSERTs through the guarded `run`
            // path; identifier quoting AND literal escaping are engine-aware
            // (see `import::SqlFlavor`).
            Engine::Mysql | Engine::Clickhouse | Engine::Postgres => {
                let flavor = match engine {
                    Engine::Postgres => crate::import::SqlFlavor::Postgres,
                    Engine::Clickhouse => crate::import::SqlFlavor::Clickhouse,
                    _ => crate::import::SqlFlavor::Mysql,
                };
                let statements = crate::import::build_insert_statements(
                    table,
                    &parsed.columns,
                    &parsed.rows,
                    batch_size,
                    flavor,
                );
                let mut counts = ImportCounts::default();
                for stmt in statements {
                    let req = QueryRequest {
                        statement: stmt,
                        confirm_write,
                        ..QueryRequest::default()
                    };
                    // Routes through guard_write + history. A guarded connection
                    // without confirm_write fails here with write_blocked: 409.
                    let res = self.run(conn_id, user_id, &req).await?;
                    counts.rows += res.rows_affected.unwrap_or(0);
                    counts.batches += 1;
                }
                Ok(counts)
            }
            Engine::Mongodb => {
                // An insertMany is a write — guard once (a `db.<c>.insertMany`
                // shape classifies as a write) with the caller's confirmation.
                let guard_req = QueryRequest {
                    statement: format!("db.{table}.insertMany([])"),
                    confirm_write,
                    ..QueryRequest::default()
                };
                self.guard_write(conn_id, &guard_req).await?;
                // Mongo is schemaless: coerce CSV/TSV string cells to inferred
                // types (numbers/bools/null). NDJSON/JSON already carry types.
                let rows: Vec<Vec<serde_json::Value>> = if matches!(
                    format,
                    crate::import::ImportFormat::Csv | crate::import::ImportFormat::Tsv
                ) {
                    parsed
                        .rows
                        .iter()
                        .map(|r| r.iter().map(crate::import::coerce_scalar).collect())
                        .collect()
                } else {
                    parsed.rows.clone()
                };
                self.authorize(conn_id, user_id, None, "db_data").await?;
        let child = crate::access::child(None);
        let r = self.resolve(conn_id, user_id, child.as_deref(), "db_data").await?;
        self.verify_native(conn_id, user_id, &r).await?;
                let (inserted, batches) = r
                    .driver
                    .import_rows(&r.config, table, &parsed.columns, &rows, batch_size, None)
                    .await?;
                // Best-effort history entry (one row for the whole import).
                let _ = self
                    .repo
                    .add_history(
                        conn_id,
                        user_id,
                        &format!("import {inserted} document(s) → {table}"),
                        true,
                        0,
                        inserted as i64,
                        None,
                    )
                    .await;
                Ok(ImportCounts {
                    rows: inserted,
                    batches,
                })
            }
            Engine::Redis => unreachable!("redis rejected above"),
        }
    }

    /// Produce a structured [`DbQueryPlan`] for `statement`, dispatching to the
    /// engine's native EXPLAIN via the driver. The statement is EXPLAIN-wrapped
    /// (never executed raw), so this is read-only by construction — no write
    /// guard needed. The connection's SSH tunnel is established by `resolve`,
    /// exactly like [`Self::run`]. Redis has no plan surface (the driver default
    /// returns an `Invalid` error → 400).
    pub async fn query_plan(
        &self,
        conn_id: &Id, user_id: &Id,
        statement: &str,
        node: Option<&str>,
    ) -> Result<DbQueryPlan> {
        let child = crate::access::child(node);
        let r = self.resolve(conn_id, user_id, child.as_deref(), "db_query").await?;
        self.execution_access(conn_id, user_id, &QueryRequest { statement: statement.into(), node: node.map(str::to_owned), ..Default::default() }).await?;
        self.verify_native(conn_id, user_id, &r).await?;
        let result = r.driver.query_plan(&r.config, statement, node).await?;
        self.authorize(conn_id, user_id, node, "db_query").await?;
        Ok(result)
    }

    /// Validate a candidate **read** query by asking the engine for its plan,
    /// without returning rows. SQL engines run `EXPLAIN <sql>`; Mongo runs with
    /// the `explain` flag. Returns the plan rendered as text on success, or the
    /// engine's error message on failure (so the NL loop can feed it back to the
    /// drafter). The caller has already classified `sql` as a read.
    pub async fn explain_validate(
        &self,
        conn_id: &Id,
        user_id: &Id,
        node: Option<&str>,
        sql: &str,
    ) -> std::result::Result<String, String> {
        let conn = self
            .connections
            .get(conn_id)
            .await
            .map_err(|e| e.to_string())?;
        let engine = Engine::from_kind(conn.kind)
            .ok_or_else(|| "connection is not a browsable database".to_string())?;
        let req = match engine {
            Engine::Mysql | Engine::Clickhouse | Engine::Postgres => QueryRequest {
                statement: format!("EXPLAIN {sql}"),
                node: node.map(str::to_string),
                ..QueryRequest::default()
            },
            Engine::Mongodb => QueryRequest {
                statement: sql.to_string(),
                explain: true,
                node: node.map(str::to_string),
                ..QueryRequest::default()
            },
            // Redis has no SQL/plan surface; NL→SQL is gated off for it in the UI.
            Engine::Redis => return Err("EXPLAIN is not supported for Redis".to_string()),
        };
        match self.run(conn_id, user_id, &req).await {
            Ok(res) => Ok(render_plan_text(&res)),
            Err(e) => Err(e.to_string()),
        }
    }

    /// A compact, model-grounding summary of the schema under `node` (or the
    /// connection's default): up to `max_tables` tables, each as
    /// `table(col type, col type, …)`. Built from the same lazy tree the UI
    /// browses, so it's engine-agnostic. Best-effort — an object that fails to
    /// introspect is skipped.
    pub async fn schema_summary(
        &self,
        conn_id: &Id, user_id: &Id,
        node: Option<&str>,
        max_tables: usize,
    ) -> Result<String> {
        let schema = node_to_schema(node);
        let graph = self.schema_graph(conn_id, user_id, &schema, max_tables).await?;
        let mut out = String::new();
        for t in &graph.tables {
            let cols: Vec<String> = t
                .columns
                .iter()
                .map(|c| format!("{} {}", c.name, c.data_type))
                .collect();
            out.push_str(&format!("{}({})\n", t.name, cols.join(", ")));
        }
        if out.is_empty() {
            out.push_str("(no tables introspected)");
        }
        Ok(out)
    }

    /// A COMPLETE, model-grounding markdown schema for the active database — the
    /// context the DB Assistant agent reads from `SCHEMA.md` before drafting SQL.
    ///
    /// Unlike [`Self::schema_summary`] (one terse line per table) this renders one
    /// block per table with every column (`name type` + PK/FK/NOT-NULL flags) and
    /// a foreign-key section, so the agent can write correct JOINs without probing.
    /// Built from [`Self::schema_graph`] (engine-agnostic introspection) with a
    /// generous table cap; engines without FK metadata (Redis/Mongo) yield tables
    /// with no edges. See [`format_schema_context`] for the (unit-tested) layout.
    pub async fn schema_context(&self, conn_id: &Id, user_id: &Id, node: Option<&str>) -> Result<String> {
        let schema = node_to_schema(node);
        let graph = self
            .schema_graph(conn_id, user_id, &schema, SCHEMA_CONTEXT_MAX_TABLES)
            .await?;
        Ok(format_schema_context(&graph))
    }

    /// Cancel an in-flight query (issued via [`Self::run`] with the same
    /// `query_id`) using engine-native cancellation. The cancel runs on a FRESH
    /// connection re-resolved to the query's endpoint — you can't `KILL` on the
    /// blocked one. An unknown id, a query that already finished, or an engine
    /// without a captured handle is a no-op success (never an error/panic): the
    /// caller just wants the query gone, and a race with completion is benign.
    ///
    /// `conn_id` is the connection the client *thinks* the query belongs to (the
    /// route is connection-scoped for role-gating); we additionally require the
    /// registry entry to match it, so a cancel can't reach across connections.
    pub async fn cancel(&self, conn_id: &Id, user_id: &Id, query_id: &str) -> Result<()> {
        self.authorize(conn_id, user_id, None, "discover").await?;
        let key = if self.is_enforced(conn_id).await? { format!("{user_id}:{query_id}") } else { query_id.into() };
        let target = self.in_flight.lock().map_err(|_| Error::Internal("in-flight registry poisoned".into()))?
            .get(&key).filter(|q| &q.conn_id == conn_id && &q.user_id == user_id).cloned();
        if let Some(target) = target {
            if let (Some(handle), Some(r)) = (target.token.handle(), target.resolved) {
                r.driver.cancel(&r.config, &handle).await?;
            }
        }
        Ok(())
    }

    pub async fn completion(
        &self,
        conn_id: &Id, user_id: &Id,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        let child = crate::access::child(ctx.database.as_deref().or(ctx.node.as_deref()));
        let r = self.resolve(conn_id, user_id, child.as_deref(), "db_browse").await?;
        if self.is_enforced(conn_id).await? {
            let schema = child.ok_or_else(|| Error::Forbidden("select an authorized database before requesting completion".into()))?;
            let graph = self.schema_graph(conn_id, user_id, &schema, 100).await?;
            let mut items = Vec::new();
            for table in graph.tables {
                items.push(crate::types::CompletionItem::new(table.name, crate::types::CompletionKind::Table));
                for column in table.columns {
                    items.push(crate::types::CompletionItem::new(column.name, crate::types::CompletionKind::Column));
                }
            }
            return Ok(CompletionResponse { items });
        }
        r.driver.completion(&r.config, ctx).await
    }

    /// Drop the cached completion snapshot for a connection so the next
    /// completion re-introspects the live schema. Backs the UI "Refresh schema"
    /// action — keeping smart completion in sync with a schema the user just
    /// changed — and is a no-op for engines without a snapshot cache (Redis).
    pub async fn refresh_completion_cache(&self, conn_id: &Id, user_id: &Id) -> Result<()> {
        let child = crate::access::child(None);
        let r = self.resolve(conn_id, user_id, child.as_deref(), "discover").await?;
        r.driver.invalidate_completion_cache(&r.config).await;
        Ok(())
    }

    // -- Persistence pass-throughs (saved queries / history / dashboards) ----

    /// All saved queries for a workspace — root / ws-Admin view.
    pub async fn list_saved(&self, ws: &Id) -> Result<Vec<SavedQuery>> {
        self.repo.list_saved(ws).await
    }
    /// Saved queries for a workspace scoped to a single user — non-admin view.
    pub async fn list_saved_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<SavedQuery>> {
        let mut visible = Vec::new();
        for query in self.repo.list_saved_for_user(ws, user_id).await? {
            if let Some(conn) = &query.connection_id {
                if self.authorize(conn, user_id, None, "db_query").await.is_err() { continue; }
            }
            visible.push(query);
        }
        Ok(visible)
    }
    pub async fn create_saved(&self, q: NewSavedQuery) -> Result<SavedQuery> {
        if let Some(conn) = &q.connection_id { self.authorize(conn, &q.created_by, None, "db_query").await?; }
        self.repo.create_saved(q).await
    }
    pub async fn get_saved(&self, id: &Id) -> Result<SavedQuery> {
        self.repo.get_saved(id).await
    }
    pub async fn update_saved(
        &self,
        id: &Id,
        name: Option<&str>,
        statement: Option<&str>,
    ) -> Result<SavedQuery> {
        self.repo.update_saved(id, name, statement).await
    }
    pub async fn delete_saved(&self, id: &Id) -> Result<()> {
        self.repo.delete_saved(id).await
    }
    /// All history for a connection — root / ws-Admin view.
    pub async fn list_history(&self, conn_id: &Id, limit: i64) -> Result<Vec<HistoryEntry>> {
        self.repo.list_history(conn_id, limit).await
    }
    /// History for a connection scoped to a single user — non-admin view.
    pub async fn list_history_for_user(
        &self,
        conn_id: &Id,
        user_id: &Id,
        limit: i64,
    ) -> Result<Vec<HistoryEntry>> {
        self.authorize(conn_id, user_id, None, "db_query").await?;
        self.repo.list_history_for_user(conn_id, user_id, limit).await
    }

    /// All dashboards for a workspace — root / ws-Admin view.
    pub async fn list_dashboards(&self, ws: &Id) -> Result<Vec<Dashboard>> {
        self.repo.list_dashboards(ws).await
    }
    /// Dashboards for a workspace scoped to a single user — non-admin view (#L13).
    pub async fn list_dashboards_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<Dashboard>> {
        self.repo.list_dashboards_for_user(ws, user_id).await
    }
    pub async fn get_dashboard(&self, id: &Id) -> Result<Dashboard> {
        self.repo.get_dashboard(id).await
    }
    pub async fn create_dashboard(&self, ws: &Id, name: &str, by: &Id) -> Result<Dashboard> {
        self.repo.create_dashboard(ws, name, by).await
    }
    pub async fn update_dashboard(
        &self,
        id: &Id,
        name: Option<&str>,
        layout: Option<&Value>,
        refresh_secs: Option<Option<i64>>,
    ) -> Result<Dashboard> {
        self.repo.update_dashboard(id, name, layout, refresh_secs).await
    }
    pub async fn delete_dashboard(&self, id: &Id) -> Result<()> {
        self.repo.delete_dashboard(id).await
    }

    /// All widgets for a workspace — root / ws-Admin view.
    pub async fn list_widgets(&self, ws: &Id) -> Result<Vec<Widget>> {
        self.repo.list_widgets(ws).await
    }
    /// Widgets for a workspace scoped to a single user — non-admin view (#L13).
    pub async fn list_widgets_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<Widget>> {
        let mut visible = Vec::new();
        for widget in self.repo.list_widgets_for_user(ws, user_id).await? {
            if self.authorize(&widget.connection_id, user_id, None, "db_query").await.is_ok() { visible.push(widget); }
        }
        Ok(visible)
    }
    pub async fn get_widget(&self, id: &Id) -> Result<Widget> {
        self.repo.get_widget(id).await
    }
    pub async fn create_widget(&self, w: NewWidget) -> Result<Widget> {
        self.authorize(&w.connection_id, &w.created_by, None, "db_query").await?;
        self.repo.create_widget(w).await
    }
    #[allow(clippy::too_many_arguments)]
    pub async fn update_widget(
        &self,
        id: &Id,
        dashboard_id: Option<Option<&str>>,
        title: Option<&str>,
        statement: Option<&str>,
        viz: Option<&str>,
        mapping: Option<&Value>,
        options: Option<&Value>,
    ) -> Result<Widget> {
        self.repo
            .update_widget(id, dashboard_id, title, statement, viz, mapping, options)
            .await
    }
    pub async fn delete_widget(&self, id: &Id) -> Result<()> {
        self.repo.delete_widget(id).await
    }

    /// Run a widget's query for rendering.
    ///
    /// `user_id` is threaded through to history recording (since migration 0042).
    pub async fn run_widget(&self, id: &Id, user_id: &Id) -> Result<QueryResult> {
        let widget = self.repo.get_widget(id).await?;
        let req = QueryRequest {
            statement: widget.statement,
            max_rows: Some(5000),
            ..Default::default()
        };
        self.run(&widget.connection_id, user_id, &req).await
    }
}

/// Render a plan `QueryResult` as compact text for display + drafter feedback.
fn render_plan_text(res: &crate::types::QueryResult) -> String {
    let mut out = String::new();
    for row in &res.rows {
        let line: Vec<String> = row
            .iter()
            .map(|c| match c {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .collect();
        out.push_str(&line.join(" | "));
        out.push('\n');
    }
    if out.is_empty() {
        out.push_str("(empty plan)");
    }
    out
}

/// Derive the schema/database name to introspect from a UI node id.
///
/// RC2 fix: the Database tree passes the *active database* node — sometimes a
/// `db:`-tagged [`NodePath`] (`db:shop/table:orders`), sometimes the bare schema
/// name (`player_details`). The old code only handled the first form and silently
/// produced `""` for the second, so the assistant got `(no tables introspected)`.
/// Now an un-tagged node falls back to its raw name. `None` → the connection's
/// default schema (empty string, resolved downstream).
fn node_to_schema(node: Option<&str>) -> String {
    node.map(|n| {
        NodePath::parse(n)
            .get("db")
            .map(str::to_string)
            .unwrap_or_else(|| n.to_string())
    })
    .unwrap_or_default()
}

/// Render a [`SchemaGraph`] as a COMPLETE markdown schema for the DB Assistant —
/// one block per table (every column as `name type` with PK/FK/NOT-NULL flags)
/// plus a foreign-key section. Pure + unit-tested; never panics on an empty graph.
fn format_schema_context(graph: &SchemaGraph) -> String {
    let schema_label = if graph.schema.trim().is_empty() {
        "(default)"
    } else {
        graph.schema.as_str()
    };
    let mut out = format!(
        "# Database schema: {schema_label}\n{} table(s)/collection(s).",
        graph.tables.len()
    );
    if graph.truncated {
        out.push_str(" (list truncated — more tables exist than shown)");
    }
    if !graph.relationships {
        out.push_str(" (this engine exposes no foreign-key relationships)");
    }
    out.push_str("\n\n");

    for t in &graph.tables {
        out.push_str(&format!("## {}\n", t.name));
        if t.columns.is_empty() {
            out.push_str("(no columns introspected)\n\n");
            continue;
        }
        for c in &t.columns {
            let mut flags: Vec<&str> = Vec::new();
            if c.primary_key {
                flags.push("PK");
            }
            if c.foreign_key {
                flags.push("FK");
            }
            if !c.nullable {
                flags.push("NOT NULL");
            }
            let suffix = if flags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", flags.join(", "))
            };
            out.push_str(&format!("- {} {}{}\n", c.name, c.data_type, suffix));
        }
        out.push('\n');
    }

    if !graph.edges.is_empty() {
        out.push_str("## Foreign keys\n");
        for e in &graph.edges {
            out.push_str(&format!(
                "- {}({}) -> {}.{}({})\n",
                e.from_table,
                e.from_columns.join(", "),
                e.to_schema,
                e.to_table,
                e.to_columns.join(", "),
            ));
        }
        out.push('\n');
    }
    out
}

/// Adapts `DbViewerService::explain_validate` to the `nl::SqlValidator` trait so
/// the NL loop can validate candidates without knowing about the service.
pub struct ServiceValidator<'a> {
    pub db: &'a DbViewerService,
    pub conn_id: Id,
    pub user_id: Id,
    pub node: Option<String>,
}

#[async_trait::async_trait]
impl crate::nl::SqlValidator for ServiceValidator<'_> {
    async fn validate(&self, sql: &str) -> std::result::Result<String, String> {
        self.db
            .explain_validate(&self.conn_id, &self.user_id, self.node.as_deref(), sql)
            .await
    }
}

#[cfg(test)]
mod tests {
    //! Unit tests for the in-flight cancel registry — the decision logic
    //! ([`cancel_handle_for`]) and the RAII [`InFlightGuard`] — without standing
    //! up a full `DbViewerService` (which needs a DB pool + secret store). These
    //! prove the security/robustness properties: a cancel only targets a matching
    //! connection's query, only when a native handle was captured, and a finished
    //! query leaves no stale entry a later cancel could hit.

    use super::*;

    fn entry(conn_id: &str, handle: Option<QueryHandle>) -> InFlightQuery {
        let token = CancelToken::new();
        if let Some(h) = handle {
            token.set(h);
        }
        InFlightQuery {
            conn_id: conn_id.to_string(),
            user_id: "test".into(),
            resolved: None,
            token,
        }
    }

    // --- ensure_read_only: the MCP read-only policy gate (no DB needed) --------

    #[test]
    fn ensure_read_only_allows_sql_reads() {
        for stmt in [
            "SELECT * FROM users",
            "  select 1",
            "WITH t AS (SELECT 1) SELECT * FROM t",
            "SHOW TABLES",
            "DESCRIBE users",
            "EXPLAIN SELECT 1",
            "-- a comment\nSELECT 2",
        ] {
            assert!(
                ensure_read_only(Engine::Mysql, stmt).is_ok(),
                "expected read to be allowed: {stmt:?}"
            );
        }
    }

    #[test]
    fn ensure_read_only_blocks_sql_writes_and_injection() {
        for stmt in [
            "INSERT INTO t VALUES (1)",
            "UPDATE t SET a=1",
            "DELETE FROM t",
            "DROP TABLE t",
            "ALTER TABLE t ADD c INT",
            "TRUNCATE t",
            "SELECT 1; DROP TABLE t", // injection via a trailing statement
            "SET GLOBAL x=1",
        ] {
            let err = ensure_read_only(Engine::Mysql, stmt).unwrap_err();
            assert!(
                matches!(err, Error::Forbidden(_)),
                "expected Forbidden for write {stmt:?}, got {err:?}"
            );
        }
    }

    #[test]
    fn ensure_read_only_empty_statement_is_invalid() {
        let err = ensure_read_only(Engine::Mysql, "   ").unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    }

    #[test]
    fn ensure_read_only_redis_reads_vs_writes() {
        assert!(ensure_read_only(Engine::Redis, "GET key").is_ok());
        assert!(ensure_read_only(Engine::Redis, "SCAN 0").is_ok());
        assert!(matches!(
            ensure_read_only(Engine::Redis, "SET k v").unwrap_err(),
            Error::Forbidden(_)
        ));
        assert!(matches!(
            ensure_read_only(Engine::Redis, "DEL k").unwrap_err(),
            Error::Forbidden(_)
        ));
    }

    #[test]
    fn ensure_read_only_mongo_reads_vs_writes() {
        assert!(ensure_read_only(Engine::Mongodb, "db.users.find({})").is_ok());
        assert!(matches!(
            ensure_read_only(Engine::Mongodb, "db.users.deleteOne({})").unwrap_err(),
            Error::Forbidden(_)
        ));
    }

    #[test]
    fn cancel_handle_for_returns_captured_handle_on_match() {
        let mut map = HashMap::new();
        map.insert(
            "q1".to_string(),
            entry("conn-A", Some(QueryHandle::MysqlConnId(7))),
        );
        let h = cancel_handle_for(&map, &"conn-A".to_string(), "q1");
        assert!(matches!(h, Some(QueryHandle::MysqlConnId(7))));
    }

    #[test]
    fn cancel_handle_for_unknown_query_is_none() {
        let map: HashMap<String, InFlightQuery> = HashMap::new();
        assert!(cancel_handle_for(&map, &"conn-A".to_string(), "nope").is_none());
    }

    #[test]
    fn cancel_handle_for_wrong_connection_is_none() {
        // The query exists but under a different connection — a cancel must not
        // reach across connections.
        let mut map = HashMap::new();
        map.insert(
            "q1".to_string(),
            entry("conn-A", Some(QueryHandle::MysqlConnId(7))),
        );
        assert!(cancel_handle_for(&map, &"conn-B".to_string(), "q1").is_none());
    }

    #[test]
    fn cancel_handle_for_no_captured_handle_is_none() {
        // Right connection + known id, but the driver never set a native handle
        // (e.g. Redis/Mongo, or the query hadn't reached the capture point).
        let mut map = HashMap::new();
        map.insert("q1".to_string(), entry("conn-A", None));
        assert!(cancel_handle_for(&map, &"conn-A".to_string(), "q1").is_none());
    }

    // ---- FinishedStore: parked outcomes for detached-query re-attach ------

    fn ok_result() -> QueryResult {
        QueryResult::empty()
    }

    #[test]
    fn finished_store_parks_and_serves_scoped_to_connection() {
        let mut store = FinishedStore::default();
        let now = Instant::now();
        store.park("q1".into(), "conn-A".into(), Ok(ok_result()), now);
        // Same connection → served; other connection → invisible.
        assert!(store.get("q1", &"conn-A".to_string(), now).is_some());
        assert!(store.get("q1", &"conn-B".to_string(), now).is_none());
        assert!(store.get("q2", &"conn-A".to_string(), now).is_none());
    }

    #[test]
    fn finished_store_expires_entries_past_ttl() {
        let mut store = FinishedStore::default();
        let now = Instant::now();
        store.park("q1".into(), "conn-A".into(), Err("boom".into()), now);
        assert!(store.get("q1", &"conn-A".to_string(), now).is_some());
        let later = now + FINISHED_TTL + Duration::from_secs(1);
        assert!(
            store.get("q1", &"conn-A".to_string(), later).is_none(),
            "entry must expire after the TTL"
        );
    }

    #[test]
    fn finished_store_evicts_oldest_beyond_cap() {
        let mut store = FinishedStore::default();
        let base = Instant::now();
        for i in 0..FINISHED_MAX {
            store.park(
                format!("q{i}"),
                "conn-A".into(),
                Ok(ok_result()),
                base + Duration::from_millis(i as u64),
            );
        }
        // One over the cap evicts the OLDEST (q0), keeps everything newer.
        store.park(
            "q-new".into(),
            "conn-A".into(),
            Ok(ok_result()),
            base + Duration::from_secs(1),
        );
        let now = base + Duration::from_secs(1);
        assert!(store.get("q0", &"conn-A".to_string(), now).is_none());
        assert!(store.get("q1", &"conn-A".to_string(), now).is_some());
        assert!(store.get("q-new", &"conn-A".to_string(), now).is_some());
    }

    // ---- RC2: node → schema derivation -----------------------------------

    #[test]
    fn node_to_schema_handles_tagged_bare_and_none() {
        // A `db:`-tagged NodePath → the db segment.
        assert_eq!(node_to_schema(Some("db:shop/table:orders")), "shop");
        assert_eq!(node_to_schema(Some("db:player_details")), "player_details");
        // RC2: a bare active-db node (no `db:` tag) → its raw name (was "" before).
        assert_eq!(node_to_schema(Some("player_details")), "player_details");
        // No node → the connection default (empty schema, resolved downstream).
        assert_eq!(node_to_schema(None), "");
    }

    // ---- schema_context formatting ---------------------------------------

    fn graph_col(name: &str, ty: &str, pk: bool, fk: bool, nullable: bool) -> GraphColumn {
        GraphColumn {
            name: name.into(),
            data_type: ty.into(),
            nullable,
            primary_key: pk,
            foreign_key: fk,
        }
    }

    #[test]
    fn format_schema_context_renders_tables_columns_flags_and_fks() {
        let graph = SchemaGraph {
            schema: "shop".into(),
            tables: vec![
                GraphTable {
                    id: "db:shop/table:orders".into(),
                    schema: "shop".into(),
                    name: "orders".into(),
                    kind: NodeKind::Table,
                    columns: vec![
                        graph_col("id", "int", true, false, false),
                        graph_col("customer_id", "int", false, true, false),
                        graph_col("note", "text", false, false, true),
                    ],
                },
                GraphTable {
                    id: "db:shop/table:customers".into(),
                    schema: "shop".into(),
                    name: "customers".into(),
                    kind: NodeKind::Table,
                    columns: vec![graph_col("id", "int", true, false, false)],
                },
            ],
            edges: vec![GraphEdge {
                name: "fk_customer".into(),
                from_table: "orders".into(),
                from_columns: vec!["customer_id".into()],
                to_schema: "shop".into(),
                to_table: "customers".into(),
                to_columns: vec!["id".into()],
            }],
            relationships: true,
            truncated: false,
        };
        let md = format_schema_context(&graph);
        assert!(md.contains("# Database schema: shop"));
        assert!(md.contains("2 table(s)"));
        assert!(md.contains("## orders"));
        assert!(md.contains("## customers"));
        // Column lines carry type + the right flags.
        assert!(md.contains("- id int [PK, NOT NULL]"));
        assert!(md.contains("- customer_id int [FK, NOT NULL]"));
        assert!(md.contains("- note text\n")); // nullable, no flags
        // FK section resolves to schema.table(cols).
        assert!(md.contains("## Foreign keys"));
        assert!(md.contains("orders(customer_id) -> shop.customers(id)"));
    }

    #[test]
    fn format_schema_context_empty_and_no_relationships() {
        let graph = SchemaGraph {
            schema: String::new(),
            tables: Vec::new(),
            edges: Vec::new(),
            relationships: false,
            truncated: true,
        };
        let md = format_schema_context(&graph);
        assert!(md.contains("# Database schema: (default)"));
        assert!(md.contains("0 table(s)"));
        assert!(md.contains("truncated"));
        assert!(md.contains("no foreign-key relationships"));
        // No FK section when there are no edges.
        assert!(!md.contains("## Foreign keys"));
    }

    #[test]
    fn in_flight_guard_removes_its_entry_on_drop() {
        let map: Arc<std::sync::Mutex<HashMap<String, InFlightQuery>>> =
            Arc::new(std::sync::Mutex::new(HashMap::new()));
        map.lock()
            .unwrap()
            .insert("q1".to_string(), entry("conn-A", None));
        {
            let _guard = InFlightGuard {
                map: Arc::clone(&map),
                key: "q1".to_string(),
            };
            assert!(map.lock().unwrap().contains_key("q1"));
        }
        // Guard dropped → entry gone, so a later cancel can't target it.
        assert!(map.lock().unwrap().is_empty());
    }
}
