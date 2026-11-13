//! Project Environments, Keychain-backed Secret Env Vars, `.env` & Python
//! Virtualenv Manager, Run Configurations CRUD, and Multi-Language Test/Run
//! Execution Engine (`EnvironmentManager`).

mod runner;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use dashmap::DashMap;
use otto_core::auth::BoxFuture;
use otto_core::secrets::{delete_async, get_async, put_async, SecretStore};
use otto_core::{new_id, Error, Result};
use otto_state::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;
use tokio::process::Command;
use tokio::sync::watch;

use crate::target::TargetManager;
use crate::toolchains::{find_executable_in, is_executable_file, read_subdirs, system_search_dirs};
use crate::types::{
    IdeCreatePythonVenvReq, IdeCreatePythonVenvResp, IdeEnvVarEntry, IdeEnvironmentsResp,
    IdeProjectEnvironment, IdePythonVenvInfo, IdeRunConfig, IdeRunState, IdeRunnerKind, IdeSshSpec,
    IdeStartRunReq, IdeTargetKind, IdeTargetSpec, IdeUpdateEnvironmentReq, IdeUpsertRunConfigReq,
};

pub use self::runner::parse_test_output;
use self::runner::{evict_finished_runs, execute_run_task, ActiveRunHandle, RunStateInner};

/// Uniform async secret accessor accepted by `EnvironmentManager` whether a
/// caller holds `&Arc<dyn SecretStore>` (`ctx.secrets()`) or `&dyn SecretStore`.
pub trait SecretAccess: Sync {
    fn get_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>>>;
    fn put_secret<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<()>>;
    fn delete_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>>;
}

impl SecretAccess for Arc<dyn SecretStore> {
    fn get_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>>> {
        let store = Arc::clone(self);
        let k = key.to_string();
        Box::pin(async move { get_async(&store, &k).await })
    }

    fn put_secret<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<()>> {
        let store = Arc::clone(self);
        let (k, v) = (key.to_string(), value.to_string());
        Box::pin(async move { put_async(&store, &k, &v).await })
    }

    fn delete_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        let store = Arc::clone(self);
        let k = key.to_string();
        Box::pin(async move { delete_async(&store, &k).await })
    }
}

impl SecretAccess for dyn SecretStore {
    #[allow(clippy::disallowed_methods)] // trait-object fallback when caller only has &dyn SecretStore
    fn get_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<Option<String>>> {
        let res = match self.get_cached(key) {
            Some(cached) => Ok(cached),
            None => self.get(key),
        };
        Box::pin(async move { res })
    }

    #[allow(clippy::disallowed_methods)] // trait-object fallback when caller only has &dyn SecretStore
    fn put_secret<'a>(&'a self, key: &'a str, value: &'a str) -> BoxFuture<'a, Result<()>> {
        let res = self.put(key, value);
        Box::pin(async move { res })
    }

    #[allow(clippy::disallowed_methods)] // trait-object fallback when caller only has &dyn SecretStore
    fn delete_secret<'a>(&'a self, key: &'a str) -> BoxFuture<'a, Result<()>> {
        let res = self.delete(key);
        Box::pin(async move { res })
    }
}

/// Internal SQLite representation inside `ide_project_environments.env_vars_json`.
/// Never stores plaintext values when `secret == true`; stores `secret_ref` only.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEnvVar {
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    value: Option<String>,
    #[serde(default)]
    secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    secret_ref: Option<String>,
}

/// Manages per-project SDK/env configuration, Keychain secrets, Python venvs,
/// saved Run Configurations, and streaming Test/Run execution.
#[derive(Clone)]
pub struct EnvironmentManager {
    data_dir: PathBuf,
    runs: Arc<DashMap<String, Arc<ActiveRunHandle>>>,
    run_seq: Arc<AtomicU64>,
}

impl EnvironmentManager {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            runs: Arc::new(DashMap::new()),
            run_seq: Arc::new(AtomicU64::new(0)),
        }
    }

    // ── SQLite Project Environment + Keychain Secret CRUD ───────────────────

    pub async fn get_project_env(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
    ) -> Result<IdeProjectEnvironment> {
        target.validate()?;
        let (mut env, stored) = load_stored_project_env(pool, &target.project_key()).await?;
        env.env_vars = stored
            .into_iter()
            .map(|s| IdeEnvVarEntry {
                key: s.key,
                value: if s.secret { None } else { s.value },
                secret: s.secret,
                has_secret: s.secret && s.secret_ref.is_some(),
            })
            .collect();
        Ok(env)
    }

    pub async fn get_environments<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        _secrets: &S,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
    ) -> Result<IdeEnvironmentsResp> {
        target.validate()?;
        let env = self.get_project_env(pool, target).await?;
        let configs = self.list_run_configs(pool, target).await?;
        let dotenv_files = self
            .discover_dotenv_files(targets, target, ssh)
            .await
            .unwrap_or_default();
        let python_venvs = self
            .discover_python_venvs(targets, target, ssh)
            .await
            .unwrap_or_default();
        Ok(IdeEnvironmentsResp {
            env,
            python_venvs,
            dotenv_files,
            configs,
        })
    }

    pub async fn update_environment<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        req: IdeUpdateEnvironmentReq,
    ) -> Result<IdeProjectEnvironment> {
        req.target.validate()?;
        let project_key = req.target.project_key();
        let (_, prev_stored) = load_stored_project_env(pool, &project_key).await?;
        let prev_map: BTreeMap<String, StoredEnvVar> = prev_stored
            .into_iter()
            .map(|e| (e.key.clone(), e))
            .collect();

        let mut next_stored = Vec::with_capacity(req.env_vars.len());
        let mut kept_refs = HashSet::new();

        for item in &req.env_vars {
            let key = item.key.trim().to_string();
            if key.is_empty() || !is_valid_env_key(&key) {
                return Err(Error::Invalid(format!(
                    "invalid environment variable name '{key}'"
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

        let env_vars_json = serde_json::to_string(&next_stored)
            .map_err(|e| Error::Internal(format!("serialize env_vars_json: {e}")))?;
        let now = Utc::now().to_rfc3339();
        let target_kind = match req.target.kind {
            IdeTargetKind::Local => "local",
            IdeTargetKind::Ssh => "ssh",
        };

        sqlx::query(
            r#"INSERT INTO ide_project_environments (
                   project_key, target_kind, root_path, connection_id,
                   java_home, go_root, rust_toolchain, node_home,
                   python_interpreter, dotenv_path, extra_path, env_vars_json, updated_at
               ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
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
                   updated_at = excluded.updated_at"#,
        )
        .bind(&project_key)
        .bind(target_kind)
        .bind(&req.target.root)
        .bind(&req.target.connection_id)
        .bind(clean_opt(&req.java_home))
        .bind(clean_opt(&req.go_root))
        .bind(clean_opt(&req.rust_toolchain))
        .bind(clean_opt(&req.node_home))
        .bind(clean_opt(&req.python_interpreter))
        .bind(clean_opt(&req.dotenv_path))
        .bind(clean_opt(&req.extra_path))
        .bind(&env_vars_json)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| Error::Internal(format!("upsert ide_project_environments: {e}")))?;

        self.get_project_env(pool, &req.target).await
    }

    // ── Effective Environment & PATH Builder ─────────────────────────────────

    pub async fn effective_env<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
    ) -> Result<BTreeMap<String, String>> {
        self.build_effective_env(pool, secrets, target).await
    }

    pub async fn build_effective_env<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
    ) -> Result<BTreeMap<String, String>> {
        target.validate()?;
        let (proj, stored_vars) = load_stored_project_env(pool, &target.project_key()).await?;
        let mut map = BTreeMap::new();

        if target.kind == IdeTargetKind::Local {
            if let Some(dotenv_rel) = proj.dotenv_path.as_deref().filter(|s| !s.is_empty()) {
                if let Some(dotenv_file) = safe_join_local_root(&target.root, dotenv_rel) {
                    if let Ok(raw) = tokio::fs::read_to_string(&dotenv_file).await {
                        map.extend(parse_dotenv(&raw));
                    }
                }
            }
        }

        for item in &stored_vars {
            if item.secret {
                if let Some(sref) = &item.secret_ref {
                    if let Ok(Some(secret_val)) = secrets.get_secret(sref).await {
                        map.insert(item.key.clone(), secret_val);
                    }
                }
            } else if let Some(val) = &item.value {
                map.insert(item.key.clone(), val.clone());
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
        if let Some(py) = proj.python_interpreter.as_deref().filter(|s| !s.is_empty()) {
            if let Some(bin_dir) = Path::new(py).parent() {
                push_unique(&mut path_dirs, bin_dir.to_string_lossy().into_owned());
                if let Some(venv) = bin_dir.parent() {
                    if venv.join("pyvenv.cfg").is_file() || target.kind == IdeTargetKind::Ssh {
                        map.insert("VIRTUAL_ENV".into(), venv.to_string_lossy().into_owned());
                    }
                }
            }
        }
        if let Some(jh) = proj.java_home.as_deref().filter(|s| !s.is_empty()) {
            map.insert("JAVA_HOME".into(), jh.to_string());
            push_unique(&mut path_dirs, format!("{}/bin", jh.trim_end_matches('/')));
        }
        if let Some(gr) = proj.go_root.as_deref().filter(|s| !s.is_empty()) {
            map.insert("GOROOT".into(), gr.to_string());
            push_unique(&mut path_dirs, format!("{}/bin", gr.trim_end_matches('/')));
        }
        if let Some(rt) = proj.rust_toolchain.as_deref().filter(|s| !s.is_empty()) {
            if rt.starts_with('/') {
                push_unique(&mut path_dirs, format!("{}/bin", rt.trim_end_matches('/')));
            } else {
                map.insert("RUSTUP_TOOLCHAIN".into(), rt.to_string());
            }
        }
        if let Some(nh) = proj.node_home.as_deref().filter(|s| !s.is_empty()) {
            push_unique(&mut path_dirs, format!("{}/bin", nh.trim_end_matches('/')));
        }

        if target.kind == IdeTargetKind::Local {
            for d in crate::toolchains::ToolchainManager::new(self.data_dir.clone())
                .extra_local_bin_dirs()
            {
                push_unique(&mut path_dirs, d.to_string_lossy().into_owned());
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
        Ok(map)
    }

    // ── `.env` & Python Virtualenv Discovery + 1-Click Creation ──────────────

    pub async fn discover_dotenv_files(
        &self,
        targets: &TargetManager,
        target: &IdeTargetSpec,
        ssh: Option<&IdeSshSpec>,
    ) -> Result<Vec<String>> {
        const CANDIDATES: &[&str] = &[
            ".env",
            ".env.local",
            ".env.development",
            ".env.test",
            ".env.staging",
            ".env.production",
        ];
        match target.kind {
            IdeTargetKind::Local => {
                let root = PathBuf::from(&target.root);
                Ok(CANDIDATES
                    .iter()
                    .filter(|c| root.join(c).is_file())
                    .map(|c| (*c).to_string())
                    .collect())
            }
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
        let rel_dir = req
            .venv_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(".venv");
        if rel_dir.contains("..") || rel_dir.starts_with('/') {
            return Err(Error::Invalid(
                "venv_dir must be a safe workspace-relative directory".into(),
            ));
        }

        let resp = match req.target.kind {
            IdeTargetKind::Local => {
                self.create_local_venv(&req.target.root, rel_dir, req.python_version.as_deref())
                    .await?
            }
            IdeTargetKind::Ssh => {
                let spec = ssh.ok_or_else(|| Error::Invalid("SSH spec required".into()))?;
                let resolved = targets.resolve_with_ssh(&req.target, "ide-venv", spec.clone())?;
                let q_dir = shell_words::quote(rel_dir);
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
                        Some(&req.target.root),
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
                let venv_path = format!("{}/{rel_dir}", req.target.root.trim_end_matches('/'));
                IdeCreatePythonVenvResp {
                    interpreter_path: format!("{venv_path}/bin/python"),
                    path: venv_path,
                    version: ver,
                }
            }
        };

        let (existing, _) = load_stored_project_env(pool, &req.target.project_key()).await?;
        if existing.python_interpreter.is_none() {
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
            let py = find_executable_in(&search, &["python3", "python"])
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
        let ver_out = Command::new(&interp)
            .arg("--version")
            .kill_on_drop(true)
            .output()
            .await
            .ok();
        let version = ver_out
            .map(|o| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr)
                )
                .trim()
                .trim_start_matches("Python ")
                .to_string()
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "3".into());
        Ok(IdeCreatePythonVenvResp {
            path: venv_path.to_string_lossy().into_owned(),
            interpreter_path: interp.to_string_lossy().into_owned(),
            version,
        })
    }

    // ── Run Configurations SQLite CRUD (`ide_run_configs`) ───────────────────

    pub async fn list_run_configs(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
    ) -> Result<Vec<IdeRunConfig>> {
        target.validate()?;
        let rows = sqlx::query(
            r#"SELECT id, name, runner, command, args_json, cwd, env_overrides_json,
                      before_launch, target_path, test_filter
               FROM ide_run_configs
               WHERE project_key = ?
               ORDER BY created_at ASC, name ASC"#,
        )
        .bind(target.project_key())
        .fetch_all(pool)
        .await
        .map_err(|e| Error::Internal(format!("list ide_run_configs: {e}")))?;

        Ok(rows.iter().map(row_to_run_config).collect())
    }

    pub async fn get_run_config(&self, pool: &DbPool, id: &str) -> Result<IdeRunConfig> {
        let row = sqlx::query(
            r#"SELECT id, name, runner, command, args_json, cwd, env_overrides_json,
                      before_launch, target_path, test_filter
               FROM ide_run_configs WHERE id = ?"#,
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .map_err(|e| Error::Internal(format!("get ide_run_config: {e}")))?
        .ok_or_else(|| Error::NotFound(format!("run config '{id}'")))?;
        Ok(row_to_run_config(&row))
    }

    pub async fn upsert_run_config(
        &self,
        pool: &DbPool,
        req: IdeUpsertRunConfigReq,
    ) -> Result<IdeRunConfig> {
        self.upsert_run_config_parts(pool, &req.target, req.config)
            .await
    }

    pub async fn upsert_run_config_parts(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        mut cfg: IdeRunConfig,
    ) -> Result<IdeRunConfig> {
        target.validate()?;
        if cfg.name.trim().is_empty() {
            return Err(Error::Invalid(
                "run configuration name cannot be empty".into(),
            ));
        }
        if cfg.id.trim().is_empty() {
            cfg.id = format!("rc_{}", new_id());
        }
        let target_kind = match target.kind {
            IdeTargetKind::Local => "local",
            IdeTargetKind::Ssh => "ssh",
        };
        let runner_str = serde_json::to_value(cfg.runner)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "custom".into());
        let args_json = serde_json::to_string(&cfg.args).unwrap_or_else(|_| "[]".into());
        let env_overrides_json =
            serde_json::to_string(&cfg.env_overrides).unwrap_or_else(|_| "{}".into());
        let now = Utc::now().to_rfc3339();

        sqlx::query(
            r#"INSERT INTO ide_run_configs (
                   id, project_key, target_kind, connection_id, root_path,
                   name, runner, command, args_json, cwd,
                   env_overrides_json, before_launch, target_path, test_filter,
                   created_at, updated_at
               ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   project_key = excluded.project_key,
                   target_kind = excluded.target_kind,
                   connection_id = excluded.connection_id,
                   root_path = excluded.root_path,
                   name = excluded.name,
                   runner = excluded.runner,
                   command = excluded.command,
                   args_json = excluded.args_json,
                   cwd = excluded.cwd,
                   env_overrides_json = excluded.env_overrides_json,
                   before_launch = excluded.before_launch,
                   target_path = excluded.target_path,
                   test_filter = excluded.test_filter,
                   updated_at = excluded.updated_at"#,
        )
        .bind(&cfg.id)
        .bind(target.project_key())
        .bind(target_kind)
        .bind(&target.connection_id)
        .bind(&target.root)
        .bind(cfg.name.trim())
        .bind(&runner_str)
        .bind(clean_opt(&cfg.command))
        .bind(&args_json)
        .bind(clean_opt(&cfg.cwd))
        .bind(&env_overrides_json)
        .bind(clean_opt(&cfg.before_launch))
        .bind(clean_opt(&cfg.target_path))
        .bind(clean_opt(&cfg.test_filter))
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| Error::Internal(format!("upsert ide_run_config: {e}")))?;

        Ok(cfg)
    }

    pub async fn delete_run_config(&self, pool: &DbPool, id: &str) -> Result<()> {
        let res = sqlx::query("DELETE FROM ide_run_configs WHERE id = ?")
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| Error::Internal(format!("delete ide_run_config: {e}")))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("run config '{id}'")));
        }
        Ok(())
    }

    // ── Test & Run Execution Engine (Local + Remote SSH) ─────────────────────

    pub fn get_run(&self, run_id: &str) -> Result<IdeRunState> {
        self.get_run_with_tail(run_id, None)
    }

    pub fn get_run_with_tail(
        &self,
        run_id: &str,
        tail_lines: Option<usize>,
    ) -> Result<IdeRunState> {
        self.runs
            .get(run_id)
            .map(|e| e.value().snapshot_with_tail(tail_lines))
            .ok_or_else(|| Error::NotFound(format!("IDE run '{run_id}'")))
    }

    pub fn list_runs(&self) -> Vec<IdeRunState> {
        let mut out: Vec<IdeRunState> = self.runs.iter().map(|e| e.value().snapshot()).collect();
        out.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        out
    }

    pub async fn stop_run(&self, run_id: &str) -> Result<IdeRunState> {
        let snap = self
            .runs
            .get(run_id)
            .map(|e| e.value().stop())
            .ok_or_else(|| Error::NotFound(format!("IDE run '{run_id}'")))?;
        evict_finished_runs(&self.runs);
        Ok(snap)
    }

    pub async fn start_run<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        targets: &TargetManager,
        req: IdeStartRunReq,
        ssh: Option<IdeSshSpec>,
    ) -> Result<IdeRunState> {
        req.target.validate()?;
        let cfg = if let Some(adhoc) = req.adhoc {
            adhoc
        } else if let Some(cid) = req.config_id.as_deref().filter(|s| !s.is_empty()) {
            self.get_run_config(pool, cid).await?
        } else {
            return Err(Error::Invalid(
                "start_run requires either config_id or adhoc configuration".into(),
            ));
        };

        let mut env_map = self.build_effective_env(pool, secrets, &req.target).await?;
        for (k, v) in &cfg.env_overrides {
            env_map.insert(k.clone(), v.clone());
        }

        let run_id = format!("run_{}", new_id());
        let title = if cfg.name.trim().is_empty() {
            format!("{:?}", cfg.runner)
        } else {
            cfg.name.clone()
        };
        let started_at = Utc::now().to_rfc3339();
        let seq = self.run_seq.fetch_add(1, Ordering::Relaxed);
        let (stop_tx, stop_rx) = watch::channel(false);
        let handle = Arc::new(ActiveRunHandle {
            seq,
            state: Mutex::new(RunStateInner::new(
                run_id.clone(),
                title,
                cfg.runner,
                started_at,
            )),
            stop_tx,
        });
        let initial = handle.snapshot();
        self.runs.insert(run_id, Arc::clone(&handle));
        evict_finished_runs(&self.runs);

        let runs_clone = Arc::clone(&self.runs);
        let targets_clone = targets.clone();
        let target_spec = req.target.clone();
        tokio::spawn(async move {
            execute_run_task(
                runs_clone,
                handle,
                targets_clone,
                target_spec,
                ssh,
                cfg,
                env_map,
                stop_rx,
            )
            .await;
        });

        Ok(initial)
    }
}

// ── Pure `.env` Parser & SQLite / Filesystem Helpers ─────────────────────────

/// Safe, non-evaluating `.env` key-value parser supporting `#` comments,
/// `export KEY=VAL`, single/double quotes, and `\n` / `\"` escapes.
pub fn parse_dotenv(content: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
        let Some((raw_k, raw_v)) = line.split_once('=') else {
            continue;
        };
        let key = raw_k.trim();
        if !is_valid_env_key(key) {
            continue;
        }
        let v = raw_v.trim();
        let val = if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
            v[1..v.len() - 1]
                .replace("\\n", "\n")
                .replace("\\r", "\r")
                .replace("\\t", "\t")
                .replace("\\\"", "\"")
        } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
            v[1..v.len() - 1].to_string()
        } else {
            v.split(" #").next().unwrap_or(v).trim().to_string()
        };
        out.insert(key.to_string(), val);
    }
    out
}

pub(super) fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn secret_key_ref(project_key: &str, var_name: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in project_key.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("ide-env:{hash:016x}:{var_name}")
}

fn clean_opt(opt: &Option<String>) -> Option<String> {
    opt.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

fn push_unique(vec: &mut Vec<String>, item: String) {
    if !item.is_empty() && !vec.contains(&item) {
        vec.push(item);
    }
}

fn safe_join_local_root(root: &str, rel_or_abs: &str) -> Option<PathBuf> {
    let root_p = Path::new(root);
    let cand = if Path::new(rel_or_abs).is_absolute() {
        PathBuf::from(rel_or_abs)
    } else {
        if rel_or_abs.contains("..") {
            return None;
        }
        root_p.join(rel_or_abs)
    };
    if let (Ok(cr), Ok(cc)) = (std::fs::canonicalize(root_p), std::fs::canonicalize(&cand)) {
        return cc.starts_with(&cr).then_some(cc);
    }
    None
}

async fn load_stored_project_env(
    pool: &DbPool,
    project_key: &str,
) -> Result<(IdeProjectEnvironment, Vec<StoredEnvVar>)> {
    let row = sqlx::query(
        r#"SELECT java_home, go_root, rust_toolchain, node_home, python_interpreter,
                  dotenv_path, extra_path, env_vars_json
           FROM ide_project_environments WHERE project_key = ?"#,
    )
    .bind(project_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| Error::Internal(format!("query ide_project_environments: {e}")))?;

    let Some(row) = row else {
        return Ok((IdeProjectEnvironment::default(), Vec::new()));
    };
    let raw_json: String = row.try_get("env_vars_json").unwrap_or_else(|_| "[]".into());
    let stored: Vec<StoredEnvVar> = serde_json::from_str(&raw_json).unwrap_or_default();
    let env = IdeProjectEnvironment {
        java_home: row.try_get("java_home").ok().flatten(),
        go_root: row.try_get("go_root").ok().flatten(),
        rust_toolchain: row.try_get("rust_toolchain").ok().flatten(),
        node_home: row.try_get("node_home").ok().flatten(),
        python_interpreter: row.try_get("python_interpreter").ok().flatten(),
        dotenv_path: row.try_get("dotenv_path").ok().flatten(),
        extra_path: row.try_get("extra_path").ok().flatten(),
        env_vars: Vec::new(),
    };
    Ok((env, stored))
}

fn row_to_run_config(row: &sqlx::sqlite::SqliteRow) -> IdeRunConfig {
    let runner_s: String = row.try_get("runner").unwrap_or_else(|_| "custom".into());
    let runner: IdeRunnerKind = serde_json::from_value(serde_json::Value::String(runner_s))
        .unwrap_or(IdeRunnerKind::Custom);
    let args_s: String = row.try_get("args_json").unwrap_or_else(|_| "[]".into());
    let env_s: String = row
        .try_get("env_overrides_json")
        .unwrap_or_else(|_| "{}".into());
    IdeRunConfig {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        runner,
        command: row.try_get("command").ok().flatten(),
        args: serde_json::from_str(&args_s).unwrap_or_default(),
        cwd: row.try_get("cwd").ok().flatten(),
        env_overrides: serde_json::from_str(&env_s).unwrap_or_default(),
        before_launch: row.try_get("before_launch").ok().flatten(),
        target_path: row.try_get("target_path").ok().flatten(),
        test_filter: row.try_get("test_filter").ok().flatten(),
    }
}

fn discover_local_python_venvs(root: &Path, data_dir: &Path) -> Vec<IdePythonVenvInfo> {
    let mut out = Vec::new();
    for dir_name in [".venv", "venv", "env"] {
        let p = root.join(dir_name);
        if p.join("bin/python").is_file() {
            let kind = if p.join("pyvenv.cfg").is_file()
                && std::fs::read_to_string(p.join("pyvenv.cfg"))
                    .map(|c| c.contains("uv ="))
                    .unwrap_or(false)
            {
                "uv"
            } else {
                "venv"
            };
            out.push(IdePythonVenvInfo {
                version: read_pyvenv_version(&p).unwrap_or_else(|| "3".into()),
                path: p.to_string_lossy().into_owned(),
                kind: kind.into(),
            });
        }
    }
    for py_dir in read_subdirs(&data_dir.join("toolchains/python/interpreters")) {
        if py_dir.join("bin/python3").is_file() {
            out.push(IdePythonVenvInfo {
                version: py_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("3")
                    .to_string(),
                path: py_dir.to_string_lossy().into_owned(),
                kind: "uv".into(),
            });
        }
    }
    if !otto_core::e2e::isolated() {
        if let Some(home) = dirs::home_dir() {
            for (kind, parent) in [
                ("conda", home.join(".conda/envs")),
                ("conda", home.join("miniconda3/envs")),
                ("poetry", home.join("Library/Caches/pypoetry/virtualenvs")),
                ("poetry", home.join(".cache/pypoetry/virtualenvs")),
            ] {
                for d in read_subdirs(&parent) {
                    if d.join("bin/python").is_file() {
                        out.push(IdePythonVenvInfo {
                            version: read_pyvenv_version(&d).unwrap_or_else(|| "3".into()),
                            path: d.to_string_lossy().into_owned(),
                            kind: kind.into(),
                        });
                    }
                }
            }
        }
    }
    out
}

fn read_pyvenv_version(venv: &Path) -> Option<String> {
    let cfg = std::fs::read_to_string(venv.join("pyvenv.cfg")).ok()?;
    cfg.lines()
        .find_map(|l| {
            let (k, v) = l.split_once('=')?;
            matches!(k.trim(), "version" | "version_info").then(|| v.trim().to_string())
        })
        .filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemSecrets(Mutex<HashMap<String, String>>);

    impl SecretStore for MemSecrets {
        fn put(&self, key: &str, value: &str) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }

        fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }

        fn delete(&self, key: &str) -> Result<()> {
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    #[tokio::test]
    async fn sqlite_crud_redacts_secrets_and_injects_effective_env() {
        let tmp = tempfile::tempdir().unwrap();
        let pool = otto_state::db::open(&tmp.path().join("test.db"))
            .await
            .unwrap();
        let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
        std::fs::write(
            tmp.path().join(".env"),
            "FROM_DOTENV=\"hello\\nworld\"\nAPP_ENV=dev\n",
        )
        .unwrap();

        let mgr = EnvironmentManager::new(tmp.path().join("data"));
        let target = IdeTargetSpec {
            kind: IdeTargetKind::Local,
            root: tmp.path().to_string_lossy().into_owned(),
            workspace_id: None,
            connection_id: None,
        };

        let saved = mgr
            .update_environment(
                &pool,
                &secrets,
                IdeUpdateEnvironmentReq {
                    target: target.clone(),
                    java_home: Some("/opt/jdk-21".into()),
                    go_root: Some("/opt/go-1.24".into()),
                    rust_toolchain: Some("stable".into()),
                    node_home: None,
                    python_interpreter: None,
                    dotenv_path: Some(".env".into()),
                    extra_path: Some("/custom/bin".into()),
                    env_vars: vec![
                        IdeEnvVarEntry {
                            key: "APP_ENV".into(),
                            value: Some("staging".into()),
                            secret: false,
                            has_secret: false,
                        },
                        IdeEnvVarEntry {
                            key: "STRIPE_API_SECRET".into(),
                            value: Some("sk_test_super_secret".into()),
                            secret: true,
                            has_secret: false,
                        },
                    ],
                },
            )
            .await
            .unwrap();

        let sec_entry = saved
            .env_vars
            .iter()
            .find(|e| e.key == "STRIPE_API_SECRET")
            .unwrap();
        assert!(sec_entry.has_secret);
        assert_eq!(sec_entry.value, None);

        let raw: String = sqlx::query_scalar("SELECT env_vars_json FROM ide_project_environments")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(!raw.contains("sk_test_super_secret"));

        let eff = mgr
            .effective_env(&pool, secrets.as_ref(), &target)
            .await
            .unwrap();
        assert_eq!(eff.get("FROM_DOTENV").unwrap(), "hello\nworld");
        assert_eq!(eff.get("APP_ENV").unwrap(), "staging");
        assert_eq!(
            eff.get("STRIPE_API_SECRET").unwrap(),
            "sk_test_super_secret"
        );
        assert_eq!(eff.get("JAVA_HOME").unwrap(), "/opt/jdk-21");
        assert!(eff.get("PATH").unwrap().starts_with("/custom/bin:"));

        let rc = mgr
            .upsert_run_config(
                &pool,
                IdeUpsertRunConfigReq {
                    target: target.clone(),
                    config: IdeRunConfig {
                        id: "rc_1".into(),
                        name: "Go Suite".into(),
                        runner: IdeRunnerKind::GoTest,
                        command: None,
                        args: vec!["./calc/...".into()],
                        cwd: None,
                        env_overrides: BTreeMap::new(),
                        before_launch: None,
                        target_path: None,
                        test_filter: None,
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(mgr.list_run_configs(&pool, &target).await.unwrap().len(), 1);
        mgr.delete_run_config(&pool, &rc.id).await.unwrap();
        assert!(mgr
            .list_run_configs(&pool, &target)
            .await
            .unwrap()
            .is_empty());
    }

    #[test]
    fn finished_runs_evict_cleanly_past_max_finished_runs_preserving_running() {
        use super::runner::MAX_FINISHED_RUNS;
        use crate::types::IdeRunStatus;

        let mgr = EnvironmentManager::new(PathBuf::from("/tmp/otto-ide-test"));
        for i in 0..24u64 {
            let (stop_tx, _) = watch::channel(false);
            let id = format!("run_fin_{i:02}");
            let handle = Arc::new(ActiveRunHandle {
                seq: i,
                state: Mutex::new(RunStateInner::new(
                    id.clone(),
                    "Finished".into(),
                    IdeRunnerKind::Custom,
                    format!("2026-01-01T00:00:{i:02}Z"),
                )),
                stop_tx,
            });
            runner::finish_run(
                &handle,
                IdeRunStatus::Passed,
                Some(0),
                Duration::from_millis(5),
                None,
            );
            mgr.runs.insert(id, handle);
            evict_finished_runs(&mgr.runs);
        }

        let (stop_tx, _) = watch::channel(false);
        mgr.runs.insert(
            "run_active".into(),
            Arc::new(ActiveRunHandle {
                seq: 100,
                state: Mutex::new(RunStateInner::new(
                    "run_active".into(),
                    "Active".into(),
                    IdeRunnerKind::Custom,
                    "2026-01-01T00:00:00Z".into(),
                )),
                stop_tx,
            }),
        );
        evict_finished_runs(&mgr.runs);

        assert_eq!(mgr.runs.len(), MAX_FINISHED_RUNS + 1);
        assert!(mgr.get_run("run_active").is_ok());
        for i in 0..8 {
            assert!(mgr.get_run(&format!("run_fin_{i:02}")).is_err());
        }
        for i in 8..24 {
            assert!(mgr.get_run(&format!("run_fin_{i:02}")).is_ok());
        }
    }
}
