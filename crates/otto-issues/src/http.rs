//! Axum router for issue-tracking endpoints.
//! All paths are relative to the `/api/v1` mount point.

use std::collections::HashMap;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};
use axum::{Extension, Json, Router};
use otto_core::api::{CreateIssueAccountReq, Problem, UpdateIssueAccountReq};
use otto_core::auth::AuthUser;
use otto_core::domain::{IssueAccount, IssueDetail, IssueProject, IssueSummary};
use otto_core::secrets::SecretStore;
use otto_core::{new_id, Error, Id};
use otto_state::{IssuesRepo, NewIssueAccount};

use crate::confluence::ConfluenceClient;
use crate::confluence::{ConfluencePageSummary, ConfluenceSpace};
use crate::jira::{CommentRef, IssueFull, JiraClient, JiraTransition, JiraUser};

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
// Ownership authorization
// ---------------------------------------------------------------------------

/// Reject access to an issue account that the caller does not own.
///
/// The caller is permitted only when they are the account owner or root. This
/// guard is the single chokepoint for every handler that resolves an
/// `account_id` and acts with that account's Atlassian credentials — without it,
/// any authenticated user could read or write through another user's Jira /
/// Confluence identity.
fn authorize_account(account: &IssueAccount, user: &AuthUser) -> Result<(), Error> {
    if account.user_id != user.0.id && !user.0.is_root {
        return Err(Error::Forbidden("not the account owner".into()));
    }
    Ok(())
}

/// Load an issue account by id and authorize the caller in one step.
///
/// Returns the account only when the caller owns it (or is root). Every read /
/// use handler must obtain accounts through this helper so the ownership check
/// can never be forgotten.
async fn load_authorized_account<S: IssuesCtx>(
    s: &S,
    id: &Id,
    user: &AuthUser,
) -> Result<IssueAccount, Error> {
    let account = s.issues().get_account(id).await?;
    authorize_account(&account, user)?;
    Ok(account)
}

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
        .route("/issue/confluence/spaces", get(list_spaces_cf::<S>))
        .route("/issue/confluence/search", get(search_pages_cf::<S>))
        .route("/issue/{account_id}/{key}", get(get_issue::<S>))
        // Extended issue view + write operations
        .route("/issue/{account_id}/{key}/full", get(get_issue_full::<S>))
        .route(
            "/issue/{account_id}/{key}/transitions",
            get(list_transitions::<S>).post(do_transition::<S>),
        )
        .route("/issue/{account_id}/{key}/assignable", get(list_assignable::<S>))
        .route("/issue/{account_id}/{key}/assignee", put(assign_issue::<S>))
        .route(
            "/issue/{account_id}/{key}/attachment/{attachment_id}",
            get(get_attachment::<S>),
        )
        .route(
            "/issue/{account_id}/{project_key}/issue-types",
            get(list_issue_types_handler::<S>),
        )
        .route(
            "/issue/{account_id}/{key}/comment",
            post(add_comment::<S>),
        )
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
    let account = load_authorized_account(&s, &id, &user).await?;

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
    let account = load_authorized_account(&s, &id, &user).await?;
    let _ = s.secrets().delete(&account.token_ref);
    s.issues().delete_account(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_projects<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<Json<Vec<IssueProject>>> {
    let account_id: Id = params
        .get("account_id")
        .ok_or_else(|| Error::Invalid("account_id query param required".into()))?
        .clone();
    let account = load_authorized_account(&s, &account_id, &user).await?;
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
    Extension(user): Extension<AuthUser>,
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
    let account = load_authorized_account(&s, &account_id, &user).await?;
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
    Extension(user): Extension<AuthUser>,
    Path((account_id, key)): Path<(Id, String)>,
) -> ApiResult<Json<IssueDetail>> {
    let account = load_authorized_account(&s, &account_id, &user).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    let detail = client.get_issue(&key).await?;
    Ok(Json(detail))
}

async fn list_spaces_cf<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<Json<Vec<ConfluenceSpace>>> {
    let account_id: Id = params
        .get("account_id")
        .ok_or_else(|| Error::Invalid("account_id query param required".into()))?
        .clone();
    let account = load_authorized_account(&s, &account_id, &user).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = ConfluenceClient::new(&account.base_url, &account.email, &token);
    let spaces = client.list_spaces().await?;
    Ok(Json(spaces))
}

async fn search_pages_cf<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<Json<Vec<ConfluencePageSummary>>> {
    let account_id: Id = params
        .get("account_id")
        .ok_or_else(|| Error::Invalid("account_id query param required".into()))?
        .clone();
    let q = params
        .get("q")
        .ok_or_else(|| Error::Invalid("q query param required".into()))?
        .clone();
    let space = params
        .get("space")
        .map(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let account = load_authorized_account(&s, &account_id, &user).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = ConfluenceClient::new(&account.base_url, &account.email, &token);
    let results = client.search_pages(space.as_deref(), &q).await?;
    Ok(Json(results))
}

// ─────────────────────────────────────────────────────────────────────────────
// Extended issue view + write handlers
// ─────────────────────────────────────────────────────────────────────────────

/// `GET /issue/{account_id}/{key}/full`
///
/// Returns a fully-expanded [`IssueFull`] containing description, comments,
/// changelog, attachments, links, and all non-empty fields with display labels.
async fn get_issue_full<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((account_id, key)): Path<(Id, String)>,
) -> ApiResult<Json<IssueFull>> {
    let account = load_authorized_account(&s, &account_id, &user).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    let full = client.get_issue_full(&key).await?;
    Ok(Json(full))
}

/// `GET /issue/{account_id}/{key}/transitions`
///
/// Returns the list of status transitions available for the issue.
async fn list_transitions<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((account_id, key)): Path<(Id, String)>,
) -> ApiResult<Json<Vec<JiraTransition>>> {
    let account = load_authorized_account(&s, &account_id, &user).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    let transitions = client.list_transitions(&key).await?;
    Ok(Json(transitions))
}

/// `POST /issue/{account_id}/{key}/transitions`
///
/// Body: `{"transition_id": "21"}` — move the issue to the target status.
/// Returns `204 No Content` on success.
async fn do_transition<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((account_id, key)): Path<(Id, String)>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<StatusCode> {
    let transition_id = body
        .get("transition_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Invalid("transition_id is required".into()))?
        .to_string();
    let account = load_authorized_account(&s, &account_id, &user).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    client.transition_issue(&key, &transition_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /issue/{account_id}/{key}/assignable`
///
/// Returns users that can be assigned to the issue.
async fn list_assignable<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((account_id, key)): Path<(Id, String)>,
) -> ApiResult<Json<Vec<JiraUser>>> {
    let account = load_authorized_account(&s, &account_id, &user).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    let users = client.list_assignable(&key).await?;
    Ok(Json(users))
}

/// `PUT /issue/{account_id}/{key}/assignee`
///
/// Body: `{"account_id": "..."}` — assign the issue to a user.
/// Pass `"-1"` as the account_id value to unassign.
/// Returns `204 No Content` on success.
async fn assign_issue<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((account_id, key)): Path<(Id, String)>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<StatusCode> {
    let assignee_account_id = body
        .get("account_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Invalid("account_id is required".into()))?
        .to_string();
    let account = load_authorized_account(&s, &account_id, &user).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    client.assign_issue(&key, &assignee_account_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `GET /issue/{account_id}/{key}/attachment/{attachment_id}`
///
/// Proxies the attachment bytes from Jira (keeping the auth token server-side).
/// Sets the correct `Content-Type` header from the upstream response so the
/// browser can render images / PDFs directly.
async fn get_attachment<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((account_id, _key, attachment_id)): Path<(Id, String, String)>,
) -> ApiResult<Response> {
    let account = load_authorized_account(&s, &account_id, &user).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    let (mime, bytes) = client.attachment_bytes(&attachment_id).await?;

    let content_type = HeaderValue::from_str(&mime)
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(Body::from(bytes))
        .unwrap_or_else(|_| {
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(Body::empty())
                .unwrap()
        });

    Ok(response)
}

/// `GET /issue/{account_id}/{project_key}/issue-types`
///
/// Returns the non-subtask issue types for a Jira project (e.g. "Story", "Task", "Bug").
async fn list_issue_types_handler<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((account_id, project_key)): Path<(Id, String)>,
) -> ApiResult<Json<Vec<String>>> {
    let account = load_authorized_account(&s, &account_id, &user).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    let types = client.list_issue_types(&project_key).await?;
    Ok(Json(types))
}

/// `POST /issue/{account_id}/{key}/comment`
///
/// Body: `{"body": "Comment text"}` — post a comment on the issue.
/// Returns a [`CommentRef`] containing the new comment's `id` and optional URL.
async fn add_comment<S: IssuesCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((account_id, key)): Path<(Id, String)>,
    Json(body): Json<serde_json::Value>,
) -> ApiResult<Json<CommentRef>> {
    let comment_body = body
        .get("body")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Invalid("body is required".into()))?
        .to_string();
    let account = load_authorized_account(&s, &account_id, &user).await?;
    let token = s
        .secrets()
        .get(&account.token_ref)?
        .ok_or_else(|| Error::Invalid(format!("token missing for issue account {}", account.id)))?;
    let client = JiraClient::new(&account.base_url, &account.email, &token);
    let comment_ref = client.add_comment(&key, &comment_body).await?;
    Ok(Json(comment_ref))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use otto_core::domain::{IssueProviderKind, User};

    fn account_owned_by(owner: &str) -> IssueAccount {
        IssueAccount {
            id: "acct-1".into(),
            user_id: owner.into(),
            provider: IssueProviderKind::Jira,
            label: "work".into(),
            email: "owner@example.com".into(),
            token_ref: "issueacct-1".into(),
            base_url: "https://example.atlassian.net".into(),
            token_expires_at: None,
            created_at: Utc::now(),
        }
    }

    fn user(id: &str, is_root: bool) -> AuthUser {
        AuthUser(User {
            id: id.into(),
            username: id.into(),
            display_name: id.into(),
            is_root,
            disabled: false,
            created_at: Utc::now(),
        })
    }

    #[test]
    fn owner_is_authorized() {
        let account = account_owned_by("alice");
        assert!(authorize_account(&account, &user("alice", false)).is_ok());
    }

    #[test]
    fn non_owner_is_forbidden() {
        let account = account_owned_by("alice");
        let err = authorize_account(&account, &user("mallory", false)).unwrap_err();
        assert!(matches!(err, Error::Forbidden(_)), "expected Forbidden, got {err:?}");
    }

    #[test]
    fn root_can_access_any_account() {
        let account = account_owned_by("alice");
        assert!(authorize_account(&account, &user("root", true)).is_ok());
    }
}
