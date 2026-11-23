//! SSH tunnel (SOCKS5) for IP-whitelisted upstreams.

use super::*;

// ===========================================================================
// SSH tunnel (SOCKS5) for IP-whitelisted upstreams
// ===========================================================================

/// How long an idle cached tunnel is kept before it is torn down. Mirrors the
/// DB Explorer's tunnel cache (`otto-dbviewer`).
pub(crate) const TUNNEL_IDLE_TTL: Duration = Duration::from_secs(600);

pub(crate) struct CachedTunnel {
    tunnel: Arc<SshTunnel>,
    last_used: Instant,
}

/// Daemon-global cache of live SOCKS5 tunnels, keyed by the bastion identity
/// (`host|port|user|identity`) so distinct requests through the same bastion
/// share one `ssh` process. The child is killed on `Drop` once evicted.
pub(crate) fn tunnel_cache() -> &'static StdMutex<HashMap<String, CachedTunnel>> {
    static C: OnceLock<StdMutex<HashMap<String, CachedTunnel>>> = OnceLock::new();
    C.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Build an [`SshTunnelConfig`] from an `ssh`-kind connection's params. The
/// shape matches what the Connections page already stores (`host`/`port`/`user`/
/// `identity_file`); auth flows through the system `ssh` client (agent /
/// `~/.ssh/config` / identity file), so no secret is needed here.
pub(crate) fn ssh_config_from_conn(conn: &Connection) -> Result<SshTunnelConfig, String> {
    let p = &conn.params;
    let host = p
        .get("host")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("SSH connection '{}' has no host", conn.name))?;
    let user = p
        .get("user")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "SSH connection '{}' has no user — required to use it as an API tunnel",
                conn.name
            )
        })?;
    let port = match p.get("port") {
        None | Some(Value::Null) => 22,
        Some(Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u16::try_from(v).ok())
            .ok_or_else(|| format!("SSH connection '{}' has an invalid port", conn.name))?,
        Some(Value::String(s)) if s.is_empty() => 22,
        Some(Value::String(s)) => s
            .parse::<u16>()
            .map_err(|_| format!("SSH connection '{}' has an invalid port", conn.name))?,
        Some(_) => {
            return Err(format!(
                "SSH connection '{}' has an invalid port",
                conn.name
            ));
        }
    };
    let identity_file = p
        .get("identity_file")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let proxy_jump = p
        .get("jump")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(SshTunnelConfig {
        host: host.to_string(),
        port,
        user: user.to_string(),
        identity_file,
        proxy_jump,
    })
}

fn tunnel_cache_key(cfg: &SshTunnelConfig) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        cfg.host,
        cfg.port,
        cfg.user,
        cfg.identity_file.as_deref().unwrap_or(""),
        cfg.proxy_jump.as_deref().unwrap_or(""),
    )
}

/// Sweep idle or dead SOCKS5 tunnels out of the cache so `ssh -D` children
/// don't live forever after the last request.
fn reap_idle_tunnels() {
    let mut cache = tunnel_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, c| c.last_used.elapsed() < TUNNEL_IDLE_TTL && c.tunnel.is_alive());
}

/// Ensure a single daemon-wide background reaper periodically sweeps idle
/// `ssh -D` tunnels even when no new API requests arrive.
fn ensure_tunnel_reaper() {
    static STARTED: OnceLock<()> = OnceLock::new();
    STARTED.get_or_init(|| {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async {
                loop {
                    tokio::time::sleep(TUNNEL_IDLE_TTL / 4).await;
                    reap_idle_tunnels();
                }
            });
        }
    });
}

/// Open (or reuse) a cached SOCKS5 tunnel for `cfg` and return its proxy URL
/// (`socks5h://127.0.0.1:<port>`). Dead/idle entries are evicted on access
/// and periodically by the background reaper.
pub(crate) async fn socks_proxy_url(cfg: &SshTunnelConfig) -> Result<String, String> {
    ensure_tunnel_reaper();
    let key = tunnel_cache_key(cfg);
    // Fast path: a live cached tunnel. Evict idle/dead entries while we hold the
    // lock. The lock is never held across an await.
    {
        let mut cache = tunnel_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.retain(|_, c| c.last_used.elapsed() < TUNNEL_IDLE_TTL && c.tunnel.is_alive());
        if let Some(c) = cache.get_mut(&key) {
            c.last_used = Instant::now();
            return Ok(format!("socks5h://127.0.0.1:{}", c.tunnel.local_port()));
        }
    }
    // Slow path: open a fresh tunnel (blocks until the local SOCKS port is up),
    // then cache it. A concurrent opener racing us just replaces the entry; the
    // loser's tunnel drops (and its `ssh` child is killed) when overwritten.
    let tunnel = SshTunnel::open_socks(cfg)
        .await
        .map_err(|e| format!("SSH tunnel failed: {e}"))?;
    let port = tunnel.local_port();
    let tunnel = Arc::new(tunnel);
    {
        let mut cache = tunnel_cache().lock().unwrap_or_else(|e| e.into_inner());
        cache.insert(
            key,
            CachedTunnel {
                tunnel,
                last_used: Instant::now(),
            },
        );
    }
    Ok(format!("socks5h://127.0.0.1:{port}"))
}

/// Resolve the SOCKS5 proxy URL for a request that opts into an SSH tunnel via
/// `ssh_connection_id`. `Ok(None)` when no tunnel is requested. The referenced
/// connection must be an `ssh`-kind profile visible to `wid` (workspace-scoped
/// or a global profile). Errors are human-readable (surfaced as a 502).
pub async fn resolve_socks_proxy<C: ApiClientCtx>(
    ctx: &C,
    wid: &Id,
    ssh_connection_id: Option<&Id>,
    actor: &Id,
) -> Result<Option<String>, String> {
    let Some(conn_id) = ssh_connection_id else {
        return Ok(None);
    };
    let conn = ctx.tunnel_connection(conn_id, actor).await?;
    if conn.kind != ConnectionKind::Ssh {
        return Err(format!(
            "connection '{}' is not an SSH connection",
            conn.name
        ));
    }
    // Global profiles (workspace_id = None) are visible everywhere; otherwise it
    // must belong to this workspace.
    if let Some(ws) = &conn.workspace_id {
        if ws != wid {
            return Err("SSH tunnel connection belongs to another workspace".into());
        }
    }
    let cfg = ssh_config_from_conn(&conn)?;
    Ok(Some(socks_proxy_url(&cfg).await?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::Environment;

    fn ssh_conn(params: Value) -> Connection {
        Connection {
            id: "c1".into(),
            workspace_id: None,
            name: "bastion".into(),
            kind: ConnectionKind::Ssh,
            params,
            secret_ref: None,
            first_command: None,
            section_id: None,
            environment: Environment::Dev,
            read_only: false,
            pinned: false,
            created_by: "u1".into(),
            created_at: chrono::Utc::now(),
            last_opened_at: None,
        }
    }

    #[test]
    fn ssh_config_preserves_jump_and_distinguishes_cache_key() {
        let with_jump = ssh_conn(serde_json::json!({
            "host": "internal.example",
            "user": "ops",
            "port": 2222,
            "jump": "edge@jump.example:22"
        }));
        let without_jump = ssh_conn(serde_json::json!({
            "host": "internal.example",
            "user": "ops",
            "port": 2222,
            "jump": ""
        }));
        let cfg1 = ssh_config_from_conn(&with_jump).unwrap();
        let cfg2 = ssh_config_from_conn(&without_jump).unwrap();
        assert_eq!(cfg1.proxy_jump.as_deref(), Some("edge@jump.example:22"));
        assert_eq!(cfg2.proxy_jump, None);
        assert_ne!(tunnel_cache_key(&cfg1), tunnel_cache_key(&cfg2));
    }
}
