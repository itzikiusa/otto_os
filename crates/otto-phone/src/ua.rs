//! A short human label ("iPhone · Safari") from a `User-Agent` header — shown
//! on the Mac's approval prompt and as the paired device's default name.

/// `<device> · <browser>`; `Phone` when the agent string says nothing useful.
pub fn device_label(ua: &str) -> String {
    let has = |s: &str| ua.contains(s);
    let device = if has("iPhone") {
        Some("iPhone")
    } else if has("iPad") {
        Some("iPad")
    } else if has("Android") {
        Some(if has("Mobile") {
            "Android phone"
        } else {
            "Android tablet"
        })
    } else if has("Macintosh") {
        // iPadOS Safari asks for the desktop site by default.
        Some("Mac / iPad")
    } else if has("Windows") {
        Some("Windows")
    } else if has("Linux") {
        Some("Linux")
    } else {
        None
    };
    let browser = if has("EdgiOS") || has("EdgA") || has("Edg/") {
        Some("Edge")
    } else if has("FxiOS") || has("Firefox/") {
        Some("Firefox")
    } else if has("CriOS") || (has("Chrome/") && !has("Chromium")) {
        Some("Chrome")
    } else if has("Safari/") {
        Some("Safari")
    } else {
        None
    };
    match (device, browser) {
        (Some(d), Some(b)) => format!("{d} · {b}"),
        (Some(d), None) => d.to_string(),
        (None, Some(b)) => b.to_string(),
        (None, None) => "Phone".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::device_label;

    #[test]
    fn labels() {
        let iphone = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1";
        assert_eq!(device_label(iphone), "iPhone · Safari");
        let crios = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/131.0 Mobile/15E148 Safari/604.1";
        assert_eq!(device_label(crios), "iPhone · Chrome");
        let android = "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0 Mobile Safari/537.36";
        assert_eq!(device_label(android), "Android phone · Chrome");
        assert_eq!(device_label(""), "Phone");
    }
}
