//! Room-capability HTTP routes for admitted members (header
//! `x-otto-room-token`, never a URL; owner auth is never consulted):
//!
//! - `GET /room-transcript/{id}` (+ `/images/{img}`): the shared session's
//!   conversation, read-only — what the owner's Chat view reads.
//! - `POST /room-paste/{id}`: an image pasted into a room terminal.
//! - `POST /room-prompt/{id}`: the room composer's prompt / interrupt.
//!
//! The writes are the terminal driver's only (grant `x-otto-grant-epoch`,
//! re-checked by `room_input` at each write, like a typed key). A pasted image
//! is stored on the host (the snip store) and its PATH is typed into the PTY.
use crate::routes::transcript::{session_image, transcript_page, TranscriptQuery};
use crate::{ApiResult, ServerCtx};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::HeaderMap,
    response::Response,
    Json,
};
use otto_core::Error;
use otto_rooms::capability::well_formed;
use otto_transcript::Transcript;

/// Largest pasted image accepted (PNG bytes, after the client re-encode).
pub const MAX_ROOM_PASTE_BYTES: usize = 100 * 1024 * 1024;

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn token(headers: &HeaderMap) -> Result<&str, Error> {
    header(headers, "x-otto-room-token")
        .filter(|t| well_formed(t))
        .ok_or(Error::Unauthorized)
}

/// The admitted member's shared session (any role: the terminal is already
/// visible to viewers, and so is its conversation).
async fn member_session(
    ctx: &ServerCtx,
    id: &str,
    headers: &HeaderMap,
) -> ApiResult<otto_core::domain::Session> {
    let session = ctx.rooms.member_session(id, token(headers)?).await?;
    Ok(ctx.manager.get(&session).await?)
}

pub async fn transcript(
    State(ctx): State<ServerCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<TranscriptQuery>,
) -> ApiResult<Json<Transcript>> {
    // Subagent transcripts stay owner-only (they can carry other workspaces' paths).
    let q = TranscriptQuery { sub: None, ..q };
    let session = member_session(&ctx, &id, &headers).await?;
    Ok(Json(transcript_page(&ctx, &session.id, &session, q).await?))
}

pub async fn transcript_image(
    State(ctx): State<ServerCtx>,
    Path((id, img)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let session = member_session(&ctx, &id, &headers).await?;
    session_image(&ctx, &session, &img).await
}

/// `(room id, session, member, epoch)` of the driving member.
async fn driver(
    ctx: &ServerCtx,
    id: &str,
    headers: &HeaderMap,
) -> ApiResult<(String, String, String, u64)> {
    // Authenticate before validating anything else: no credential → 401.
    let token = token(headers)?;
    let epoch: u64 = header(headers, "x-otto-grant-epoch")
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| Error::Invalid("Missing terminal control grant".into()))?;
    let (room, session, member) = ctx.rooms.driver_session(id, token, epoch).await?;
    Ok((room, session, member, epoch))
}

#[derive(serde::Deserialize)]
pub struct PromptReq {
    text: String,
    /// true: paste + Enter; false: raw bytes (the composer's Esc interrupt).
    #[serde(default)]
    submit: bool,
}

pub async fn prompt(
    State(ctx): State<ServerCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    // The body is parsed only after the room capability is checked.
    let (room_id, session, member, epoch) = driver(&ctx, &id, &headers).await?;
    let req: PromptReq = serde_json::from_slice(&body)
        .map_err(|_| Error::Invalid("Expected {\"text\", \"submit\"}".into()))?;
    if req.text.is_empty() || req.text.len() > 256 * 1024 {
        return Err(Error::Invalid("A prompt must be 1 byte to 256 KB".into()).into());
    }
    if req.submit {
        ctx.manager
            .room_paste_text(&session, &room_id, &member, epoch, &req.text, true)
            .await?;
    } else {
        ctx.manager
            .room_input(&session, &room_id, &member, epoch, req.text.as_bytes())
            .await?;
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

pub async fn paste(
    State(ctx): State<ServerCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> ApiResult<Json<serde_json::Value>> {
    let (room_id, session, member, epoch) = driver(&ctx, &id, &headers).await?;
    if body.is_empty() || body.len() > MAX_ROOM_PASTE_BYTES {
        return Err(Error::Invalid("Paste a PNG image up to 100 MB".into()).into());
    }
    let snip = crate::routes::snips::store_snip(&ctx, &body, "room-paste").await?;
    // Bracketed paste: one literal chunk the TUI will not auto-submit.
    ctx.manager
        .room_paste_text(&session, &room_id, &member, epoch, &snip.path, false)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}
