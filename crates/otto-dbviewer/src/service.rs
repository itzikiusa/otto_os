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

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

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
use crate::tunnel::SshTunnel;
use crate::types::{
    statement_is_write, Capabilities, CompletionContext, CompletionResponse, Engine, NodePath,
    ObjectDetail, QueryRequest, QueryResult, ResolvedConfig, SchemaNode, TestResult,
};

/// Stable marker prefixed to the write-gate rejection message so the UI can
/// recognise it (and prompt for a typed confirmation) without string-matching
/// prose. The text after it stays human-readable.
pub const WRITE_BLOCKED_PREFIX: &str = "write_blocked: ";

/// Evict cached SSH tunnels that haven't been used for longer than this on the
/// next `resolve` — dropping them kills the `ssh` child via `SshTunnel::Drop`.
const TUNNEL_IDLE_TTL: Duration = Duration::from_secs(600);

/// A cached SSH tunnel kept alive between operations.
struct CachedTunnel {
    tunnel: Arc<SshTunnel>,
    last_used: Instant,
}

#[derive(Clone)]
pub struct DbViewerService {
    connections: ConnectionsRepo,
    secrets: Arc<dyn SecretStore>,
    repo: DbExplorerRepo,
    registry: Registry,
    /// Live SSH tunnels, keyed by connection id, reused across operations.
    tunnels: Arc<Mutex<HashMap<Id, CachedTunnel>>>,
}

/// A driver + endpoint resolved for one operation. The `_tunnel` clone, when
/// present, keeps a reference to the (cached) SSH forward alive for at least
/// the lifetime of this struct; the cache holds the other reference, so the
/// tunnel persists after the operation completes.
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
        }
    }

    /// Fetch a stored connection profile (for workspace/role resolution).
    pub async fn get_connection(&self, id: &Id) -> Result<Connection> {
        self.connections.get(id).await
    }

    /// Static capabilities for the engine behind a connection.
    pub async fn capabilities(&self, conn_id: &Id) -> Result<Capabilities> {
        let conn = self.connections.get(conn_id).await?;
        let engine = Engine::from_kind(conn.kind).ok_or_else(|| {
            otto_core::Error::Invalid(format!(
                "connection kind '{}' is not a browsable database",
                conn.kind.as_str()
            ))
        })?;
        Ok(self.registry.get(engine).capabilities())
    }

    async fn resolve(&self, conn_id: &Id) -> Result<Resolved> {
        let conn = self.connections.get(conn_id).await?;
        let secret = match &conn.secret_ref {
            Some(r) => self.secrets.get(r)?,
            None => None,
        };
        let parsed = config::parse(&conn, secret)?;
        let engine = parsed.config.engine;
        let driver = self.registry.get(engine);
        let mut config = parsed.config;
        let tunnel = match parsed.ssh {
            Some(ssh) => Some(self.tunnel_for(conn_id, &ssh, &config, engine).await?),
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
        Ok(Resolved {
            driver,
            config,
            _tunnel: tunnel,
        })
    }

    /// Get a live SSH tunnel for `conn_id`, reusing a cached one when it's still
    /// alive (and re-pointing at its local port), else opening a fresh tunnel.
    /// Idle tunnels are evicted on the way through, which drops their `ssh`
    /// child. Kept alive in the cache after this returns — the caller's clone is
    /// just an extra reference, not the sole owner.
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

    pub async fn test(&self, conn_id: &Id) -> Result<TestResult> {
        let r = self.resolve(conn_id).await?;
        r.driver.test(&r.config).await
    }

    pub async fn schema_root(&self, conn_id: &Id) -> Result<Vec<SchemaNode>> {
        let r = self.resolve(conn_id).await?;
        r.driver.schema_root(&r.config).await
    }

    pub async fn schema_children(
        &self,
        conn_id: &Id,
        path: &str,
        filter: Option<&str>,
    ) -> Result<Vec<SchemaNode>> {
        let r = self.resolve(conn_id).await?;
        let node = NodePath::parse(path);
        // An empty/whitespace filter is treated as "no filter".
        let filter = filter.map(str::trim).filter(|s| !s.is_empty());
        r.driver.schema_children(&r.config, &node, filter).await
    }

    pub async fn object_detail(&self, conn_id: &Id, path: &str) -> Result<ObjectDetail> {
        let r = self.resolve(conn_id).await?;
        let node = NodePath::parse(path);
        r.driver.object_detail(&r.config, &node).await
    }

    /// Production / read-only guardrail. When a connection is `Prod` or
    /// `read_only`, a statement classified as a write/DDL is refused unless the
    /// request carries `confirm_write`. Classification is conservative (unknown
    /// counts as a write), and `explain` requests never write, so they pass.
    /// The error is a 409 with a [`WRITE_BLOCKED_PREFIX`]-tagged message so the
    /// UI can detect it and ask for a typed confirmation.
    async fn guard_write(&self, conn_id: &Id, req: &QueryRequest) -> Result<()> {
        if req.confirm_write || req.explain {
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

    /// Run a query/command and record it in history (best-effort).
    pub async fn run(&self, conn_id: &Id, req: &QueryRequest) -> Result<QueryResult> {
        self.guard_write(conn_id, req).await?;
        let r = self.resolve(conn_id).await?;
        let started = Instant::now();
        let result = r.driver.run(&r.config, req).await;
        let elapsed = started.elapsed().as_millis() as i64;
        match &result {
            Ok(res) => {
                let _ = self
                    .repo
                    .add_history(
                        conn_id,
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
                    .add_history(conn_id, &req.statement, false, elapsed, 0, Some(&e.to_string()))
                    .await;
            }
        }
        result
    }

    pub async fn completion(
        &self,
        conn_id: &Id,
        ctx: &CompletionContext,
    ) -> Result<CompletionResponse> {
        let r = self.resolve(conn_id).await?;
        r.driver.completion(&r.config, ctx).await
    }

    // -- Persistence pass-throughs (saved queries / history / dashboards) ----

    pub async fn list_saved(&self, ws: &Id) -> Result<Vec<SavedQuery>> {
        self.repo.list_saved(ws).await
    }
    pub async fn create_saved(&self, q: NewSavedQuery) -> Result<SavedQuery> {
        self.repo.create_saved(q).await
    }
    pub async fn get_saved(&self, id: &Id) -> Result<SavedQuery> {
        self.repo.get_saved(id).await
    }
    pub async fn delete_saved(&self, id: &Id) -> Result<()> {
        self.repo.delete_saved(id).await
    }
    pub async fn list_history(&self, conn_id: &Id, limit: i64) -> Result<Vec<HistoryEntry>> {
        self.repo.list_history(conn_id, limit).await
    }

    pub async fn list_dashboards(&self, ws: &Id) -> Result<Vec<Dashboard>> {
        self.repo.list_dashboards(ws).await
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

    pub async fn list_widgets(&self, ws: &Id) -> Result<Vec<Widget>> {
        self.repo.list_widgets(ws).await
    }
    pub async fn get_widget(&self, id: &Id) -> Result<Widget> {
        self.repo.get_widget(id).await
    }
    pub async fn create_widget(&self, w: NewWidget) -> Result<Widget> {
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
    pub async fn run_widget(&self, id: &Id) -> Result<QueryResult> {
        let widget = self.repo.get_widget(id).await?;
        let req = QueryRequest {
            statement: widget.statement,
            max_rows: Some(5000),
            ..Default::default()
        };
        self.run(&widget.connection_id, &req).await
    }
}
