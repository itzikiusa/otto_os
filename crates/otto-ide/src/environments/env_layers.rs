//! Effective environment layer resolution, `.env` file discovery, and Python virtualenv
//! discovery/creation methods for [`super::EnvironmentManager`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::Utc;
use otto_core::{Error, Result};
use otto_state::DbPool;
use tokio::process::Command;

use super::db_store::{
    apply_run_config_sdk_override, discover_local_dotenv_files, discover_local_python_venvs,
    is_valid_env_key, load_stored_project_env, parse_dotenv, push_unique, read_pyvenv_version,
    safe_join_allowed_roots, StoredEnvVar,
};
use super::{shell_env, EnvironmentManager, SecretAccess};
use crate::target::TargetManager;
use crate::toolchains::{find_executable_in, is_executable_file, system_search_dirs};
use crate::types::{
    IdeCreatePythonVenvReq, IdeCreatePythonVenvResp, IdeProjectEnvironment, IdePythonVenvInfo,
    IdeRunConfig, IdeSshSpec, IdeTargetKind, IdeTargetSpec,
};

impl EnvironmentManager {
    pub(super) async fn resolve_env_layers<S: SecretAccess + ?Sized>(
        &self,
        proj: &IdeProjectEnvironment,
        stored_vars: &[StoredEnvVar],
        secrets: Option<&S>,
        target: &IdeTargetSpec,
        cfg: Option<&IdeRunConfig>,
        await_shell: bool,
    ) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        let inherit_shell =
            proj.inherit_shell_env && cfg.map(|c| c.inherit_shell_env).unwrap_or(true);

        if target.kind == IdeTargetKind::Local {
            map.extend(shell_env::collect_process_base_env());
            if inherit_shell {
                let shell_vars = if await_shell {
                    self.shell_env.vars_or_load().await
                } else {
                    self.shell_env.vars_snapshot()
                };
                map.extend(shell_vars);
            }
        }

        let mut path_dirs: Vec<String> = Vec::new();
        if let Some(extra) = proj.extra_path.as_deref().filter(|s| !s.is_empty()) {
            path_dirs.extend(
                extra
                    .split(':')
                    .filter(|s| !s.is_empty())
                    .map(str::to_string),
            );
        }

        let mut proj_path_dirs: Vec<String> = Vec::new();
        if let Some(py) = proj.python_interpreter.as_deref().filter(|s| !s.is_empty()) {
            if let Some(bin_dir) = Path::new(py).parent() {
                push_unique(&mut proj_path_dirs, bin_dir.to_string_lossy().into_owned());
                if let Some(venv) = bin_dir.parent() {
                    if venv.join("pyvenv.cfg").is_file() || target.kind == IdeTargetKind::Ssh {
                        map.insert("VIRTUAL_ENV".into(), venv.to_string_lossy().into_owned());
                    }
                }
            }
        }
        if let Some(jh) = proj.java_home.as_deref().filter(|s| !s.is_empty()) {
            map.insert("JAVA_HOME".into(), jh.to_string());
            push_unique(
                &mut proj_path_dirs,
                format!("{}/bin", jh.trim_end_matches('/')),
            );
        }
        if let Some(gr) = proj.go_root.as_deref().filter(|s| !s.is_empty()) {
            map.insert("GOROOT".into(), gr.to_string());
            push_unique(
                &mut proj_path_dirs,
                format!("{}/bin", gr.trim_end_matches('/')),
            );
        }
        if let Some(rt) = proj.rust_toolchain.as_deref().filter(|s| !s.is_empty()) {
            if rt.starts_with('/') {
                push_unique(
                    &mut proj_path_dirs,
                    format!("{}/bin", rt.trim_end_matches('/')),
                );
            } else {
                map.insert("RUSTUP_TOOLCHAIN".into(), rt.to_string());
            }
        }
        if let Some(nh) = proj.node_home.as_deref().filter(|s| !s.is_empty()) {
            push_unique(
                &mut proj_path_dirs,
                format!("{}/bin", nh.trim_end_matches('/')),
            );
        }

        let cfg_sdk = cfg
            .and_then(|c| c.sdk_path.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        if let (Some(c), Some(sdk_p)) = (cfg, cfg_sdk) {
            apply_run_config_sdk_override(c.runner, sdk_p, target.kind, &mut map, &mut path_dirs);
        }
        for d in proj_path_dirs {
            push_unique(&mut path_dirs, d);
        }

        if target.kind == IdeTargetKind::Local {
            for d in crate::toolchains::ToolchainManager::new(self.data_dir.clone())
                .extra_local_bin_dirs()
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
            if let Some(inherited_path) = map.get("PATH") {
                for seg in inherited_path.split(':').filter(|s| !s.is_empty()) {
                    push_unique(&mut path_dirs, seg.to_string());
                }
            }
            for d in system_search_dirs() {
                push_unique(&mut path_dirs, d.to_string_lossy().into_owned());
            }
        } else {
            for rel in [
                "$HOME/.otto/toolchains/go/bin",
                "$HOME/.otto/toolchains/rust/bin",
                "$HOME/.otto/toolchains/rust/cargo/bin",
                "$HOME/.otto/toolchains/python/bin",
                "/opt/homebrew/bin",
                "/usr/local/go/bin",
                "/usr/local/bin",
                "/usr/bin",
                "/bin",
            ] {
                push_unique(&mut path_dirs, rel.to_string());
            }
        }
        if !path_dirs.is_empty() {
            map.insert("PATH".into(), path_dirs.join(":"));
        }

        if target.kind == IdeTargetKind::Local {
            let mut all_roots = vec![target.root.clone()];
            for r in proj
                .additional_roots
                .iter()
                .cloned()
                .chain(target.merged_extra_roots())
            {
                if !r.is_empty() && !all_roots.contains(&r) {
                    all_roots.push(r);
                }
            }

            if let Some(dotenv_rel) = proj.dotenv_path.as_deref().filter(|s| !s.is_empty()) {
                if let Some(dotenv_file) = safe_join_allowed_roots(&all_roots, dotenv_rel) {
                    if let Ok(raw) = tokio::fs::read_to_string(&dotenv_file).await {
                        map.extend(parse_dotenv(&raw));
                    }
                }
            } else {
                let default_dotenv = Path::new(&target.root).join(".env");
                if default_dotenv.is_file() {
                    if let Ok(raw) = tokio::fs::read_to_string(&default_dotenv).await {
                        map.extend(parse_dotenv(&raw));
                    }
                }
            }

            if let Some(cfg_env_file) = cfg
                .and_then(|c| c.env_file.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                if let Some(resolved_ef) = safe_join_allowed_roots(&all_roots, cfg_env_file) {
                    if let Ok(raw) = tokio::fs::read_to_string(&resolved_ef).await {
                        map.extend(parse_dotenv(&raw));
                    }
                }
            }
        }

        for item in stored_vars {
            if item.secret {
                if let Some(sec_store) = secrets {
                    if let Some(sref) = &item.secret_ref {
                        if let Ok(Some(secret_val)) = sec_store.get_secret(sref).await {
                            map.insert(item.key.clone(), secret_val);
                        }
                    }
                } else if item.secret_ref.is_some() {
                    map.insert(item.key.clone(), "••••••••".into());
                }
            } else if let Some(val) = &item.value {
                map.insert(item.key.clone(), val.clone());
            }
        }

        if let Some(c) = cfg {
            for (k, v) in &c.env_overrides {
                if is_valid_env_key(k) {
                    map.insert(k.clone(), v.clone());
                }
            }
        }

        map
    }

    pub async fn discover_dotenv_files(
        &self,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
    ) -> Result<Vec<String>> {
        match target.kind {
            IdeTargetKind::Local => Ok(discover_local_dotenv_files(Path::new(&target.root))),
            IdeTargetKind::Ssh => {
                let Some(spec) = ssh else {
                    return Ok(Vec::new());
                };
                let resolved = targets.resolve_with_ssh(target, "ide-env", spec.clone())?;
                let script = "for f in .env .env.local .env.development .env.test .env.staging .env.production; do [ -f \"$f\" ] && echo \"$f\"; done";
                let out = targets
                    .exec_shell(
                        &resolved,
                        script,
                        Some(&target.root),
                        &BTreeMap::new(),
                        Duration::from_secs(10),
                    )
                    .await?;
                Ok(out
                    .stdout
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string)
                    .collect())
            }
        }
    }

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
        let mut effective_root = req
            .root
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(req.target.root.as_str())
            .to_string();
        let mut rel_dir = req
            .venv_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(".venv")
            .to_string();

        if req.target.kind == IdeTargetKind::Local && Path::new(&rel_dir).is_absolute() {
            let vp = Path::new(&rel_dir).to_path_buf();
            if let Ok(rel) = vp.strip_prefix(&effective_root) {
                let rel_s = rel.to_string_lossy().into_owned();
                if !rel_s.is_empty() {
                    rel_dir = rel_s;
                }
            } else if let (Some(parent), Some(fname)) =
                (vp.parent(), vp.file_name().and_then(|n| n.to_str()))
            {
                effective_root = parent.to_string_lossy().into_owned();
                rel_dir = fname.to_string();
            }
        }
        if rel_dir.contains("..") || rel_dir.starts_with('/') {
            return Err(Error::Invalid(
                "venv_dir must be a safe workspace-relative directory".into(),
            ));
        }

        let resp = match req.target.kind {
            IdeTargetKind::Local => {
                self.create_local_venv(&effective_root, &rel_dir, req.python_version.as_deref())
                    .await?
            }
            IdeTargetKind::Ssh => {
                let spec = ssh.ok_or_else(|| Error::Invalid("SSH spec required".into()))?;
                let resolved = targets.resolve_with_ssh(&req.target, "ide-venv", spec.clone())?;
                let q_dir = shell_words::quote(&rel_dir);
                let script = match req.python_version.as_deref().filter(|s| !s.is_empty()) {
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
                    return Err(Error::Internal(format!(
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

        let (existing, _) = load_stored_project_env(pool, &req.target.project_key()).await?;
        if existing.python_interpreter.is_none()
            && effective_root.trim_end_matches('/') == req.target.root.trim_end_matches('/')
        {
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

        let status_out = if let Some(uv) = uv_bin {
            let mut cmd = Command::new(uv);
            cmd.arg("venv");
            if let Some(v) = py_ver.filter(|s| !s.trim().is_empty()) {
                cmd.args(["--python", v]);
            }
            cmd.arg(&venv_path).kill_on_drop(true).output().await
        } else {
            let explicit_py = py_ver
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .filter(|p| is_executable_file(p));
            let py = explicit_py
                .or_else(|| find_executable_in(&search, &["python3", "python"]))
                .ok_or_else(|| Error::Invalid("neither uv nor python3 found on host".into()))?;
            Command::new(py)
                .args(["-m", "venv"])
                .arg(&venv_path)
                .kill_on_drop(true)
                .output()
                .await
        }
        .map_err(|e| Error::Internal(format!("spawn venv creator: {e}")))?;

        if !status_out.status.success() {
            return Err(Error::Internal(format!(
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
