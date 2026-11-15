//! Effective environment layer resolution and `.env` file discovery
//! for [`super::EnvironmentManager`].

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use otto_core::Result;

use super::db_store::{
    apply_run_config_sdk_override, discover_local_dotenv_files, is_valid_env_key, parse_dotenv,
    push_unique, safe_join_allowed_roots, StoredEnvVar,
};
use super::{shell_env, EnvironmentManager, SecretAccess};
use crate::target::TargetManager;
use crate::toolchains::system_search_dirs;
use crate::types::{IdeProjectEnvironment, IdeRunConfig, IdeSshSpec, IdeTargetKind, IdeTargetSpec};

impl EnvironmentManager {
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn resolve_env_layers<S: SecretAccess + ?Sized>(
        &self,
        proj: &IdeProjectEnvironment,
        stored_vars: &[StoredEnvVar],
        secrets: Option<&S>,
        target: &IdeTargetSpec,
        cfg: Option<&IdeRunConfig>,
        await_shell: bool,
        caller_is_root: bool,
    ) -> Result<BTreeMap<String, String>> {
        let mut map = BTreeMap::new();
        let inherit_shell =
            proj.inherit_shell_env && cfg.map(|c| c.inherit_shell_env).unwrap_or(true);

        if target.kind == IdeTargetKind::Local {
            if caller_is_root {
                map.extend(shell_env::collect_process_base_env());
                if inherit_shell {
                    let shell_vars = if await_shell {
                        self.shell_env.vars_or_load().await
                    } else {
                        self.shell_env.vars_snapshot()
                    };
                    map.extend(shell_vars);
                }
            } else {
                map.extend(shell_env::collect_safe_non_root_base_env());
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

        // On SSH targets, do not populate literal `$HOME/...` entries or clobber
        // the remote shell's `$PATH`; `build_remote_exec_line` prepends explicit
        // project SDK overrides + remote `$HOME/.otto/toolchains/...` onto `$PATH`.
        if target.kind == IdeTargetKind::Local {
            for d in crate::toolchains::ToolchainManager::with_shell_env(
                self.data_dir.clone(),
                Arc::clone(&self.shell_env),
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
            if let Some(inherited_path) = map.get("PATH") {
                for seg in inherited_path.split(':').filter(|s| !s.is_empty()) {
                    push_unique(&mut path_dirs, seg.to_string());
                }
            }
            for d in system_search_dirs() {
                push_unique(&mut path_dirs, d.to_string_lossy().into_owned());
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
                    if caller_is_root || !otto_core::secret_paths::is_denied_file(&dotenv_file) {
                        if let Ok(raw) = tokio::fs::read_to_string(&dotenv_file).await {
                            map.extend(parse_dotenv(&raw));
                        }
                    }
                }
            } else {
                let default_dotenv = Path::new(&target.root).join(".env");
                if default_dotenv.is_file()
                    && (caller_is_root || !otto_core::secret_paths::is_denied_file(&default_dotenv))
                {
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
                    if caller_is_root || !otto_core::secret_paths::is_denied_file(&resolved_ef) {
                        if let Ok(raw) = tokio::fs::read_to_string(&resolved_ef).await {
                            map.extend(parse_dotenv(&raw));
                        }
                    }
                }
            }
        }

        for item in stored_vars {
            if item.secret {
                if let Some(sec_store) = secrets {
                    if let Some(sref) = &item.secret_ref {
                        match sec_store.get_secret(sref).await {
                            Ok(Some(secret_val)) => {
                                map.insert(item.key.clone(), secret_val);
                            }
                            Ok(None) => {
                                tracing::warn!(
                                    key = %item.key,
                                    "configured IDE secret env var was not found in secret store"
                                );
                            }
                            Err(err) => {
                                tracing::warn!(
                                    key = %item.key,
                                    error = %err,
                                    "failed to read IDE secret env var from secret store"
                                );
                                return Err(err);
                            }
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

        Ok(map)
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
}
