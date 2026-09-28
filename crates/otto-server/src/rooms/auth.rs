use axum::http::HeaderMap;
use otto_core::{auth::AuthContext, domain::Session, Result};
pub(super) fn owner_auth(auth: &AuthContext) -> Result<()> {
    if auth.scope.is_some()
        || auth.mcp_only
        || auth.mcp_scope.is_some()
        || auth.mcp_internal
        || auth.managed_session_id.is_some()
        || auth.real_user.id != auth.effective_user.id
    {
        return Err(otto_core::Error::Forbidden(
            "Rooms require a normal session-owner credential".into(),
        ));
    }
    Ok(())
}
pub(super) fn eligible(session: &Session) -> Result<()> {
    if session.kind != otto_core::domain::SessionKind::Agent
        || session.connection_id.is_some()
        || ["k8s", "aws", "connection_id"]
            .iter()
            .any(|key| session.meta.get(key).is_some())
        || session.meta.get("source").and_then(|s| s.as_str()) == Some("db_assist")
    {
        return Err(otto_core::Error::Forbidden(
            "Only local agent and shell sessions can host rooms".into(),
        ));
    }
    Ok(())
}
pub(super) fn socket_token(headers: &HeaderMap) -> Result<String> {
    let mut values = headers.get_all("sec-websocket-protocol").iter();
    let value = values
        .next()
        .and_then(|v| v.to_str().ok())
        .ok_or(otto_core::Error::Unauthorized)?;
    if values.next().is_some() {
        return Err(otto_core::Error::Unauthorized);
    }
    let parts: Vec<_> = value.split(',').map(str::trim).collect();
    if parts.len() != 2 || parts[0] != "otto-room" || parts[1].is_empty() {
        return Err(otto_core::Error::Unauthorized);
    }
    Ok(parts[1].into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_room_subprotocol_pair_is_accepted() {
        let mut headers = HeaderMap::new();
        assert!(socket_token(&headers).is_err());
        headers.insert("authorization", "Bearer owner-token".parse().unwrap());
        assert!(socket_token(&headers).is_err());
        headers.insert("sec-websocket-protocol", "otto-room, abc".parse().unwrap());
        assert_eq!(socket_token(&headers).unwrap(), "abc");
        headers.insert(
            "sec-websocket-protocol",
            "otto-room, abc, extra".parse().unwrap(),
        );
        assert!(socket_token(&headers).is_err());
    }
}
