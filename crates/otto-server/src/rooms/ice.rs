//! Operator-provided ICE configuration. Only an opaque Keychain reference is
//! persisted; admitted members receive ten-minute TURN REST credentials.
use crate::{
    auth::{require_root, CurrentAuthContext},
    ApiResult, ServerCtx,
};
use axum::{extract::State, Json};
use base64::{engine::general_purpose::STANDARD, Engine};
use hmac::{Hmac, KeyInit, Mac};
use otto_core::{api::*, Error, Result};
use sha1::Sha1;
const SECRET_KEY: &str = "otto:rooms:turn-shared-secret";
fn urls(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}
pub(super) fn valid_origin(origin: &str) -> Result<String> {
    let parsed = reqwest::Url::parse(origin)
        .map_err(|_| Error::Invalid("Enter a valid room origin".into()))?;
    let local = matches!(
        parsed.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    );
    if parsed.host_str() == Some("tauri.localhost")
        || (parsed.scheme() != "https" && !(parsed.scheme() == "http" && local))
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || parsed.path() != "/"
    {
        return Err(Error::Invalid(
            "Room origin must be HTTPS (or loopback HTTP), without paths or credentials".into(),
        ));
    }
    Ok(parsed.origin().ascii_serialization())
}
fn valid_urls(items: &[String], turn: bool) -> Result<()> {
    if items.len() > 8 {
        return Err(Error::Invalid(
            "At most eight ICE URLs are supported".into(),
        ));
    }
    for item in items {
        let schemes = if turn {
            ["turn:", "turns:"]
        } else {
            ["stun:", "stuns:"]
        };
        if item.len() > 512
            || !schemes.iter().any(|scheme| item.starts_with(scheme))
            || item.contains('@')
            || item.chars().any(char::is_whitespace)
            || item.split_once(':').is_none_or(|(_, host)| host.is_empty())
        {
            return Err(Error::Invalid(
                "Enter STUN/TURN URLs without embedded credentials".into(),
            ));
        }
    }
    Ok(())
}
pub async fn get_settings(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
) -> ApiResult<Json<RoomSettings>> {
    super::auth::owner_auth(&auth)?;
    require_root(&auth.effective_user)?;
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    let ice = repo.get("room_ice").await?.unwrap_or_default();
    let origin = repo
        .get("room_public_origin")
        .await?
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or(ctx.base_url.clone());
    Ok(Json(RoomSettings {
        public_origin: origin,
        stun_urls: urls(ice.get("stun_urls")),
        turn_urls: urls(ice.get("turn_urls")),
        relay_only: ice
            .get("relay_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        turn_secret_configured: ctx.secrets.get(SECRET_KEY)?.is_some_and(|s| !s.is_empty()),
    }))
}
pub async fn put_settings(
    State(ctx): State<ServerCtx>,
    CurrentAuthContext(auth): CurrentAuthContext,
    Json(req): Json<RoomSettingsReq>,
) -> ApiResult<Json<RoomSettings>> {
    super::auth::owner_auth(&auth)?;
    require_root(&auth.effective_user)?;
    let origin = valid_origin(&req.public_origin)?;
    valid_urls(&req.stun_urls, false)?;
    valid_urls(&req.turn_urls, true)?;
    if let Some(secret) = &req.turn_secret {
        if secret.len() > 4096 {
            return Err(Error::Invalid("TURN secret is too long".into()).into());
        }
        if secret.is_empty() {
            ctx.secrets.delete(SECRET_KEY)?;
        } else {
            ctx.secrets.put(SECRET_KEY, secret)?;
        }
    }
    let repo = otto_state::SettingsRepo::new(ctx.pool.clone());
    repo.put("room_public_origin", &serde_json::json!(origin))
        .await?;
    repo.put("room_ice",&serde_json::json!({"stun_urls":req.stun_urls,"turn_urls":req.turn_urls,"relay_only":req.relay_only,"turn_secret_key":SECRET_KEY})).await?;
    get_settings(State(ctx), CurrentAuthContext(auth)).await
}
pub(super) async fn configuration(ctx: &ServerCtx, member: &str) -> Result<RoomEvent> {
    let ice = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("room_ice")
        .await?
        .unwrap_or_default();
    let stun = urls(ice.get("stun_urls"));
    let turn = urls(ice.get("turn_urls"));
    valid_urls(&stun, false)?;
    valid_urls(&turn, true)?;
    let expires = chrono::Utc::now() + chrono::Duration::minutes(10);
    let mut servers = vec![];
    if !stun.is_empty() {
        servers.push(RoomIceServer {
            urls: stun,
            username: None,
            credential: None,
        });
    }
    let secret = if turn.is_empty() {
        None
    } else {
        ctx.secrets.get(SECRET_KEY)?
    };
    let relay_configured = secret.as_ref().is_some_and(|s| !s.is_empty());
    if let Some(secret) = secret.filter(|s| !s.is_empty()) {
        let username = format!("{}:{member}", expires.timestamp());
        let mut mac = Hmac::<Sha1>::new_from_slice(secret.as_bytes())
            .map_err(|_| Error::Internal("TURN credential generation failed".into()))?;
        mac.update(username.as_bytes());
        let credential = STANDARD.encode(mac.finalize().into_bytes());
        servers.push(RoomIceServer {
            urls: turn,
            username: Some(username),
            credential: Some(credential),
        });
    }
    Ok(RoomEvent::Ice {
        ice_servers: servers,
        relay_configured,
        relay_only: ice
            .get("relay_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        expires_at: expires.to_rfc3339(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_origins_reject_credentials_paths_and_remote_http() {
        assert_eq!(
            valid_origin("https://rooms.example.test/").unwrap(),
            "https://rooms.example.test"
        );
        assert!(valid_origin("http://127.0.0.1:7700").is_ok());
        for url in [
            "http://remote.test",
            "https://tauri.localhost",
            "https://user:password@host.test",
            "https://host.test/path",
            "https://host.test/#secret",
            "file:///tmp/room",
        ] {
            assert!(valid_origin(url).is_err(), "{url}");
        }
    }
    #[test]
    fn ice_urls_never_accept_embedded_credentials() {
        assert!(valid_urls(&["turn:relay.test:3478?transport=udp".into()], true).is_ok());
        assert!(valid_urls(&["turn:user:password@relay.test".into()], true).is_err());
        assert!(valid_urls(&["https://relay.test".into()], true).is_err());
    }
}
