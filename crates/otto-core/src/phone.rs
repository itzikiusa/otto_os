//! Phone access DTOs (docs/contracts/api.md — *Phone access*; mirrored in
//! `ui/src/lib/api/types.ts`).

use serde::{Deserialize, Serialize};

use crate::Id;

/// A paired phone (its device token's metadata; never the token).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneDevice {
    pub id: Id,
    pub name: String,
    pub user_agent: Option<String>,
    pub created_at: String,
    pub last_seen_at: String,
    pub last_ip: Option<String>,
    /// Sliding idle expiry (30 days after last use, capped at `expires_at`).
    pub idle_expires_at: String,
    /// Absolute expiry (180 days after pairing).
    pub expires_at: String,
    /// True when this row is the calling credential.
    #[serde(default)]
    pub current: bool,
}

/// Lifecycle of one QR pairing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairingStatus {
    /// The QR is showing; no phone has opened it.
    Waiting,
    /// A phone opened it; both screens show the confirmation code.
    Claimed,
    /// The person on the Mac approved; the phone has not collected its token.
    Approved,
    Denied,
    /// The phone collected its device token (terminal).
    Consumed,
    Expired,
}

impl PairingStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Waiting => "waiting",
            Self::Claimed => "claimed",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Consumed => "consumed",
            Self::Expired => "expired",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "waiting" => Self::Waiting,
            "claimed" => Self::Claimed,
            "approved" => Self::Approved,
            "denied" => Self::Denied,
            "consumed" => Self::Consumed,
            "expired" => Self::Expired,
            _ => return None,
        })
    }
}

/// The Mac-side view of a pairing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingView {
    pub id: Id,
    pub status: PairingStatus,
    pub expires_at: String,
    pub device_label: Option<String>,
    pub peer_ip: Option<String>,
    /// The 6-digit code both screens show, once a phone has claimed.
    pub confirm_code: Option<String>,
}

/// `POST /phone/pairings` → the QR payload (the raw code appears only here).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingCreated {
    pub id: Id,
    pub code: String,
    pub url: String,
    pub expires_at: String,
}

/// `POST /phone/pair/claim` → what the phone keeps to poll with.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairClaimResult {
    pub pairing_id: Id,
    pub poll_secret: String,
    pub confirm_code: String,
    pub expires_at: String,
}

/// `POST /phone/pair/poll`. `token` is present exactly once, on approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairPollResult {
    /// `pending` | `approved` | `denied` | `expired`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<PhoneDevice>,
}

/// One device on the tailnet (`tailscale status --json` → `Peer`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TailnetDevice {
    pub name: String,
    pub dns_name: String,
    pub os: String,
    pub online: bool,
    pub ips: Vec<String>,
}

/// How Otto is served on the tailnet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneServing {
    pub url: String,
    pub https: bool,
    pub port: u16,
}

/// `GET /phone/status`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhoneAccessStatus {
    pub installed: bool,
    pub cli_path: Option<String>,
    pub backend_state: Option<String>,
    pub logged_in: bool,
    pub tailnet: Option<String>,
    pub magic_dns: bool,
    pub self_name: Option<String>,
    pub self_ips: Vec<String>,
    pub devices: Vec<TailnetDevice>,
    pub serving: Option<PhoneServing>,
    /// `tailscale serve` has entries that are not Otto's (left untouched).
    pub other_serves: bool,
    pub note: Option<String>,
    pub error: Option<String>,
    /// Whether the caller may turn serving on/off (root, on the Mac).
    pub can_manage: bool,
}

/// The in-app routes a pairing / continue-on-phone link may land on.
/// `''`, `home`, `agents`, `threads`, `agents/<id>`, `threads/<id>` with an id
/// of 1–64 `[A-Za-z0-9_-]`. Anything else is refused (`None`) — a QR must not
/// carry an arbitrary route (or an off-app URL) through the pairing.
pub fn validate_next_route(raw: &str) -> Option<String> {
    let r = raw.trim().trim_start_matches('#').trim_start_matches('/');
    let ok_id = |id: &str| {
        !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    let valid = match r.split_once('/') {
        None => matches!(r, "" | "home" | "agents" | "threads"),
        Some((m, id)) => matches!(m, "agents" | "threads") && ok_id(id),
    };
    valid.then(|| r.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_route_allow_list() {
        assert_eq!(
            validate_next_route("agents/01HXY_z-9").as_deref(),
            Some("agents/01HXY_z-9")
        );
        assert_eq!(
            validate_next_route("#/threads/abc").as_deref(),
            Some("threads/abc")
        );
        assert_eq!(validate_next_route("home").as_deref(), Some("home"));
        assert_eq!(validate_next_route("").as_deref(), Some(""));
        for bad in [
            "settings/phone",
            "agents/a/b",
            "agents/",
            "https://evil.example",
            "//evil.example",
            "agents/../settings",
            "threads/a%2Fb",
            "s/abc/token",
            "pair/x",
        ] {
            assert_eq!(validate_next_route(bad), None, "{bad}");
        }
        assert_eq!(
            validate_next_route(&format!("agents/{}", "a".repeat(65))),
            None
        );
    }

    #[test]
    fn pairing_status_round_trips() {
        for s in [
            "waiting", "claimed", "approved", "denied", "consumed", "expired",
        ] {
            assert_eq!(PairingStatus::parse(s).unwrap().as_str(), s);
        }
        assert!(PairingStatus::parse("bogus").is_none());
    }
}
