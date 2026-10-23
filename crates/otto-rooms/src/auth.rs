use http::HeaderMap;
use otto_core::{auth::AuthContext, domain::Session, Result};
pub fn owner_auth(auth: &AuthContext) -> Result<()> {
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
pub fn eligible(session: &Session) -> Result<()> {
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
/// The room relays' subprotocol is `otto-room, <token>`; the shared parser
/// also requires a well-formed (43-char) token, as the game relay always did.
pub fn socket_token(headers: &HeaderMap) -> Result<String> {
    crate::capability::socket_token(headers, "otto-room")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_room_subprotocol_pair_is_accepted() {
        let token = "t".repeat(crate::capability::TOKEN_LEN);
        let mut headers = HeaderMap::new();
        assert!(socket_token(&headers).is_err());
        headers.insert("authorization", "Bearer owner-token".parse().unwrap());
        assert!(socket_token(&headers).is_err());
        headers.insert(
            "sec-websocket-protocol",
            format!("otto-room, {token}").parse().unwrap(),
        );
        assert_eq!(socket_token(&headers).unwrap(), token);
        headers.insert(
            "sec-websocket-protocol",
            format!("otto-game, {token}").parse().unwrap(),
        );
        assert!(socket_token(&headers).is_err());
        headers.insert(
            "sec-websocket-protocol",
            format!("otto-room, {token}, extra").parse().unwrap(),
        );
        assert!(socket_token(&headers).is_err());
    }
}
