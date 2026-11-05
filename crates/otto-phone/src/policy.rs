//! Route policy for Phone access, consumed by otto-server's feature guard.
//!
//! * [`exempt`] — the `/phone/*` routes skip the feature-grant axis: each one is
//!   self-owned (your own phones) and its handler enforces who may call it
//!   (a person on the Mac; root for serve on/off).
//! * [`device_gate`] — what a paired phone's **device token** may NOT do,
//!   whatever its owner could: mint / list / reveal credentials, administer
//!   Otto, manage Phone access or approve a pairing. Human approval gates
//!   (an agent's permission prompt, a workflow / DB-change approval) stay
//!   allowed — answering those from the phone is the point of the feature.

use axum::http::Method;

/// otto-server's `route_class` for the route, reduced to what matters here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sensitivity {
    /// No credential class (or `Outward`).
    Normal,
    /// Identity / policy / daemon administration or a human approval gate.
    Admin,
    /// Plaintext credentials or credential minting.
    Secret,
}

/// The Phone access routes the feature axis does not gate (prefix-stripped).
pub fn exempt(p: &str) -> bool {
    p == "/phone" || p.starts_with("/phone/")
}

/// Human approval gates a phone may answer (an Admin-class write otherwise).
fn approval_gate(p: &str) -> bool {
    p == "/runtime-sessions/{id}/requests/{request_id}"
        || p == "/mcp/approvals/{id}/decide"
        || p == "/workflow-runs/{id}/approve"
        || p == "/database-changes/{id}/approve"
        || matches!(p.rsplit('/').next(), Some("approve" | "reject" | "decide"))
}

/// `Err(reason)` when a device token may not use `(method, template)`.
pub fn device_gate(
    _method: &Method,
    template: &str,
    class: Sensitivity,
) -> Result<(), &'static str> {
    let p = template.strip_prefix("/api/v1").unwrap_or(template);
    // Phone access itself: only "who am I" / "unpair me" from a phone.
    if exempt(p) {
        return if p == "/phone/device/me" {
            Ok(())
        } else {
            Err("a paired phone cannot manage Phone access — use Otto on your Mac")
        };
    }
    // API tokens, MCP tokens and share links: not even a listing.
    if p == "/auth/tokens"
        || p.starts_with("/auth/tokens/")
        || p.starts_with("/mcp/tokens")
        || p.starts_with("/auth/shares")
    {
        return Err("a paired phone cannot see or mint credentials — use Otto on your Mac");
    }
    match class {
        Sensitivity::Normal => Ok(()),
        Sensitivity::Secret => {
            Err("a paired phone cannot reveal or mint credentials — use Otto on your Mac")
        }
        Sensitivity::Admin if approval_gate(p) => Ok(()),
        Sensitivity::Admin => Err("a paired phone cannot administer Otto — use Otto on your Mac"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_cannot_touch_credentials_or_phone_admin() {
        let post = Method::POST;
        let get = Method::GET;
        for (m, p) in [
            (&get, "/api/v1/auth/tokens"),
            (&post, "/api/v1/auth/tokens"),
            (&get, "/api/v1/mcp/tokens"),
            (&get, "/api/v1/auth/shares"),
            (&post, "/api/v1/phone/pairings"),
            (&post, "/api/v1/phone/pairings/{id}/approve"),
            (&post, "/api/v1/phone/serve"),
            (&get, "/api/v1/phone/devices"),
            (&get, "/api/v1/phone/status"),
        ] {
            assert!(device_gate(m, p, Sensitivity::Normal).is_err(), "{m} {p}");
        }
        assert!(device_gate(&post, "/api/v1/sessions/{id}/share", Sensitivity::Secret).is_err());
        assert!(device_gate(&post, "/api/v1/users", Sensitivity::Admin).is_err());
        assert!(device_gate(&Method::PATCH, "/api/v1/settings", Sensitivity::Admin).is_err());
    }

    #[test]
    fn device_keeps_normal_use_and_approval_gates() {
        assert!(device_gate(&Method::GET, "/api/v1/phone/device/me", Sensitivity::Normal).is_ok());
        assert!(device_gate(
            &Method::DELETE,
            "/api/v1/phone/device/me",
            Sensitivity::Normal
        )
        .is_ok());
        assert!(device_gate(
            &Method::POST,
            "/api/v1/sessions/{id}/input",
            Sensitivity::Normal
        )
        .is_ok());
        assert!(device_gate(
            &Method::POST,
            "/api/v1/runtime-sessions/{id}/prompt",
            Sensitivity::Normal
        )
        .is_ok());
        assert!(device_gate(
            &Method::POST,
            "/api/v1/runtime-sessions/{id}/requests/{request_id}",
            Sensitivity::Admin
        )
        .is_ok());
        assert!(device_gate(
            &Method::POST,
            "/api/v1/repos/{id}/push",
            Sensitivity::Normal
        )
        .is_ok());
    }

    #[test]
    fn exempt_is_only_phone_routes() {
        assert!(exempt("/phone/status"));
        assert!(exempt("/phone/pair/claim"));
        assert!(!exempt("/phones"));
        assert!(!exempt("/sessions"));
    }
}
