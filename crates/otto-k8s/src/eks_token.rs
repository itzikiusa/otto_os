//! EKS bearer-token cache (sweep-c SC-11).
//!
//! An `eks` cluster's kubeconfig authenticates through an exec plugin
//! (`aws eks get-token …`). kubectl does not keep exec credentials across
//! processes, so EVERY kubectl call — the 10 s list refresh, metrics, each
//! drawer open, the monitor — started a Python `aws` process (~0.3 s CPU).
//!
//! Here the plugin is run ONCE per cluster; its token is written into an
//! Otto-owned, `0600` copy of the minified kubeconfig (the user entry's `exec`
//! replaced by `token`) and short-lived kubectl calls use that file until
//! `expirationTimestamp - REFRESH_MARGIN`. Nothing changes for other sources,
//! for non-exec users, or when minting fails (callers fall back to the
//! original kubeconfig and its exec plugin).
//!
//! Credential handling: the token is never logged or put in argv (`--token`
//! would show in `ps`); the file lives under `<data_dir>/kube/tokens` (0700),
//! is replaced atomically, and is removed by [`forget`] when the cluster is
//! deleted. k9s PTY sessions keep the exec plugin (they outlive a token).
//!
//! Scope (perf r3-08-15): not only `eks` rows — any cluster whose kubeconfig
//! context authenticates through an exec plugin (`aws eks get-token` in a
//! user's `~/.kube/config`, `gke-gcloud-auth-plugin`, `kubelogin` …) paid a
//! plugin process per kubectl call. Every source now gets the overlay. The
//! user's kubeconfig is only READ (`config view`); the fingerprint carries the
//! kubeconfig files' mtimes, so an edit re-derives the overlay. A context
//! without an exec plugin (or one whose credential isn't a bearer token) is
//! remembered as such — no `config view` per call — until its kubeconfig
//! changes; other failures back off for [`RETRY_AFTER`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, SystemTime};

use otto_state::{K8sCluster, K8sClusterSource};
use serde_json::{json, Value};

use crate::cli;

/// Re-mint this long before the token's stated expiry.
const REFRESH_MARGIN: Duration = Duration::from_secs(60);
/// Used when the plugin states no expiry (EKS tokens live 15 min).
const DEFAULT_LIFETIME: Duration = Duration::from_secs(10 * 60);
const MINT_TIMEOUT: Duration = Duration::from_secs(30);
/// A mint that failed for a transient reason is retried after this long
/// (kubectl runs the plugin itself meanwhile).
const RETRY_AFTER: Duration = Duration::from_secs(120);
/// "Not applicable" (no exec plugin, cluster-info plugin, non-token credential)
/// holds until the kubeconfig changes; this only bounds a missed mtime change.
const NOT_APPLICABLE_FOR: Duration = Duration::from_secs(6 * 3600);

#[derive(Clone)]
struct Entry {
    /// `None` = negative entry: don't mint (or `config view`) until
    /// `fresh_until` or a fingerprint change.
    path: Option<PathBuf>,
    fresh_until: SystemTime,
    /// The cluster fields the overlay was derived from; an edit invalidates it.
    fingerprint: String,
}

/// Why minting did not produce an overlay.
enum MintError {
    /// Permanent for this kubeconfig (no exec plugin, …): remember it.
    NotApplicable(String),
    /// Anything else: retry after [`RETRY_AFTER`].
    Failed(String),
}

impl std::fmt::Display for MintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MintError::NotApplicable(w) | MintError::Failed(w) => f.write_str(w),
        }
    }
}

impl From<String> for MintError {
    fn from(s: String) -> Self {
        MintError::Failed(s)
    }
}

static CACHE: LazyLock<Mutex<HashMap<String, Entry>>> = LazyLock::new(Default::default);
/// Per-cluster single-flight for minting (concurrent first calls share one).
static MINT: LazyLock<Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>> =
    LazyLock::new(Default::default);

fn fingerprint(c: &K8sCluster) -> String {
    let mut fp = format!(
        "{}\u{1}{}\u{1}{}",
        c.kubeconfig_path.as_deref().unwrap_or(""),
        c.context_name,
        c.aws_account_id.as_deref().unwrap_or("")
    );
    for p in kubeconfig_files(c) {
        let m = std::fs::metadata(&p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        fp.push_str(&format!("\u{2}{m}"));
    }
    fp
}

/// The kubeconfig files kubectl reads for `c`: its own path, else
/// `$KUBECONFIG` (colon list), else `~/.kube/config`.
fn kubeconfig_files(c: &K8sCluster) -> Vec<PathBuf> {
    if let Some(p) = c
        .kubeconfig_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        return vec![crate::clusters::expand_tilde(p)];
    }
    match std::env::var("KUBECONFIG")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        Some(v) => std::env::split_paths(&v).collect(),
        None => dirs::home_dir()
            .map(|h| vec![h.join(".kube").join("config")])
            .unwrap_or_default(),
    }
}

/// The fresh cache entry for `cluster` (positive or negative), if any.
fn fresh_entry(cluster: &K8sCluster) -> Option<Entry> {
    let fp = fingerprint(cluster);
    let map = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let e = map.get(cluster.id.as_str())?;
    (e.fingerprint == fp
        && SystemTime::now() < e.fresh_until
        && e.path.as_ref().is_none_or(|p| p.is_file()))
    .then(|| e.clone())
}

/// The token dir: `<data_dir>/kube/tokens`.
pub fn token_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("kube").join("tokens")
}

fn overlay_path(data_dir: &Path, cluster_id: &str) -> PathBuf {
    // Ids are daemon-generated; keep the file name to a safe alphabet anyway.
    let safe: String = cluster_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    token_dir(data_dir).join(format!("eks-{safe}.json"))
}

/// A still-fresh cached overlay for `cluster` (no process, no Keychain read).
pub fn cached(cluster: &K8sCluster) -> Option<PathBuf> {
    fresh_entry(cluster)?.path
}

/// The token overlay for a cluster whose context authenticates with an exec
/// plugin: cached, or minted now by running the plugin with `env` (for `eks`
/// rows the linked AWS account's credentials; empty otherwise). `None` ⇒ use
/// the original kubeconfig (no exec plugin, or minting failed recently).
pub async fn overlay_for(
    program: &str,
    cluster: &K8sCluster,
    env: &[(String, String)],
    data_dir: &Path,
) -> Option<PathBuf> {
    if let Some(e) = fresh_entry(cluster) {
        return e.path;
    }
    let gate = MINT
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .entry(cluster.id.as_str().to_string())
        .or_default()
        .clone();
    let _g = gate.lock().await;
    if let Some(e) = fresh_entry(cluster) {
        return e.path;
    }
    let entry = match mint(program, cluster, env, data_dir).await {
        Ok(entry) => entry,
        Err(why) => {
            // `why` never carries the token (see `mint`).
            let hold = match &why {
                MintError::NotApplicable(_) => NOT_APPLICABLE_FOR,
                MintError::Failed(_) => RETRY_AFTER,
            };
            // An eks row always has a plugin; for the rest "no exec plugin" is
            // the common, expected case — not worth a debug line per cluster.
            if cluster.source == K8sClusterSource::Eks || matches!(why, MintError::Failed(_)) {
                tracing::debug!(cluster = %cluster.id, "exec token cache skipped: {why}");
            }
            Entry {
                path: None,
                fresh_until: SystemTime::now() + hold,
                fingerprint: fingerprint(cluster),
            }
        }
    };
    let path = entry.path.clone();
    CACHE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .insert(cluster.id.as_str().to_string(), entry);
    path
}

/// Drop the cache entry and delete the overlay file (cluster deleted).
pub fn forget(cluster_id: &str, data_dir: &Path) {
    CACHE
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(cluster_id);
    MINT.lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(cluster_id);
    let path = overlay_path(data_dir, cluster_id);
    if let Err(e) = std::fs::remove_file(&path) {
        if e.kind() != std::io::ErrorKind::NotFound {
            tracing::warn!("k8s: could not remove eks token file: {e}");
        }
    }
}

async fn mint(
    program: &str,
    cluster: &K8sCluster,
    env: &[(String, String)],
    data_dir: &Path,
) -> Result<Entry, MintError> {
    // Fingerprint BEFORE reading the kubeconfig: an edit racing the mint then
    // invalidates the entry instead of being masked by it.
    let fp = fingerprint(cluster);
    // 1. The cluster's minified, flattened kubeconfig (inline CA data; the
    //    exec plugin is NOT run by `config view`).
    let mut args: Vec<String> = Vec::new();
    if let Some(p) = cluster
        .kubeconfig_path
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        args.push("--kubeconfig".into());
        args.push(p.to_string());
    }
    args.extend(
        [
            "config",
            "view",
            "--minify",
            "--flatten",
            "--raw",
            "-o",
            "json",
            "--context",
        ]
        .into_iter()
        .map(String::from),
    );
    args.push(cluster.context_name.clone());
    let out = cli::run(program, &args, env, MINT_TIMEOUT, None)
        .await
        .map_err(|e| format!("config view: {e}"))?;
    let mut cfg: Value =
        serde_json::from_str(out.stdout.trim()).map_err(|_| "config view: not JSON".to_string())?;

    // 2. The context's user must authenticate through an exec plugin.
    let exec = cfg
        .pointer("/users/0/user/exec")
        .cloned()
        .ok_or_else(|| MintError::NotApplicable("user has no exec plugin".into()))?;
    if exec.get("provideClusterInfo").and_then(Value::as_bool) == Some(true) {
        return Err(MintError::NotApplicable(
            "exec plugin wants cluster info".into(),
        ));
    }
    let (plugin, plugin_args, mut plugin_env) = exec_invocation(&exec)?;
    plugin_env.splice(0..0, env.iter().cloned());
    let api_version = exec
        .get("apiVersion")
        .and_then(Value::as_str)
        .unwrap_or("client.authentication.k8s.io/v1beta1");
    plugin_env.push((
        "KUBERNETES_EXEC_INFO".into(),
        json!({"apiVersion": api_version, "kind": "ExecCredential", "spec": {"interactive": false}})
            .to_string(),
    ));

    // 3. Run it once. Errors are reduced to a generic reason — plugin output
    //    may echo credentials.
    let cred = cli::run_raw(&plugin, &plugin_args, &plugin_env, MINT_TIMEOUT, None)
        .await
        .map_err(|_| "exec plugin could not run".to_string())?;
    if cred.status != 0 {
        return Err(format!("exec plugin exited {}", cred.status).into());
    }
    let (token, expires) = parse_exec_credential(&cred.stdout).map_err(|why| {
        if why.contains("has no token") {
            // A client-certificate credential: kubectl keeps running the plugin.
            MintError::NotApplicable(why)
        } else {
            MintError::Failed(why)
        }
    })?;

    // 4. Swap every exec user for the static token and persist 0600.
    if let Some(users) = cfg.get_mut("users").and_then(Value::as_array_mut) {
        for u in users {
            if let Some(obj) = u.get_mut("user").and_then(Value::as_object_mut) {
                obj.clear();
                obj.insert("token".into(), Value::String(token.clone()));
            }
        }
    }
    let path = overlay_path(data_dir, cluster.id.as_str());
    let body = serde_json::to_vec(&cfg).map_err(|_| "serialize".to_string())?;
    let p2 = path.clone();
    tokio::task::spawn_blocking(move || write_private(&p2, &body))
        .await
        .map_err(|_| "write task".to_string())?
        .map_err(|e| format!("write token file: {e}"))?;

    let now = SystemTime::now();
    let expires = expires.unwrap_or(now + DEFAULT_LIFETIME);
    let fresh_until = expires.checked_sub(REFRESH_MARGIN).unwrap_or(now);
    if fresh_until <= now {
        return Err("token already expiring".to_string().into());
    }
    Ok(Entry {
        path: Some(path),
        fresh_until,
        fingerprint: fp,
    })
}

/// `(command, args, env)` of an exec plugin.
type ExecInvocation = (String, Vec<String>, Vec<(String, String)>);

/// `(command, args, env)` from a kubeconfig `user.exec` block.
fn exec_invocation(exec: &Value) -> Result<ExecInvocation, String> {
    let command = exec
        .get("command")
        .and_then(Value::as_str)
        .filter(|c| !c.trim().is_empty())
        .ok_or_else(|| "exec plugin has no command".to_string())?
        .to_string();
    let args = exec
        .get("args")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let env = exec
        .get("env")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some((
                        e.get("name")?.as_str()?.to_string(),
                        e.get("value")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok((command, args, env))
}

/// `status.token` + `status.expirationTimestamp` (RFC 3339) of an
/// `ExecCredential`.
fn parse_exec_credential(stdout: &str) -> Result<(String, Option<SystemTime>), String> {
    let v: Value = serde_json::from_str(stdout.trim())
        .map_err(|_| "exec plugin output is not JSON".to_string())?;
    let token = v
        .pointer("/status/token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "exec credential has no token".to_string())?
        .to_string();
    let expires = v
        .pointer("/status/expirationTimestamp")
        .and_then(Value::as_str)
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| SystemTime::from(dt.with_timezone(&chrono::Utc)));
    Ok((token, expires))
}

/// Write `body` to `path` as 0600 (dir 0700), atomically via a temp file.
fn write_private(path: &Path, body: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let tmp = path.with_extension("json.tmp");
    let _ = std::fs::remove_file(&tmp);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(&tmp)?;
    f.write_all(body)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_exec_credential() {
        let (t, exp) = parse_exec_credential(
            r#"{"kind":"ExecCredential","apiVersion":"client.authentication.k8s.io/v1beta1",
                "spec":{},"status":{"expirationTimestamp":"2030-01-01T00:14:00Z","token":"k8s-aws-v1.abc"}}"#,
        )
        .unwrap();
        assert_eq!(t, "k8s-aws-v1.abc");
        assert!(exp.unwrap() > SystemTime::now());
        assert!(parse_exec_credential(r#"{"status":{}}"#).is_err());
        assert!(parse_exec_credential("not json").is_err());
    }

    #[test]
    fn exec_invocation_reads_command_args_env() {
        let (cmd, args, env) = exec_invocation(&json!({
            "command": "aws",
            "args": ["--region", "eu-west-1", "eks", "get-token", "--cluster-name", "c1"],
            "env": [{"name": "AWS_PROFILE", "value": "dev"}],
        }))
        .unwrap();
        assert_eq!(cmd, "aws");
        assert_eq!(args[3], "get-token");
        assert_eq!(env, vec![("AWS_PROFILE".to_string(), "dev".to_string())]);
        assert!(exec_invocation(&json!({"args": []})).is_err());
    }

    fn kubeconfig_cluster(id: &str, path: &Path) -> K8sCluster {
        K8sCluster {
            id: id.into(),
            name: "c".into(),
            source: K8sClusterSource::Kubeconfig,
            kubeconfig_path: Some(path.to_string_lossy().into_owned()),
            context_name: "ctx".into(),
            default_namespace: None,
            aws_account_id: None,
            environment: otto_core::domain::Environment::Dev,
            color: None,
            known_namespaces: Vec::new(),
            params: json!({}),
            capabilities: None,
            created_by: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_used_at: None,
        }
    }

    /// A non-exec kubeconfig context is remembered as such (no `config view`
    /// per kubectl call) until the kubeconfig file changes.
    #[tokio::test]
    async fn negative_entry_holds_until_the_kubeconfig_changes() {
        let dir = tempfile::tempdir().unwrap();
        let kc = dir.path().join("config");
        std::fs::write(&kc, "apiVersion: v1\nkind: Config\n").unwrap();
        let c = kubeconfig_cluster("neg-1", &kc);
        CACHE.lock().unwrap().insert(
            "neg-1".into(),
            Entry {
                path: None,
                fresh_until: SystemTime::now() + NOT_APPLICABLE_FOR,
                fingerprint: fingerprint(&c),
            },
        );
        assert!(cached(&c).is_none());
        // Fresh negative ⇒ `overlay_for` returns without running anything
        // (the program would fail to spawn if it were run).
        assert!(overlay_for("/nonexistent/kubectl", &c, &[], dir.path())
            .await
            .is_none());
        assert!(fresh_entry(&c).is_some(), "still remembered");
        // Editing the kubeconfig invalidates the entry.
        std::fs::File::options()
            .write(true)
            .open(&kc)
            .unwrap()
            .set_modified(SystemTime::now() + Duration::from_secs(10))
            .unwrap();
        assert!(fresh_entry(&c).is_none());
        forget("neg-1", dir.path());
    }

    #[cfg(unix)]
    #[test]
    fn token_file_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = overlay_path(dir.path(), "c/1");
        assert!(p
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("eks-c_1"));
        write_private(&p, b"{}").unwrap();
        write_private(&p, b"{\"a\":1}").unwrap(); // replace in place
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let dmode = std::fs::metadata(p.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(dmode, 0o700);
        forget("c/1", dir.path());
        assert!(!p.exists());
    }
}
