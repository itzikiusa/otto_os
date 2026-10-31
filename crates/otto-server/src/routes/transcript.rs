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

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use axum::body::Body;
use axum::extract::{Path as AxPath, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::Json;
use base64::Engine;
use otto_core::api::{CreateAgentTaskReq, HistoryImportReq, InboxUploadReq, InboxUploadResp};
use otto_core::domain::{
    AgentTask, Session, SessionKind, SessionStatus, TrailKind, TrailLevel, TrailSource,
    WorkspaceRole,
};
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_state::{NewSession, NewTrail, SessionsRepo};
use otto_transcript::{
    fold_file_limited, read_subagents_limited, subagent_path, Artifact, FoldOpts, Folded,
    HistoryEntry, Provider, Transcript,
};
use serde::Deserialize;

use crate::auth::{
    require_session_owner_or_admin, require_ws_role, CurrentAuthContext, CurrentUser,
};
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

// Transcript resolution (provider roots, the effective provider and session →
// transcript path), the fold knobs and the work-graph artifact mirror live in
// `crate::transcript` (architecture-3-03/-3-03r); callers outside this route
// import them from there. Two stay re-exported for sibling routes not yet
// repointed: `routes/history_page.rs` (`transcript::resolve_transcript_sync`)
// and `routes/slash_commands.rs` (`super::transcript::effective_provider`).
use crate::transcript::artifacts::register_work_artifacts;
use crate::transcript::fold::{image_store, store_key};
use crate::transcript::resolve::{e2e_enabled, resolve_transcript, transcript_roots};
pub(crate) use crate::transcript::resolve::{effective_provider, resolve_transcript_sync};

/// Workspace Admin or root — the callers allowed to see transcripts they do
/// not own (`on_disk` History rows, arbitrary paths under the roots).
pub(crate) async fn is_ws_admin(ctx: &ServerCtx, user: &otto_core::domain::User, wid: &Id) -> bool {
    user.is_root
        || ctx
            .roles
            .check(user, wid, WorkspaceRole::Admin)
            .await
            .is_ok()
}

/// History path gate: admins/root may read any confined path; everyone else
/// only a path that is the transcript of one of THEIR OWN sessions in `wid`
/// (same owner rule as every per-session route).
///
/// backend-performance-5-02: one indexed lookup answers the common case — an
/// own session whose persisted `transcript_path` is this file, or (no path
/// persisted yet) whose `provider_session_id` is the id in its filename. Only
/// a miss falls back to resolving the caller's sessions one by one, skipping
/// rows with a fresh "no transcript" mark, and a hit there persists the path
/// so the next page takes the indexed route.
async fn history_path_gate(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    wid: &Id,
    path: &Path,
) -> Result<(), ApiError> {
    if is_ws_admin(ctx, user, wid).await {
        return Ok(());
    }
    let raw = path.to_path_buf();
    #[allow(clippy::disallowed_methods)] // canonicalize on the blocking pool
    let canon = crate::offload::blocking(move || raw.canonicalize().ok()).await;
    let Some(canon) = canon else {
        return Err(not_your_transcript());
    };
    if own_session_for_path(ctx, wid, &user.id, path, &canon)
        .await
        .map_err(ApiError)?
    {
        return Ok(());
    }
    // Fallback: a session whose path was never persisted (or was stored in a
    // non-canonical form). Rows the root scan found nothing for within the
    // mark's TTL are skipped — they would only re-scan the roots.
    let own = ctx
        .manager
        .list_by_workspace_for_user(wid, &user.id)
        .await
        .map_err(ApiError)?;
    let ids: Vec<String> = own.iter().map(|s| s.id.clone()).collect();
    let missing = crate::transcript::resolve::fresh_missing_marks(&ctx.pool, &ids).await;
    let persisted: HashMap<Id, Option<String>> = SessionsRepo::new(ctx.pool.clone())
        .transcript_paths_for_workspace(wid)
        .await
        .map_err(ApiError)?
        .into_iter()
        .collect();
    let data_dir = ctx.data_dir.clone();
    let hit = tokio::task::spawn_blocking(move || {
        own.iter()
            .filter(|s| !missing.contains(&s.id))
            .find_map(|s| {
                let p = persisted.get(&s.id).cloned().flatten();
                let (r, fresh) = resolve_transcript_sync(&data_dir, p.as_deref(), s).ok()?;
                let c = r.path.canonicalize().ok()?;
                (c == canon).then(|| (s.id.clone(), fresh.then_some(r.path)))
            })
    })
    .await
    .map_err(|e| ApiError(Error::Internal(format!("gate task: {e}"))))?;
    match hit {
        Some((id, found)) => {
            if let Some(found) = found {
                crate::transcript::resolve::record_found(&ctx.pool, &id, &found).await;
            }
            Ok(())
        }
        None => Err(not_your_transcript()),
    }
}

fn not_your_transcript() -> ApiError {
    ApiError(Error::Forbidden(
        "this transcript is not one of your sessions; workspace admin required".into(),
    ))
}

/// The indexed half of [`history_path_gate`]: does `user_id` own a session in
/// `wid` whose persisted transcript is `path` / `canon`, or — with no path
/// persisted — whose provider session id is the one in the file's name?
async fn own_session_for_path(
    ctx: &ServerCtx,
    wid: &Id,
    user_id: &Id,
    path: &Path,
    canon: &Path,
) -> otto_core::Result<bool> {
    let sid = otto_transcript::provider_for_path(canon)
        .and_then(|p| otto_transcript::peek::session_id_from_name(p, canon));
    let row: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM sessions WHERE workspace_id = ?1 AND created_by = ?2 \
         AND (transcript_path IN (?3, ?4) \
              OR (?5 IS NOT NULL AND provider_session_id = ?5 AND transcript_path IS NULL)) \
         LIMIT 1",
    )
    .bind(wid)
    .bind(user_id)
    .bind(canon.to_string_lossy().as_ref())
    .bind(path.to_string_lossy().as_ref())
    .bind(sid)
    .fetch_optional(&ctx.pool)
    .await
    .map_err(|e| Error::Internal(format!("history gate lookup: {e}")))?;
    Ok(row.is_some())
}

/// Fold a transcript file with the server's knobs: images written to the
/// store, cost priced exactly like the Usage module, subagent tree attached
/// (and returned, so the caller does not read the sidecars a second time).
pub(crate) fn fold_with(
    ctx: &ServerCtx,
    provider: Provider,
    path: &Path,
    sub: Option<&str>,
) -> Result<(Folded, Vec<otto_transcript::SubagentMeta>), Error> {
    let store = image_store(ctx, &store_key(provider, path));
    let subagents = if sub.is_none() && provider == Provider::Claude {
        read_subagents_limited(path).map_err(|e| {
            if e.kind() == std::io::ErrorKind::OutOfMemory {
                Error::PayloadTooLarge(e.to_string())
            } else {
                Error::Internal(format!("read subagent metadata: {e}"))
            }
        })?
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
    let folded = fold_file_limited(
        provider,
        &target,
        FoldOpts {
            images: Some(store),
            price: Some(price),
            subagents: subagents.clone(),
        },
        Default::default(),
    )
    .map_err(|e| {
        if e.kind() == std::io::ErrorKind::OutOfMemory {
            Error::PayloadTooLarge(e.to_string())
        } else {
            Error::Internal(format!("read transcript: {e}"))
        }
    })?;
    Ok((folded, subagents))
}

/// Authorization/resolution have already completed before entering the cache.
async fn cached_fold(
    ctx: &ServerCtx,
    provider: Provider,
    path: &Path,
    sub: Option<&str>,
) -> ApiResult<std::sync::Arc<crate::transcript_cache::Snapshot>> {
    if let Some(sub) = sub {
        if subagent_path(path, sub).is_none() {
            return Err(ApiError(Error::Invalid("bad subagent id".into())));
        }
    }
    let key = crate::transcript_cache::CacheKey {
        root: ctx.data_dir.clone(),
        path: path.to_path_buf(),
        provider,
        sub: sub.map(str::to_string),
    };
    let cx = ctx.clone();
    let path = path.to_path_buf();
    let sub = sub.map(str::to_string);
    ctx.transcript_cache
        .get(key, move || {
            let (folded, subagents) = fold_with(&cx, provider, &path, sub.as_deref())?;
            Ok(crate::transcript_cache::Snapshot { folded, subagents })
        })
        .await
        .map_err(ApiError)
}

/// Page a fold into the wire `Transcript`. A subagent view keeps only turns
/// and `stats.turns/tool_calls` (design §3). The subagent tree rides on the
/// newest page only: an earlier page (`before`) extends a list whose first
/// page already carried it (283 sidecars ≈ 55 KB per "Load earlier").
fn page(
    folded: &Folded,
    before: Option<usize>,
    limit: usize,
    sub: Option<&str>,
    subagents: Vec<otto_transcript::SubagentMeta>,
) -> Transcript {
    let subagents = if before.is_some() {
        Vec::new()
    } else {
        subagents
    };
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
pub(crate) async fn session_gate(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    id: &Id,
    min: WorkspaceRole,
) -> Result<Session, ApiError> {
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
    let safe_name: String = name
        .chars()
        .filter(|c| !matches!(c, '"' | '\\' | '\n' | '\r'))
        .collect();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime)
        .header(
            header::CONTENT_DISPOSITION,
            format!("{disposition}; filename=\"{safe_name}\""),
        )
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
    transcript_page(&ctx, &id, &session, q).await.map(Json)
}

/// The page behind `GET …/transcript`, for a session the caller was already
/// authorized for (owner gate above, or a session room's capability).
pub(crate) async fn transcript_page(
    ctx: &ServerCtx,
    id: &Id,
    session: &Session,
    q: TranscriptQuery,
) -> Result<Transcript, ApiError> {
    let (ctx, id, session) = (ctx.clone(), id.clone(), session.clone());
    let before = parse_before(q.before.as_deref())?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let provider_name = effective_provider(&session);
    let resolved = match resolve_transcript(&ctx, &session).await {
        Ok(r) => r,
        Err(why) => {
            let provider = Provider::parse(&provider_name).unwrap_or(Provider::Agy);
            return Ok(Transcript::unavailable(
                provider,
                session.provider_session_id.clone(),
                why,
            ));
        }
    };
    let sub = q.sub.as_deref().filter(|s| !s.is_empty());
    let provider = resolved.provider;
    // A live session's page comes from its tail: arm the tail FIRST and page
    // its fold (waiting for the initial one when it is just starting). That
    // is ONE fold of the file per open — the read path used to fold it and
    // then the tail folded it again — and the page is exactly the state the
    // `transcript_appended` deltas continue from. A file the agent keeps
    // appending to never makes this read "busy". Falls back to the cache
    // when no tail runs (not live, the tail cap, a different file).
    let is_live = sub.is_none() && ctx.manager.is_live(&id);
    if is_live {
        crate::transcript_tail::touch(&ctx, &session, provider, &resolved.path);
    }
    let live = match sub {
        None if is_live => {
            crate::transcript_tail::live_page_settled(&id, provider, &resolved.path, before, limit)
                .await
        }
        None => {
            crate::transcript_tail::live_page(&id, provider, &resolved.path, before, limit).await
        }
        Some(_) => None,
    };
    // Paging sizes (serializes) up to a 2 MB page of turns — off the runtime.
    let sub_owned = sub.map(str::to_string);
    let mut t = match live {
        Some(page) => page,
        None => {
            let snapshot = cached_fold(&ctx, provider, &resolved.path, sub).await?;
            crate::offload::blocking(move || {
                page(
                    &snapshot.folded,
                    before,
                    limit,
                    sub_owned.as_deref(),
                    snapshot.subagents.clone(),
                )
            })
            .await
        }
    };
    if t.session_id.is_none() {
        t.session_id = session.provider_session_id.clone();
    }
    Ok(t)
}

/// Passive viewers keep an existing tail warm without starting a process.
#[derive(Default, Deserialize)]
pub struct TranscriptTouchQuery {
    #[serde(default)]
    pub view: bool,
}

/// `POST /sessions/{id}/transcript/touch` — keep the live tail armed for a
/// session whose conversation is open (the tail otherwise stops 5 min after
/// the last `GET …/transcript`; clients ping this every minute while the view
/// is mounted), and count the caller as a VIEWER for the idle-suspend sweep
/// (`VIEW_HOLD`): a chat mounts no terminal, so without this the daemon saw
/// it as unattached and suspended the session under the user. Starts the tail
/// when it is not running. `204`; a session with no transcript yet → `409` so
/// the client keeps retrying the GET instead.
pub async fn touch_transcript(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(q): Query<TranscriptTouchQuery>,
) -> ApiResult<StatusCode> {
    let session = session_gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    // An open chat is a viewer: hold the session against the idle-suspend
    // sweep (it mounts no terminal, so this ping is its only attachment).
    ctx.manager.note_view(&id);
    // Match terminal view-only attach: keepalive/recovery must not activate
    // a dormant process. Omitting `view` preserves the legacy active attach.
    if !q.view
        && !ctx.manager.is_live(&id)
        && session.status == SessionStatus::Reconnectable
        && session.provider_session_id.is_some()
    {
        if let Err(e) = ctx.manager.ensure_live(&id).await {
            tracing::warn!(session = %id, "transcript touch: resume failed: {e}");
        }
    }
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
/// (unarchived) agent session in the workspace armed, so chats you are not
/// looking at right now have their history hot when you open them. The UI
/// arms one session at a time (`POST /sessions/{id}/transcript/touch`); this
/// bulk form is for API and e2e callers. Only the live agent rows are read
/// (`live_agent_sessions`, backend-performance-5-06) — never the workspace's
/// whole history — and only sessions the caller may read (owner / workspace
/// admin / root) are touched; the cap (`MAX_TAILS`) still applies. Returns how
/// many tails were armed.
pub async fn touch_workspace_transcripts(
    AxPath(wid): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<serde_json::Value>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let sessions = ctx
        .manager
        .live_agent_sessions(&wid)
        .await
        .map_err(ApiError)?;
    let mut armed = 0usize;
    for s in sessions {
        // Re-checked: a session may have exited since the live snapshot.
        if s.kind != SessionKind::Agent || !ctx.manager.is_live(&s.id) {
            continue;
        }
        if require_session_owner_or_admin(&ctx, &user, &s)
            .await
            .is_err()
        {
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
    session_image(&ctx, &session, &img_id).await
}

/// One transcript image of an already-authorized session.
pub(crate) async fn session_image(
    ctx: &ServerCtx,
    session: &Session,
    img_id: &str,
) -> ApiResult<Response> {
    let resolved = resolve_transcript(ctx, session)
        .await
        .map_err(|_| ApiError(Error::NotFound("transcript not available".into())))?;
    serve_image(ctx, resolved.provider, &resolved.path, img_id).await
}

/// `GET /sessions/{id}/transcript/tool/{tool_id}` — one `tool_call` block
/// in full (result `text`/`patch` at the 64 KB fold cap, never `elided`).
/// The lazy half of the live push's oversize trim (SA-04): an over-cap
/// `transcript_appended` delta ships each tool result cut to 4 KB with
/// `elided: true`, and the conversation view fetches the rest here when the
/// step is expanded. Served from the running tail's fold when there is one
/// (no disk read), else from the fold cache. Unknown id → 404.
pub async fn transcript_tool(
    AxPath((id, tool_id)): AxPath<(Id, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<otto_transcript::Block>> {
    let session = session_gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let resolved = resolve_transcript(&ctx, &session)
        .await
        .map_err(|_| ApiError(Error::NotFound("transcript not available".into())))?;
    let block = match crate::transcript_tail::live_tool(
        &id,
        resolved.provider,
        &resolved.path,
        tool_id.clone(),
    )
    .await
    {
        Some(block) => block,
        None => {
            let snapshot = cached_fold(&ctx, resolved.provider, &resolved.path, None).await?;
            crate::offload::blocking(move || {
                crate::transcript_tail::find_tool_block(&snapshot.folded, &tool_id)
            })
            .await
        }
    };
    block
        .map(Json)
        .ok_or_else(|| ApiError(Error::NotFound("tool call not found".into())))
}

async fn serve_image(
    ctx: &ServerCtx,
    provider: Provider,
    path: &Path,
    img_id: &str,
) -> ApiResult<Response> {
    let store = image_store(ctx, &store_key(provider, path));
    let (file, mime) = store
        .find(img_id)
        .ok_or_else(|| ApiError(Error::NotFound("image not found".into())))?;
    let bytes = tokio::fs::read(&file)
        .await
        .map_err(|e| ApiError(Error::Internal(format!("read image: {e}"))))?;
    let name = file
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("image")
        .to_string();
    serve_bytes(bytes, mime, &name, true)
}

// ── artifacts ──────────────────────────────────────────────────────────────

/// Fold the whole transcript and return its artifacts, newest first.
async fn session_artifacts(ctx: &ServerCtx, session: &Session) -> ApiResult<Vec<Artifact>> {
    let Ok(resolved) = resolve_transcript(ctx, session).await else {
        return Ok(Vec::new());
    };
    // A live session's tail already holds the folded artifacts: copy them
    // rather than refold a file that grew since the cached fold (which is
    // then never stored as current).
    if let Some(arts) =
        crate::transcript_tail::live_artifacts(&session.id, resolved.provider, &resolved.path).await
    {
        return Ok(newest_first(&arts));
    }
    let snapshot = cached_fold(ctx, resolved.provider, &resolved.path, None).await?;
    Ok(newest_first(&snapshot.folded.artifacts))
}

/// `GET /sessions/{id}/artifacts` — everything the agent produced (design §4.7).
pub async fn list_artifacts(
    AxPath(id): AxPath<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Artifact>>> {
    let session = session_gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let arts = session_artifacts(&ctx, &session).await?;
    // Best-effort work-graph registration, off the request path. The live
    // tail already registers each artifact as it appears, so this usually
    // inserts nothing: one item lookup + one key read, never a per-artifact
    // query pair (the panel re-fetches on every open and WS resync).
    let cx = ctx.clone();
    let s = session.clone();
    let snapshot = arts.clone();
    tokio::spawn(async move { register_work_artifacts(&cx, &s, &snapshot).await });
    Ok(Json(arts))
}

/// The artifact-serving allow-list (design §4.7): the session cwd, the daemon
/// data dir, the per-user temp dir (`std::env::temp_dir()`, NOT `/tmp`) — all
/// canonicalized; plus the fs.rs secret-store deny-list on top.
///
/// `owner`: the root user on their own credential (`ui_bridge::is_human`).
/// They read any file of this machine through `/fs/read` already, so an
/// artifact the agent wrote to `/tmp/report.html` or named `server.pem` is
/// served to them too. The list still binds every other caller — a viewer,
/// an agent session's or MCP credential — since the transcript (and so the
/// path) is agent-controlled.
fn artifact_path_allowed(
    canonical: &Path,
    session_cwd: &str,
    data_dir: &Path,
    owner: bool,
) -> bool {
    if owner {
        return true;
    }
    let roots: Vec<PathBuf> = [Path::new(session_cwd), data_dir, &std::env::temp_dir()]
        .iter()
        .filter_map(|p| p.canonicalize().ok())
        .collect();
    let within = roots
        .iter()
        .any(|r| canonical == r || canonical.starts_with(r));
    within
        && !crate::routes::fs::sandbox::is_denied_dir(canonical)
        && !crate::routes::fs::sandbox::is_denied_file(canonical)
}

/// The canonical path of an artifact file the caller may read (blocking):
/// gone → 404, unresolvable → 400, outside [`artifact_path_allowed`] → 403.
fn confine_artifact(
    target: &Path,
    session_cwd: &str,
    data_dir: &Path,
    owner: bool,
) -> Result<PathBuf, Error> {
    if !target.is_file() {
        return Err(Error::NotFound("artifact file is gone".into()));
    }
    let canonical = target
        .canonicalize()
        .map_err(|e| Error::Invalid(format!("cannot resolve artifact: {e}")))?;
    if !artifact_path_allowed(&canonical, session_cwd, data_dir, owner) {
        return Err(Error::Forbidden("artifact path is not permitted".into()));
    }
    Ok(canonical)
}

/// Read a confined artifact through [`crate::routes::fs::read_at_most`]
/// (`O_NOFOLLOW | O_NONBLOCK`, regular files only, `cap + 1` bytes at most), so
/// a symlink or FIFO swapped in after [`confine_artifact`] is refused and a
/// file grown past the cap is never loaded whole. Blocking.
fn read_artifact_capped(canonical: &Path, cap: u64) -> Result<Vec<u8>, Error> {
    let bytes = crate::routes::fs::read_at_most(canonical, cap as usize + 1)
        .map_err(|e| Error::Internal(format!("read artifact: {e}")))?;
    if bytes.len() as u64 > cap {
        return Err(Error::PayloadTooLarge(format!(
            "artifact is over the {cap}-byte cap"
        )));
    }
    Ok(bytes)
}

/// `GET /sessions/{id}/artifacts/{artifact_id}` — the bytes behind an artifact,
/// by opaque id only. Confinement per §4.7; 25 MB cap.
pub async fn get_artifact(
    AxPath((id, artifact_id)): AxPath<(Id, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Response> {
    let session = session_gate(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let owner = user.is_root && crate::ui_bridge::is_human(&auth);
    let arts = session_artifacts(&ctx, &session).await?;
    let art = arts
        .into_iter()
        .find(|a| a.id == artifact_id)
        .ok_or_else(|| ApiError(Error::NotFound("artifact not found".into())))?;
    let Some(path) = art.path.as_deref() else {
        return Err(ApiError(Error::Invalid(
            "artifact has no local file (a link)".into(),
        )));
    };
    // Images the fold extracted itself live under the data dir already. The
    // path is agent-chosen under the agent's own writable cwd, so the check
    // and the read share ONE blocking hop and the read refuses a symlink or
    // FIFO swapped in after the check and stops at the cap (concurrency-async-6-02).
    let (target, cwd, data_dir) = (
        PathBuf::from(path),
        session.cwd.clone(),
        ctx.data_dir.clone(),
    );
    let bytes = crate::offload::blocking(move || {
        let canonical = confine_artifact(&target, &cwd, &data_dir, owner)?;
        read_artifact_capped(&canonical, ARTIFACT_CAP)
    })
    .await
    .map_err(ApiError)?;
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
    if ctx.manager.is_live(&session.id)
        && !crate::agent_tasks_nudge::agent_running(&ctx, &session).await
    {
        return Err(ApiError(Error::Conflict(
            "agent not running in this terminal".into(),
        )));
    }
    // One safe line each: control characters (incl. a paste terminator) gone.
    let title = crate::agent_tasks_nudge::sanitize_prompt_text(&req.title);
    let title = title.as_str();
    if title.is_empty() {
        return Err(ApiError(Error::Invalid("title is required".into())));
    }
    if title.chars().count() > 500 {
        return Err(ApiError(Error::Invalid(
            "title is too long (500 chars max)".into(),
        )));
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
        .insert_user_task(
            &session.id,
            &session.workspace_id,
            title,
            description.as_deref(),
        )
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
            summary: format!(
                "Board task added: {}",
                otto_transcript::util::clip(title, 120)
            ),
            detail: None,
        })
        .await;
    // Deliver immediately when the session is already idle.
    let cx = ctx.clone();
    let sid = session.id.clone();
    tokio::spawn(async move {
        crate::agent_tasks_nudge::sweep_session(&cx, &sid).await;
    });
    // Re-arm the fast safety tick in case delivery has to defer.
    crate::agent_tasks_nudge::wake();
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
        return Err(ApiError(Error::UnsupportedMedia(
            "only image/* uploads are accepted".into(),
        )));
    }
    // Reject before decoding when the base64 alone already exceeds the cap.
    if req.data_b64.len() > INBOX_CAP / 3 * 4 + 4 {
        return Err(ApiError(Error::PayloadTooLarge(
            "image exceeds 10 MB".into(),
        )));
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(req.data_b64.trim())
        .map_err(|e| ApiError(Error::Invalid(format!("bad base64: {e}"))))?;
    if bytes.is_empty() {
        return Err(ApiError(Error::Invalid("empty image".into())));
    }
    if bytes.len() > INBOX_CAP {
        return Err(ApiError(Error::PayloadTooLarge(
            "image exceeds 10 MB".into(),
        )));
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
    let stem = if stem.is_empty() {
        "image".to_string()
    } else {
        stem
    };
    let dir = ctx
        .data_dir
        .join("sessions")
        .join(&session.id)
        .join("inbox");
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

/// Confine a client-supplied transcript path to the two provider roots
/// (symlink-aware). Returns the canonical path + provider. Under `OTTO_E2E` a
/// checked-in fixture (`…/crates/otto-transcript/fixtures/**.jsonl`) is also
/// accepted.
pub(crate) fn confine_history_path(
    ctx: &ServerCtx,
    raw: &str,
) -> Result<(PathBuf, Provider), ApiError> {
    let candidate = Path::new(raw);
    if !candidate.is_absolute() {
        return Err(ApiError(Error::Invalid(
            "transcript_path must be absolute".into(),
        )));
    }
    if e2e_enabled() && raw.contains("/crates/otto-transcript/fixtures/") {
        if let (Ok(canon), Some(provider)) = (
            candidate.canonicalize(),
            otto_transcript::provider_for_path(candidate),
        ) {
            if canon.is_file() {
                return Ok((canon, provider));
            }
        }
    }
    let roots = transcript_roots(&ctx.data_dir);
    let claude = roots.claude_dirs().map(|root| (root, Provider::Claude));
    for (root, provider) in claude.chain(roots.codex_dirs().map(|root| (root, Provider::Codex))) {
        if let Some(canon) = otto_core::paths::resolves_under(root, candidate) {
            if canon.is_file() && canon.extension().is_some_and(|e| e == "jsonl") {
                return Ok((canon, provider));
            }
        }
    }
    Err(ApiError(Error::Forbidden(
        "transcript_path must be a .jsonl under a Claude projects or Codex sessions root".into(),
    )))
}

/// History's "Resume in Otto" gate for an Otto session row: the CLI left
/// resumable state (a provider session id). Deliberately NOT gated on status —
/// `ensure_live` resumes any agent with a provider id (History's transcripts
/// are claude/codex only, both resume-capable), so an ENDED conversation is
/// resumable; the old `status != Exited` check greyed it out with a false
/// "no resumable state" reason. Reconnectable rows always are. Archived rows
/// stay `resumable` and carry `archived: true` (the UI offers Unarchive).
pub(crate) fn history_resumable(s: &Session) -> bool {
    s.provider_session_id.is_some() || s.status == SessionStatus::Reconnectable
}

/// `GET /workspaces/{wid}/history?q=&provider=&cwd=&status=&before=&limit=` —
/// the legacy array form of History. It delegates to the bounded, paged
/// [`super::history_page::list`] (same scoping, filters and resolution) and
/// returns that page's entries; `before` (an entry's `last_active_at`,
/// exclusive) stands in for the opaque cursor.
pub async fn history(
    AxPath(wid): AxPath<Id>,
    Query(q): Query<HistoryQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<HistoryEntry>>> {
    let limit = q
        .limit
        .unwrap_or(HISTORY_DEFAULT_LIMIT)
        .clamp(1, HISTORY_MAX_LIMIT);
    super::history_page::legacy_entries(&ctx, &user, &wid, q, limit)
        .await
        .map(Json)
}

pub(crate) fn repo_name(cwd: &str) -> Option<String> {
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
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let sub = q.sub.as_deref().filter(|s| !s.is_empty());
    let snapshot = cached_fold(&ctx, provider, &path, sub).await?;
    Ok(Json(page(
        &snapshot.folded,
        before,
        limit,
        sub,
        snapshot.subagents.clone(),
    )))
}

#[derive(Debug, Deserialize)]
pub struct HistoryArtifactsQuery {
    pub path: String,
}

/// `GET /workspaces/{wid}/history/artifacts?path=` — the artifacts an
/// on-disk transcript produced, newest first (the same order as
/// `GET /sessions/{id}/artifacts`). History's "Outputs" used to pull 500
/// full turns (up to the 2 MB page budget) just to pick the artifact chips
/// out of their blocks (SA-07); the fold already collects them. Same path
/// confinement and gate as `history/transcript`.
pub async fn history_artifacts(
    AxPath(wid): AxPath<Id>,
    Query(q): Query<HistoryArtifactsQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<Artifact>>> {
    require_ws_role(&ctx, &user, &wid, WorkspaceRole::Viewer).await?;
    let (path, provider) = confine_history_path(&ctx, &q.path)?;
    history_path_gate(&ctx, &user, &wid, &path).await?;
    let snapshot = cached_fold(&ctx, provider, &path, None).await?;
    Ok(Json(newest_first(&snapshot.folded.artifacts)))
}

/// Artifacts sorted newest first (stable for equal timestamps).
fn newest_first(arts: &[Artifact]) -> Vec<Artifact> {
    let mut arts = arts.to_vec();
    arts.sort_by(|a, b| b.produced_at.cmp(&a.produced_at));
    arts
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
        return Err(ApiError(Error::Invalid(
            "provider does not match the transcript location".into(),
        )));
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
    if let Some(existing) = repo
        .find_by_provider_session(&psid)
        .await
        .map_err(ApiError)?
    {
        if existing.workspace_id != wid {
            return Err(ApiError(Error::Conflict(
                "this conversation already belongs to a session in another workspace".into(),
            )));
        }
        let _ = repo
            .set_transcript_path(&existing.id, &path.to_string_lossy())
            .await;
        return Ok(Json(existing));
    }
    let ws = ctx.workspaces.get(&wid).await.map_err(ApiError)?;
    let title = peek
        .title
        .clone()
        .or_else(|| {
            peek.first_prompt
                .as_deref()
                .map(|p| otto_transcript::util::clip(p, 80))
        })
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
    let _ = repo
        .set_transcript_path(&session.id, &path.to_string_lossy())
        .await;
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
    fn named_accounts_resolve_same_provider_session_id_only_in_their_own_home() {
        let dir = tempfile::tempdir().unwrap();
        for provider in ["claude", "codex"] {
            let mut s = Session {
                id: "s".into(),
                workspace_id: "ws".into(),
                kind: SessionKind::Agent,
                provider: provider.into(),
                title: "fixture".into(),
                status: SessionStatus::Exited,
                cwd: "/fixture".into(),
                provider_session_id: Some("same-session".into()),
                connection_id: None,
                created_by: "u".into(),
                created_at: chrono::Utc::now(),
                last_active_at: chrono::Utc::now(),
                archived: false,
                meta: serde_json::json!({"account_id":"account-a"}),
            };
            let relative = if provider == "claude" {
                "projects/-fixture/same-session.jsonl"
            } else {
                "sessions/2026/09/20/rollout-2026-09-20T10-00-00-same-session.jsonl"
            };
            for account in ["account-a", "account-b"] {
                let path = dir
                    .path()
                    .join("provider-accounts")
                    .join(account)
                    .join(relative);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, account).unwrap();
            }
            for account in ["account-a", "account-b"] {
                s.meta["account_id"] = account.into();
                let (resolved, _) = resolve_transcript_sync(dir.path(), None, &s).unwrap();
                assert_eq!(std::fs::read_to_string(&resolved.path).unwrap(), account);
            }
            s.meta["account_id"] = "missing-profile".into();
            assert!(resolve_transcript_sync(dir.path(), None, &s).is_err());
        }
    }

    #[test]
    fn transcript_roots_honour_env_override() {
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("OTTO_TRANSCRIPT_ROOTS", "/a/claude:/b/codex");
        let roots = transcript_roots(dir.path());
        std::env::remove_var("OTTO_TRANSCRIPT_ROOTS");
        let dirs: Vec<_> = roots.all_dirs().map(Path::to_path_buf).collect();
        assert_eq!(
            dirs,
            vec![PathBuf::from("/a/claude"), PathBuf::from("/b/codex")]
        );
    }

    /// concurrency-async-6-02: the read refuses a symlink swapped in after
    /// the confinement check (the canonical path's last component must stay a
    /// regular file) and never loads more than the cap.
    #[test]
    fn artifact_read_refuses_a_post_check_symlink_swap_and_caps_size() {
        let dir = tempfile::tempdir().unwrap();
        let cwd = dir.path().canonicalize().unwrap();
        let file = cwd.join("report.html");
        std::fs::write(&file, b"<p>ok</p>").unwrap();
        let canonical = confine_artifact(&file, cwd.to_str().unwrap(), &cwd, false).unwrap();
        assert_eq!(
            read_artifact_capped(&canonical, 1024).unwrap(),
            b"<p>ok</p>"
        );
        assert!(matches!(
            read_artifact_capped(&canonical, 4),
            Err(Error::PayloadTooLarge(_))
        ));
        // Swap the checked file for a symlink to a file the check would refuse.
        let secret = dir.path().join("secret.txt");
        std::fs::write(&secret, b"top secret").unwrap();
        std::fs::remove_file(&file).unwrap();
        std::os::unix::fs::symlink(&secret, &file).unwrap();
        let swapped = read_artifact_capped(&canonical, 1024);
        assert!(
            swapped.is_err(),
            "followed a swapped-in symlink: {swapped:?}"
        );
        // A missing file is a 404, not a read error.
        assert!(matches!(
            confine_artifact(&cwd.join("gone.html"), cwd.to_str().unwrap(), &cwd, false),
            Err(Error::NotFound(_))
        ));
    }

    #[test]
    fn artifact_allow_list_is_cwd_data_and_temp_only() {
        let cwd = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let inside = cwd.path().join("out.html");
        std::fs::write(&inside, "x").unwrap();
        let canon = inside.canonicalize().unwrap();
        assert!(artifact_path_allowed(
            &canon,
            &cwd.path().to_string_lossy(),
            data.path(),
            false
        ));
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
        assert!(!artifact_path_allowed(
            &env.canonicalize().unwrap(),
            &cwd.path().to_string_lossy(),
            data.path(),
            false
        ));
        let key = cwd.path().join("id_rsa.pem");
        std::fs::write(&key, "k").unwrap();
        assert!(!artifact_path_allowed(
            &key.canonicalize().unwrap(),
            &cwd.path().to_string_lossy(),
            data.path(),
            false
        ));
    }

    #[test]
    fn the_owner_opens_any_artifact_the_agent_wrote() {
        let cwd = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let key = cwd.path().join("server.pem");
        std::fs::write(&key, "k").unwrap();
        let key = key.canonicalize().unwrap();
        let elsewhere = Path::new("/");
        let cwd = cwd.path().to_string_lossy();
        // A key-like name inside the cwd, and a path outside every root:
        // refused to everyone else, served to the root owner.
        assert!(!artifact_path_allowed(&key, &cwd, data.path(), false));
        assert!(artifact_path_allowed(&key, &cwd, data.path(), true));
        assert!(!artifact_path_allowed(elsewhere, &cwd, data.path(), false));
        assert!(artifact_path_allowed(elsewhere, &cwd, data.path(), true));
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

    /// S20-03: an ENDED (or archived) conversation with a provider id is
    /// resumable — History used to grey it out ("no resumable state"), though
    /// `ensure_live` resumes it fine.
    #[test]
    fn history_resumable_ignores_status_and_archive() {
        let mut s = Session {
            id: "s".into(),
            workspace_id: "ws".into(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "t".into(),
            status: SessionStatus::Exited,
            cwd: "/x".into(),
            provider_session_id: Some("psid".into()),
            connection_id: None,
            created_by: "u".into(),
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            archived: true,
            meta: serde_json::json!({}),
        };
        assert!(
            history_resumable(&s),
            "exited + archived with a provider id"
        );
        s.archived = false;
        assert!(history_resumable(&s), "exited with a provider id");
        s.provider_session_id = None;
        assert!(!history_resumable(&s), "no provider id → nothing to resume");
        s.status = SessionStatus::Reconnectable;
        assert!(history_resumable(&s), "reconnectable always resumes");
    }
}
