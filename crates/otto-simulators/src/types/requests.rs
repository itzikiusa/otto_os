//! Request/response payloads and validated input coordinate scaling.

use serde::{Deserialize, Serialize};

use super::{Appearance, Orientation, ScreenMetrics, MAX_INPUT_COORD_PX};

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "platform", rename_all = "snake_case")]
pub enum CreateSimDeviceReq {
    Ios {
        name: String,
        device_type_id: String,
        runtime_id: String,
    },
    Android {
        name: String,
        system_image_id: String,
        #[serde(default)]
        device_profile: Option<String>,
    },
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BootSimDeviceReq {
    #[serde(default)]
    pub wait: Option<bool>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub open_gui: Option<bool>,
    #[serde(default)]
    pub headless: Option<bool>,
    #[serde(default)]
    pub wipe_data: Option<bool>,
}

/// Alias sharing [`BootSimDeviceReq`] (`restart_device` forces `wipe_data = Some(false)`).
pub type RestartSimDeviceReq = BootSimDeviceReq;

#[derive(Debug, Clone, Deserialize)]
pub struct OpenSimUrlReq {
    pub url: String,
    #[serde(default)]
    pub auto_forward_localhost: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenSimUrlResp {
    pub ok: bool,
    pub url: String,
    pub effective_url: String,
    pub forwarded_ports: Vec<u16>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotFormat {
    #[default]
    Png,
    Jpeg,
}

impl ScreenshotFormat {
    pub fn mime(self) -> &'static str {
        match self {
            ScreenshotFormat::Png => "image/png",
            ScreenshotFormat::Jpeg => "image/jpeg",
        }
    }

    pub fn simctl_type(self) -> &'static str {
        match self {
            ScreenshotFormat::Png => "png",
            ScreenshotFormat::Jpeg => "jpeg",
        }
    }
}

/// Typed `?encoding=` query parameter: rejects unknown encodings with 400.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenshotEncoding {
    #[default]
    Binary,
    Base64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotData {
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
    pub captured_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenshotJsonResp {
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub base64: String,
    pub captured_at: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum InputAction {
    Tap {
        x: f64,
        y: f64,
        #[serde(default)]
        normalized: bool,
    },
    Swipe {
        x1: f64,
        y1: f64,
        x2: f64,
        y2: f64,
        #[serde(default)]
        duration_ms: Option<u32>,
        #[serde(default)]
        normalized: bool,
    },
    Text {
        text: String,
    },
    KeyEvent {
        key: String,
    },
}

/// Validate a single coordinate value: must be finite, non-negative,
/// and within `0.0..=1.0` when normalized or `0.0..=MAX_INPUT_COORD_PX` when pixel.
pub fn validate_coord(val: f64, normalized: bool, label: &str) -> otto_core::Result<f64> {
    if !val.is_finite() {
        return Err(otto_core::Error::Invalid(format!(
            "{label} must be a finite number"
        )));
    }
    if normalized {
        if !(0.0..=1.0).contains(&val) {
            return Err(otto_core::Error::Invalid(format!(
                "normalized {label} must be in 0.0..=1.0"
            )));
        }
    } else if !(0.0..=MAX_INPUT_COORD_PX).contains(&val) {
        return Err(otto_core::Error::Invalid(format!(
            "pixel {label} must be in 0..={MAX_INPUT_COORD_PX}"
        )));
    }
    Ok(val)
}

/// Resolve a `(x, y)` pair against orientation-adjusted screen metrics.
pub fn resolve_point(
    x: f64,
    y: f64,
    normalized: bool,
    screen: Option<&ScreenMetrics>,
    orientation: Orientation,
    fallback_dims: (f64, f64),
) -> otto_core::Result<(i64, i64)> {
    let vx = validate_coord(x, normalized, "x")?;
    let vy = validate_coord(y, normalized, "y")?;
    let (w, h) = screen
        .map(|s| s.oriented_dims(orientation))
        .unwrap_or_else(|| {
            if orientation.is_landscape() && fallback_dims.1 > fallback_dims.0 {
                (fallback_dims.1, fallback_dims.0)
            } else {
                fallback_dims
            }
        });
    let (px, py) = if normalized {
        (vx * w, vy * h)
    } else {
        (vx, vy)
    };
    Ok((px.round() as i64, py.round() as i64))
}

#[derive(Debug, Clone, Deserialize)]
pub struct OrientationReq {
    pub orientation: Orientation,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AppearanceReq {
    pub appearance: Appearance,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct StatusBarReq {
    #[serde(default)]
    pub clear: bool,
    #[serde(default)]
    pub time: Option<String>,
    #[serde(default)]
    pub battery_level: Option<u8>,
    #[serde(default)]
    pub battery_state: Option<String>,
    #[serde(default)]
    pub wifi_bars: Option<u8>,
    #[serde(default)]
    pub cellular_bars: Option<u8>,
    #[serde(default)]
    pub operator_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardResp {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SetClipboardReq {
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct InstallAppReq {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LaunchAppReq {
    pub app_id: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TerminateAppReq {
    pub app_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortForward {
    pub remote_port: u16,
    pub local_port: u16,
    pub protocol: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortsResp {
    pub ports: Vec<PortForward>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CreatePortForwardReq {
    pub remote_port: u16,
    pub local_port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionOkResp {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl ActionOkResp {
    pub fn ok() -> Self {
        Self {
            ok: true,
            detail: None,
        }
    }

    pub fn with_detail(detail: impl Into<String>) -> Self {
        Self {
            ok: true,
            detail: Some(detail.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_point_swaps_for_landscape_and_rejects_bad_coords() {
        let portrait = ScreenMetrics {
            width: 1080,
            height: 2424,
            scale: Some(2.625),
            dpi: Some(420),
        };
        assert_eq!(
            resolve_point(
                0.5,
                0.5,
                true,
                Some(&portrait),
                Orientation::Portrait,
                (1080.0, 2400.0)
            )
            .unwrap(),
            (540, 1212)
        );
        // Rotated to Landscape: width=2424, height=1080
        assert_eq!(
            resolve_point(
                0.85,
                0.5,
                true,
                Some(&portrait),
                Orientation::Landscape,
                (1080.0, 2400.0)
            )
            .unwrap(),
            (2060, 540)
        );
        // Negative, NaN, Inf, and oversized pixel coords are rejected with 400 Invalid.
        assert!(resolve_point(
            -50.0,
            10.0,
            false,
            Some(&portrait),
            Orientation::Portrait,
            (1080.0, 2400.0)
        )
        .is_err());
        assert!(resolve_point(
            1e20,
            10.0,
            false,
            Some(&portrait),
            Orientation::Portrait,
            (1080.0, 2400.0)
        )
        .is_err());
        assert!(resolve_point(
            f64::NAN,
            10.0,
            false,
            Some(&portrait),
            Orientation::Portrait,
            (1080.0, 2400.0)
        )
        .is_err());
        assert!(resolve_point(
            1.5,
            0.5,
            true,
            Some(&portrait),
            Orientation::Portrait,
            (1080.0, 2400.0)
        )
        .is_err());
    }
}
