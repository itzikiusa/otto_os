//! Wire + settings types of the remote live browser (mirrored in
//! `ui/src/lib/api/types.ts` — `BrowserLive*`, `BrowserChromeBuild`, …; the
//! contract is `docs/contracts/api.md` "Browser — remote live view").

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Settings-KV key the daemon-wide [`LiveSettings`] are stored under.
pub const SETTINGS_KEY: &str = "browser_live";

/// At most this many Chromium processes at once (the shared ephemeral one
/// plus one per open persistent profile).
pub const MAX_PROCESSES: usize = 4;

/// A Chromium process with no sessions exits after this long.
pub const PROCESS_IDLE_EXIT_SECS: u64 = 60;

/// Viewers (WS attachments) per session.
pub const MAX_VIEWERS: usize = 8;

/// The profile name meaning "own throwaway browser context, wiped on close".
pub const EPHEMERAL_PROFILE: &str = "ephemeral";

/// Which Chrome for Testing binary runs the remote engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ChromeBuild {
    /// Full Chrome for Testing in new headless mode (default).
    Chrome,
    /// The lighter headless-only shell.
    ChromeHeadlessShell,
}

impl ChromeBuild {
    pub const ALL: [ChromeBuild; 2] = [ChromeBuild::Chrome, ChromeBuild::ChromeHeadlessShell];

    pub fn as_str(self) -> &'static str {
        match self {
            ChromeBuild::Chrome => "chrome",
            ChromeBuild::ChromeHeadlessShell => "chrome-headless-shell",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "chrome" => Some(ChromeBuild::Chrome),
            "chrome-headless-shell" => Some(ChromeBuild::ChromeHeadlessShell),
            _ => None,
        }
    }

    /// Only the full build can open a visible window.
    pub fn supports_headed(self) -> bool {
        matches!(self, ChromeBuild::Chrome)
    }
}

/// What happens to a page-initiated download.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadPolicy {
    Block,
    Quarantine,
}

/// Daemon-wide remote-live settings (`PUT /browser/live/settings`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveSettings {
    pub build: ChromeBuild,
    /// "Show the window on this Mac". Requires [`ChromeBuild::Chrome`].
    pub headed: bool,
    pub max_sessions: u32,
    pub idle_timeout_secs: u64,
    pub downloads: DownloadPolicy,
}

impl Default for LiveSettings {
    fn default() -> Self {
        Self {
            build: ChromeBuild::Chrome,
            headed: false,
            max_sessions: 6,
            idle_timeout_secs: 900,
            downloads: DownloadPolicy::Quarantine,
        }
    }
}

/// A partial [`LiveSettings`] (every field optional) — the PUT body.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct LiveSettingsPatch {
    pub build: Option<ChromeBuild>,
    pub headed: Option<bool>,
    pub max_sessions: Option<u32>,
    pub idle_timeout_secs: Option<u64>,
    pub downloads: Option<DownloadPolicy>,
}

impl LiveSettings {
    /// Range + combination rules (the PUT's 400s).
    pub fn validate(&self) -> Result<(), String> {
        if self.headed && !self.build.supports_headed() {
            return Err(
                "headed mode needs the full \"chrome\" build, not chrome-headless-shell".into(),
            );
        }
        if !(1..=16).contains(&self.max_sessions) {
            return Err("max_sessions must be between 1 and 16".into());
        }
        if !(60..=86_400).contains(&self.idle_timeout_secs) {
            return Err("idle_timeout_secs must be between 60 and 86400".into());
        }
        Ok(())
    }

    /// Settings as stored, falling back to the defaults for an absent,
    /// malformed or invalid value (a hand-edited KV row must never wedge the
    /// engine).
    pub fn from_stored(value: Option<&Value>) -> Self {
        let Some(v) = value else {
            return Self::default();
        };
        let patch: LiveSettingsPatch = serde_json::from_value(v.clone()).unwrap_or_default();
        Self::default().patched(&patch).unwrap_or_default()
    }

    /// Apply `patch` on top of `self` and validate the result.
    pub fn patched(&self, patch: &LiveSettingsPatch) -> Result<Self, String> {
        let next = Self {
            build: patch.build.unwrap_or(self.build),
            headed: patch.headed.unwrap_or(self.headed),
            max_sessions: patch.max_sessions.unwrap_or(self.max_sessions),
            idle_timeout_secs: patch.idle_timeout_secs.unwrap_or(self.idle_timeout_secs),
            downloads: patch.downloads.unwrap_or(self.downloads),
        };
        next.validate()?;
        Ok(next)
    }
}

/// Viewport in CSS px.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    #[serde(default = "default_dsf")]
    pub device_scale_factor: f64,
}

fn default_dsf() -> f64 {
    1.0
}

impl Default for Viewport {
    fn default() -> Self {
        Self {
            width: 1280,
            height: 800,
            device_scale_factor: 1.0,
        }
    }
}

impl Viewport {
    /// Clamp into the supported range (200..=3840 × 200..=2160, dsf 1..=3).
    pub fn clamped(self) -> Self {
        let dsf = if self.device_scale_factor.is_finite() {
            self.device_scale_factor.clamp(1.0, 3.0)
        } else {
            1.0
        };
        Self {
            width: self.width.clamp(200, 3840),
            height: self.height.clamp(200, 2160),
            device_scale_factor: dsf,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Starting,
    Ready,
    Crashed,
    Closed,
}

impl SessionState {
    pub fn as_str(self) -> &'static str {
        match self {
            SessionState::Starting => "starting",
            SessionState::Ready => "ready",
            SessionState::Crashed => "crashed",
            SessionState::Closed => "closed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControllerKind {
    None,
    Human,
    Agent,
}

/// `BrowserLiveSession` on the wire.
#[derive(Debug, Clone, Serialize)]
pub struct LiveSessionInfo {
    pub tab_id: String,
    pub workspace_id: String,
    pub owner_id: String,
    pub engine: &'static str,
    pub build: ChromeBuild,
    pub version: String,
    pub profile: String,
    pub headed: bool,
    pub state: SessionState,
    pub url: String,
    pub title: String,
    pub loading: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub viewport: Viewport,
    pub controller: ControllerKind,
    pub controller_user_id: Option<String>,
    pub viewers: u32,
    pub created_at: String,
    pub last_activity_at: String,
}

/// Validate a profile name: `ephemeral`, or `[a-z0-9_-]{1,40}`.
pub fn valid_profile_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 40
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
}

/// A path segment derived from an id we don't control the shape of (workspace
/// / user ids are ULIDs today, but a path must never be built from anything
/// that could hold `/` or `..`).
pub fn safe_segment(raw: &str) -> String {
    let s: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(80)
        .collect();
    if s.is_empty() || s.chars().all(|c| c == '_') {
        "_".to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn build_names_round_trip_on_the_wire() {
        for b in ChromeBuild::ALL {
            let v = serde_json::to_value(b).unwrap();
            assert_eq!(v, json!(b.as_str()));
            assert_eq!(ChromeBuild::parse(b.as_str()), Some(b));
        }
        assert_eq!(ChromeBuild::parse("chromium"), None);
    }

    #[test]
    fn settings_default_and_validation() {
        let d = LiveSettings::default();
        assert_eq!(d.build, ChromeBuild::Chrome);
        assert!(!d.headed);
        assert!(d.validate().is_ok());
        // Headed needs the full build.
        let bad = d.patched(&LiveSettingsPatch {
            build: Some(ChromeBuild::ChromeHeadlessShell),
            headed: Some(true),
            ..Default::default()
        });
        assert!(bad.is_err());
        let ok = d
            .patched(&LiveSettingsPatch {
                headed: Some(true),
                ..Default::default()
            })
            .unwrap();
        assert!(ok.headed);
        assert!(d
            .patched(&LiveSettingsPatch {
                max_sessions: Some(0),
                ..Default::default()
            })
            .is_err());
        assert!(d
            .patched(&LiveSettingsPatch {
                idle_timeout_secs: Some(5),
                ..Default::default()
            })
            .is_err());
    }

    #[test]
    fn stored_settings_fall_back_to_defaults() {
        assert_eq!(LiveSettings::from_stored(None), LiveSettings::default());
        assert_eq!(
            LiveSettings::from_stored(Some(&json!("garbage"))),
            LiveSettings::default()
        );
        // An invalid combination is not honoured half-way.
        assert_eq!(
            LiveSettings::from_stored(Some(
                &json!({"build": "chrome-headless-shell", "headed": true})
            )),
            LiveSettings::default()
        );
        let s = LiveSettings::from_stored(Some(&json!({"build": "chrome-headless-shell"})));
        assert_eq!(s.build, ChromeBuild::ChromeHeadlessShell);
    }

    #[test]
    fn viewport_is_clamped() {
        let v = Viewport {
            width: 10,
            height: 99_999,
            device_scale_factor: f64::NAN,
        }
        .clamped();
        assert_eq!((v.width, v.height), (200, 2160));
        assert_eq!(v.device_scale_factor, 1.0);
    }

    #[test]
    fn profile_names_and_segments() {
        assert!(valid_profile_name("ephemeral"));
        assert!(valid_profile_name("work-1_a"));
        assert!(!valid_profile_name(""));
        assert!(!valid_profile_name("../x"));
        assert!(!valid_profile_name("Work"));
        assert!(!valid_profile_name(&"a".repeat(41)));
        assert_eq!(safe_segment("../../etc"), "______etc");
        assert_eq!(safe_segment(""), "_");
        assert_eq!(safe_segment("01HX-ab_c"), "01HX-ab_c");
    }
}
