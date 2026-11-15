//! Project environment read/folder-attachment helpers and effective-environment
//! loader entry points for [`super::EnvironmentManager`].

use std::collections::BTreeMap;

use otto_core::domain::User;
use otto_core::{Error, Result};
use otto_state::DbPool;

use super::db_store::{
    is_reserved_internal_env_key, load_stored_project_env, persist_project_roots,
    validate_attached_folder_path, StoredEnvVar,
};
use super::module_sdks::layer_child_and_module_sdk_for_run;
use super::{EnvironmentManager, SecretAccess};
use crate::projects::ProjectStore;
use crate::types::{
    IdeEnvVarEntry, IdeProjectEnvironment, IdeRunConfig, IdeTargetKind, IdeTargetSpec,
};

pub(super) fn redact_stored_vars(stored: &[StoredEnvVar]) -> Vec<IdeEnvVarEntry> {
    stored
        .iter()
        .filter(|s| !is_reserved_internal_env_key(&s.key))
        .map(|s| IdeEnvVarEntry {
            key: s.key.clone(),
            value: if s.secret { None } else { s.value.clone() },
            secret: s.secret,
            has_secret: s.secret && s.secret_ref.is_some(),
        })
        .collect()
}

impl EnvironmentManager {
    pub async fn get_project_env(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
    ) -> Result<IdeProjectEnvironment> {
        target.validate()?;
        let project_key = target.project_key();
        let (mut env, stored) = load_stored_project_env(pool, &project_key).await?;
        self.record_snapshot(&project_key, &env, &stored);
        env.env_vars = redact_stored_vars(&stored);
        Ok(env)
    }

    /// Hydrate `target.additional_roots` with server-stored folders while filtering
    /// each local secondary root against `user`'s RBAC so an unpermitted sibling root
    /// never poisons primary workspace resolution with `403 Forbidden`.
    pub async fn enrich_target_roots_authorized<S: crate::IdeCtx>(
        &self,
        ctx: &S,
        user: &User,
        target: &mut IdeTargetSpec,
        write: bool,
    ) {
        target.absorb_project_folders(&[]);
        if target.kind != IdeTargetKind::Local || user.is_root {
            self.enrich_target_roots(&ctx.pool(), Some(&ctx.ide().projects), Some(user), target)
                .await;
            return;
        }
        let pool = ctx.pool();
        let project_key = target.project_key();
        let mut candidates = Vec::new();
        if let Ok((proj, stored)) = load_stored_project_env(&pool, &project_key).await {
            self.record_snapshot(&project_key, &proj, &stored);
            for r in proj.additional_roots {
                if !r.is_empty() && r != target.root && !candidates.contains(&r) {
                    candidates.push(r);
                }
            }
        }
        let _ = ctx.ide().projects.ensure_hydrated(&pool).await;
        for r in ctx
            .ide()
            .projects
            .folders_for_root_for_user(&target.root, user)
        {
            if !r.is_empty() && r != target.root && !candidates.contains(&r) {
                candidates.push(r);
            }
        }
        for cand in candidates {
            if target.additional_roots.contains(&cand) {
                continue;
            }
            if ctx
                .authorize_local_target(user, &IdeTargetSpec::local(cand.clone()), write)
                .await
                .is_ok()
            {
                target.additional_roots.push(cand);
            }
        }
    }

    pub async fn enrich_target_roots(
        &self,
        pool: &DbPool,
        projects: Option<&ProjectStore>,
        user: Option<&User>,
        target: &mut IdeTargetSpec,
    ) {
        target.absorb_project_folders(&[]);
        let project_key = target.project_key();
        if let Ok((proj, stored)) = load_stored_project_env(pool, &project_key).await {
            self.record_snapshot(&project_key, &proj, &stored);
            for r in proj.additional_roots {
                if !r.is_empty() && r != target.root && !target.additional_roots.contains(&r) {
                    target.additional_roots.push(r);
                }
            }
        }
        if target.kind == IdeTargetKind::Local {
            if let Some(store) = projects {
                let _ = store.ensure_hydrated(pool).await;
                let extra = match user {
                    Some(u) => store.folders_for_root_for_user(&target.root, u),
                    None => store.folders_for_root(&target.root),
                };
                for r in extra {
                    if !r.is_empty() && r != target.root && !target.additional_roots.contains(&r) {
                        target.additional_roots.push(r);
                    }
                }
            }
        }
    }

    pub async fn add_project_folder(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        folder_path: &str,
    ) -> Result<IdeProjectEnvironment> {
        target.validate()?;
        let normalized = validate_attached_folder_path(target, folder_path)?;
        let project_key = target.project_key();
        let (mut proj, _) = load_stored_project_env(pool, &project_key).await?;
        if normalized != target.root && !proj.additional_roots.contains(&normalized) {
            proj.additional_roots.push(normalized);
        }
        persist_project_roots(pool, target, &proj.additional_roots).await?;
        self.get_project_env(pool, target).await
    }

    /// Bulk-attach multiple folders in a single SQLite write (`POST /ide/projects/folders/bulk`).
    pub async fn add_project_folders_bulk(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        folders: &[String],
    ) -> Result<(IdeProjectEnvironment, usize)> {
        target.validate()?;
        let project_key = target.project_key();
        let (mut proj, _) = load_stored_project_env(pool, &project_key).await?;
        let mut added = 0usize;
        for raw in folders {
            if raw.trim().is_empty() {
                continue;
            }
            let normalized = validate_attached_folder_path(target, raw)?;
            if normalized != target.root && !proj.additional_roots.contains(&normalized) {
                proj.additional_roots.push(normalized);
                added += 1;
            }
        }
        persist_project_roots(pool, target, &proj.additional_roots).await?;
        let env = self.get_project_env(pool, target).await?;
        Ok((env, added))
    }

    #[allow(clippy::disallowed_methods)]
    pub async fn remove_project_folder(
        &self,
        pool: &DbPool,
        target: &IdeTargetSpec,
        folder_path: &str,
    ) -> Result<IdeProjectEnvironment> {
        target.validate()?;
        let raw_trimmed = folder_path.trim().trim_end_matches('/');
        if raw_trimmed.is_empty() {
            return Err(Error::Invalid("folder path must not be empty".into()));
        }
        let canon_opt = std::fs::canonicalize(raw_trimmed)
            .ok()
            .map(|p| p.to_string_lossy().into_owned());
        let project_key = target.project_key();
        let (mut proj, _) = load_stored_project_env(pool, &project_key).await?;
        proj.additional_roots.retain(|entry| {
            let trimmed = entry.trim_end_matches('/');
            trimmed != raw_trimmed && canon_opt.as_deref() != Some(entry.as_str())
        });
        persist_project_roots(pool, target, &proj.additional_roots).await?;
        self.get_project_env(pool, target).await
    }

    pub async fn effective_env<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
    ) -> Result<BTreeMap<String, String>> {
        self.build_effective_env_for_run(pool, secrets, target, None)
            .await
    }

    pub async fn build_effective_env<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
    ) -> Result<BTreeMap<String, String>> {
        self.build_effective_env_for_run(pool, secrets, target, None)
            .await
    }

    pub async fn build_effective_env_for_run<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
        cfg: Option<&IdeRunConfig>,
    ) -> Result<BTreeMap<String, String>> {
        self.build_effective_env_for_run_as(pool, secrets, target, cfg, true)
            .await
    }

    pub async fn build_effective_env_for_run_as<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
        cfg: Option<&IdeRunConfig>,
        caller_is_root: bool,
    ) -> Result<BTreeMap<String, String>> {
        Ok(self
            .build_effective_env_with_loaded(pool, secrets, target, cfg, caller_is_root)
            .await?
            .1)
    }

    /// Resolve both the redacted [`IdeProjectEnvironment`] and the layered
    /// `KEY=VALUE` process environment in a single SQLite + secret store pass.
    pub async fn build_effective_env_with_loaded<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
        cfg: Option<&IdeRunConfig>,
        caller_is_root: bool,
    ) -> Result<(IdeProjectEnvironment, BTreeMap<String, String>)> {
        self.build_effective_env_with_loaded_opts(pool, secrets, target, cfg, true, caller_is_root)
            .await
    }

    /// Single-pass project + effective environment loader with configurable
    /// login-shell awaiting (`await_shell = false` avoids blocking HTTP start
    /// endpoints when `start_session_async` awaits a cold shell in the background).
    pub async fn build_effective_env_with_loaded_opts<S: SecretAccess + ?Sized>(
        &self,
        pool: &DbPool,
        secrets: &S,
        target: &IdeTargetSpec,
        cfg: Option<&IdeRunConfig>,
        await_shell: bool,
        caller_is_root: bool,
    ) -> Result<(IdeProjectEnvironment, BTreeMap<String, String>)> {
        target.validate()?;
        let project_key = target.project_key();
        let (mut proj, mut stored_vars) = load_stored_project_env(pool, &project_key).await?;
        self.record_snapshot(&project_key, &proj, &stored_vars);
        let redacted_vars = redact_stored_vars(&stored_vars);
        layer_child_and_module_sdk_for_run(pool, target, cfg, &mut proj, &mut stored_vars).await;
        let mut out_proj = proj.clone();
        out_proj.env_vars = redacted_vars;
        let map = self
            .resolve_env_layers(
                &proj,
                &stored_vars,
                Some(secrets),
                target,
                cfg,
                await_shell,
                caller_is_root,
            )
            .await?;
        Ok((out_proj, map))
    }
}
