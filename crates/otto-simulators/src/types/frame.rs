//! Device ID parsing, URL validation, localhost port extraction, binary
//! screencast frame envelope, and PNG/JPEG header dimension parsing.

use serde::{Deserialize, Serialize};

use super::{Orientation, Platform, FRAME_VERSION};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceId {
    IosUdid(String),
    AndroidAvd(String),
    AndroidSerial(String),
}

impl DeviceId {
    pub fn parse(raw: &str) -> otto_core::Result<Self> {
        let s = raw.trim();
        if s.is_empty() || s.len() > 256 || s.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(otto_core::Error::Invalid(
                "invalid simulator device id".into(),
            ));
        }
        if let Some(udid) = s.strip_prefix("ios:") {
            if is_safe_token(udid) {
                return Ok(DeviceId::IosUdid(udid.to_string()));
            }
            return Err(otto_core::Error::Invalid(
                "invalid iOS simulator UDID".into(),
            ));
        }
        if let Some(avd) = s.strip_prefix("android:avd:") {
            if is_safe_avd_name(avd) {
                return Ok(DeviceId::AndroidAvd(avd.to_string()));
            }
            return Err(otto_core::Error::Invalid("invalid Android AVD name".into()));
        }
        if let Some(serial) = s.strip_prefix("android:serial:") {
            if is_safe_serial(serial) {
                return Ok(DeviceId::AndroidSerial(serial.to_string()));
            }
            return Err(otto_core::Error::Invalid("invalid Android serial".into()));
        }
        Err(otto_core::Error::Invalid(format!(
            "unsupported simulator id {s:?}; expected ios:<udid>, android:avd:<name>, or android:serial:<serial>"
        )))
    }

    pub fn platform(&self) -> Platform {
        match self {
            DeviceId::IosUdid(_) => Platform::Ios,
            DeviceId::AndroidAvd(_) | DeviceId::AndroidSerial(_) => Platform::Android,
        }
    }

    pub fn as_canonical(&self) -> String {
        match self {
            DeviceId::IosUdid(u) => format!("ios:{u}"),
            DeviceId::AndroidAvd(a) => format!("android:avd:{a}"),
            DeviceId::AndroidSerial(s) => format!("android:serial:{s}"),
        }
    }
}

pub fn is_safe_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

pub fn is_safe_avd_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('-')
        && !s.contains("..")
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

pub fn is_safe_serial(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 128
        && !s.starts_with('-')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b':'))
}

/// Validate a URL or deep link before handing it to `simctl openurl` / `am start`.
pub fn validate_open_url(raw: &str) -> otto_core::Result<String> {
    let url = raw.trim();
    if url.is_empty() || url.len() > 8192 {
        return Err(otto_core::Error::Invalid(
            "url must be 1..=8192 bytes".into(),
        ));
    }
    if url.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(otto_core::Error::Invalid(
            "url must not contain whitespace or control characters".into(),
        ));
    }
    let Some((scheme, rest)) = url.split_once(':') else {
        return Err(otto_core::Error::Invalid(
            "url must include a scheme (e.g. http://, https://, myapp://)".into(),
        ));
    };
    if scheme.is_empty()
        || !scheme
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic())
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        return Err(otto_core::Error::Invalid("invalid url scheme".into()));
    }
    if matches!(
        scheme.to_ascii_lowercase().as_str(),
        "javascript" | "file" | "data" | "vbscript"
    ) {
        return Err(otto_core::Error::Invalid(format!(
            "scheme {scheme:?} is not allowed"
        )));
    }
    if rest.is_empty() {
        return Err(otto_core::Error::Invalid(
            "url body must not be empty".into(),
        ));
    }
    Ok(url.to_string())
}

/// Inspect an `http(s)` URL via `reqwest::Url` for loopback hosts
/// (`localhost`, `127.0.0.1`, `[::1]`, `0.0.0.0`) and return
/// `(effective_url, Option<port>)` so Android can run `adb reverse`.
pub fn extract_localhost_port(url: &str) -> (String, Option<u16>) {
    let Ok(mut parsed) = reqwest::Url::parse(url) else {
        return (url.to_string(), None);
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return (url.to_string(), None);
    }
    let Some(host) = parsed.host_str() else {
        return (url.to_string(), None);
    };
    let host_lc = host.to_ascii_lowercase();
    if !matches!(
        host_lc.as_str(),
        "localhost" | "127.0.0.1" | "[::1]" | "0.0.0.0"
    ) {
        return (url.to_string(), None);
    }
    let port = parsed.port_or_known_default().filter(|&p| p > 0);
    // Android cannot connect to `0.0.0.0` as a loopback target — rewrite to `127.0.0.1`.
    if host_lc == "0.0.0.0" && parsed.set_host(Some("127.0.0.1")).is_ok() {
        return (parsed.to_string(), port);
    }
    (url.to_string(), port)
}

// ---------------------------------------------------------------------------
// Binary frame envelope (matches otto_browser::live::protocol)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimFrameHeader {
    pub seq: u64,
    pub mime: String,
    pub width: u32,
    pub height: u32,
    pub orientation: Orientation,
    pub timestamp_ms: i64,
}

/// Encode a binary frame: `[version: u8 = 1][header_len: u32 BE][JSON header][image bytes]`.
pub fn encode_frame(header: &SimFrameHeader, image_bytes: &[u8]) -> Vec<u8> {
    let header_json = serde_json::to_vec(header).unwrap_or_default();
    let header_len = header_json.len() as u32;
    let mut out = Vec::with_capacity(5 + header_json.len() + image_bytes.len());
    out.push(FRAME_VERSION);
    out.extend_from_slice(&header_len.to_be_bytes());
    out.extend_from_slice(&header_json);
    out.extend_from_slice(image_bytes);
    out
}

/// Decode a binary frame encoded by [`encode_frame`].
pub fn decode_frame(data: &[u8]) -> Option<(SimFrameHeader, &[u8])> {
    if data.len() < 5 || data[0] != FRAME_VERSION {
        return None;
    }
    let header_len = u32::from_be_bytes([data[1], data[2], data[3], data[4]]) as usize;
    let payload_start = 5usize.checked_add(header_len)?;
    if payload_start > data.len() {
        return None;
    }
    let header: SimFrameHeader = serde_json::from_slice(&data[5..payload_start]).ok()?;
    Some((header, &data[payload_start..]))
}

/// Extract `(width, height)` from PNG or JPEG header bytes without external image crates.
pub fn image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    // PNG: 8-byte signature + 4-byte length + "IHDR" + 4-byte width + 4-byte height.
    if data.len() >= 24 && &data[..8] == b"\x89PNG\r\n\x1a\n" && &data[12..16] == b"IHDR" {
        let w = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
        let h = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);
        return Some((w, h));
    }
    // JPEG: SOI (FF D8), then walk markers to SOF0..SOF15 (excluding DHT/DAC/JPG).
    if data.len() >= 4 && data[0] == 0xFF && data[1] == 0xD8 {
        let mut i = 2usize;
        while i + 4 <= data.len() {
            if data[i] != 0xFF {
                break;
            }
            let marker = data[i + 1];
            if marker == 0xD9 || marker == 0xDA {
                break;
            }
            if marker == 0x00 || (0xD0..=0xD8).contains(&marker) {
                i += 2;
                continue;
            }
            let len = u16::from_be_bytes([data[i + 2], data[i + 3]]) as usize;
            let is_sof = matches!(
                marker,
                0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF
            );
            if is_sof && i + 9 <= data.len() {
                let h = u16::from_be_bytes([data[i + 5], data[i + 6]]) as u32;
                let w = u16::from_be_bytes([data[i + 7], data[i + 8]]) as u32;
                return Some((w, h));
            }
            if len < 2 {
                break;
            }
            i = i.saturating_add(2 + len);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_id_parsing() {
        assert_eq!(
            DeviceId::parse("ios:A1B2-C3D4").unwrap(),
            DeviceId::IosUdid("A1B2-C3D4".into())
        );
        assert_eq!(
            DeviceId::parse("android:avd:Pixel_9_API_35").unwrap(),
            DeviceId::AndroidAvd("Pixel_9_API_35".into())
        );
        assert_eq!(
            DeviceId::parse("android:serial:emulator-5554").unwrap(),
            DeviceId::AndroidSerial("emulator-5554".into())
        );
        assert_eq!(
            DeviceId::parse("android:serial:192.168.1.5:5555").unwrap(),
            DeviceId::AndroidSerial("192.168.1.5:5555".into())
        );
        assert!(DeviceId::parse("ios:../bad").is_err());
        assert!(DeviceId::parse("android:avd:-flag").is_err());
        assert!(DeviceId::parse("unknown:123").is_err());
        assert!(DeviceId::parse("ios:abc def").is_err());
    }

    #[test]
    fn url_validation_and_localhost_port_extraction() {
        assert!(validate_open_url("http://localhost:5173/test").is_ok());
        assert!(validate_open_url("myapp://open/screen?id=1").is_ok());
        assert!(validate_open_url("javascript:alert(1)").is_err());
        assert!(validate_open_url("file:///etc/passwd").is_err());
        assert!(validate_open_url("http://evil.com/\nfoo").is_err());
        assert!(validate_open_url("no-scheme").is_err());

        assert_eq!(
            extract_localhost_port("http://localhost:5173/app"),
            ("http://localhost:5173/app".into(), Some(5173))
        );
        assert_eq!(
            extract_localhost_port("http://127.0.0.1:7700/"),
            ("http://127.0.0.1:7700/".into(), Some(7700))
        );
        assert_eq!(
            extract_localhost_port("http://[::1]:3000/x"),
            ("http://[::1]:3000/x".into(), Some(3000))
        );
        assert_eq!(
            extract_localhost_port("http://0.0.0.0:8080/hi"),
            ("http://127.0.0.1:8080/hi".into(), Some(8080))
        );
        assert_eq!(
            extract_localhost_port("https://example.com:8443/"),
            ("https://example.com:8443/".into(), None)
        );
        assert_eq!(
            extract_localhost_port("http://localhost:5173@evil.com/"),
            ("http://localhost:5173@evil.com/".into(), None)
        );
    }

    #[test]
    fn binary_frame_round_trip() {
        let hdr = SimFrameHeader {
            seq: 42,
            mime: "image/png".into(),
            width: 393,
            height: 852,
            orientation: Orientation::Portrait,
            timestamp_ms: 1_700_000_000_000,
        };
        let encoded = encode_frame(&hdr, b"\x89PNGtest");
        let (decoded, payload) = decode_frame(&encoded).unwrap();
        assert_eq!(decoded, hdr);
        assert_eq!(payload, b"\x89PNGtest");
    }

    #[test]
    fn png_header_dimensions() {
        let mut png = Vec::new();
        png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
        png.extend_from_slice(&13u32.to_be_bytes());
        png.extend_from_slice(b"IHDR");
        png.extend_from_slice(&1179u32.to_be_bytes());
        png.extend_from_slice(&2556u32.to_be_bytes());
        assert_eq!(image_dimensions(&png), Some((1179, 2556)));
    }
}
