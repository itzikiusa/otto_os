//! Project Environments, Cached User Login+Interactive Shell Environment,
//! Keychain-backed Secret Env Vars, Multi-Folder Project Roots, `.env` &
//! Python Virtualenv Manager, Run Configurations CRUD + Instant Auto-Discovery,
//! and Multi-Language Test/Run Execution Engine (`EnvironmentManager`).

pub mod discovery;
mod runner;
pub mod shell_env;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use otto_core::auth::BoxFuture;
use otto_core::secrets::{delete_async, get_async, put_async, SecretStore};
use otto_core::{new_id, Error, Result};
use otto_state::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tokio::process::Command;
use tokio::sync::watch;

use crate::target::TargetManager;
use crate::toolchains::{find_executable_in, is_executable_file, read_subdirs, system_search_dirs};
use crate::types::{
    IdeCreatePythonVenvReq, IdeCreatePythonVenvResp, IdeEnvVarEntry, IdeEnvironmentsResp,
    IdeProjectEnvironment, IdePythonVenvInfo, IdeRunConfig, IdeRunState, IdeRunnerKind,
    IdeShellEnvResp, IdeSshSpec, IdeStartRunReq, IdeTargetKind, IdeTargetSpec,
    IdeUpdateEnvironmentReq, IdeUpsertRunConfigReq,
};

pub use self::discovery::discover_run_configs;
pub use self::runner::parse_test_output;
use self::runner::{evict_finished_runs, execute_run_task, ActiveRunHandle, RunStateInner};
pub use self::shell_env::{global_shell_env, parse_sentinel_env_zero, ShellEnvResolver};

/// Kick off background warming of the user's login+interactive shell environment.
/// Never blocks IDE startup or HTTP handlers.
pub fn ensure_shell_env_warming() {
    global_shell_env().ensure_warming_in_background();
}

/// Non-blocking snapshot of the cached login+interactive shell environment.
pub fn cached_shell_env_snapshot() -> BTreeMap<String, String> {
    global_shell_env().vars_snapshot()
}

/// Alias of [`cached_shell_env_snapshot`] for peer crates/modules (`lsp_host`, `dap`).
pub fn cached_shell_env_vars() -> BTreeMap<String, String> {
    global_shell_env().vars_snapshot()
}

const MAX_SYNC_PROJECT_SNAPSHOTS: usize = 16;

#[derive(Clone, Default)]
struct SyncProjectSnapshot {
    key: String,
    env: IdeProjectEnvironment,
}

fn sync_project_snapshots() -> &'static Mutex<Vec<SyncProjectSnapshot>> {
    static SNAPSHOTS: std::sync::OnceLock<Mutex<Vec<SyncProjectSnapshot>>> =
        std::sync::OnceLock::new();
    SNAPSHOTS.get_or_init(|| Mutex::new(Vec::new()))
}

fn record_sync_project_snapshot(project_key: &str, env: &IdeProjectEnvironment) {
    let mut guard = sync_project_snapshots()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(pos) = guard.iter().position(|s| s.key == project_key) {
        guard.remove(pos);
    }
    guard.push(SyncProjectSnapshot {
        key: project_key.to_string(),
        env: IdeProjectEnvironment {
            env_vars: Vec::new(),
            ..env.clone()
        },
    });
    if guard.len() > MAX_SYNC_PROJECT_SNAPSHOTS {
        let excess = guard.len() - MAX_SYNC_PROJECT_SNAPSHOTS;
        guard.drain(0..excess);
    }
}

fn lookup_sync_project_snapshot(project_key_or_root: &str) -> Option<IdeProjectEnvironment> {
    let trimmed = project_key_or_root.trim().trim_end_matches('/');
    let local_key = format!("local:{trimmed}");
    let guard = sync_project_snapshots()
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    guard
        .iter()
        .rev()
        .find(|s| {
            let sk = s.key.trim_end_matches('/');
            sk == trimmed || sk == local_key
        })
        .map(|s| s.env.clone())
}

/// Non-blocking accessor for attached multi-folder project roots (`additional_roots`),
/// keyed by either `IdeTargetSpec::project_key()` (`"local:/path"`) or raw `root`.
/// Updated in lockstep with SQLite reads/writes so synchronous `lsp_host` spawners
/// can read multi-root workspace folders without blocking on a database pool.
pub fn cached_additional_roots(project_key_or_root: &str) -> Vec<String> {
    lookup_sync_project_snapshot(project_key_or_root)
        .map(|e| e.additional_roots)
        .unwrap_or_default()
}

/// Synchronous, zero-blocking effective environment resolver for local roots
/// (used by synchronous spawners such as `lsp_host` `gopls` / `jdtls` / `pyright`).
///
/// Layers: `ottod base env` -> `cached login+interactive shell env (when inherit_shell_env)` ->
/// `selected project SDKs (GOROOT, JAVA_HOME, SDK bin) + managed toolchains + ~/go/bin on PATH` ->
/// `project .env`.
pub fn cached_effective_env_for_local_root(
    data_dir: &Path,
    root: &str,
) -> BTreeMap<String, String> {
    let proj_opt = lookup_sync_project_snapshot(root);
    let inherit_shell = proj_opt
        .as_ref()
        .map(|p| p.inherit_shell_env)
        .unwrap_or(true);

    let mut map = shell_env::collect_process_base_env();
    if inherit_shell {
        map.extend(global_shell_env().vars_snapshot());
    }

    let mut path_dirs: Vec<String> = Vec::new();
    if let Some(proj) = &proj_opt {
        if let Some(jh) = proj.java_home.as_deref().filter(|s| !s.is_empty()) {
            map.insert("JAVA_HOME".into(), jh.to_string());
            push_unique(&mut path_dirs, format!("{}/bin", jh.trim_end_matches('/')));
        }
        if let Some(gr) = proj.go_root.as_deref().filter(|s| !s.is_empty()) {
            map.insert("GOROOT".into(), gr.to_string());
            push_unique(&mut path_dirs, format!("{}/bin", gr.trim_end_matches('/')));
        }
        if let Some(nh) = proj.node_home.as_deref().filter(|s| !s.is_empty()) {
            push_unique(&mut path_dirs, format!("{}/bin", nh.trim_end_matches('/')));
        }
        if let Some(py) = proj.python_interpreter.as_deref().filter(|s| !s.is_empty()) {
            if let Some(bin_dir) = Path::new(py).parent() {
                push_unique(&mut path_dirs, bin_dir.to_string_lossy().into_owned());
            }
        }
        if let Some(extra) = proj.extra_path.as_deref().filter(|s| !s.is_empty()) {
            for seg in extra.split(':').filter(|s| !s.trim().is_empty()) {
                push_unique(&mut path_dirs, seg.trim().to_string());
            }
        }
    }
    for d in crate::toolchains::ToolchainManager::new(data_dir.to_path_buf()).extra_local_bin_dirs()
    {
        push_unique(&mut path_dirs, d.to_string_lossy().into_owned());
    }
    if !otto_core::e2e::isolated() {
        if let Some(home) = dirs::home_dir() {
            for rel in ["go/bin", ".cargo/bin", ".local/bin"] {
                let p = home.join(rel);
                if p.is_dir() {
                    push_unique(&mut path_dirs, p.to_string_lossy().into_owned());
                }
            }
        }
    }
    if let Some(gopath) = map.get("GOPATH").filter(|s| !s.trim().is_empty()) {
        for seg in std::env::split_paths(gopath) {
            push_unique(
                &mut path_dirs,
                seg.join("bin").to_string_lossy().into_owned(),
            );
        }
    }
    if let Some(existing_path) = map.get("PATH") {
        for seg in existing_path.split(':').filter(|s| !s.is_empty()) {
            push_unique(&mut path_dirs, seg.to_string());
        }
    }
    for d in system_search_dirs() {
        push_unique(&mut path_dirs, d.to_string_lossy().into_owned());
    }
    if !path_dirs.is_empty() {
        map.insert("PATH".into(), path_dirs.join(":"));
    }

    let dotenv = proj_opt
        .as_ref()
        .and_then(|p| p.dotenv_path.as_deref())
        .filter(|s| !s.trim().is_empty())
        .map(|dp| {
            let p = Path::new(dp);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                Path::new(root).join(p)
            }
        })
        .unwrap_or_else(|| Path::new(root).join(".env"));
    if dotenv.is_file() {
        if let Ok(raw) = std::fs::read_to_string(&dotenv) {
            map.extend(parse_dotenv(&raw));
        }
    }
    map
}

/// Uniform async secret accessor accepted by `EnvironmentManager` whether a
/// caller holds `&Arc<dyn SecretStore>` (`ctx.secrets()`) or `&dyn SecretStore`.
pub trait SecretAccess: Sync {
    fn get_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>>>;
    fn put_secret<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<()>>;
    fn delete_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>>;
}

impl SecretAccess for Arc<dyn SecretStore> {
    fn get_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>>> {
        let store = Arc::clone(self);
        let k = key.to_string();
        Box::pin(async move { get_async(&store, &k).await })
    }

    fn put_secret<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<()>> {
        let store = Arc::clone(self);
        let (k, v) = (key.to_string(), value.to_string());
        Box::pin(async move { put_async(&store, &k, &v).await })
    }

    fn delete_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        let store = Arc::clone(self);
        let k = key.to_string();
        Box::pin(async move { delete_async(&store, &k).await })
    }
}

impl SecretAccess for dyn SecretStore {
    #[allow(clippy::disallowed_methods)] // trait-object fallback when caller only has &dyn SecretStore
    fn get_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>>> {
        let res = match self.get_cached(key) {
            Some(cached) => Ok(cached),
            None => self.get(key),
        };
        Box::pin(async move { res })
    }

    #[allow(clippy::disallowed_methods)] // trait-object fallback when caller only has &dyn SecretStore
    fn put_secret<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<()>> {
        let res = self.put(key, value);
        Box::pin(async move { res })
    }

    #[allow(clippy::disallowed_methods)] // trait-object fallback when caller only has &dyn SecretStore
    fn delete_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        let res = self.delete(key);
        Box::pin(async move { res })
    }
}

/// Internal SQLite representation inside `ide_project_environments.env_vars_json`.
/// Never stores plaintext values when `secret == true`; stores `secret_ref` only.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEnvVar {
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(default)]
    secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_ref: Option<String>,
}

/// Manages per-project SDK/env configuration, cached login+interactive shell
/// environment, multi-folder project roots, Keychain secrets, Python venvs,
/// saved + auto-detected Run Configurations, and streaming Test/Run execution.
#[derive(Clone)]
pub struct EnvironmentManager {
    data_dir: PathBuf,
    runs: Arc<DashMap<String, Arc<ActiveRunHandle>>>,
    run_seq: Arc<AtomicU64>,
}

impl EnvironmentManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            runs: Arc::new(DashMap::new()),
            run_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    // ── Cached Login+Interactive Shell Environment Accessors ─────────────────

    pub fn cached_shell_env_snapshot(&self) -> BTreeMap<String, String> {
        global_shell_env().vars_snapshot()
    }

    pub fn shell_env_snapshot(&self) -> IdeShellEnvResp {
        global_shell_env().snapshot()
    }

    pub async fn get_shell_env(&self) -> IdeShellEnvResp {
        global_shell_env().get_or_load().await
    }

    pub async fn reload_shell_env(&self) -> IdeShellEnvResp {
        global_shell_env().reload().await
    }

    // ── SQLite Project Environment + Multi-Folder Roots + Keychain CRUD ──────

    pub async fn get_project_env(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
    ) -> Result<IdeProjectEnvironment> {
        target.validate()?;
        let (mut env, stored) = load_stored_project_env(pool, &target.project_key()).await?;
        env.env_vars = stored
            .into_iter()
            .map(|s| IdeEnvVarEntry {
                key: s.key,
                value: if s.secret { None } else { s.value },
                secret: s.secret,
                has_secret: s.secret && s.secret_ref.is_some(),
            })
            .collect();
        Ok(env)
    }

    /// Hydrate `target.additional_roots` with any folders persisted in SQLite for
    /// this workspace/project (`<1ms` indexed primary-key lookup).
    pub async fn enrich_target_roots(&self, pool: &DbPool, target: &mut IdeTargetSpec) {
        if let Ok((proj, _)) = load_stored_project_env(pool, &target.project_key()).await {
            for r in proj.additional_roots {
                if !r.is_empty() && r != target.root && !target.additional_roots.contains(&r) {
                    target.additional_roots.push(r);
                }
            }
        }
    }

    /// Attach an additional project folder (`POST /ide/projects/folders`).
    pub async fn add_project_folder(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        folder_path: &str,
    ) -> Result<IdeProjectEnvironment> {
        target.validate()?;
        let normalized = validate_attached_folder_path(target, folder_path)?;
        let project_key = target.project_key();
        let (mut proj, _) = load_stored_project_env(pool, &project_key).await?;
        if normalized != target.root && !proj.additional_roots.contains(&normalized) {
            proj.additional_roots.push(normalized);
        }
        persist_project_roots(pool, target, &proj.additional_roots).await?;
        self.get_project_env(pool, target).await
    }

    /// Detach an attached project folder (`DELETE /ide/projects/folders`).
    pub async fn remove_project_folder(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        folder_path: &str,
    ) -> Result<IdeProjectEnvironment> {
        target.validate()?;
        let raw_trimmed = folder_path.trim().trim_end_matches('/');
        if raw_trimmed.is_empty() {
            return Err(Error::Invalid("folder path must not be empty".into()));
        }
        let canon_opt = std::fs::canonicalize(raw_trimmed)
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        let project_key = target.project_key();
        let (mut proj, _) = load_stored_project_env(pool, &project_key).await?;
        proj.additional_roots.retain(|entry| {
            let trimmed = entry.trim_end_matches('/');
            trimmed != raw_trimmed && canon_opt.as_deref() != Some(entry.as_str())
        });
        persist_project_roots(pool, target, &proj.additional_roots).await?;
        self.get_project_env(pool, target).await
    }

    pub async fn get_environments<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        _secrets: &S,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
    ) -> Result<IdeEnvironmentsResp> {
        target.validate()?;
        let (mut env, stored_vars) = load_stored_project_env(pool, &target.project_key()).await?;
        for extra in &target.additional_roots {
            if !extra.is_empty() && extra != &target.root && !env.additional_roots.contains(extra) {
                env.additional_roots.push(extra.clone());
            }
        }
        env.env_vars = stored_vars
            .iter()
            .map(|s| IdeEnvVarEntry {
                key: s.key.clone(),
                value: if s.secret { None } else { s.value.clone() },
                secret: s.secret,
                has_secret: s.secret && s.secret_ref.is_some(),
            })
            .collect();

        let saved_configs = self.list_run_configs(pool, target).await?;

        let (dotenv_files, python_venvs, detected_configs) = match target.kind {
            IdeTargetKind::Local => {
                let target_clone = target.clone();
                let extra_roots = env.additional_roots.clone();
                let data_dir = self.data_dir.clone();
                tokio::task::spawn_blocking(move || {
                    let root = PathBuf::from(&target_clone.root);
                    let dotenvs = discover_local_dotenv_files(&root);
                    let venvs = discover_local_python_venvs(&root, &data_dir);
                    let detected = discover_run_configs(&target_clone, &extra_roots);
                    (dotenvs, venvs, detected)
                })
                .await
                .map_err(|e| Error::Internal(format!("local environment discovery: {e}")))?
            }
            IdeTargetKind::Ssh => {
                let dotenvs = self
                    .discover_dotenv_files(targets, target, ssh)
                    .await
                    .unwrap_or_default();
                let venvs = self
                    .discover_python_venvs(targets, target, ssh)
                    .await
                    .unwrap_or_default();
                (dotenvs, venvs, Vec::new())
            }
        };

        let mut configs = saved_configs;
        for det in &detected_configs {
            let duplicate = configs.iter().any(|c| {
                c.id == det.id
                    || (c.runner == det.runner
                        && c.name.eq_ignore_ascii_case(&det.name)
                        && c.cwd == det.cwd)
            });
            if !duplicate {
                configs.push(det.clone());
            }
        }

        // Fast non-blocking effective_env preview (zero subprocesses, secrets redacted).
        let effective_env = self
            .resolve_env_layers(&env, &stored_vars, None::<&S>, target, None, false)
            .await;

        Ok(IdeEnvironmentsResp {
            env,
            python_venvs,
            dotenv_files,
            configs,
            detected_configs,
            effective_env,
        })
    }

    pub async fn update_environment<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        req: IdeUpdateEnvironmentReq,
    ) -> Result<IdeProjectEnvironment> {
        req.target.validate()?;
        let project_key = req.target.project_key();
        let (prev_proj, prev_stored) = load_stored_project_env(pool, &project_key).await?;
        let prev_map: BTreeMap<String, StoredEnvVar> = prev_stored
            .into_iter()
            .map(|e| (e.key.clone(), e))
            .collect();

        let mut next_stored = Vec::with_capacity(req.env_vars.len());
        let mut kept_refs = HashSet::new();

        for item in &req.env_vars {
            let key = item.key.trim().to_string();
            if key.is_empty() || !is_valid_env_key(&key) {
                return Err(Error::Invalid(format!(
                    "invalid environment variable name '{key}'"
                )));
            }
            if item.secret {
                let sref = secret_key_ref(&project_key, &key);
                if let Some(val) = item.value.as_deref().filter(|v| !v.is_empty()) {
                    secrets.put_secret(&sref, val).await?;
                    kept_refs.insert(sref.clone());
                    next_stored.push(StoredEnvVar {
                        key,
                        value: None,
                        secret: true,
                        secret_ref: Some(sref),
                    });
                } else if let Some(prev_ref) = prev_map.get(&key).and_then(|p| p.secret_ref.clone())
                {
                    kept_refs.insert(prev_ref.clone());
                    next_stored.push(StoredEnvVar {
                        key,
                        value: None,
                        secret: true,
                        secret_ref: Some(prev_ref),
                    });
                } else {
                    next_stored.push(StoredEnvVar {
                        key,
                        value: None,
                        secret: true,
                        secret_ref: None,
                    });
                }
            } else {
                next_stored.push(StoredEnvVar {
                    key,
                    value: Some(item.value.clone().unwrap_or_default()),
                    secret: false,
                    secret_ref: None,
                });
            }
        }

        for old in prev_map.values() {
            if let Some(r) = &old.secret_ref {
                if !kept_refs.contains(r) {
                    let _ = secrets.delete_secret(r).await;
                }
            }
        }

        let inherit_shell_env = req.inherit_shell_env.unwrap_or(prev_proj.inherit_shell_env);
        let next_roots = match &req.additional_roots {
            Some(explicit_roots) => {
                let mut validated = Vec::new();
                for r in explicit_roots {
                    if r.trim().is_empty() || r.trim() == req.target.root {
                        continue;
                    }
                    let norm = validate_attached_folder_path(&req.target, r)?;
                    if norm != req.target.root && !validated.contains(&norm) {
                        validated.push(norm);
                    }
                }
                validated
            }
            None => prev_proj.additional_roots,
        };

        let env_vars_json = serde_json::to_string(&next_stored)
            .map_err(|e| Error::Internal(format!("serialize env_vars_json: {e}")))?;
        let roots_json = serde_json::to_string(&next_roots)
            .map_err(|e| Error::Internal(format!("serialize additional_roots_json: {e}")))?;
        let now = Utc::now().to_rfc3339();
        let target_kind = match req.target.kind {
            IdeTargetKind::Local => "local",
            IdeTargetKind::Ssh => "ssh",
        };

        sqlx::query(
            r#"INSERT INTO ide_project_environments (
                   project_key, target_kind, root_path, connection_id,
                   java_home, go_root, rust_toolchain, node_home,
                   python_interpreter, dotenv_path, extra_path, env_vars_json,
                   additional_roots_json, inherit_shell_env, updated_at
               ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(project_key) DO UPDATE SET
                   target_kind = excluded.target_kind,
                   root_path = excluded.root_path,
                   connection_id = excluded.connection_id,
                   java_home = excluded.java_home,
                   go_root = excluded.go_root,
                   rust_toolchain = excluded.rust_toolchain,
                   node_home = excluded.node_home,
                   python_interpreter = excluded.python_interpreter,
                   dotenv_path = excluded.dotenv_path,
                   extra_path = excluded.extra_path,
                   env_vars_json = excluded.env_vars_json,
                   additional_roots_json = excluded.additional_roots_json,
                   inherit_shell_env = excluded.inherit_shell_env,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&project_key)
        .bind(target_kind)
        .bind(&req.target.root)
        .bind(&req.target.connection_id)
        .bind(clean_opt(&req.java_home))
        .bind(clean_opt(&req.go_root))
        .bind(clean_opt(&req.rust_toolchain))
        .bind(clean_opt(&req.node_home))
        .bind(clean_opt(&req.python_interpreter))
        .bind(clean_opt(&req.dotenv_path))
        .bind(clean_opt(&req.extra_path))
        .bind(&env_vars_json)
        .bind(&roots_json)
        .bind(i32::from(inherit_shell_env))
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| Error::Internal(format!("upsert ide_project_environments: {e}")))?;

        self.get_project_env(pool, &req.target).await
    }

    // ── Effective Environment & PATH Builder (Strict IntelliJ Order) ─────────

    pub async fn effective_env<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
    ) -> Result<BTreeMap<String, String>> {
        self.build_effective_env(pool, secrets, target).await
    }

    pub async fn build_effective_env<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
    ) -> Result<BTreeMap<String, String>> {
        self.build_effective_env_for_run(pool, secrets, target, None)
            .await
    }

    /// Resolve process environment variables in strict IntelliJ / GoLand order:
    /// 1. `ottod` base process env (`Local`)
    /// 2. Cached user login+interactive shell env (`$SHELL -lic`, when `inherit_shell_env = true`)
    /// 3. Selected SDK / Toolchain env (`GOROOT`, `JAVA_HOME`, `VIRTUAL_ENV`, prepended SDK `bin` + `~/go/bin` on `PATH`)
    /// 4. Project `.env` file + Run Configuration `env_file`
    /// 5. Project `env_vars` (+ Keychain secrets) -> Explicit Run/Debug Config `env` (`env_overrides`)
    pub async fn build_effective_env_for_run<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
        cfg: Option<&IdeRunConfig>,
    ) -> Result<BTreeMap<String, String>> {
        target.validate()?;
        let (proj, stored_vars) = load_stored_project_env(pool, &target.project_key()).await?;
        Ok(self
            .resolve_env_layers(&proj, &stored_vars, Some(secrets), target, cfg, true)
            .await)
    }

    async fn resolve_env_layers<S: SecretAccess + ?Sized>(
        &self,
        proj: &IdeProjectEnvironment,
        stored_vars: &[StoredEnvVar],
        secrets: Option<&S>,
        target: &IdeTargetSpec,
        cfg: Option<&IdeRunConfig>,
        await_shell: bool,
    ) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        let inherit_shell =
            proj.inherit_shell_env && cfg.map(|c| c.inherit_shell_env).unwrap_or(true);

        // Layer 1 & 2: ottod base env -> cached login+interactive shell env
        if target.kind == IdeTargetKind::Local {
            map.extend(shell_env::collect_process_base_env());
            if inherit_shell {
                let shell_vars = if await_shell {
                    global_shell_env().vars_or_load().await
                } else {
                    global_shell_env().vars_snapshot()
                };
                map.extend(shell_vars);
            }
        }

        // Layer 3: Selected SDK / Toolchain env + prepended PATH
        let mut path_dirs: Vec<String> = Vec::new();
        if let Some(extra) = proj.extra_path.as_deref().filter(|s| !s.is_empty()) {
            path_dirs.extend(
                extra
                    .split(':')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            );
        }

        // Per-Run-Config SDK override (`sdk_path`) takes precedence over project-wide SDK.
        let cfg_sdk = cfg
            .and_then(|c| c.sdk_path.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let (Some(c), Some(sdk_p)) = (cfg, cfg_sdk) {
            apply_run_config_sdk_override(c.runner, sdk_p, target.kind, &mut map, &mut path_dirs);
        }

        if let Some(py) = proj.python_interpreter.as_deref().filter(|s| !s.is_empty()) {
            if let Some(bin_dir) = Path::new(py).parent() {
                push_unique(&mut path_dirs, bin_dir.to_string_lossy().into_owned());
                if let Some(venv) = bin_dir.parent() {
                    if venv.join("pyvenv.cfg").is_file() || target.kind == IdeTargetKind::Ssh {
                        map.entry("VIRTUAL_ENV".into())
                            .or_insert_with(|| venv.to_string_lossy().into_owned());
                    }
                }
            }
        }
        if let Some(jh) = proj.java_home.as_deref().filter(|s| !s.is_empty()) {
            map.entry("JAVA_HOME".into())
                .or_insert_with(|| jh.to_string());
            push_unique(&mut path_dirs, format!("{}/bin", jh.trim_end_matches('/')));
        }
        if let Some(gr) = proj.go_root.as_deref().filter(|s| !s.is_empty()) {
            map.entry("GOROOT".into()).or_insert_with(|| gr.to_string());
            push_unique(&mut path_dirs, format!("{}/bin", gr.trim_end_matches('/')));
        }
        if let Some(rt) = proj.rust_toolchain.as_deref().filter(|s| !s.is_empty()) {
            if rt.starts_with('/') {
                push_unique(&mut path_dirs, format!("{}/bin", rt.trim_end_matches('/')));
            } else {
                map.entry("RUSTUP_TOOLCHAIN".into())
                    .or_insert_with(|| rt.to_string());
            }
        }
        if let Some(nh) = proj.node_home.as_deref().filter(|s| !s.is_empty()) {
            push_unique(&mut path_dirs, format!("{}/bin", nh.trim_end_matches('/')));
        }

        if target.kind == IdeTargetKind::Local {
            for d in crate::toolchains::ToolchainManager::new(self.data_dir.clone())
                .extra_local_bin_dirs()
            {
                push_unique(&mut path_dirs, d.to_string_lossy().into_owned());
            }
            if !otto_core::e2e::isolated() {
                if let Some(home) = dirs::home_dir() {
                    for rel in ["go/bin", ".cargo/bin", ".local/bin"] {
                        let p = home.join(rel);
                        if p.is_dir() {
                            push_unique(&mut path_dirs, p.to_string_lossy().into_owned());
                        }
                    }
                }
            }
            if let Some(gopath) = map.get("GOPATH").filter(|s| !s.trim().is_empty()) {
                for seg in std::env::split_paths(gopath) {
                    push_unique(
                        &mut path_dirs,
                        seg.join("bin").to_string_lossy().into_owned(),
                    );
                }
            }
            // Preserve every directory from the user's inherited login+interactive shell PATH!
            if let Some(inherited_path) = map.get("PATH") {
                for seg in inherited_path.split(':').filter(|s| !s.is_empty()) {
                    push_unique(&mut path_dirs, seg.to_string());
                }
            }
            for d in system_search_dirs() {
                push_unique(&mut path_dirs, d.to_string_lossy().into_owned());
            }
        } else {
            for rel in [
                "$HOME/.otto/toolchains/go/bin",
                "$HOME/.otto/toolchains/rust/bin",
                "$HOME/.otto/toolchains/rust/cargo/bin",
                "$HOME/.otto/toolchains/python/bin",
                "/opt/homebrew/bin",
                "/usr/local/go/bin",
                "/usr/local/bin",
                "/usr/bin",
                "/bin",
            ] {
                push_unique(&mut path_dirs, rel.to_string());
            }
        }
        if !path_dirs.is_empty() {
            map.insert("PATH".into(), path_dirs.join(":"));
        }

        // Layer 4: Project `.env` file + Run Configuration `env_file`
        if target.kind == IdeTargetKind::Local {
            let mut all_roots = vec![target.root.clone()];
            for r in proj
                .additional_roots
                .iter()
                .chain(target.additional_roots.iter())
            {
                if !r.is_empty() && !all_roots.contains(r) {
                    all_roots.push(r.clone());
                }
            }

            if let Some(dotenv_rel) = proj.dotenv_path.as_deref().filter(|s| !s.is_empty()) {
                if let Some(dotenv_file) = safe_join_allowed_roots(&all_roots, dotenv_rel) {
                    if let Ok(raw) = tokio::fs::read_to_string(&dotenv_file).await {
                        map.extend(parse_dotenv(&raw));
                    }
                }
            } else {
                let default_dotenv = Path::new(&target.root).join(".env");
                if default_dotenv.is_file() {
                    if let Ok(raw) = tokio::fs::read_to_string(&default_dotenv).await {
                        map.extend(parse_dotenv(&raw));
                    }
                }
            }

            if let Some(cfg_env_file) = cfg
                .and_then(|c| c.env_file.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if let Some(resolved_ef) = safe_join_allowed_roots(&all_roots, cfg_env_file) {
                    if let Ok(raw) = tokio::fs::read_to_string(&resolved_ef).await {
                        map.extend(parse_dotenv(&raw));
                    }
                }
            }
        }

        // Layer 5: Project `env_vars` (+ Keychain secrets) -> Run Config `env` (`env_overrides`)
        for item in stored_vars {
            if item.secret {
                if let Some(sec_store) = secrets {
                    if let Some(sref) = &item.secret_ref {
                        if let Ok(Some(secret_val)) = sec_store.get_secret(sref).await {
                            map.insert(item.key.clone(), secret_val);
                        }
                    }
                } else if item.secret_ref.is_some() {
                    // Redacted placeholder in HTTP preview (`GET /ide/environments`)
                    map.insert(item.key.clone(), "••••••••".into());
                }
            } else if let Some(val) = &item.value {
                map.insert(item.key.clone(), val.clone());
            }
        }

        if let Some(c) = cfg {
            for (k, v) in &c.env_overrides {
                if is_valid_env_key(k) {
                    map.insert(k.clone(), v.clone());
                }
            }
        }

        map
    }

    // ── `.env` & Python Virtualenv Discovery + 1-Click Creation ──────────────

    pub async fn discover_dotenv_files(
        &self,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
    ) -> Result<Vec<String>> {
        match target.kind {
            IdeTargetKind::Local => Ok(discover_local_dotenv_files(Path::new(&target.root))),
            IdeTargetKind::Ssh => {
                let Some(spec) = ssh else {
                    return Ok(Vec::new());
                };
                let resolved = targets.resolve_with_ssh(target, "ide-env", spec.clone())?;
                let script = "for f in .env .env.local .env.development .env.test .env.staging .env.production; do [ -f \"$f\" ] && echo \"$f\"; done";
                let out = targets
                    .exec_shell(
                        &resolved,
                        script,
                        Some(&target.root),
                        &BTreeMap::new(),
                        Duration::from_secs(10),
                    )
                    .await?;
                Ok(out
                    .stdout
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect())
            }
        }
    }

    pub async fn discover_python_venvs(
        &self,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
    ) -> Result<Vec<IdePythonVenvInfo>> {
        match target.kind {
            IdeTargetKind::Local => {
                let root = PathBuf::from(&target.root);
                let data_dir = self.data_dir.clone();
                tokio::task::spawn_blocking(move || discover_local_python_venvs(&root, &data_dir))
                    .await
                    .map_err(|e| Error::Internal(format!("python venv discovery: {e}")))
            }
            IdeTargetKind::Ssh => {
                let Some(spec) = ssh else {
                    return Ok(Vec::new());
                };
                let resolved = targets.resolve_with_ssh(target, "ide-env", spec.clone())?;
                let script = "for d in .venv venv env; do [ -x \"$d/bin/python\" ] && echo \"venv:$PWD/$d:$($d/bin/python --version 2>&1)\"; done; for d in \"$HOME\"/.conda/envs/* \"$HOME\"/miniconda3/envs/*; do [ -x \"$d/bin/python\" ] && echo \"conda:$d:$($d/bin/python --version 2>&1)\"; done";
                let out = targets
                    .exec_shell(
                        &resolved,
                        script,
                        Some(&target.root),
                        &BTreeMap::new(),
                        Duration::from_secs(12),
                    )
                    .await?;
                Ok(out
                    .stdout
                    .lines()
                    .filter_map(|l| {
                        let mut p = l.trim().splitn(3, ':');
                        Some(IdePythonVenvInfo {
                            kind: p.next()?.to_string(),
                            path: p.next()?.to_string(),
                            version: p.next()?.trim_start_matches("Python ").trim().to_string(),
                        })
                    })
                    .collect())
            }
        }
    }

    pub async fn create_python_venv(
        &self,
        pool: &DbPool,
        targets: &TargetManager,
        req: IdeCreatePythonVenvReq,
        ssh: Option<&IdeSshSpec>,
    ) -> Result<IdeCreatePythonVenvResp> {
        req.target.validate()?;
        let rel_dir = req
            .venv_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(".venv");
        if rel_dir.contains("..") || rel_dir.starts_with('/') {
            return Err(Error::Invalid(
                "venv_dir must be a safe workspace-relative directory".into(),
            ));
        }

        let resp = match req.target.kind {
            IdeTargetKind::Local => {
                self.create_local_venv(&req.target.root, rel_dir, req.python_version.as_deref())
                    .await?
            }
            IdeTargetKind::Ssh => {
                let spec = ssh.ok_or_else(|| Error::Invalid("SSH spec required".into()))?;
                let resolved = targets.resolve_with_ssh(&req.target, "ide-venv", spec.clone())?;
                let q_dir = shell_words::quote(rel_dir);
                let script = match req.python_version.as_deref().filter(|s| !s.is_empty()) {
                    Some(ver) => format!(
                        "if command -v uv >/dev/null 2>&1; then uv venv --python {} {q_dir}; else python3 -m venv {q_dir}; fi && echo \"VER:$({q_dir}/bin/python --version 2>&1)\"",
                        shell_words::quote(ver)
                    ),
                    None => format!(
                        "if command -v uv >/dev/null 2>&1; then uv venv {q_dir}; else python3 -m venv {q_dir}; fi && echo \"VER:$({q_dir}/bin/python --version 2>&1)\""
                    ),
                };
                let out = targets
                    .exec_shell(
                        &resolved,
                        &script,
                        Some(&req.target.root),
                        &BTreeMap::new(),
                        Duration::from_secs(120),
                    )
                    .await?;
                if !out.success() {
                    return Err(Error::Internal(format!(
                        "remote venv creation failed: {}",
                        out.stderr.trim()
                    )));
                }
                let ver = out
                    .stdout
                    .lines()
                    .find_map(|l| l.strip_prefix("VER:"))
                    .unwrap_or("3")
                    .trim_start_matches("Python ")
                    .trim()
                    .to_string();
                let venv_path = format!("{}/{rel_dir}", req.target.root.trim_end_matches('/'));
                IdeCreatePythonVenvResp {
                    interpreter_path: format!("{venv_path}/bin/python"),
                    path: venv_path,
                    version: ver,
                }
            }
        };

        let (existing, _) = load_stored_project_env(pool, &req.target.project_key()).await?;
        if existing.python_interpreter.is_none() {
            let now = Utc::now().to_rfc3339();
            let _ = sqlx::query(
                r#"INSERT INTO ide_project_environments (project_key, root_path, python_interpreter, updated_at)
                   VALUES (?, ?, ?, ?)
                   ON CONFLICT(project_key) DO UPDATE SET python_interpreter = excluded.python_interpreter, updated_at = excluded.updated_at"#,
            )
            .bind(req.target.project_key())
            .bind(&req.target.root)
            .bind(&resp.interpreter_path)
            .bind(&now)
            .execute(pool)
            .await;
        }
        Ok(resp)
    }

    async fn create_local_venv(
        &self,
        root: &str,
        rel_dir: &str,
        py_ver: Option<&str>,
    ) -> Result<IdeCreatePythonVenvResp> {
        let venv_path = PathBuf::from(root).join(rel_dir);
        let search = system_search_dirs();
        let managed_uv = self.data_dir.join("toolchains/python/bin/uv");
        let uv_bin = if is_executable_file(&managed_uv) {
            Some(managed_uv)
        } else {
            find_executable_in(&search, &["uv"])
        };

        let status_out = if let Some(uv) = uv_bin {
            let mut cmd = Command::new(uv);
            cmd.arg("venv");
            if let Some(v) = py_ver.filter(|s| !s.trim().is_empty()) {
                cmd.args(["--python", v]);
            }
            cmd.arg(&venv_path).kill_on_drop(true).output().await
        } else {
            let py = find_executable_in(&search, &["python3", "python"])
                .ok_or_else(|| Error::Invalid("neither uv nor python3 found on host".into()))?;
            Command::new(py)
                .args(["-m", "venv"])
                .arg(&venv_path)
                .kill_on_drop(true)
                .output()
                .await
        }
        .map_err(|e| Error::Internal(format!("spawn venv creator: {e}")))?;

        if !status_out.status.success() {
            return Err(Error::Internal(format!(
                "venv creation failed: {}",
                String::from_utf8_lossy(&status_out.stderr).trim()
            )));
        }
        let interp = venv_path.join("bin/python");
        let version = read_pyvenv_version(&venv_path).unwrap_or_else(|| "3".into());
        Ok(IdeCreatePythonVenvResp {
            path: venv_path.to_string_lossy().into_owned(),
            interpreter_path: interp.to_string_lossy().into_owned(),
            version,
        })
    }

    // ── Run Configurations SQLite CRUD (`ide_run_configs`) + Auto Lookup ─────

    pub async fn list_run_configs(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
    ) -> Result<Vec<IdeRunConfig>> {
        target.validate()?;
        let rows = sqlx::query(
            r#"SELECT id, name, runner, command, args_json, build_flags_json, cwd,
                      env_overrides_json, env_file, inherit_shell_env, sdk_path,
                      toolchain_id, before_launch, target_path, test_filter, compound_ids_json
               FROM ide_run_configs
               WHERE project_key = ?
               ORDER BY created_at ASC, name ASC"#,
        )
        .bind(target.project_key())
        .fetch_all(pool)
        .await
        .map_err(|e| Error::Internal(format!("list ide_run_configs: {e}")))?;

        Ok(rows.iter().map(row_to_run_config).collect())
    }

    pub async fn get_run_config(&self, pool: &DbPool, id: &str) -> Result<IdeRunConfig> {
        let row = sqlx::query(
            r#"SELECT id, name, runner, command, args_json, build_flags_json, cwd,
                      env_overrides_json, env_file, inherit_shell_env, sdk_path,
                      toolchain_id, before_launch, target_path, test_filter, compound_ids_json
               FROM ide_run_configs WHERE id = ?"#,
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| Error::Internal(format!("get ide_run_config: {e}")))?
        .ok_or_else(|| Error::NotFound(format!("run config '{id}'")))?;
        Ok(row_to_run_config(&row))
    }

    /// Resolve a saved SQLite Run Configuration OR an `auto:*` live-discovered
    /// Run Configuration for `target`.
    pub async fn resolve_run_config_or_auto(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        id: &str,
    ) -> Result<IdeRunConfig> {
        if let Ok(saved) = self.get_run_config(pool, id).await {
            return Ok(saved);
        }
        if id.starts_with("auto:") && target.kind == IdeTargetKind::Local {
            let mut enriched = target.clone();
            self.enrich_target_roots(pool, &mut enriched).await;
            let extra = enriched.additional_roots.clone();
            let id_owned = id.to_string();
            let found = tokio::task::spawn_blocking(move || {
                discover_run_configs(&enriched, &extra)
                    .into_iter()
                    .find(|c| c.id == id_owned)
            })
            .await
            .ok()
            .flatten();
            if let Some(cfg) = found {
                return Ok(cfg);
            }
        }
        Err(Error::NotFound(format!("run config '{id}'")))
    }

    pub async fn upsert_run_config(
        &self,
        pool: &DbPool,
        req: IdeUpsertRunConfigReq,
    ) -> Result<IdeRunConfig> {
        self.upsert_run_config_parts(pool, &req.target, req.config)
            .await
    }

    pub async fn upsert_run_config_parts(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        mut cfg: IdeRunConfig,
    ) -> Result<IdeRunConfig> {
        target.validate()?;
        if cfg.name.trim().is_empty() {
            return Err(Error::Invalid(
                "run configuration name cannot be empty".into(),
            ));
        }
        if cfg.id.trim().is_empty() || cfg.id.starts_with("auto:") {
            cfg.id = format!("rc_{}", new_id());
        }
        cfg.auto_detected = false;
        let target_kind = match target.kind {
            IdeTargetKind::Local => "local",
            IdeTargetKind::Ssh => "ssh",
        };
        let runner_str = serde_json::to_value(cfg.runner)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "custom".into());
        let args_json = serde_json::to_string(&cfg.args).unwrap_or_else(|_| "[]".into());
        let build_flags_json =
            serde_json::to_string(&cfg.build_flags).unwrap_or_else(|_| "[]".into());
        let env_overrides_json =
            serde_json::to_string(&cfg.env_overrides).unwrap_or_else(|_| "{}".into());
        let compound_ids_json =
            serde_json::to_string(&cfg.compound_configs).unwrap_or_else(|_| "[]".into());
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"INSERT INTO ide_run_configs (
                   id, project_key, target_kind, connection_id, root_path,
                   name, runner, command, args_json, build_flags_json, cwd,
                   env_overrides_json, env_file, inherit_shell_env, sdk_path,
                   toolchain_id, before_launch, target_path, test_filter, compound_ids_json,
                   created_at, updated_at
               ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   project_key = excluded.project_key,
                   target_kind = excluded.target_kind,
                   connection_id = excluded.connection_id,
                   root_path = excluded.root_path,
                   name = excluded.name,
                   runner = excluded.runner,
                   command = excluded.command,
                   args_json = excluded.args_json,
                   build_flags_json = excluded.build_flags_json,
                   cwd = excluded.cwd,
                   env_overrides_json = excluded.env_overrides_json,
                   env_file = excluded.env_file,
                   inherit_shell_env = excluded.inherit_shell_env,
                   sdk_path = excluded.sdk_path,
                   toolchain_id = excluded.toolchain_id,
                   before_launch = excluded.before_launch,
                   target_path = excluded.target_path,
                   test_filter = excluded.test_filter,
                   compound_ids_json = excluded.compound_ids_json,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&cfg.id)
        .bind(target.project_key())
        .bind(target_kind)
        .bind(&target.connection_id)
        .bind(&target.root)
        .bind(cfg.name.trim())
        .bind(&runner_str)
        .bind(clean_opt(&cfg.command))
        .bind(&args_json)
        .bind(&build_flags_json)
        .bind(clean_opt(&cfg.cwd))
        .bind(&env_overrides_json)
        .bind(clean_opt(&cfg.env_file))
        .bind(i32::from(cfg.inherit_shell_env))
        .bind(clean_opt(&cfg.sdk_path))
        .bind(clean_opt(&cfg.toolchain_id))
        .bind(clean_opt(&cfg.before_launch))
        .bind(clean_opt(&cfg.target_path))
        .bind(clean_opt(&cfg.test_filter))
        .bind(&compound_ids_json)
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| Error::Internal(format!("upsert ide_run_config: {e}")))?;

        Ok(cfg)
    }

    pub async fn delete_run_config(&self, pool: &DbPool, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM ide_run_configs WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| Error::Internal(format!("delete ide_run_config: {e}")))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("run config '{id}'")));
        }
        Ok(())
    }

    // ── Test & Run Execution Engine (Local + Remote SSH) ─────────────────────

    pub fn get_run(&self, run_id: &str) -> Result<IdeRunState> {
        self.get_run_with_tail(run_id, None)
    }

    pub fn get_run_with_tail(
        &self,
        run_id: &str,
        tail_lines: Option<usize>,
    ) -> Result<IdeRunState> {
        self.runs
            .get(run_id)
            .map(|e| e.value().snapshot_with_tail(tail_lines))
            .ok_or_else(|| Error::NotFound(format!("IDE run '{run_id}'")))
    }

    pub fn list_runs(&self) -> Vec<IdeRunState> {
        let mut out: Vec<IdeRunState> = self.runs.iter().map(|e| e.value().snapshot()).collect();
        out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        out
    }

    pub async fn stop_run(&self, run_id: &str) -> Result<IdeRunState> {
        let snap = self
            .runs
            .get(run_id)
            .map(|e| e.value().stop())
            .ok_or_else(|| Error::NotFound(format!("IDE run '{run_id}'")))?;
        evict_finished_runs(&self.runs);
        Ok(snap)
    }

    pub async fn start_run<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        targets: &TargetManager,
        mut req: IdeStartRunReq,
        ssh: Option<IdeSshSpec>,
    ) -> Result<IdeRunState> {
        req.target.validate()?;
        self.enrich_target_roots(pool, &mut req.target).await;

        let cfg = if let Some(adhoc) = req.adhoc {
            adhoc
        } else if let Some(cid) = req.config_id.as_deref().filter(|s| !s.is_empty()) {
            self.resolve_run_config_or_auto(pool, &req.target, cid)
                .await?
        } else {
            return Err(Error::Invalid(
                "start_run requires either config_id or adhoc configuration".into(),
            ));
        };

        let mut steps: Vec<(IdeRunConfig, BTreeMap<String, String>)> = Vec::new();
        if cfg.runner == IdeRunnerKind::Compound && !cfg.compound_configs.is_empty() {
            for member_id in cfg.compound_configs.iter().take(12) {
                if member_id == &cfg.id {
                    continue;
                }
                let member_cfg = self
                    .resolve_run_config_or_auto(pool, &req.target, member_id)
                    .await?;
                if member_cfg.runner == IdeRunnerKind::Compound {
                    continue;
                }
                let member_env = self
                    .build_effective_env_for_run(pool, secrets, &req.target, Some(&member_cfg))
                    .await?;
                steps.push((member_cfg, member_env));
            }
            if steps.is_empty() {
                return Err(Error::Invalid(
                    "compound run configuration contains no runnable steps".into(),
                ));
            }
        } else {
            let env_map = self
                .build_effective_env_for_run(pool, secrets, &req.target, Some(&cfg))
                .await?;
            steps.push((cfg.clone(), env_map));
        }

        let run_id = format!("run_{}", new_id());
        let title = if cfg.name.trim().is_empty() {
            format!("{:?}", cfg.runner)
        } else {
            cfg.name.clone()
        };
        let started_at = Utc::now().to_rfc3339();
        let seq = self.run_seq.fetch_add(1, Ordering::Relaxed);
        let (stop_tx, stop_rx) = watch::channel(false);
        let handle = Arc::new(ActiveRunHandle {
            seq,
            state: Mutex::new(RunStateInner::new(
                run_id.clone(),
                title,
                cfg.runner,
                started_at,
            )),
            stop_tx,
        });
        let initial = handle.snapshot();
        self.runs.insert(run_id, Arc::clone(&handle));
        evict_finished_runs(&self.runs);

        let runs_clone = Arc::clone(&self.runs);
        let targets_clone = targets.clone();
        let target_spec = req.target;
        tokio::spawn(async move {
            execute_run_task(
                runs_clone,
                handle,
                targets_clone,
                target_spec,
                ssh,
                steps,
                stop_rx,
            )
            .await;
        });

        Ok(initial)
    }
}

// ── Pure `.env` Parser & SQLite / Filesystem Helpers ─────────────────────────

/// Safe, non-evaluating `.env` key-value parser supporting `#` comments,
/// `export KEY=VAL`, single/double quotes, and `\n` / `\"` escapes.
pub fn parse_dotenv(content: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
        let Some((raw_k, raw_v)) = line.split_once('=') else {
            continue;
        };
        let key = raw_k.trim();
        if !is_valid_env_key(key) {
            continue;
        }
        let v = raw_v.trim();
        let val = if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
            v[1..v.len() - 1]
                .replace("\\n", "\n")
                .replace("\\r", "\r")
                .replace("\\t", "\t")
                .replace("\\\"", "\"")
        } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
            v[1..v.len() - 1].to_string()
        } else {
            v.split(" #").next().unwrap_or(v).trim().to_string()
        };
        out.insert(key.to_string(), val);
    }
    out
}

pub(super) fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn secret_key_ref(project_key: &str, var_name: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in project_key.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("ide-env:{hash:016x}:{var_name}")
}

fn clean_opt(opt: &Option<String>) -> Option<String> {
    opt.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn push_unique(vec: &mut Vec<String>, item: String) {
    if !item.is_empty() && !vec.contains(&item) {
        vec.push(item);
    }
}

fn apply_run_config_sdk_override(
    runner: IdeRunnerKind,
    sdk_path: &str,
    target_kind: IdeTargetKind,
    map: &mut BTreeMap<String, String>,
    path_dirs: &mut Vec<String>,
) {
    let trimmed = sdk_path.trim_end_matches('/');
    match runner {
        IdeRunnerKind::GoRun | IdeRunnerKind::GoBuild | IdeRunnerKind::GoTest => {
            map.insert("GOROOT".into(), trimmed.to_string());
            push_unique(path_dirs, format!("{trimmed}/bin"));
        }
        IdeRunnerKind::JavaMain
        | IdeRunnerKind::JavaJunit
        | IdeRunnerKind::Maven
        | IdeRunnerKind::Gradle => {
            map.insert("JAVA_HOME".into(), trimmed.to_string());
            push_unique(path_dirs, format!("{trimmed}/bin"));
        }
        IdeRunnerKind::CargoRun | IdeRunnerKind::CargoTest => {
            if trimmed.starts_with('/') {
                push_unique(path_dirs, format!("{trimmed}/bin"));
            } else {
                map.insert("RUSTUP_TOOLCHAIN".into(), trimmed.to_string());
            }
        }
        IdeRunnerKind::Python | IdeRunnerKind::Pytest => {
            let p = Path::new(trimmed);
            if p.is_file() {
                if let Some(bin_dir) = p.parent() {
                    push_unique(path_dirs, bin_dir.to_string_lossy().into_owned());
                    if let Some(venv) = bin_dir.parent() {
                        if venv.join("pyvenv.cfg").is_file() || target_kind == IdeTargetKind::Ssh {
                            map.insert("VIRTUAL_ENV".into(), venv.to_string_lossy().into_owned());
                        }
                    }
                }
            } else {
                push_unique(path_dirs, format!("{trimmed}/bin"));
            }
        }
        IdeRunnerKind::Node | IdeRunnerKind::Npm | IdeRunnerKind::Vitest => {
            push_unique(path_dirs, format!("{trimmed}/bin"));
        }
        IdeRunnerKind::Shell | IdeRunnerKind::Compound | IdeRunnerKind::Custom => {
            if trimmed.starts_with('/') {
                push_unique(path_dirs, format!("{trimmed}/bin"));
            }
        }
    }
}

fn validate_attached_folder_path(target: &IdeTargetSpec, raw_folder: &str) -> Result<String> {
    let trimmed = raw_folder.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(Error::Invalid("folder path must not be empty".into()));
    }
    if !trimmed.starts_with('/') {
        return Err(Error::Invalid(
            "attached project folder must be an absolute path".into(),
        ));
    }
    if target.kind == IdeTargetKind::Local {
        let canon = std::fs::canonicalize(trimmed).map_err(|e| {
            Error::Invalid(format!(
                "attached project folder '{trimmed}' not found: {e}"
            ))
        })?;
        if !canon.is_dir() {
            return Err(Error::Invalid(format!(
                "attached project folder '{}' is not a directory",
                canon.display()
            )));
        }
        let canon_str = canon.to_string_lossy().into_owned();
        if is_forbidden_sensitive_dir(&canon_str) {
            return Err(Error::Forbidden(
                "refusing to attach sensitive credential directory".into(),
            ));
        }
        Ok(canon_str)
    } else {
        Ok(trimmed.trim_end_matches('/').to_string())
    }
}

fn is_forbidden_sensitive_dir(path: &str) -> bool {
    if path == "/" || path == "/etc" || path == "/private/etc" {
        return true;
    }
    if let Some(home) = dirs::home_dir() {
        let hs = home.to_string_lossy();
        for rel in [".ssh", ".aws", ".gnupg", ".kube", ".config/gcloud"] {
            let blocked = format!("{}/{rel}", hs.trim_end_matches('/'));
            if path == blocked || path.starts_with(&format!("{blocked}/")) {
                return true;
            }
        }
    }
    false
}

async fn persist_project_roots(
    pool: &DbPool,
    target: &IdeTargetSpec,
    roots: &[String],
) -> Result<()> {
    let roots_json = serde_json::to_string(roots)
        .map_err(|e| Error::Internal(format!("serialize additional_roots_json: {e}")))?;
    let now = Utc::now().to_rfc3339();
    let target_kind = match target.kind {
        IdeTargetKind::Local => "local",
        IdeTargetKind::Ssh => "ssh",
    };
    sqlx::query(
        r#"INSERT INTO ide_project_environments (
               project_key, target_kind, root_path, connection_id,
               additional_roots_json, updated_at
           ) VALUES (?, ?, ?, ?, ?, ?)
           ON CONFLICT(project_key) DO UPDATE SET
               additional_roots_json = excluded.additional_roots_json,
               updated_at = excluded.updated_at"#,
    )
    .bind(target.project_key())
    .bind(target_kind)
    .bind(&target.root)
    .bind(&target.connection_id)
    .bind(&roots_json)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| Error::Internal(format!("persist additional_roots_json: {e}")))?;
    Ok(())
}

fn safe_join_allowed_roots(allowed_roots: &[String], rel_or_abs: &str) -> Option<PathBuf> {
    if Path::new(rel_or_abs).is_absolute() {
        let cc = std::fs::canonicalize(rel_or_abs).ok()?;
        for root in allowed_roots {
            if let Ok(cr) = std::fs::canonicalize(root) {
                if cc.starts_with(&cr) {
                    return Some(cc);
                }
            }
        }
        return None;
    }
    if rel_or_abs.contains("..") {
        return None;
    }
    for root in allowed_roots {
        let cand = Path::new(root).join(rel_or_abs);
        if let (Ok(cr), Ok(cc)) = (std::fs::canonicalize(root), std::fs::canonicalize(&cand)) {
            if cc.starts_with(&cr) {
                return Some(cc);
            }
        }
    }
    None
}

async fn load_stored_project_env(
    pool: &DbPool,
    project_key: &str,
) -> Result<(IdeProjectEnvironment, Vec<StoredEnvVar>)> {
    let row = sqlx::query(
        r#"SELECT java_home, go_root, rust_toolchain, node_home, python_interpreter,
                  dotenv_path, extra_path, env_vars_json, additional_roots_json, inherit_shell_env
           FROM ide_project_environments WHERE project_key = ?"#,
    )
    .bind(project_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| Error::Internal(format!("query ide_project_environments: {e}")))?;

    let Some(row) = row else {
        return Ok((IdeProjectEnvironment::default(), Vec::new()));
    };
    let raw_json: String = row.try_get("env_vars_json").unwrap_or_else(|_| "[]".into());
    let stored: Vec<StoredEnvVar> = serde_json::from_str(&raw_json).unwrap_or_default();
    let roots_json: String = row
        .try_get("additional_roots_json")
        .unwrap_or_else(|_| "[]".into());
    let additional_roots: Vec<String> = serde_json::from_str(&roots_json).unwrap_or_default();
    let inherit_shell_env = row.try_get::<i64, _>("inherit_shell_env").unwrap_or(1) != 0;

    let env = IdeProjectEnvironment {
        java_home: row.try_get("java_home").ok().flatten(),
        go_root: row.try_get("go_root").ok().flatten(),
        rust_toolchain: row.try_get("rust_toolchain").ok().flatten(),
        node_home: row.try_get("node_home").ok().flatten(),
        python_interpreter: row.try_get("python_interpreter").ok().flatten(),
        dotenv_path: row.try_get("dotenv_path").ok().flatten(),
        extra_path: row.try_get("extra_path").ok().flatten(),
        inherit_shell_env,
        additional_roots,
        env_vars: Vec::new(),
    };
    record_sync_project_snapshot(project_key, &env);
    Ok((env, stored))
}

fn row_to_run_config(row: &sqlx::sqlite::SqliteRow) -> IdeRunConfig {
    let runner_s: String = row.try_get("runner").unwrap_or_else(|_| "custom".into());
    let runner: IdeRunnerKind = serde_json::from_value(serde_json::Value::String(runner_s))
        .unwrap_or(IdeRunnerKind::Custom);
    let args_s: String = row.try_get("args_json").unwrap_or_else(|_| "[]".into());
    let flags_s: String = row
        .try_get("build_flags_json")
        .unwrap_or_else(|_| "[]".into());
    let env_s: String = row
        .try_get("env_overrides_json")
        .unwrap_or_else(|_| "{}".into());
    let compound_s: String = row
        .try_get("compound_ids_json")
        .unwrap_or_else(|_| "[]".into());
    let inherit_shell_env = row.try_get::<i64, _>("inherit_shell_env").unwrap_or(1) != 0;

    IdeRunConfig {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        runner,
        command: row.try_get("command").ok().flatten(),
        args: serde_json::from_str(&args_s).unwrap_or_default(),
        build_flags: serde_json::from_str(&flags_s).unwrap_or_default(),
        cwd: row.try_get("cwd").ok().flatten(),
        env_overrides: serde_json::from_str(&env_s).unwrap_or_default(),
        env_file: row.try_get("env_file").ok().flatten(),
        inherit_shell_env,
        sdk_path: row.try_get("sdk_path").ok().flatten(),
        toolchain_id: row.try_get("toolchain_id").ok().flatten(),
        before_launch: row.try_get("before_launch").ok().flatten(),
        target_path: row.try_get("target_path").ok().flatten(),
        test_filter: row.try_get("test_filter").ok().flatten(),
        compound_configs: serde_json::from_str(&compound_s).unwrap_or_default(),
        auto_detected: false,
    }
}

fn discover_local_dotenv_files(root: &Path) -> Vec<String> {
    const CANDIDATES: &[&str] = &[
        ".env",
        ".env.local",
        ".env.development",
        ".env.test",
        ".env.staging",
        ".env.production",
    ];
    CANDIDATES
        .iter()
        .filter(|c| root.join(c).is_file())
        .map(|c| (*c).to_string())
        .collect()
}

fn discover_local_python_venvs(root: &Path, data_dir: &Path) -> Vec<IdePythonVenvInfo> {
    let mut out = Vec::new();
    for dir_name in [".venv", "venv", "env"] {
        let p = root.join(dir_name);
        if p.join("bin/python").is_file() {
            let kind = if p.join("pyvenv.cfg").is_file()
                && std::fs::read_to_string(p.join("pyvenv.cfg"))
                    .map(|c| c.contains("uv ="))
                    .unwrap_or(false)
            {
                "uv"
            } else {
                "venv"
            };
            out.push(IdePythonVenvInfo {
                version: read_pyvenv_version(&p).unwrap_or_else(|| "3".into()),
                path: p.to_string_lossy().into_owned(),
                kind: kind.into(),
            });
        }
    }
    for py_dir in read_subdirs(&data_dir.join("toolchains/python/interpreters"))
        .into_iter()
        .take(16)
    {
        if py_dir.join("bin/python3").is_file() {
            out.push(IdePythonVenvInfo {
                version: py_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("3")
                    .to_string(),
                path: py_dir.to_string_lossy().into_owned(),
                kind: "uv".into(),
            });
        }
    }
    if !otto_core::e2e::isolated() {
        if let Some(home) = dirs::home_dir() {
            for (kind, parent) in [
                ("conda", home.join(".conda/envs")),
                ("conda", home.join("miniconda3/envs")),
                ("poetry", home.join("Library/Caches/pypoetry/virtualenvs")),
                ("poetry", home.join(".cache/pypoetry/virtualenvs")),
            ] {
                for d in read_subdirs(&parent).into_iter().take(12) {
                    if d.join("bin/python").is_file() {
                        out.push(IdePythonVenvInfo {
                            version: read_pyvenv_version(&d).unwrap_or_else(|| "3".into()),
                            path: d.to_string_lossy().into_owned(),
                            kind: kind.into(),
                        });
                    }
                }
            }
        }
    }
    out
}

fn read_pyvenv_version(venv: &Path) -> Option<String> {
    let cfg = std::fs::read_to_string(venv.join("pyvenv.cfg")).ok()?;
    cfg.lines()
        .find_map(|l| {
            let (k, v) = l.split_once('=')?;
            matches!(k.trim(), "version" | "version_info").then(|| v.trim().to_string())
        })
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemSecrets(Mutex<HashMap<String, String>>);

    impl SecretStore for MemSecrets {
        fn put(&self, key: &str, value: &str) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }

        fn delete(&self, key: &str) -> Result<()> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[tokio::test]
    async fn sqlite_crud_redacts_secrets_preserves_multiroot_and_layers_intellij_env_order() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = otto_state::db::open(&tmp.path().join("test.db"))
            .await
            .unwrap();
        let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
        std::fs::write(
            tmp.path().join(".env"),
            "FROM_DOTENV=\"hello\\nworld\"\nAPP_ENV=dev\nCFG_WIN=from_dotenv\n",
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".env.qa"),
            "CFG_WIN=from_cfg_env_file\nQA_ONLY=1\n",
        )
        .unwrap();

        let sibling = tmp.path().join("go_casino_kit");
        std::fs::create_dir_all(&sibling).unwrap();

        let mgr = EnvironmentManager::new(tmp.path().join("data"));
        let target = IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned());

        // Attach sibling folder and ensure subsequent PUT omitting additional_roots preserves it!
        let after_attach = mgr
            .add_project_folder(&pool, &target, &sibling.to_string_lossy())
            .await
            .unwrap();
        assert_eq!(after_attach.additional_roots.len(), 1);

        let saved = mgr
            .update_environment(
                &pool,
                &secrets,
                IdeUpdateEnvironmentReq {
                    target: target.clone(),
                    java_home: Some("/opt/jdk-21".into()),
                    go_root: Some("/opt/go-1.24".into()),
                    rust_toolchain: Some("stable".into()),
                    node_home: None,
                    python_interpreter: None,
                    dotenv_path: Some(".env".into()),
                    extra_path: Some("/custom/bin".into()),
                    inherit_shell_env: None,
                    additional_roots: None,
                    env_vars: vec![
                        IdeEnvVarEntry {
                            key: "APP_ENV".into(),
                            value: Some("staging".into()),
                            secret: false,
                            has_secret: false,
                        },
                        IdeEnvVarEntry {
                            key: "STRIPE_API_SECRET".into(),
                            value: Some("sk_test_super_secret".into()),
                            secret: true,
                            has_secret: false,
                        },
                    ],
                },
            )
            .await
            .unwrap();

        assert_eq!(saved.additional_roots.len(), 1);
        assert!(saved.inherit_shell_env);
        let sec_entry = saved
            .env_vars
            .iter()
            .find(|e| e.key == "STRIPE_API_SECRET")
            .unwrap();
        assert!(sec_entry.has_secret);
        assert_eq!(sec_entry.value, None);

        let raw: String = sqlx::query_scalar("SELECT env_vars_json FROM ide_project_environments")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!raw.contains("sk_test_super_secret"));

        // Verify strict IntelliJ precedence with Run Config env_file + env_overrides + sdk_path:
        let mut overrides = BTreeMap::new();
        overrides.insert("APP_ENV".into(), "override_wins".into());
        let rc = mgr
            .upsert_run_config(
                &pool,
                IdeUpsertRunConfigReq {
                    target: target.clone(),
                    config: IdeRunConfig {
                        id: "rc_1".into(),
                        name: "Go Suite".into(),
                        runner: IdeRunnerKind::GoTest,
                        args: vec!["./calc/...".into()],
                        build_flags: vec!["-race".into()],
                        env_overrides: overrides,
                        env_file: Some(".env.qa".into()),
                        sdk_path: Some("/opt/go-1.25".into()),
                        ..Default::default()
                    },
                },
            )
            .await
            .unwrap();

        let eff = mgr
            .build_effective_env_for_run(&pool, secrets.as_ref(), &target, Some(&rc))
            .await
            .unwrap();
        assert_eq!(eff.get("FROM_DOTENV").unwrap(), "hello\nworld");
        assert_eq!(eff.get("QA_ONLY").unwrap(), "1");
        assert_eq!(eff.get("CFG_WIN").unwrap(), "from_cfg_env_file");
        assert_eq!(eff.get("APP_ENV").unwrap(), "override_wins");
        assert_eq!(
            eff.get("STRIPE_API_SECRET").unwrap(),
            "sk_test_super_secret"
        );
        assert_eq!(eff.get("GOROOT").unwrap(), "/opt/go-1.25");
        assert_eq!(eff.get("JAVA_HOME").unwrap(), "/opt/jdk-21");
        assert!(eff
            .get("PATH")
            .unwrap()
            .starts_with("/custom/bin:/opt/go-1.25/bin:"));

        let loaded_rc = mgr.get_run_config(&pool, &rc.id).await.unwrap();
        assert_eq!(loaded_rc.build_flags, vec!["-race"]);
        assert_eq!(loaded_rc.env_file.as_deref(), Some(".env.qa"));
        assert_eq!(loaded_rc.sdk_path.as_deref(), Some("/opt/go-1.25"));

        mgr.delete_run_config(&pool, &rc.id).await.unwrap();
        assert!(mgr
            .list_run_configs(&pool, &target)
            .await
            .unwrap()
            .is_empty());

        let after_detach = mgr
            .remove_project_folder(&pool, &target, &sibling.to_string_lossy())
            .await
            .unwrap();
        assert!(after_detach.additional_roots.is_empty());
    }

    #[test]
    fn finished_runs_evict_cleanly_past_max_finished_runs_preserving_running() {
        use super::runner::MAX_FINISHED_RUNS;
        use crate::types::IdeRunStatus;

        let mgr = EnvironmentManager::new(PathBuf::from("/tmp/otto-ide-test"));
        let total_finished = (MAX_FINISHED_RUNS + 8) as u64;
        for i in 0..total_finished {
            let (stop_tx, _) = watch::channel(false);
            let id = format!("run_fin_{i:02}");
            let handle = Arc::new(ActiveRunHandle {
                seq: i,
                state: Mutex::new(RunStateInner::new(
                    id.clone(),
                    "Finished".into(),
                    IdeRunnerKind::Custom,
                    format!("2026-01-01T00:00:{i:02}Z"),
                )),
                stop_tx,
            });
            runner::finish_run(
                &handle,
                IdeRunStatus::Passed,
                Some(0),
                Duration::from_millis(5),
                None,
            );
            mgr.runs.insert(id, handle);
            evict_finished_runs(&mgr.runs);
        }

        let (stop_tx, _) = watch::channel(false);
        mgr.runs.insert(
            "run_active".into(),
            Arc::new(ActiveRunHandle {
                seq: 100,
                state: Mutex::new(RunStateInner::new(
                    "run_active".into(),
                    "Active".into(),
                    IdeRunnerKind::Custom,
                    "2026-01-01T00:00:00Z".into(),
                )),
                stop_tx,
            }),
        );
        evict_finished_runs(&mgr.runs);

        assert_eq!(mgr.runs.len(), MAX_FINISHED_RUNS + 1);
        assert!(mgr.get_run("run_active").is_ok());
        for i in 0..8 {
            assert!(mgr.get_run(&format!("run_fin_{i:02}")).is_err());
        }
        for i in 8..total_finished {
            assert!(mgr.get_run(&format!("run_fin_{i:02}")).is_ok());
        }
    }
}
