//! Project Environments, Cached User Login+Interactive Shell Environment,
//! Keychain-backed Secret Env Vars, Multi-Folder Project Roots, `.env` &
//! Python Virtualenv Manager, Run Configurations CRUD + Instant Auto-Discovery,
//! and Multi-Language Test/Run Execution Engine (`EnvironmentManager`).

mod crud;
mod crud_folders;
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
mod runner_steps;
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

pub(crate) use self::crud::local_env_mutation_requires_root;
pub use self::db_store::parse_dotenv;
pub(crate) use self::db_store::{
    is_dangerous_loader_env_key, is_reserved_internal_env_key, is_valid_env_key, push_unique,
};
use self::db_store::{load_all_stored_project_envs, load_stored_project_env, StoredEnvVar};
pub use self::discovery::discover_run_configs;
pub(crate) use self::env_layers::{
    apply_project_sdk_layers, extend_from_dotenv_str, insert_filtered_env_var,
    resolve_confined_dotenv_path,
};
pub(crate) use self::module_sdks::effective_go_build_tags_for_paths_in_project;
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
    owner_key: Option<String>,
    env: IdeProjectEnvironment,
}

fn upsert_snapshot_entry(
    guard: &mut Vec<SyncProjectSnapshot>,
    key: String,
    owner_key: Option<String>,
    env: IdeProjectEnvironment,
) {
    if let Some(pos) = guard.iter().position(|s| s.key == key) {
        if owner_key.is_some() && guard[pos].owner_key.is_none() {
            // Do not overwrite an explicit primary project snapshot with a synthetic module entry.
            return;
        }
        guard.remove(pos);
    }
    guard.push(SyncProjectSnapshot {
        key,
        owner_key,
        env,
    });
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
    guard.retain(|s| s.owner_key.as_deref() != Some(project_key));
    upsert_snapshot_entry(&mut guard, project_key.to_string(), None, base_env.clone());
    if let Some(primary_root) = project_key.strip_prefix("local:") {
        for (mod_root, sdk_val) in &base_env.module_sdks {
            let mod_key = format!("local:{}", mod_root.trim_end_matches('/'));
            if mod_key == project_key {
                continue;
            }
            let mut mod_env = base_env.clone();
            apply_module_sdk_to_project_env(&mut mod_env, mod_root, sdk_val, None, None);
            mod_env.go_build_tags = effective_go_build_tags_for_paths_in_project(
                Some(primary_root),
                &base_env,
                std::slice::from_ref(mod_root),
                true,
            );
            upsert_snapshot_entry(&mut guard, mod_key, Some(project_key.to_string()), mod_env);
        }
        for mod_root in base_env.module_go_build_tags.keys() {
            if base_env.module_sdks.contains_key(mod_root) {
                continue;
            }
            let mod_key = format!("local:{}", mod_root.trim_end_matches('/'));
            if mod_key == project_key {
                continue;
            }
            let mut mod_env = base_env.clone();
            mod_env.go_build_tags = effective_go_build_tags_for_paths_in_project(
                Some(primary_root),
                &base_env,
                std::slice::from_ref(mod_root),
                true,
            );
            upsert_snapshot_entry(&mut guard, mod_key, Some(project_key.to_string()), mod_env);
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
    read_dotenv_and_workspace_venv: bool,
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
    if caller_is_root {
        if let Some(proj) = proj_opt {
            let (proj_dirs, explicit_py) =
                apply_project_sdk_layers(proj, crate::types::IdeTargetKind::Local, true, &mut map);
            has_explicit_py = explicit_py;
            for d in proj_dirs {
                push_unique(&mut path_dirs, d);
            }
            if read_dotenv_and_workspace_venv {
                if let Some(extra) = proj.extra_path.as_deref().filter(|s| !s.is_empty()) {
                    for seg in extra.split(':').filter(|s| !s.trim().is_empty()) {
                        push_unique(&mut path_dirs, seg.trim().to_string());
                    }
                }
            }
        }
        if read_dotenv_and_workspace_venv && !has_explicit_py {
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

    if !read_dotenv_and_workspace_venv {
        let keep_sdk_keys = [
            "PATH",
            "JAVA_HOME",
            "GOROOT",
            "GOPATH",
            "RUSTUP_TOOLCHAIN",
            "VIRTUAL_ENV",
            "HOME",
            "TMPDIR",
        ];
        map.retain(|k, _| {
            keep_sdk_keys.iter().any(|sk| k.eq_ignore_ascii_case(sk))
                || (!is_reserved_internal_env_key(k) && !is_dangerous_loader_env_key(k))
        });
        return map;
    }

    let mut allowed_roots = vec![root.to_string()];
    if caller_is_root {
        if let Some(proj) = proj_opt {
            for r in &proj.additional_roots {
                if !r.is_empty() && !allowed_roots.contains(r) {
                    allowed_roots.push(r.clone());
                }
            }
        }
    }
    let dotenv_rel = proj_opt
        .and_then(|p| p.dotenv_path.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(".env");
    if let Some(dotenv_file) =
        resolve_confined_dotenv_path(&allowed_roots, dotenv_rel, caller_is_root)
    {
        if let Ok(raw) = std::fs::read_to_string(&dotenv_file) {
            extend_from_dotenv_str(&mut map, &raw);
        }
    }

    if let Some(proj) = proj_opt {
        for item in &proj.env_vars {
            if !item.secret {
                if let Some(val) = &item.value {
                    insert_filtered_env_var(&mut map, &item.key, val.clone(), caller_is_root);
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
    /// ancestor/umbrella project snapshots, and `module_go_build_tags`.
    pub fn effective_go_build_tags_for_local_root(&self, root: &str) -> Vec<String> {
        let trimmed = root.trim().trim_end_matches('/');
        if trimmed.is_empty() {
            return Vec::new();
        }
        let cands = vec![trimmed.to_string()];
        let local_key = format!("local:{trimmed}");
        let guard = self.snapshots.read().unwrap_or_else(|e| e.into_inner());
        if let Some(exact) = guard.iter().rev().find(|s| {
            let sk = s.key.trim_end_matches('/');
            sk == trimmed || sk == local_key
        }) {
            return effective_go_build_tags_for_paths(&exact.env, &cands, true);
        }
        let mut out = Vec::new();
        for snap in guard.iter().rev() {
            let Some(snap_root) = snap.key.strip_prefix("local:") else {
                continue;
            };
            if let Some((_, mod_tags)) =
                match_longest_module_go_build_tags(&snap.env.module_go_build_tags, &cands, true)
            {
                for t in mod_tags {
                    push_unique(&mut out, t);
                }
                return out;
            }
            let sr = snap_root.trim_end_matches('/');
            let under_primary =
                !sr.is_empty() && (trimmed == sr || trimmed.starts_with(&format!("{sr}/")));
            if under_primary {
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
            true,
        )
    }

    /// Build a safe read-only/background probe environment for `root` (LSP servers,
    /// polyglot syntax checks, dependency inspection, formatters) without reading
    /// workspace `.env` / `proj.env_vars` and without prepending workspace `.venv/bin`.
    pub fn cached_probe_env_for_local_root_as(
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
            false,
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

    /// Stop all active test/program runs and reap local child process groups on shutdown.
    pub async fn shutdown(&self) {
        let pids = self.active_local_pids();
        let handles: Vec<Arc<runner::ActiveRunHandle>> =
            self.runs.iter().map(|e| Arc::clone(e.value())).collect();
        for h in handles {
            let _ = h.stop();
        }
        for pid in pids {
            runner_proc::cleanup_remaining_process_group(Some(pid), true).await;
        }
    }
}
