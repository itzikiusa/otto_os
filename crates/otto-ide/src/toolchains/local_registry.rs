//! Persistence & path validation for user-registered local SDKs (`ide_local_sdks`).

use std::collections::HashMap;
use std::path::Path;

use otto_core::{Error, Result};
use otto_state::DbPool;
use sqlx::Row;

use crate::toolchains_catalog::EcosystemKind;
use crate::types::{IdeInstalledSdk, IdeTargetKind, IdeTargetSpec};

use super::discovery::{
    dir_name_version, is_executable_file, python_dir_version, read_go_root_version,
    read_java_release_version, resolve_java_home_dir, rust_toolchain_label,
};

pub(super) fn host_key_for_target(target: &IdeTargetSpec) -> String {
    match target.kind {
        IdeTargetKind::Local => "local".into(),
        IdeTargetKind::Ssh => format!(
            "ssh:{}",
            target.connection_id.as_deref().unwrap_or("default")
        ),
    }
}

pub(crate) async fn load_registered_local_sdks_by_host(
    pool: &DbPool,
    host_key: &str,
) -> HashMap<EcosystemKind, Vec<IdeInstalledSdk>> {
    let rows = match sqlx::query(
        r#"SELECT id, ecosystem, version, path
           FROM ide_local_sdks
           WHERE host_key = ?
           ORDER BY created_at DESC"#,
    )
    .bind(host_key)
    .fetch_all(pool)
    .await
    {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(error = %err, host_key, "failed to load ide_local_sdks");
            return HashMap::new();
        }
    };

    let mut out: HashMap<EcosystemKind, Vec<IdeInstalledSdk>> = HashMap::new();
    for row in rows {
        let eco_s: String = row.try_get("ecosystem").unwrap_or_default();
        let Some(eco) = EcosystemKind::parse(&eco_s) else {
            continue;
        };
        let path: String = row.try_get("path").unwrap_or_default();
        if path.is_empty() {
            continue;
        }
        if host_key == "local" && !Path::new(&path).exists() {
            continue;
        }
        let id: String = row.try_get("id").unwrap_or_default();
        let version: String = row.try_get("version").unwrap_or_else(|_| "custom".into());
        out.entry(eco).or_default().push(IdeInstalledSdk {
            id,
            version,
            path,
            source: "local".into(),
            active: false,
        });
    }
    out
}

pub(crate) async fn load_all_local_host_sdks(pool: &DbPool) -> Vec<IdeInstalledSdk> {
    load_registered_local_sdks_by_host(pool, "local")
        .await
        .into_values()
        .flatten()
        .collect()
}

pub(super) fn normalize_and_validate_local_sdk(
    target: &IdeTargetSpec,
    eco: EcosystemKind,
    raw_path: &str,
) -> Result<(String, String)> {
    let trimmed = raw_path.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(Error::Invalid("SDK path must not be empty".into()));
    }
    if !trimmed.starts_with('/') {
        return Err(Error::Invalid("SDK path must be an absolute path".into()));
    }

    if target.kind == IdeTargetKind::Ssh {
        let clean = trimmed.trim_end_matches('/').to_string();
        let ver = dir_name_version(Path::new(&clean));
        return Ok((clean, ver));
    }

    let canon = std::fs::canonicalize(trimmed)
        .map_err(|e| Error::Invalid(format!("SDK path '{trimmed}' does not exist: {e}")))?;
    let canon_str = canon.to_string_lossy();
    if canon_str == "/" || canon_str == "/etc" || canon_str == "/private/etc" {
        return Err(Error::Forbidden(
            "refusing to register root or system configuration directory as SDK".into(),
        ));
    }
    crate::target::enforce_non_root_path(&canon)?;

    match eco {
        EcosystemKind::Java => {
            let jh = if canon.is_file() && is_executable_file(&canon) {
                canon
                    .parent()
                    .and_then(|b| b.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or(canon)
            } else {
                resolve_java_home_dir(&canon).ok_or_else(|| {
                    Error::Invalid(format!(
                        "'{}' does not contain a valid JDK (expected bin/java or Contents/Home/bin/java)",
                        canon.display()
                    ))
                })?
            };
            let ver = read_java_release_version(&jh).unwrap_or_else(|| dir_name_version(&jh));
            Ok((jh.to_string_lossy().into_owned(), ver))
        }
        EcosystemKind::Go => {
            let gr = if canon.is_file() && is_executable_file(&canon) {
                canon
                    .parent()
                    .and_then(|b| b.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or(canon)
            } else if canon.join("go/bin/go").is_file() {
                canon.join("go")
            } else {
                canon
            };
            if !gr.join("bin/go").is_file() {
                return Err(Error::Invalid(format!(
                    "'{}' does not contain bin/go",
                    gr.display()
                )));
            }
            let ver = read_go_root_version(&gr).unwrap_or_else(|| dir_name_version(&gr));
            Ok((gr.to_string_lossy().into_owned(), ver))
        }
        EcosystemKind::Rust => {
            let root = if canon.is_file() && is_executable_file(&canon) {
                canon
                    .parent()
                    .and_then(|b| b.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or(canon)
            } else {
                canon
            };
            if !root.join("bin/rustc").is_file() && !root.join("bin/cargo").is_file() {
                return Err(Error::Invalid(format!(
                    "'{}' does not contain bin/rustc or bin/cargo",
                    root.display()
                )));
            }
            let ver = rust_toolchain_label(&root);
            Ok((root.to_string_lossy().into_owned(), ver))
        }
        EcosystemKind::Node => {
            let root = if canon.is_file() && is_executable_file(&canon) {
                canon
                    .parent()
                    .and_then(|b| b.parent())
                    .map(Path::to_path_buf)
                    .unwrap_or(canon)
            } else {
                canon
            };
            if !root.join("bin/node").is_file() && !is_executable_file(&root) {
                return Err(Error::Invalid(format!(
                    "'{}' does not contain bin/node",
                    root.display()
                )));
            }
            let ver = dir_name_version(&root);
            Ok((root.to_string_lossy().into_owned(), ver))
        }
        EcosystemKind::Python => {
            let interp = if canon.is_file() && is_executable_file(&canon) {
                canon
            } else if canon.join("bin/python3").is_file() {
                canon.join("bin/python3")
            } else if canon.join("bin/python").is_file() {
                canon.join("bin/python")
            } else {
                return Err(Error::Invalid(format!(
                    "'{}' is neither a python executable nor a virtualenv/SDK with bin/python3",
                    canon.display()
                )));
            };
            let ver = interp
                .parent()
                .and_then(|b| b.parent())
                .map(python_dir_version)
                .unwrap_or_else(|| "custom".into());
            Ok((interp.to_string_lossy().into_owned(), ver))
        }
        EcosystemKind::Shell | EcosystemKind::Json => {
            if !canon.is_dir() && !is_executable_file(&canon) {
                return Err(Error::Invalid(format!(
                    "'{}' must be an executable binary or tool directory",
                    canon.display()
                )));
            }
            let ver = dir_name_version(&canon);
            Ok((canon.to_string_lossy().into_owned(), ver))
        }
    }
}
