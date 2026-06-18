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
use axum::routing::{delete, get, patch, post};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use otto_state::{
    LearningPatch, NewEvent, NewLearning, NewNote, NewQuestion, NewTranscript, ProductRepo,
    QuestionPatch, StoryPatch, TestcasePatch,
};
use serde::Deserialize;

use crate::service::ProductService;
use crate::types::{
    NewDraftReq, NewLearningReq, NewNoteReq, NewQuestionReq, NewTranscriptReq, PostQuestionsReq,
    PublishAsRfcReq, PublishAsStoryReq, PublishTestsReq, UpdateDraftReq, UpdateLearningReq,
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
}

// ---------------------------------------------------------------------------
// Error → response
// ---------------------------------------------------------------------------

pub(crate) struct ApiErr(pub Error);

impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        ApiErr(e)
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
        let problem = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(problem)).into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiErr>;

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
        // Versions (under-story collection + flat version item)
        .route("/product/stories/{sid}/versions", get(list_versions::<S>))
        .route("/product/versions/{vid}", get(get_version::<S>))
        .route("/product/versions/{vid}/publish", post(publish_version::<S>))
        // Analyses (under-story collection + flat analysis item)
        .route("/product/stories/{sid}/analyses", get(list_analyses::<S>))
        .route("/product/analyses/{aid}", get(get_analysis::<S>))
        // Questions (under-story collection + flat question item)
        .route(
            "/product/stories/{sid}/questions",
            get(list_questions::<S>).post(create_question::<S>),
        )
        .route("/product/stories/{sid}/questions/post", post(post_questions::<S>))
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
        .route("/product/stories/{sid}/testcases", get(list_testcase_runs::<S>))
        .route("/product/testcases/{tid}", patch(update_testcase::<S>))
        // NOTE: /product/testcase-runs/{rid}/approve is registered in otto-server
        // (modules.rs) so it can trigger skill self-improvement. Registering it
        // here too would cause an axum duplicate-route panic at startup.
        .route("/product/testcase-runs/{rid}/publish", post(publish_tests::<S>))
        // Inject bundle
        .route("/product/stories/{sid}/inject", get(get_inject::<S>))
        // Learnings (flat item)
        .route(
            "/product/learnings/{lid}",
            patch(update_learning::<S>).delete(delete_learning::<S>),
        )
        .route("/product/learnings/{lid}/accept", post(accept_learning::<S>))
        // Drafts
        .route(
            "/workspaces/{ws}/product/drafts",
            post(create_draft::<S>),
        )
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
            "/product/transcripts/{trid}",
            delete(delete_transcript::<S>),
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
    let stories = ctx.product_repo().list_stories(&ws).await?;
    Ok(Json(stories).into_response())
}

async fn import_story<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(WsPath { ws }): Path<WsPath>,
    Json(req): Json<crate::types::ImportStoryReq>,
) -> ApiResult<Response> {
    ctx.roles().check(&user, &ws, WorkspaceRole::Editor).await?;
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
    let source = ctx.product_repo().latest_source_version(&sid).await?;
    let versions = ctx.product_repo().list_versions(&sid).await?;
    let analyses = ctx.product_repo().list_analyses(&sid).await?;
    let questions = ctx.product_repo().list_questions(&sid).await?;
    let notes = ctx.product_repo().list_notes(&sid).await?;
    let runs = ctx.product_repo().list_testcase_runs(&sid).await?;
    let open_questions = questions.iter().filter(|q| q.status == "open").count() as i64;
    let testcases: i64 = {
        let mut tc_count: i64 = 0;
        for run in &runs {
            let tcs = ctx.product_repo().list_testcases(&run.id).await?;
            tc_count += tcs.len() as i64;
        }
        tc_count
    };
    let detail = crate::types::ProductStoryDetail {
        story,
        source,
        counts: crate::types::StoryCounts {
            versions: versions.len() as i64,
            analyses: analyses.len() as i64,
            open_questions,
            notes: notes.len() as i64,
            testcases,
        },
    };
    Ok(Json(detail).into_response())
}

async fn patch_story<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Json(req): Json<UpdateStoryReq>,
) -> ApiResult<Response> {
    let ws = ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    let updated = ctx
        .product_repo()
        .update_story(
            &sid,
            StoryPatch {
                title: None,
                url: None,
                issue_type: None,
                stage: req.stage,
                cwd: req.cwd.map(Some),
                watch_enabled: req.watch_enabled,
                watch_cadence_min: req.watch_cadence_min,
                confluence_tests_page_id: None,
                confluence_tests_url: None,
                tags: req.tags,
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
    ctx.product_repo().delete_story(&sid).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn refresh_story<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
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
    let learnings = ctx.product_repo().list_learnings(&ws, active_only).await?;
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

async fn list_transcripts<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Viewer).await?;
    let transcripts = ctx.product_repo().list_transcripts(&sid).await?;
    Ok(Json(transcripts).into_response())
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
    let detail = ctx
        .product()
        .publish_as_rfc(
            &sid,
            &req.account_id,
            &req.space_key,
            req.parent_id.as_deref(),
            req.title.as_deref(),
            &user.id,
        )
        .await?;
    Ok(Json(detail).into_response())
}

async fn publish_as_story<S: ProductCtx>(
    State(ctx): State<S>,
    Extension(AuthUser(user)): Extension<AuthUser>,
    Path(StoryId { sid }): Path<StoryId>,
    Json(req): Json<PublishAsStoryReq>,
) -> ApiResult<Response> {
    ws_from_story(&ctx, &user, &sid, WorkspaceRole::Editor).await?;
    let detail = ctx
        .product()
        .publish_as_story(&sid, &req.account_id, &req.project_key, &req.issue_type, &user.id)
        .await?;
    Ok(Json(detail).into_response())
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
    use otto_state::{IssuesRepo, NewStory, ProductRepo};
    use sqlx::SqlitePool;
    use tower::ServiceExt;

    use crate::service::ProductService;

    use super::{router, ProductCtx};

    // -----------------------------------------------------------------------
    // In-memory pool helper — mirrors otto-state's test setup
    // -----------------------------------------------------------------------

    async fn mem_pool() -> SqlitePool {
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
        pool
    }

    // Seed a minimal user row
    async fn seed_user(pool: &SqlitePool) -> Id {
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
    async fn seed_workspace(pool: &SqlitePool) -> Id {
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
        fn new(pool: SqlitePool) -> Self {
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

    async fn body_json(resp: axum::response::Response) -> serde_json::Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

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
}
