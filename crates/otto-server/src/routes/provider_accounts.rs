//! Owner-scoped subscription profiles and provider-native interactive sign-in.
use crate::{
    auth::{require_ws_role, CurrentAuthContext, CurrentUser},
    error::{ApiError, ApiResult},
    state::ServerCtx,
};
use axum::{
    extract::{Path, State},
    routing::{get, post},
    Json, Router,
};
use otto_core::api::CreateSessionReq;
use otto_core::domain::{Session, SessionKind, WorkspaceRole};
use otto_core::provider_accounts::{CreateProviderAccount, LoginProviderAccount, ProviderAccount};
use otto_core::{Error, Id};
use otto_state::provider_accounts::ProviderAccountsRepo;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/auth/provider-accounts", get(list).post(create))
        .route("/auth/provider-accounts/{id}/login", post(login))
        .route("/auth/provider-accounts/{id}/status", get(status))
}

async fn list(
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<ProviderAccount>>> {
    Ok(Json(
        ProviderAccountsRepo::new(ctx.pool.clone())
            .list(&user.id)
            .await?,
    ))
}

async fn create(
    State(ctx): State<ServerCtx>,
    auth: CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateProviderAccount>,
) -> ApiResult<Json<ProviderAccount>> {
    if auth.real_user().id != auth.effective_user().id {
        return Err(ApiError(Error::Forbidden(
            "cannot create subscription profiles while impersonating".into(),
        )));
    }
    let account = ProviderAccountsRepo::new(ctx.pool.clone())
        .create(&user.id, &req.provider, &req.label)
        .await?;
    let home = otto_sessions::accounts::account_home(
        &ctx.data_dir.join("provider-accounts"),
        &account.id,
    )?;
    otto_sessions::accounts::prepare_home(&home)?;
    Ok(Json(account))
}

fn account_spec(
    ctx: &ServerCtx,
    account: &ProviderAccount,
    args: Vec<String>,
) -> ApiResult<otto_pty::CommandSpec> {
    let home = otto_sessions::accounts::account_home(
        &ctx.data_dir.join("provider-accounts"),
        &account.id,
    )?;
    otto_sessions::accounts::prepare_home(&home)?;
    let program = ctx
        .manager
        .provider_program(&account.provider)
        .ok_or_else(|| {
            ApiError(Error::Invalid(
                "install this provider CLI before signing in".into(),
            ))
        })?;
    let mut spec = otto_pty::CommandSpec {
        program,
        args,
        cwd: Some(home.to_string_lossy().into_owned()),
        env: vec![],
    };
    otto_sessions::accounts::apply_account(&mut spec, account, &home)?;
    Ok(spec)
}

async fn login(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    auth: CurrentAuthContext,
    CurrentUser(user): CurrentUser,
    Json(req): Json<LoginProviderAccount>,
) -> ApiResult<Json<Session>> {
    if auth.real_user().id != auth.effective_user().id {
        return Err(ApiError(Error::Forbidden(
            "cannot sign into a subscription while impersonating".into(),
        )));
    }
    let account = ProviderAccountsRepo::new(ctx.pool.clone())
        .get(&user.id, &id)
        .await?;
    require_ws_role(&ctx, &user, &req.workspace_id, WorkspaceRole::Editor).await?;
    let workspace = otto_state::WorkspacesRepo::new(ctx.pool.clone())
        .get(&req.workspace_id)
        .await?;
    let spec = account_spec(
        &ctx,
        &account,
        otto_sessions::accounts::login_args(&account.provider)?,
    )?;
    // A connection terminal runs the login command only: no agent prompts,
    // context overlays or session MCP credentials are injected into sign-in.
    let request = CreateSessionReq {
        kind: SessionKind::Connection,
        provider: Some("shell".into()),
        title: Some(format!("Sign in: {} · {}", account.provider, account.label)),
        cwd: spec.cwd.clone(),
        connection_id: None,
        model: None,
        meta: Some(serde_json::json!({"source":"provider-login", "login_account_id":id})),
    };
    Ok(Json(
        ctx.manager
            .create(&workspace, &user.id, request, Some(spec))
            .await?,
    ))
}

async fn status(
    Path(id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<serde_json::Value>> {
    let account = ProviderAccountsRepo::new(ctx.pool.clone())
        .get(&user.id, &id)
        .await?;
    let args = if account.provider == "claude" {
        vec!["auth", "status"]
    } else {
        vec!["login", "status"]
    };
    let spec = account_spec(
        &ctx,
        &account,
        args.into_iter().map(str::to_owned).collect(),
    )?;
    let mut command = tokio::process::Command::new(&spec.program);
    command.args(&spec.args).envs(spec.env).kill_on_drop(true);
    if let Some(cwd) = spec.cwd {
        command.current_dir(cwd);
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
        .await
        .map_err(|_| {
            ApiError(Error::Upstream(
                "sign-in check timed out; open the provider sign-in terminal".into(),
            ))
        })?
        .map_err(|_| {
            ApiError(Error::Upstream(
                "could not run the provider sign-in check".into(),
            ))
        })?;
    // Deliberately do not return or log provider output: it can contain account
    // details. Exit status (and Claude's boolean) is all this UI needs.
    let signed_in = output.status.success()
        && (account.provider != "claude"
            || serde_json::from_slice::<serde_json::Value>(&output.stdout)
                .ok()
                .and_then(|v| v.get("loggedIn").and_then(|v| v.as_bool()))
                .unwrap_or(false));
    Ok(Json(serde_json::json!({"signed_in": signed_in})))
}
