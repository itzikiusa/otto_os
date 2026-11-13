//! Unified IDE Target layer (`Local` filesystem/processes + `Ssh` SFTP/exec).
//!
//! All IDE subsystems (file tree, buffer read/write, search/replace, OS/arch
//! toolchain installer, environments, test runner, LSP helpers, and DAP
//! debugger) operate through [`TargetManager`] and [`ResolvedTarget`] so every
//! feature works transparently on both the local Mac/Linux machine and remote
//! SSH hosts.

mod fs_ops;
pub(crate) mod sdk_paths;
mod search;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::domain::User;
use otto_core::{Error, Result};
use otto_ssh::{
    valid_proxy_jump, valid_ssh_host, valid_ssh_user, SftpParams, SftpSession, SshTunnel,
    SshTunnelConfig,
};
use tokio::process::{Child, Command};

use crate::types::{
    IdeFileReadResp, IdeFileWriteResp, IdeFsMutateOp, IdeFsMutateResp, IdeFsTreeResp,
    IdePlatformInfo, IdeReplaceReq, IdeReplaceResp, IdeSearchReq, IdeSearchResp, IdeSshSpec,
    IdeTargetKind, IdeTargetSpec,
};
use crate::IdeCtx;

pub(super) use self::fs_ops::enforce_non_root_path;
use self::fs_ops::{
    build_file_read_resp, build_remote_tree_entries, list_local_dir_sync, mutate_local_fs_in_roots,
    mutate_remote_fs, normalize_arch_name, normalize_os_name, parse_uname_output,
    read_bounded_bytes, resolve_local_existing_in_roots, sha256_hex, verify_local_expected_sha,
    verify_remote_expected_sha, write_local_atomic,
};
use self::sdk_paths::{
    collect_readonly_sdk_roots, is_virtual_archive_uri, read_virtual_java_archive,
    resolve_local_readable, strip_file_uri,
};
use self::search::{
    build_matcher, collect_local_files_multi, replace_local_sync, replace_remote_ssh,
    search_local_sync, search_remote_ssh,
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
const SFTP_IDLE_TTL: Duration = Duration::from_secs(90);

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
struct SshPoolKey {
    actor_id: String,
    connection_id: String,
    params: SftpParams,
}

/// Private ControlMaster socket directory for multiplexed `ssh` exec/stdio spawns.
struct SshControlSocket {
    dir: PathBuf,
    ctl_path: String,
    ssh_target: String,
}

impl SshControlSocket {
    fn new(spec: &IdeSshSpec) -> Result<Self> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let pid = std::process::id();
        let base = if Path::new("/tmp").is_dir() {
            PathBuf::from("/tmp")
        } else {
            std::env::temp_dir()
        };
        let dir = base.join(format!("oi-{pid}-{n}"));
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Internal(format!("create ssh control dir: {e}")))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| Error::Internal(format!("secure ssh control dir: {e}")))?;
        }
        let ctl_path = dir.join("c.s").to_string_lossy().into_owned();
        let ssh_target = if spec.user.is_empty() {
            spec.host.clone()
        } else {
            format!("{}@{}", spec.user, spec.host)
        };
        Ok(Self {
            dir,
            ctl_path,
            ssh_target,
        })
    }
}

impl Drop for SshControlSocket {
    fn drop(&mut self) {
        let dir = self.dir.clone();
        let ctl = self.ctl_path.clone();
        let target = self.ssh_target.clone();
        std::thread::spawn(move || {
            if Path::new(&ctl).exists() {
                let _ = std::process::Command::new("ssh")
                    .args([
                        "-o",
                        &format!("ControlPath={ctl}"),
                        "-O",
                        "exit",
                        "--",
                        &target,
                    ])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
            #[allow(clippy::disallowed_methods)]
            let _ = std::fs::remove_dir_all(dir);
        });
    }
}

/// Manages local and multiplexed SSH filesystem, search, and process operations.
#[derive(Clone)]
pub struct TargetManager {
    sftp_pool: Arc<DashMap<SshPoolKey, (Arc<SftpSession>, Instant)>>,
    ssh_sockets: Arc<DashMap<SshPoolKey, Arc<SshControlSocket>>>,
    ssh_platform_cache: Arc<DashMap<String, IdePlatformInfo>>,
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
        let mut additional_roots = Vec::with_capacity(spec.additional_roots.len());
        for raw_extra in &spec.additional_roots {
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
        for extra in &spec.additional_roots {
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
    /// optimistic concurrency check (`409 Conflict` on mismatch).
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
                write_local_atomic(&dest, new_bytes).await?;
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
                mutate_local_fs_in_roots(root, additional_roots, *is_root_user, op, path, dest_path)
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

    /// Execute a bounded command (`program` + `args`) on `target`.
    pub async fn exec_command(
        &self,
        target: &ResolvedTarget,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<ExecOutput> {
        let mut cmd = self.build_piped_command(target, program, args, cwd, env)?;
        cmd.stdin(Stdio::null());
        run_command_bounded(cmd, timeout).await
    }

    /// Execute a bounded POSIX `sh -c <script>` command on `target`.
    pub async fn exec_shell(
        &self,
        target: &ResolvedTarget,
        script: &str,
        cwd: Option<&str>,
        env: &BTreeMap<String, String>,
        timeout: Duration,
    ) -> Result<ExecOutput> {
        let args = vec!["-c".to_string(), script.to_string()];
        self.exec_command(target, "sh", &args, cwd, env, timeout)
            .await
    }

    /// Build a `tokio::process::Command` with piped `stdin`/`stdout`/`stderr`
    /// and `kill_on_drop(true)` targeting either local OS or multiplexed SSH.
    pub fn build_piped_command(
        &self,
        target: &ResolvedTarget,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &BTreeMap<String, String>,
    ) -> Result<Command> {
        if program.trim().is_empty() || program.chars().any(char::is_control) {
            return Err(Error::Invalid("invalid command program name".into()));
        }
        for k in env.keys() {
            if !valid_env_key(k) {
                return Err(Error::Invalid(format!(
                    "invalid environment variable name '{k}'"
                )));
            }
        }
        match target {
            ResolvedTarget::Local {
                root,
                additional_roots,
                is_root_user,
                ..
            } => {
                let work_dir = match cwd.filter(|c| !c.trim().is_empty()) {
                    Some(c) => {
                        resolve_local_existing_in_roots(root, additional_roots, *is_root_user, c)?.0
                    }
                    None => root.clone(),
                };
                let mut cmd = Command::new(program);
                cmd.args(args)
                    .current_dir(work_dir)
                    .envs(env)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true);
                Ok(cmd)
            }
            ResolvedTarget::Ssh {
                spec,
                root,
                actor_id,
                ..
            } => {
                validate_ssh_spec(spec)?;
                let ctl = self.acquire_ssh_socket(actor_id, spec)?;
                let remote_cwd = cwd
                    .filter(|c| !c.trim().is_empty())
                    .unwrap_or(root.as_str());
                let remote_cmd = build_remote_exec_line(remote_cwd, program, args, env);
                let mut ssh_args = build_ssh_base_args(spec, &ctl.ctl_path);
                ssh_args.push("--".into());
                ssh_args.push(ctl.ssh_target.clone());
                ssh_args.push(remote_cmd);

                let mut cmd = Command::new("ssh");
                cmd.args(ssh_args)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .kill_on_drop(true);
                Ok(cmd)
            }
        }
    }

    /// Spawn a long-lived piped child process on either local OS or multiplexed SSH.
    pub async fn spawn_piped(
        &self,
        target: &ResolvedTarget,
        program: &str,
        args: &[String],
        cwd: Option<&str>,
        env: &BTreeMap<String, String>,
    ) -> Result<Child> {
        let mut cmd = self.build_piped_command(target, program, args, cwd, env)?;
        cmd.spawn()
            .map_err(|e| Error::Upstream(format!("failed to spawn '{program}': {e}")))
    }

    /// Open a local port forward (`ssh -L`) to `remote_host:remote_port` on an
    /// SSH target (used by TCP-mode DAP adapters like `dlv` or JDWP).
    pub async fn open_ssh_tunnel(
        &self,
        spec: &IdeSshSpec,
        remote_host: &str,
        remote_port: u16,
    ) -> Result<SshTunnel> {
        validate_ssh_spec(spec)?;
        let cfg = SshTunnelConfig {
            host: spec.host.clone(),
            port: spec.port,
            user: spec.user.clone(),
            identity_file: spec.identity_file.clone(),
            proxy_jump: spec.proxy_jump.clone(),
        };
        SshTunnel::open(&cfg, remote_host, remote_port).await
    }

    fn acquire_sftp(&self, actor_id: &str, spec: &IdeSshSpec) -> Result<Arc<SftpSession>> {
        validate_ssh_spec(spec)?;
        let key = SshPoolKey {
            actor_id: actor_id.to_string(),
            connection_id: spec.connection_id.to_string(),
            params: SftpParams {
                host: spec.host.clone(),
                port: Some(spec.port),
                user: (!spec.user.is_empty()).then(|| spec.user.clone()),
                identity_file: spec.identity_file.clone(),
                jump: spec.proxy_jump.clone(),
            },
        };
        if let Some(mut entry) = self.sftp_pool.get_mut(&key) {
            entry.1 = Instant::now();
            return Ok(entry.0.clone());
        }
        self.sftp_pool.retain(|_, (sess, touched)| {
            touched.elapsed() < SFTP_IDLE_TTL || Arc::strong_count(sess) > 1
        });
        let session = Arc::new(SftpSession::new(key.params.clone())?);
        self.sftp_pool
            .insert(key, (session.clone(), Instant::now()));
        Ok(session)
    }

    fn acquire_ssh_socket(
        &self,
        actor_id: &str,
        spec: &IdeSshSpec,
    ) -> Result<Arc<SshControlSocket>> {
        let key = SshPoolKey {
            actor_id: actor_id.to_string(),
            connection_id: spec.connection_id.to_string(),
            params: SftpParams {
                host: spec.host.clone(),
                port: Some(spec.port),
                user: (!spec.user.is_empty()).then(|| spec.user.clone()),
                identity_file: spec.identity_file.clone(),
                jump: spec.proxy_jump.clone(),
            },
        };
        if let Some(existing) = self.ssh_sockets.get(&key) {
            return Ok(existing.clone());
        }
        let sock = Arc::new(SshControlSocket::new(spec)?);
        self.ssh_sockets.insert(key, sock.clone());
        Ok(sock)
    }
}

// ---------------------------------------------------------------------------
// Validation & Path Resolution Helpers
// ---------------------------------------------------------------------------

fn canonicalize_local_root_dir(raw_root: &str, is_root_user: bool) -> Result<PathBuf> {
    let decoded = strip_file_uri(raw_root);
    let expanded = otto_core::paths::expand_home_strict(&decoded).map_err(Error::Invalid)?;
    let canonical = expanded
        .canonicalize()
        .map_err(|e| map_io_err("resolve workspace root", &expanded, e))?;
    let meta = std::fs::metadata(&canonical)
        .map_err(|e| map_io_err("stat workspace root", &canonical, e))?;
    if !meta.is_dir() {
        return Err(Error::Invalid(format!(
            "target root '{}' is not a directory",
            canonical.display()
        )));
    }
    if otto_core::secret_paths::is_denied_dir(&canonical) {
        return Err(Error::Forbidden(format!(
            "{} holds protected credentials; access denied",
            canonical.display()
        )));
    }
    if !is_root_user {
        enforce_non_root_path(&canonical)?;
    }
    Ok(canonical)
}

fn validate_ssh_spec(spec: &IdeSshSpec) -> Result<()> {
    if !valid_ssh_host(&spec.host) {
        return Err(Error::Invalid(format!(
            "SSH host '{}' is not a valid hostname or IP",
            spec.host
        )));
    }
    if !valid_ssh_user(&spec.user) {
        return Err(Error::Invalid(format!(
            "SSH user '{}' is not a valid username",
            spec.user
        )));
    }
    if let Some(jump) = spec.proxy_jump.as_deref().filter(|s| !s.trim().is_empty()) {
        if !valid_proxy_jump(jump) {
            return Err(Error::Invalid(format!(
                "SSH proxy_jump '{jump}' is invalid"
            )));
        }
    }
    Ok(())
}

fn valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn build_ssh_base_args(spec: &IdeSshSpec, ctl_path: &str) -> Vec<String> {
    let mut args = vec![
        "-T".into(),
        "-o".into(),
        "BatchMode=yes".into(),
        "-o".into(),
        "StrictHostKeyChecking=accept-new".into(),
        "-o".into(),
        "ConnectTimeout=12".into(),
        "-o".into(),
        "ControlMaster=auto".into(),
        "-o".into(),
        format!("ControlPath={ctl_path}"),
        "-o".into(),
        "ControlPersist=60s".into(),
        "-o".into(),
        "ServerAliveInterval=15".into(),
        "-o".into(),
        "ServerAliveCountMax=2".into(),
        "-p".into(),
        spec.port.to_string(),
    ];
    if let Some(id_file) = spec.identity_file.as_deref().filter(|s| !s.is_empty()) {
        args.push("-i".into());
        args.push(id_file.to_string());
    }
    if let Some(jump) = spec.proxy_jump.as_deref().filter(|s| !s.trim().is_empty()) {
        args.push("-J".into());
        args.push(jump.to_string());
    }
    args
}

fn build_remote_exec_line(
    cwd: &str,
    program: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
) -> String {
    let mut parts = Vec::new();
    parts.push(format!("cd {} &&", shell_words::quote(cwd)));
    if !env.is_empty() {
        parts.push("env".into());
        for (k, v) in env {
            parts.push(format!("{k}={}", shell_words::quote(v)));
        }
    }
    parts.push("exec".into());
    parts.push(shell_words::quote(program).into_owned());
    for arg in args {
        parts.push(shell_words::quote(arg).into_owned());
    }
    parts.join(" ")
}

async fn run_command_bounded(mut cmd: Command, timeout: Duration) -> Result<ExecOutput> {
    let child = cmd
        .spawn()
        .map_err(|e| Error::Upstream(format!("failed to start process: {e}")))?;
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => Ok(ExecOutput {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        }),
        Ok(Err(e)) => Err(Error::Upstream(format!("process wait failed: {e}"))),
        Err(_) => Err(Error::Upstream(format!(
            "command timed out after {}s",
            timeout.as_secs()
        ))),
    }
}

pub(super) async fn resolve_local_prospective_in_roots(
    root: &Path,
    additional_roots: &[PathBuf],
    is_root_user: bool,
    raw: &str,
    create_dirs: bool,
) -> Result<(PathBuf, PathBuf)> {
    let decoded = strip_file_uri(raw);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid("file path must not be empty".into()));
    }
    let abs = if trimmed.starts_with('~') {
        otto_core::paths::expand_home_strict(trimmed).map_err(Error::Invalid)?
    } else {
        let p = Path::new(trimmed);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        }
    };
    let mut prospective = otto_core::secret_paths::canonicalize_prospective(&abs)
        .ok_or_else(|| Error::Invalid(format!("invalid target path '{}'", abs.display())))?;

    let matched_root = std::iter::once(root)
        .chain(additional_roots.iter().map(PathBuf::as_path))
        .find(|r| prospective.starts_with(r))
        .ok_or_else(|| {
            Error::Forbidden(format!(
                "path '{}' escapes workspace root '{}'",
                prospective.display(),
                root.display()
            ))
        })?
        .to_path_buf();

    if !is_root_user {
        enforce_non_root_path(&prospective)?;
    }
    if create_dirs {
        if let Some(parent) = prospective.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| map_io_err("create parent directories", parent, e))?;
            if let Ok(canon_parent) = parent.canonicalize() {
                if !canon_parent.starts_with(&matched_root) {
                    return Err(Error::Forbidden(
                        "parent directory escapes workspace root".into(),
                    ));
                }
                if !is_root_user {
                    enforce_non_root_path(&canon_parent)?;
                }
                if let Some(name) = prospective.file_name() {
                    prospective = canon_parent.join(name);
                }
            }
        }
    }
    Ok((prospective, matched_root))
}

fn normalize_remote_root(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(Error::Invalid("invalid remote root path".into()));
    }
    if !trimmed.starts_with('/') && !trimmed.starts_with('~') {
        return Err(Error::Invalid(
            "remote SSH root path must start with '/' or '~'".into(),
        ));
    }
    let clean = trimmed.trim_end_matches('/');
    Ok(if clean.is_empty() {
        "/".into()
    } else {
        clean.into()
    })
}

pub(super) fn resolve_remote_subpath(root: &str, raw: &str) -> Result<String> {
    let (p, _) = resolve_remote_in_roots(root, &[], raw)?;
    Ok(p)
}

fn resolve_remote_in_roots(
    root: &str,
    additional_roots: &[String],
    raw: &str,
) -> Result<(String, String)> {
    let decoded = strip_file_uri(raw);
    if decoded.chars().any(char::is_control) {
        return Err(Error::Invalid(
            "remote path contains control characters".into(),
        ));
    }
    let trimmed = decoded.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Ok((root.to_string(), root.to_string()));
    }
    let combined = if trimmed.starts_with('/') || trimmed.starts_with('~') {
        trimmed.to_string()
    } else {
        join_remote(root, trimmed)
    };
    let normalized = lexical_normalize_remote(&combined)?;
    for candidate_root in std::iter::once(root).chain(additional_roots.iter().map(String::as_str)) {
        if candidate_root == "/"
            || normalized == candidate_root
            || normalized.starts_with(&format!("{candidate_root}/"))
        {
            return Ok((normalized, candidate_root.to_string()));
        }
    }
    Err(Error::Forbidden(format!(
        "remote path '{normalized}' is outside workspace root '{root}'"
    )))
}

fn resolve_remote_in_roots_or_sdk(
    root: &str,
    additional_roots: &[String],
    raw: &str,
) -> Result<(String, Option<String>)> {
    if let Ok((p, r)) = resolve_remote_in_roots(root, additional_roots, raw) {
        return Ok((p, Some(r)));
    }
    let decoded = strip_file_uri(raw);
    let normalized = lexical_normalize_remote(decoded.trim())?;
    let is_remote_sdk = [
        "/usr/local/go/",
        "/usr/lib/go",
        "/usr/share/go",
        "/usr/lib/python",
        "/usr/lib/jvm/",
        "/.otto/toolchains/",
        "/go/pkg/mod/",
        "/.cargo/registry/",
        "/.rustup/toolchains/",
    ]
    .iter()
    .any(|prefix| normalized.contains(prefix));
    if is_remote_sdk {
        return Ok((normalized, None));
    }
    Err(Error::Forbidden(format!(
        "remote path '{normalized}' is outside workspace root '{root}'"
    )))
}

fn lexical_normalize_remote(path: &str) -> Result<String> {
    let is_tilde = path.starts_with('~');
    let mut stack: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if stack.is_empty() || (is_tilde && stack.len() == 1) {
                    return Err(Error::Forbidden("path traversal above root refused".into()));
                }
                stack.pop();
            }
            other => stack.push(other),
        }
    }
    if is_tilde {
        Ok(stack.join("/"))
    } else {
        Ok(format!("/{}", stack.join("/")))
    }
}

pub(super) fn join_remote(base: &str, rel: &str) -> String {
    let b = base.trim_end_matches('/');
    let r = rel.trim_start_matches('/');
    if b.is_empty() {
        format!("/{r}")
    } else {
        format!("{b}/{r}")
    }
}

pub(super) fn remote_parent(path: &str) -> Option<String> {
    let clean = path.trim_end_matches('/');
    if clean.is_empty() || clean == "/" || clean == "~" {
        return None;
    }
    match clean.rsplit_once('/') {
        Some(("", _)) => Some("/".into()),
        Some((parent, _)) => Some(parent.into()),
        None => None,
    }
}

pub(super) fn map_io_err(action: &str, path: &Path, err: std::io::Error) -> Error {
    let msg = format!("cannot {action} '{}': {err}", path.display());
    match err.kind() {
        std::io::ErrorKind::NotFound => Error::NotFound(msg),
        std::io::ErrorKind::PermissionDenied => Error::Forbidden(msg),
        std::io::ErrorKind::AlreadyExists => Error::Conflict(msg),
        _ => Error::Invalid(msg),
    }
}

#[cfg(test)]
mod tests;
