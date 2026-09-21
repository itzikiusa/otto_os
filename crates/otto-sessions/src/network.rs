//! Per-session localhost TCP forwards. Ports are fixed for each PTY lifetime.
use otto_core::access::{AccessMode, ResourceKind, ResourceRef};
use otto_core::domain::{Capability, Connection, ConnectionKind, Feature, Session, User};
use otto_core::network_profiles::{
    NetworkEndpoint, NetworkLocalEndpoint, NetworkProfile, SessionNetworkStatus,
};
use otto_core::{Error, Id, Result};
use otto_ssh::{SshTunnel, SshTunnelConfig};
use otto_state::{
    ConnectionsRepo, GrantsRepo, ResourceAccessRepo, SqlitePool, UsersRepo, WorkspacesRepo,
};
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

trait Tunnel: Send + Sync {
    fn port(&self) -> u16;
    fn alive(&self) -> bool;
    fn error(&self) -> Option<String>;
}
impl Tunnel for SshTunnel {
    fn port(&self) -> u16 {
        self.local_port()
    }
    fn alive(&self) -> bool {
        self.is_alive()
    }
    fn error(&self) -> Option<String> {
        self.last_stderr()
    }
}
type OpenFuture<'a> = Pin<Box<dyn Future<Output = Result<Box<dyn Tunnel>>> + Send + 'a>>;
trait Opener: Send + Sync {
    fn open<'a>(
        &'a self,
        config: &'a SshTunnelConfig,
        endpoint: &'a NetworkEndpoint,
    ) -> OpenFuture<'a>;
}
struct SshOpener;
impl Opener for SshOpener {
    fn open<'a>(
        &'a self,
        config: &'a SshTunnelConfig,
        endpoint: &'a NetworkEndpoint,
    ) -> OpenFuture<'a> {
        Box::pin(async move {
            Ok(
                Box::new(
                    SshTunnel::open(config, &endpoint.remote_host, endpoint.remote_port).await?,
                ) as Box<dyn Tunnel>,
            )
        })
    }
}

pub struct PreparedNetwork {
    pub env: Vec<(String, String)>,
    status: SessionNetworkStatus,
    tunnels: Vec<Box<dyn Tunnel>>,
}
struct ActiveNetwork {
    status: Mutex<SessionNetworkStatus>,
    tunnels: Mutex<Vec<Box<dyn Tunnel>>>,
}
impl ActiveNetwork {
    fn stop(&self) {
        self.tunnels.lock().unwrap().clear();
        let mut status = self.status.lock().unwrap();
        if status.status != "error" {
            status.status = "stopped".into();
        }
    }
    fn inspect(&self) -> bool {
        let mut tunnels = self.tunnels.lock().unwrap();
        if tunnels.is_empty() {
            return false;
        }
        if let Some(dead) = tunnels.iter().find(|tunnel| !tunnel.alive()) {
            let error = dead
                .error()
                .unwrap_or_else(|| "SSH tunnel exited; restart the session to reconnect".into());
            tunnels.clear();
            let mut status = self.status.lock().unwrap();
            status.status = "error".into();
            status.error = Some(error);
            return false;
        }
        true
    }
}

pub struct SessionNetworks {
    pool: SqlitePool,
    live: Mutex<HashMap<Id, Arc<ActiveNetwork>>>,
}
impl SessionNetworks {
    pub fn new(pool: SqlitePool) -> Arc<Self> {
        Arc::new(Self {
            pool,
            live: Mutex::new(HashMap::new()),
        })
    }
    pub async fn prepare(&self, session: &Session) -> Result<Option<PreparedNetwork>> {
        let profile = otto_state::network_profiles::NetworkProfilesRepo::new(self.pool.clone())
            .selected(&session.workspace_id, &session.meta)
            .await?;
        let Some(profile) = profile else {
            return Ok(None);
        };
        let user = UsersRepo::new(self.pool.clone())
            .get(&session.created_by)
            .await?;
        let connection = authorize(&self.pool, &user, &profile).await?;
        let config = ssh_config(&connection)?;
        Ok(Some(prepare_profile(&profile, &config, &SshOpener).await?))
    }
    pub fn status(&self, id: &str) -> SessionNetworkStatus {
        self.live
            .lock()
            .unwrap()
            .get(id)
            .map(|live| live.status.lock().unwrap().clone())
            .unwrap_or_default()
    }
    pub fn clear(&self, id: &str) {
        if let Some(old) = self.live.lock().unwrap().remove(id) {
            old.stop();
        }
    }
    pub fn activate(
        self: &Arc<Self>,
        id: Id,
        prepared: Option<PreparedNetwork>,
        mut exited: tokio::sync::watch::Receiver<Option<i32>>,
    ) {
        self.clear(&id);
        let Some(prepared) = prepared else { return };
        let entry = Arc::new(ActiveNetwork {
            status: Mutex::new(prepared.status),
            tunnels: Mutex::new(prepared.tunnels),
        });
        self.live.lock().unwrap().insert(id.clone(), entry.clone());
        let this = Arc::downgrade(self);
        tokio::spawn(async move {
            loop {
                if exited.borrow().is_some() {
                    entry.stop();
                    break;
                }
                let Some(owner) = this.upgrade() else {
                    entry.stop();
                    break;
                };
                let current = owner
                    .live
                    .lock()
                    .unwrap()
                    .get(&id)
                    .is_some_and(|value| Arc::ptr_eq(value, &entry));
                if !current {
                    entry.stop();
                    break;
                }
                if !entry.inspect() {
                    break;
                }
                drop(owner);
                tokio::select! {
                    _=tokio::time::sleep(Duration::from_secs(1))=>{},
                    result=exited.changed()=>if result.is_err() { entry.stop(); break; },
                }
            }
        });
    }
}

/// Execution authorization is rechecked at every launch, independent of who
/// created the profile. The connection's shell permission covers SSH forwards.
pub async fn authorize(
    pool: &SqlitePool,
    user: &User,
    profile: &NetworkProfile,
) -> Result<Connection> {
    if user.disabled {
        return Err(Error::Forbidden("account disabled".into()));
    }
    if WorkspacesRepo::new(pool.clone())
        .role_of(user, &profile.workspace_id)
        .await?
        .is_none()
    {
        return Err(Error::NotFound("network profile".into()));
    }
    let conn = ConnectionsRepo::new(pool.clone())
        .get(&profile.input.ssh_connection_id)
        .await?;
    if conn.kind != ConnectionKind::Ssh
        || conn
            .workspace_id
            .as_ref()
            .is_some_and(|id| id != &profile.workspace_id)
    {
        return Err(Error::Invalid(
            "network profile requires an SSH connection visible to its workspace".into(),
        ));
    }
    let policy = ResourceAccessRepo::new(pool.clone())
        .get_live_policy(ResourceKind::Connection, &conn.id)
        .await?;
    let capability = if policy.mode == AccessMode::Legacy {
        Capability::Edit
    } else {
        Capability::View
    };
    GrantsRepo::new(pool.clone())
        .check_global(
            user,
            Feature::Connections,
            capability,
            "Connections execution access is required for session network profiles",
        )
        .await?;
    otto_rbac::ResourceAccess::new(pool.clone())
        .check(
            user,
            &ResourceRef {
                kind: ResourceKind::Connection,
                id: conn.id.clone(),
                child: None,
            },
            "shell",
        )
        .await?;
    Ok(conn)
}
fn ssh_config(conn: &Connection) -> Result<SshTunnelConfig> {
    let string = |key| {
        conn.params
            .get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
    };
    let host = string("host");
    let user = string("user");
    if !otto_core::network_profiles::valid_host(host)
        || user.starts_with('-')
        || !user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    {
        return Err(Error::Invalid(
            "SSH connection needs a valid host and username".into(),
        ));
    }
    let port = match conn.params.get("port") {
        None | Some(serde_json::Value::Null) => 22,
        Some(serde_json::Value::String(value)) if value.is_empty() => 22,
        Some(serde_json::Value::String(value)) => value
            .parse()
            .map_err(|_| Error::Invalid("invalid SSH port".into()))?,
        Some(value) => value
            .as_u64()
            .and_then(|value| u16::try_from(value).ok())
            .ok_or_else(|| Error::Invalid("invalid SSH port".into()))?,
    };
    if port == 0 {
        return Err(Error::Invalid("SSH port must be nonzero".into()));
    }
    if !string("jump").is_empty() {
        return Err(Error::Invalid("network forwarding uses ProxyJump from ~/.ssh/config; move this connection's jump setting there".into()));
    }
    Ok(SshTunnelConfig {
        host: host.into(),
        user: user.into(),
        port,
        identity_file: (!string("identity_file").is_empty())
            .then(|| string("identity_file").into()),
    })
}
async fn prepare_profile(
    profile: &NetworkProfile,
    config: &SshTunnelConfig,
    opener: &dyn Opener,
) -> Result<PreparedNetwork> {
    profile.input.validate()?;
    let mut tunnels = Vec::new();
    let mut endpoints = Vec::new();
    let mut env = Vec::new();
    for endpoint in &profile.input.endpoints {
        let tunnel = opener.open(config, endpoint).await?;
        let port = tunnel.port();
        let name = endpoint.name.to_ascii_uppercase();
        env.push((format!("OTTO_TUNNEL_{name}_HOST"), "127.0.0.1".into()));
        env.push((format!("OTTO_TUNNEL_{name}_PORT"), port.to_string()));
        if let Some(key) = &endpoint.host_env {
            env.push((key.clone(), "127.0.0.1".into()));
        }
        if let Some(key) = &endpoint.port_env {
            env.push((key.clone(), port.to_string()));
        }
        endpoints.push(NetworkLocalEndpoint {
            name: endpoint.name.clone(),
            host: "127.0.0.1".into(),
            port,
            remote_host: endpoint.remote_host.clone(),
            remote_port: endpoint.remote_port,
            host_env: endpoint.host_env.clone(),
            port_env: endpoint.port_env.clone(),
        });
        tunnels.push(tunnel);
    }
    env.push((
        "OTTO_NETWORK_ENDPOINTS".into(),
        serde_json::to_string(&endpoints).map_err(|e| Error::Internal(e.to_string()))?,
    ));
    Ok(PreparedNetwork {
        env,
        tunnels,
        status: SessionNetworkStatus {
            profile_id: Some(profile.id.clone()),
            profile_name: Some(profile.input.name.clone()),
            profile_version: Some(profile.version),
            status: "connected".into(),
            error: None,
            endpoints,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    struct FakeTunnel {
        dropped: Arc<AtomicUsize>,
        alive: Arc<AtomicBool>,
    }
    impl Drop for FakeTunnel {
        fn drop(&mut self) {
            self.dropped.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Tunnel for FakeTunnel {
        fn port(&self) -> u16 {
            15432
        }
        fn alive(&self) -> bool {
            self.alive.load(Ordering::SeqCst)
        }
        fn error(&self) -> Option<String> {
            Some("bastion disconnected".into())
        }
    }
    struct FakeOpener {
        dropped: Arc<AtomicUsize>,
        alive: Arc<AtomicBool>,
        fail_name: String,
    }
    impl Opener for FakeOpener {
        fn open<'a>(
            &'a self,
            _: &'a SshTunnelConfig,
            endpoint: &'a NetworkEndpoint,
        ) -> OpenFuture<'a> {
            Box::pin(async move {
                if endpoint.name == self.fail_name {
                    return Err(Error::Upstream("refused".into()));
                }
                Ok(Box::new(FakeTunnel {
                    dropped: self.dropped.clone(),
                    alive: self.alive.clone(),
                }) as Box<dyn Tunnel>)
            })
        }
    }
    fn fixture() -> (NetworkProfile, SshTunnelConfig, FakeOpener) {
        let profile=serde_json::from_value(serde_json::json!({"id":"p","workspace_id":"w","name":"Office","ssh_connection_id":"ssh","version":1,"created_by":"u","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z", "endpoints":[{"name":"db","remote_host":"db.internal","remote_port":5432,"host_env":"PGHOST","port_env":"PGPORT"}]})).unwrap();
        let cfg = SshTunnelConfig {
            host: "office".into(),
            port: 22,
            user: "user".into(),
            identity_file: None,
        };
        let opener = FakeOpener {
            dropped: Arc::new(AtomicUsize::new(0)),
            alive: Arc::new(AtomicBool::new(true)),
            fail_name: "fail".into(),
        };
        (profile, cfg, opener)
    }
    #[tokio::test]
    async fn network_authorization_rechecks_membership_feature_and_shell_permission() {
        use otto_core::access::{AccessActor, AccessMode, AccessRule, RuleEffect, SubjectKind};
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        let user = UsersRepo::new(pool.clone())
            .create("network-user", "hash", "Network User", false)
            .await
            .unwrap();
        let ws = WorkspacesRepo::new(pool.clone())
            .create("Workspace", "/tmp", &user.id)
            .await
            .unwrap();
        sqlx::query("INSERT INTO connections(id,workspace_id,name,kind,params_json,created_by,created_at) VALUES('ssh',?,'Office','ssh','{}',?,'2026-01-01T00:00:00Z')").bind(&ws.id).bind(&user.id).execute(&pool).await.unwrap();
        let (mut profile, _, _) = fixture();
        profile.workspace_id = ws.id.clone();
        assert!(authorize(&pool, &user, &profile).await.is_err());
        sqlx::query("INSERT INTO user_feature_grants(user_id,feature,capability) VALUES(?,'connections','edit')").bind(&user.id).execute(&pool).await.unwrap();
        let policies = ResourceAccessRepo::new(pool.clone());
        let mut policy = policies
            .get_policy(ResourceKind::Connection, &"ssh".into())
            .await
            .unwrap();
        policy.mode = AccessMode::Enforced;
        let actor = AccessActor {
            real_user_id: user.id.clone(),
            effective_user_id: None,
        };
        policy = policies
            .put_policy(&policy, policy.revision, &actor)
            .await
            .unwrap();
        assert!(authorize(&pool, &user, &profile).await.is_err());
        policy.rules.push(AccessRule {
            id: otto_core::new_id(),
            subject_kind: SubjectKind::User,
            subject_id: user.id.clone(),
            effect: RuleEffect::Allow,
            operations: vec!["discover".into(), "shell".into()],
            children: None,
            grantable_operations: vec![],
            credential_connection_id: None,
        });
        policies
            .put_policy(&policy, policy.revision, &actor)
            .await
            .unwrap();
        assert!(authorize(&pool, &user, &profile).await.is_ok());
        sqlx::query("DELETE FROM workspace_members WHERE workspace_id=? AND user_id=?")
            .bind(&ws.id)
            .bind(&user.id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(authorize(&pool, &user, &profile).await.is_err());
    }

    #[tokio::test]
    async fn network_exact_env_mapping_and_partial_failure_cleanup() {
        let (mut profile, cfg, opener) = fixture();
        let prepared = prepare_profile(&profile, &cfg, &opener).await.unwrap();
        assert!(prepared.env.contains(&("PGPORT".into(), "15432".into())));
        assert!(prepared
            .env
            .contains(&("OTTO_TUNNEL_DB_HOST".into(), "127.0.0.1".into())));
        assert!(!prepared.env.iter().any(|(key, _)| key.contains("PROXY")));
        drop(prepared);
        assert_eq!(opener.dropped.load(Ordering::SeqCst), 1);
        profile.input.endpoints.push(NetworkEndpoint {
            name: "fail".into(),
            remote_host: "redis.internal".into(),
            remote_port: 6379,
            host_env: None,
            port_env: None,
        });
        assert!(prepare_profile(&profile, &cfg, &opener).await.is_err());
        assert_eq!(opener.dropped.load(Ordering::SeqCst), 2);
    }
    #[tokio::test]
    async fn network_pty_exit_releases_forwards_and_marks_stopped() {
        let (profile, cfg, opener) = fixture();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let runtime = SessionNetworks::new(pool);
        let (exit, rx) = tokio::sync::watch::channel(None);
        runtime.activate(
            "s".into(),
            Some(prepare_profile(&profile, &cfg, &opener).await.unwrap()),
            rx,
        );
        exit.send(Some(0)).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(runtime.status("s").status, "stopped");
        assert_eq!(opener.dropped.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn network_old_monitor_cannot_clear_replacement_and_death_keeps_error() {
        let (profile, cfg, opener) = fixture();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let runtime = SessionNetworks::new(pool);
        let (old_tx, old_rx) = tokio::sync::watch::channel(None);
        runtime.activate(
            "s".into(),
            Some(prepare_profile(&profile, &cfg, &opener).await.unwrap()),
            old_rx,
        );
        let (_new_tx, new_rx) = tokio::sync::watch::channel(None);
        runtime.activate(
            "s".into(),
            Some(prepare_profile(&profile, &cfg, &opener).await.unwrap()),
            new_rx,
        );
        old_tx.send(Some(0)).unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(runtime.status("s").status, "connected");
        assert_eq!(opener.dropped.load(Ordering::SeqCst), 1);
        opener.alive.store(false, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(1100)).await;
        assert_eq!(runtime.status("s").status, "error");
        assert!(runtime
            .status("s")
            .error
            .unwrap()
            .contains("bastion disconnected"));
        assert_eq!(opener.dropped.load(Ordering::SeqCst), 2);
        runtime.clear("s");
        assert_eq!(runtime.status("s").status, "disabled");
    }
}
