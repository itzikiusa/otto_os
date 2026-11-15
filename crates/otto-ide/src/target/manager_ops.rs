//! High-level workspace filesystem and platform detection methods on [`TargetManager`].

#[path = "manager_search.rs"]
mod manager_search;

use std::collections::BTreeMap;
use std::time::Duration;

use otto_core::{Error, Result};

use crate::types::{
    IdeFileReadResp, IdeFileWriteResp, IdeFsMutateOp, IdeFsMutateResp, IdeFsTreeResp,
    IdePlatformInfo,
};

use super::fs_ops::{
    build_file_read_resp, build_remote_tree_entries, list_local_dir_sync, mutate_local_fs_in_roots,
    mutate_remote_fs, normalize_arch_name, normalize_os_name, parse_uname_output,
    read_bounded_bytes, resolve_local_existing_in_roots, sha256_hex, verify_local_expected_sha,
    verify_remote_expected_sha, write_local_atomic, HistoryWriteOpts,
};
use super::sdk_paths::{is_virtual_archive_uri, read_virtual_java_archive, resolve_local_readable};
use super::{
    map_io_err, remote_parent, resolve_local_prospective_in_roots, resolve_remote_in_roots,
    resolve_remote_in_roots_or_sdk, ResolvedTarget, TargetManager, MAX_FILE_READ_BYTES,
};

impl TargetManager {
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
                actor_id,
                ..
            } => {
                let (resolved_dir, matched_root) = resolve_local_existing_in_roots(
                    root,
                    additional_roots,
                    *is_root_user,
                    path.unwrap_or(""),
                )?;
                if path.unwrap_or("").trim().is_empty() {
                    if let Some(pool) = &self.lsp_pool {
                        pool.evict_detached_other_roots_for_user(
                            actor_id,
                            &matched_root,
                            root,
                            additional_roots,
                        );
                    }
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
                ..
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
        self.write_file_with_history(
            target,
            path,
            content,
            expected_sha256,
            create_dirs,
            HistoryWriteOpts::default(),
        )
        .await
    }

    /// Atomically write a UTF-8 file honoring explicit [`HistoryWriteOpts`] preferences.
    pub async fn write_file_with_history(
        &self,
        target: &ResolvedTarget,
        path: &str,
        content: &str,
        expected_sha256: Option<&str>,
        create_dirs: bool,
        history_opts: HistoryWriteOpts,
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
                let (dest, matched_root) = resolve_local_prospective_in_roots(
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
                if history_opts.enabled {
                    if let Some(hist) = &self.history {
                        let hist_clone = hist.clone();
                        let mr = matched_root.clone();
                        let dest_clone = dest.clone();
                        let ret = history_opts.retention_days;
                        let _ = tokio::task::spawn_blocking(move || {
                            if dest_clone.is_file() {
                                let _ = hist_clone.record_existing_file_with_retention_sync(
                                    &mr,
                                    &dest_clone,
                                    "external",
                                    "Before edit",
                                    true,
                                    ret,
                                );
                            }
                        })
                        .await;
                    }
                }
                write_local_atomic(&dest, new_bytes, self.lsp_pool.as_deref()).await?;
                if matches!(
                    dest.file_name().and_then(|n| n.to_str()),
                    Some("go.mod" | "go.work")
                ) {
                    self.invalidate_local_resolve_cache();
                }
                if history_opts.enabled {
                    if let Some(hist) = &self.history {
                        let hist_clone = hist.clone();
                        let mr = matched_root.clone();
                        let dest_clone = dest.clone();
                        let saved_bytes = new_bytes.to_vec();
                        let ret = history_opts.retention_days;
                        let _ = tokio::task::spawn_blocking(move || {
                            let _ = hist_clone.record_bytes_with_retention_sync(
                                &mr,
                                &dest_clone,
                                &saved_bytes,
                                "saved",
                                "Saved",
                                true,
                                ret,
                            );
                        })
                        .await;
                    }
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
        self.mutate_fs_with_history(target, op, path, dest_path, HistoryWriteOpts::default())
            .await
    }

    /// Perform filesystem tree mutations honoring explicit [`HistoryWriteOpts`].
    pub async fn mutate_fs_with_history(
        &self,
        target: &ResolvedTarget,
        op: IdeFsMutateOp,
        path: &str,
        dest_path: Option<&str>,
        history_opts: HistoryWriteOpts,
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
                    history_opts,
                    self.lsp_pool.as_deref(),
                )
                .await
            }
            ResolvedTarget::Ssh {
                spec,
                root,
                additional_roots,
                actor_id,
            } => {
                let sftp = self.acquire_sftp(actor_id, spec)?;
                mutate_remote_fs(&sftp, root, additional_roots, op, path, dest_path).await
            }
        }
    }
}
