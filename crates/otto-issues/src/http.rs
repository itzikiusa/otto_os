//! Axum router for issue-tracking endpoints.
//! All paths are relative to the `/api/v1` mount point.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch};
use axum::{Extension, Json, Router};
use otto_core::api::{CreateIssueAccountReq, Problem, UpdateIssueAccountReq};
use otto_core::auth::AuthUser;
use otto_core::domain::{IssueAccount, IssueDetail, IssueProject, IssueSummary};
use otto_core::secrets::SecretStore;
use otto_core::{new_id, Error, Id};
use otto_state::{IssuesRepo, NewIssueAccount};

use crate::jira::JiraClient;

/// Dependencies the issues router needs from the host application state.
pub trait IssuesCtx: Clone + Send + Sync + 'static {
    fn issues(&self) -> &IssuesRepo;
    fn secrets(&self) -> &Arc<dyn SecretStore>;
}

// ---------------------------------------------------------------------------
// Error → response
// ---------------------------------------------------------------------------

struct ApiErr(Error);

impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(body)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the issues router. Paths are relative to the `/api/v1` mount point.
pub fn router<S: IssuesCtx>() -> Router<S> {
    Router::new()
        .route(
            "/issue/accounts",
            get(list_accounts::<S>).post(create_account::<S>),
        )
        .route(
            "/issue/accounts/{id}",
            patch(update_account::<S>).delete(delete_account::<S>),
        )
        .route("/issue/projects", get(list_projects::<S>))
        .route("/issue/search", get(search_issues::<S>))
        .route("/issue/{account_id}/{key}", get(get_issue::<S>))
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_accounts<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
) -> ApiResult<Json<Vec<IssueAccount>>> {
    let accounts = s.issues().list_accounts(&user.0.id).await?;
    Ok(Json(accounts))
}

async fn create_account<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Json(req): Json<CreateIssueAccountReq>,
) -> ApiResult<Json<IssueAccount>> {
    if req.token.trim().is_empty() {
        return Err(Error::Invalid("token must not be empty".into()).into());
    }
    if req.email.trim().is_empty() {
        return Err(Error::Invalid("email must not be empty".into()).into());
    }
    if req.base_url.trim().is_empty() {
        return Err(Error::Invalid("base_url must not be empty".into()).into());
    }
    let token_ref = format!("issueacct-{}", new_id());
    s.secrets().put(&token_ref, &req.token)?;
    let created = s
        .issues()
        .create_account(NewIssueAccount {
            user_id: user.0.id.clone(),
            provider: req.provider,
            label: req.label,
            email: req.email,
            token_ref: token_ref.clone(),
            base_url: req.base_url,
            token_expires_at: req.token_expires_at,
        })
        .await;
    match created {
        Ok(a) => Ok(Json(a)),
        Err(e) => {
            // Don't leave an orphan secret behind.
            let _ = s.secrets().delete(&token_ref);
            Err(e.into())
        }
    }
}

async fn update_account<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
    Json(req): Json<UpdateIssueAccountReq>,
) -> ApiResult<Json<IssueAccount>> {
    let account = s.issues().get_account(&id).await?;
    if account.user_id != user.0.id && !user.0.is_root {
        return Err(Error::Forbidden("not the account owner".into()).into());
    }

    // Merge: absent field keeps current value; present field overwrites.
    let label = req.label.as_deref().unwrap_or(&account.label);
    let email = req.email.as_deref().unwrap_or(&account.email);
    let base_url = req.base_url.as_deref().unwrap_or(&account.base_url);

    // Token rotation: non-empty → store new ref, delete old; empty/absent → keep.
    let token_ref = if let Some(tok) = req.token.as_deref().filter(|t| !t.is_empty()) {
        let new_ref = format!("issueacct-{}", new_id());
        s.secrets().put(&new_ref, tok)?;
        let _ = s.secrets().delete(&account.token_ref);
        new_ref
    } else {
        account.token_ref.clone()
    };

    // token_expires_at: present → set; absent (None) → keep current.
    let token_expires_at = req.token_expires_at.or(account.token_expires_at);

    let updated = s
        .issues()
        .update_account(&id, label, email, &token_ref, base_url, token_expires_at)
        .await?;
    Ok(Json(updated))
}

async fn delete_account<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(id): Path<Id>,
) -> ApiResult<StatusCode> {
    let account = s.issues().get_account(&id).await?;
    if account.user_id != user.0.id && !user.0.is_root {
        return Err(Error::Forbidden("not the account owner".into()).into());
    }
    let _ = s.secrets().delete(&account.token_ref);
    s.issues().delete_account(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_projects<S: IssuesCtx>(
    State(s): State<S>,
    Extension(_user): Extension<AuthUser>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<Json<Vec<IssueProject>>> {
    let account_id: Id = params
        .get("account_id")
        .ok_or_else(|| Error::Invalid("account_id query param required".into()))?
        .clone();
    let account = s.issues().get_account(&account_id).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    let projects = client.list_projects().await?;
    Ok(Json(projects))
}

async fn search_issues<S: IssuesCtx>(
    State(s): State<S>,
    Extension(_user): Extension<AuthUser>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<Json<Vec<IssueSummary>>> {
    let account_id: Id = params
        .get("account_id")
        .ok_or_else(|| Error::Invalid("account_id query param required".into()))?
        .clone();
    let q = params
        .get("q")
        .ok_or_else(|| Error::Invalid("q query param required".into()))?
        .clone();
    let project = params
        .get("project")
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let account = s.issues().get_account(&account_id).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    let results = client.search(&q, project.as_deref()).await?;
    Ok(Json(results))
}

async fn get_issue<S: IssuesCtx>(
    State(s): State<S>,
    Extension(_user): Extension<AuthUser>,
    Path((account_id, key)): Path<(Id, String)>,
) -> ApiResult<Json<IssueDetail>> {
    let account = s.issues().get_account(&account_id).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    let detail = client.get_issue(&key).await?;
    Ok(Json(detail))
}
