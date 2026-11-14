//! Project Environments, Cached User Login+Interactive Shell Environment,
//! Keychain-backed Secret Env Vars, Multi-Folder Project Roots, `.env` &
//! Python Virtualenv Manager, Run Configurations CRUD + Instant Auto-Discovery,
//! and Multi-Language Test/Run Execution Engine (`EnvironmentManager`).

mod db_store;
pub mod discovery;
mod discovery_jvm_py;
mod env_layers;
pub(crate) mod module_sdks;
mod run_configs;
mod runner;
mod runner_cmd;
mod runner_parsers;
pub mod shell_env;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock, Weak};

use chrono::Utc;
use dashmap::DashMap;
use otto_core::auth::BoxFuture;
use otto_core::domain::User;
use otto_core::secrets::{delete_async, get_async, put_async, SecretStore};
use otto_core::{Error, Result};
use otto_state::DbPool;

use crate::projects::ProjectStore;
use crate::target::TargetManager;
use crate::toolchains::system_search_dirs;
use crate::types::{
    IdeEnvVarEntry, IdeEnvironmentsResp, IdeProjectEnvironment, IdeRunConfig, IdeShellEnvResp,
    IdeSshSpec, IdeTargetKind, IdeTargetSpec, IdeUpdateEnvironmentReq,
};

pub use self::db_store::parse_dotenv;
use self::db_store::{
    clean_opt, discover_local_dotenv_files, discover_local_python_venvs, is_valid_env_key,
    load_all_stored_project_envs, load_stored_project_env, persist_project_roots, push_unique,
    secret_key_ref, validate_attached_folder_path, StoredEnvVar, RESERVED_MODULE_SDKS_KEY,
};
pub use self::discovery::discover_run_configs;
use self::module_sdks::{
    apply_module_sdk_to_project_env, layer_child_and_module_sdk_for_run, match_longest_module_sdk,
    sanitize_module_sdks,
};
use self::runner::ActiveRunHandle;
pub use self::runner_parsers::parse_test_output;
pub use self::shell_env::{
    global_shell_env, parse_sentinel_env_zero, probe_remote_shell_env, ShellEnvResolver,
};

/// Kick off background warming of the user's login+interactive shell environment.
pub fn ensure_shell_env_warming() {
    global_shell_env().ensure_warming_in_background();
}

/// Non-blocking snapshot of the cached login+interactive shell environment.
pub fn cached_shell_env_snapshot() -> BTreeMap<String, String> {
    global_shell_env().vars_snapshot()
}

const MAX_SYNC_PROJECT_SNAPSHOTS: usize = 128;

#[derive(Clone, Default)]
struct SyncProjectSnapshot {
    key: String,
    env: IdeProjectEnvironment,
}

type WeakSnapshotRegistry = RwLock<HashMap<PathBuf, Weak<RwLock<Vec<SyncProjectSnapshot>>>>>;

fn snapshot_registry() -> &'static WeakSnapshotRegistry {
    static REGISTRY: OnceLock<WeakSnapshotRegistry> = OnceLock::new();
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

#[allow(clippy::disallowed_methods)]
fn register_instance_snapshots(data_dir: &Path, snapshots: &Arc<RwLock<Vec<SyncProjectSnapshot>>>) {
    let mut guard = snapshot_registry()
        .write()
        .unwrap_or_else(|e| e.into_inner());
    guard.retain(|_, weak| weak.strong_count() > 0);
    guard.insert(data_dir.to_path_buf(), Arc::downgrade(snapshots));
    if let Ok(canon) = std::fs::canonicalize(data_dir) {
        guard.insert(canon, Arc::downgrade(snapshots));
    }
}

#[allow(clippy::disallowed_methods)]
fn find_instance_snapshots(data_dir: &Path) -> Option<Arc<RwLock<Vec<SyncProjectSnapshot>>>> {
    let guard = snapshot_registry()
        .read()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(arc) = guard.get(data_dir).and_then(Weak::upgrade) {
        return Some(arc);
    }
    let canon = std::fs::canonicalize(data_dir).ok()?;
    guard.get(&canon).and_then(Weak::upgrade)
}

fn upsert_snapshot_entry(
    guard: &mut Vec<SyncProjectSnapshot>,
    key: String,
    env: IdeProjectEnvironment,
) {
    if let Some(pos) = guard.iter().position(|s| s.key == key) {
        guard.remove(pos);
    }
    guard.push(SyncProjectSnapshot { key, env });
    if guard.len() > MAX_SYNC_PROJECT_SNAPSHOTS {
        let excess = guard.len() - MAX_SYNC_PROJECT_SNAPSHOTS;
        guard.drain(0..excess);
    }
}

fn record_snapshot_into(
    snapshots: &RwLock<Vec<SyncProjectSnapshot>>,
    project_key: &str,
    env: &IdeProjectEnvironment,
    stored_vars: &[StoredEnvVar],
) {
    let non_secret_vars: Vec<IdeEnvVarEntry> = stored_vars
        .iter()
        .filter(|s| !s.secret)
        .map(|s| IdeEnvVarEntry {
            key: s.key.clone(),
            value: s.value.clone(),
            secret: false,
            has_secret: false,
        })
        .collect();
    let base_env = IdeProjectEnvironment {
        env_vars: non_secret_vars,
        ..env.clone()
    };
    let mut guard = snapshots.write().unwrap_or_else(|e| e.into_inner());
    upsert_snapshot_entry(&mut guard, project_key.to_string(), base_env.clone());
    if project_key.starts_with("local:") {
        for (mod_root, sdk_val) in &base_env.module_sdks {
            let mut mod_env = base_env.clone();
            apply_module_sdk_to_project_env(&mut mod_env, mod_root, sdk_val, None, None);
            let mod_key = format!("local:{}", mod_root.trim_end_matches('/'));
            upsert_snapshot_entry(&mut guard, mod_key, mod_env);
        }
    }
}

fn lookup_snapshot_in(
    snapshots: &RwLock<Vec<SyncProjectSnapshot>>,
    project_key_or_root: &str,
) -> Option<IdeProjectEnvironment> {
    let trimmed = project_key_or_root.trim().trim_end_matches('/');
    let local_key = format!("local:{trimmed}");
    let guard = snapshots.read().unwrap_or_else(|e| e.into_inner());
    if let Some(exact) = guard.iter().rev().find(|s| {
        let sk = s.key.trim_end_matches('/');
        sk == trimmed || sk == local_key
    }) {
        return Some(exact.env.clone());
    }
    if !trimmed.starts_with("local:") && trimmed.starts_with('/') {
        let cands = vec![trimmed.to_string()];
        for snap in guard.iter().rev() {
            if let Some((mod_root, sdk_val)) =
                match_longest_module_sdk(&snap.env.module_sdks, &cands, true)
            {
                let mut mod_env = snap.env.clone();
                apply_module_sdk_to_project_env(&mut mod_env, &mod_root, &sdk_val, None, None);
                return Some(mod_env);
            }
        }
    }
    None
}

/// Synchronous effective environment resolver for `data_dir` + `root` using the
/// owning [`EnvironmentManager`] instance's hydrated project environment cache.
pub fn cached_effective_env_for_local_root(
    data_dir: &Path,
    root: &str,
) -> BTreeMap<String, String> {
    let proj_opt =
        find_instance_snapshots(data_dir).and_then(|snaps| lookup_snapshot_in(&snaps, root));
    build_cached_local_env(data_dir, root, proj_opt.as_ref(), global_shell_env())
}

#[allow(clippy::disallowed_methods)]
fn build_cached_local_env(
    data_dir: &Path,
    root: &str,
    proj_opt: Option<&IdeProjectEnvironment>,
    shell_env: &Arc<ShellEnvResolver>,
) -> BTreeMap<String, String> {
    let inherit_shell = proj_opt.map(|p| p.inherit_shell_env).unwrap_or(true);

    let mut map = shell_env::collect_process_base_env();
    if inherit_shell {
        map.extend(shell_env.vars_snapshot());
    }

    let mut path_dirs: Vec<String> = Vec::new();
    let mut has_explicit_py = false;
    if let Some(proj) = proj_opt {
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
        if let Some(rt) = proj.rust_toolchain.as_deref().filter(|s| !s.is_empty()) {
            if rt.starts_with('/') {
                push_unique(&mut path_dirs, format!("{}/bin", rt.trim_end_matches('/')));
            } else {
                map.insert("RUSTUP_TOOLCHAIN".into(), rt.to_string());
            }
        }
        if let Some(py) = proj.python_interpreter.as_deref().filter(|s| !s.is_empty()) {
            has_explicit_py = true;
            if let Some(bin_dir) = Path::new(py).parent() {
                push_unique(&mut path_dirs, bin_dir.to_string_lossy().into_owned());
                if let Some(venv) = bin_dir.parent() {
                    if venv.join("pyvenv.cfg").is_file() {
                        map.insert("VIRTUAL_ENV".into(), venv.to_string_lossy().into_owned());
                    }
                }
            }
        }
        if let Some(extra) = proj.extra_path.as_deref().filter(|s| !s.is_empty()) {
            for seg in extra.split(':').filter(|s| !s.trim().is_empty()) {
                push_unique(&mut path_dirs, seg.trim().to_string());
            }
        }
    }
    if !has_explicit_py {
        for vname in [".venv", "venv", ".env"] {
            let cand = Path::new(root).join(vname);
            if cand.join("pyvenv.cfg").is_file() && cand.join("bin/python").is_file() {
                push_unique(
                    &mut path_dirs,
                    cand.join("bin").to_string_lossy().into_owned(),
                );
                map.insert("VIRTUAL_ENV".into(), cand.to_string_lossy().into_owned());
                break;
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

    if let Some(proj) = proj_opt {
        for item in &proj.env_vars {
            if !item.secret {
                if let Some(val) = &item.value {
                    if is_valid_env_key(&item.key) {
                        map.insert(item.key.clone(), val.clone());
                    }
                }
            }
        }
    }
    map
}

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
    #[allow(clippy::disallowed_methods)]
    fn get_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>>> {
        let res = match self.get_cached(key) {
            Some(cached) => Ok(cached),
            None => self.get(key),
        };
        Box::pin(async move { res })
    }

    #[allow(clippy::disallowed_methods)]
    fn put_secret<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<()>> {
        let res = self.put(key, value);
        Box::pin(async move { res })
    }

    #[allow(clippy::disallowed_methods)]
    fn delete_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        let res = self.delete(key);
        Box::pin(async move { res })
    }
}

#[derive(Clone)]
pub struct EnvironmentManager {
    data_dir: PathBuf,
    runs: Arc<DashMap<String, Arc<ActiveRunHandle>>>,
    run_seq: Arc<AtomicU64>,
    shell_env: Arc<ShellEnvResolver>,
    snapshots: Arc<RwLock<Vec<SyncProjectSnapshot>>>,
    snapshots_hydrated: Arc<AtomicBool>,
}

impl EnvironmentManager {
    pub fn new(data_dir: PathBuf) -> Self {
        let snapshots = Arc::new(RwLock::new(Vec::new()));
        register_instance_snapshots(&data_dir, &snapshots);
        Self {
            data_dir,
            runs: Arc::new(DashMap::new()),
            run_seq: Arc::new(AtomicU64::new(0)),
            shell_env: Arc::clone(global_shell_env()),
            snapshots,
            snapshots_hydrated: Arc::new(AtomicBool::new(false)),
        }
    }

    #[cfg(test)]
    pub(super) fn with_shell_env(data_dir: PathBuf, shell_env: Arc<ShellEnvResolver>) -> Self {
        let snapshots = Arc::new(RwLock::new(Vec::new()));
        register_instance_snapshots(&data_dir, &snapshots);
        Self {
            data_dir,
            runs: Arc::new(DashMap::new()),
            run_seq: Arc::new(AtomicU64::new(0)),
            shell_env,
            snapshots,
            snapshots_hydrated: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Hydrate the instance project environment snapshot cache from SQLite on
    /// startup or first `/ws/lsp` reconnect so saved `go_root`, `java_home`,
    /// `python_interpreter`, `dotenv_path`, `env_vars`, and `inherit_shell_env`
    /// take effect before `/api/v1/ide/environments` is visited.
    pub async fn ensure_hydrated(&self, pool: &DbPool) {
        if self.snapshots_hydrated.load(Ordering::Acquire) {
            return;
        }
        match load_all_stored_project_envs(pool).await {
            Ok(all) => {
                for (key, env, stored) in &all {
                    record_snapshot_into(&self.snapshots, key, env, stored);
                }
                self.snapshots_hydrated.store(true, Ordering::Release);
            }
            Err(err) => {
                tracing::warn!(error = %err, "failed to hydrate ide_project_environments cache");
            }
        }
    }

    pub async fn reload_from_db(&self, pool: &DbPool) {
        self.snapshots_hydrated.store(false, Ordering::Release);
        self.snapshots
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.ensure_hydrated(pool).await;
    }

    pub(super) async fn refresh_project_snapshot(&self, pool: &DbPool, project_key: &str) {
        if let Ok((env, stored)) = load_stored_project_env(pool, project_key).await {
            record_snapshot_into(&self.snapshots, project_key, &env, &stored);
        }
    }

    pub fn cached_additional_roots(&self, project_key_or_root: &str) -> Vec<String> {
        lookup_snapshot_in(&self.snapshots, project_key_or_root)
            .map(|e| e.additional_roots)
            .unwrap_or_default()
    }

    pub fn cached_effective_env_for_local_root(&self, root: &str) -> BTreeMap<String, String> {
        let proj_opt = lookup_snapshot_in(&self.snapshots, root);
        build_cached_local_env(&self.data_dir, root, proj_opt.as_ref(), &self.shell_env)
    }

    pub fn cached_shell_env_snapshot(&self) -> BTreeMap<String, String> {
        self.shell_env.vars_snapshot()
    }

    pub fn shell_env_snapshot(&self) -> IdeShellEnvResp {
        self.shell_env.snapshot()
    }

    pub async fn get_shell_env(&self) -> IdeShellEnvResp {
        self.shell_env.get_or_load().await
    }

    pub async fn reload_shell_env(&self) -> IdeShellEnvResp {
        self.shell_env.reload().await
    }

    pub async fn get_project_env(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
    ) -> Result<IdeProjectEnvironment> {
        target.validate()?;
        let project_key = target.project_key();
        let (mut env, stored) = load_stored_project_env(pool, &project_key).await?;
        record_snapshot_into(&self.snapshots, &project_key, &env, &stored);
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

    /// Hydrate `target.additional_roots` with folders from `project_folders`,
    /// SQLite `ide_project_environments`, and the caller's Named IDE Projects.
    pub async fn enrich_target_roots(
        &self,
        pool: &DbPool,
        projects: Option<&ProjectStore>,
        user: Option<&User>,
        target: &mut IdeTargetSpec,
    ) {
        target.absorb_project_folders(&[]);
        let project_key = target.project_key();
        if let Ok((proj, stored)) = load_stored_project_env(pool, &project_key).await {
            record_snapshot_into(&self.snapshots, &project_key, &proj, &stored);
            for r in proj.additional_roots {
                if !r.is_empty() && r != target.root && !target.additional_roots.contains(&r) {
                    target.additional_roots.push(r);
                }
            }
        }
        if target.kind == IdeTargetKind::Local {
            if let Some(store) = projects {
                let _ = store.ensure_hydrated(pool).await;
                let extra = match user {
                    Some(u) => store.folders_for_root_for_user(&target.root, u),
                    None => store.folders_for_root(&target.root),
                };
                for r in extra {
                    if !r.is_empty() && r != target.root && !target.additional_roots.contains(&r) {
                        target.additional_roots.push(r);
                    }
                }
            }
        }
    }

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

    /// Bulk-attach multiple folders in a single SQLite write (`POST /ide/projects/folders/bulk`).
    pub async fn add_project_folders_bulk(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        folders: &[String],
    ) -> Result<(IdeProjectEnvironment, usize)> {
        target.validate()?;
        let project_key = target.project_key();
        let (mut proj, _) = load_stored_project_env(pool, &project_key).await?;
        let mut added = 0usize;
        for raw in folders {
            if raw.trim().is_empty() {
                continue;
            }
            let normalized = validate_attached_folder_path(target, raw)?;
            if normalized != target.root && !proj.additional_roots.contains(&normalized) {
                proj.additional_roots.push(normalized);
                added += 1;
            }
        }
        persist_project_roots(pool, target, &proj.additional_roots).await?;
        let env = self.get_project_env(pool, target).await?;
        Ok((env, added))
    }

    #[allow(clippy::disallowed_methods)]
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
        secrets: &S,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
    ) -> Result<IdeEnvironmentsResp> {
        self.get_environments_for_user(pool, secrets, targets, target, ssh, true)
            .await
    }

    pub async fn get_environments_for_user<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        _secrets: &S,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
        caller_is_root: bool,
    ) -> Result<IdeEnvironmentsResp> {
        target.validate()?;
        let project_key = target.project_key();
        let (mut env, stored_vars) = load_stored_project_env(pool, &project_key).await?;
        record_snapshot_into(&self.snapshots, &project_key, &env, &stored_vars);
        for extra in target.merged_extra_roots() {
            if !extra.is_empty() && extra != target.root && !env.additional_roots.contains(&extra) {
                env.additional_roots.push(extra);
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
                    let mut venvs = discover_local_python_venvs(&root, &data_dir);
                    for er in &extra_roots {
                        for mv in discover_local_python_venvs(Path::new(er), &data_dir) {
                            if !venvs.iter().any(|v| v.path == mv.path) {
                                venvs.push(mv);
                            }
                        }
                    }
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

        let effective_env = self
            .resolve_env_layers(
                &env,
                &stored_vars,
                None::<&S>,
                target,
                None,
                false,
                caller_is_root,
            )
            .await?;

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
            if key.is_empty() || key == RESERVED_MODULE_SDKS_KEY || !is_valid_env_key(&key) {
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

        let next_module_sdks = match &req.module_sdks {
            Some(explicit_map) => sanitize_module_sdks(explicit_map),
            None => prev_proj.module_sdks,
        };
        if !next_module_sdks.is_empty() {
            let encoded = serde_json::to_string(&next_module_sdks)
                .map_err(|e| Error::Internal(format!("serialize module_sdks: {e}")))?;
            next_stored.push(StoredEnvVar {
                key: RESERVED_MODULE_SDKS_KEY.into(),
                value: Some(encoded),
                secret: false,
                secret_ref: None,
            });
        }

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

    pub async fn build_effective_env_for_run<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
        cfg: Option<&IdeRunConfig>,
    ) -> Result<BTreeMap<String, String>> {
        self.build_effective_env_for_run_as(pool, secrets, target, cfg, true)
            .await
    }

    pub async fn build_effective_env_for_run_as<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
        cfg: Option<&IdeRunConfig>,
        caller_is_root: bool,
    ) -> Result<BTreeMap<String, String>> {
        target.validate()?;
        let project_key = target.project_key();
        let (mut proj, mut stored_vars) = load_stored_project_env(pool, &project_key).await?;
        record_snapshot_into(&self.snapshots, &project_key, &proj, &stored_vars);
        layer_child_and_module_sdk_for_run(pool, target, cfg, &mut proj, &mut stored_vars).await;
        self.resolve_env_layers(
            &proj,
            &stored_vars,
            Some(secrets),
            target,
            cfg,
            true,
            caller_is_root,
        )
        .await
    }
}
