//! Inbound webhook channel: `POST /api/v1/webhooks/{workspace_id}`.
//!
//! Public (no Otto session) — authenticated purely by a per-webhook secret key,
//! mirroring the `/workflows/{id}/webhook/{token}` trigger. A valid request
//! triggers an agent session via the channel `Bridge` (reused verbatim); the
//! agent's reply is POSTed to a callback URL by the `WebhookAdapter`.
//!
//! The key is supplied in the `X-Otto-Webhook-Key` header (or
//! `Authorization: Bearer <key>`) and compared against the keychain value in
//! constant time. Processing is spawned in the background so the caller gets a
//! prompt `202 Accepted` and never blocks on agent startup.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use otto_channels::{Adapter, Inbound, WebhookAdapter};
use otto_core::api::Problem;
use otto_core::domain::Channel;
use otto_core::Id;
use serde::{Deserialize, Serialize};

use crate::state::ServerCtx;

/// Inbound webhook payload.
#[derive(Debug, Deserialize)]
pub struct InboundReq {
    /// The prompt for the agent (required, non-empty).
    pub text: String,
    /// Conversation key — reuses the same agent session across calls. When
    /// omitted, falls back to `user`, then to a fresh unique id (so distinct
    /// callers are never silently merged into one session).
    #[serde(default)]
    pub conversation: Option<String>,
    /// Secondary reuse key within a conversation.
    #[serde(default)]
    pub thread: Option<String>,
    /// Caller identity, matched against the integration's `allowed_users` filter.
    #[serde(default)]
    pub user: Option<String>,
    /// Where to POST the agent's reply. Overrides the integration's configured
    /// default callback URL (`channel_id`). Omit for fire-and-forget.
    #[serde(default)]
    pub callback_url: Option<String>,
}

/// `202 Accepted` body.
#[derive(Debug, Serialize)]
pub struct InboundResp {
    pub accepted: bool,
    pub conversation: String,
}

/// Build a `{code,message}` problem response with an explicit status.
fn problem(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(Problem {
            code: code.to_string(),
            message: message.to_string(),
        }),
    )
        .into_response()
}

/// Pull the webhook key from `X-Otto-Webhook-Key`, falling back to a
/// `Authorization: Bearer <key>` header.
fn extract_key(headers: &HeaderMap) -> Option<String> {
    if let Some(k) = headers
        .get("x-otto-webhook-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(k.to_string());
    }
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Constant-time byte equality so key validation doesn't leak via timing.
fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// `POST /api/v1/webhooks/{workspace_id}` — trigger an agent session.
pub async fn inbound(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // 1. The workspace must have an enabled webhook integration.
    let integ = match ctx.integrations_store.get(&ws_id, Channel::Webhook).await {
        Ok(Some(i)) if i.enabled => i,
        Ok(_) => {
            return problem(
                StatusCode::NOT_FOUND,
                "not_found",
                "no enabled webhook for this workspace",
            )
        }
        Err(e) => {
            return problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                &e.to_string(),
            )
        }
    };

    // 2. Validate the key against the keychain value (constant-time).
    let expected = ctx
        .secrets
        .get(&format!("chan-bot-{ws_id}-webhook"))
        .ok()
        .flatten()
        .filter(|k| !k.is_empty());
    let Some(expected) = expected else {
        return problem(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "webhook key not configured",
        );
    };
    match extract_key(&headers) {
        Some(p) if ct_eq(p.as_bytes(), expected.as_bytes()) => {}
        _ => {
            return problem(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "invalid or missing webhook key",
            )
        }
    }

    // 3. Parse + validate the body.
    let req: InboundReq = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            return problem(
                StatusCode::BAD_REQUEST,
                "invalid",
                &format!("invalid JSON body: {e}"),
            )
        }
    };
    let text = req.text.trim().to_string();
    if text.is_empty() {
        return problem(
            StatusCode::BAD_REQUEST,
            "invalid",
            "`text` must not be empty",
        );
    }

    // 4. A root user must exist (the session is created on their behalf).
    let Some(bridge) = ctx.channel_bridge.clone() else {
        return problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
            "no root user yet — finish onboarding first",
        );
    };

    // 5. Resolve the conversation key + reply callback, then dispatch in the
    //    background (never block the caller on agent startup).
    //
    // The conversation key drives session reuse (ConvKey = workspace+chat+thread).
    // Unlike a Slack/Telegram room — a single shared space — distinct webhook
    // callers are usually independent automations, so we must NOT silently fold
    // them into one shared session. Precedence: explicit `conversation` →
    // per-caller `user` → a fresh unique id (each anonymous call is isolated).
    // The resolved key is echoed back so a caller can pass it as `conversation`
    // to deliberately continue the same session.
    let user = req.user.filter(|u| !u.trim().is_empty());
    let conversation = req
        .conversation
        .filter(|c| !c.trim().is_empty())
        .or_else(|| user.clone())
        .unwrap_or_else(|| format!("wh-{}", uuid::Uuid::new_v4()));
    let callback = req
        .callback_url
        .filter(|u| !u.trim().is_empty())
        .or_else(|| {
            let c = integ.channel_id.trim();
            (!c.is_empty()).then(|| c.to_string())
        });
    let msg = Inbound {
        workspace_id: ws_id.clone(),
        chat: conversation.clone(),
        thread: req.thread.filter(|t| !t.trim().is_empty()),
        user: user.unwrap_or_else(|| "webhook".to_string()),
        text,
    };
    let adapter: Arc<dyn Adapter> = Arc::new(WebhookAdapter::new(callback));
    tokio::spawn(async move {
        bridge.handle(&integ, adapter, msg).await;
    });

    (
        StatusCode::ACCEPTED,
        Json(InboundResp {
            accepted: true,
            conversation,
        }),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ct_eq_matches_only_identical_keys() {
        assert!(ct_eq(b"s3cret-key", b"s3cret-key"));
        assert!(!ct_eq(b"s3cret-key", b"s3cret-keX"));
        assert!(!ct_eq(b"short", b"longer-key"));
    }

    #[test]
    fn extract_key_prefers_custom_header_then_bearer() {
        let mut h = HeaderMap::new();
        h.insert("x-otto-webhook-key", "abc123".parse().unwrap());
        assert_eq!(extract_key(&h).as_deref(), Some("abc123"));

        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer tok-987".parse().unwrap(),
        );
        assert_eq!(extract_key(&h).as_deref(), Some("tok-987"));

        assert_eq!(extract_key(&HeaderMap::new()), None);
    }
}
