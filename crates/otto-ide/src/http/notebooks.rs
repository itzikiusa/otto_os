//! HTTP handlers for notebook kernels (`/ide/notebooks/*`): Jupyter kernels
//! plus the built-in Python fallback, streamed outputs (SSE), stdin and lifecycle.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use axum::extract::{Query, State};
use axum::response::Response;
use axum::{Extension, Json};
use otto_core::auth::AuthUser;
use otto_core::{Error, Id, Result};
use serde::Deserialize;

use super::sse_framing::build_notebook_sse_response;
use super::ApiResult;
use crate::notebooks::kernelspecs::KernelLaunch;
use crate::notebooks::{
    default_cell_timeout, detect_local_accelerators, discover_kernels, record_kernel_trust,
    DiscoveredKernel, IdeNotebookExecuteReq, IdeNotebookExecuteResp, IdeNotebookInputReq,
    IdeNotebookKernelsReq, IdeNotebookKernelsResp, IdeNotebookOkResp, IdeNotebookSessionReq,
    IdeNotebookVariablesResp, NotebookKernelCtx,
};
use crate::types::{IdeTargetKind, IdeTargetQuery, IdeTargetSpec};
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

async fn list_kernels<S: IdeCtx>(ctx: &S, target: &IdeTargetSpec) -> Vec<DiscoveredKernel> {
    let proj_env = ctx
        .ide()
        .environments
        .get_project_env(&ctx.pool(), target)
        .await
        .ok();
    discover_kernels(
        Path::new(&target.root),
        ctx.data_dir(),
        &target.project_key(),
        proj_env.as_ref(),
    )
    .await
}

/// The kernel a request runs on, and the env it runs with. `kernel_id` picks
/// from discovery (a client never supplies a command); without it, a matching
/// `interpreter`, then the project's `VIRTUAL_ENV`, then the active kernel.
/// A workspace kernelspec runs only once the person accepted it (`trust_kernel`).
async fn resolve_kernel<S: IdeCtx>(
    ctx: &S,
    target: &IdeTargetSpec,
    kernel_id: Option<&str>,
    interpreter: Option<&str>,
    trust_kernel: bool,
) -> Result<(DiscoveredKernel, BTreeMap<String, String>)> {
    let env_map = ctx
        .ide()
        .environments
        .build_effective_env_for_run_as(&ctx.pool(), ctx.secrets().as_ref(), target, None, true)
        .await?;
    let kernels = list_kernels(ctx, target).await;
    let nonempty = |s: Option<&str>| {
        s.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let venv_python = env_map
        .get("VIRTUAL_ENV")
        .filter(|s| !s.trim().is_empty())
        .map(|v| format!("{}/bin/python", v.trim_end_matches('/')));
    let chosen = if let Some(id) = nonempty(kernel_id) {
        kernels
            .into_iter()
            .find(|k| k.info.id == id)
            .ok_or_else(|| Error::NotFound(format!("kernel '{id}' is no longer available")))?
    } else if let Some(interp) = nonempty(interpreter).or(venv_python) {
        match kernels.iter().position(|k| k.info.interpreter == interp) {
            Some(i) => kernels.into_iter().nth(i).expect("index in range"),
            None if Path::new(&interp).is_file() => DiscoveredKernel {
                info: crate::notebooks::IdeNotebookKernelInfo {
                    id: format!("builtin:{interp}"),
                    display_name: interp.clone(),
                    language: "python".into(),
                    interpreter: interp.clone(),
                    kind: "builtin".into(),
                    version: None,
                    active: true,
                    spec_name: None,
                    origin: None,
                    argv_preview: None,
                    needs_trust: false,
                    debugger: false,
                    env_kind: None,
                },
                launch: KernelLaunch::Builtin {
                    interpreter: interp,
                },
                trust_key: None,
            },
            None => {
                return Err(Error::NotFound(format!(
                    "interpreter '{interp}' wasn't found"
                )));
            }
        }
    } else {
        kernels.into_iter().find(|k| k.info.active).ok_or_else(|| {
            Error::NotFound(
                "No kernel found. Create a Python environment or install a Jupyter kernel.".into(),
            )
        })?
    };
    if let Some(key) = chosen
        .trust_key
        .as_deref()
        .filter(|_| chosen.info.needs_trust)
    {
        if !trust_kernel {
            return Err(Error::Forbidden(format!(
                "kernel_requires_trust: Start kernel '{}' from this project? It runs: {}",
                chosen.info.display_name,
                chosen.info.argv_preview.as_deref().unwrap_or("")
            )));
        }
        record_kernel_trust(ctx.data_dir(), key)?;
    }
    Ok((chosen, env_map))
}

fn notebook_cwd(notebook_path: &str, target: &IdeTargetSpec) -> String {
    Path::new(notebook_path)
        .parent()
        .filter(|p| p.is_dir())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| target.root.clone())
}

fn runtime_dir<S: IdeCtx>(ctx: &S) -> PathBuf {
    ctx.data_dir().join("jupyter-runtime")
}

pub(super) async fn notebook_kernels<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<IdeNotebookKernelsReq>,
) -> ApiResult<Json<IdeNotebookKernelsResp>> {
    authorize_notebook_target(&ctx, &auth, &req.target, false).await?;
    let project_key = req.target.project_key();
    let kernels = list_kernels(&ctx, &req.target).await;
    let accelerators = tokio::task::spawn_blocking(detect_local_accelerators)
        .await
        .map_err(|e| Error::Internal(format!("notebook accelerator probe: {e}")))?;
    let notebooks = &ctx.ide().notebooks;
    let path = req.notebook_path.as_deref();
    let summary = path.and_then(|p| notebooks.active_session_summary(&auth.0.id, &project_key, p));
    let kernel_status = path.and_then(|p| notebooks.kernel_status(&auth.0.id, &project_key, p));
    let (active_session_id, active_interpreter, execution_count) = match summary {
        Some((sid, interp, cnt)) => (Some(sid), Some(interp), cnt),
        None => (None, None, 0),
    };
    Ok(Json(IdeNotebookKernelsResp {
        kernels: kernels.into_iter().map(|k| k.info).collect(),
        accelerators,
        active_session_id,
        active_interpreter,
        execution_count,
        kernel_status,
    }))
}

pub(super) async fn notebook_execute<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<IdeNotebookExecuteReq>,
) -> ApiResult<Json<IdeNotebookExecuteResp>> {
    authorize_notebook_target(&ctx, &auth, &req.target, true).await?;
    let (kernel, env_map) = resolve_kernel(
        &ctx,
        &req.target,
        req.kernel_id.as_deref(),
        req.interpreter.as_deref(),
        req.trust_kernel,
    )
    .await?;
    let cwd = notebook_cwd(&req.notebook_path, &req.target);
    let runtime = runtime_dir(&ctx);
    let project_key = req.target.project_key();
    let kctx = NotebookKernelCtx {
        owner: &auth.0.id,
        project_key: &project_key,
        notebook_path: &req.notebook_path,
        cwd: &cwd,
        env: &env_map,
        runtime_dir: &runtime,
    };
    let timeout = req.timeout_secs.map(|s| default_cell_timeout(Some(s)));
    let run_id = req.run_id.clone().unwrap_or_default();
    let resp = ctx
        .ide()
        .notebooks
        .execute(
            &kctx,
            &kernel.info.id,
            &kernel.launch,
            &req.code,
            &run_id,
            timeout,
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
    let (ok, message) = ctx
        .ide()
        .notebooks
        .interrupt_kernel(&auth.0.id, &req.target.project_key(), &req.notebook_path)
        .await;
    Ok(Json(IdeNotebookOkResp {
        ok,
        message: Some(message),
    }))
}

pub(super) async fn notebook_restart<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<IdeNotebookSessionReq>,
) -> ApiResult<Json<IdeNotebookOkResp>> {
    authorize_notebook_target(&ctx, &auth, &req.target, true).await?;
    let (kernel, env_map) = resolve_kernel(
        &ctx,
        &req.target,
        req.kernel_id.as_deref(),
        req.interpreter.as_deref(),
        req.trust_kernel,
    )
    .await?;
    let cwd = notebook_cwd(&req.notebook_path, &req.target);
    let runtime = runtime_dir(&ctx);
    let project_key = req.target.project_key();
    let kctx = NotebookKernelCtx {
        owner: &auth.0.id,
        project_key: &project_key,
        notebook_path: &req.notebook_path,
        cwd: &cwd,
        env: &env_map,
        runtime_dir: &runtime,
    };
    let sid = ctx
        .ide()
        .notebooks
        .restart_kernel(&kctx, &kernel.info.id, &kernel.launch)
        .await?;
    Ok(Json(IdeNotebookOkResp {
        ok: true,
        message: Some(format!("Kernel restarted ({sid})")),
    }))
}

pub(super) async fn notebook_input<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthUser>,
    Json(req): Json<IdeNotebookInputReq>,
) -> ApiResult<Json<IdeNotebookOkResp>> {
    authorize_notebook_target(&ctx, &auth, &req.target, true).await?;
    let ok = ctx
        .ide()
        .notebooks
        .send_input(
            &auth.0.id,
            &req.target.project_key(),
            &req.notebook_path,
            &req.value,
        )
        .await?;
    Ok(Json(IdeNotebookOkResp {
        ok,
        message: (!ok).then(|| "No cell is waiting for input".into()),
    }))
}

#[derive(Debug, Deserialize)]
pub(super) struct NotebookStreamQuery {
    #[serde(default)]
    kind: Option<IdeTargetKind>,
    #[serde(default)]
    workspace_id: Option<Id>,
    root: String,
    notebook_path: String,
}

/// `GET /ide/notebooks/stream`: the notebook's live kernel events (`status`,
/// `output`, `clear_output`, `input_request`, `input_done`) as SSE.
pub(super) async fn notebook_stream<S: IdeCtx>(
    State(ctx): State<S>,
    Extension(auth): Extension<AuthUser>,
    Query(q): Query<NotebookStreamQuery>,
) -> ApiResult<Response> {
    let target = IdeTargetQuery {
        kind: q.kind,
        workspace_id: q.workspace_id,
        root: Some(q.root),
        connection_id: None,
        include_catalog: false,
    }
    .into_target_spec()?;
    authorize_notebook_target(&ctx, &auth, &target, false).await?;
    let (initial, rx) =
        ctx.ide()
            .notebooks
            .subscribe(&auth.0.id, &target.project_key(), &q.notebook_path);
    Ok(build_notebook_sse_response(initial, rx))
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
        // POST /ide/notebooks/{execute,restart,interrupt} specifically from the root
        // host-execution gate (passing `interpreter: null` so auto-discovery would
        // otherwise succeed and NOT trip the unknown-interpreter 403 check).
        let exec_body = serde_json::json!({
            "target": { "kind": "local", "root": root_str },
            "notebook_path": ws.join("demo.ipynb").to_string_lossy(),
            "code": "1 + 1",
            "interpreter": null
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
        let exec_err_text = String::from_utf8_lossy(
            &axum::body::to_bytes(exec_resp.into_body(), 65536)
                .await
                .unwrap(),
        )
        .to_ascii_lowercase();
        assert!(
            exec_err_text.contains("root") && !exec_err_text.contains("discovered python kernel"),
            "SEC-8: 403 must come from the root host-execution gate, got: {exec_err_text}"
        );

        let restart_body = serde_json::json!({
            "target": { "kind": "local", "root": ws.to_string_lossy() },
            "notebook_path": ws.join("demo.ipynb").to_string_lossy(),
            "interpreter": null
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
        let restart_err_text = String::from_utf8_lossy(
            &axum::body::to_bytes(restart_resp.into_body(), 65536)
                .await
                .unwrap(),
        )
        .to_ascii_lowercase();
        assert!(
            restart_err_text.contains("root")
                && !restart_err_text.contains("discovered python kernel"),
            "SEC-8: restart 403 must come from the root host-execution gate, got: {restart_err_text}"
        );

        let interrupt_req = Request::builder()
            .method("POST")
            .uri("/ide/notebooks/interrupt")
            .header("content-type", "application/json")
            .extension(non_root)
            .body(Body::from(restart_body.to_string()))
            .unwrap();
        let interrupt_resp = app.clone().oneshot(interrupt_req).await.unwrap();
        assert_eq!(
            interrupt_resp.status(),
            StatusCode::FORBIDDEN,
            "SEC-8: routed /ide/notebooks/interrupt must reject non-root editor with 403"
        );
        let interrupt_err_text = String::from_utf8_lossy(
            &axum::body::to_bytes(interrupt_resp.into_body(), 65536)
                .await
                .unwrap(),
        )
        .to_ascii_lowercase();
        assert!(
            interrupt_err_text.contains("root"),
            "SEC-8: interrupt 403 must come from the root host-execution gate, got: {interrupt_err_text}"
        );

        // J13: stdin answers execute code paths too: same root gate.
        let input_body = serde_json::json!({
            "target": { "kind": "local", "root": ws.to_string_lossy() },
            "notebook_path": ws.join("demo.ipynb").to_string_lossy(),
            "value": "ada"
        });
        let input_req = Request::builder()
            .method("POST")
            .uri("/ide/notebooks/input")
            .header("content-type", "application/json")
            .extension(AuthUser(make_user(false)))
            .body(Body::from(input_body.to_string()))
            .unwrap();
        let input_resp = app.clone().oneshot(input_req).await.unwrap();
        assert_eq!(
            input_resp.status(),
            StatusCode::FORBIDDEN,
            "routed /ide/notebooks/input must reject a non-root editor with 403"
        );

        assert!(require_notebook_exec_authority(&make_user(true)).is_ok());
    }
}
