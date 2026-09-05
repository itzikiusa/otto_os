//! Cluster registry service: CRUD over `k8s_clusters`, kubeconfig context
//! discovery, YAML import, connectivity test and the capability probe.
//!
//! Invariants (contract §0, §1, §3.1):
//! - Otto **never writes** the user's `~/.kube/config`; discovery only reads
//!   `contexts[]` + `clusters[].cluster.server` out of `kubectl config view -o json`
//!   (which itself omits certificate/token data) — no secrets are returned.
//! - Imported YAML lands in `<data_dir>/kube/<id>.yaml` with mode 0600 and is
//!   validated by kubectl BEFORE the row is created; the file is removed when
//!   the row is deleted (`imported` / `eks` sources only).
//! - Every kubectl call for a cluster goes through [`kubectl_for`], which builds
//!   the §4.1 base flags and, for `eks` clusters, injects the linked AWS
//!   account's environment so the kubeconfig's `aws eks get-token` exec plugin
//!   can authenticate.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_core::domain::{Environment, User};
use otto_core::event::Event;
use otto_core::{new_id, Error, Id, Result};
use otto_state::{K8sCluster, K8sClusterPatch, K8sClusterSource, K8sClustersRepo, NewK8sCluster};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;

use crate::cli::{self, Kubectl};
use crate::install::{self, Tool};
use crate::K8sCtx;

/// Max accepted size of a pasted kubeconfig.
const IMPORT_MAX_BYTES: usize = 1024 * 1024;

/// `POST /k8s/clusters` body (contract §3.1 `UpsertK8sClusterReq`).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct UpsertK8sClusterReq {
    pub name: String,
    /// Only `kubeconfig` is accepted here; `imported` comes from `/import`,
    /// `eks` from the AWS module.
    #[serde(default)]
    pub source: Option<String>,
    pub kubeconfig_path: Option<String>,
    pub context_name: String,
    pub default_namespace: Option<String>,
    pub environment: Option<Environment>,
    pub color: Option<String>,
}

/// `PATCH /k8s/clusters/{id}` body — every field optional; `""` clears the
/// nullable strings.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PatchK8sClusterReq {
    pub name: Option<String>,
    pub kubeconfig_path: Option<String>,
    pub context_name: Option<String>,
    pub default_namespace: Option<String>,
    pub environment: Option<Environment>,
    pub color: Option<String>,
}

/// `POST /k8s/clusters/import` body.
#[derive(Debug, Clone, Deserialize)]
pub struct ImportK8sClusterReq {
    pub name: String,
    pub kubeconfig_yaml: String,
    pub context_name: Option<String>,
    pub default_namespace: Option<String>,
    pub environment: Option<Environment>,
    pub color: Option<String>,
}

/// One entry of `GET /k8s/discover`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveredContext {
    pub name: String,
    pub cluster: String,
    pub user: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    pub kubeconfig_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,
    /// `true` for the file's `current-context`.
    pub current: bool,
}

/// `POST /k8s/clusters/{id}/test` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct K8sTestResp {
    pub ok: bool,
    pub latency_ms: u64,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
}

/// `GET /k8s/clusters/{id}/capabilities` response (also the cached JSON).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct K8sCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    pub metrics_server: bool,
    pub argo_rollouts: bool,
    pub argocd: bool,
    pub checked_at: DateTime<Utc>,
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Namespace names are DNS-1123 labels — always lowercase. Users type them
/// (and WebKit auto-capitalizes the first letter), so a stored "Mscasino"
/// makes every namespaced request 403 for a namespace that doesn't exist.
fn clean_ns(s: Option<String>) -> Option<String> {
    clean(s).map(|v| v.to_ascii_lowercase())
}

/// `~` / `~/x` → home-relative absolute path.
pub fn expand_tilde(p: &str) -> PathBuf {
    if let Some(rest) = p.strip_prefix("~/") {
        if let Some(h) = dirs::home_dir() {
            return h.join(rest);
        }
    }
    if p == "~" {
        if let Some(h) = dirs::home_dir() {
            return h;
        }
    }
    PathBuf::from(p)
}

/// Program path for kubectl: located binary, else bare `kubectl` (PATH).
pub fn kubectl_program(data_dir: &Path) -> String {
    install::locate(Tool::Kubectl, data_dir)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| "kubectl".to_string())
}

/// Build the kubectl handle for a cluster (base flags + AWS env for `eks`).
pub async fn kubectl_for<S: K8sCtx>(ctx: &S, cluster: &K8sCluster) -> Result<Kubectl> {
    let env = aws_env_for(ctx, cluster).await?;
    Ok(Kubectl::new(kubectl_program(ctx.data_dir()), cluster, env))
}

/// AWS credential environment for an `eks`-source cluster (contract §1 "Auth
/// injection"): profile accounts ⇒ `AWS_PROFILE`; access-key accounts ⇒ the
/// three `AWS_*` credential vars read from the Keychain (`aws-<id>`). Empty for
/// every other source or when the linked account is gone (FK set NULL).
///
/// TODO(otto-aws): share `otto_aws::accounts::build_env` once the AWS crate
/// exports it — this is a deliberately minimal mirror so the k8s crate has no
/// dependency on otto-aws while both are built in parallel.
pub async fn aws_env_for<S: K8sCtx>(
    ctx: &S,
    cluster: &K8sCluster,
) -> Result<Vec<(String, String)>> {
    if cluster.source != K8sClusterSource::Eks {
        return Ok(vec![]);
    }
    let Some(account_id) = cluster.aws_account_id.as_deref() else {
        return Ok(vec![]);
    };
    let row = sqlx::query(
        "SELECT auth_mode, profile, region, params_json, secret_ref FROM aws_accounts WHERE id = ?",
    )
    .bind(account_id)
    .fetch_optional(&ctx.pool())
    .await
    .map_err(|e| Error::Internal(format!("aws account lookup: {e}")))?;
    let Some(row) = row else {
        return Ok(vec![]);
    };
    let auth_mode: String = row.get("auth_mode");
    let profile: Option<String> = row.get("profile");
    let region: String = row.get("region");
    let params: Value =
        serde_json::from_str(&row.get::<String, _>("params_json")).unwrap_or(Value::Null);
    let secret_ref: Option<String> = row.get("secret_ref");
    let secret = match &secret_ref {
        Some(r) => ctx.secrets().get(r)?,
        None => None,
    };
    Ok(build_aws_env(
        &auth_mode,
        profile.as_deref(),
        &region,
        &params,
        secret.as_deref(),
    ))
}

/// Pure half of [`aws_env_for`] — `params` / `secret_json` are the ACCOUNT's
/// `params_json` and Keychain payload.
pub fn build_aws_env(
    auth_mode: &str,
    profile: Option<&str>,
    region: &str,
    params: &Value,
    secret_json: Option<&str>,
) -> Vec<(String, String)> {
    let mut env = vec![
        ("AWS_REGION".to_string(), region.to_string()),
        ("AWS_DEFAULT_REGION".to_string(), region.to_string()),
        ("AWS_PAGER".to_string(), String::new()),
        ("AWS_CLI_AUTO_PROMPT".to_string(), "off".to_string()),
    ];
    match auth_mode {
        "profile" => {
            if let Some(p) = profile.map(str::trim).filter(|p| !p.is_empty()) {
                env.push(("AWS_PROFILE".into(), p.to_string()));
            }
        }
        _ => {
            let key_id = params
                .get("access_key_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let secret: Value = secret_json
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or(Value::Null);
            if !key_id.is_empty() {
                env.push(("AWS_ACCESS_KEY_ID".into(), key_id.to_string()));
            }
            if let Some(sk) = secret.get("secret_access_key").and_then(Value::as_str) {
                env.push(("AWS_SECRET_ACCESS_KEY".into(), sk.to_string()));
            }
            if let Some(t) = secret
                .get("session_token")
                .and_then(Value::as_str)
                .filter(|t| !t.is_empty())
            {
                env.push(("AWS_SESSION_TOKEN".into(), t.to_string()));
            }
            // A stray AWS_PROFILE in the daemon env must not override the keys.
            env.push(("AWS_PROFILE".into(), String::new()));
        }
    }
    env
}

/// The registry service — thin wrapper over the repo + kubectl.
pub struct Clusters<S: K8sCtx> {
    ctx: S,
    repo: K8sClustersRepo,
}

impl<S: K8sCtx> Clusters<S> {
    pub fn new(ctx: &S) -> Self {
        Self {
            ctx: ctx.clone(),
            repo: K8sClustersRepo::new(ctx.pool()),
        }
    }

    pub fn repo(&self) -> &K8sClustersRepo {
        &self.repo
    }

    fn kube_dir(&self) -> PathBuf {
        self.ctx.data_dir().join("kube")
    }

    fn broadcast(&self, id: &Id, deleted: bool) {
        let _ = self.ctx.events().send(Event::K8sClusterUpdated {
            cluster_id: id.clone(),
            deleted,
        });
    }

    pub async fn list(&self) -> Result<Vec<K8sCluster>> {
        self.repo.list().await
    }

    pub async fn get(&self, id: &Id) -> Result<K8sCluster> {
        self.repo.get(id).await
    }

    /// `POST /k8s/clusters` — register an existing kubeconfig context.
    pub async fn create(&self, user: &User, req: UpsertK8sClusterReq) -> Result<K8sCluster> {
        let name = req.name.trim().to_string();
        if name.is_empty() {
            return Err(Error::Invalid("name is required".into()));
        }
        if let Some(src) = clean(req.source.clone()) {
            if src != "kubeconfig" {
                return Err(Error::Invalid(
                    "source must be 'kubeconfig' here — use /k8s/clusters/import for pasted YAML or the AWS module for EKS".into(),
                ));
            }
        }
        let context_name = req.context_name.trim().to_string();
        if context_name.is_empty() {
            return Err(Error::Invalid("context_name is required".into()));
        }
        let kubeconfig_path = match clean(req.kubeconfig_path) {
            Some(p) => {
                let abs = expand_tilde(&p);
                if !abs.is_file() {
                    return Err(Error::Invalid(format!("kubeconfig file not found: {p}")));
                }
                Some(abs.to_string_lossy().to_string())
            }
            None => None,
        };
        let id = new_id();
        let c = self
            .repo
            .create(NewK8sCluster {
                id,
                name,
                source: K8sClusterSource::Kubeconfig,
                kubeconfig_path,
                context_name,
                default_namespace: clean_ns(req.default_namespace),
                aws_account_id: None,
                environment: req.environment.unwrap_or_default(),
                color: clean(req.color),
                params: json!({}),
                created_by: Some(user.id.clone()),
            })
            .await?;
        self.broadcast(&c.id, false);
        Ok(c)
    }

    /// `POST /k8s/clusters/import` — persist pasted YAML as an Otto-owned
    /// kubeconfig, validated by kubectl before the row exists.
    pub async fn import(&self, user: &User, req: ImportK8sClusterReq) -> Result<K8sCluster> {
        let name = req.name.trim().to_string();
        if name.is_empty() {
            return Err(Error::Invalid("name is required".into()));
        }
        let yaml = req.kubeconfig_yaml.trim();
        if yaml.is_empty() {
            return Err(Error::Invalid("kubeconfig_yaml is required".into()));
        }
        if yaml.len() > IMPORT_MAX_BYTES {
            return Err(Error::PayloadTooLarge(
                "kubeconfig_yaml exceeds 1 MiB".into(),
            ));
        }
        let id = new_id();
        let path = self.write_kubeconfig(&id, yaml)?;
        let path_str = path.to_string_lossy().to_string();

        // Validate with kubectl (also resolves the current-context default).
        let program = kubectl_program(self.ctx.data_dir());
        let view = cli::run(
            &program,
            &[
                "--kubeconfig".into(),
                path_str.clone(),
                "config".into(),
                "view".into(),
                "-o".into(),
                "json".into(),
            ],
            &[],
            Duration::from_secs(15),
            None,
        )
        .await
        .and_then(|o| cli::parse_json(&o.stdout));
        let view = match view {
            Ok(v) => v,
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                return Err(match e {
                    Error::Invalid(m) if m.contains("not installed") => Error::Invalid(m),
                    other => Error::Invalid(format!("kubeconfig did not validate: {other}")),
                });
            }
        };
        let contexts = parse_config_view(&view, &path_str);
        if contexts.is_empty() {
            let _ = std::fs::remove_file(&path);
            return Err(Error::Invalid("kubeconfig has no contexts".into()));
        }
        let context_name = match clean(req.context_name) {
            Some(c) => {
                if !contexts.iter().any(|x| x.name == c) {
                    let _ = std::fs::remove_file(&path);
                    return Err(Error::Invalid(format!(
                        "context '{c}' not found in the pasted kubeconfig (has: {})",
                        contexts
                            .iter()
                            .map(|x| x.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
                c
            }
            None => contexts
                .iter()
                .find(|x| x.current)
                .or(contexts.first())
                .map(|x| x.name.clone())
                .expect("non-empty"),
        };
        let default_namespace = clean_ns(req.default_namespace).or_else(|| {
            contexts
                .iter()
                .find(|x| x.name == context_name)
                .and_then(|x| x.namespace.clone())
        });
        let created = self
            .repo
            .create(NewK8sCluster {
                id: id.clone(),
                name,
                source: K8sClusterSource::Imported,
                kubeconfig_path: Some(path_str),
                context_name,
                default_namespace,
                aws_account_id: None,
                environment: req.environment.unwrap_or_default(),
                color: clean(req.color),
                params: json!({}),
                created_by: Some(user.id.clone()),
            })
            .await;
        match created {
            Ok(c) => {
                self.broadcast(&c.id, false);
                Ok(c)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&path);
                Err(e)
            }
        }
    }

    /// Write `<data_dir>/kube/<id>.yaml` (dir 0700, file 0600).
    fn write_kubeconfig(&self, id: &Id, yaml: &str) -> Result<PathBuf> {
        let dir = self.kube_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Internal(format!("create {}: {e}", dir.display())))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        }
        let path = dir.join(format!("{id}.yaml"));
        write_private(&path, yaml.as_bytes())?;
        Ok(path)
    }

    /// `PATCH /k8s/clusters/{id}`.
    pub async fn update(&self, id: &Id, req: PatchK8sClusterReq) -> Result<K8sCluster> {
        let cur = self.repo.get(id).await?;
        let kubeconfig_path = match req.kubeconfig_path {
            None => None,
            Some(p) => {
                if cur.source.otto_owned_kubeconfig() {
                    return Err(Error::Invalid(
                        "kubeconfig_path of an imported/EKS cluster is managed by Otto".into(),
                    ));
                }
                match clean(Some(p)) {
                    Some(p) => {
                        let abs = expand_tilde(&p);
                        if !abs.is_file() {
                            return Err(Error::Invalid(format!("kubeconfig file not found: {p}")));
                        }
                        Some(Some(abs.to_string_lossy().to_string()))
                    }
                    None => Some(None),
                }
            }
        };
        if let Some(n) = &req.name {
            if n.trim().is_empty() {
                return Err(Error::Invalid("name cannot be empty".into()));
            }
        }
        if let Some(c) = &req.context_name {
            if c.trim().is_empty() {
                return Err(Error::Invalid("context_name cannot be empty".into()));
            }
        }
        let c = self
            .repo
            .update(
                id,
                K8sClusterPatch {
                    name: req.name,
                    kubeconfig_path,
                    context_name: req.context_name.map(|c| c.trim().to_string()),
                    default_namespace: req.default_namespace.map(|d| clean_ns(Some(d))),
                    environment: req.environment,
                    color: req.color.map(|c| clean(Some(c))),
                },
            )
            .await?;
        self.broadcast(id, false);
        Ok(c)
    }

    /// `DELETE /k8s/clusters/{id}` — removes the Otto-owned kubeconfig file for
    /// `imported` / `eks` sources; user files are never touched.
    pub async fn delete(&self, id: &Id) -> Result<()> {
        let c = self.repo.delete(id).await?;
        // Monitoring rows cascade in SQLite; the ClickHouse partitions do not —
        // best-effort purge so a re-imported cluster never inherits old series.
        if let Some(sink) = self.ctx.monitor_sink() {
            if sink.available() {
                for q in crate::monitor::schema::purge_cluster_sql(id.as_str(), None) {
                    if let Err(e) = sink.exec(&q).await {
                        tracing::debug!("k8s monitor purge on delete: {e}");
                    }
                }
            }
        }
        if c.source.otto_owned_kubeconfig() {
            if let Some(p) = c.kubeconfig_path.as_deref() {
                let path = PathBuf::from(p);
                // Belt and braces: only delete inside our own kube dir.
                if path.starts_with(self.kube_dir()) {
                    if let Err(e) = std::fs::remove_file(&path) {
                        if e.kind() != std::io::ErrorKind::NotFound {
                            tracing::warn!("k8s: could not remove {}: {e}", path.display());
                        }
                    }
                }
            }
        }
        self.broadcast(id, true);
        Ok(())
    }

    /// `POST /k8s/clusters/{id}/test` — `kubectl version -o json --request-timeout=8s`.
    pub async fn test(&self, cluster: &K8sCluster) -> Result<K8sTestResp> {
        let k = kubectl_for(&self.ctx, cluster).await?;
        let mut argv = k.argv_stream(["version", "-o", "json", "--request-timeout=8s"]);
        argv.retain(|a| a != "--request-timeout"); // (base_stream has none; explicit for clarity)
        let out = cli::run_raw(&k.program, &argv, &k.env, Duration::from_secs(15), None).await?;
        let parsed = cli::parse_json(&out.stdout).unwrap_or(Value::Null);
        let server_version = parsed
            .pointer("/serverVersion/gitVersion")
            .and_then(Value::as_str)
            .map(str::to_string);
        let ok = out.status == 0 && server_version.is_some();
        let message = if ok {
            format!(
                "connected to {} ({})",
                cluster.context_name,
                server_version.as_deref().unwrap_or("?")
            )
        } else {
            let err = cli::classify_failure(&k.program, &out.stderr);
            match err {
                Error::Invalid(m) if out.stderr.trim().is_empty() => m,
                e => e.to_string(),
            }
        };
        if ok {
            let _ = self.repo.touch(&cluster.id).await;
        }
        Ok(K8sTestResp {
            ok,
            latency_ms: out.duration_ms,
            message,
            server_version,
        })
    }

    /// `GET /k8s/clusters/{id}/capabilities` — cached in `capabilities_json`.
    pub async fn capabilities(
        &self,
        cluster: &K8sCluster,
        refresh: bool,
    ) -> Result<K8sCapabilities> {
        if !refresh {
            if let Some(cached) = cluster
                .capabilities
                .as_ref()
                .and_then(|v| serde_json::from_value::<K8sCapabilities>(v.clone()).ok())
            {
                return Ok(cached);
            }
        }
        let k = kubectl_for(&self.ctx, cluster).await?;
        let caps = probe_capabilities(&k).await?;
        self.repo
            .set_capabilities(
                &cluster.id,
                &serde_json::to_value(&caps).unwrap_or(Value::Null),
            )
            .await?;
        Ok(caps)
    }

    /// Cached capabilities without probing (what list/nodes use to decide
    /// whether to ask metrics-server). Falls back to a probe when never cached.
    pub async fn cached_capabilities(&self, cluster: &K8sCluster) -> K8sCapabilities {
        match self.capabilities(cluster, false).await {
            Ok(c) => c,
            Err(_) => K8sCapabilities {
                server_version: None,
                metrics_server: false,
                argo_rollouts: false,
                argocd: false,
                checked_at: Utc::now(),
            },
        }
    }
}

/// Run the three capability probes concurrently (contract §1).
pub async fn probe_capabilities(k: &Kubectl) -> Result<K8sCapabilities> {
    let (version, metrics, argo) = tokio::join!(
        k.run_timeout(["version", "-o", "json"], Duration::from_secs(15)),
        k.run_timeout(
            ["get", "--raw", "/apis/metrics.k8s.io/v1beta1"],
            Duration::from_secs(15)
        ),
        k.run_timeout(
            ["api-resources", "--api-group=argoproj.io", "-o", "name"],
            Duration::from_secs(20)
        ),
    );
    // If even `version` is a not-installed error, surface it (nothing else can work).
    if let Err(Error::Invalid(m)) = &version {
        if m.contains("not installed") {
            return Err(Error::Invalid(m.clone()));
        }
    }
    let server_version = version.ok().and_then(|o| {
        cli::parse_json(&o.stdout)
            .ok()?
            .pointer("/serverVersion/gitVersion")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let metrics_server = metrics.is_ok();
    let (argo_rollouts, argocd) = argo
        .map(|o| parse_argo_resources(&o.stdout))
        .unwrap_or((false, false));
    Ok(K8sCapabilities {
        server_version,
        metrics_server,
        argo_rollouts,
        argocd,
        checked_at: Utc::now(),
    })
}

/// `(argo_rollouts, argocd)` from `kubectl api-resources --api-group=argoproj.io -o name`.
pub fn parse_argo_resources(stdout: &str) -> (bool, bool) {
    let mut rollouts = false;
    let mut apps = false;
    for line in stdout.lines() {
        let l = line.trim();
        if l == "rollouts.argoproj.io" || l.starts_with("rollouts.") {
            rollouts = true;
        }
        if l == "applications.argoproj.io" || l.starts_with("applications.") {
            apps = true;
        }
    }
    (rollouts, apps)
}

/// Kubeconfig files to scan: `~/.kube/config` + every `$KUBECONFIG` entry
/// (deduplicated, existing files only).
pub fn discovery_files() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    if let Some(h) = dirs::home_dir() {
        files.push(h.join(".kube/config"));
    }
    if let Some(kc) = std::env::var_os("KUBECONFIG") {
        for p in std::env::split_paths(&kc) {
            if !p.as_os_str().is_empty() {
                files.push(expand_tilde(&p.to_string_lossy()));
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    files
        .into_iter()
        .filter(|p| p.is_file())
        .filter(|p| seen.insert(p.clone()))
        .collect()
}

/// `GET /k8s/discover` — contexts across every kubeconfig file.
pub async fn discover(data_dir: &Path) -> Result<Vec<DiscoveredContext>> {
    let program = kubectl_program(data_dir);
    let mut out = Vec::new();
    for file in discovery_files() {
        let path = file.to_string_lossy().to_string();
        let res = cli::run(
            &program,
            &[
                "--kubeconfig".into(),
                path.clone(),
                "config".into(),
                "view".into(),
                "-o".into(),
                "json".into(),
            ],
            &[],
            Duration::from_secs(15),
            None,
        )
        .await;
        match res {
            Ok(o) => {
                if let Ok(v) = cli::parse_json(&o.stdout) {
                    out.extend(parse_config_view(&v, &path));
                }
            }
            Err(Error::Invalid(m)) if m.contains("not installed") => {
                return Err(Error::Invalid(m));
            }
            Err(e) => tracing::warn!("k8s discover: {path}: {e}"),
        }
    }
    Ok(out)
}

/// Pure parser for `kubectl config view -o json`: reads `contexts[]` and joins
/// `clusters[].cluster.server`; certificate / token material is never read.
pub fn parse_config_view(v: &Value, kubeconfig_path: &str) -> Vec<DiscoveredContext> {
    let current = v
        .get("current-context")
        .and_then(Value::as_str)
        .unwrap_or("");
    let servers: std::collections::HashMap<&str, &str> = v
        .get("clusters")
        .and_then(Value::as_array)
        .map(|cs| {
            cs.iter()
                .filter_map(|c| {
                    Some((
                        c.get("name")?.as_str()?,
                        c.pointer("/cluster/server")?.as_str()?,
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    v.get("contexts")
        .and_then(Value::as_array)
        .map(|cs| {
            cs.iter()
                .filter_map(|c| {
                    let name = c.get("name")?.as_str()?.to_string();
                    let cluster = c
                        .pointer("/context/cluster")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    Some(DiscoveredContext {
                        server: servers.get(cluster.as_str()).map(|s| s.to_string()),
                        user: c
                            .pointer("/context/user")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string(),
                        namespace: c
                            .pointer("/context/namespace")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        kubeconfig_path: kubeconfig_path.to_string(),
                        current: name == current,
                        cluster,
                        name,
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Create/overwrite a file readable by the owner only.
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts
        .open(path)
        .map_err(|e| Error::Internal(format!("write {}: {e}", path.display())))?;
    f.write_all(bytes)
        .map_err(|e| Error::Internal(format!("write {}: {e}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_ns_lowercases_and_trims() {
        assert_eq!(clean_ns(Some(" Mscasino ".into())), Some("mscasino".into()));
        assert_eq!(clean_ns(Some("   ".into())), None);
        assert_eq!(clean_ns(None), None);
    }

    #[test]
    fn config_view_parsing_reads_only_contexts_and_servers() {
        let v: Value = serde_json::from_str(include_str!("../testdata/config_view.json")).unwrap();
        let ctxs = parse_config_view(&v, "/Users/me/.kube/config");
        assert_eq!(ctxs.len(), 2);
        assert_eq!(
            ctxs[0],
            DiscoveredContext {
                name: "prod".into(),
                cluster: "prod-eks".into(),
                user: "prod-user".into(),
                namespace: Some("shop".into()),
                kubeconfig_path: "/Users/me/.kube/config".into(),
                server: Some("https://ABC.gr7.eu-west-1.eks.amazonaws.com".into()),
                current: false,
            }
        );
        assert_eq!(ctxs[1].name, "kind-kind");
        assert!(ctxs[1].current);
        assert_eq!(ctxs[1].namespace, None);
        let ser = serde_json::to_string(&ctxs).unwrap();
        assert!(
            !ser.contains("REDACTED") && !ser.contains("DATA+OMITTED") && !ser.contains("exec")
        );
        assert!(parse_config_view(&json!({}), "x").is_empty());
    }

    #[test]
    fn argo_resource_detection() {
        assert_eq!(parse_argo_resources(""), (false, false));
        assert_eq!(
            parse_argo_resources("analysisruns.argoproj.io\nrollouts.argoproj.io\n"),
            (true, false)
        );
        assert_eq!(
            parse_argo_resources("applications.argoproj.io\napplicationsets.argoproj.io\nappprojects.argoproj.io\nrollouts.argoproj.io"),
            (true, true)
        );
    }

    #[test]
    fn aws_env_rules() {
        let env = build_aws_env("profile", Some("dev-sso"), "eu-west-1", &json!({}), None);
        assert!(env.contains(&("AWS_PROFILE".into(), "dev-sso".into())));
        assert!(env.contains(&("AWS_REGION".into(), "eu-west-1".into())));
        assert!(env.contains(&("AWS_PAGER".into(), String::new())));
        assert!(env.contains(&("AWS_CLI_AUTO_PROMPT".into(), "off".into())));
        assert!(!env.iter().any(|(k, _)| k == "AWS_ACCESS_KEY_ID"));

        let env = build_aws_env(
            "access_keys",
            None,
            "us-east-1",
            &json!({"access_key_id": "AKIAEXAMPLE"}),
            Some(r#"{"secret_access_key": "s3cr3t", "session_token": "tok"}"#),
        );
        assert!(env.contains(&("AWS_ACCESS_KEY_ID".into(), "AKIAEXAMPLE".into())));
        assert!(env.contains(&("AWS_SECRET_ACCESS_KEY".into(), "s3cr3t".into())));
        assert!(env.contains(&("AWS_SESSION_TOKEN".into(), "tok".into())));
        assert!(env.contains(&("AWS_PROFILE".into(), String::new())));
    }

    #[test]
    fn tilde_expansion() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_tilde("~/.kube/config"), home.join(".kube/config"));
        assert_eq!(expand_tilde("/abs/x"), PathBuf::from("/abs/x"));
    }

    #[test]
    fn capabilities_roundtrip_through_cache_json() {
        let c = K8sCapabilities {
            server_version: Some("v1.30.2".into()),
            metrics_server: true,
            argo_rollouts: false,
            argocd: true,
            checked_at: Utc::now(),
        };
        let v = serde_json::to_value(&c).unwrap();
        let back: K8sCapabilities = serde_json::from_value(v).unwrap();
        assert!(back.metrics_server && back.argocd && !back.argo_rollouts);
    }
}
