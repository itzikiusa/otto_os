//! The `WS /ws/browser/{tab_id}/live` wire protocol (docs/contracts/ws.md
//! §1b): client → daemon [`ClientMsg`], daemon → client [`ServerMsg`], and the
//! self-describing binary screencast frame ([`encode_frame`]).

use serde::{Deserialize, Serialize};

use super::types::LiveSessionInfo;

/// Largest client → daemon text frame accepted.
pub const MAX_CLIENT_FRAME_BYTES: usize = 256 * 1024;

/// Largest paste / insert-text payload (chars).
pub const MAX_PASTE_CHARS: usize = 100_000;

/// Binary frame format version (byte 0).
pub const FRAME_VERSION: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseAction {
    Move,
    Down,
    Up,
    Wheel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
    #[default]
    None,
}

impl MouseButton {
    pub fn cdp(self) -> &'static str {
        match self {
            MouseButton::Left => "left",
            MouseButton::Middle => "middle",
            MouseButton::Right => "right",
            MouseButton::Back => "back",
            MouseButton::Forward => "forward",
            MouseButton::None => "none",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    Down,
    Up,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NavAction {
    Goto,
    Back,
    Forward,
    Reload,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlAction {
    TakeOver,
    HandBack,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    Ack {
        seq: u64,
    },
    Mouse {
        action: MouseAction,
        x: f64,
        y: f64,
        #[serde(default)]
        button: MouseButton,
        #[serde(default)]
        buttons: u32,
        #[serde(default)]
        click_count: u32,
        #[serde(default)]
        delta_x: f64,
        #[serde(default)]
        delta_y: f64,
        #[serde(default)]
        modifiers: u32,
    },
    Key {
        action: KeyAction,
        #[serde(default)]
        key: String,
        #[serde(default)]
        code: String,
        #[serde(default)]
        text: Option<String>,
        #[serde(default)]
        key_code: Option<u32>,
        #[serde(default)]
        location: u32,
        #[serde(default)]
        repeat: bool,
        #[serde(default)]
        modifiers: u32,
    },
    Text {
        text: String,
    },
    Ime {
        text: String,
        #[serde(default)]
        selection_start: i64,
        #[serde(default)]
        selection_end: i64,
    },
    Paste {
        text: String,
    },
    Nav {
        action: NavAction,
        #[serde(default)]
        url: Option<String>,
    },
    Resize {
        width: u32,
        height: u32,
        #[serde(default)]
        device_scale_factor: Option<f64>,
    },
    Control {
        action: ControlAction,
    },
    Dialog {
        accept: bool,
        #[serde(default)]
        prompt_text: Option<String>,
    },
}

impl ClientMsg {
    /// Parse one text frame (size-capped).
    pub fn parse(text: &str) -> Result<Self, String> {
        if text.len() > MAX_CLIENT_FRAME_BYTES {
            return Err("frame too large".into());
        }
        serde_json::from_str(text).map_err(|e| format!("bad frame: {e}"))
    }

    /// Frames that change the page (need Edit + the driver role). `ack` is the
    /// only watch-only frame.
    pub fn is_drive(&self) -> bool {
        !matches!(self, ClientMsg::Ack { .. })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    State {
        session: Box<LiveSessionInfo>,
    },
    Cursor {
        cursor: String,
    },
    Dialog {
        dialog_type: String,
        message: String,
        default_prompt: String,
        url: String,
    },
    Blocked {
        host: String,
        reason: &'static str,
    },
    Popup {
        url: String,
    },
    Download {
        status: &'static str,
        filename: String,
        bytes: Option<u64>,
    },
    Approval {
        approval_id: String,
        status: &'static str,
        title: String,
    },
    Error {
        code: &'static str,
        message: String,
    },
    Closed {
        reason: &'static str,
    },
}

impl ServerMsg {
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        ServerMsg::Error {
            code,
            message: message.into(),
        }
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{\"type\":\"error\",\"code\":\"bad_frame\",\"message\":\"encode\"}".into())
    }
}

/// Header of one binary screencast frame (see [`encode_frame`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FrameHeader {
    pub seq: u64,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub device_width: f64,
    pub device_height: f64,
    pub page_scale_factor: f64,
    pub offset_top: f64,
    pub scroll_x: f64,
    pub scroll_y: f64,
    pub timestamp: f64,
}

/// `[u8 version][u32 BE header length][JSON header][image bytes]`.
pub fn encode_frame(header: &FrameHeader, image: &[u8]) -> Vec<u8> {
    let json = serde_json::to_vec(header).unwrap_or_default();
    let mut out = Vec::with_capacity(5 + json.len() + image.len());
    out.push(FRAME_VERSION);
    out.extend_from_slice(&(json.len() as u32).to_be_bytes());
    out.extend_from_slice(&json);
    out.extend_from_slice(image);
    out
}

/// Inverse of [`encode_frame`] (tests + any Rust client).
pub fn decode_frame(bytes: &[u8]) -> Option<(FrameHeader, &[u8])> {
    if bytes.len() < 5 || bytes[0] != FRAME_VERSION {
        return None;
    }
    let n = u32::from_be_bytes([bytes[1], bytes[2], bytes[3], bytes[4]]) as usize;
    let end = 5usize.checked_add(n)?;
    if bytes.len() < end {
        return None;
    }
    let header: FrameHeader = serde_json::from_slice(&bytes[5..end]).ok()?;
    Some((header, &bytes[end..]))
}

/// Read the pixel size from a baseline/progressive JPEG's SOF marker (the
/// screencast metadata carries the viewport, not the encoded image size).
pub fn jpeg_size(data: &[u8]) -> Option<(u32, u32)> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }
    let mut i = 2usize;
    while i + 9 < data.len() {
        if data[i] != 0xFF {
            i += 1;
            continue;
        }
        let marker = data[i + 1];
        // Standalone markers without a length.
        if marker == 0xD8 || marker == 0x01 || (0xD0..=0xD7).contains(&marker) || marker == 0xFF {
            i += 2;
            continue;
        }
        let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
        let is_sof = matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF);
        if is_sof {
            let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
            let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
            return Some((w, h));
        }
        if len < 2 {
            return None;
        }
        i += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_client_frame_kind() {
        let cases = [
            r#"{"type":"ack","seq":3}"#,
            r#"{"type":"mouse","action":"down","x":1,"y":2,"button":"left","buttons":1,"click_count":1,"modifiers":8}"#,
            r#"{"type":"mouse","action":"wheel","x":1,"y":2,"delta_y":120}"#,
            r#"{"type":"key","action":"down","key":"a","code":"KeyA","text":"a"}"#,
            r#"{"type":"text","text":"日本"}"#,
            r#"{"type":"ime","text":"にほ","selection_start":2,"selection_end":2}"#,
            r#"{"type":"paste","text":"x"}"#,
            r#"{"type":"nav","action":"goto","url":"https://example.com/"}"#,
            r#"{"type":"nav","action":"back"}"#,
            r#"{"type":"resize","width":800,"height":600,"device_scale_factor":2}"#,
            r#"{"type":"control","action":"take_over"}"#,
            r#"{"type":"dialog","accept":true,"prompt_text":"ok"}"#,
        ];
        for c in cases {
            let m = ClientMsg::parse(c).unwrap_or_else(|e| panic!("{c}: {e}"));
            assert_eq!(m.is_drive(), !c.contains("\"ack\""));
        }
        assert!(ClientMsg::parse(r#"{"type":"eval","js":"alert(1)"}"#).is_err());
        assert!(ClientMsg::parse(&"x".repeat(MAX_CLIENT_FRAME_BYTES + 1)).is_err());
    }

    #[test]
    fn server_frames_are_tagged_snake_case() {
        let j = ServerMsg::Blocked {
            host: "10.0.0.1".into(),
            reason: "ssrf",
        }
        .to_json();
        assert_eq!(j, r#"{"type":"blocked","host":"10.0.0.1","reason":"ssrf"}"#);
        let j = ServerMsg::error("not_driver", "take over first").to_json();
        assert!(j.starts_with(r#"{"type":"error","code":"not_driver""#));
        let j = ServerMsg::Closed { reason: "idle" }.to_json();
        assert_eq!(j, r#"{"type":"closed","reason":"idle"}"#);
    }

    #[test]
    fn binary_frames_round_trip() {
        let h = FrameHeader {
            seq: 7,
            mime: "image/jpeg".into(),
            width: 640,
            height: 400,
            device_width: 1280.0,
            device_height: 800.0,
            page_scale_factor: 1.0,
            offset_top: 0.0,
            scroll_x: 0.0,
            scroll_y: 12.5,
            timestamp: 1.5,
        };
        let bytes = encode_frame(&h, b"\xFF\xD8jpeg");
        assert_eq!(bytes[0], FRAME_VERSION);
        let (back, img) = decode_frame(&bytes).unwrap();
        assert_eq!(back, h);
        assert_eq!(img, b"\xFF\xD8jpeg");
        assert!(decode_frame(&bytes[..3]).is_none());
        let mut wrong = bytes.clone();
        wrong[0] = 9;
        assert!(decode_frame(&wrong).is_none());
    }

    #[test]
    fn reads_jpeg_dimensions_from_the_sof_marker() {
        // SOI, APP0 (len 16), SOF0 (len 17: precision, h=0x0190, w=0x0280, …).
        let mut j = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
        j.extend_from_slice(&[0u8; 14]);
        j.extend_from_slice(&[0xFF, 0xC0, 0x00, 0x11, 0x08, 0x01, 0x90, 0x02, 0x80, 0x03]);
        j.extend_from_slice(&[0u8; 12]);
        assert_eq!(jpeg_size(&j), Some((640, 400)));
        assert_eq!(jpeg_size(b"not a jpeg"), None);
    }
}
