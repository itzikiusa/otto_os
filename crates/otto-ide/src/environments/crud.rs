//! Project environment inspection and persistence (`get_environments` /
//! `update_environment`) for [`super::EnvironmentManager`].

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::Utc;
use otto_core::{Error, Result};
use otto_state::DbPool;

use super::crud_folders::redact_stored_vars;
use super::db_store::{
    clean_opt, discover_local_dotenv_files, discover_local_python_venvs,
    is_dangerous_loader_env_key, is_reserved_internal_env_key, is_valid_env_key,
    load_stored_project_env, secret_key_ref, validate_attached_folder_path, StoredEnvVar,
    RESERVED_GO_BUILD_TAGS_KEY, RESERVED_MODULE_GO_BUILD_TAGS_KEY, RESERVED_MODULE_SDKS_KEY,
    RESERVED_NETBRIDGE_KEY,
};
use super::discovery::discover_run_configs;
use super::module_sdks::{sanitize_module_go_build_tags, sanitize_module_sdks};
use super::{EnvironmentManager, SecretAccess};
use crate::netbridge::sanitize_network_bridge;
use crate::target::TargetManager;
use crate::types::{
    IdeEnvironmentsResp, IdeProjectEnvironment, IdeSshSpec, IdeTargetKind, IdeTargetSpec,
    IdeUpdateEnvironmentReq,
};

fn validate_environment_update_permissions(
    req: &IdeUpdateEnvironmentReq,
    prev_proj: &IdeProjectEnvironment,
    caller_is_root: bool,
) -> Result<()> {
    if !caller_is_root {
        for (field_name, req_val, prev_val) in [
            (
                "java_home",
                clean_opt(&req.java_home),
                prev_proj.java_home.clone(),
            ),
            (
                "go_root",
                clean_opt(&req.go_root),
                prev_proj.go_root.clone(),
            ),
            (
                "rust_toolchain",
                clean_opt(&req.rust_toolchain),
                prev_proj.rust_toolchain.clone(),
            ),
            (
                "node_home",
                clean_opt(&req.node_home),
                prev_proj.node_home.clone(),
            ),
            (
                "python_interpreter",
                clean_opt(&req.python_interpreter),
                prev_proj.python_interpreter.clone(),
            ),
            (
                "extra_path",
                clean_opt(&req.extra_path),
                prev_proj.extra_path.clone(),
            ),
        ] {
            if let Some(ref next_v) = req_val {
                if Some(next_v.as_str()) != prev_val.as_deref() {
                    return Err(Error::Forbidden(format!(
                        "non-root users cannot override '{field_name}'"
                    )));
                }
            }
        }
        if let Some(ref explicit_sdks) = req.module_sdks {
            let sanitized = sanitize_module_sdks(explicit_sdks);
            if !sanitized.is_empty() && sanitized != prev_proj.module_sdks {
                return Err(Error::Forbidden(
                    "non-root users cannot override 'module_sdks'".into(),
                ));
            }
        }
        if let Some(ref raw_dotenv) = clean_opt(&req.dotenv_path) {
            let mut check_target = req.target.clone();
            if let Some(ref explicit_roots) = req.additional_roots {
                for r in explicit_roots {
                    let tr = r.trim();
                    if !tr.is_empty()
                        && tr != check_target.root
                        && !check_target.additional_roots.iter().any(|x| x == tr)
                    {
                        check_target.additional_roots.push(tr.to_string());
                    }
                }
            }
            super::run_configs::validate_target_confined_subpath(
                &check_target,
                raw_dotenv,
                "dotenv_path",
            )?;
        }
    } else if let Some(ref raw_dotenv) = clean_opt(&req.dotenv_path) {
        if raw_dotenv.chars().any(char::is_control) {
            return Err(Error::Invalid(
                "invalid control characters in 'dotenv_path'".into(),
            ));
        }
    }
    Ok(())
}

impl EnvironmentManager {
    pub async fn get_environments<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
    ) -> Result<IdeEnvironmentsResp> {
        self.get_environments_for_user(pool, secrets, targets, target, ssh, true)
            .await
    }

    pub async fn get_environments_for_user<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        _secrets: &S,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
        caller_is_root: bool,
    ) -> Result<IdeEnvironmentsResp> {
        target.validate()?;
        let project_key = target.project_key();
        let (mut env, stored_vars) = load_stored_project_env(pool, &project_key).await?;
        self.record_snapshot(&project_key, &env, &stored_vars);
        let req_extra = target.merged_extra_roots();
        if !caller_is_root && target.kind == IdeTargetKind::Local {
            env.additional_roots = req_extra;
        } else {
            for extra in req_extra {
                if !extra.is_empty()
                    && extra != target.root
                    && !env.additional_roots.contains(&extra)
                {
                    env.additional_roots.push(extra);
                }
            }
        }
        env.env_vars = redact_stored_vars(&stored_vars);

        let saved_configs = self.list_run_configs(pool, target).await?;

        let (dotenv_files, python_venvs, detected_configs) = match target.kind {
            IdeTargetKind::Local => {
                let target_clone = target.clone();
                let extra_roots = env.additional_roots.clone();
                let data_dir = self.data_dir.clone();
                tokio::task::spawn_blocking(move || {
                    let root = PathBuf::from(&target_clone.root);
                    let dotenvs = discover_local_dotenv_files(&root);
                    let mut venvs = discover_local_python_venvs(&root, &data_dir);
                    for er in &extra_roots {
                        for mv in discover_local_python_venvs(Path::new(er), &data_dir) {
                            if !venvs.iter().any(|v| v.path == mv.path) {
                                venvs.push(mv);
                            }
                        }
                    }
                    let detected = discover_run_configs(&target_clone, &extra_roots);
                    (dotenvs, venvs, detected)
                })
                .await
                .map_err(|e| Error::Internal(format!("local environment discovery: {e}")))?
            }
            IdeTargetKind::Ssh => {
                let dotenvs = self
                    .discover_dotenv_files(targets, target, ssh)
                    .await
                    .unwrap_or_default();
                let venvs = self
                    .discover_python_venvs(targets, target, ssh)
                    .await
                    .unwrap_or_default();
                (dotenvs, venvs, Vec::new())
            }
        };

        let mut configs = saved_configs;
        for det in &detected_configs {
            let duplicate = configs.iter().any(|c| {
                c.id == det.id
                    || (c.runner == det.runner
                        && c.name.eq_ignore_ascii_case(&det.name)
                        && c.cwd == det.cwd)
            });
            if !duplicate {
                configs.push(det.clone());
            }
        }

        let effective_env = self
            .resolve_env_layers(
                &env,
                &stored_vars,
                None::<&S>,
                target,
                None,
                false,
                caller_is_root,
            )
            .await?;

        Ok(IdeEnvironmentsResp {
            env,
            python_venvs,
            dotenv_files,
            configs,
            detected_configs,
            effective_env,
        })
    }

    pub async fn update_environment<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        req: IdeUpdateEnvironmentReq,
    ) -> Result<IdeProjectEnvironment> {
        self.update_environment_for_user(pool, secrets, req, true)
            .await
    }

    pub async fn update_environment_for_user<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        req: IdeUpdateEnvironmentReq,
        caller_is_root: bool,
    ) -> Result<IdeProjectEnvironment> {
        req.target.validate()?;
        let project_key = req.target.project_key();
        let (prev_proj, prev_stored) = load_stored_project_env(pool, &project_key).await?;
        validate_environment_update_permissions(&req, &prev_proj, caller_is_root)?;

        let prev_map: BTreeMap<String, StoredEnvVar> = prev_stored
            .into_iter()
            .map(|e| (e.key.clone(), e))
            .collect();

        let mut next_stored = Vec::with_capacity(req.env_vars.len() + 4);
        let mut kept_refs = HashSet::new();

        for item in &req.env_vars {
            let key = item.key.trim().to_string();
            if key.is_empty() || is_reserved_internal_env_key(&key) || !is_valid_env_key(&key) {
                return Err(Error::Invalid(format!(
                    "invalid environment variable name '{key}'"
                )));
            }
            if !caller_is_root && is_dangerous_loader_env_key(&key) {
                return Err(Error::Forbidden(format!(
                    "non-root users cannot set loader environment variable '{key}'"
                )));
            }
            if item.secret {
                let sref = secret_key_ref(&project_key, &key);
                if let Some(val) = item.value.as_deref().filter(|v| !v.is_empty()) {
                    secrets.put_secret(&sref, val).await?;
                    kept_refs.insert(sref.clone());
                    next_stored.push(StoredEnvVar {
                        key,
                        value: None,
                        secret: true,
                        secret_ref: Some(sref),
                    });
                } else if let Some(prev_ref) = prev_map.get(&key).and_then(|p| p.secret_ref.clone())
                {
                    kept_refs.insert(prev_ref.clone());
                    next_stored.push(StoredEnvVar {
                        key,
                        value: None,
                        secret: true,
                        secret_ref: Some(prev_ref),
                    });
                } else {
                    next_stored.push(StoredEnvVar {
                        key,
                        value: None,
                        secret: true,
                        secret_ref: None,
                    });
                }
            } else {
                next_stored.push(StoredEnvVar {
                    key,
                    value: Some(item.value.clone().unwrap_or_default()),
                    secret: false,
                    secret_ref: None,
                });
            }
        }

        for old in prev_map.values() {
            if let Some(r) = &old.secret_ref {
                if !kept_refs.contains(r) {
                    let _ = secrets.delete_secret(r).await;
                }
            }
        }

        let inherit_shell_env = req.inherit_shell_env.unwrap_or(prev_proj.inherit_shell_env);
        let next_roots = match &req.additional_roots {
            Some(explicit_roots) => {
                let mut validated = Vec::new();
                for r in explicit_roots {
                    if r.trim().is_empty() || r.trim() == req.target.root {
                        continue;
                    }
                    let norm = validate_attached_folder_path(&req.target, r)?;
                    if norm != req.target.root && !validated.contains(&norm) {
                        validated.push(norm);
                    }
                }
                validated
            }
            None => prev_proj.additional_roots,
        };

        let next_module_sdks = if caller_is_root {
            match &req.module_sdks {
                Some(explicit_map) => sanitize_module_sdks(explicit_map),
                None => prev_proj.module_sdks,
            }
        } else {
            prev_proj.module_sdks
        };
        if !next_module_sdks.is_empty() {
            let encoded = serde_json::to_string(&next_module_sdks)
                .map_err(|e| Error::Internal(format!("serialize module_sdks: {e}")))?;
            next_stored.push(StoredEnvVar {
                key: RESERVED_MODULE_SDKS_KEY.into(),
                value: Some(encoded),
                secret: false,
                secret_ref: None,
            });
        }

        let next_go_build_tags = match &req.go_build_tags {
            Some(raw_tags) => crate::lsp_service::validate_go_build_tags(raw_tags)?,
            None => prev_proj.go_build_tags,
        };
        if !next_go_build_tags.is_empty() {
            let encoded = serde_json::to_string(&next_go_build_tags)
                .map_err(|e| Error::Internal(format!("serialize go_build_tags: {e}")))?;
            next_stored.push(StoredEnvVar {
                key: RESERVED_GO_BUILD_TAGS_KEY.into(),
                value: Some(encoded),
                secret: false,
                secret_ref: None,
            });
        }

        let next_module_go_build_tags = match &req.module_go_build_tags {
            Some(raw_map) => sanitize_module_go_build_tags(raw_map)?,
            None => prev_proj.module_go_build_tags,
        };
        if !next_module_go_build_tags.is_empty() {
            let encoded = serde_json::to_string(&next_module_go_build_tags)
                .map_err(|e| Error::Internal(format!("serialize module_go_build_tags: {e}")))?;
            next_stored.push(StoredEnvVar {
                key: RESERVED_MODULE_GO_BUILD_TAGS_KEY.into(),
                value: Some(encoded),
                secret: false,
                secret_ref: None,
            });
        }

        let next_network_bridge = match req.network_bridge {
            Some(explicit_nb) => sanitize_network_bridge(Some(explicit_nb))?,
            None => prev_proj.network_bridge,
        };
        if let Some(ref nb) = next_network_bridge {
            let encoded = serde_json::to_string(nb)
                .map_err(|e| Error::Internal(format!("serialize network_bridge: {e}")))?;
            next_stored.push(StoredEnvVar {
                key: RESERVED_NETBRIDGE_KEY.into(),
                value: Some(encoded),
                secret: false,
                secret_ref: None,
            });
        }

        let java_home_db = if caller_is_root {
            clean_opt(&req.java_home)
        } else {
            prev_proj.java_home
        };
        let go_root_db = if caller_is_root {
            clean_opt(&req.go_root)
        } else {
            prev_proj.go_root
        };
        let rust_toolchain_db = if caller_is_root {
            clean_opt(&req.rust_toolchain)
        } else {
            prev_proj.rust_toolchain
        };
        let node_home_db = if caller_is_root {
            clean_opt(&req.node_home)
        } else {
            prev_proj.node_home
        };
        let python_interpreter_db = if caller_is_root {
            clean_opt(&req.python_interpreter)
        } else {
            prev_proj.python_interpreter
        };
        let extra_path_db = if caller_is_root {
            clean_opt(&req.extra_path)
        } else {
            prev_proj.extra_path
        };

        let env_vars_json = serde_json::to_string(&next_stored)
            .map_err(|e| Error::Internal(format!("serialize env_vars_json: {e}")))?;
        let roots_json = serde_json::to_string(&next_roots)
            .map_err(|e| Error::Internal(format!("serialize additional_roots_json: {e}")))?;
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"INSERT INTO ide_project_environments (
                   project_key, target_kind, root_path, connection_id,
                   java_home, go_root, rust_toolchain, node_home,
                   python_interpreter, dotenv_path, extra_path, env_vars_json,
                   additional_roots_json, inherit_shell_env, updated_at
               ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(project_key) DO UPDATE SET
                   target_kind = excluded.target_kind,
                   root_path = excluded.root_path,
                   connection_id = excluded.connection_id,
                   java_home = excluded.java_home,
                   go_root = excluded.go_root,
                   rust_toolchain = excluded.rust_toolchain,
                   node_home = excluded.node_home,
                   python_interpreter = excluded.python_interpreter,
                   dotenv_path = excluded.dotenv_path,
                   extra_path = excluded.extra_path,
                   env_vars_json = excluded.env_vars_json,
                   additional_roots_json = excluded.additional_roots_json,
                   inherit_shell_env = excluded.inherit_shell_env,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&project_key)
        .bind(req.target.kind.as_str())
        .bind(&req.target.root)
        .bind(&req.target.connection_id)
        .bind(java_home_db)
        .bind(go_root_db)
        .bind(rust_toolchain_db)
        .bind(node_home_db)
        .bind(python_interpreter_db)
        .bind(clean_opt(&req.dotenv_path))
        .bind(extra_path_db)
        .bind(&env_vars_json)
        .bind(&roots_json)
        .bind(i32::from(inherit_shell_env))
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| Error::Internal(format!("upsert ide_project_environments: {e}")))?;

        self.get_project_env(pool, &req.target).await
    }
}
