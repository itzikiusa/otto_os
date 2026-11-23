//! Device emulation for the remote live browser: mobile viewport metrics,
//! screen orientation, touch emulation, user-agent override, and
//! `prefers-color-scheme` media emulation. Pure CDP parameter builders live
//! here so they can be unit-tested without launching Chromium.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::types::Viewport;

/// Longest User-Agent override accepted (chars).
pub const MAX_USER_AGENT_CHARS: usize = 512;
/// Longest preset identifier accepted (chars).
pub const MAX_PRESET_ID_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorScheme {
    Light,
    Dark,
}

impl ColorScheme {
    pub fn as_cdp(self) -> &'static str {
        match self {
            ColorScheme::Light => "light",
            ColorScheme::Dark => "dark",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    Portrait,
    Landscape,
}

impl Orientation {
    pub fn cdp_type(self) -> &'static str {
        match self {
            Orientation::Portrait => "portraitPrimary",
            Orientation::Landscape => "landscapePrimary",
        }
    }

    pub fn cdp_angle(self) -> u32 {
        match self {
            Orientation::Portrait => 0,
            Orientation::Landscape => 90,
        }
    }
}

/// Active device-emulation profile for a [`super::session::LiveSession`].
/// `EmulationConfig::default()` represents standard desktop responsive viewing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmulationConfig {
    /// Selected preset id (e.g. `"iphone-16-pro"`, `"pixel-9"`, `"custom"`),
    /// or `None` when unemulated / responsive panel fit.
    #[serde(default)]
    pub preset_id: Option<String>,
    /// Whether Chromium emulates a mobile device (meta viewport, text
    /// autosizing, overlay scrollbars).
    #[serde(default)]
    pub mobile: bool,
    /// Whether CDP touch emulation is enabled.
    #[serde(default)]
    pub has_touch: bool,
    /// Optional User-Agent override (`None` resets to Chromium's default UA).
    #[serde(default)]
    pub user_agent: Option<String>,
    /// Optional `prefers-color-scheme` override (`None` clears media override).
    #[serde(default)]
    pub color_scheme: Option<ColorScheme>,
    /// Screen orientation reported via `Emulation.setDeviceMetricsOverride`.
    #[serde(default)]
    pub orientation: Option<Orientation>,
}

impl EmulationConfig {
    /// Normalize and bound untrusted fields from a client/API request.
    pub fn sanitized(self) -> Self {
        let preset_id = self
            .preset_id
            .map(|s| {
                s.chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                    .take(MAX_PRESET_ID_CHARS)
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty());
        let user_agent = self
            .user_agent
            .map(|s| {
                s.chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_USER_AGENT_CHARS)
                    .collect::<String>()
                    .trim()
                    .to_string()
            })
            .filter(|s| !s.is_empty());
        Self {
            preset_id,
            mobile: self.mobile,
            has_touch: self.has_touch,
            user_agent,
            color_scheme: self.color_scheme,
            orientation: self.orientation,
        }
    }
}

/// `Emulation.setDeviceMetricsOverride` params for `vp` + `emu`.
pub fn device_metrics_params(vp: Viewport, emu: &EmulationConfig) -> Value {
    let mut p = json!({
        "width": vp.width,
        "height": vp.height,
        "deviceScaleFactor": vp.device_scale_factor,
        "mobile": emu.mobile,
    });
    let orientation = emu.orientation.or(if emu.mobile {
        Some(if vp.width > vp.height {
            Orientation::Landscape
        } else {
            Orientation::Portrait
        })
    } else {
        None
    });
    if let Some(o) = orientation {
        p["screenOrientation"] = json!({
            "type": o.cdp_type(),
            "angle": o.cdp_angle(),
        });
    }
    p
}

/// `Emulation.setTouchEmulationEnabled` params.
pub fn touch_emulation_params(emu: &EmulationConfig) -> Value {
    json!({
        "enabled": emu.has_touch,
        "maxTouchPoints": if emu.has_touch { 5 } else { 0 },
    })
}

/// `Emulation.setUserAgentOverride` params. Passing an empty `userAgent`
/// restores Chromium's default UA string.
pub fn user_agent_params(emu: &EmulationConfig) -> Value {
    json!({
        "userAgent": emu.user_agent.as_deref().unwrap_or(""),
    })
}

/// `Emulation.setEmulatedMedia` params. Empty value clears the override.
pub fn emulated_media_params(emu: &EmulationConfig) -> Value {
    let features = match emu.color_scheme {
        Some(scheme) => vec![json!({
            "name": "prefers-color-scheme",
            "value": scheme.as_cdp(),
        })],
        None => vec![json!({
            "name": "prefers-color-scheme",
            "value": "",
        })],
    };
    json!({ "features": features })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_emulation_matches_desktop_metrics() {
        let vp = Viewport {
            width: 1280,
            height: 800,
            device_scale_factor: 2.0,
        };
        let emu = EmulationConfig::default();
        let p = device_metrics_params(vp, &emu);
        assert_eq!(p["width"], 1280);
        assert_eq!(p["height"], 800);
        assert_eq!(p["deviceScaleFactor"], 2.0);
        assert_eq!(p["mobile"], false);
        assert!(p.get("screenOrientation").is_none());
        assert_eq!(touch_emulation_params(&emu)["enabled"], false);
        assert_eq!(user_agent_params(&emu)["userAgent"], "");
        assert_eq!(emulated_media_params(&emu)["features"][0]["value"], "");
    }

    #[test]
    fn mobile_portrait_and_landscape_orientation_and_touch() {
        let emu = EmulationConfig {
            preset_id: Some("iphone-16-pro".into()),
            mobile: true,
            has_touch: true,
            user_agent: Some("Mozilla/5.0 (iPhone)".into()),
            color_scheme: Some(ColorScheme::Dark),
            orientation: None,
        };
        let portrait = device_metrics_params(
            Viewport {
                width: 393,
                height: 852,
                device_scale_factor: 3.0,
            },
            &emu,
        );
        assert_eq!(portrait["mobile"], true);
        assert_eq!(portrait["screenOrientation"]["type"], "portraitPrimary");
        assert_eq!(portrait["screenOrientation"]["angle"], 0);

        let landscape = device_metrics_params(
            Viewport {
                width: 852,
                height: 393,
                device_scale_factor: 3.0,
            },
            &emu,
        );
        assert_eq!(landscape["screenOrientation"]["type"], "landscapePrimary");
        assert_eq!(landscape["screenOrientation"]["angle"], 90);
        assert_eq!(touch_emulation_params(&emu)["maxTouchPoints"], 5);
        assert_eq!(user_agent_params(&emu)["userAgent"], "Mozilla/5.0 (iPhone)");
        assert_eq!(emulated_media_params(&emu)["features"][0]["value"], "dark");
    }

    #[test]
    fn sanitize_strips_control_chars_and_bounds_lengths() {
        let raw = EmulationConfig {
            preset_id: Some("iphone 16/../evil!".into()),
            mobile: true,
            has_touch: true,
            user_agent: Some(format!("UA\r\nInjected: 1 {}", "a".repeat(600))),
            color_scheme: Some(ColorScheme::Light),
            orientation: Some(Orientation::Portrait),
        };
        let clean = raw.sanitized();
        assert_eq!(clean.preset_id.as_deref(), Some("iphone16evil"));
        let ua = clean.user_agent.unwrap();
        assert!(!ua.contains('\r') && !ua.contains('\n'));
        assert!(ua.len() <= MAX_USER_AGENT_CHARS);
    }
}
