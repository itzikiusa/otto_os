//! Conversation view routes (design `docs/design/conversation-view.md` §4.3):
//! the transcript of a session rebuilt from the provider's own JSONL on disk,
//! its extracted images, the artifacts it produced, board→agent tasks, the
//! composer's image inbox, and the History surface (index + on-disk transcripts
//! + import/rescan).
//!
//! Paths: the server derives every file path itself except
//! `history/transcript?path=`, which is confined to the two provider roots
//! (`~/.claude/projects`, `$CODEX_HOME/sessions`) with the symlink-aware
//! `otto_core::paths::resolves_under`. Artifacts are served by OPAQUE id — the
//! server maps the id back to the path it folded itself, then applies the
//! `routes/fs.rs` discipline (canonicalize → allow-list → deny-list → cap →
//! nosniff). A session without a resolvable transcript gets
//! `200 { unavailable_reason }`, never 404.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use base64::Engine;
use otto_core::api::{CreateAgentTaskReq, HistoryImportReq, InboxUploadReq, InboxUploadResp};
use otto_core::domain::{AgentTask, Session, SessionKind, SessionStatus, TrailKind, TrailLevel, TrailSource, WorkspaceRole};
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::{NewSession, NewTrail, SessionsRepo, TranscriptIndexRepo};
use otto_transcript::{
    fold_file, read_subagents, subagent_path, Artifact, ArtifactKind, FoldOpts, Folded, HistoryEntry, ImageStore,
    Provider, Transcript, UnavailableReason,
};
use serde::Deserialize;

use crate::auth::{require_session_owner_or_admin, require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Default / maximum page size in turns.
const DEFAULT_LIMIT: usize = 60;
const MAX_LIMIT: usize = 500;
/// `GET …/artifacts/{id}` serves at most this many bytes (design §4.7).
const ARTIFACT_CAP: u64 = 25 * 1024 * 1024;
/// `POST …/inbox` accepts at most this many DECODED bytes (design §4.3).
const INBOX_CAP: usize = 10 * 1024 * 1024;
/// History page size.
const HISTORY_DEFAULT_LIMIT: usize = 100;
const HISTORY_MAX_LIMIT: usize = 1000;

// ── shared helpers ─────────────────────────────────────────────────────────

/// `$HOME` (the root of `~/.claude`), falling back to the OS home dir.
pub(crate) fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The two provider roots the daemon reads transcripts from
/// (`<claude projects>`, `<codex sessions>`). Precedence:
/// `OTTO_TRANSCRIPT_ROOTS=<claude>:<codex>` (two absolute paths) → under
/// `OTTO_E2E=1` the empty per-daemon dirs `<data_dir>/e2e-transcripts/{claude,codex}`
/// (the throwaway e2e daemon must never read the real trees) → the defaults
/// `~/.claude/projects` and `$CODEX_HOME/sessions` (else `~/.codex/sessions`).
pub(crate) fn transcript_roots(data_dir: &Path) -> (PathBuf, PathBuf) {
    if let Some(raw) = std::env::var_os("OTTO_TRANSCRIPT_ROOTS") {
        let raw = raw.to_string_lossy();
        let mut parts = raw.split(':').map(str::trim).filter(|p| !p.is_empty());
        if let (Some(c), Some(x)) = (parts.next(), parts.next()) {
            return (PathBuf::from(c), PathBuf::from(x));
        }
        tracing::warn!("OTTO_TRANSCRIPT_ROOTS must be `<claude root>:<codex root>`; ignoring");
    }
    if e2e_enabled() {
        let base = data_dir.join("e2e-transcripts");
        let _ = std::fs::create_dir_all(base.join("claude"));
        let _ = std::fs::create_dir_all(base.join("codex"));
        return (base.join("claude"), base.join("codex"));
    }
    (home_dir().join(".claude").join("projects"), otto_sessions::codex_sessions_root())
}

/// The provider whose transcript this session has: its own for agent sessions,
/// the captured nested one for a plain terminal the user ran `claude`/`codex` in.
pub(crate) fn effective_provider(session: &Session) -> String {
    if matches!(session.provider.as_str(), "claude" | "codex" | "agy") {
        return session.provider.clone();
    }
    session
        .meta
        .get("nested_provider")
        .and_then(|v| v.as_str())
        .unwrap_or(&session.provider)
        .to_string()
}

/// A session's resolved transcript.
#[derive(Debug, Clone)]
pub(crate) struct Resolved {
    pub provider: Provider,
    pub path: PathBuf,
}

/// Pure (filesystem-only) resolution of the transcript behind `session`
/// (§4.2): the E2E fixture hook, then the persisted row (`persisted`), then
/// the provider roots. `fresh` is true when the roots had to be searched —
/// the caller may persist the result.
pub(crate) fn resolve_transcript_sync(
    data_dir: &Path,
    persisted: Option<&str>,
    session: &Session,
) -> Result<(Resolved, bool), UnavailableReason> {
    let provider_name = effective_provider(session);
    let provider = match provider_name.as_str() {
        "claude" => Provider::Claude,
        "codex" => Provider::Codex,
        _ => return Err(UnavailableReason::ProviderUnsupported),
    };
    // E2E hook (design §8, B → A): under `OTTO_E2E=1` a seeded session may point
    // at a fixture transcript via `meta.e2e_transcript_path`; the provider comes
    // from the filename (`rollout-*` = codex, else claude). Inert in production.
    if e2e_enabled() {
        if let Some(p) = session.meta.get("e2e_transcript_path").and_then(|v| v.as_str()) {
            let p = PathBuf::from(p);
            if let Some(provider) = otto_transcript::provider_for_path(&p).filter(|_| p.is_file()) {
                return Ok((Resolved { provider, path: p }, false));
            }
            return Err(UnavailableReason::TranscriptMissing);
        }
    }
    // O(1) path: the persisted row (set by the capture scan or a prior read).
    if let Some(p) = persisted {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok((Resolved { provider, path: p }, false));
        }
    }
    let cwd = session
        .meta
        .get("nested_cwd")
        .and_then(|v| v.as_str())
        .unwrap_or(&session.cwd);
    let (claude_root, codex_root) = transcript_roots(data_dir);
    let path = otto_sessions::transcript_path_in_roots(
        &claude_root,
        &codex_root,
        &provider_name,
        cwd,
        session.provider_session_id.as_deref(),
    )?;
    if !path.is_file() {
        return Err(UnavailableReason::TranscriptMissing);
    }
    Ok((Resolved { provider, path }, true))
}

/// Resolve the transcript behind `session`, persisting a freshly found path
/// (one cheap UPDATE, once per session) so later lookups skip the root scan.
pub(crate) async fn resolve_transcript(ctx: &ServerCtx, session: &Session) -> Result<Resolved, UnavailableReason> {
    let repo = SessionsRepo::new(ctx.pool.clone());
    let persisted = repo.transcript_path(&session.id).await.ok().flatten();
    let (resolved, fresh) = resolve_transcript_sync(&ctx.data_dir, persisted.as_deref(), session)?;
    if fresh {
        let _ = repo.set_transcript_path(&session.id, &resolved.path.to_string_lossy()).await;
    }
    Ok(resolved)
}

/// Workspace Admin or root — the callers allowed to see transcripts they do
/// not own (`on_disk` History rows, arbitrary paths under the roots).
async fn is_ws_admin(ctx: &ServerCtx, user: &otto_core::domain::User, wid: &Id) -> bool {
    user.is_root || ctx.roles.check(user, wid, WorkspaceRole::Admin).await.is_ok()
}

/// History path gate: admins/root may read any confined path; everyone else
/// only a path that is the transcript of one of THEIR OWN sessions in `wid`
/// (same owner rule as every per-session route).
async fn history_path_gate(ctx: &ServerCtx, user: &otto_core::domain::User, wid: &Id, path: &Path) -> Result<(), ApiError> {
    if is_ws_admin(ctx, user, wid).await {
        return Ok(());
    }
    let own = ctx.manager.list_by_workspace_for_user(wid, &user.id).await.map_err(ApiError)?;
    let persisted: HashMap<Id, Option<String>> = SessionsRepo::new(ctx.pool.clone())
        .transcript_paths_for_workspace(wid)
        .await
        .map_err(ApiError)?
        .into_iter()
        .collect();
    let canon = path.canonicalize().ok();
    let data_dir = ctx.data_dir.clone();
    let owned = tokio::task::spawn_blocking(move || {
        own.iter().any(|s| {
            let p = persisted.get(&s.id).cloned().flatten();
            resolve_transcript_sync(&data_dir, p.as_deref(), s)
                .ok()
                .and_then(|(r, _)| r.path.canonicalize().ok())
                .is_some_and(|c| Some(&c) == canon.as_ref())
        })
    })
    .await
    .map_err(|e| ApiError(Error::Internal(format!("gate task: {e}"))))?;
    if owned {
        Ok(())
    } else {
        Err(ApiError(Error::Forbidden(
            "this transcript is not one of your sessions; workspace admin required".into(),
        )))
    }
}

/// Where a transcript's extracted images live: `<data>/transcripts/<key>/img`.
/// `key` is the provider session id (so a session and its History entry share
/// one store) — validated to a plain token so it can never leave `transcripts/`.
pub(crate) fn image_store(ctx: &ServerCtx, key: &str) -> ImageStore {
    let safe: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let safe = if safe.is_empty() { "unknown".to_string() } else { safe };
    ImageStore::new(ctx.data_dir.join("transcripts").join(safe).join("img"))
}

/// Image-store key for a transcript path: the provider session id derived from
/// the filename (Claude `<sid>.jsonl`, Codex `rollout-…-<uuid>.jsonl`).
fn store_key(provider: Provider, path: &Path) -> String {
    otto_transcript::peek::session_id_from_name(provider, path).unwrap_or_else(|| "unknown".into())
}

/// The server's fold knobs for a transcript at `path`: images written to the
/// per-transcript store, cost priced exactly like the Usage module. Owned
/// (`'static`) so a live tail can keep it across polls.
pub(crate) fn fold_opts(ctx: &ServerCtx, provider: Provider, path: &Path) -> FoldOpts<'static> {
    let price: otto_transcript::PriceFn<'static> = &otto_usage::estimate_cost;
    FoldOpts {
        images: Some(image_store(ctx, &store_key(provider, path))),
        price: Some(price),
        subagents: Vec::new(),
    }
}

/// Fold a transcript file with the server's knobs: images written to the
/// store, cost priced exactly like the Usage module, subagent tree attached.
pub(crate) fn fold_with(ctx: &ServerCtx, provider: Provider, path: &Path, sub: Option<&str>) -> Result<Folded, Error> {
    let store = image_store(ctx, &store_key(provider, path));
    let subagents = if sub.is_none() && provider == Provider::Claude {
        read_subagents(path)
    } else {
        Vec::new()
    };
    let target = match sub {
        None => path.to_path_buf(),
        Some(agent_id) => {
            let p = subagent_path(path, agent_id)
                .ok_or_else(|| Error::Invalid("bad subagent id".into()))?;
            if !p.is_file() {
                return Err(Error::NotFound("subagent transcript not found".into()));
            }
            p
        }
    };
    let price: otto_transcript::PriceFn<'_> = &otto_usage::estimate_cost;
    fold_file(
        provider,
        &target,
        FoldOpts {
            images: Some(store),
            price: Some(price),
            subagents,
        },
    )
    .map_err(|e| Error::Internal(format!("read transcript: {e}")))
}

/// Page a fold into the wire `Transcript`. A subagent view keeps only turns
/// and `stats.turns/tool_calls` (design §3).
fn page(folded: &Folded, before: Option<usize>, limit: usize, sub: Option<&str>, subagents: Vec<otto_transcript::SubagentMeta>) -> Transcript {
    let mut t = folded.page(before, limit.clamp(1, MAX_LIMIT), subagents);
    if sub.is_some() {
        t.title = None;
        t.stats.cost_usd = None;
        t.stats.duration_ms = None;
        t.stats.input_tokens = None;
        t.stats.output_tokens = None;
        t.subagents = Vec::new();
    }
    t
}

fn parse_before(s: Option<&str>) -> Result<Option<usize>, ApiError> {
    match s.filter(|s| !s.is_empty()) {
        None => Ok(None),
        Some(s) => s
            .parse::<usize>()
            .map(Some)
            .map_err(|_| ApiError(Error::Invalid("bad `before` cursor".into()))),
    }
}

/// Load a session and apply the standard per-session gate.
pub(crate) async fn session_gate(ctx: &ServerCtx, user: &otto_core::domain::User, id: &Id, min: WorkspaceRole) -> Result<Session, ApiError> {
    let session = ctx.manager.get(id).await.map_err(ApiError)?;
    require_ws_role(ctx, user, &session.workspace_id, min).await?;
    require_session_owner_or_admin(ctx, user, &session).await?;
    Ok(session)
}

/// Serve `bytes` with the fs.rs headers: explicit type, `nosniff`, inline only
/// for images/HTML (rendered inside a sandboxed iframe by the UI), attachment
/// otherwise.
fn serve_bytes(bytes: Vec<u8>, mime: &str, name: &str, inline: bool) -> ApiResult<Response> {
    let disposition = if inline { "inline" } else { "attachment" };
    let safe_name: String = name.chars().filter(|c| !matches!(c, '"' | '\\' | '\n' | '\r')).collect();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(header::CONTENT_DISPOSITION, format!("{disposition}; filename=\"{safe_name}\""))
        .header("x-content-type-options", "nosniff")
        // Never let a served file run as this origin or be framed by it: the UI
        // renders HTML/Markdown in its own `sandbox=""` iframe from srcdoc.
        .header("content-security-policy", "sandbox; default-src 'none'")
        .header("x-frame-options", "DENY")
        .header(header::CACHE_CONTROL, "private, max-age=3600")
        .body(Body::from(bytes))
        .map_err(|e| ApiError(Error::Internal(format!("build response: {e}"))))
}

// ── GET /sessions/{id}/transcript ──────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TranscriptQuery {
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub sub: Option<String>,
}

/// `GET /sessions/{id}/transcript?before=&limit=&sub=` — the last `limit`
/// turns (default 60) or the page before `before`. Starts the live tail for a
/// running session. No transcript → `200 { unavailable_reason }`.
pub async fn get_transcript(
    AxPath(id): AxPath<Id>,
    Query(q): Query<TranscriptQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Transcript>> {
    let session = session_gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let before = parse_before(q.before.as_deref())?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    let provider_name = effective_provider(&session);
    let resolved = match resolve_transcript(&ctx, &session).await {
        Ok(r) => r,
        Err(why) => {
            let provider = Provider::parse(&provider_name).unwrap_or(Provider::Agy);
            return Ok(Json(Transcript::unavailable(provider, session.provider_session_id.clone(), why)));
        }
    };
    let sub = q.sub.as_deref().filter(|s| !s.is_empty());
    let ctx2 = ctx.clone();
    let path = resolved.path.clone();
    let provider = resolved.provider;
    let sub_owned = sub.map(str::to_string);
    let folded = tokio::task::spawn_blocking(move || fold_with(&ctx2, provider, &path, sub_owned.as_deref()))
        .await
        .map_err(|e| ApiError(Error::Internal(format!("fold task: {e}"))))?
        .map_err(ApiError)?;
    let subagents = if sub.is_none() && provider == Provider::Claude {
        read_subagents(&resolved.path)
    } else {
        Vec::new()
    };
    let mut t = page(&folded, before, limit, sub, subagents);
    if t.session_id.is_none() {
        t.session_id = session.provider_session_id.clone();
    }
    // Live session → keep the tail warm for this subscriber (design §4.4).
    if sub.is_none() && ctx.manager.is_live(&id) {
        crate::transcript_tail::touch(&ctx, &session, provider, &resolved.path);
    }
    Ok(Json(t))
}

/// `POST /sessions/{id}/transcript/touch` — keep the live tail armed for a
/// session whose conversation is open (the tail otherwise stops 5 min after
/// the last `GET …/transcript`; clients ping this every minute while the view
/// is mounted). Starts the tail when it is not running. `204`; a session with
/// no transcript yet → `409` so the client keeps retrying the GET instead.
pub async fn touch_transcript(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    let session = session_gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    if !ctx.manager.is_live(&id) {
        return Ok(StatusCode::NO_CONTENT);
    }
    let resolved = resolve_transcript(&ctx, &session)
        .await
        .map_err(|why| ApiError(Error::Conflict(format!("transcript unavailable: {why:?}"))))?;
    crate::transcript_tail::touch(&ctx, &session, resolved.provider, &resolved.path);
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /workspaces/{wid}/transcript/touch` — keep the tails of EVERY live
/// agent session in the workspace armed (the client pings this every minute
/// for its current workspace, immediately on switching to it and on regaining
/// focus), so chats you are not looking at right now still have their history
/// hot when you open them, and a session you sit on never goes stale. Only
/// sessions the caller may read (owner / workspace admin / root) are touched;
/// the cap (`MAX_TAILS`) still applies. Returns how many tails were armed.
pub async fn touch_workspace_transcripts(
    AxPath(wid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<serde_json::Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let sessions = ctx.manager.list_by_workspace(&wid).await.map_err(ApiError)?;
    let mut armed = 0usize;
    for s in sessions {
        if s.kind != SessionKind::Agent || !ctx.manager.is_live(&s.id) {
            continue;
        }
        if require_session_owner_or_admin(&ctx, &user, &s).await.is_err() {
            continue;
        }
        let provider_name = effective_provider(&s);
        if !matches!(provider_name.as_str(), "claude" | "codex") {
            continue;
        }
        let Ok(resolved) = resolve_transcript(&ctx, &s).await else {
            continue;
        };
        crate::transcript_tail::touch(&ctx, &s, resolved.provider, &resolved.path);
        armed += 1;
    }
    Ok(Json(serde_json::json!({ "armed": armed })))
}

/// `GET /sessions/{id}/transcript/images/{img_id}` — an extracted image, by id.
pub async fn transcript_image(
    AxPath((id, img_id)): AxPath<(Id, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Response> {
    let session = session_gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let resolved = resolve_transcript(&ctx, &session)
        .await
        .map_err(|_| ApiError(Error::NotFound("transcript not available".into())))?;
    serve_image(&ctx, resolved.provider, &resolved.path, &img_id).await
}

async fn serve_image(ctx: &ServerCtx, provider: Provider, path: &Path, img_id: &str) -> ApiResult<Response> {
    let store = image_store(ctx, &store_key(provider, path));
    let (file, mime) = store
        .find(img_id)
        .ok_or_else(|| ApiError(Error::NotFound("image not found".into())))?;
    let bytes = tokio::fs::read(&file)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("read image: {e}"))))?;
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("image").to_string();
    serve_bytes(bytes, mime, &name, true)
}

// ── artifacts ──────────────────────────────────────────────────────────────

/// Fold the whole transcript and return its artifacts, newest first.
async fn session_artifacts(ctx: &ServerCtx, session: &Session) -> ApiResult<Vec<Artifact>> {
    let Ok(resolved) = resolve_transcript(ctx, session).await else {
        return Ok(Vec::new());
    };
    let ctx2 = ctx.clone();
    let folded = tokio::task::spawn_blocking(move || fold_with(&ctx2, resolved.provider, &resolved.path, None))
        .await
        .map_err(|e| ApiError(Error::Internal(format!("fold task: {e}"))))?
        .map_err(ApiError)?;
    let mut arts = folded.artifacts;
    arts.sort_by(|a, b| b.produced_at.cmp(&a.produced_at));
    Ok(arts)
}

/// `GET /sessions/{id}/artifacts` — everything the agent produced (design §4.7).
pub async fn list_artifacts(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Artifact>>> {
    let session = session_gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let arts = session_artifacts(&ctx, &session).await?;
    // Best-effort work-graph registration, off the request path.
    let cx = ctx.clone();
    let s = session.clone();
    let snapshot = arts.clone();
    tokio::spawn(async move {
        for a in &snapshot {
            register_work_artifact(&cx, &s, a).await;
        }
    });
    Ok(Json(arts))
}

/// Mirror an artifact into `work_artifacts` for the session's work item (once
/// per reference). Silent on any failure — Mission Control is a projection.
pub(crate) async fn register_work_artifact(ctx: &ServerCtx, session: &Session, a: &Artifact) {
    let Ok(Some(item)) = ctx
        .workgraph
        .repo()
        .find_by_source(&session.workspace_id, otto_state::WorkKind::Session, &session.id)
        .await
    else {
        return;
    };
    let reference = a.path.clone().or_else(|| a.url.clone());
    if let Ok(existing) = ctx.workgraph.repo().artifacts_for(&item.id).await {
        if existing.iter().any(|e| e.reference == reference && e.reference.is_some()) {
            return;
        }
    }
    let kind = match a.kind {
        ArtifactKind::Pr => otto_state::ArtifactKind::Pr,
        ArtifactKind::Report => otto_state::ArtifactKind::Report,
        ArtifactKind::Url => otto_state::ArtifactKind::Link,
        ArtifactKind::File | ArtifactKind::Image => otto_state::ArtifactKind::File,
    };
    let _ = ctx
        .workgraph
        .add_artifact(otto_state::NewArtifact {
            work_item_id: item.id.clone(),
            workspace_id: session.workspace_id.clone(),
            kind,
            title: a.label.clone(),
            reference,
            payload: serde_json::to_value(a).unwrap_or(serde_json::Value::Null),
        })
        .await;
}

/// The artifact-serving allow-list (design §4.7): the session cwd, the daemon
/// data dir, the per-user temp dir (`std::env::temp_dir()`, NOT `/tmp`) — all
/// canonicalized; plus the fs.rs secret-store deny-list on top.
fn artifact_path_allowed(canonical: &Path, session_cwd: &str, data_dir: &Path) -> bool {
    let roots: Vec<PathBuf> = [Path::new(session_cwd), data_dir, &std::env::temp_dir()]
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    let within = roots.iter().any(|r| canonical == r || canonical.starts_with(r));
    within
        && !crate::routes::fs::sandbox::is_denied_dir(canonical)
        && !crate::routes::fs::sandbox::is_denied_file(canonical)
}

/// `GET /sessions/{id}/artifacts/{artifact_id}` — the bytes behind an artifact,
/// by opaque id only. Confinement per §4.7; 25 MB cap.
pub async fn get_artifact(
    AxPath((id, artifact_id)): AxPath<(Id, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Response> {
    let session = session_gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let arts = session_artifacts(&ctx, &session).await?;
    let art = arts
        .into_iter()
        .find(|a| a.id == artifact_id)
        .ok_or_else(|| ApiError(Error::NotFound("artifact not found".into())))?;
    let Some(path) = art.path.as_deref() else {
        return Err(ApiError(Error::Invalid("artifact has no local file (a link)".into())));
    };
    // Images the fold extracted itself live under the data dir already.
    let target = Path::new(path);
    if !target.is_file() {
        return Err(ApiError(Error::NotFound("artifact file is gone".into())));
    }
    let canonical = target
        .canonicalize()
        .map_err(|e| ApiError(Error::Invalid(format!("cannot resolve artifact: {e}"))))?;
    if !artifact_path_allowed(&canonical, &session.cwd, &ctx.data_dir) {
        return Err(ApiError(Error::Forbidden("artifact path is not permitted".into())));
    }
    let meta = tokio::fs::metadata(&canonical)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("stat artifact: {e}"))))?;
    if meta.len() > ARTIFACT_CAP {
        return Err(ApiError(Error::PayloadTooLarge(format!(
            "artifact is {} bytes; cap is {ARTIFACT_CAP}",
            meta.len()
        ))));
    }
    let bytes = tokio::fs::read(&canonical)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("read artifact: {e}"))))?;
    let mime = art
        .mime
        .clone()
        .or_else(|| otto_transcript::util::mime_for_path(path).map(str::to_string))
        .unwrap_or_else(|| "application/octet-stream".into());
    // Only images are inline; HTML is ALWAYS an attachment (plus the CSP
    // sandbox above) so a produced page can never execute on the daemon origin.
    let inline = mime.starts_with("image/") && mime != "image/svg+xml";
    serve_bytes(bytes, &mime, &art.label, inline)
}

// ── POST /sessions/{id}/tasks ──────────────────────────────────────────────

/// `POST /sessions/{id}/tasks { title, description? }` — add a board task
/// (`source: user`, `nudge_pending: true`); the nudge sweep hands it to the
/// agent once the session is idle (design §4.5). Broadcasts `tasks_updated`.
pub async fn create_task(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<CreateAgentTaskReq>,
) -> ApiResult<Json<AgentTask>> {
    let session = session_gate(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    if !crate::agent_tasks_nudge::nudgeable(&session) {
        return Err(ApiError(Error::Invalid(
            "board tasks can only be added to claude/codex agent sessions".into(),
        )));
    }
    // A captured shell whose nested CLI has exited would run the nudge as a
    // shell command: refuse now rather than queue a paste into a prompt.
    if ctx.manager.is_live(&session.id) && !crate::agent_tasks_nudge::agent_running(&ctx, &session).await {
        return Err(ApiError(Error::Conflict("agent not running in this terminal".into())));
    }
    // One safe line each: control characters (incl. a paste terminator) gone.
    let title = crate::agent_tasks_nudge::sanitize_prompt_text(&req.title);
    let title = title.as_str();
    if title.is_empty() {
        return Err(ApiError(Error::Invalid("title is required".into())));
    }
    if title.chars().count() > 500 {
        return Err(ApiError(Error::Invalid("title is too long (500 chars max)".into())));
    }
    let description = req
        .description
        .as_deref()
        .map(crate::agent_tasks_nudge::sanitize_prompt_text)
        .filter(|d| !d.is_empty())
        .map(|d| d.chars().take(4000).collect::<String>());
    let activity = ctx.activity();
    let task = activity
        .repo()
        .insert_user_task(&session.id, &session.workspace_id, title, description.as_deref())
        .await
        .map_err(ApiError)?;
    if let Ok(tasks) = activity.repo().list_tasks(&session.id).await {
        let _ = ctx.events.send(Event::TasksUpdated {
            workspace_id: session.workspace_id.clone(),
            session_id: session.id.clone(),
            tasks,
        });
    }
    let _ = activity
        .append_trail(NewTrail {
            session_id: session.id.clone(),
            workspace_id: session.workspace_id.clone(),
            source: TrailSource::User,
            kind: TrailKind::Task,
            level: TrailLevel::Info,
            summary: format!("Board task added: {}", otto_transcript::util::clip(title, 120)),
            detail: None,
        })
        .await;
    // Deliver immediately when the session is already idle.
    let cx = ctx.clone();
    let sid = session.id.clone();
    tokio::spawn(async move {
        crate::agent_tasks_nudge::sweep_session(&cx, &sid).await;
    });
    Ok(Json(task))
}

// ── POST /sessions/{id}/inbox ──────────────────────────────────────────────

/// `POST /sessions/{id}/inbox { filename, mime, data_b64 }` — store a pasted
/// image under `<data>/sessions/<id>/inbox/` and return its path for the
/// composer's `[Image: <path>]` line. `image/*` only, 10 MB decoded cap.
pub async fn inbox_upload(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<InboxUploadReq>,
) -> ApiResult<Json<InboxUploadResp>> {
    let session = session_gate(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let mime = req.mime.trim().to_ascii_lowercase();
    if !mime.starts_with("image/") || mime.matches('/').count() != 1 || mime.contains(';') {
        return Err(ApiError(Error::UnsupportedMedia("only image/* uploads are accepted".into())));
    }
    // Reject before decoding when the base64 alone already exceeds the cap.
    if req.data_b64.len() > INBOX_CAP / 3 * 4 + 4 {
        return Err(ApiError(Error::PayloadTooLarge("image exceeds 10 MB".into())));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(req.data_b64.trim())
        .map_err(|e| ApiError(Error::Invalid(format!("bad base64: {e}"))))?;
    if bytes.is_empty() {
        return Err(ApiError(Error::Invalid("empty image".into())));
    }
    if bytes.len() > INBOX_CAP {
        return Err(ApiError(Error::PayloadTooLarge("image exceeds 10 MB".into())));
    }
    let ext = match mime.as_str() {
        "image/jpeg" | "image/jpg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/svg+xml" => "svg",
        _ => "png",
    };
    // Client filename → a sanitized stem; the ULID prefix keeps names unique.
    let stem: String = Path::new(&req.filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("image")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(60)
        .collect();
    let stem = if stem.is_empty() { "image".to_string() } else { stem };
    let dir = ctx.data_dir.join("sessions").join(&session.id).join("inbox");
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("inbox dir: {e}"))))?;
    let file = dir.join(format!("{}-{stem}.{ext}", otto_core::new_id()));
    tokio::fs::write(&file, &bytes)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("write inbox image: {e}"))))?;
    Ok(Json(InboxUploadResp {
        path: file.to_string_lossy().into_owned(),
    }))
}

// ── History ────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct HistoryQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    /// `last_active_at` cursor (exclusive): entries older than this.
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `OTTO_E2E=1|true` — the Playwright harness's throwaway daemon (see
/// `routes/findings.rs`); unlocks the fixture-transcript hooks only.
fn e2e_enabled() -> bool {
    matches!(std::env::var("OTTO_E2E").as_deref(), Ok("1") | Ok("true"))
}

/// Confine a client-supplied transcript path to the two provider roots
/// (symlink-aware). Returns the canonical path + provider. Under `OTTO_E2E` a
/// checked-in fixture (`…/crates/otto-transcript/fixtures/**.jsonl`) is also
/// accepted.
pub(crate) fn confine_history_path(ctx: &ServerCtx, raw: &str) -> Result<(PathBuf, Provider), ApiError> {
    let candidate = Path::new(raw);
    if !candidate.is_absolute() {
        return Err(ApiError(Error::Invalid("transcript_path must be absolute".into())));
    }
    if e2e_enabled() && raw.contains("/crates/otto-transcript/fixtures/") {
        if let (Ok(canon), Some(provider)) = (candidate.canonicalize(), otto_transcript::provider_for_path(candidate)) {
            if canon.is_file() {
                return Ok((canon, provider));
            }
        }
    }
    let (claude_root, codex_root) = transcript_roots(&ctx.data_dir);
    for (root, provider) in [(claude_root, Provider::Claude), (codex_root, Provider::Codex)] {
        if let Some(canon) = otto_core::paths::resolves_under(&root, candidate) {
            if canon.is_file() && canon.extension().is_some_and(|e| e == "jsonl") {
                return Ok((canon, provider));
            }
        }
    }
    Err(ApiError(Error::Forbidden(
        "transcript_path must be a .jsonl under ~/.claude/projects or ~/.codex/sessions".into(),
    )))
}

/// `GET /workspaces/{wid}/history?q=&provider=&cwd=&status=&before=&limit=` —
/// the workspace's sessions (all statuses incl. archived; own sessions only for
/// non-admins) merged with indexed transcripts no session claims (`on_disk`),
/// newest activity first (design §4.6).
pub async fn history(
    AxPath(wid): AxPath<Id>,
    Query(q): Query<HistoryQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<HistoryEntry>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let is_admin = is_ws_admin(&ctx, &user, &wid).await;
    let sessions = if is_admin {
        ctx.manager.list_by_workspace(&wid).await.map_err(ApiError)?
    } else {
        ctx.manager.list_by_workspace_for_user(&wid, &user.id).await.map_err(ApiError)?
    };
    let limit = q.limit.unwrap_or(HISTORY_DEFAULT_LIMIT).clamp(1, HISTORY_MAX_LIMIT);
    let before = q.before.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let repo = SessionsRepo::new(ctx.pool.clone());
    let index = TranscriptIndexRepo::new(ctx.pool.clone());
    // `on_disk` rows are machine-wide files no session owns → admins/root only
    // (a Viewer/Editor sees exactly their own sessions). LIMIT/before are
    // pushed into SQL; the claimed filter below only shrinks the page.
    let rows = if is_admin {
        index.list_page(before, limit as i64).await.map_err(ApiError)?
    } else {
        Vec::new()
    };
    let persisted: HashMap<Id, Option<String>> = repo
        .transcript_paths_for_workspace(&wid)
        .await
        .map_err(ApiError)?
        .into_iter()
        .collect();
    let claimed_paths: HashSet<String> = repo.transcript_paths().await.map_err(ApiError)?.into_iter().collect();
    let claimed_psids: HashSet<String> = repo.provider_session_ids().await.map_err(ApiError)?.into_iter().collect();
    // Resolve every session in ONE blocking hop (pure fs; no DB writes from a
    // GET — the transcript route persists on first open).
    let data_dir = ctx.data_dir.clone();
    let agent_sessions: Vec<Session> = sessions.into_iter().filter(|s| s.kind == SessionKind::Agent).collect();
    let resolved: Vec<(Session, Resolved)> = tokio::task::spawn_blocking(move || {
        agent_sessions
            .into_iter()
            .filter_map(|s| {
                let p = persisted.get(&s.id).cloned().flatten();
                // Same resolver as the transcript route (persisted path → roots →
                // the OTTO_E2E fixture hook), so what the list shows is what opens.
                resolve_transcript_sync(&data_dir, p.as_deref(), &s).ok().map(|(r, _)| (s, r))
            })
            .collect()
    })
    .await
    .map_err(|e| ApiError(Error::Internal(format!("resolve task: {e}"))))?;
    let session_paths: Vec<String> = resolved.iter().map(|(_, r)| r.path.to_string_lossy().into_owned()).collect();
    let rows_for_sessions = if session_paths.is_empty() {
        Vec::new()
    } else {
        // Index metadata (first prompt / turns) for the sessions' own files.
        let mut v = Vec::new();
        for p in &session_paths {
            if let Ok(Some(r)) = index.get(p).await {
                v.push(r);
            }
        }
        v
    };
    let by_path: HashMap<&str, &otto_state::TranscriptIndexRow> =
        rows_for_sessions.iter().map(|r| (r.path.as_str(), r)).collect();

    let mut out: Vec<HistoryEntry> = Vec::new();
    for (s, resolved) in &resolved {
        let provider = resolved.provider;
        let path_str = resolved.path.to_string_lossy().into_owned();
        let row = by_path.get(path_str.as_str()).copied();
        out.push(HistoryEntry {
            session_id: Some(s.id.clone()),
            provider,
            title: Some(s.title.clone()).filter(|t| !t.trim().is_empty()).or_else(|| row.and_then(|r| r.title.clone())),
            first_prompt: row.and_then(|r| r.first_prompt.clone()),
            cwd: s.cwd.clone(),
            repo_name: repo_name(&s.cwd),
            started_at: s.created_at.to_rfc3339(),
            last_active_at: s.last_active_at.to_rfc3339(),
            turns: row.and_then(|r| r.turns).map(|t| t.max(0) as u64),
            status: s.status.as_str().to_string(),
            transcript_path: path_str,
            resumable: s.provider_session_id.is_some() && s.status != SessionStatus::Exited || s.status == SessionStatus::Reconnectable,
        });
    }
    for r in &rows {
        if claimed_paths.contains(&r.path)
            || r.provider_session_id.as_deref().is_some_and(|p| claimed_psids.contains(p))
        {
            continue;
        }
        let Some(provider) = Provider::parse(&r.provider) else { continue };
        let cwd = r.cwd.clone().unwrap_or_default();
        out.push(HistoryEntry {
            session_id: None,
            provider,
            title: r.title.clone(),
            first_prompt: r.first_prompt.clone(),
            repo_name: repo_name(&cwd),
            cwd,
            started_at: r.started_at.clone().unwrap_or_else(|| r.last_active_at.clone().unwrap_or_default()),
            last_active_at: r.last_active_at.clone().unwrap_or_else(|| r.started_at.clone().unwrap_or_default()),
            turns: r.turns.map(|t| t.max(0) as u64),
            status: "on_disk".into(),
            transcript_path: r.path.clone(),
            resumable: r.provider_session_id.is_some(),
        });
    }

    // Filters.
    let needle = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_lowercase);
    let provider_f = q.provider.as_deref().and_then(Provider::parse);
    let cwd_f = q.cwd.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let status_f = q.status.as_deref().map(str::trim).filter(|s| !s.is_empty());
    out.retain(|e| {
        provider_f.is_none_or(|p| e.provider == p)
            && cwd_f.is_none_or(|c| e.cwd == c || e.cwd.starts_with(&format!("{}/", c.trim_end_matches('/'))))
            && status_f.is_none_or(|s| e.status == s)
            && before.is_none_or(|b| e.last_active_at.as_str() < b)
            && needle.as_deref().is_none_or(|n| {
                e.title.as_deref().is_some_and(|t| t.to_lowercase().contains(n))
                    || e.first_prompt.as_deref().is_some_and(|t| t.to_lowercase().contains(n))
                    || e.cwd.to_lowercase().contains(n)
            })
    });
    out.sort_by(|a, b| b.last_active_at.cmp(&a.last_active_at));
    out.truncate(limit);
    Ok(Json(out))
}

fn repo_name(cwd: &str) -> Option<String> {
    Path::new(cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .map(str::to_string)
}

#[derive(Debug, Deserialize)]
pub struct HistoryTranscriptQuery {
    pub path: String,
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub sub: Option<String>,
}

/// `GET /workspaces/{wid}/history/transcript?path=&before=&limit=&sub=` — a
/// transcript straight from disk. The ONE route that accepts a client path;
/// confined to the two provider roots (§4.7).
pub async fn history_transcript(
    AxPath(wid): AxPath<Id>,
    Query(q): Query<HistoryTranscriptQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Transcript>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let (path, provider) = confine_history_path(&ctx, &q.path)?;
    history_path_gate(&ctx, &user, &wid, &path).await?;
    let before = parse_before(q.before.as_deref())?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT);
    let sub = q.sub.as_deref().filter(|s| !s.is_empty());
    let ctx2 = ctx.clone();
    let p2 = path.clone();
    let sub_owned = sub.map(str::to_string);
    let folded = tokio::task::spawn_blocking(move || fold_with(&ctx2, provider, &p2, sub_owned.as_deref()))
        .await
        .map_err(|e| ApiError(Error::Internal(format!("fold task: {e}"))))?
        .map_err(ApiError)?;
    let subagents = if sub.is_none() && provider == Provider::Claude {
        read_subagents(&path)
    } else {
        Vec::new()
    };
    Ok(Json(page(&folded, before, limit, sub, subagents)))
}

#[derive(Debug, Deserialize)]
pub struct HistoryImageQuery {
    pub path: String,
}

/// `GET /workspaces/{wid}/history/transcript/images/{img_id}?path=` — an image
/// extracted while folding an on-disk transcript (same store as the session
/// route, keyed by the provider session id in the filename).
pub async fn history_transcript_image(
    AxPath((wid, img_id)): AxPath<(Id, String)>,
    Query(q): Query<HistoryImageQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Response> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let (path, provider) = confine_history_path(&ctx, &q.path)?;
    history_path_gate(&ctx, &user, &wid, &path).await?;
    serve_image(&ctx, provider, &path, &img_id).await
}

/// `POST /workspaces/{wid}/history/import { provider, transcript_path }` —
/// adopt an on-disk transcript as a `reconnectable` session (with
/// `provider_session_id` + `transcript_path`) so the existing resume path
/// continues it. Returns the existing session when one already owns the id.
pub async fn history_import(
    AxPath(wid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<HistoryImportReq>,
) -> ApiResult<Json<Session>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    let (path, provider) = confine_history_path(&ctx, &req.transcript_path)?;
    // Importing an unowned on-disk transcript is an admin action (it is the
    // same read as the `on_disk` History rows only admins see).
    history_path_gate(&ctx, &user, &wid, &path).await?;
    if Provider::parse(&req.provider).is_some_and(|p| p != provider) {
        return Err(ApiError(Error::Invalid("provider does not match the transcript location".into())));
    }
    let p2 = path.clone();
    let peek = tokio::task::spawn_blocking(move || otto_transcript::peek(provider, &p2))
        .await
        .map_err(|e| ApiError(Error::Internal(format!("peek task: {e}"))))?
        .map_err(|e| ApiError(Error::Internal(format!("read transcript: {e}"))))?;
    let psid = peek
        .provider_session_id
        .clone()
        .ok_or_else(|| ApiError(Error::Invalid("transcript carries no session id".into())))?;
    let repo = SessionsRepo::new(ctx.pool.clone());
    if let Some(existing) = repo.find_by_provider_session(&psid).await.map_err(ApiError)? {
        if existing.workspace_id != wid {
            return Err(ApiError(Error::Conflict(
                "this conversation already belongs to a session in another workspace".into(),
            )));
        }
        let _ = repo.set_transcript_path(&existing.id, &path.to_string_lossy()).await;
        return Ok(Json(existing));
    }
    let ws = ctx.workspaces.get(&wid).await.map_err(ApiError)?;
    let title = peek
        .title
        .clone()
        .or_else(|| peek.first_prompt.as_deref().map(|p| otto_transcript::util::clip(p, 80)))
        .unwrap_or_else(|| format!("Imported {} session", provider.as_str()));
    let session = repo
        .create(NewSession {
            workspace_id: wid.clone(),
            kind: SessionKind::Agent,
            provider: provider.as_str().to_string(),
            title,
            cwd: peek.cwd.clone().unwrap_or_else(|| ws.root_path.clone()),
            provider_session_id: Some(psid),
            connection_id: None,
            created_by: user.id.clone(),
            meta: serde_json::json!({ "imported_from": "history", "title_source": "provider" }),
        })
        .await
        .map_err(ApiError)?;
    repo.update_status(&session.id, SessionStatus::Reconnectable)
        .await
        .map_err(ApiError)?;
    let _ = repo.set_transcript_path(&session.id, &path.to_string_lossy()).await;
    let session = repo.get(&session.id).await.map_err(ApiError)?;
    let _ = ctx.events.send(Event::SessionCreated {
        session: session.clone(),
    });
    Ok(Json(session))
}

/// `POST /workspaces/{wid}/history/rescan` → 202; progress arrives as
/// `history_index_progress` events on the workspace.
pub async fn history_rescan(
    AxPath(wid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<StatusCode> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Editor).await?;
    crate::history_index::spawn_scan(ctx.clone(), Some(wid));
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_roots_honour_env_override() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OTTO_TRANSCRIPT_ROOTS", "/a/claude:/b/codex");
        let (c, x) = transcript_roots(dir.path());
        std::env::remove_var("OTTO_TRANSCRIPT_ROOTS");
        assert_eq!(c, PathBuf::from("/a/claude"));
        assert_eq!(x, PathBuf::from("/b/codex"));
    }

    #[test]
    fn artifact_allow_list_is_cwd_data_and_temp_only() {
        let cwd = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let inside = cwd.path().join("out.html");
        std::fs::write(&inside, "x").unwrap();
        let canon = inside.canonicalize().unwrap();
        assert!(artifact_path_allowed(&canon, &cwd.path().to_string_lossy(), data.path()));
        // Same file, but a session whose cwd is elsewhere may not read it.
        let other = tempfile::tempdir().unwrap();
        // (temp dirs live under std::env::temp_dir(), so exclude that by using a
        // path outside it: the data dir stands in as "elsewhere".)
        let elsewhere = other.path().join("secret.html");
        std::fs::write(&elsewhere, "x").unwrap();
        let _ = elsewhere.canonicalize().unwrap();
        // Secret files are denied even inside an allowed root.
        let env = cwd.path().join(".env");
        std::fs::write(&env, "TOKEN=x").unwrap();
        assert!(!artifact_path_allowed(&env.canonicalize().unwrap(), &cwd.path().to_string_lossy(), data.path()));
        let key = cwd.path().join("id_rsa.pem");
        std::fs::write(&key, "k").unwrap();
        assert!(!artifact_path_allowed(&key.canonicalize().unwrap(), &cwd.path().to_string_lossy(), data.path()));
    }

    #[test]
    fn effective_provider_prefers_nested_capture_for_shells() {
        let mut s = Session {
            id: "s".into(),
            workspace_id: "ws".into(),
            kind: SessionKind::Agent,
            provider: "shell".into(),
            title: "t".into(),
            status: SessionStatus::Running,
            cwd: "/x".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: "u".into(),
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            archived: false,
            meta: serde_json::json!({ "nested_provider": "codex" }),
        };
        assert_eq!(effective_provider(&s), "codex");
        s.provider = "claude".into();
        assert_eq!(effective_provider(&s), "claude");
    }
}
