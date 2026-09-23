//! Axum router for workspace channel-integration endpoints.
//! All paths are relative to the `/api/v1` mount point.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use otto_core::api::{Problem, UpsertIntegrationReq};
use otto_core::auth::{AuthUser, RoleChecker};
use otto_core::domain::{Channel, Integration, WorkspaceRole};
use otto_core::secrets::SecretStore;
use otto_core::{Error, Id};
use otto_state::{IntegrationsRepo, WorkspacesRepo};
use serde::Serialize;

use crate::adapter::Adapter;
use crate::seed;
use crate::slack::SlackAdapter;
use crate::telegram::TelegramAdapter;

/// Dependencies the channels router needs from the host application state.
pub trait ChannelsCtx: Clone + Send + Sync + 'static {
    fn integrations(&self) -> &IntegrationsRepo;
    fn secrets(&self) -> &Arc<dyn SecretStore>;
    fn roles(&self) -> &Arc<dyn RoleChecker>;
    fn workspaces(&self) -> &WorkspacesRepo;
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
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
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

/// Build the channels router. Paths are relative to the `/api/v1` mount point.
pub fn router<S: ChannelsCtx>() -> Router<S> {
    Router::new()
        .route("/workspaces/{id}/integrations", get(list_integrations::<S>))
        .route(
            "/workspaces/{id}/integrations/{channel}",
            put(upsert_integration::<S>).delete(delete_integration::<S>),
        )
        .route(
            "/workspaces/{id}/integrations/{channel}/test",
            post(test_integration::<S>),
        )
        .route(
            "/workspaces/{id}/integrations/seed-from-loom",
            post(seed_from_loom::<S>),
        )
}

/// Response body for a test-message call.
#[derive(Serialize)]
pub struct TestMessageResp {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

async fn list_integrations<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<Vec<Integration>>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Viewer)
        .await?;
    let list = s.integrations().list(&ws_id).await?;
    Ok(Json(list))
}

async fn upsert_integration<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((ws_id, channel_str)): Path<(Id, String)>,
    Json(req): Json<UpsertIntegrationReq>,
) -> ApiResult<Json<Integration>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Editor)
        .await?;
    let channel = Channel::parse(&channel_str)
        .ok_or_else(|| Error::Invalid(format!("unknown channel '{channel_str}'")))?;

    // One listener per bot: refuse to ENABLE an integration whose inbound
    // token (Slack app token / Telegram bot token) another workspace's enabled
    // integration already listens with. Checked before any secret is stored so
    // a refused request changes nothing.
    if req.enabled {
        let incoming = match channel {
            Channel::Slack => req.app_token.as_deref(),
            Channel::Telegram => req.bot_token.as_deref(),
            Channel::Webhook => None,
        }
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .or_else(|| {
            listener_token_ref(&ws_id, channel)
                .and_then(|r| s.secrets().get(&r).ok().flatten())
                .filter(|t| !t.is_empty())
        });
        if let Some(token) = incoming {
            ensure_listener_token_unique(&s, &ws_id, channel, &token).await?;
        }
    }

    // Store bot token if provided and non-empty.
    let bot_token_ref = if let Some(tok) = req.bot_token.as_deref().filter(|t| !t.trim().is_empty())
    {
        let r = format!("chan-bot-{}-{}", ws_id, channel.as_str());
        s.secrets().put(&r, tok)?;
        Some(r)
    } else {
        None
    };

    // Store app token if provided and non-empty (Slack-only in practice).
    let app_token_ref = if let Some(tok) = req.app_token.as_deref().filter(|t| !t.trim().is_empty())
    {
        let r = format!("chan-app-{}-{}", ws_id, channel.as_str());
        s.secrets().put(&r, tok)?;
        Some(r)
    } else {
        None
    };

    s.integrations()
        .upsert(
            &ws_id,
            channel,
            req.enabled,
            bot_token_ref,
            app_token_ref,
            &req.allowed_users,
            req.agent_reply,
            &req.reply_instructions,
            &req.channel_id,
            &req.preferred_cli,
        )
        .await?;

    let integration = s
        .integrations()
        .get(&ws_id, channel)
        .await?
        .ok_or_else(|| Error::Internal("integration not found after upsert".into()))?;
    Ok(Json(integration))
}

/// Keychain ref of the token that identifies a channel's inbound listener: the
/// Slack **app** token (Socket Mode connection) or the Telegram bot token
/// (`getUpdates` long-poll). Webhooks have no listener.
pub(crate) fn listener_token_ref(ws_id: &str, channel: Channel) -> Option<String> {
    match channel {
        Channel::Slack => Some(format!("chan-app-{ws_id}-slack")),
        Channel::Telegram => Some(format!("chan-bot-{ws_id}-telegram")),
        Channel::Webhook => None,
    }
}

/// The workspace (other than `ws_id`) among the `enabled` integrations whose
/// `channel` listener uses `token`, reading stored tokens via `secret`.
fn workspace_listening_with(
    enabled: &[Integration],
    ws_id: &str,
    channel: Channel,
    token: &str,
    secret: impl Fn(&str) -> Option<String>,
) -> Option<Id> {
    enabled
        .iter()
        .filter(|o| o.channel == channel && o.workspace_id != ws_id)
        .find(|o| {
            listener_token_ref(&o.workspace_id, channel)
                .and_then(|r| secret(&r))
                .is_some_and(|t| t == token)
        })
        .map(|o| o.workspace_id.clone())
}

/// `Err(Conflict)` when another workspace's ENABLED `channel` integration
/// listens with the same `token`. Two Socket Mode connections on one Slack app
/// get each event delivered to only ONE of them at random, so messages would
/// land in a random workspace (wrong repos/cwd, wrong `allowed_users`) and a
/// thread could alternate between two agents; two Telegram pollers on one bot
/// fight over `getUpdates` (409s) the same way.
async fn ensure_listener_token_unique<S: ChannelsCtx>(
    s: &S,
    ws_id: &Id,
    channel: Channel,
    token: &str,
) -> std::result::Result<(), Error> {
    let enabled = s.integrations().list_all_enabled().await?;
    let clash = workspace_listening_with(&enabled, ws_id, channel, token, |r| {
        s.secrets().get(r).ok().flatten()
    });
    let Some(other_ws) = clash else {
        return Ok(());
    };
    let name = s
        .workspaces()
        .get(&other_ws)
        .await
        .map(|w| w.name)
        .unwrap_or_else(|_| other_ws.clone());
    let what = match channel {
        Channel::Slack => "Slack app token",
        _ => "bot token",
    };
    Err(Error::Conflict(format!(
        "this {what} is already used by the enabled {} integration of workspace \
         '{name}'. One bot can only feed one workspace — the platform delivers each \
         message to just one of its connections, so both workspaces would receive a \
         random share. Disable that integration first, or create a separate {} app/bot \
         for this workspace.",
        channel.as_str(),
        channel.as_str()
    )))
}

async fn delete_integration<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((ws_id, channel_str)): Path<(Id, String)>,
) -> ApiResult<StatusCode> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Editor)
        .await?;
    let channel = Channel::parse(&channel_str)
        .ok_or_else(|| Error::Invalid(format!("unknown channel '{channel_str}'")))?;

    // Delete keychain secrets (ignore errors — they may already be absent).
    if let Ok(Some((bot_ref, app_ref))) = s.integrations().get_refs(&ws_id, channel).await {
        if let Some(r) = bot_ref {
            let _ = s.secrets().delete(&r);
        }
        if let Some(r) = app_ref {
            let _ = s.secrets().delete(&r);
        }
    }

    s.integrations().delete(&ws_id, channel).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/workspaces/{id}/integrations/{channel}/test`
///
/// Sends "Otto is connected ✅" to the integration's configured default chat.
/// Returns `{ ok: true }` on success or `{ ok: false, error: "…" }` on failure.
/// Editor role required (same as upsert).
async fn test_integration<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path((ws_id, channel_str)): Path<(Id, String)>,
) -> ApiResult<Json<TestMessageResp>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Editor)
        .await?;
    let channel = Channel::parse(&channel_str)
        .ok_or_else(|| Error::Invalid(format!("unknown channel '{channel_str}'")))?;

    let integ = s
        .integrations()
        .get(&ws_id, channel)
        .await?
        .ok_or_else(|| Error::NotFound(format!("{channel_str} integration")))?;

    // Webhook has no bot/chat — its "test" probes the configured callback URL
    // (stored in channel_id) by POSTing the connected message to it. Use
    // `send_formatted` because the webhook adapter intentionally no-ops `send`
    // (the streaming activity feed is suppressed for webhooks).
    if channel == Channel::Webhook {
        let url = integ.channel_id.trim();
        if url.is_empty() {
            return Ok(Json(TestMessageResp {
                ok: false,
                error: Some(
                    "No callback URL configured — set one in the webhook settings first.".into(),
                ),
            }));
        }
        let adapter = crate::webhook::WebhookAdapter::new(Some(url.to_string()));
        return match adapter
            .send_formatted("test", None, "Otto is connected \u{2705}")
            .await
        {
            Ok(_) => Ok(Json(TestMessageResp {
                ok: true,
                error: None,
            })),
            Err(e) => Ok(Json(TestMessageResp {
                ok: false,
                error: Some(e.to_string()),
            })),
        };
    }

    if integ.channel_id.trim().is_empty() {
        return Ok(Json(TestMessageResp {
            ok: false,
            error: Some(
                "No default chat ID configured — set one in the integration settings first.".into(),
            ),
        }));
    }

    // Resolve the bot token from the keychain and build an adapter.
    let adapter: Arc<dyn Adapter> = match channel {
        Channel::Slack => {
            let token = s
                .secrets()
                .get(&format!("chan-bot-{ws_id}-slack"))
                .ok()
                .flatten();
            match token {
                Some(t) if !t.is_empty() => Arc::new(SlackAdapter::new(t)),
                _ => {
                    return Ok(Json(TestMessageResp {
                        ok: false,
                        error: Some("Slack bot token not set or empty.".into()),
                    }))
                }
            }
        }
        Channel::Telegram => {
            let token = s
                .secrets()
                .get(&format!("chan-bot-{ws_id}-telegram"))
                .ok()
                .flatten();
            match token {
                Some(t) if !t.is_empty() => Arc::new(TelegramAdapter::new(t)),
                _ => {
                    return Ok(Json(TestMessageResp {
                        ok: false,
                        error: Some("Telegram bot token not set or empty.".into()),
                    }))
                }
            }
        }
        // Handled by the dedicated branch above (callback-URL probe).
        Channel::Webhook => unreachable!("webhook test handled before adapter resolution"),
    };

    let chat = integ.channel_id.trim();
    match adapter.send(chat, None, "Otto is connected \u{2705}").await {
        Ok(_) => Ok(Json(TestMessageResp {
            ok: true,
            error: None,
        })),
        Err(e) => Ok(Json(TestMessageResp {
            ok: false,
            error: Some(e.to_string()),
        })),
    }
}

async fn seed_from_loom<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<Vec<Integration>>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Editor)
        .await?;
    let integrations = seed::seed_from_loom(&s, &ws_id).await?;
    Ok(Json(integrations))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn integ(ws: &str, channel: Channel) -> Integration {
        Integration {
            workspace_id: ws.to_string(),
            channel,
            enabled: true,
            allowed_users: String::new(),
            agent_reply: true,
            reply_instructions: String::new(),
            channel_id: String::new(),
            preferred_cli: String::new(),
            has_bot_token: true,
            has_app_token: channel == Channel::Slack,
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn a_listener_token_can_only_be_enabled_once() {
        let enabled = vec![
            integ("ws_a", Channel::Slack),
            integ("ws_b", Channel::Telegram),
        ];
        let secret = |r: &str| match r {
            "chan-app-ws_a-slack" => Some("xapp-1".to_string()),
            "chan-bot-ws_b-telegram" => Some("123:tg".to_string()),
            _ => None,
        };
        // Same Slack app token as ws_a → clash names ws_a.
        assert_eq!(
            workspace_listening_with(&enabled, "ws_c", Channel::Slack, "xapp-1", secret).as_deref(),
            Some("ws_a")
        );
        // A different app token, or re-saving ws_a itself, is fine.
        assert_eq!(
            workspace_listening_with(&enabled, "ws_c", Channel::Slack, "xapp-2", secret),
            None
        );
        assert_eq!(
            workspace_listening_with(&enabled, "ws_a", Channel::Slack, "xapp-1", secret),
            None
        );
        // Telegram compares the bot token, per channel.
        assert_eq!(
            workspace_listening_with(&enabled, "ws_c", Channel::Telegram, "123:tg", secret)
                .as_deref(),
            Some("ws_b")
        );
        assert_eq!(
            workspace_listening_with(&enabled, "ws_c", Channel::Slack, "123:tg", secret),
            None
        );
    }
}
