//! Project Environments, Cached User Login+Interactive Shell Environment,
//! Keychain-backed Secret Env Vars, Multi-Folder Project Roots, `.env` &
//! Python Virtualenv Manager, Run Configurations CRUD + Instant Auto-Discovery,
//! and Multi-Language Test/Run Execution Engine (`EnvironmentManager`).

mod crud;
mod db_store;
pub mod discovery;
mod discovery_jvm_py;
mod env_layers;
pub(crate) mod module_sdks;
mod python_venv;
mod run_configs;
mod runner;
mod runner_cmd;
mod runner_parsers;
mod runner_proc;
pub mod shell_env;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_runs;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use dashmap::DashMap;
use otto_core::auth::BoxFuture;
use otto_core::secrets::{delete_async, get_async, put_async, SecretStore};
use otto_core::Result;
use otto_state::DbPool;

use crate::toolchains::system_search_dirs;
use crate::types::{IdeEnvVarEntry, IdeProjectEnvironment, IdeShellEnvResp};

pub use self::db_store::parse_dotenv;
use self::db_store::{
    is_valid_env_key, load_all_stored_project_envs, load_stored_project_env, push_unique,
    safe_join_allowed_roots, StoredEnvVar,
};
pub use self::discovery::discover_run_configs;
use self::module_sdks::{
    apply_module_sdk_to_project_env, effective_go_build_tags_for_paths,
    match_longest_module_go_build_tags, match_longest_module_sdk,
};
use self::runner::ActiveRunHandle;
pub use self::runner_parsers::parse_test_output;
pub use self::shell_env::{parse_sentinel_env_zero, probe_remote_shell_env, ShellEnvResolver};

const MAX_SYNC_PROJECT_SNAPSHOTS: usize = 128;

#[derive(Clone, Default)]
struct SyncProjectSnapshot {
    key: String,
    env: IdeProjectEnvironment,
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
            mod_env.go_build_tags =
                effective_go_build_tags_for_paths(&base_env, std::slice::from_ref(mod_root), true);
            let mod_key = format!("local:{}", mod_root.trim_end_matches('/'));
            upsert_snapshot_entry(&mut guard, mod_key, mod_env);
        }
        for mod_root in base_env.module_go_build_tags.keys() {
            if base_env.module_sdks.contains_key(mod_root) {
                continue;
            }
            let mut mod_env = base_env.clone();
            mod_env.go_build_tags =
                effective_go_build_tags_for_paths(&base_env, std::slice::from_ref(mod_root), true);
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

#[allow(clippy::disallowed_methods)]
fn build_cached_local_env(
    data_dir: &Path,
    root: &str,
    proj_opt: Option<&IdeProjectEnvironment>,
    shell_env: &Arc<ShellEnvResolver>,
    caller_is_root: bool,
) -> BTreeMap<String, String> {
    let inherit_shell = proj_opt.map(|p| p.inherit_shell_env).unwrap_or(true);

    let mut map = if caller_is_root {
        let mut base = shell_env::collect_process_base_env();
        if inherit_shell {
            base.extend(shell_env.vars_snapshot());
        }
        base
    } else {
        shell_env::collect_safe_non_root_base_env()
    };

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
    for d in crate::toolchains::ToolchainManager::with_shell_env(
        data_dir.to_path_buf(),
        Arc::clone(shell_env),
    )
    .extra_local_bin_dirs()
    {
        push_unique(&mut path_dirs, d.to_string_lossy().into_owned());
    }
    if !otto_core::e2e::isolated() && caller_is_root {
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

    let mut allowed_roots = vec![root.to_string()];
    if let Some(proj) = proj_opt {
        for r in &proj.additional_roots {
            if !r.is_empty() && !allowed_roots.contains(r) {
                allowed_roots.push(r.clone());
            }
        }
    }
    let dotenv_rel = proj_opt
        .and_then(|p| p.dotenv_path.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(".env");
    if let Some(dotenv_file) = safe_join_allowed_roots(&allowed_roots, dotenv_rel) {
        if caller_is_root || !otto_core::secret_paths::is_denied_file(&dotenv_file) {
            if let Ok(raw) = std::fs::read_to_string(&dotenv_file) {
                map.extend(parse_dotenv(&raw));
            }
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
    generation: Arc<AtomicU64>,
}

impl EnvironmentManager {
    pub fn new(data_dir: PathBuf) -> Self {
        let shell_env = Arc::new(ShellEnvResolver::with_cache_dir(&data_dir));
        Self::with_shell_env(data_dir, shell_env)
    }

    pub fn with_shell_env(data_dir: PathBuf, shell_env: Arc<ShellEnvResolver>) -> Self {
        Self {
            data_dir,
            runs: Arc::new(DashMap::new()),
            run_seq: Arc::new(AtomicU64::new(0)),
            shell_env,
            snapshots: Arc::new(RwLock::new(Vec::new())),
            snapshots_hydrated: Arc::new(AtomicBool::new(false)),
            generation: Arc::new(AtomicU64::new(1)),
        }
    }

    pub fn shell_env_resolver(&self) -> &Arc<ShellEnvResolver> {
        &self.shell_env
    }

    pub fn ensure_shell_env_warming(&self) {
        self.shell_env.ensure_warming_in_background();
    }

    pub fn cache_generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    fn record_snapshot(
        &self,
        project_key: &str,
        env: &IdeProjectEnvironment,
        stored_vars: &[StoredEnvVar],
    ) {
        record_snapshot_into(&self.snapshots, project_key, env, stored_vars);
        self.generation.fetch_add(1, Ordering::AcqRel);
    }

    pub async fn ensure_hydrated(&self, pool: &DbPool) {
        if self.snapshots_hydrated.load(Ordering::Acquire) {
            return;
        }
        match load_all_stored_project_envs(pool).await {
            Ok(all) => {
                for (key, env, stored) in &all {
                    record_snapshot_into(&self.snapshots, key, env, stored);
                }
                self.generation.fetch_add(1, Ordering::AcqRel);
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
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.ensure_hydrated(pool).await;
    }

    pub(super) async fn refresh_project_snapshot(&self, pool: &DbPool, project_key: &str) {
        if let Ok((env, stored)) = load_stored_project_env(pool, project_key).await {
            self.record_snapshot(project_key, &env, &stored);
        }
    }

    pub fn cached_project_env(&self, project_key_or_root: &str) -> Option<IdeProjectEnvironment> {
        lookup_snapshot_in(&self.snapshots, project_key_or_root)
    }

    pub fn cached_additional_roots(&self, project_key_or_root: &str) -> Vec<String> {
        lookup_snapshot_in(&self.snapshots, project_key_or_root)
            .map(|e| e.additional_roots)
            .unwrap_or_default()
    }

    /// Resolve effective Go build tags for `root` across exact module snapshots,
    /// ancestor/umbrella project snapshots, `additional_roots`, and `module_go_build_tags`.
    pub fn effective_go_build_tags_for_local_root(&self, root: &str) -> Vec<String> {
        let trimmed = root.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return Vec::new();
        }
        let cands = vec![trimmed.to_string()];
        let local_key = format!("local:{trimmed}");
        let guard = self.snapshots.read().unwrap_or_else(|e| e.into_inner());
        let mut out = Vec::new();
        if let Some(exact) = guard.iter().rev().find(|s| {
            let sk = s.key.trim_end_matches('/');
            sk == trimmed || sk == local_key
        }) {
            for t in effective_go_build_tags_for_paths(&exact.env, &cands, true) {
                push_unique(&mut out, t);
            }
        }
        for snap in guard.iter().rev() {
            let Some(snap_root) = snap.key.strip_prefix("local:") else {
                continue;
            };
            let sr = snap_root.trim_end_matches('/');
            let under_primary =
                !sr.is_empty() && (trimmed == sr || trimmed.starts_with(&format!("{sr}/")));
            let under_extra = snap.env.additional_roots.iter().any(|ar| {
                let art = ar.trim().trim_end_matches('/');
                !art.is_empty() && (trimmed == art || trimmed.starts_with(&format!("{art}/")))
            });
            let matches_mod =
                match_longest_module_go_build_tags(&snap.env.module_go_build_tags, &cands, true)
                    .is_some();
            if under_primary || under_extra || matches_mod {
                for t in effective_go_build_tags_for_paths(&snap.env, &cands, true) {
                    push_unique(&mut out, t);
                }
            }
        }
        out
    }

    pub fn cached_effective_env_for_local_root(&self, root: &str) -> BTreeMap<String, String> {
        self.cached_effective_env_for_local_root_as(root, true)
    }

    pub fn cached_effective_env_for_local_root_as(
        &self,
        root: &str,
        caller_is_root: bool,
    ) -> BTreeMap<String, String> {
        let proj_opt = lookup_snapshot_in(&self.snapshots, root);
        build_cached_local_env(
            &self.data_dir,
            root,
            proj_opt.as_ref(),
            &self.shell_env,
            caller_is_root,
        )
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

    pub fn active_local_pids(&self) -> Vec<u32> {
        self.runs
            .iter()
            .filter_map(|entry| {
                let guard = entry.value().state.lock().ok()?;
                (guard.status == crate::types::IdeRunStatus::Running)
                    .then_some(guard.active_local_pid)
                    .flatten()
            })
            .collect()
    }
}
