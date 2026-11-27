//! HTTP handlers for Jupyter / Python Notebook kernels (`/ide/notebooks/*`).

use std::path::PathBuf;

use axum::extract::State;
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::{Error, Result};

use super::ApiResult;
use crate::notebooks::{
    default_cell_timeout, detect_local_accelerators, discover_notebook_kernels,
    IdeNotebookExecuteReq, IdeNotebookExecuteResp, IdeNotebookKernelsReq, IdeNotebookKernelsResp,
    IdeNotebookOkResp, IdeNotebookSessionReq, IdeNotebookVariablesResp,
};
use crate::types::{IdeTargetKind, IdeTargetSpec};
use crate::IdeCtx;

async fn authorize_notebook_target<S: IdeCtx>(
    ctx: &S,
    user: &AuthUser,
    target: &IdeTargetSpec,
    write: bool,
) -> Result<()> {
    target.validate()?;
    if target.kind != IdeTargetKind::Local {
        return Err(Error::Invalid(
            "notebook kernels currently support local workspaces only".into(),
        ));
    }
    if write {
        require_notebook_exec_authority(&user.0)?;
    }
    ctx.authorize_local_target(&user.0, target, write).await
}

fn require_notebook_exec_authority(user: &otto_core::domain::User) -> Result<()> {
    super::require_root_user(user)
}

async fn resolve_notebook_interpreter_and_env<S: IdeCtx>(
    ctx: &S,
    target: &IdeTargetSpec,
    requested_interpreter: Option<&str>,
    caller_is_root: bool,
) -> Result<(String, std::collections::BTreeMap<String, String>)> {
    let pool = ctx.pool();
    let secrets = ctx.secrets();
    let env_map = ctx
        .ide()
        .environments
        .build_effective_env_for_run_as(&pool, secrets.as_ref(), target, None, caller_is_root)
        .await?;

    let req_interp = requested_interpreter
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let interp = if let Some(explicit) = req_interp {
        if !caller_is_root {
            let root = PathBuf::from(&target.root);
            let data_dir = ctx.data_dir().to_path_buf();
            let allowed = tokio::task::spawn_blocking(move || {
                discover_notebook_kernels(&root, &data_dir, None)
            })
            .await
            .map_err(|e| Error::Internal(format!("kernel lookup: {e}")))?;
            if !allowed.iter().any(|k| k.interpreter == explicit) {
                return Err(Error::Forbidden(
                    "non-root users must pick a discovered Python kernel".into(),
                ));
            }
        }
        explicit.to_string()
    } else if let Some(venv) = env_map.get("VIRTUAL_ENV").filter(|s| !s.trim().is_empty()) {
        let py = format!("{}/bin/python", venv.trim_end_matches('/'));
        if std::path::Path::new(&py).is_file() {
            py
        } else {
            "python3".into()
        }
    } else {
        let root = PathBuf::from(&target.root);
        let data_dir = ctx.data_dir().to_path_buf();
        let discovered =
            tokio::task::spawn_blocking(move || discover_notebook_kernels(&root, &data_dir, None))
                .await
                .map_err(|e| Error::Internal(format!("kernel lookup: {e}")))?;
        discovered
            .into_iter()
            .next()
            .map(|k| k.interpreter)
            .unwrap_or_else(|| "python3".into())
    };

    Ok((interp, env_map))
}

pub(super) async fn notebook_kernels<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<IdeNotebookKernelsReq>,
) -> ApiResult<Json<IdeNotebookKernelsResp>> {
    authorize_notebook_target(&ctx, &auth, &req.target, false).await?;
    let pool = ctx.pool();
    let project_key = req.target.project_key();
    let proj_env = ctx
        .ide()
        .environments
        .get_project_env(&pool, &req.target)
        .await
        .ok();
    let root = PathBuf::from(&req.target.root);
    let data_dir = ctx.data_dir().to_path_buf();
    let (kernels, accelerators) = tokio::task::spawn_blocking(move || {
        (
            discover_notebook_kernels(&root, &data_dir, proj_env.as_ref()),
            detect_local_accelerators(),
        )
    })
    .await
    .map_err(|e| Error::Internal(format!("notebook kernel probe: {e}")))?;

    let summary = req.notebook_path.as_deref().and_then(|p| {
        ctx.ide()
            .notebooks
            .active_session_summary(&auth.0.id, &project_key, p)
    });
    let (active_session_id, active_interpreter, execution_count) = match summary {
        Some((sid, interp, cnt)) => (Some(sid), Some(interp), cnt),
        None => (None, None, 0),
    };

    Ok(Json(IdeNotebookKernelsResp {
        kernels,
        accelerators,
        active_session_id,
        active_interpreter,
        execution_count,
    }))
}

pub(super) async fn notebook_execute<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<IdeNotebookExecuteReq>,
) -> ApiResult<Json<IdeNotebookExecuteResp>> {
    authorize_notebook_target(&ctx, &auth, &req.target, true).await?;
    let is_root = otto_core::auth::root_authority(&auth.0);
    let (interp, env_map) = resolve_notebook_interpreter_and_env(
        &ctx,
        &req.target,
        req.interpreter.as_deref(),
        is_root,
    )
    .await?;
    let cwd = std::path::Path::new(&req.notebook_path)
        .parent()
        .filter(|p| p.is_dir())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| req.target.root.clone());
    let resp = ctx
        .ide()
        .notebooks
        .execute_cell(
            &auth.0.id,
            &req.target.project_key(),
            &req.notebook_path,
            &cwd,
            &interp,
            &env_map,
            &req.code,
            default_cell_timeout(req.timeout_secs),
        )
        .await?;
    Ok(Json(resp))
}

pub(super) async fn notebook_interrupt<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<IdeNotebookSessionReq>,
) -> ApiResult<Json<IdeNotebookOkResp>> {
    authorize_notebook_target(&ctx, &auth, &req.target, true).await?;
    let ok =
        ctx.ide()
            .notebooks
            .interrupt(&auth.0.id, &req.target.project_key(), &req.notebook_path);
    Ok(Json(IdeNotebookOkResp {
        ok,
        message: Some(if ok {
            "Interrupt signal sent to kernel".into()
        } else {
            "No active kernel for this notebook".into()
        }),
    }))
}

pub(super) async fn notebook_restart<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<IdeNotebookSessionReq>,
) -> ApiResult<Json<IdeNotebookOkResp>> {
    authorize_notebook_target(&ctx, &auth, &req.target, true).await?;
    let is_root = otto_core::auth::root_authority(&auth.0);
    let (interp, env_map) = resolve_notebook_interpreter_and_env(
        &ctx,
        &req.target,
        req.interpreter.as_deref(),
        is_root,
    )
    .await?;
    let cwd = std::path::Path::new(&req.notebook_path)
        .parent()
        .filter(|p| p.is_dir())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| req.target.root.clone());
    let sid = ctx
        .ide()
        .notebooks
        .restart(
            &auth.0.id,
            &req.target.project_key(),
            &req.notebook_path,
            &cwd,
            &interp,
            &env_map,
        )
        .await?;
    Ok(Json(IdeNotebookOkResp {
        ok: true,
        message: Some(format!("Kernel restarted ({sid})")),
    }))
}

pub(super) async fn notebook_variables<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<IdeNotebookSessionReq>,
) -> ApiResult<Json<IdeNotebookVariablesResp>> {
    authorize_notebook_target(&ctx, &auth, &req.target, false).await?;
    let resp = ctx
        .ide()
        .notebooks
        .list_variables(&auth.0.id, &req.target.project_key(), &req.notebook_path)
        .await?;
    Ok(Json(resp))
}

pub(super) async fn notebook_shutdown<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<IdeNotebookSessionReq>,
) -> ApiResult<Json<IdeNotebookOkResp>> {
    authorize_notebook_target(&ctx, &auth, &req.target, true).await?;
    ctx.ide()
        .notebooks
        .shutdown_session(&auth.0.id, &req.target.project_key(), &req.notebook_path)
        .await;
    Ok(Json(IdeNotebookOkResp {
        ok: true,
        message: Some("Kernel stopped".into()),
    }))
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use otto_core::auth::{BoxFuture, RoleChecker};
    use otto_core::domain::{User, WorkspaceRole};
    use otto_core::secrets::SecretStore;
    use otto_core::{Error, Id, Result};
    use otto_state::DbPool;
    use tower::ServiceExt;

    use super::*;
    use crate::{IdeCtx, IdeEngine, IdeSshSpec};

    struct NoopSecrets;
    impl SecretStore for NoopSecrets {
        fn put(&self, _k: &str, _v: &str) -> Result<()> {
            Ok(())
        }
        fn get(&self, _k: &str) -> Result<Option<String>> {
            Ok(None)
        }
        fn delete(&self, _k: &str) -> Result<()> {
            Ok(())
        }
    }

    struct AllowRoles;
    impl RoleChecker for AllowRoles {
        fn check<'a>(
            &'a self,
            _user: &'a User,
            _workspace_id: &'a Id,
            _min: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    #[derive(Clone)]
    struct TestNotebookCtx {
        ide: Arc<IdeEngine>,
        pool: DbPool,
        secrets: Arc<dyn SecretStore>,
        roles: Arc<dyn RoleChecker>,
        data_dir: PathBuf,
    }

    impl IdeCtx for TestNotebookCtx {
        fn ide(&self) -> &Arc<IdeEngine> {
            &self.ide
        }
        fn pool(&self) -> DbPool {
            self.pool.clone()
        }
        fn secrets(&self) -> &Arc<dyn SecretStore> {
            &self.secrets
        }
        fn data_dir(&self) -> &Path {
            &self.data_dir
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
        fn resolve_ssh_connection<'a>(
            &'a self,
            _user: &'a User,
            _connection_id: &'a Id,
        ) -> BoxFuture<'a, Result<IdeSshSpec>> {
            Box::pin(async { Err(Error::Invalid("ssh unused in test".into())) })
        }
        fn authorize_local_target<'a>(
            &'a self,
            _user: &'a User,
            _spec: &'a IdeTargetSpec,
            _write: bool,
        ) -> BoxFuture<'a, Result<()>> {
            // Simulate a non-root workspace editor who HAS local workspace write access;
            // SEC-8 requires the notebook execution gate itself to reject non-root callers.
            Box::pin(async { Ok(()) })
        }
    }

    fn make_user(is_root: bool) -> User {
        User {
            id: if is_root { "root-1" } else { "editor-1" }.into(),
            username: if is_root { "root" } else { "editor" }.into(),
            display_name: "User".into(),
            is_root,
            disabled: false,
            created_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn routed_notebook_execute_and_restart_reject_non_root_workspace_editor() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("workspace");
        std::fs::create_dir_all(&ws).unwrap();
        let db_path = tmp.path().join("test.sqlite");
        let pool = otto_state::open(&db_path).await.unwrap();
        let data_dir = tmp.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        let ctx = TestNotebookCtx {
            ide: Arc::new(IdeEngine::new(data_dir.clone())),
            pool,
            secrets: Arc::new(NoopSecrets),
            roles: Arc::new(AllowRoles),
            data_dir,
        };
        let app = crate::http::api_router::<TestNotebookCtx>().with_state(ctx);
        let non_root = AuthUser(make_user(false));
        let root_str = ws.to_string_lossy().into_owned();

        // SEC-8: Non-root workspace editor must receive 403 Forbidden from routed
        // POST /notebooks/execute and POST /notebooks/restart before any interpreter runs.
        let exec_body = serde_json::json!({
            "target": { "kind": "local", "root": root_str },
            "notebook_path": ws.join("demo.ipynb").to_string_lossy(),
            "code": "1 + 1",
            "interpreter": "/nonexistent/must-never-be-resolved"
        });
        let exec_req = Request::builder()
            .method("POST")
            .uri("/ide/notebooks/execute")
            .header("content-type", "application/json")
            .extension(non_root.clone())
            .body(Body::from(exec_body.to_string()))
            .unwrap();
        let exec_resp = app.clone().oneshot(exec_req).await.unwrap();
        assert_eq!(
            exec_resp.status(),
            StatusCode::FORBIDDEN,
            "SEC-8: routed /ide/notebooks/execute must reject non-root editor with 403"
        );

        let restart_body = serde_json::json!({
            "target": { "kind": "local", "root": ws.to_string_lossy() },
            "notebook_path": ws.join("demo.ipynb").to_string_lossy(),
            "interpreter": "/nonexistent/must-never-be-resolved"
        });
        let restart_req = Request::builder()
            .method("POST")
            .uri("/ide/notebooks/restart")
            .header("content-type", "application/json")
            .extension(non_root.clone())
            .body(Body::from(restart_body.to_string()))
            .unwrap();
        let restart_resp = app.clone().oneshot(restart_req).await.unwrap();
        assert_eq!(
            restart_resp.status(),
            StatusCode::FORBIDDEN,
            "SEC-8: routed /ide/notebooks/restart must reject non-root editor with 403"
        );

        let interrupt_req = Request::builder()
            .method("POST")
            .uri("/ide/notebooks/interrupt")
            .header("content-type", "application/json")
            .extension(non_root)
            .body(Body::from(restart_body.to_string()))
            .unwrap();
        let interrupt_resp = app.oneshot(interrupt_req).await.unwrap();
        assert_eq!(
            interrupt_resp.status(),
            StatusCode::FORBIDDEN,
            "SEC-8: routed /notebooks/interrupt must reject non-root editor with 403"
        );

        assert!(require_notebook_exec_authority(&make_user(true)).is_ok());
    }
}
