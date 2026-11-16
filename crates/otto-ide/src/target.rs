//! Unified IDE Target layer (`Local` filesystem/processes + `Ssh` SFTP/exec).
//!
//! All IDE subsystems (file tree, buffer read/write, search/replace, OS/arch
//! toolchain installer, environments, test runner, LSP helpers, and DAP
//! debugger) operate through [`TargetManager`] and [`ResolvedTarget`] so every
//! feature works transparently on both the local Mac/Linux machine and remote
//! SSH hosts.

pub mod complete;
mod fs_mutate;
mod fs_ops;
mod git_base;
mod manager_ops;
mod path_resolve;
pub(crate) mod sdk_archives;
pub(crate) mod sdk_paths;
mod search;
mod search_replace;
pub(crate) mod ssh_exec;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::domain::User;
use otto_core::{Error, Result};
use otto_ssh::{SftpParams, SftpSession};

use crate::local_history::LocalHistoryStore;
use crate::types::{IdePlatformInfo, IdeSshSpec, IdeTargetKind, IdeTargetSpec};
use crate::IdeCtx;

pub use self::fs_ops::{enforce_non_root_path, HistoryWriteOpts};
pub(crate) use self::path_resolve::normalize_posix_clean_root;
use self::sdk_paths::{
    collect_base_readonly_sdk_roots, collect_readonly_sdk_roots_with_env,
    discover_local_go_replace_roots,
};
pub(crate) use self::search::is_generated_build_dir;
pub(crate) use self::ssh_exec::canonicalize_local_root_dir;
use self::ssh_exec::{
    join_remote, map_io_err, remote_parent, resolve_local_prospective_in_roots,
    resolve_remote_in_roots, resolve_remote_in_roots_or_sdk, SshControlSocket,
};

/// Maximum bytes read into the text editor buffer (`4 MiB`).
pub const MAX_FILE_READ_BYTES: u64 = 4 * 1024 * 1024;
/// Maximum file size scanned during multi-file text/symbol search (`2 MiB`).
pub(super) const MAX_SEARCH_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Default cap on returned search items.
pub(super) const DEFAULT_SEARCH_LIMIT: usize = 200;
/// Hard ceiling on returned search items.
pub(super) const MAX_SEARCH_LIMIT: usize = 1000;
/// Idle retention for pooled SFTP sessions.
pub(super) const SFTP_IDLE_TTL: Duration = Duration::from_secs(90);
/// Short TTL for resolved local target root + SDK allowlist snapshots.
const LOCAL_RESOLVE_TTL: Duration = Duration::from_secs(5);
const LOCAL_RESOLVE_CACHE_CAP: usize = 256;

/// Captured output of a bounded command executed on a [`ResolvedTarget`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

impl ExecOutput {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// Canonical, authorized target handle passed to [`TargetManager`] operations.
#[derive(Debug, Clone)]
pub enum ResolvedTarget {
    Local {
        root: PathBuf,
        additional_roots: Vec<PathBuf>,
        readonly_roots: Vec<PathBuf>,
        is_root_user: bool,
        actor_id: String,
    },
    Ssh {
        spec: IdeSshSpec,
        root: String,
        additional_roots: Vec<String>,
        actor_id: String,
    },
}

impl ResolvedTarget {
    pub fn root_str(&self) -> String {
        match self {
            Self::Local { root, .. } => root.to_string_lossy().into_owned(),
            Self::Ssh { root, .. } => root.clone(),
        }
    }

    pub fn is_ssh(&self) -> bool {
        matches!(self, Self::Ssh { .. })
    }

    pub fn into_ssh_spec(self) -> Option<IdeSshSpec> {
        match self {
            Self::Ssh { spec, .. } => Some(spec),
            Self::Local { .. } => None,
        }
    }

    pub fn local_project_roots(&self) -> Vec<PathBuf> {
        match self {
            Self::Local {
                root,
                additional_roots,
                ..
            } => {
                let mut v = Vec::with_capacity(1 + additional_roots.len());
                v.push(root.clone());
                v.extend(additional_roots.iter().cloned());
                v
            }
            Self::Ssh { .. } => Vec::new(),
        }
    }

    pub fn local_readonly_roots(&self) -> &[PathBuf] {
        match self {
            Self::Local { readonly_roots, .. } => readonly_roots,
            Self::Ssh { .. } => &[],
        }
    }

    pub fn cache_key(&self) -> String {
        match self {
            Self::Local { root, .. } => format!("local:{}", root.display()),
            Self::Ssh { spec, root, .. } => {
                format!(
                    "ssh:{}:{}@{}:{}:{root}",
                    spec.connection_id,
                    spec.user,
                    spec.host,
                    spec.effective_port().unwrap_or(22)
                )
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct SshPoolKey {
    pub(super) actor_id: String,
    pub(super) connection_id: String,
    pub(super) params: SftpParams,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct LocalResolveCacheKey {
    actor_id: String,
    is_root_user: bool,
    write: bool,
    root: String,
    req_extra_roots: Vec<String>,
    env_gen: u64,
    proj_gen: u64,
    sdk_gen: u64,
}

/// Manages local and multiplexed SSH filesystem, search, and process operations.
#[derive(Clone)]
pub struct TargetManager {
    pub(super) sftp_pool: Arc<DashMap<SshPoolKey, (Arc<SftpSession>, Instant)>>,
    pub(super) ssh_sockets: Arc<DashMap<SshPoolKey, (Arc<SshControlSocket>, Instant)>>,
    pub(super) ssh_platform_cache: Arc<DashMap<String, IdePlatformInfo>>,
    local_resolve_cache: Arc<DashMap<LocalResolveCacheKey, (ResolvedTarget, Instant)>>,
    pub(super) history: Option<LocalHistoryStore>,
    pub(super) lsp_pool: Option<Arc<crate::lsp_host::pool::LspPool>>,
    pub(super) environments: Option<crate::environments::EnvironmentManager>,
}

impl Default for TargetManager {
    fn default() -> Self {
        Self::new()
    }
}

fn path_within_any(cand: &Path, allowed: &[PathBuf]) -> bool {
    let Ok(canon) = cand.canonicalize() else {
        return false;
    };
    allowed.iter().any(|base| canon.starts_with(base))
}

impl TargetManager {
    pub fn new() -> Self {
        Self {
            sftp_pool: Arc::new(DashMap::new()),
            ssh_sockets: Arc::new(DashMap::new()),
            ssh_platform_cache: Arc::new(DashMap::new()),
            local_resolve_cache: Arc::new(DashMap::new()),
            history: None,
            lsp_pool: None,
            environments: None,
        }
    }

    pub fn with_history(history: LocalHistoryStore) -> Self {
        Self {
            history: Some(history),
            ..Self::new()
        }
    }

    pub fn with_services(
        history: LocalHistoryStore,
        lsp_pool: Arc<crate::lsp_host::pool::LspPool>,
        environments: crate::environments::EnvironmentManager,
    ) -> Self {
        Self {
            history: Some(history),
            lsp_pool: Some(lsp_pool),
            environments: Some(environments),
            ..Self::new()
        }
    }

    pub fn invalidate_local_resolve_cache(&self) {
        self.local_resolve_cache.clear();
    }

    /// Resolve and authorize an [`IdeTargetSpec`] for read operations using [`IdeCtx`].
    pub async fn resolve<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        spec: &IdeTargetSpec,
    ) -> Result<ResolvedTarget> {
        self.resolve_with_access(ctx, user, spec, false).await
    }

    /// Resolve and authorize an [`IdeTargetSpec`] for write/exec operations using [`IdeCtx`].
    pub async fn resolve_write<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        spec: &IdeTargetSpec,
    ) -> Result<ResolvedTarget> {
        self.resolve_with_access(ctx, user, spec, true).await
    }

    async fn resolve_with_access<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        spec: &IdeTargetSpec,
        write: bool,
    ) -> Result<ResolvedTarget> {
        spec.validate()?;
        match spec.kind {
            IdeTargetKind::Local => {
                let pool = ctx.pool();
                let _ = ctx.ide().projects.ensure_hydrated(&pool).await;
                ctx.ide().environments.ensure_hydrated(&pool).await;
                let has_root_auth = otto_core::auth::root_authority(user);

                let cache_key = LocalResolveCacheKey {
                    actor_id: user.id.as_str().to_string(),
                    is_root_user: has_root_auth,
                    write,
                    root: spec.root.clone(),
                    req_extra_roots: spec.merged_extra_roots(),
                    env_gen: ctx.ide().environments.cache_generation(),
                    proj_gen: ctx.ide().projects.cache_generation(),
                    sdk_gen: ctx.ide().toolchains.local_sdks_generation(),
                };
                if let Some(entry) = self.local_resolve_cache.get(&cache_key) {
                    if entry.1.elapsed() < LOCAL_RESOLVE_TTL {
                        return Ok(entry.0.clone());
                    }
                }

                // Validate the caller's explicitly requested primary + extra roots first.
                ctx.authorize_local_target(user, spec, write).await?;

                let mut merged_spec = spec.clone();
                let proj_env_opt = ctx
                    .ide()
                    .environments
                    .cached_project_env(&spec.project_key())
                    .or_else(|| {
                        self.environments
                            .as_ref()
                            .and_then(|e| e.cached_project_env(&spec.project_key()))
                    });
                let mut server_candidates: Vec<String> = Vec::new();
                if let Some(proj_env) = &proj_env_opt {
                    for ar in &proj_env.additional_roots {
                        if !ar.trim().is_empty() && !server_candidates.contains(ar) {
                            server_candidates.push(ar.clone());
                        }
                    }
                }
                for proj_ar in ctx
                    .ide()
                    .projects
                    .folders_for_root_for_user(&spec.root, user)
                {
                    if !proj_ar.trim().is_empty() && !server_candidates.contains(&proj_ar) {
                        server_candidates.push(proj_ar);
                    }
                }
                for legacy_ar in ctx
                    .ide()
                    .projects
                    .legacy_unowned_folders_for_root(&spec.root, has_root_auth)
                {
                    if !legacy_ar.trim().is_empty() && !server_candidates.contains(&legacy_ar) {
                        server_candidates.push(legacy_ar);
                    }
                }
                // Scope server-stored additional_roots and shared Named Project folders
                // per user so an unpermitted secondary root is filtered out rather than
                // poisoning primary workspace access with 403.
                for cand_ar in server_candidates {
                    if merged_spec.additional_roots.contains(&cand_ar) {
                        continue;
                    }
                    if has_root_auth
                        || ctx
                            .authorize_local_target(user, &IdeTargetSpec::local(&cand_ar), write)
                            .await
                            .is_ok()
                    {
                        merged_spec.additional_roots.push(cand_ar);
                    }
                }

                let toolchains_dir = ctx.ide().toolchains.toolchains_dir();
                let reg_sdks = ctx.ide().toolchains.cached_local_sdks(&pool).await;
                let mut trusted_sdk_bases = vec![toolchains_dir.clone()];
                let mut extra_sdk_dirs = vec![toolchains_dir];
                for reg_sdk in reg_sdks {
                    let p = PathBuf::from(reg_sdk.path);
                    if let Ok(c) = p.canonicalize() {
                        trusted_sdk_bases.push(c);
                    }
                    extra_sdk_dirs.push(p);
                }
                if !has_root_auth {
                    if let Ok(tc_canon) = extra_sdk_dirs[0].canonicalize() {
                        trusted_sdk_bases[0] = tc_canon;
                    }
                    trusted_sdk_bases.extend(collect_base_readonly_sdk_roots(None));
                }

                if let Some(proj_env) = proj_env_opt {
                    let mut custom_sdks = Vec::new();
                    for sdk_opt in [
                        proj_env.go_root,
                        proj_env.java_home,
                        proj_env.python_interpreter,
                        proj_env.node_home,
                    ] {
                        if let Some(s) = sdk_opt.filter(|v| !v.trim().is_empty()) {
                            custom_sdks.push(s);
                        }
                    }
                    for mod_sdk in proj_env.module_sdks.into_values() {
                        if !mod_sdk.trim().is_empty() {
                            custom_sdks.push(mod_sdk);
                        }
                    }
                    for custom in custom_sdks {
                        let p = PathBuf::from(&custom);
                        if has_root_auth
                            || path_within_any(&p, &trusted_sdk_bases)
                            || ctx
                                .authorize_local_target(user, &IdeTargetSpec::local(&custom), false)
                                .await
                                .is_ok()
                        {
                            extra_sdk_dirs.push(p);
                        }
                    }
                }

                let is_root = has_root_auth;
                let actor_id = user.id.as_str().to_string();
                let shell_snap = if is_root {
                    Some(ctx.ide().toolchains.shell_env_snapshot())
                } else {
                    None
                };
                let (mut resolved, go_candidates) =
                    tokio::task::spawn_blocking(move || -> Result<_> {
                        let canonical = canonicalize_local_root_dir(&merged_spec.root, is_root)?;
                        let extra_list = merged_spec.merged_extra_roots();
                        let mut additional_roots = Vec::with_capacity(extra_list.len());
                        for raw_extra in &extra_list {
                            let trimmed = raw_extra.trim();
                            if trimmed.is_empty() {
                                continue;
                            }
                            if let Ok(canon_extra) = canonicalize_local_root_dir(trimmed, is_root) {
                                if canon_extra != canonical
                                    && !additional_roots.contains(&canon_extra)
                                {
                                    additional_roots.push(canon_extra);
                                }
                            }
                        }
                        let go_cands =
                            discover_local_go_replace_roots(&canonical, &additional_roots, is_root);
                        let mut sdk_inputs = extra_sdk_dirs;
                        if is_root {
                            for gc in &go_cands {
                                if *gc != canonical && !additional_roots.contains(gc) {
                                    sdk_inputs.push(gc.clone());
                                }
                            }
                        }
                        let readonly_roots =
                            collect_readonly_sdk_roots_with_env(&sdk_inputs, shell_snap.as_ref());
                        Ok((
                            ResolvedTarget::Local {
                                root: canonical,
                                additional_roots,
                                readonly_roots,
                                is_root_user: is_root,
                                actor_id,
                            },
                            go_cands,
                        ))
                    })
                    .await
                    .map_err(|e| Error::Internal(format!("resolve_local join: {e}")))??;

                // Non-root callers only gain read-only access to discovered Go parent/replace
                // roots that explicitly pass workspace RBAC authorization.
                if !has_root_auth && !go_candidates.is_empty() {
                    if let ResolvedTarget::Local {
                        ref root,
                        ref additional_roots,
                        ref mut readonly_roots,
                        ..
                    } = resolved
                    {
                        for cand in go_candidates {
                            if cand == *root
                                || additional_roots.contains(&cand)
                                || readonly_roots.contains(&cand)
                            {
                                continue;
                            }
                            let cand_str = cand.to_string_lossy().into_owned();
                            if ctx
                                .authorize_local_target(
                                    user,
                                    &IdeTargetSpec::local(cand_str),
                                    false,
                                )
                                .await
                                .is_ok()
                            {
                                readonly_roots.push(cand);
                            }
                        }
                    }
                }

                if self.local_resolve_cache.len() >= LOCAL_RESOLVE_CACHE_CAP {
                    self.local_resolve_cache
                        .retain(|_, (_, at)| at.elapsed() < LOCAL_RESOLVE_TTL);
                    if self.local_resolve_cache.len() >= LOCAL_RESOLVE_CACHE_CAP {
                        self.local_resolve_cache.clear();
                    }
                }
                self.local_resolve_cache
                    .insert(cache_key, (resolved.clone(), Instant::now()));
                Ok(resolved)
            }
            IdeTargetKind::Ssh => {
                let conn_id = spec.connection_id.as_ref().ok_or_else(|| {
                    Error::Invalid("connection_id is required for SSH targets".into())
                })?;
                let ssh_spec = ctx.resolve_ssh_connection(user, conn_id).await?;
                let pool = ctx.pool();
                ctx.ide().environments.ensure_hydrated(&pool).await;
                let norm_root = self::path_resolve::normalize_remote_root(&spec.root)?;
                let mut merged_spec = spec.clone();
                merged_spec.root = norm_root.clone();
                let norm_key = format!("ssh:{conn_id}:{norm_root}");
                for key in [spec.project_key(), norm_key] {
                    let stored_env =
                        ctx.ide().environments.cached_project_env(&key).or_else(|| {
                            self.environments
                                .as_ref()
                                .and_then(|e| e.cached_project_env(&key))
                        });
                    if let Some(proj_env) = stored_env {
                        for ar in proj_env.additional_roots {
                            if let Ok(clean_ar) = self::path_resolve::normalize_remote_root(&ar) {
                                if clean_ar != norm_root
                                    && !merged_spec.additional_roots.contains(&clean_ar)
                                {
                                    merged_spec.additional_roots.push(clean_ar);
                                }
                            }
                        }
                    }
                }
                self.resolve_with_ssh(&merged_spec, user.id.as_str(), ssh_spec)
            }
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_extra;
