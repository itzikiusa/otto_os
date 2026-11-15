//! Run Configurations CRUD and Active Run Session management for [`super::EnvironmentManager`].

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use chrono::Utc;
use otto_core::domain::User;
use otto_core::{new_id, Error, Result};
use otto_state::DbPool;
use sqlx::Row;
use tokio::sync::{broadcast, watch};

use super::db_store::{
    clean_opt, is_dangerous_loader_env_key, is_reserved_internal_env_key, row_to_run_config,
    RESERVED_RUN_COVERAGE_KEY,
};
use super::discovery::discover_run_configs;
use super::runner::{evict_finished_runs, execute_run_task, ActiveRunHandle, RunStateInner};
use super::{EnvironmentManager, SecretAccess};
use crate::projects::ProjectStore;
use crate::target::TargetManager;
use crate::types::{
    IdeRunConfig, IdeRunState, IdeRunnerKind, IdeSshSpec, IdeStartRunReq, IdeTargetKind,
    IdeTargetSpec, IdeUpsertRunConfigReq,
};

const SELECT_RUN_CONFIG_SQL: &str = r#"SELECT id, name, runner, command, args_json, build_flags_json, cwd,
       env_overrides_json, env_file, inherit_shell_env, sdk_path,
       toolchain_id, before_launch, target_path, test_filter, compound_ids_json,
       maven_profiles_json, spring_profiles_json, skip_tests, offline
FROM ide_run_configs"#;

impl EnvironmentManager {
    pub async fn list_run_configs(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
    ) -> Result<Vec<IdeRunConfig>> {
        target.validate()?;
        let sql = format!(
            "{SELECT_RUN_CONFIG_SQL} WHERE project_key = ? ORDER BY created_at ASC, name ASC"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(target.project_key())
            .fetch_all(pool)
            .await
            .map_err(|e| Error::Internal(format!("list ide_run_configs: {e}")))?;

        Ok(rows.iter().map(row_to_run_config).collect())
    }

    pub async fn get_run_config(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        id: &str,
    ) -> Result<IdeRunConfig> {
        target.validate()?;
        let sql = format!("{SELECT_RUN_CONFIG_SQL} WHERE id = ? AND project_key = ?");
        let row = sqlx::query(sqlx::AssertSqlSafe(sql))
            .bind(id)
            .bind(target.project_key())
            .fetch_optional(pool)
            .await
            .map_err(|e| Error::Internal(format!("get ide_run_config: {e}")))?
            .ok_or_else(|| Error::NotFound(format!("run config '{id}'")))?;
        Ok(row_to_run_config(&row))
    }

    pub async fn get_run_config_target(
        &self,
        pool: &DbPool,
        target_opt: Option<&IdeTargetSpec>,
        id: &str,
    ) -> Result<IdeTargetSpec> {
        let row = match target_opt {
            Some(target) => {
                target.validate()?;
                sqlx::query(
                    "SELECT target_kind, root_path, connection_id FROM ide_run_configs WHERE id = ? AND project_key = ?",
                )
                .bind(id)
                .bind(target.project_key())
                .fetch_optional(pool)
                .await
            }
            None => {
                sqlx::query(
                    "SELECT target_kind, root_path, connection_id FROM ide_run_configs WHERE id = ?",
                )
                .bind(id)
                .fetch_optional(pool)
                .await
            }
        }
        .map_err(|e| Error::Internal(format!("get ide_run_config target: {e}")))?
        .ok_or_else(|| Error::NotFound(format!("run config '{id}'")))?;

        let kind_str: String = row.try_get("target_kind").unwrap_or_default();
        let kind = if kind_str == "ssh" {
            IdeTargetKind::Ssh
        } else {
            IdeTargetKind::Local
        };
        Ok(IdeTargetSpec {
            kind,
            root: row.try_get("root_path").unwrap_or_default(),
            workspace_id: None,
            connection_id: row.try_get("connection_id").ok().flatten(),
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        })
    }

    pub async fn resolve_run_config_or_auto(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        id: &str,
    ) -> Result<IdeRunConfig> {
        if let Ok(saved) = self.get_run_config(pool, target, id).await {
            return Ok(saved);
        }
        if id.starts_with("auto:") && target.kind == IdeTargetKind::Local {
            let mut enriched = target.clone();
            self.enrich_target_roots(pool, None, None, &mut enriched)
                .await;
            let extra = enriched.additional_roots.clone();
            let id_owned = id.to_string();
            let found = tokio::task::spawn_blocking(move || {
                discover_run_configs(&enriched, &extra)
                    .into_iter()
                    .find(|c| c.id == id_owned)
            })
            .await
            .ok()
            .flatten();
            if let Some(cfg) = found {
                return Ok(cfg);
            }
        }
        Err(Error::NotFound(format!("run config '{id}'")))
    }

    pub async fn upsert_run_config(
        &self,
        pool: &DbPool,
        req: IdeUpsertRunConfigReq,
    ) -> Result<IdeRunConfig> {
        self.upsert_run_config_parts_for_user(pool, &req.target, req.config, true)
            .await
    }

    pub async fn upsert_run_config_for_user(
        &self,
        pool: &DbPool,
        req: IdeUpsertRunConfigReq,
        caller_is_root: bool,
    ) -> Result<IdeRunConfig> {
        self.upsert_run_config_parts_for_user(pool, &req.target, req.config, caller_is_root)
            .await
    }

    /// Upsert a run configuration scoped strictly to `target.project_key()` so
    /// reusing or guessing an existing `id` can never overwrite another workspace's row.
    pub async fn upsert_run_config_parts(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        cfg: IdeRunConfig,
    ) -> Result<IdeRunConfig> {
        self.upsert_run_config_parts_for_user(pool, target, cfg, true)
            .await
    }

    pub async fn upsert_run_config_parts_for_user(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        mut cfg: IdeRunConfig,
        caller_is_root: bool,
    ) -> Result<IdeRunConfig> {
        target.validate()?;
        if cfg.name.trim().is_empty() {
            return Err(Error::Invalid(
                "run configuration name cannot be empty".into(),
            ));
        }
        for k in cfg.env_overrides.keys() {
            if is_reserved_internal_env_key(k) {
                return Err(Error::Invalid(format!(
                    "reserved environment variable key '{k}'"
                )));
            }
        }
        if !caller_is_root {
            validate_run_config_for_non_root(target, &cfg)?;
        }
        if cfg.id.trim().is_empty() || cfg.id.starts_with("auto:") {
            cfg.id = format!("rc_{}", new_id());
        }
        cfg.auto_detected = false;
        let runner_str = serde_json::to_value(cfg.runner)
            .ok()
            .and_then(|v| v.as_str().map(str::to_string))
            .unwrap_or_else(|| "custom".into());
        let args_json = serde_json::to_string(&cfg.args).unwrap_or_else(|_| "[]".into());
        let build_flags_json =
            serde_json::to_string(&cfg.build_flags).unwrap_or_else(|_| "[]".into());
        let mut persisted_env_overrides = cfg.env_overrides.clone();
        if cfg.coverage {
            persisted_env_overrides.insert(RESERVED_RUN_COVERAGE_KEY.into(), "1".into());
        }
        let env_overrides_json =
            serde_json::to_string(&persisted_env_overrides).unwrap_or_else(|_| "{}".into());
        let compound_ids_json =
            serde_json::to_string(&cfg.compound_configs).unwrap_or_else(|_| "[]".into());
        let maven_profiles_json =
            serde_json::to_string(&cfg.maven_profiles).unwrap_or_else(|_| "[]".into());
        let spring_profiles_json =
            serde_json::to_string(&cfg.spring_profiles).unwrap_or_else(|_| "[]".into());
        let now = Utc::now().to_rfc3339();

        let res = sqlx::query(
            r#"INSERT INTO ide_run_configs (
                   id, project_key, target_kind, connection_id, root_path, name, runner, command,
                   args_json, build_flags_json, cwd, env_overrides_json, env_file, inherit_shell_env,
                   sdk_path, toolchain_id, before_launch, target_path, test_filter, compound_ids_json,
                   maven_profiles_json, spring_profiles_json, skip_tests, offline, created_at, updated_at
               ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
               ON CONFLICT(id) DO UPDATE SET
                   target_kind = excluded.target_kind, connection_id = excluded.connection_id,
                   root_path = excluded.root_path, name = excluded.name, runner = excluded.runner,
                   command = excluded.command, args_json = excluded.args_json,
                   build_flags_json = excluded.build_flags_json, cwd = excluded.cwd,
                   env_overrides_json = excluded.env_overrides_json, env_file = excluded.env_file,
                   inherit_shell_env = excluded.inherit_shell_env, sdk_path = excluded.sdk_path,
                   toolchain_id = excluded.toolchain_id, before_launch = excluded.before_launch,
                   target_path = excluded.target_path, test_filter = excluded.test_filter,
                   compound_ids_json = excluded.compound_ids_json,
                   maven_profiles_json = excluded.maven_profiles_json,
                   spring_profiles_json = excluded.spring_profiles_json,
                   skip_tests = excluded.skip_tests, offline = excluded.offline,
                   updated_at = excluded.updated_at
               WHERE ide_run_configs.project_key = excluded.project_key"#,
        )
        .bind(&cfg.id)
        .bind(target.project_key())
        .bind(target.kind.as_str())
        .bind(&target.connection_id)
        .bind(&target.root)
        .bind(cfg.name.trim())
        .bind(&runner_str)
        .bind(clean_opt(&cfg.command))
        .bind(&args_json)
        .bind(&build_flags_json)
        .bind(clean_opt(&cfg.cwd))
        .bind(&env_overrides_json)
        .bind(clean_opt(&cfg.env_file))
        .bind(i32::from(cfg.inherit_shell_env))
        .bind(clean_opt(&cfg.sdk_path))
        .bind(clean_opt(&cfg.toolchain_id))
        .bind(clean_opt(&cfg.before_launch))
        .bind(clean_opt(&cfg.target_path))
        .bind(clean_opt(&cfg.test_filter))
        .bind(&compound_ids_json)
        .bind(&maven_profiles_json)
        .bind(&spring_profiles_json)
        .bind(i32::from(cfg.skip_tests))
        .bind(i32::from(cfg.offline))
        .bind(&now)
        .bind(&now)
        .execute(pool)
        .await
        .map_err(|e| Error::Internal(format!("upsert ide_run_config: {e}")))?;

        if res.rows_affected() == 0 {
            return Err(Error::Forbidden(format!(
                "run configuration '{}' belongs to another workspace",
                cfg.id
            )));
        }

        Ok(cfg)
    }

    pub async fn delete_run_config(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        id: &str,
    ) -> Result<()> {
        target.validate()?;
        let res = sqlx::query("DELETE FROM ide_run_configs WHERE id = ? AND project_key = ?")
            .bind(id)
            .bind(target.project_key())
            .execute(pool)
            .await
            .map_err(|e| Error::Internal(format!("delete ide_run_config: {e}")))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("run config '{id}'")));
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn get_run(&self, run_id: &str) -> Result<IdeRunState> {
        self.get_run_with_tail(run_id, None)
    }

    #[cfg(test)]
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

    pub fn get_run_for_user(
        &self,
        user: &User,
        run_id: &str,
        tail_lines: Option<usize>,
    ) -> Result<IdeRunState> {
        let entry = self
            .runs
            .get(run_id)
            .ok_or_else(|| Error::NotFound(format!("IDE run '{run_id}'")))?;
        if !can_access_run(entry.value(), user) {
            return Err(Error::Forbidden(
                "cannot access an IDE run owned by another user".into(),
            ));
        }
        Ok(entry.value().snapshot_with_tail(tail_lines))
    }

    /// Subscribe to real-time SSE output and status frames for `run_id` after
    /// enforcing the same run-owner check as [`Self::get_run_for_user`].
    pub fn subscribe_run_for_user(
        &self,
        user: &User,
        run_id: &str,
    ) -> Result<(IdeRunState, broadcast::Receiver<IdeRunState>)> {
        let entry = self
            .runs
            .get(run_id)
            .ok_or_else(|| Error::NotFound(format!("IDE run '{run_id}'")))?;
        if !can_access_run(entry.value(), user) {
            return Err(Error::Forbidden(
                "cannot access an IDE run owned by another user".into(),
            ));
        }
        let rx = entry.value().events_tx.subscribe();
        let snap = entry.value().snapshot_with_tail(Some(500));
        Ok((snap, rx))
    }

    pub async fn stop_run_for_user(&self, user: &User, run_id: &str) -> Result<IdeRunState> {
        let snap = {
            let entry = self
                .runs
                .get(run_id)
                .ok_or_else(|| Error::NotFound(format!("IDE run '{run_id}'")))?;
            if !can_access_run(entry.value(), user) {
                return Err(Error::Forbidden(
                    "cannot stop an IDE run owned by another user".into(),
                ));
            }
            entry.value().stop()
        };
        evict_finished_runs(&self.runs);
        Ok(snap)
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start_run_for_user<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        targets: &TargetManager,
        projects: Option<&ProjectStore>,
        user: Option<&User>,
        req: IdeStartRunReq,
        ssh: Option<IdeSshSpec>,
    ) -> Result<IdeRunState> {
        self.start_run_with_netbridge(pool, secrets, targets, projects, user, req, ssh, None)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn start_run_with_netbridge<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        targets: &TargetManager,
        projects: Option<&ProjectStore>,
        user: Option<&User>,
        mut req: IdeStartRunReq,
        ssh: Option<IdeSshSpec>,
        netbridge: Option<Arc<crate::netbridge::NetBridgeLease>>,
    ) -> Result<IdeRunState> {
        req.target.validate()?;
        for pf in &req.project_folders {
            if !pf.is_empty() && !req.target.project_folders.contains(pf) {
                req.target.project_folders.push(pf.clone());
            }
        }
        self.enrich_target_roots(pool, projects, user, &mut req.target)
            .await;

        let mut cfg = if let Some(adhoc) = req.adhoc {
            adhoc
        } else if let Some(cid) = req.config_id.as_deref().filter(|s| !s.is_empty()) {
            self.resolve_run_config_or_auto(pool, &req.target, cid)
                .await?
        } else {
            return Err(Error::Invalid(
                "start_run requires either config_id or adhoc configuration".into(),
            ));
        };

        for mp in req.maven_profiles {
            if !mp.is_empty() && !cfg.maven_profiles.contains(&mp) {
                cfg.maven_profiles.push(mp);
            }
        }
        for sp in req.spring_profiles {
            if !sp.is_empty() && !cfg.spring_profiles.contains(&sp) {
                cfg.spring_profiles.push(sp);
            }
        }
        if req.skip_tests {
            cfg.skip_tests = true;
        }
        if req.offline {
            cfg.offline = true;
        }
        if req.coverage {
            cfg.coverage = true;
        }

        let caller_is_root = user.map(|u| u.is_root).unwrap_or(true);
        if !caller_is_root {
            validate_run_config_for_non_root(&req.target, &cfg)?;
        }
        let mut steps: Vec<(IdeRunConfig, BTreeMap<String, String>)> = Vec::new();
        if cfg.runner == IdeRunnerKind::Compound && !cfg.compound_configs.is_empty() {
            for member_id in cfg.compound_configs.iter().take(12) {
                if member_id == &cfg.id {
                    continue;
                }
                let mut member_cfg = self
                    .resolve_run_config_or_auto(pool, &req.target, member_id)
                    .await?;
                if member_cfg.runner == IdeRunnerKind::Compound {
                    continue;
                }
                if !caller_is_root {
                    validate_run_config_for_non_root(&req.target, &member_cfg)?;
                }
                if member_cfg.maven_profiles.is_empty() {
                    member_cfg.maven_profiles = cfg.maven_profiles.clone();
                }
                if member_cfg.spring_profiles.is_empty() {
                    member_cfg.spring_profiles = cfg.spring_profiles.clone();
                }
                member_cfg.skip_tests |= cfg.skip_tests;
                member_cfg.offline |= cfg.offline;
                member_cfg.coverage |= cfg.coverage;
                let (member_proj, member_env) = self
                    .build_effective_env_with_loaded(
                        pool,
                        secrets,
                        &req.target,
                        Some(&member_cfg),
                        caller_is_root,
                    )
                    .await?;
                apply_project_go_build_tags_to_cfg(&mut member_cfg, &member_proj.go_build_tags);
                steps.push((member_cfg, member_env));
            }
            if steps.is_empty() {
                return Err(Error::Invalid(
                    "compound run configuration contains no runnable steps".into(),
                ));
            }
        } else {
            let (step_proj, env_map) = self
                .build_effective_env_with_loaded(
                    pool,
                    secrets,
                    &req.target,
                    Some(&cfg),
                    caller_is_root,
                )
                .await?;
            apply_project_go_build_tags_to_cfg(&mut cfg, &step_proj.go_build_tags);
            steps.push((cfg.clone(), env_map));
        }

        let run_id = format!("run_{}", new_id());
        let mut title = if cfg.name.trim().is_empty() {
            format!("{:?}", cfg.runner)
        } else {
            cfg.name.clone()
        };
        if cfg.coverage && !title.contains("(Coverage)") {
            title.push_str(" (Coverage)");
        }
        let started_at = Utc::now().to_rfc3339();
        let seq = self.run_seq.fetch_add(1, Ordering::Relaxed);
        let (stop_tx, stop_rx) = watch::channel(false);
        let (events_tx, _) = broadcast::channel(256);
        let owner_user_id = user.map(|u| u.id.clone()).unwrap_or_default();
        let handle = Arc::new(ActiveRunHandle {
            seq,
            owner_user_id,
            state: Mutex::new(RunStateInner::new(
                run_id.clone(),
                title,
                cfg.runner,
                started_at,
            )),
            stop_tx,
            events_tx,
        });
        let initial = handle.snapshot();
        self.runs.insert(run_id, Arc::clone(&handle));
        evict_finished_runs(&self.runs);

        let runs_clone = Arc::clone(&self.runs);
        let targets_clone = targets.clone();
        let target_spec = req.target;
        tokio::spawn(async move {
            execute_run_task(
                runs_clone,
                handle,
                targets_clone,
                target_spec,
                ssh,
                steps,
                netbridge,
                stop_rx,
            )
            .await;
        });

        Ok(initial)
    }
}

fn can_access_run(handle: &ActiveRunHandle, user: &User) -> bool {
    user.is_root || (!handle.owner_user_id.is_empty() && handle.owner_user_id == user.id)
}

fn apply_project_go_build_tags_to_cfg(cfg: &mut IdeRunConfig, tags: &[String]) {
    if !tags.is_empty()
        && matches!(
            cfg.runner,
            IdeRunnerKind::GoTest | IdeRunnerKind::GoRun | IdeRunnerKind::GoBuild
        )
    {
        crate::lsp_service::merge_go_tags_into_args(&mut cfg.build_flags, tags);
    }
}

fn is_allowed_standard_runner_command(runner: IdeRunnerKind, cmd: &str) -> bool {
    match runner {
        IdeRunnerKind::GoTest | IdeRunnerKind::GoRun | IdeRunnerKind::GoBuild => cmd == "go",
        IdeRunnerKind::CargoTest | IdeRunnerKind::CargoRun => cmd == "cargo",
        IdeRunnerKind::Python => matches!(cmd, "python" | "python3"),
        IdeRunnerKind::Pytest => matches!(cmd, "pytest" | "python" | "python3"),
        IdeRunnerKind::Node => cmd == "node",
        IdeRunnerKind::Npm => cmd == "npm",
        IdeRunnerKind::Vitest => matches!(cmd, "npx" | "vitest"),
        IdeRunnerKind::JavaMain => cmd == "java",
        IdeRunnerKind::JavaJunit | IdeRunnerKind::Maven => matches!(cmd, "mvn" | "./mvnw"),
        IdeRunnerKind::Gradle => matches!(cmd, "gradle" | "./gradlew"),
        IdeRunnerKind::Shell => matches!(cmd, "bash" | "sh" | "bats"),
        IdeRunnerKind::Compound => true,
        IdeRunnerKind::Custom => false,
    }
}

#[allow(clippy::disallowed_methods)]
pub(super) fn validate_target_confined_subpath(
    target: &IdeTargetSpec,
    raw_subpath: &str,
    field_name: &str,
) -> Result<()> {
    let trimmed = raw_subpath.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == "./..." || trimmed == "..." {
        return Ok(());
    }
    if trimmed.chars().any(char::is_control) {
        return Err(Error::Invalid(format!(
            "invalid control characters in '{field_name}'"
        )));
    }
    let normalized = trimmed
        .strip_suffix("/...")
        .unwrap_or(trimmed)
        .trim_end_matches('/');
    if normalized.is_empty() || normalized == "." {
        return Ok(());
    }
    if Path::new(normalized)
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(Error::Forbidden(format!(
            "'{field_name}' cannot traverse outside workspace roots"
        )));
    }

    let mut allowed = vec![target.root.clone()];
    for extra in target.merged_extra_roots() {
        if !extra.trim().is_empty() && !allowed.contains(&extra) {
            allowed.push(extra);
        }
    }

    match target.kind {
        IdeTargetKind::Local => {
            let cand = if normalized.starts_with('~') {
                otto_core::paths::expand_home_strict(normalized).map_err(Error::Forbidden)?
            } else if Path::new(normalized).is_absolute() {
                PathBuf::from(normalized)
            } else {
                Path::new(&target.root).join(normalized)
            };
            let canon = std::fs::canonicalize(&cand)
                .ok()
                .or_else(|| otto_core::secret_paths::canonicalize_prospective(&cand))
                .ok_or_else(|| {
                    Error::Forbidden(format!("'{field_name}' could not be confined to workspace"))
                })?;
            let in_roots = allowed.iter().any(|r| {
                let cr = std::fs::canonicalize(r).unwrap_or_else(|_| PathBuf::from(r));
                canon.starts_with(&cr)
            });
            if !in_roots {
                return Err(Error::Forbidden(format!(
                    "'{field_name}' escapes authorized workspace roots"
                )));
            }
            crate::target::enforce_non_root_path(&canon)?;
            Ok(())
        }
        IdeTargetKind::Ssh => {
            if normalized.starts_with('~') {
                return Err(Error::Forbidden(format!(
                    "'{field_name}' escapes remote workspace roots"
                )));
            }
            if normalized.starts_with('/') {
                let in_roots = allowed.iter().any(|r| {
                    let rt = r.trim().trim_end_matches('/');
                    !rt.is_empty()
                        && (normalized == rt || normalized.starts_with(&format!("{rt}/")))
                });
                if !in_roots {
                    return Err(Error::Forbidden(format!(
                        "'{field_name}' escapes remote workspace roots"
                    )));
                }
            }
            Ok(())
        }
    }
}

pub(super) fn validate_run_config_for_non_root(
    target: &IdeTargetSpec,
    cfg: &IdeRunConfig,
) -> Result<()> {
    if clean_opt(&cfg.sdk_path).is_some() {
        return Err(Error::Forbidden(
            "non-root users cannot override 'sdk_path' on run configurations".into(),
        ));
    }
    if clean_opt(&cfg.before_launch).is_some() {
        return Err(Error::Forbidden(
            "non-root users cannot configure 'before_launch' hooks".into(),
        ));
    }
    if cfg.runner == IdeRunnerKind::Custom {
        return Err(Error::Forbidden(
            "non-root users cannot execute custom command runners".into(),
        ));
    }
    if let Some(cmd) = clean_opt(&cfg.command) {
        if !is_allowed_standard_runner_command(cfg.runner, &cmd) {
            return Err(Error::Forbidden(format!(
                "non-root users cannot override runner binary to '{cmd}'"
            )));
        }
    }
    for k in cfg.env_overrides.keys() {
        if is_dangerous_loader_env_key(k) {
            return Err(Error::Forbidden(format!(
                "non-root users cannot set loader environment variable '{k}'"
            )));
        }
    }
    if let Some(cwd) = clean_opt(&cfg.cwd) {
        validate_target_confined_subpath(target, &cwd, "cwd")?;
    }
    if let Some(tp) = clean_opt(&cfg.target_path) {
        validate_target_confined_subpath(target, &tp, "target_path")?;
    }
    if let Some(ef) = clean_opt(&cfg.env_file) {
        validate_target_confined_subpath(target, &ef, "env_file")?;
    }
    Ok(())
}
