//! Operator-provided ICE configuration. Only an opaque Keychain reference is
//! persisted; admitted members receive ten-minute TURN REST credentials.
use crate::RoomsCtx;
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
pub fn valid_origin(origin: &str) -> Result<String> {
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
/// The public origin invites point at: the configured `room_public_origin`,
/// else the LAN TLS listener's address when it is serving, else this daemon's
/// (loopback) base URL — validated either way.
pub async fn public_origin<C: RoomsCtx>(ctx: &C) -> Result<String> {
    let configured = ctx.setting("room_public_origin").await?;
    if let Some(origin) = configured.as_ref().and_then(|v| v.as_str()) {
        return valid_origin(origin);
    }
    match ctx.lan_origin() {
        Some(lan) => valid_origin(&format!("{lan}/")),
        None => valid_origin(ctx.base_url()),
    }
}
/// Lowercase-hex SHA-256 of the first certificate (DER) in a PEM file — the
/// network listener's self-signed leaf, pinned by LAN invites.
pub fn pem_sha256(pem: &str) -> Option<String> {
    use base64::Engine;
    use sha2::Digest;
    let body: String = pem
        .lines()
        .skip_while(|l| !l.starts_with("-----BEGIN CERTIFICATE"))
        .skip(1)
        .take_while(|l| !l.starts_with("-----END"))
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body)
        .ok()?;
    Some(
        sha2::Sha256::digest(&der)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}
/// [`public_origin`] plus, when it is the LAN listener (self-signed), that
/// certificate's SHA-256 for the invite to carry (`…/<code>/<sha256>`).
pub async fn invite_origin<C: RoomsCtx>(ctx: &C) -> Result<(String, Option<String>)> {
    let configured = ctx.setting("room_public_origin").await?;
    if configured.as_ref().and_then(|v| v.as_str()).is_none() {
        if let Some(lan) = ctx.lan_origin() {
            return Ok((
                valid_origin(&format!("{lan}/"))?,
                ctx.lan_certificate_sha256()
                    .filter(|fp| fp.len() == 64 && fp.bytes().all(|b| b.is_ascii_hexdigit())),
            ));
        }
    }
    Ok((public_origin(ctx).await?, None))
}
/// The operator's room settings (root-only routes; the handler checks).
pub async fn settings<C: RoomsCtx>(ctx: &C) -> Result<RoomSettings> {
    let ice = ctx.setting("room_ice").await?.unwrap_or_default();
    let origin = ctx
        .setting("room_public_origin")
        .await?
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_else(|| ctx.base_url().to_owned());
    Ok(RoomSettings {
        public_origin: origin,
        stun_urls: urls(ice.get("stun_urls")),
        turn_urls: urls(ice.get("turn_urls")),
        relay_only: ice
            .get("relay_only")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        turn_secret_configured: otto_core::secrets::get_async(ctx.secrets(), SECRET_KEY)
            .await?
            .is_some_and(|s| !s.is_empty()),
    })
}
pub async fn put_settings<C: RoomsCtx>(ctx: &C, req: RoomSettingsReq) -> Result<RoomSettings> {
    let origin = valid_origin(&req.public_origin)?;
    valid_urls(&req.stun_urls, false)?;
    valid_urls(&req.turn_urls, true)?;
    if let Some(secret) = &req.turn_secret {
        if secret.len() > 4096 {
            return Err(Error::Invalid("TURN secret is too long".into()));
        }
        if secret.is_empty() {
            otto_core::secrets::delete_async(ctx.secrets(), SECRET_KEY).await?;
        } else {
            otto_core::secrets::put_async(ctx.secrets(), SECRET_KEY, secret).await?;
        }
    }
    ctx.put_setting("room_public_origin", &serde_json::json!(origin))
        .await?;
    ctx.put_setting("room_ice",&serde_json::json!({"stun_urls":req.stun_urls,"turn_urls":req.turn_urls,"relay_only":req.relay_only,"turn_secret_key":SECRET_KEY})).await?;
    settings(ctx).await
}
/// A `RequestIce` answer: STUN as configured, TURN with a ten-minute REST
/// credential for this member when a shared secret is set.
pub async fn configuration<C: RoomsCtx>(ctx: &C, member: &str) -> Result<RoomEvent> {
    let ice = ctx.setting("room_ice").await?.unwrap_or_default();
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
        otto_core::secrets::get_async(ctx.secrets(), SECRET_KEY).await?
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
