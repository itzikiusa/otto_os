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
    Ok(SshTunnelConfig {
        host: host.to_string(),
        port,
        user: user.to_string(),
        identity_file,
    })
}

/// Open (or reuse) a cached SOCKS5 tunnel for `cfg` and return its proxy URL
/// (`socks5h://127.0.0.1:<port>`). Dead/idle entries are evicted on access.
pub(crate) async fn socks_proxy_url(cfg: &SshTunnelConfig) -> Result<String, String> {
    let key = format!(
        "{}|{}|{}|{}",
        cfg.host,
        cfg.port,
        cfg.user,
        cfg.identity_file.as_deref().unwrap_or("")
    );
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
