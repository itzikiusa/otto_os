//! Local and remote filesystem tree mutation (`mkdir`, `create_file`, `rename`,
//! `delete`, `duplicate`) and Local History capture options.

use std::path::{Path, PathBuf};

use otto_core::{Error, Result};
use otto_ssh::SftpSession;

use crate::local_history::LocalHistoryStore;
use crate::lsp_host::pool::LspPool;
use crate::types::{IdeFsMutateOp, IdeFsMutateResp};

use super::fs_ops::{
    is_remote_sftp_not_found, resolve_local_existing_in_roots, write_local_atomic,
};
use super::{
    map_io_err, remote_parent, resolve_local_prospective_in_roots, resolve_remote_in_roots,
};

/// Per-call Local History capture policy forwarded from IDE preferences.
#[derive(Debug, Clone, Copy)]
pub struct HistoryWriteOpts {
    pub enabled: bool,
    pub retention_days: Option<u32>,
}

impl Default for HistoryWriteOpts {
    fn default() -> Self {
        Self {
            enabled: true,
            retention_days: None,
        }
    }
}

fn is_protected_root(target_path: &Path, root: &Path, additional_roots: &[PathBuf]) -> bool {
    target_path == root || additional_roots.iter().any(|r| r == target_path)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn mutate_local_fs_in_roots(
    root: &Path,
    additional_roots: &[PathBuf],
    is_root_user: bool,
    op: IdeFsMutateOp,
    path: &str,
    dest_path: Option<&str>,
    history: Option<&LocalHistoryStore>,
    history_opts: HistoryWriteOpts,
    lsp_pool: Option<&LspPool>,
) -> Result<IdeFsMutateResp> {
    let active_history = if history_opts.enabled { history } else { None };
    let retention_days = history_opts.retention_days;

    match op {
        IdeFsMutateOp::Mkdir => {
            let (dir, _) = resolve_local_prospective_in_roots(
                root,
                additional_roots,
                is_root_user,
                path,
                true,
            )
            .await?;
            tokio::fs::create_dir_all(&dir)
                .await
                .map_err(|e| map_io_err("create directory", &dir, e))?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: dir.to_string_lossy().into_owned(),
            })
        }
        IdeFsMutateOp::CreateFile => {
            let (file, matched_root) = resolve_local_prospective_in_roots(
                root,
                additional_roots,
                is_root_user,
                path,
                true,
            )
            .await?;
            if tokio::fs::metadata(&file).await.is_ok() {
                return Err(Error::Conflict(format!(
                    "'{}' already exists",
                    file.display()
                )));
            }
            let file_for_tpl = file.clone();
            let initial_bytes =
                tokio::task::spawn_blocking(move || default_new_file_template(&file_for_tpl))
                    .await
                    .unwrap_or_default();
            write_local_atomic(&file, initial_bytes.as_bytes(), lsp_pool).await?;
            if let Some(hist) = active_history {
                let hist_clone = hist.clone();
                let file_clone = file.clone();
                let bytes_vec = initial_bytes.into_bytes();
                let _ = tokio::task::spawn_blocking(move || {
                    let _ = hist_clone.record_bytes_with_retention_sync(
                        &matched_root,
                        &file_clone,
                        &bytes_vec,
                        "created",
                        "Created",
                        false,
                        retention_days,
                    );
                })
                .await;
            }
            if let Some(pool) = lsp_pool {
                pool.notify_watched_file_if_build_manifest(&file, 1);
            }
            Ok(IdeFsMutateResp {
                ok: true,
                path: file.to_string_lossy().into_owned(),
            })
        }
        IdeFsMutateOp::Rename => {
            let dst_raw = dest_path
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| Error::Invalid("dest_path is required for rename".into()))?;
            let (src, _) =
                resolve_local_existing_in_roots(root, additional_roots, is_root_user, path)?;
            if is_protected_root(&src, root, additional_roots) {
                return Err(Error::Forbidden("cannot rename workspace root".into()));
            }
            let (dst, matched_root) = resolve_local_prospective_in_roots(
                root,
                additional_roots,
                is_root_user,
                dst_raw,
                true,
            )
            .await?;
            tokio::fs::rename(&src, &dst)
                .await
                .map_err(|e| map_io_err("rename path", &src, e))?;
            if let Some(hist) = active_history {
                let hist_clone = hist.clone();
                let dst_clone = dst.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    if dst_clone.is_file() {
                        let _ = hist_clone.record_existing_file_with_retention_sync(
                            &matched_root,
                            &dst_clone,
                            "renamed",
                            "Renamed",
                            false,
                            retention_days,
                        );
                    }
                })
                .await;
            }
            if let Some(pool) = lsp_pool {
                pool.notify_watched_file_if_build_manifest(&src, 3);
                pool.notify_watched_file_if_build_manifest(&dst, 1);
            }
            Ok(IdeFsMutateResp {
                ok: true,
                path: dst.to_string_lossy().into_owned(),
            })
        }
        IdeFsMutateOp::Delete => {
            let (target_path, matched_root) =
                resolve_local_existing_in_roots(root, additional_roots, is_root_user, path)?;
            if is_protected_root(&target_path, root, additional_roots) {
                return Err(Error::Forbidden("cannot delete workspace root".into()));
            }
            let meta = tokio::fs::symlink_metadata(&target_path)
                .await
                .map_err(|e| map_io_err("stat path for delete", &target_path, e))?;
            let is_symlink = meta.file_type().is_symlink();
            let is_real_dir = !is_symlink && meta.is_dir();

            if let Some(hist) = active_history {
                let target_buf = target_path.clone();
                let hist_clone = hist.clone();
                let extra_roots = additional_roots.to_vec();
                let _ = tokio::task::spawn_blocking(move || {
                    if is_real_dir {
                        let _ = hist_clone.record_deleted_dir_with_retention_sync(
                            &matched_root,
                            &extra_roots,
                            &target_buf,
                            is_root_user,
                            retention_days,
                        );
                    } else if !is_symlink {
                        let _ = hist_clone.record_existing_file_with_retention_sync(
                            &matched_root,
                            &target_buf,
                            "deleted",
                            "Deleted",
                            false,
                            retention_days,
                        );
                    }
                })
                .await;
            }
            if is_real_dir {
                tokio::fs::remove_dir_all(&target_path)
                    .await
                    .map_err(|e| map_io_err("delete directory", &target_path, e))?;
            } else {
                tokio::fs::remove_file(&target_path)
                    .await
                    .map_err(|e| map_io_err("delete file", &target_path, e))?;
                if let Some(pool) = lsp_pool {
                    pool.notify_watched_file_if_build_manifest(&target_path, 3);
                }
            }
            Ok(IdeFsMutateResp {
                ok: true,
                path: target_path.to_string_lossy().into_owned(),
            })
        }
        IdeFsMutateOp::Duplicate => {
            let dst_raw = dest_path
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| Error::Invalid("dest_path is required for duplicate".into()))?;
            let (src, _) =
                resolve_local_existing_in_roots(root, additional_roots, is_root_user, path)?;
            let (dst, _) = resolve_local_prospective_in_roots(
                root,
                additional_roots,
                is_root_user,
                dst_raw,
                true,
            )
            .await?;
            if tokio::fs::metadata(&dst).await.is_ok() {
                return Err(Error::Conflict(format!(
                    "'{}' already exists",
                    dst.display()
                )));
            }
            tokio::fs::copy(&src, &dst)
                .await
                .map_err(|e| map_io_err("duplicate file", &src, e))?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: dst.to_string_lossy().into_owned(),
            })
        }
    }
}

/// Infer an idiomatic `package <name>\n` header when creating a new `.go` file
/// (matching sibling `.go` files in the same directory or directory name).
#[allow(clippy::disallowed_methods)]
fn default_new_file_template(file: &Path) -> String {
    let ext = file.extension().and_then(|e| e.to_str()).unwrap_or("");
    if !ext.eq_ignore_ascii_case("go") {
        return String::new();
    }
    let Some(dir) = file.parent() else {
        return "package main\n".into();
    };
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten().take(16) {
            let p = entry.path();
            if p != file
                && p.extension().and_then(|e| e.to_str()) == Some("go")
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("_test.go"))
            {
                if let Ok(src) = std::fs::read_to_string(&p) {
                    for line in src.lines().take(20) {
                        let t = line.trim();
                        if let Some(rest) = t.strip_prefix("package ") {
                            let pkg = rest.split_whitespace().next().unwrap_or("");
                            if !pkg.is_empty() {
                                return format!("package {pkg}\n\n");
                            }
                        }
                    }
                }
            }
        }
    }
    let dir_name = dir.file_name().and_then(|n| n.to_str()).unwrap_or("main");
    let pkg: String =
        if matches!(dir_name, "cmd" | "main" | "server" | "cli") || dir.join("go.mod").is_file() {
            "main".into()
        } else {
            let cleaned: String = dir_name
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .map(|c| c.to_ascii_lowercase())
                .collect();
            if cleaned.is_empty() || cleaned.chars().next().is_some_and(|c| c.is_ascii_digit()) {
                "main".into()
            } else {
                cleaned
            }
        };
    format!("package {pkg}\n\n")
}

fn is_protected_remote_root(target: &str, root: &str, additional_roots: &[String]) -> bool {
    target == root || additional_roots.iter().any(|r| r == target)
}

async fn ensure_remote_path_absent(sftp: &SftpSession, remote_path: &str) -> Result<()> {
    if let (Some(parent), Some(leaf)) = (
        remote_parent(remote_path),
        remote_path.trim_end_matches('/').rsplit('/').next(),
    ) {
        match sftp.list_capped(&parent, 20_000).await {
            Ok((entries, truncated)) => {
                if entries.iter().any(|e| e.name == leaf) {
                    return Err(Error::Conflict(format!("'{remote_path}' already exists")));
                }
                if !truncated {
                    return Ok(());
                }
            }
            Err(err) if is_remote_sftp_not_found(&err) => return Ok(()),
            Err(err) => return Err(err),
        }
    }
    if sftp.file_size(remote_path).await.is_ok() {
        return Err(Error::Conflict(format!("'{remote_path}' already exists")));
    }
    Ok(())
}

pub(super) async fn mutate_remote_fs(
    sftp: &SftpSession,
    root: &str,
    additional_roots: &[String],
    op: IdeFsMutateOp,
    path: &str,
    dest_path: Option<&str>,
) -> Result<IdeFsMutateResp> {
    let (remote, _) = resolve_remote_in_roots(root, additional_roots, path)?;
    match op {
        IdeFsMutateOp::Mkdir => {
            sftp.mkdir(&remote).await?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: remote,
            })
        }
        IdeFsMutateOp::CreateFile => {
            ensure_remote_path_absent(sftp, &remote).await?;
            let tmp = tempfile::NamedTempFile::new()
                .map_err(|e| Error::Internal(format!("temp file: {e}")))?;
            let tmp_str = tmp.path().to_string_lossy().into_owned();
            sftp.upload(&tmp_str, &remote).await?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: remote,
            })
        }
        IdeFsMutateOp::Rename => {
            if is_protected_remote_root(&remote, root, additional_roots) {
                return Err(Error::Forbidden("cannot rename workspace root".into()));
            }
            let dst_raw = dest_path
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| Error::Invalid("dest_path is required for rename".into()))?;
            let (dst, _) = resolve_remote_in_roots(root, additional_roots, dst_raw)?;
            sftp.rename(&remote, &dst).await?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: dst,
            })
        }
        IdeFsMutateOp::Delete => {
            if is_protected_remote_root(&remote, root, additional_roots) {
                return Err(Error::Forbidden("cannot delete workspace root".into()));
            }
            if sftp.remove(&remote).await.is_err() {
                sftp.rmdir(&remote).await?;
            }
            Ok(IdeFsMutateResp {
                ok: true,
                path: remote,
            })
        }
        IdeFsMutateOp::Duplicate => {
            let dst_raw = dest_path
                .filter(|s| !s.trim().is_empty())
                .ok_or_else(|| Error::Invalid("dest_path is required for duplicate".into()))?;
            let (dst, _) = resolve_remote_in_roots(root, additional_roots, dst_raw)?;
            ensure_remote_path_absent(sftp, &dst).await?;
            let tmp = tempfile::NamedTempFile::new()
                .map_err(|e| Error::Internal(format!("temp duplicate file: {e}")))?;
            let tmp_str = tmp.path().to_string_lossy().into_owned();
            sftp.download(&remote, &tmp_str).await?;
            sftp.upload(&tmp_str, &dst).await?;
            Ok(IdeFsMutateResp {
                ok: true,
                path: dst,
            })
        }
    }
}
