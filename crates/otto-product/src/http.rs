//! Product Story Analysis REST router.
//!
//! Two-tier routing:
//!   - Collection routes: `/workspaces/{ws}/product/...` (workspace in path)
//!   - Item routes:       `/product/<entity>/{id}`       (flat; workspace resolved from row)
//!
//! Reads require workspace `Viewer`; mutations require workspace `Editor`.
//! The server nests this under `/api/v1`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use otto_state::{
    LearningPatch, NewEvent, NewLearning, NewNote, NewQuestion, NewTranscript, ProductRepo,
    QuestionPatch, RunFilter, StoryPatch, TestcasePatch,
};
use serde::Deserialize;

use crate::service::ProductService;
use crate::types::{
    BulkApproveTestcasesReq, CreateChildReq, NewDraftReq, NewLearningReq, NewNoteReq,
    NewQuestionReq, NewTranscriptReq, PostQuestionsReq, PublishAsRfcReq, PublishAsStoryReq,
    PublishTestsReq, ReorderTestcasesReq, StorySwarmLink, UpdateDraftReq, UpdateLearningReq,
    UpdateNoteReq, UpdateQuestionReq, UpdateStoryReq, UpdateTestcaseReq,
};

// ---------------------------------------------------------------------------
// Context trait
// ---------------------------------------------------------------------------

/// Host-application context required by the product router.
pub trait ProductCtx: Clone + Send + Sync + 'static {
    fn product(&self) -> &Arc<ProductService>;
    fn product_repo(&self) -> &ProductRepo;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    /// Swarm repository — used by the Product↔Swarm closure endpoint to read
    /// swarm projects/tasks/runs linked to a story.  Implementors that have no
    /// swarm layer return `None`; the endpoint returns an empty `StorySwarmLink`.
    fn swarm_repo(&self) -> Option<&otto_state::SwarmRepo> {
        None
    }
    /// Root directory of story attachment files
    /// (`data_dir/product/attachments`). `DELETE /product/stories/{sid}` removes
    /// `<root>/<sid>/` best-effort after the rows are gone; implementors without
    /// a filesystem (tests) return `None` and only the rows are deleted.
    fn attachments_root(&self) -> Option<std::path::PathBuf> {
        None
    }
    /// Root of the design-assist scratch dirs (`data_dir/product/mockup_assist`,
    /// one `<attachment_id>/` per artifact — see `otto-server::mockup_assist`).
    /// Removed best-effort with the story's attachments on delete.
    fn mockup_scratch_root(&self) -> Option<std::path::PathBuf> {
        None
    }
    /// Attachment repo, used only to enumerate a story's attachment ids before
    /// the rows go (for the scratch-dir cleanup above). `None` = skip.
    fn attachment_repo(&self) -> Option<&otto_state::ProductAttachmentRepo> {
        None
    }
    /// The workspace's root folder, for validating a story `cwd` on PATCH
    /// (S4-13). Default `None` (then only temp dirs / git checkouts pass).
    fn workspace_root<'a>(
        &'a self,
        _ws: &'a Id,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Option<String>> + Send + 'a>> {
        Box::pin(async { None })
    }
    /// Stop every live agent of `story_id` — analysis agents AND the
    /// rewrite / test-generation / plan sessions (trip their cancel flags so
    /// the recovery loop does not retry, then kill the sessions) — called by
    /// `DELETE /product/stories/{sid}` BEFORE the rows go (S4-23), so deleted
    /// stories don't keep agents burning budget until the next restart.
    /// Default: no-op (hosts without sessions).
    fn stop_story_agents<'a>(
        &'a self,
        _story_id: &'a Id,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async {})
    }
}

// ---------------------------------------------------------------------------
// Error → response
// ---------------------------------------------------------------------------

/// `otto_core::Error` as an RFC-7807-ish `Problem` response (same status
/// mapping as `otto-server`'s `ApiError`).
#[derive(Debug)]
pub struct ApiError(pub Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        if status == StatusCode::INTERNAL_SERVER_ERROR {
            tracing::error!("internal error: {}", self.0);
        }
        let problem = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(problem)).into_response()
    }
}

pub type ApiResult<T> = std::result::Result<T, ApiError>;

/// Extractor for the authenticated user (the [`AuthUser`] extension the
/// host's auth middleware inserts); rejects with 401 when absent. Mirrors
/// `otto-server`'s `auth::CurrentUser` for the handlers that moved here.
#[derive(Debug, Clone)]
pub struct CurrentUser(pub otto_core::domain::User);

impl<S> axum::extract::FromRequestParts<S> for CurrentUser
where
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthUser>()
            .map(|a| CurrentUser(a.0.clone()))
            .ok_or(ApiError(Error::Unauthorized))
    }
}

// ---------------------------------------------------------------------------
// Path extractors — collection tier (workspace-scoped)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct WsPath {
    ws: Id,
}

// ---------------------------------------------------------------------------
// Path extractors — item tier (flat)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct StoryId {
    sid: Id,
}

#[derive(Deserialize)]
struct VersionId {
    vid: Id,
}

#[derive(Deserialize)]
struct AnalysisId {
    aid: Id,
}

#[derive(Deserialize)]
struct QuestionId {
    qid: Id,
}

#[derive(Deserialize)]
struct NoteId {
    nid: Id,
}

#[derive(Deserialize)]
struct TestcaseId {
    tid: Id,
}

#[derive(Deserialize)]
struct RunId {
    rid: Id,
}

#[derive(Deserialize)]
struct LearningId {
    lid: Id,
}

#[derive(Deserialize)]
struct TranscriptItemId {
    trid: Id,
}

// ---------------------------------------------------------------------------
// Query extractors
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ActiveQuery {
    #[serde(default)]
    active: Option<bool>,
}

#[derive(Deserialize)]
struct SectionQuery {
    #[serde(default)]
    section: Option<String>,
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

/// Build the product router. Paths are relative to the `/api/v1` mount point.
pub fn router<S: ProductCtx>() -> Router<S> {
    Router::new()
        // ---- Collection routes (workspace-prefixed) ----
        // Stories
        .route(
            "/workspaces/{ws}/product/stories",
            get(list_stories::<S>).post(import_story::<S>),
        )
        // Learnings
        .route(
            "/workspaces/{ws}/product/learnings",
            get(list_learnings::<S>).post(create_learning::<S>),
        )
        // ---- Item routes (flat /product/<entity>/{id}) ----
        // Stories
        .route(
            "/product/stories/{sid}",
            get(get_story::<S>)
                .patch(patch_story::<S>)
                .delete(delete_story::<S>),
        )
        .route("/product/stories/{sid}/refresh", post(refresh_story::<S>))
        // Epic tree: file a `story` / `doc` child under an epic.
        .route("/product/stories/{sid}/children", post(create_child::<S>))
        // Versions (under-story collection + flat version item)
        .route("/product/stories/{sid}/versions", get(list_versions::<S>))
        .route("/product/versions/{vid}", get(get_version::<S>))
        .route(
            "/product/versions/{vid}/publish",
            post(publish_version::<S>),
        )
        // Analyses (under-story collection + flat analysis item)
        .route("/product/stories/{sid}/analyses", get(list_analyses::<S>))
        .route("/product/analyses/{aid}", get(get_analysis::<S>))
        // Questions (under-story collection + flat question item)
        .route(
            "/product/stories/{sid}/questions",
            get(list_questions::<S>).post(create_question::<S>),
        )
        .route(
            "/product/stories/{sid}/questions/post",
            post(post_questions::<S>),
        )
        .route(
            "/product/questions/{qid}",
            patch(update_question::<S>).delete(delete_question::<S>),
        )
        // Notes (under-story collection + flat note item)
        .route(
            "/product/stories/{sid}/notes",
            get(list_notes::<S>).post(create_note::<S>),
        )
        .route(
            "/product/notes/{nid}",
            patch(update_note::<S>).delete(delete_note::<S>),
        )
        // Events
        .route("/product/stories/{sid}/events", get(list_events::<S>))
        // Testcases
        .route(
            "/product/stories/{sid}/testcases",
            get(list_testcase_runs::<S>),
        )
        .route("/product/testcases/{tid}", patch(update_testcase::<S>))
        // NOTE: /product/testcase-runs/{rid}/approve is registered in otto-server
        // (modules.rs) so it can trigger skill self-improvement. Registering it
        // here too would cause an axum duplicate-route panic at startup.
        .route(
            "/product/testcase-runs/{rid}/publish",
            post(publish_tests::<S>),
        )
        // Bulk-approve a selected subset of draft test cases within a run.
        .route(
            "/product/testcase-runs/{rid}/testcases/bulk-approve",
            post(bulk_approve_testcases::<S>),
        )
        // Persist a new display ordering for the run's cases.
        .route(
            "/product/testcase-runs/{rid}/testcases/reorder",
            post(reorder_testcases::<S>),
        )
        // Inject bundle
        .route("/product/stories/{sid}/inject", get(get_inject::<S>))
        // Learnings (flat item)
        .route(
            "/product/learnings/{lid}",
            patch(update_learning::<S>).delete(delete_learning::<S>),
        )
        .route(
            "/product/learnings/{lid}/accept",
            post(accept_learning::<S>),
        )
        // Drafts
        .route("/workspaces/{ws}/product/drafts", post(create_draft::<S>))
        // Draft body update
        .route(
            "/product/stories/{sid}/draft",
            patch(update_draft_body::<S>),
        )
        // Transcripts
        .route(
            "/product/stories/{sid}/transcripts",
            get(list_transcripts::<S>).post(create_transcript::<S>),
        )
        .route(
            "/product/stories/{sid}/transcripts/search",
            get(search_transcripts::<S>),
        )
        .route(
            "/product/transcripts/{trid}",
            get(get_transcript::<S>).delete(delete_transcript::<S>),
        )
        // Publish discovery
        .route(
            "/product/stories/{sid}/publish-as-rfc",
            post(publish_as_rfc::<S>),
        )
        .route(
            "/product/stories/{sid}/publish-as-story",
            post(publish_as_story::<S>),
        )
        // Product↔Swarm closure: full swarm project view linked to a story.
        .route("/product/stories/{sid}/swarm", get(story_swarm_link::<S>))
}

// ---------------------------------------------------------------------------
// Helper: resolve workspace from a story id, then role-check
// ---------------------------------------------------------------------------

async fn ws_from_story<S: ProductCtx>(
    ctx: &S,
    user: &otto_core::domain::User,
    story_id: &Id,
    role: WorkspaceRole,
) -> ApiResult<Id> {
    let story = ctx.product_repo().get_story(story_id).await?;
    ctx.roles().check(user, &story.workspace_id, role).await?;
    Ok(story.workspace_id)
}

// ---------------------------------------------------------------------------
// Stories — collection (workspace-scoped)
// ---------------------------------------------------------------------------

async fn list_stories<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &ws, WorkspaceRole::Viewer).await?;
    // Product is a global library — list every story regardless of workspace.
    let stories = ctx.product_repo().list_stories().await?;
    Ok(Json(stories).into_response())
}

async fn import_story<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(req): Json<crate::types::ImportStoryReq>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &ws, WorkspaceRole::Editor).await?;
    // S4: the caller may only bind a story to an issue account they own (or root).
    ctx.product()
        .authorize_account_id(&req.account_id, &user)
        .await?;
    let detail = ctx.product().import_story(&ws, &req, &user.id).await?;
    Ok(Json(detail).into_response())
}

// ---------------------------------------------------------------------------
// Stories — item (flat)
// ---------------------------------------------------------------------------

async fn get_story<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<Response> {
    // Resolve workspace from row, then role-check
    let story = ctx.product_repo().get_story(&sid).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Viewer)
        .await?;
    let detail = ctx.product().story_detail(&sid).await?;
    Ok(Json(detail).into_response())
}

async fn patch_story<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Json(req): Json<UpdateStoryReq>,
) -> ApiResult<Response> {
    let ws = ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    // Epic tree: one level only. A new parent must be a top-level story and this
    // story must not have children of its own; `tree_kind` is a closed enum.
    let tree_kind = match req.tree_kind.as_deref() {
        Some(k) => Some(crate::service::validate_tree_kind(k)?.to_string()),
        None => None,
    };
    if let Some(Some(pid)) = req.parent_id.as_ref() {
        ctx.product().validate_parent(pid, Some(&sid)).await?;
    }
    let updated = ctx
        .product_repo()
        .update_story(
            &sid,
            StoryPatch {
                title: None,
                url: None,
                issue_type: None,
                stage: req.stage,
                cwd: match req.cwd {
                    // Agents are spawned (and pre-trusted) here — validate (S4-13).
                    Some(c) if !c.trim().is_empty() => {
                        let root = ctx.workspace_root(&ws).await;
                        Some(Some(crate::service::validate_agent_cwd(
                            &c,
                            root.as_deref(),
                        )?))
                    }
                    other => other.map(Some),
                },
                watch_enabled: req.watch_enabled,
                watch_cadence_min: req.watch_cadence_min,
                confluence_tests_page_id: None,
                confluence_tests_url: None,
                tags: req.tags,
                parent_id: req.parent_id,
                tree_kind,
                folder: req.folder.map(|f| f.trim().to_string()),
                ..Default::default()
            },
        )
        .await?;
    ctx.product_repo()
        .add_event(NewEvent {
            story_id: sid.clone(),
            section: "story".into(),
            kind: "patch".into(),
            summary: "Story settings updated".into(),
            actor_id: Some(user.id),
            meta_json: None,
        })
        .await?;
    let _ = ws; // ws used for role-check only
    Ok(Json(updated).into_response())
}

async fn delete_story<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<StatusCode> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    // Stop the story's running agents first (S4-23): once the rows are gone
    // their writes fail silently while they keep spending.
    ctx.stop_story_agents(&sid).await;
    // Attachment ids BEFORE the rows go: each may own an assist scratch dir.
    let attachment_ids: Vec<Id> = match ctx.attachment_repo() {
        Some(repo) => repo
            .list_for_story(&sid)
            .await
            .map(|atts| atts.into_iter().map(|a| a.id).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    };
    ctx.product_repo().delete_story(&sid).await?;
    // Best-effort file cleanup: the story's attachment dir + each artifact's
    // assist scratch dir. The rows are already gone (source of truth); ids are
    // route params / daemon-minted, so confine every join before touching the fs.
    if let Some(root) = ctx.attachments_root() {
        if let Some(dir) = otto_core::paths::confine_join(&root, &sid) {
            let _ = tokio::fs::remove_dir_all(&dir).await;
        }
    }
    if let Some(root) = ctx.mockup_scratch_root() {
        for aid in &attachment_ids {
            if let Some(dir) = otto_core::paths::confine_join(&root, aid) {
                let _ = tokio::fs::remove_dir_all(&dir).await;
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /product/stories/{sid}/children` — Editor on the epic's workspace.
async fn create_child<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Json(req): Json<CreateChildReq>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    let detail = ctx
        .product()
        .create_child(
            &sid,
            &user.id,
            req.title
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty()),
            req.tree_kind.as_deref().unwrap_or("doc"),
            req.folder.as_deref().unwrap_or(""),
        )
        .await?;
    Ok(Json(detail).into_response())
}

async fn refresh_story<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    // S4: refreshing re-fetches via the story's bound credential — owner/root only.
    ctx.product().authorize_story_account(&sid, &user).await?;
    let detail = ctx.product().refresh_story(&sid, &user.id).await?;
    Ok(Json(detail).into_response())
}

// ---------------------------------------------------------------------------
// Versions
// ---------------------------------------------------------------------------

async fn list_versions<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;
    let versions = ctx.product_repo().list_versions(&sid).await?;
    Ok(Json(versions).into_response())
}

async fn get_version<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(VersionId { vid }): Path<VersionId>,
) -> ApiResult<Response> {
    // Resolve workspace via version → story chain
    let version = ctx.product_repo().get_version(&vid).await?;
    let story = ctx.product_repo().get_story(&version.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Viewer)
        .await?;
    Ok(Json(version).into_response())
}

async fn publish_version<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(VersionId { vid }): Path<VersionId>,
) -> ApiResult<Response> {
    // Resolve workspace via version → story chain, then role-check.
    let version = ctx.product_repo().get_version(&vid).await?;
    let story = ctx.product_repo().get_story(&version.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;
    // S4: publishing pushes through the story's bound credential — owner/root only.
    ctx.product()
        .authorize_story_account(&version.story_id, &user)
        .await?;
    // Delegate to service: push to issue tracker + record publish event.
    let (url, source_ref) = ctx.product().publish_version(&vid, &user.id).await?;
    Ok(Json(serde_json::json!({ "url": url, "ref": source_ref })).into_response())
}

// ---------------------------------------------------------------------------
// Analyses
// ---------------------------------------------------------------------------

async fn list_analyses<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;
    let analyses = ctx.product_repo().list_analyses(&sid).await?;
    Ok(Json(analyses).into_response())
}

async fn get_analysis<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(AnalysisId { aid }): Path<AnalysisId>,
) -> ApiResult<Response> {
    // Resolve workspace via analysis → story
    let analysis = ctx.product_repo().get_analysis(&aid).await?;
    let story = ctx.product_repo().get_story(&analysis.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Viewer)
        .await?;
    let agents = ctx.product_repo().list_analysis_agents(&aid).await?;
    let detail = crate::types::ProductAnalysisDetail { analysis, agents };
    Ok(Json(detail).into_response())
}

// ---------------------------------------------------------------------------
// Questions
// ---------------------------------------------------------------------------

async fn list_questions<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;
    let questions = ctx.product_repo().list_questions(&sid).await?;
    Ok(Json(questions).into_response())
}

async fn create_question<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Json(req): Json<NewQuestionReq>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    let question = ctx
        .product_repo()
        .create_question(NewQuestion {
            story_id: sid.clone(),
            analysis_id: None,
            text: req.text.clone(),
            rationale: req.rationale.unwrap_or_default(),
            category: req.category.unwrap_or_else(|| "general".into()),
            created_by: user.id.clone(),
        })
        .await?;
    ctx.product_repo()
        .add_event(NewEvent {
            story_id: sid.clone(),
            section: "questions".into(),
            kind: "create".into(),
            summary: format!("Question added: {}", req.text),
            actor_id: Some(user.id),
            meta_json: None,
        })
        .await?;
    Ok(Json(question).into_response())
}

async fn update_question<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(QuestionId { qid }): Path<QuestionId>,
    Json(req): Json<UpdateQuestionReq>,
) -> ApiResult<Response> {
    // Resolve workspace via question → story
    let q = ctx.product_repo().get_question(&qid).await?;
    let story = ctx.product_repo().get_story(&q.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;
    let updated = ctx
        .product_repo()
        .update_question(
            &qid,
            QuestionPatch {
                text: req.text,
                rationale: req.rationale,
                category: req.category,
                status: req.status,
                answer: req.answer.map(Some),
                posted_ref: None,
            },
        )
        .await?;
    Ok(Json(updated).into_response())
}

async fn delete_question<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(QuestionId { qid }): Path<QuestionId>,
) -> ApiResult<StatusCode> {
    // Resolve workspace via question → story
    let q = ctx.product_repo().get_question(&qid).await?;
    let story = ctx.product_repo().get_story(&q.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;
    ctx.product_repo().delete_question(&qid).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn post_questions<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Json(req): Json<PostQuestionsReq>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    // S4: posting comments uses the story's bound credential — owner/root only.
    ctx.product().authorize_story_account(&sid, &user).await?;
    let results = ctx
        .product()
        .post_questions(&sid, &req.ids, req.format.as_deref(), &user.id)
        .await?;
    let posted: Vec<serde_json::Value> = results
        .into_iter()
        .map(|(id, cref)| {
            serde_json::json!({
                "id": id,
                "ref": cref.id,
                "url": cref.url,
            })
        })
        .collect();
    Ok(Json(serde_json::json!({ "posted": posted })).into_response())
}

// ---------------------------------------------------------------------------
// Notes
// ---------------------------------------------------------------------------

async fn list_notes<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;
    let notes = ctx.product_repo().list_notes(&sid).await?;
    Ok(Json(notes).into_response())
}

async fn create_note<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Json(req): Json<NewNoteReq>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    let note = ctx
        .product_repo()
        .create_note(NewNote {
            story_id: sid.clone(),
            section: req.section.clone(),
            body: req.body.clone(),
            author_id: user.id.clone(),
        })
        .await?;
    ctx.product_repo()
        .add_event(NewEvent {
            story_id: sid.clone(),
            section: req.section.unwrap_or_else(|| "notes".into()),
            kind: "create".into(),
            summary: "Note added".into(),
            actor_id: Some(user.id),
            meta_json: None,
        })
        .await?;
    Ok(Json(note).into_response())
}

async fn update_note<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(NoteId { nid }): Path<NoteId>,
    Json(req): Json<UpdateNoteReq>,
) -> ApiResult<Response> {
    // Resolve workspace via note → story
    let note = ctx.product_repo().get_note(&nid).await?;
    let story = ctx.product_repo().get_story(&note.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;
    let updated = ctx.product_repo().update_note(&nid, &req.body).await?;
    Ok(Json(updated).into_response())
}

async fn delete_note<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(NoteId { nid }): Path<NoteId>,
) -> ApiResult<StatusCode> {
    // Resolve workspace via note → story
    let note = ctx.product_repo().get_note(&nid).await?;
    let story = ctx.product_repo().get_story(&note.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;
    ctx.product_repo().delete_note(&nid).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

async fn list_events<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Query(q): Query<SectionQuery>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;
    let events = ctx
        .product_repo()
        .list_events(&sid, q.section.as_deref())
        .await?;
    Ok(Json(events).into_response())
}

// ---------------------------------------------------------------------------
// Learnings — collection (workspace-scoped)
// ---------------------------------------------------------------------------

async fn list_learnings<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Query(q): Query<ActiveQuery>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &ws, WorkspaceRole::Viewer).await?;
    let active_only = q.active.unwrap_or(false);
    // Global learnings knowledge base — shared across every workspace.
    let learnings = ctx.product_repo().list_learnings(active_only).await?;
    Ok(Json(learnings).into_response())
}

async fn create_learning<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(req): Json<NewLearningReq>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &ws, WorkspaceRole::Editor).await?;
    let learning = ctx
        .product_repo()
        .create_learning(NewLearning {
            workspace_id: ws.clone(),
            kind: req.kind,
            title: req.title,
            body: req.body,
            tags: req.tags.unwrap_or_default(),
            refs_json: req
                .refs
                .map(|v| v.to_string())
                .unwrap_or_else(|| "[]".into()),
            source_story_id: req.source_story_id,
            created_by: user.id.clone(),
        })
        .await?;
    Ok(Json(learning).into_response())
}

// ---------------------------------------------------------------------------
// Learnings — item (flat)
// ---------------------------------------------------------------------------

async fn update_learning<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(LearningId { lid }): Path<LearningId>,
    Json(req): Json<UpdateLearningReq>,
) -> ApiResult<Response> {
    // Resolve workspace directly from learning row
    let learning = ctx.product_repo().get_learning(&lid).await?;
    ctx.roles()
        .check(&user, &learning.workspace_id, WorkspaceRole::Editor)
        .await?;
    let updated = ctx
        .product_repo()
        .update_learning(
            &lid,
            LearningPatch {
                kind: req.kind,
                title: req.title,
                body: req.body,
                tags: req.tags,
                refs_json: req.refs.map(|v| v.to_string()),
                active: req.active,
            },
        )
        .await?;
    Ok(Json(updated).into_response())
}

async fn delete_learning<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(LearningId { lid }): Path<LearningId>,
) -> ApiResult<StatusCode> {
    let learning = ctx.product_repo().get_learning(&lid).await?;
    ctx.roles()
        .check(&user, &learning.workspace_id, WorkspaceRole::Editor)
        .await?;
    ctx.product_repo().delete_learning(&lid).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn accept_learning<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(LearningId { lid }): Path<LearningId>,
) -> ApiResult<Response> {
    let learning = ctx.product_repo().get_learning(&lid).await?;
    ctx.roles()
        .check(&user, &learning.workspace_id, WorkspaceRole::Editor)
        .await?;
    let updated = ctx
        .product_repo()
        .update_learning(
            &lid,
            LearningPatch {
                kind: None,
                title: None,
                body: None,
                tags: None,
                refs_json: None,
                active: Some(true),
            },
        )
        .await?;
    Ok(Json(updated).into_response())
}

// ---------------------------------------------------------------------------
// Testcase runs
// ---------------------------------------------------------------------------

/// `GET /product/stories/{sid}/testcases` — returns a list of `ProductTestcaseRunDetail`
/// (each run bundled with its cases).
async fn list_testcase_runs<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;
    let runs = ctx.product_repo().list_testcase_runs(&sid).await?;
    let mut details = Vec::with_capacity(runs.len());
    for run in runs {
        let cases = ctx.product_repo().list_testcases(&run.id).await?;
        details.push(crate::types::ProductTestcaseRunDetail { run, cases });
    }
    Ok(Json(details).into_response())
}

async fn update_testcase<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(TestcaseId { tid }): Path<TestcaseId>,
    Json(req): Json<UpdateTestcaseReq>,
) -> ApiResult<Response> {
    // Resolve workspace via testcase → run → story
    let tc = ctx.product_repo().get_testcase(&tid).await?;
    let run = ctx.product_repo().get_testcase_run(&tc.run_id).await?;
    let story = ctx.product_repo().get_story(&run.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;
    let updated = ctx
        .product_repo()
        .update_testcase(
            &tid,
            TestcasePatch {
                title: req.title,
                category: req.category,
                priority: req.priority,
                steps_json: req.steps.map(|v| v.to_string()),
                status: req.status,
                review_note: req.review_note.map(Some),
                order_idx: req.order_idx,
            },
        )
        .await?;
    Ok(Json(updated).into_response())
}

async fn publish_tests<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(RunId { rid }): Path<RunId>,
    Json(req): Json<PublishTestsReq>,
) -> ApiResult<Response> {
    // Resolve workspace via run → story, then role-check.
    let run = ctx.product_repo().get_testcase_run(&rid).await?;
    let story = ctx.product_repo().get_story(&run.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;
    // S4: publishing tests writes through the story's bound credential — owner/root only.
    ctx.product()
        .authorize_story_account(&run.story_id, &user)
        .await?;
    let url = ctx
        .product()
        .publish_testcases(
            &rid,
            &user.id,
            req.space_key.as_deref(),
            req.parent_id.as_deref(),
        )
        .await?;
    Ok(Json(serde_json::json!({ "url": url })).into_response())
}

/// `POST /product/testcase-runs/{rid}/testcases/bulk-approve` — approve a
/// caller-selected subset of draft test cases in a run.  Already-approved or
/// rejected cases are left unchanged.  Returns the number of rows flipped.
async fn bulk_approve_testcases<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(RunId { rid }): Path<RunId>,
    Json(req): Json<BulkApproveTestcasesReq>,
) -> ApiResult<Response> {
    // Resolve workspace via run → story, then role-check.
    let run = ctx.product_repo().get_testcase_run(&rid).await?;
    let story = ctx.product_repo().get_story(&run.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;
    let count = ctx
        .product_repo()
        .bulk_approve_testcases(&rid, &req.ids)
        .await?;
    Ok(Json(serde_json::json!({ "approved": count })).into_response())
}

/// `POST /product/testcase-runs/{rid}/testcases/reorder` — persist a new
/// display ordering for the run's test cases.  The request body supplies the
/// full ordered list of ids; each id receives an `order_idx` equal to its
/// zero-based position.  Ids not included are left at their current index.
async fn reorder_testcases<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(RunId { rid }): Path<RunId>,
    Json(req): Json<ReorderTestcasesReq>,
) -> ApiResult<Response> {
    let run = ctx.product_repo().get_testcase_run(&rid).await?;
    let story = ctx.product_repo().get_story(&run.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;
    ctx.product_repo()
        .reorder_testcases(&rid, &req.ordered_ids)
        .await?;
    // Return the updated list so the UI can reflect the persisted order.
    let cases = ctx.product_repo().list_testcases(&rid).await?;
    Ok(Json(cases).into_response())
}

// ---------------------------------------------------------------------------
// Inject bundle
// ---------------------------------------------------------------------------

async fn get_inject<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;
    let bundle = ctx.product().build_inject_bundle(&sid).await?;
    Ok(Json(bundle).into_response())
}

// ---------------------------------------------------------------------------
// Drafts
// ---------------------------------------------------------------------------

async fn create_draft<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(req): Json<NewDraftReq>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &ws, WorkspaceRole::Editor).await?;
    let detail = ctx
        .product()
        .create_draft(&ws, &user.id, req.title.as_deref())
        .await?;
    Ok(Json(detail).into_response())
}

async fn update_draft_body<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Json(req): Json<UpdateDraftReq>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    let detail = ctx
        .product()
        .update_draft_body(&sid, &req.title, &req.body_md, &user.id)
        .await?;
    Ok(Json(detail).into_response())
}

// ---------------------------------------------------------------------------
// Transcripts
// ---------------------------------------------------------------------------

#[derive(Default, Deserialize)]
struct TranscriptQuery {
    #[serde(default)]
    summary: bool,
    limit: Option<usize>,
    cursor: Option<String>,
    q: Option<String>,
    max_matches: Option<usize>,
}

async fn list_transcripts<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Query(query): Query<TranscriptQuery>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;
    if query.summary {
        let page = ctx
            .product_repo()
            .transcript_summaries(&sid, query.limit.unwrap_or(50), query.cursor.as_deref())
            .await?;
        return Ok(Json(page).into_response());
    }
    // Preserve the full-array contract for existing agent/context callers.
    Ok(Json(ctx.product_repo().list_transcripts(&sid).await?).into_response())
}

async fn search_transcripts<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Query(query): Query<TranscriptQuery>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;
    let page = ctx
        .product_repo()
        .search_transcripts(
            &sid,
            query.q.as_deref().unwrap_or_default(),
            query.limit.unwrap_or(100),
            query.max_matches.unwrap_or(5000),
            query.cursor.as_deref(),
        )
        .await?;
    Ok(Json(page).into_response())
}

async fn get_transcript<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(TranscriptItemId { trid }): Path<TranscriptItemId>,
) -> ApiResult<Response> {
    let sid = ctx.product_repo().transcript_story_id(&trid).await?;
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;
    Ok(Json(ctx.product_repo().get_transcript(&trid).await?).into_response())
}

async fn create_transcript<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Json(req): Json<NewTranscriptReq>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    let transcript = ctx
        .product_repo()
        .create_transcript(NewTranscript {
            story_id: sid.clone(),
            title: req.title.unwrap_or_default(),
            body: req.body,
            created_by: user.id.clone(),
        })
        .await?;
    Ok(Json(transcript).into_response())
}

async fn delete_transcript<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(TranscriptItemId { trid }): Path<TranscriptItemId>,
) -> ApiResult<StatusCode> {
    let transcript = ctx.product_repo().get_transcript(&trid).await?;
    let story = ctx.product_repo().get_story(&transcript.story_id).await?;
    ctx.roles()
        .check(&user, &story.workspace_id, WorkspaceRole::Editor)
        .await?;
    ctx.product_repo().delete_transcript(&trid).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Publish discovery
// ---------------------------------------------------------------------------

async fn publish_as_rfc<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Json(req): Json<PublishAsRfcReq>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    // S4: publish through a user-supplied account — owner/root only.
    ctx.product()
        .authorize_account_id(&req.account_id, &user)
        .await?;
    let detail = ctx.product().publish_as_rfc(&sid, &req, &user.id).await?;
    Ok(Json(detail).into_response())
}

async fn publish_as_story<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Json(req): Json<PublishAsStoryReq>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    // S4: publish through a user-supplied account — owner/root only.
    ctx.product()
        .authorize_account_id(&req.account_id, &user)
        .await?;
    let detail = ctx.product().publish_as_story(&sid, &req, &user.id).await?;
    Ok(Json(detail).into_response())
}

// ---------------------------------------------------------------------------
// Product↔Swarm closure  (GET /product/stories/{sid}/swarm)
// ---------------------------------------------------------------------------

/// `GET /product/stories/{sid}/swarm` — Aggregate view of the swarm project
/// that was created from this story's implementation plan (Plan → Swarm).
///
/// Read-only join across the swarm state; requires workspace Viewer. Assembles
/// tasks, runs, and accumulated cost from the linked project. Artifacts/PRs/
/// reviews are extracted best-effort from run `result_json` blobs. Returns an
/// empty `StorySwarmLink` (no project, empty collections) when no swarm project
/// is linked — never 404; the caller can distinguish by `project: null`.
async fn story_swarm_link<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<Json<StorySwarmLink>> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;

    let Some(swarm_repo) = ctx.swarm_repo() else {
        // Host application has no swarm layer; return empty link.
        return Ok(Json(StorySwarmLink {
            project: None,
            tasks: vec![],
            runs: vec![],
            artifacts: vec![],
            prs: vec![],
            reviews: vec![],
            cost_usd: 0.0,
        }));
    };

    // Look up the swarm project that was created from this story.
    let project = swarm_repo.project_for_story(&sid).await?;

    let Some(ref proj) = project else {
        return Ok(Json(StorySwarmLink {
            project: None,
            tasks: vec![],
            runs: vec![],
            artifacts: vec![],
            prs: vec![],
            reviews: vec![],
            cost_usd: 0.0,
        }));
    };

    let tasks = swarm_repo.list_tasks(&proj.id).await.unwrap_or_default();
    let runs = swarm_repo
        .list_runs(&RunFilter {
            project_id: Some(proj.id.clone()),
            ..Default::default()
        })
        .await
        .unwrap_or_default();

    // Accumulated cost (best-effort; cost_usd may be null/0 until usage attribution).
    let cost_usd: f64 = runs.iter().filter_map(|r| r.cost_usd).sum();

    // Best-effort extraction of artifacts/PRs/reviews from run result blobs.
    let mut artifacts: Vec<String> = Vec::new();
    let mut prs: Vec<String> = Vec::new();
    let mut reviews: Vec<String> = Vec::new();
    for run in &runs {
        if let Some(result) = &run.result {
            if let Some(arr) = result.get("artifacts").and_then(|v| v.as_array()) {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        artifacts.push(s.to_string());
                    }
                }
            }
            if let Some(arr) = result.get("prs").and_then(|v| v.as_array()) {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        prs.push(s.to_string());
                    }
                }
            }
            if let Some(arr) = result.get("reviews").and_then(|v| v.as_array()) {
                for v in arr {
                    if let Some(s) = v.as_str() {
                        reviews.push(s.to_string());
                    }
                }
            }
        }
    }

    Ok(Json(StorySwarmLink {
        project,
        tasks,
        runs,
        artifacts,
        prs,
        reviews,
        cost_usd,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use axum::{Extension, Router};
    use http_body_util::BodyExt;
    use otto_core::auth::{AuthUser, BoxFuture, RoleChecker};
    use otto_core::domain::{User, WorkspaceRole};
    use otto_core::secrets::SecretStore;
    use otto_core::{Id, Result};
    use otto_state::DbPool;
    use otto_state::{IssuesRepo, NewStory, ProductRepo};
    use tower::ServiceExt;

    use crate::service::ProductService;

    use super::{router, ProductCtx};

    // -----------------------------------------------------------------------
    // In-memory pool helper — mirrors otto-state's test setup
    // -----------------------------------------------------------------------

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        pool.into()
    }

    // Seed a minimal user row
    async fn seed_user(pool: &DbPool) -> Id {
        use chrono::Utc;
        let uid = otto_core::new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&uid)
        .bind("testuser")
        .bind("hash")
        .bind("Test User")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        uid
    }

    // Seed a minimal workspace row
    async fn seed_workspace(pool: &DbPool) -> Id {
        use chrono::Utc;
        let wid = otto_core::new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at)
             VALUES (?, ?, ?, ?)",
        )
        .bind(&wid)
        .bind("ws")
        .bind("/tmp")
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        wid
    }

    // -----------------------------------------------------------------------
    // Stub implementations
    // -----------------------------------------------------------------------

    /// A RoleChecker that always authorizes (admin).
    struct AllowAll;

    impl RoleChecker for AllowAll {
        fn check<'a>(
            &'a self,
            _user: &'a User,
            _workspace_id: &'a Id,
            _min: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async { Ok(()) })
        }
    }

    /// A SecretStore that always returns None.
    struct NoopSecrets;

    impl SecretStore for NoopSecrets {
        fn put(&self, _key: &str, _value: &str) -> Result<()> {
            Ok(())
        }
        fn get(&self, _key: &str) -> Result<Option<String>> {
            Ok(None)
        }
        fn delete(&self, _key: &str) -> Result<()> {
            Ok(())
        }
    }

    /// A fake user with a stable ID used by every test request.
    fn test_user(id: &Id) -> User {
        use chrono::Utc;
        User {
            id: id.clone(),
            username: "tester".into(),
            display_name: "Tester".into(),
            is_root: true,
            disabled: false,
            created_at: Utc::now(),
        }
    }

    // -----------------------------------------------------------------------
    // TestCtx
    // -----------------------------------------------------------------------

    #[derive(Clone)]
    struct TestCtx {
        repo: ProductRepo,
        #[allow(dead_code)]
        issues: IssuesRepo,
        svc: Arc<ProductService>,
        roles: Arc<dyn RoleChecker>,
    }

    impl TestCtx {
        fn new(pool: impl Into<DbPool>) -> Self {
            let pool: DbPool = pool.into();
            let repo = ProductRepo::new(pool.clone());
            let issues = IssuesRepo::new(pool.clone());
            let secrets: Arc<dyn SecretStore> = Arc::new(NoopSecrets);
            let svc = Arc::new(ProductService::new(repo.clone(), issues.clone(), secrets));
            let roles: Arc<dyn RoleChecker> = Arc::new(AllowAll);
            Self {
                repo,
                issues,
                svc,
                roles,
            }
        }
    }

    impl ProductCtx for TestCtx {
        fn product(&self) -> &Arc<ProductService> {
            &self.svc
        }
        fn product_repo(&self) -> &ProductRepo {
            &self.repo
        }
        fn roles(&self) -> &Arc<dyn RoleChecker> {
            &self.roles
        }
    }

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn app(ctx: TestCtx, user_id: &Id) -> Router {
        let user = test_user(user_id);
        router::<TestCtx>()
            .with_state(ctx)
            .layer(Extension(AuthUser(user)))
    }

    /// Build the app with an explicit (possibly non-root) user, so S4 credential
    /// ownership can be exercised at the HTTP boundary.
    fn app_as(ctx: TestCtx, user_id: &Id, is_root: bool) -> Router {
        use chrono::Utc;
        let user = User {
            id: user_id.clone(),
            username: "tester".into(),
            display_name: "Tester".into(),
            is_root,
            disabled: false,
            created_at: Utc::now(),
        };
        router::<TestCtx>()
            .with_state(ctx)
            .layer(Extension(AuthUser(user)))
    }

    /// Seed a user row with a specific id + username (distinct usernames avoid the
    /// UNIQUE collision `seed_user` hits when called twice).
    async fn seed_named_user(pool: &DbPool, username: &str) -> Id {
        use chrono::Utc;
        let uid = otto_core::new_id();
        let now = Utc::now().to_rfc3339();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, ?, ?, ?, 0, ?)",
        )
        .bind(&uid)
        .bind(username)
        .bind("hash")
        .bind(username)
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        uid
    }

    /// Seed an issue account owned by `user_id`.
    async fn seed_issue_account(pool: &DbPool, user_id: &Id) -> Id {
        use otto_core::domain::IssueProviderKind;
        let repo = IssuesRepo::new(pool.clone());
        repo.create_account(otto_state::NewIssueAccount {
            user_id: user_id.clone(),
            provider: IssueProviderKind::Jira,
            label: "work".into(),
            email: "owner@example.com".into(),
            token_ref: "issueacct-1".into(),
            base_url: "https://example.atlassian.net".into(),
            token_expires_at: None,
        })
        .await
        .unwrap()
        .id
    }

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    struct PublishTestSecret;
    impl SecretStore for PublishTestSecret {
        fn put(&self, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        fn get(&self, _: &str) -> Result<Option<String>> {
            Ok(Some("isolated-test-token".into()))
        }
        fn delete(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }

    struct TranscriptFixture {
        pool: DbPool,
        ctx: TestCtx,
        user: Id,
        workspace: Id,
        story: Id,
        rows: Vec<otto_state::ProductTranscript>,
    }

    async fn transcript_fixture(count: usize, body_bytes: usize) -> TranscriptFixture {
        let pool = mem_pool().await;
        let user = seed_user(&pool).await;
        let workspace = seed_workspace(&pool).await;
        let ctx = TestCtx::new(pool.clone());
        let story = ctx
            .svc
            .create_draft(&workspace, &user, Some("Transcript fixture"))
            .await
            .unwrap()
            .story
            .id;
        let mut rows = Vec::new();
        for index in 0..count {
            let row = ctx
                .repo
                .create_transcript(otto_state::NewTranscript {
                    story_id: story.clone(),
                    title: format!("Transcript {index}"),
                    body: format!("{}\nbody-{index}", "x".repeat(body_bytes)),
                    created_by: user.clone(),
                })
                .await
                .unwrap();
            // Equal timestamps deliberately exercise the stable ID tie-breaker.
            sqlx::query("UPDATE product_transcripts SET created_at = ? WHERE id = ?")
                .bind("2026-01-01T00:00:00+00:00")
                .bind(&row.id)
                .execute(&pool)
                .await
                .unwrap();
            rows.push(row);
        }
        rows.sort_by(|a, b| b.id.cmp(&a.id));
        TranscriptFixture {
            pool,
            ctx,
            user,
            workspace,
            story,
            rows,
        }
    }

    #[tokio::test]
    async fn transcript_summary_page_excludes_100_large_bodies() {
        let fixture = transcript_fixture(100, 256 * 1024).await;
        let response = app(fixture.ctx, &fixture.user)
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/product/stories/{}/transcripts?summary=true&limit=25",
                        fixture.story
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(
            bytes.len() < 64 * 1024,
            "summary endpoint transferred {} bytes",
            bytes.len()
        );
        let page: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let items = page["items"].as_array().expect("summary envelope");
        assert_eq!(items.len(), 25);
        assert!(items.iter().all(|item| item.get("body").is_none()));
        assert!(items
            .iter()
            .all(|item| item["body_bytes"].as_u64().unwrap() >= 256 * 1024));
        assert!(page["next_cursor"].is_string());
    }

    #[tokio::test]
    async fn transcript_legacy_list_keeps_full_body_compatibility() {
        let fixture = transcript_fixture(1, 32).await;
        let response = app(fixture.ctx, &fixture.user)
            .oneshot(
                Request::builder()
                    .uri(format!("/product/stories/{}/transcripts", fixture.story))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let rows = body_json(response).await;
        assert_eq!(rows.as_array().unwrap().len(), 1);
        assert_eq!(rows[0]["body"], fixture.rows[0].body);
    }

    #[tokio::test]
    async fn transcript_summary_cursor_survives_timestamp_ties_and_new_imports() {
        let fixture = transcript_fixture(8, 32).await;
        let routes = app(fixture.ctx.clone(), &fixture.user);
        let mut cursor = String::new();
        let mut seen = Vec::new();
        for page_number in 0..3 {
            let response = routes
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!(
                            "/product/stories/{}/transcripts?summary=true&limit=3{cursor}",
                            fixture.story
                        ))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let page = body_json(response).await;
            let items = page["items"].as_array().expect("bounded summary envelope");
            assert!(items.len() <= 3);
            seen.extend(
                items
                    .iter()
                    .map(|item| item["id"].as_str().unwrap().to_string()),
            );
            if page_number == 0 {
                fixture
                    .ctx
                    .repo
                    .create_transcript(otto_state::NewTranscript {
                        story_id: fixture.story.clone(),
                        title: "Imported after page one".into(),
                        body: "Newer body".into(),
                        created_by: fixture.user.clone(),
                    })
                    .await
                    .unwrap();
            }
            cursor = page["next_cursor"]
                .as_str()
                .map(|value| format!("&cursor={value}"))
                .unwrap_or_default();
        }
        assert_eq!(
            seen,
            fixture
                .rows
                .iter()
                .map(|row| row.id.clone())
                .collect::<Vec<_>>()
        );
        assert!(cursor.is_empty());
        let response = routes
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/product/stories/{}/transcripts?summary=true&cursor=not-a-cursor",
                        fixture.story
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn transcript_search_reaches_old_collapsed_body_with_literal_unicode_counts() {
        let fixture = transcript_fixture(105, 16).await;
        let target = fixture.rows.last().unwrap();
        let body = "First CAFÉ.[x] then café.[x], plus unrelated caféx.";
        // This is an old row beyond the first two 50-summary pages.
        sqlx::query("UPDATE product_transcripts SET body = ? WHERE id = ?")
            .bind(body)
            .bind(&target.id)
            .execute(&fixture.pool)
            .await
            .unwrap();
        let routes = app(fixture.ctx.clone(), &fixture.user);
        let mut cursor = String::new();
        let mut found = Vec::new();
        for _ in 0..120 {
            let response = routes.clone().oneshot(Request::builder()
                .uri(format!("/product/stories/{}/transcripts/search?q=caf%C3%A9.%5Bx%5D&limit=7&max_matches=5000{cursor}", fixture.story))
                .body(Body::empty()).unwrap()).await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let page = body_json(response).await;
            let items = page["items"].as_array().expect("bounded search envelope");
            assert!(items.len() <= 7);
            assert!(items.iter().all(|item| item.get("body").is_none()));
            found.extend(items.iter().cloned());
            let Some(next) = page["next_cursor"].as_str() else {
                break;
            };
            cursor = format!("&cursor={next}");
        }
        assert_eq!(found.len(), 1);
        assert_eq!(found[0]["id"], target.id);
        assert_eq!(
            found[0]["match_count"], 2,
            "literal, case-insensitive, non-overlapping occurrences"
        );
        let response = routes
            .oneshot(
                Request::builder()
                    .uri(format!("/product/transcripts/{}", target.id))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["body"], body);
    }

    struct TranscriptRoleCheck {
        expected_workspace: Id,
        observed: Arc<std::sync::Mutex<usize>>,
    }
    impl RoleChecker for TranscriptRoleCheck {
        fn check<'a>(
            &'a self,
            _: &'a User,
            workspace: &'a Id,
            role: WorkspaceRole,
        ) -> BoxFuture<'a, Result<()>> {
            Box::pin(async move {
                assert_eq!(workspace, &self.expected_workspace);
                assert_eq!(role, WorkspaceRole::Viewer);
                *self.observed.lock().unwrap() += 1;
                Err(otto_core::Error::Forbidden(
                    "no access to transcript workspace".into(),
                ))
            })
        }
    }

    #[tokio::test]
    async fn transcript_summary_search_and_detail_authorize_owning_story() {
        let mut fixture = transcript_fixture(1, 32).await;
        let observed = Arc::new(std::sync::Mutex::new(0));
        fixture.ctx.roles = Arc::new(TranscriptRoleCheck {
            expected_workspace: fixture.workspace,
            observed: observed.clone(),
        });
        let routes = app(fixture.ctx, &fixture.user);
        for uri in [
            format!(
                "/product/stories/{}/transcripts?summary=true",
                fixture.story
            ),
            format!(
                "/product/stories/{}/transcripts/search?q=body",
                fixture.story
            ),
            format!("/product/transcripts/{}", fixture.rows[0].id),
        ] {
            let response = routes
                .clone()
                .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
        }
        assert_eq!(*observed.lock().unwrap(), 3);
    }

    /// Exercise the real preview HTTP reads, request deserialization, auth,
    /// service, and outbound clients. Every account points only at wiremock.
    async fn reviewed_publish_case(mode: &str, mutation: &str) {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let remote = MockServer::start().await;
        let outbound_path = if mode == "story" {
            "/rest/api/3/issue"
        } else {
            "/wiki/rest/api/content"
        };
        Mock::given(method("POST"))
            .and(path(outbound_path))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "101", "key": "TEST-1", "title": "Reviewed title",
                "self": format!("{}/rest/api/3/issue/101", remote.uri()),
                "space": {"key": "TEST"}, "version": {"number": 1},
                "_links": {"webui": "/spaces/TEST/pages/101"}
            })))
            .mount(&remote)
            .await;
        let pool = mem_pool().await;
        let user = seed_user(&pool).await;
        let ws = seed_workspace(&pool).await;
        let mut ctx = TestCtx::new(pool.clone());
        ctx.svc = Arc::new(ProductService::new(
            ctx.repo.clone(),
            ctx.issues.clone(),
            Arc::new(PublishTestSecret),
        ));
        let account = ctx
            .issues
            .create_account(otto_state::NewIssueAccount {
                user_id: user.clone(),
                provider: otto_core::domain::IssueProviderKind::Jira,
                label: "Isolated publish capture".into(),
                email: "test@example.invalid".into(),
                token_ref: "publish-test".into(),
                base_url: remote.uri(),
                token_expires_at: None,
            })
            .await
            .unwrap();
        let created = ctx
            .svc
            .create_draft(&ws, &user, Some("Reviewed title"))
            .await
            .unwrap();
        let sid = created.story.id;
        let original_body = "Reviewed body\nSecond line with café.";
        let saved = ctx
            .svc
            .update_draft_body(&sid, "Reviewed title", original_body, &user)
            .await
            .unwrap();
        let vid = saved.source.unwrap().id;
        let reference = format!("{}/wiki/spaces/TEST/pages/100", remote.uri());
        ctx.repo
            .update_story(
                &sid,
                otto_state::StoryPatch {
                    source_kind: Some("confluence".into()),
                    source_key: Some("100".into()),
                    account_id: Some(account.id.clone()),
                    url: Some(reference.clone()),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let routes = app(ctx.clone(), &user);
        let preview_response = routes
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/product/versions/{vid}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(preview_response.status(), StatusCode::OK);
        let preview = body_json(preview_response).await;
        assert_eq!(preview["body_md"], original_body);
        let reviewed = serde_json::json!({
            "version_id": vid, "body_sha256": otto_core::proof::content_sha256(preview["body_md"].as_str().unwrap()),
            "title": "Reviewed title", "source_kind": "confluence", "url": reference,
        });
        match mutation {
            "same_version_body" => {
                ctx.svc
                    .update_draft_body(&sid, "Reviewed title", "UNREVIEWED same-ID body", &user)
                    .await
                    .unwrap();
                assert_eq!(ctx.repo.list_versions(&sid).await.unwrap()[0].id, vid);
            }
            "new_version" => {
                ctx.repo
                    .add_version(otto_state::NewVersion {
                        story_id: sid.clone(),
                        kind: "suggested".into(),
                        title: "Reviewed title".into(),
                        body_md: "UNREVIEWED newer version".into(),
                        raw_json: None,
                        change_notes: None,
                        created_by: user.clone(),
                    })
                    .await
                    .unwrap();
            }
            "title" | "reference" => {
                ctx.repo
                    .update_story(
                        &sid,
                        otto_state::StoryPatch {
                            title: (mutation == "title").then(|| "UNREVIEWED title".into()),
                            url: (mutation == "reference")
                                .then(|| format!("{}/UNREVIEWED-reference", remote.uri())),
                            ..Default::default()
                        },
                    )
                    .await
                    .unwrap();
            }
            "unchanged" | "missing_review" => {}
            _ => panic!("unknown test mutation"),
        }
        let mut payload = serde_json::json!({
            "account_id": account.id, "project_key": "TEST", "issue_type": "Story",
            "space_key": "TEST", "parent_id": "42", "reviewed_content": reviewed,
        });
        if mutation == "missing_review" {
            payload.as_object_mut().unwrap().remove("reviewed_content");
        }
        let response = routes
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/product/stories/{sid}/publish-as-{mode}"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let detail = body_json(response).await;
        let requests = remote.received_requests().await.unwrap();
        let publications: Vec<_> = requests
            .iter()
            .filter(|request| request.url.path() == outbound_path)
            .collect();
        if mutation != "unchanged" {
            assert_eq!(status, StatusCode::CONFLICT,
                "{mode}/{mutation}: stale review must require rereview; sent {} outbound publications; response {detail}", publications.len());
            assert!(requests.is_empty(), "reject before any outbound request");
            return;
        }
        assert_eq!(status, StatusCode::OK, "{detail}");
        assert_eq!(publications.len(), 1);
        let sent: serde_json::Value = serde_json::from_slice(&publications[0].body).unwrap();
        if mode == "story" {
            assert_eq!(sent["fields"]["summary"], "Reviewed title");
            assert_eq!(
                sent["fields"]["description"],
                otto_issues::adf::text_to_adf(&format!("> RFC: {reference}\n\n{original_body}"))
            );
        } else {
            assert_eq!(sent["title"], "Reviewed title");
            assert_eq!(
                sent["body"]["storage"]["value"],
                otto_issues::confluence::markdown_to_storage(original_body)
            );
            assert_eq!(sent["ancestors"][0]["id"], "42");
        }
    }

    macro_rules! reviewed_publish_test {
        ($name:ident, $mode:literal, $mutation:literal) => {
            #[tokio::test]
            async fn $name() {
                reviewed_publish_case($mode, $mutation).await;
            }
        };
    }
    reviewed_publish_test!(
        reviewed_jira_same_version_body,
        "story",
        "same_version_body"
    );
    reviewed_publish_test!(reviewed_jira_new_version, "story", "new_version");
    reviewed_publish_test!(reviewed_jira_title, "story", "title");
    reviewed_publish_test!(reviewed_jira_reference, "story", "reference");
    reviewed_publish_test!(reviewed_jira_missing_review, "story", "missing_review");
    reviewed_publish_test!(reviewed_jira_unchanged_payload, "story", "unchanged");
    reviewed_publish_test!(
        reviewed_confluence_same_version_body,
        "rfc",
        "same_version_body"
    );
    reviewed_publish_test!(reviewed_confluence_new_version, "rfc", "new_version");
    reviewed_publish_test!(reviewed_confluence_title, "rfc", "title");
    reviewed_publish_test!(reviewed_confluence_reference, "rfc", "reference");
    reviewed_publish_test!(reviewed_confluence_missing_review, "rfc", "missing_review");
    reviewed_publish_test!(reviewed_confluence_unchanged_payload, "rfc", "unchanged");

    // -----------------------------------------------------------------------
    // Tests — collection routes (unchanged paths)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn list_stories_empty() {
        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws = seed_workspace(&pool).await;
        let ctx = TestCtx::new(pool);
        let a = app(ctx, &user_id);

        let req = Request::builder()
            .method(Method::GET)
            .uri(format!("/workspaces/{ws}/product/stories"))
            .body(Body::empty())
            .unwrap();
        let resp = a.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body, serde_json::json!([]));
    }

    #[tokio::test]
    async fn create_and_list_learning() {
        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws = seed_workspace(&pool).await;
        let ctx = TestCtx::new(pool);
        let a = app(ctx, &user_id);

        let body = serde_json::json!({
            "kind": "pattern",
            "title": "Use strong types",
            "body": "Always wrap primitives in newtype structs"
        });

        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/workspaces/{ws}/product/learnings"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = a.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let created = body_json(resp).await;
        assert_eq!(created["kind"], "pattern");
        assert_eq!(created["title"], "Use strong types");

        // Now list and check it's there
        let req2 = Request::builder()
            .method(Method::GET)
            .uri(format!("/workspaces/{ws}/product/learnings"))
            .body(Body::empty())
            .unwrap();
        let resp2 = a.oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        let list = body_json(resp2).await;
        assert_eq!(list.as_array().unwrap().len(), 1);
    }

    // -----------------------------------------------------------------------
    // Tests — flat item routes
    // -----------------------------------------------------------------------

    /// Create a learning via the collection POST, then PATCH it via the flat
    /// `/product/learnings/{lid}` route. Exercises the workspace-from-row
    /// role-check path for learnings.
    #[tokio::test]
    async fn flat_patch_learning() {
        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws = seed_workspace(&pool).await;
        let ctx = TestCtx::new(pool);
        let a = app(ctx, &user_id);

        // 1. Create via collection POST
        let create_body = serde_json::json!({
            "kind": "anti-pattern",
            "title": "Global mutable state",
            "body": "Avoid shared mutable state"
        });
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/workspaces/{ws}/product/learnings"))
            .header("content-type", "application/json")
            .body(Body::from(create_body.to_string()))
            .unwrap();
        let resp = a.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let created = body_json(resp).await;
        let lid = created["id"].as_str().expect("id field").to_string();

        // 2. PATCH via flat route — workspace resolved from row
        let patch_body = serde_json::json!({"title": "Avoid global mutable state"});
        let req2 = Request::builder()
            .method(Method::PATCH)
            .uri(format!("/product/learnings/{lid}"))
            .header("content-type", "application/json")
            .body(Body::from(patch_body.to_string()))
            .unwrap();
        let resp2 = a.clone().oneshot(req2).await.unwrap();
        assert_eq!(resp2.status(), StatusCode::OK);
        let updated = body_json(resp2).await;
        assert_eq!(updated["title"], "Avoid global mutable state");
        assert_eq!(updated["kind"], "anti-pattern");
    }

    /// Insert a story directly via the repo, then `GET /product/stories/{sid}`
    /// and verify the response is a `ProductStoryDetail` with the correct shape.
    /// Exercises the workspace-from-row role-check path for stories.
    #[tokio::test]
    async fn get_story_detail_flat() {
        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws = seed_workspace(&pool).await;
        let ctx = TestCtx::new(pool);

        // Insert story via repo directly
        let story = ctx
            .repo
            .create_story(NewStory {
                workspace_id: ws.clone(),
                source_kind: "jira".into(),
                account_id: user_id.clone(),
                source_key: "PROJ-1".into(),
                title: "My Feature".into(),
                url: "https://jira.example.com/PROJ-1".into(),
                issue_type: None,
                stage: "draft".into(),
                cwd: None,
                parent_id: None,
                tree_kind: "story".into(),
                folder: String::new(),
                created_by: user_id.clone(),
            })
            .await
            .unwrap();

        let sid = story.id.clone();
        let a = app(ctx, &user_id);

        let req = Request::builder()
            .method(Method::GET)
            .uri(format!("/product/stories/{sid}"))
            .body(Body::empty())
            .unwrap();
        let resp = a.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        // Response must have the ProductStoryDetail shape
        assert_eq!(body["story"]["id"], sid.as_str());
        assert_eq!(body["story"]["title"], "My Feature");
        // counts block must be present
        assert!(body["counts"]["versions"].is_number());
        assert!(body["counts"]["analyses"].is_number());
        assert!(body["counts"]["open_questions"].is_number());
        assert!(body["counts"]["notes"].is_number());
        assert!(body["counts"]["testcases"].is_number());
    }

    // -----------------------------------------------------------------------
    // S4: credential ownership at the HTTP boundary
    // -----------------------------------------------------------------------

    /// `POST /product/stories/{sid}/publish-as-story` with an `account_id` the
    /// caller does not own must be rejected with 403 *before* any network call —
    /// even though the workspace RoleChecker (AllowAll) would let them through.
    #[tokio::test]
    async fn publish_as_story_rejects_non_owner_account() {
        let pool = mem_pool().await;
        let owner = seed_named_user(&pool, "owner").await;
        let attacker = seed_named_user(&pool, "attacker").await;
        let ws = seed_workspace(&pool).await;
        // Account belongs to `owner`.
        let account_id = seed_issue_account(&pool, &owner).await;

        let ctx = TestCtx::new(pool);
        // A draft story to publish.
        let draft = ctx
            .repo
            .create_story(NewStory {
                workspace_id: ws.clone(),
                source_kind: "draft".into(),
                account_id: String::new(),
                source_key: String::new(),
                title: "Draft".into(),
                url: String::new(),
                issue_type: None,
                stage: "draft".into(),
                cwd: None,
                parent_id: None,
                tree_kind: "story".into(),
                folder: String::new(),
                created_by: attacker.clone(),
            })
            .await
            .unwrap();
        let sid = draft.id.clone();

        let body = serde_json::json!({
            "account_id": account_id,
            "project_key": "NEW",
            "issue_type": "Story",
        });

        // Attacker (non-root, not the account owner) → 403 Forbidden.
        let attacker_app = app_as(ctx.clone(), &attacker, false);
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/product/stories/{sid}/publish-as-story"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = attacker_app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "non-owner must be forbidden before using the credential"
        );

        // Owner passes the ownership guard (then fails later at the network step,
        // which is NOT a 403) — proving the guard does not block the legit owner.
        let owner_app = app_as(ctx, &owner, false);
        let req = Request::builder()
            .method(Method::POST)
            .uri(format!("/product/stories/{sid}/publish-as-story"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let resp = owner_app.oneshot(req).await.unwrap();
        assert_ne!(
            resp.status(),
            StatusCode::FORBIDDEN,
            "owner must clear the ownership guard"
        );
    }
    // -----------------------------------------------------------------------
    // Epic tree routes
    // -----------------------------------------------------------------------

    async fn seed_draft(ctx: &TestCtx, ws: &Id, user_id: &Id, title: &str) -> Id {
        ctx.repo
            .create_story(NewStory {
                workspace_id: ws.clone(),
                source_kind: "draft".into(),
                account_id: String::new(),
                source_key: String::new(),
                title: title.into(),
                url: String::new(),
                issue_type: None,
                stage: "draft".into(),
                cwd: None,
                parent_id: None,
                tree_kind: "story".into(),
                folder: String::new(),
                created_by: user_id.clone(),
            })
            .await
            .unwrap()
            .id
    }

    fn json_req(method: Method, uri: String, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn children_route_files_a_doc_under_the_epic() {
        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws = seed_workspace(&pool).await;
        let ctx = TestCtx::new(pool);
        let epic = seed_draft(&ctx, &ws, &user_id, "Loyalty").await;
        let a = app(ctx.clone(), &user_id);

        let resp = a
            .clone()
            .oneshot(json_req(
                Method::POST,
                format!("/product/stories/{epic}/children"),
                serde_json::json!({ "title": "Tier ladder", "folder": "Design" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = body_json(resp).await;
        assert_eq!(body["story"]["parent_id"], epic.as_str());
        assert_eq!(body["story"]["tree_kind"], "doc");
        assert_eq!(body["story"]["folder"], "Design");
        assert_eq!(body["story"]["workspace_id"], ws.as_str());
        let child: Id = body["story"]["id"].as_str().unwrap().to_string();

        // A child of a child is rejected (one level), as is an `epic` child.
        let resp = a
            .clone()
            .oneshot(json_req(
                Method::POST,
                format!("/product/stories/{child}/children"),
                serde_json::json!({ "title": "too deep" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp = a
            .oneshot(json_req(
                Method::POST,
                format!("/product/stories/{epic}/children"),
                serde_json::json!({ "title": "x", "tree_kind": "epic" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn patch_moves_marks_and_detaches_with_one_level_guard() {
        let pool = mem_pool().await;
        let user_id = seed_user(&pool).await;
        let ws = seed_workspace(&pool).await;
        let ctx = TestCtx::new(pool);
        let epic = seed_draft(&ctx, &ws, &user_id, "Epic").await;
        let s1 = seed_draft(&ctx, &ws, &user_id, "S1").await;
        let s2 = seed_draft(&ctx, &ws, &user_id, "S2").await;
        let a = app(ctx.clone(), &user_id);

        // Mark as epic.
        let resp = a
            .clone()
            .oneshot(json_req(
                Method::PATCH,
                format!("/product/stories/{epic}"),
                serde_json::json!({ "tree_kind": "epic" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_json(resp).await["tree_kind"], "epic");
        // Unknown tree_kind → 400.
        let resp = a
            .clone()
            .oneshot(json_req(
                Method::PATCH,
                format!("/product/stories/{epic}"),
                serde_json::json!({ "tree_kind": "saga" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // Move s1 under the epic in folder PO.
        let resp = a
            .clone()
            .oneshot(json_req(
                Method::PATCH,
                format!("/product/stories/{s1}"),
                serde_json::json!({ "parent_id": epic, "folder": "PO" }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let b = body_json(resp).await;
        assert_eq!(b["parent_id"], epic.as_str());
        assert_eq!(b["folder"], "PO");

        // s2 under s1 (a child) → 400; epic (has children) under s2 → 400.
        let resp = a
            .clone()
            .oneshot(json_req(
                Method::PATCH,
                format!("/product/stories/{s2}"),
                serde_json::json!({ "parent_id": s1 }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let resp = a
            .clone()
            .oneshot(json_req(
                Method::PATCH,
                format!("/product/stories/{epic}"),
                serde_json::json!({ "parent_id": s2 }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        // Self-parent → 400.
        let resp = a
            .clone()
            .oneshot(json_req(
                Method::PATCH,
                format!("/product/stories/{s2}"),
                serde_json::json!({ "parent_id": s2 }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

        // Absent key leaves the parent alone; explicit null detaches.
        let resp = a
            .clone()
            .oneshot(json_req(
                Method::PATCH,
                format!("/product/stories/{s1}"),
                serde_json::json!({ "folder": "Design" }),
            ))
            .await
            .unwrap();
        assert_eq!(body_json(resp).await["parent_id"], epic.as_str());
        let resp = a
            .oneshot(json_req(
                Method::PATCH,
                format!("/product/stories/{s1}"),
                serde_json::json!({ "parent_id": null }),
            ))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_json(resp).await["parent_id"].is_null());
    }
}
