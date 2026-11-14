//! Unified IDE Target layer (`Local` filesystem/processes + `Ssh` SFTP/exec).
//!
//! All IDE subsystems (file tree, buffer read/write, search/replace, OS/arch
//! toolchain installer, environments, test runner, LSP helpers, and DAP
//! debugger) operate through [`TargetManager`] and [`ResolvedTarget`] so every
//! feature works transparently on both the local Mac/Linux machine and remote
//! SSH hosts.

pub mod complete;
mod fs_ops;
pub(crate) mod sdk_paths;
mod search;
mod ssh_exec;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::domain::User;
use otto_core::{Error, Result};
use otto_ssh::{SftpParams, SftpSession};

use crate::local_history::LocalHistoryStore;
use crate::types::{
    IdeFileReadResp, IdeFileWriteResp, IdeFsMutateOp, IdeFsMutateResp, IdeFsTreeResp,
    IdePlatformInfo, IdeReplaceReq, IdeReplaceResp, IdeSearchReq, IdeSearchResp, IdeSshSpec,
    IdeTargetKind, IdeTargetSpec,
};
use crate::IdeCtx;

pub(crate) use self::fs_ops::enforce_non_root_path;
use self::fs_ops::{
    build_file_read_resp, build_remote_tree_entries, list_local_dir_sync, mutate_local_fs_in_roots,
    mutate_remote_fs, normalize_arch_name, normalize_os_name, parse_uname_output,
    read_bounded_bytes, resolve_local_existing_in_roots, sha256_hex, verify_local_expected_sha,
    verify_remote_expected_sha, write_local_atomic,
};
use self::sdk_paths::{
    collect_readonly_sdk_roots, discover_local_go_replace_roots, is_virtual_archive_uri,
    read_virtual_java_archive, resolve_local_readable,
};
use self::search::{
    build_matcher, collect_local_files_multi, replace_local_sync, replace_remote_ssh,
    search_local_sync, search_remote_ssh,
};
pub(crate) use self::ssh_exec::canonicalize_local_root_dir;
pub(super) use self::ssh_exec::{
    join_remote, map_io_err, normalize_remote_root, remote_parent,
    resolve_local_prospective_in_roots, resolve_remote_in_roots, resolve_remote_in_roots_or_sdk,
    resolve_remote_subpath, validate_ssh_spec, SshControlSocket,
};

/// Maximum bytes read into the text editor buffer (`4 MiB`).
pub const MAX_FILE_READ_BYTES: u64 = 4 * 1024 * 1024;
/// Maximum file size scanned during multi-file text/symbol search (`2 MiB`).
pub(super) const MAX_SEARCH_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Default cap on returned search items.
const DEFAULT_SEARCH_LIMIT: usize = 200;
/// Hard ceiling on returned search items.
const MAX_SEARCH_LIMIT: usize = 1000;
/// Idle retention for pooled SFTP sessions.
pub(super) const SFTP_IDLE_TTL: Duration = Duration::from_secs(90);

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
                    spec.connection_id, spec.user, spec.host, spec.port
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

/// Manages local and multiplexed SSH filesystem, search, and process operations.
#[derive(Clone)]
pub struct TargetManager {
    pub(super) sftp_pool: Arc<DashMap<SshPoolKey, (Arc<SftpSession>, Instant)>>,
    pub(super) ssh_sockets: Arc<DashMap<SshPoolKey, Arc<SshControlSocket>>>,
    pub(super) ssh_platform_cache: Arc<DashMap<String, IdePlatformInfo>>,
    history: Option<LocalHistoryStore>,
}

impl Default for TargetManager {
    fn default() -> Self {
        Self::new()
    }
}

impl TargetManager {
    pub fn new() -> Self {
        Self {
            sftp_pool: Arc::new(DashMap::new()),
            ssh_sockets: Arc::new(DashMap::new()),
            ssh_platform_cache: Arc::new(DashMap::new()),
            history: None,
        }
    }

    pub fn with_history(history: LocalHistoryStore) -> Self {
        Self {
            history: Some(history),
            ..Self::new()
        }
    }

    /// Resolve and authorize an [`IdeTargetSpec`] using [`IdeCtx`].
    pub async fn resolve<S: IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        spec: &IdeTargetSpec,
    ) -> Result<ResolvedTarget> {
        spec.validate()?;
        match spec.kind {
            IdeTargetKind::Local => {
                let mut merged_spec = spec.clone();
                let mut extra_sdk_dirs = vec![ctx.ide().toolchains.toolchains_dir()];
                if let Ok(proj_env) = ctx
                    .ide()
                    .environments
                    .get_project_env(&ctx.pool(), spec)
                    .await
                {
                    for ar in proj_env.additional_roots {
                        if !ar.trim().is_empty() && !merged_spec.additional_roots.contains(&ar) {
                            merged_spec.additional_roots.push(ar);
                        }
                    }
                    for sdk_opt in [
                        proj_env.go_root,
                        proj_env.java_home,
                        proj_env.python_interpreter,
                        proj_env.node_home,
                    ] {
                        if let Some(p) = sdk_opt.filter(|s| !s.trim().is_empty()) {
                            extra_sdk_dirs.push(PathBuf::from(p));
                        }
                    }
                }
                for cached_ar in crate::environments::cached_additional_roots(&spec.project_key()) {
                    if !cached_ar.trim().is_empty()
                        && !merged_spec.additional_roots.contains(&cached_ar)
                    {
                        merged_spec.additional_roots.push(cached_ar);
                    }
                }
                for proj_ar in ctx.ide().projects.folders_for_root(&spec.root) {
                    if !proj_ar.trim().is_empty()
                        && !merged_spec.additional_roots.contains(&proj_ar)
                    {
                        merged_spec.additional_roots.push(proj_ar);
                    }
                }
                self.resolve_local_with_sdks(&merged_spec, user.is_root, &extra_sdk_dirs)
            }
            IdeTargetKind::Ssh => {
                let conn_id = spec.connection_id.as_ref().ok_or_else(|| {
                    Error::Invalid("connection_id is required for SSH targets".into())
                })?;
                let ssh_spec = ctx.resolve_ssh_connection(user, conn_id).await?;
                self.resolve_with_ssh(spec, user.id.as_str(), ssh_spec)
            }
        }
    }

    /// Resolve a local target directory, expanding `~`, canonicalizing symlinks,
    /// and enforcing `otto_core::secret_paths` for non-root callers.
    pub fn resolve_local(
        &self,
        spec: &IdeTargetSpec,
        is_root_user: bool,
    ) -> Result<ResolvedTarget> {
        self.resolve_local_with_sdks(spec, is_root_user, &[])
    }

    pub fn resolve_local_with_sdks(
        &self,
        spec: &IdeTargetSpec,
        is_root_user: bool,
        extra_sdk_dirs: &[PathBuf],
    ) -> Result<ResolvedTarget> {
        spec.validate()?;
        let canonical = canonicalize_local_root_dir(&spec.root, is_root_user)?;
        let extra_list = spec.merged_extra_roots();
        let mut additional_roots = Vec::with_capacity(extra_list.len());
        for raw_extra in &extra_list {
            let trimmed = raw_extra.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Ok(canon_extra) = canonicalize_local_root_dir(trimmed, is_root_user) {
                if canon_extra != canonical && !additional_roots.contains(&canon_extra) {
                    additional_roots.push(canon_extra);
                }
            }
        }
        for go_rep in discover_local_go_replace_roots(&canonical, &additional_roots, is_root_user) {
            if go_rep != canonical && !additional_roots.contains(&go_rep) {
                additional_roots.push(go_rep);
            }
        }
        let readonly_roots = collect_readonly_sdk_roots(extra_sdk_dirs);
        Ok(ResolvedTarget::Local {
            root: canonical,
            additional_roots,
            readonly_roots,
            is_root_user,
        })
    }

    /// Resolve a remote SSH target with full hostname/user/jump validation.
    pub fn resolve_with_ssh(
        &self,
        spec: &IdeTargetSpec,
        actor_id: &str,
        ssh: IdeSshSpec,
    ) -> Result<ResolvedTarget> {
        spec.validate()?;
        validate_ssh_spec(&ssh)?;
        let norm_root = normalize_remote_root(&spec.root)?;
        let mut additional_roots = Vec::new();
        for extra in &spec.merged_extra_roots() {
            if let Ok(nr) = normalize_remote_root(extra) {
                if nr != norm_root && !additional_roots.contains(&nr) {
                    additional_roots.push(nr);
                }
            }
        }
        Ok(ResolvedTarget::Ssh {
            spec: ssh,
            root: norm_root,
            additional_roots,
            actor_id: actor_id.to_string(),
        })
    }

    /// Detect `(os, arch)` of `target` (`macos`/`linux` × `aarch64`/`x86_64`).
    pub async fn detect_platform(&self, target: &ResolvedTarget) -> Result<IdePlatformInfo> {
        match target {
            ResolvedTarget::Local { .. } => Ok(IdePlatformInfo {
                os: normalize_os_name(std::env::consts::OS),
                arch: normalize_arch_name(std::env::consts::ARCH),
            }),
            ResolvedTarget::Ssh { .. } => {
                let key = target.cache_key();
                if let Some(cached) = self.ssh_platform_cache.get(&key) {
                    return Ok(cached.clone());
                }
                let out = self
                    .exec_command(
                        target,
                        "uname",
                        &["-s".into(), "-m".into()],
                        Some("/"),
                        &BTreeMap::new(),
                        Duration::from_secs(12),
                    )
                    .await?;
                if !out.success() {
                    return Err(Error::Upstream(format!(
                        "remote uname failed: {}",
                        out.stderr.trim()
                    )));
                }
                let info = parse_uname_output(&out.stdout);
                if self.ssh_platform_cache.len() >= 64 {
                    self.ssh_platform_cache.clear();
                }
                self.ssh_platform_cache.insert(key, info.clone());
                Ok(info)
            }
        }
    }

    /// Shallow 1-level directory listing on `target` (`<1ms` locally, zero subprocesses).
    pub async fn list_tree(
        &self,
        target: &ResolvedTarget,
        path: Option<&str>,
        show_hidden: bool,
    ) -> Result<IdeFsTreeResp> {
        let platform = self.detect_platform(target).await?;
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                let (resolved_dir, matched_root) = resolve_local_existing_in_roots(
                    root,
                    additional_roots,
                    *is_root_user,
                    path.unwrap_or(""),
                )?;
                if path.unwrap_or("").trim().is_empty() {
                    crate::lsp_host::pool::evict_detached_other_roots(
                        &matched_root.to_string_lossy(),
                    );
                }
                let dir_for_scan = resolved_dir.clone();
                let restricted = !*is_root_user;
                let entries = tokio::task::spawn_blocking(move || {
                    list_local_dir_sync(&dir_for_scan, show_hidden, restricted)
                })
                .await
                .map_err(|e| Error::Internal(format!("list_tree join: {e}")))??;
                let parent = if resolved_dir == matched_root {
                    None
                } else {
                    resolved_dir
                        .parent()
                        .filter(|p| p.starts_with(&matched_root))
                        .map(|p| p.to_string_lossy().into_owned())
                };
                Ok(IdeFsTreeResp {
                    root: matched_root.to_string_lossy().into_owned(),
                    path: resolved_dir.to_string_lossy().into_owned(),
                    parent,
                    target_os: platform.os,
                    target_arch: platform.arch,
                    entries,
                })
            }
            ResolvedTarget::Ssh {
                spec,
                root,
                additional_roots,
                actor_id,
            } => {
                let (remote_dir, matched_root) =
                    resolve_remote_in_roots(root, additional_roots, path.unwrap_or(""))?;
                let sftp = self.acquire_sftp(actor_id, spec)?;
                let (raw_entries, _) = sftp.list_capped(&remote_dir, 5_000).await?;
                let entries = build_remote_tree_entries(&remote_dir, raw_entries, show_hidden);
                let parent = if remote_dir == matched_root {
                    None
                } else {
                    remote_parent(&remote_dir).filter(|p| p.starts_with(matched_root.as_str()))
                };
                Ok(IdeFsTreeResp {
                    root: matched_root,
                    path: remote_dir,
                    parent,
                    target_os: platform.os,
                    target_arch: platform.arch,
                    entries,
                })
            }
        }
    }

    /// Read a UTF-8 file from `target` (including attached folders, allowlisted
    /// read-only external SDK/module paths, and Java `jar:file://`/`jdt://` archives).
    pub async fn read_file(&self, target: &ResolvedTarget, path: &str) -> Result<IdeFileReadResp> {
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                readonly_roots,
                is_root_user,
            } => {
                if is_virtual_archive_uri(path) {
                    return read_virtual_java_archive(
                        root,
                        additional_roots,
                        readonly_roots,
                        *is_root_user,
                        path,
                    )
                    .await;
                }
                let (canonical, force_readonly) = resolve_local_readable(
                    root,
                    additional_roots,
                    readonly_roots,
                    *is_root_user,
                    path,
                )?;
                let meta = tokio::fs::metadata(&canonical)
                    .await
                    .map_err(|e| map_io_err("stat file", &canonical, e))?;
                if !meta.is_file() {
                    return Err(Error::Invalid(format!(
                        "'{}' is not a regular file",
                        canonical.display()
                    )));
                }
                let size = meta.len();
                let read_only = force_readonly || meta.permissions().readonly();
                let (bytes, truncated) =
                    read_bounded_bytes(&canonical, MAX_FILE_READ_BYTES).await?;
                build_file_read_resp(
                    canonical.to_string_lossy().into_owned(),
                    &bytes,
                    size,
                    read_only,
                    truncated,
                )
            }
            ResolvedTarget::Ssh {
                spec,
                root,
                additional_roots,
                actor_id,
            } => {
                let (remote_path, matched_root) =
                    resolve_remote_in_roots_or_sdk(root, additional_roots, path)?;
                let force_ro = matched_root.is_none();
                let sftp = self.acquire_sftp(actor_id, spec)?;
                let tmp = tempfile::NamedTempFile::new()
                    .map_err(|e| Error::Internal(format!("temp file: {e}")))?;
                let tmp_path = tmp.path().to_string_lossy().into_owned();
                let truncated = sftp
                    .download_prefix(&remote_path, &tmp_path, MAX_FILE_READ_BYTES)
                    .await?;
                let bytes = tokio::fs::read(tmp.path())
                    .await
                    .map_err(|e| Error::Internal(format!("read downloaded temp: {e}")))?;
                let size = if truncated {
                    sftp.file_size(&remote_path)
                        .await
                        .unwrap_or(bytes.len() as u64)
                } else {
                    bytes.len() as u64
                };
                build_file_read_resp(remote_path, &bytes, size, force_ro, truncated)
            }
        }
    }

    /// Atomically write a UTF-8 file to `target` with optional `expected_sha256`
    /// optimistic concurrency check (`409 Conflict` on mismatch) and Local History capture.
    pub async fn write_file(
        &self,
        target: &ResolvedTarget,
        path: &str,
        content: &str,
        expected_sha256: Option<&str>,
        create_dirs: bool,
    ) -> Result<IdeFileWriteResp> {
        if is_virtual_archive_uri(path) {
            return Err(Error::Forbidden(
                "archive and external SDK files are read-only".into(),
            ));
        }
        let new_bytes = content.as_bytes();
        let new_sha = sha256_hex(new_bytes);
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                let (dest, _) = resolve_local_prospective_in_roots(
                    root,
                    additional_roots,
                    *is_root_user,
                    path,
                    create_dirs,
                )
                .await?;
                if let Some(exp) = expected_sha256.filter(|s| !s.is_empty()) {
                    verify_local_expected_sha(&dest, exp).await?;
                }
                if let Some(hist) = &self.history {
                    if dest.is_file() {
                        let _ = hist.record_existing_file_sync(
                            root,
                            &dest,
                            "external",
                            "Before edit",
                            true,
                        );
                    }
                }
                write_local_atomic(&dest, new_bytes).await?;
                if let Some(hist) = &self.history {
                    let _ = hist.record_bytes_sync(root, &dest, new_bytes, "saved", "Saved", true);
                }
                Ok(IdeFileWriteResp {
                    path: dest.to_string_lossy().into_owned(),
                    size: new_bytes.len() as u64,
                    sha256: new_sha,
                })
            }
            ResolvedTarget::Ssh {
                spec,
                root,
                additional_roots,
                actor_id,
            } => {
                let (remote_path, _) = resolve_remote_in_roots(root, additional_roots, path)?;
                let sftp = self.acquire_sftp(actor_id, spec)?;
                if let Some(exp) = expected_sha256.filter(|s| !s.is_empty()) {
                    verify_remote_expected_sha(&sftp, &remote_path, exp).await?;
                }
                if create_dirs {
                    if let Some(parent) = remote_parent(&remote_path) {
                        let _ = self
                            .exec_command(
                                target,
                                "mkdir",
                                &["-p".into(), parent],
                                None,
                                &BTreeMap::new(),
                                Duration::from_secs(10),
                            )
                            .await;
                    }
                }
                let tmp = tempfile::NamedTempFile::new()
                    .map_err(|e| Error::Internal(format!("temp upload file: {e}")))?;
                tokio::fs::write(tmp.path(), new_bytes)
                    .await
                    .map_err(|e| Error::Internal(format!("write temp upload file: {e}")))?;
                let tmp_str = tmp.path().to_string_lossy().into_owned();
                sftp.upload(&tmp_str, &remote_path).await?;
                Ok(IdeFileWriteResp {
                    path: remote_path,
                    size: new_bytes.len() as u64,
                    sha256: new_sha,
                })
            }
        }
    }

    /// Perform filesystem tree mutations (`mkdir`, `create_file`, `rename`,
    /// `delete`, `duplicate`) across both local and SSH targets.
    pub async fn mutate_fs(
        &self,
        target: &ResolvedTarget,
        op: IdeFsMutateOp,
        path: &str,
        dest_path: Option<&str>,
    ) -> Result<IdeFsMutateResp> {
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                mutate_local_fs_in_roots(
                    root,
                    additional_roots,
                    *is_root_user,
                    op,
                    path,
                    dest_path,
                    self.history.as_ref(),
                )
                .await
            }
            ResolvedTarget::Ssh {
                spec,
                root,
                actor_id,
                ..
            } => {
                let sftp = self.acquire_sftp(actor_id, spec)?;
                mutate_remote_fs(&sftp, root, op, path, dest_path).await
            }
        }
    }

    /// List workspace source file paths across primary + attached roots.
    pub async fn list_files(&self, target: &ResolvedTarget, limit: usize) -> Result<Vec<String>> {
        let cap = limit.clamp(1, 20_000);
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                let root_clone = root.clone();
                let extra_clone = additional_roots.clone();
                let restricted = !*is_root_user;
                tokio::task::spawn_blocking(move || {
                    collect_local_files_multi(&root_clone, &extra_clone, restricted, None, cap)
                })
                .await
                .map_err(|e| Error::Internal(format!("list_files join: {e}")))?
            }
            ResolvedTarget::Ssh { root, .. } => {
                let script = format!(
                    "if command -v rg >/dev/null 2>&1; then rg --files | head -n {cap}; \
                     else find . -maxdepth 8 -type f -not -path '*/.*' -not -path '*/node_modules/*' -not -path '*/target/*' | head -n {cap}; fi"
                );
                let out = self
                    .exec_shell(
                        target,
                        &script,
                        Some(root),
                        &BTreeMap::new(),
                        Duration::from_secs(15),
                    )
                    .await?;
                let files = out
                    .stdout
                    .lines()
                    .map(|l| l.trim().trim_start_matches("./"))
                    .filter(|l| !l.is_empty())
                    .map(|rel| join_remote(root, rel))
                    .collect();
                Ok(files)
            }
        }
    }

    /// Fast multi-mode workspace search (`files`, `text`, `symbols`, `all`).
    pub async fn search(
        &self,
        target: &ResolvedTarget,
        req: &IdeSearchReq,
    ) -> Result<IdeSearchResp> {
        let query = req.query.trim().to_string();
        if query.is_empty() {
            return Ok(IdeSearchResp {
                items: Vec::new(),
                truncated: false,
            });
        }
        let limit = req
            .limit
            .unwrap_or(DEFAULT_SEARCH_LIMIT)
            .clamp(1, MAX_SEARCH_LIMIT);
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                let root_clone = root.clone();
                let extra_clone = additional_roots.clone();
                let restricted = !*is_root_user;
                let req_clone = req.clone();
                tokio::task::spawn_blocking(move || {
                    search_local_sync(&root_clone, &extra_clone, restricted, &req_clone, limit)
                })
                .await
                .map_err(|e| Error::Internal(format!("search join: {e}")))?
            }
            ResolvedTarget::Ssh { root, .. } => {
                search_remote_ssh(self, target, root, req, limit).await
            }
        }
    }

    /// Multi-file search & replace with `dry_run` diff preview and atomic writes.
    pub async fn replace(
        &self,
        target: &ResolvedTarget,
        req: &IdeReplaceReq,
    ) -> Result<IdeReplaceResp> {
        if req.query.is_empty() {
            return Err(Error::Invalid("replace query must not be empty".into()));
        }
        let matcher = build_matcher(&req.query, req.regex, req.case_sensitive, req.whole_word)?;
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                let root_clone = root.clone();
                let extra_clone = additional_roots.clone();
                let restricted = !*is_root_user;
                let req_clone = req.clone();
                tokio::task::spawn_blocking(move || {
                    replace_local_sync(&root_clone, &extra_clone, restricted, &req_clone, &matcher)
                })
                .await
                .map_err(|e| Error::Internal(format!("replace join: {e}")))?
            }
            ResolvedTarget::Ssh { root, .. } => {
                replace_remote_ssh(self, target, root, req, &matcher).await
            }
        }
    }
}

#[cfg(test)]
mod tests;
