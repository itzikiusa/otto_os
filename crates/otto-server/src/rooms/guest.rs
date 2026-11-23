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
use otto_rooms::terminal::GuestRateTable;
use otto_transcript::{Block, Transcript, Turn};

/// Largest pasted image accepted (PNG bytes, after the client re-encode).
pub const MAX_ROOM_PASTE_BYTES: usize = 20 * 1024 * 1024;
/// Largest raw (non-submit) PTY byte write via HTTP (matches WS `Frame::Input`).
const MAX_ROOM_RAW_PROMPT_BYTES: usize = 32 * 1024;

static PASTE_RATES: GuestRateTable = GuestRateTable::new();
static PROMPT_RATES: GuestRateTable = GuestRateTable::new();

/// True when `img` is referenced by at least one `Block::Image` in `turns`.
fn image_in_turns(turns: &[Turn], img: &str) -> bool {
    turns.iter().any(|t| {
        t.blocks
            .iter()
            .any(|b| matches!(b, Block::Image { id, .. } if id == img))
    })
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn token(headers: &HeaderMap) -> Result<&str, Error> {
    header(headers, "x-otto-room-token")
        .filter(|t| well_formed(t))
        .ok_or(Error::Unauthorized)
}

pub async fn transcript(
    State(ctx): State<ServerCtx>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(q): Query<TranscriptQuery>,
) -> ApiResult<Json<Transcript>> {
    // Subagent transcripts stay owner-only (they can carry other workspaces' paths).
    let q = TranscriptQuery { sub: None, ..q };
    let (sid, since) = ctx
        .rooms
        .member_session_since(&id, token(&headers)?)
        .await?;
    let session = ctx.manager.get(&sid).await?;
    let mut page = transcript_page(&ctx, &session.id, &session, q).await?;
    // Only the room's own lifetime; nothing to page back to before it.
    page.has_earlier &= !otto_rooms::retain_since(&mut page.turns, since, |t| t.ts.as_deref());
    Ok(Json(page))
}

pub async fn transcript_image(
    State(ctx): State<ServerCtx>,
    Path((id, img)): Path<(String, String)>,
    headers: HeaderMap,
) -> ApiResult<Response> {
    let (sid, since) = ctx
        .rooms
        .member_session_since(&id, token(&headers)?)
        .await?;
    let session = ctx.manager.get(&sid).await?;
    // Only serve images that belong to turns created during this room's
    // lifetime (matching `transcript`'s `retain_since` boundary).
    let mut page = transcript_page(
        &ctx,
        &session.id,
        &session,
        TranscriptQuery {
            before: None,
            limit: Some(500),
            sub: None,
        },
    )
    .await?;
    otto_rooms::retain_since(&mut page.turns, since, |t| t.ts.as_deref());
    if !image_in_turns(&page.turns, &img) {
        return Err(Error::NotFound("image not found in room transcript".into()).into());
    }
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
    PROMPT_RATES.take(&room_id, &member, 0.5, 10.0)?;
    let req: PromptReq = serde_json::from_slice(&body)
        .map_err(|_| Error::Invalid("Expected {\"text\", \"submit\"}".into()))?;
    let max_bytes = if req.submit {
        256 * 1024
    } else {
        MAX_ROOM_RAW_PROMPT_BYTES
    };
    if req.text.is_empty() || req.text.len() > max_bytes {
        return Err(Error::Invalid(format!(
            "A prompt must be 1 byte to {} KB",
            max_bytes / 1024
        ))
        .into());
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
    PASTE_RATES.take(&room_id, &member, 0.1, 4.0)?;
    if body.is_empty() || body.len() > MAX_ROOM_PASTE_BYTES {
        return Err(Error::Invalid("Paste a PNG image up to 20 MB".into()).into());
    }
    // Verify live PTY driver authority BEFORE persisting the snip to host disk.
    ctx.manager
        .check_room_driver(&session, &room_id, &member, epoch)
        .await?;
    let snip = crate::routes::snips::store_snip(&ctx, &body, "room-paste").await?;
    // Bracketed paste: one literal chunk the TUI will not auto-submit.
    ctx.manager
        .room_paste_text(&session, &room_id, &member, epoch, &snip.path, false)
        .await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_transcript::{Role, Turn};

    #[test]
    fn image_in_turns_blocks_pre_room_images() {
        let mut turns = vec![
            Turn {
                id: "pre".into(),
                role: Role::User,
                ts: Some("2026-01-01T00:00:00Z".into()),
                blocks: vec![Block::Image {
                    id: "secret-before-room".into(),
                    media_type: "image/png".into(),
                    alt: None,
                }],
                system: vec![],
                usage: None,
                model: None,
                duration_ms: None,
                reasoning_steps: 0,
            },
            Turn {
                id: "in-room".into(),
                role: Role::Assistant,
                ts: Some("2026-01-01T00:05:00Z".into()),
                blocks: vec![Block::Image {
                    id: "room-img".into(),
                    media_type: "image/png".into(),
                    alt: None,
                }],
                system: vec![],
                usage: None,
                model: None,
                duration_ms: None,
                reasoning_steps: 0,
            },
        ];
        let since = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:03:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        otto_rooms::retain_since(&mut turns, since, |t| t.ts.as_deref());
        assert!(!image_in_turns(&turns, "secret-before-room"));
        assert!(image_in_turns(&turns, "room-img"));
    }
}
