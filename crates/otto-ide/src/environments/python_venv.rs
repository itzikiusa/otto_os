//! Python virtualenv discovery and creation methods for [`super::EnvironmentManager`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use otto_core::{Error, Result};
use otto_state::DbPool;
use tokio::process::Command;

use super::db_store::{
    discover_local_python_venvs, load_stored_project_env, read_pyvenv_version,
    validate_attached_folder_path,
};
use super::{shell_env, EnvironmentManager};
use crate::target::{enforce_non_root_path, TargetManager};
use crate::toolchains::{find_executable_in, is_executable_file, system_search_dirs};
use crate::types::{
    IdeCreatePythonVenvReq, IdeCreatePythonVenvResp, IdePythonVenvInfo, IdeSshSpec, IdeTargetKind,
    IdeTargetSpec,
};

impl EnvironmentManager {
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
        let raw_root = req
            .root
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(req.target.root.as_str())
            .to_string();
        if raw_root.contains("..") {
            return Err(Error::Invalid("root must not contain '..'".into()));
        }
        let effective_root =
            if raw_root != req.target.root || req.target.kind == IdeTargetKind::Local {
                validate_attached_folder_path(&req.target, &raw_root)?
            } else {
                raw_root.clone()
            };
        let mut rel_dir = req
            .venv_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(".venv")
            .to_string();

        if Path::new(&rel_dir).is_absolute() && !rel_dir.contains("..") {
            let vp = Path::new(&rel_dir);
            if req.target.kind == IdeTargetKind::Local {
                let prospective = otto_core::secret_paths::canonicalize_prospective(vp)
                    .ok_or_else(|| Error::Invalid("invalid venv_dir path".into()))?;
                if let Ok(rel) = prospective.strip_prefix(&effective_root) {
                    let rel_s = rel.to_string_lossy().into_owned();
                    if !rel_s.is_empty() {
                        rel_dir = rel_s;
                    }
                }
            } else if let Ok(rel) = vp.strip_prefix(&effective_root) {
                let rel_s = rel.to_string_lossy().into_owned();
                if !rel_s.is_empty() {
                    rel_dir = rel_s;
                }
            }
        }
        if rel_dir.contains("..") || rel_dir.starts_with('/') || rel_dir.starts_with('-') {
            return Err(Error::Invalid(
                "venv_dir must be a safe workspace-relative directory".into(),
            ));
        }
        if req.target.kind == IdeTargetKind::Local {
            let full = Path::new(&effective_root).join(&rel_dir);
            let prospective = otto_core::secret_paths::canonicalize_prospective(&full)
                .ok_or_else(|| Error::Invalid("invalid venv_dir path".into()))?;
            if !prospective.starts_with(&effective_root) {
                return Err(Error::Forbidden("venv_dir escapes workspace root".into()));
            }
            enforce_non_root_path(&prospective)?;
        }

        let validated_py = match req.python_version.as_deref().map(str::trim) {
            Some(v) if !v.is_empty() => {
                let registered_py_sdks = if req.target.kind == IdeTargetKind::Local {
                    crate::toolchains::load_registered_local_sdks_by_host(pool, "local")
                        .await
                        .remove(&crate::toolchains_catalog::EcosystemKind::Python)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|s| PathBuf::from(s.path))
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                };
                Some(validate_python_spec(
                    v,
                    req.target.kind == IdeTargetKind::Local,
                    &registered_py_sdks,
                )?)
            }
            _ => None,
        };

        let resp = match req.target.kind {
            IdeTargetKind::Local => {
                self.create_local_venv(&effective_root, &rel_dir, validated_py.as_deref())
                    .await?
            }
            IdeTargetKind::Ssh => {
                let spec = ssh.ok_or_else(|| Error::Invalid("SSH spec required".into()))?;
                let resolved = targets.resolve_with_ssh(&req.target, "ide-venv", spec.clone())?;
                let q_dir = shell_words::quote(&rel_dir);
                let script = match validated_py.as_deref() {
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
                        Some(&effective_root),
                        &BTreeMap::new(),
                        Duration::from_secs(120),
                    )
                    .await?;
                if !out.success() {
                    return Err(Error::Invalid(format!(
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
                let venv_path = format!("{}/{rel_dir}", effective_root.trim_end_matches('/'));
                IdeCreatePythonVenvResp {
                    interpreter_path: format!("{venv_path}/bin/python"),
                    path: venv_path,
                    version: ver,
                }
            }
        };

        let is_primary_root = raw_root.trim_end_matches('/')
            == req.target.root.trim_end_matches('/')
            || effective_root.trim_end_matches('/') == req.target.root.trim_end_matches('/');
        let (existing, _) = load_stored_project_env(pool, &req.target.project_key()).await?;
        if existing.python_interpreter.is_none() && is_primary_root {
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
            self.refresh_project_snapshot(pool, &req.target.project_key())
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

        let explicit_py_path = py_ver.map(PathBuf::from).filter(|p| p.is_absolute());
        let safe_env = shell_env::collect_safe_non_root_base_env();

        let status_out = if let Some(uv) = uv_bin {
            let mut cmd = Command::new(uv);
            cmd.env_clear().envs(&safe_env).arg("venv");
            if let Some(v) = py_ver.filter(|s| !s.trim().is_empty()) {
                cmd.arg("--python").arg(v);
            }
            cmd.arg("--")
                .arg(&venv_path)
                .kill_on_drop(true)
                .output()
                .await
        } else {
            if let Some(v) = py_ver {
                if explicit_py_path.is_none() && !is_known_python_bin_name(v) {
                    return Err(Error::Invalid(format!(
                        "Python version '{v}' requires uv or an installed python binary path"
                    )));
                }
            }
            let py = explicit_py_path
                .or_else(|| {
                    py_ver
                        .filter(|v| is_known_python_bin_name(v))
                        .and_then(|v| find_executable_in(&search, &[v]))
                })
                .or_else(|| find_executable_in(&search, &["python3", "python"]))
                .ok_or_else(|| Error::Invalid("neither uv nor python3 found on host".into()))?;
            Command::new(py)
                .env_clear()
                .envs(&safe_env)
                .args(["-m", "venv", "--"])
                .arg(&venv_path)
                .kill_on_drop(true)
                .output()
                .await
        }
        .map_err(|e| Error::Internal(format!("spawn venv creator: {e}")))?;

        if !status_out.status.success() {
            return Err(Error::Invalid(format!(
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
}

#[allow(clippy::disallowed_methods)]
fn validate_python_spec(raw: &str, is_local: bool, registered_sdks: &[PathBuf]) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.starts_with('-')
        || trimmed.contains("..")
        || trimmed.chars().any(|c| c.is_control())
    {
        return Err(Error::Invalid(format!(
            "invalid Python version or interpreter '{trimmed}'"
        )));
    }

    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.starts_with('~') {
        if !is_local {
            if !trimmed.starts_with('/')
                || !trimmed
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '.' | '_' | '-' | '+'))
            {
                return Err(Error::Invalid(format!(
                    "invalid remote Python interpreter path '{trimmed}'"
                )));
            }
            let base = Path::new(trimmed)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("");
            if !is_known_python_bin_name(base) {
                return Err(Error::Invalid(format!(
                    "remote interpreter '{trimmed}' is not a Python binary"
                )));
            }
            return Ok(trimmed.to_string());
        }

        let expanded = otto_core::paths::expand_home_strict(trimmed).map_err(Error::Invalid)?;
        let canon = std::fs::canonicalize(&expanded).map_err(|_| {
            Error::Invalid(format!(
                "Python interpreter '{}' does not exist",
                expanded.display()
            ))
        })?;
        enforce_non_root_path(&canon)?;
        if !is_executable_file(&canon) {
            return Err(Error::Invalid(format!(
                "Python interpreter '{}' is not an executable file",
                canon.display()
            )));
        }
        let in_registered_sdk = registered_sdks.iter().any(|sdk| {
            let sdk_canon = std::fs::canonicalize(sdk).unwrap_or_else(|_| sdk.clone());
            canon == sdk_canon || canon.starts_with(&sdk_canon)
        });
        let req_name = expanded.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let canon_name = canon.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let canon_str = canon.to_string_lossy();
        let is_valid_python = in_registered_sdk
            || (is_known_python_bin_name(req_name)
                && (is_known_python_bin_name(canon_name)
                    || (canon_name == "Python" && canon_str.contains(".framework/"))));
        if !is_valid_python {
            return Err(Error::Invalid(format!(
                "path '{}' is not a recognized Python interpreter",
                expanded.display()
            )));
        }
        return Ok(canon.to_string_lossy().into_owned());
    }

    if trimmed.len() > 32
        || !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+'))
    {
        return Err(Error::Invalid(format!(
            "invalid Python version '{trimmed}'"
        )));
    }
    let starts_digit = trimmed.chars().next().is_some_and(|c| c.is_ascii_digit());
    if !starts_digit && !is_known_python_bin_name(trimmed) && !trimmed.starts_with("cpython-") {
        return Err(Error::Invalid(format!(
            "unsupported Python version selector '{trimmed}'"
        )));
    }
    Ok(trimmed.to_string())
}

fn is_known_python_bin_name(name: &str) -> bool {
    if matches!(name, "python" | "python3" | "pypy" | "pypy3") {
        return true;
    }
    for prefix in ["python3.", "python2.", "pypy3."] {
        if let Some(rest) = name.strip_prefix(prefix) {
            return !rest.is_empty()
                && rest.chars().all(|c| {
                    c.is_ascii_alphanumeric() && matches!(c, '0'..='9' | 't' | 'd' | 'm')
                });
        }
    }
    false
}
