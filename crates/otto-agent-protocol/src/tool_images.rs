//! Bounded raster results shared by the Claude/MCP, Codex and ACP adapters.
//! Only inline image blocks are accepted: no URL fetches, SVG or HTML.
use base64::{engine::general_purpose::STANDARD, Engine};
use otto_core::agent_runtime::ToolOutputImage;
use serde_json::Value;

pub const MAX_IMAGE_BYTES: usize = 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 2 * MAX_IMAGE_BYTES;
const MAX_IMAGES: usize = 4;
const OMITTED: &str = "[Image omitted: unsupported, malformed, or exceeds preview limits.]";

#[derive(Default)]
pub(crate) struct ToolOutput {
    pub text: String,
    pub images: Vec<ToolOutputImage>,
    bytes: usize,
}

impl ToolOutput {
    pub fn image(&mut self, mime: &str, data: &str) {
        // Reject unsupported types / excess images before allocating a decode buffer.
        if self.images.len() >= MAX_IMAGES
            || !matches!(
                mime,
                "image/png" | "image/jpeg" | "image/gif" | "image/webp"
            )
        {
            self.line(OMITTED);
            return;
        }
        let decoded = (data.len() <= MAX_IMAGE_BYTES.div_ceil(3) * 4)
            .then(|| STANDARD.decode(data).ok())
            .flatten();
        if let Some(bytes) = decoded.filter(|b| {
            !b.is_empty()
                && b.len() <= MAX_IMAGE_BYTES
                && self.bytes + b.len() <= MAX_TOTAL_BYTES
                && image_mime(b) == Some(mime)
        }) {
            self.bytes += bytes.len();
            self.images.push(ToolOutputImage {
                mime: mime.into(),
                data_base64: data.into(),
            });
        } else {
            self.line(OMITTED);
        }
    }

    fn line(&mut self, text: &str) {
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(text);
    }

    pub(crate) fn block(&mut self, part: &Value) {
        match part.get("type").and_then(Value::as_str) {
            Some("image") => {
                let source = part.get("source").unwrap_or(part);
                let mime = source
                    .get("media_type")
                    .or_else(|| source.get("mimeType"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let data = source
                    .get("data")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.image(mime, data);
            }
            // OpenCode inline file attachments (only data URIs; never fetch a URL).
            Some("file") => {
                let mime = part
                    .get("mime")
                    .or_else(|| part.get("mimeType"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let url = part.get("url").and_then(Value::as_str).unwrap_or_default();
                let prefix = format!("data:{mime};base64,");
                self.image(mime, url.strip_prefix(&prefix).unwrap_or_default());
            }
            Some("text") => {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    self.line(text);
                }
            }
            // MCP links and embedded resources read as what they are, never
            // as base64: a link is its name and address, a text resource its
            // text, an image blob an image, any other blob one line.
            Some("resource_link") => {
                let s = |k: &str| part.get(k).and_then(Value::as_str).unwrap_or_default();
                let name = [s("title"), s("name"), s("uri")]
                    .into_iter()
                    .find(|v| !v.is_empty());
                let mut line = format!("Link: {} — {}", name.unwrap_or_default(), s("uri"));
                if !s("description").is_empty() {
                    line.push_str(&format!(" ({})", s("description")));
                }
                self.line(&line);
            }
            Some("resource") => {
                let r = part.get("resource").unwrap_or(part);
                let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or_default();
                if let Some(text) = r.get("text").and_then(Value::as_str) {
                    self.line(text);
                } else if s("mimeType").starts_with("image/") {
                    self.image(s("mimeType"), s("blob"));
                } else {
                    let kb = s("blob").len() * 3 / 4 / 1024;
                    self.line(&format!(
                        "Resource: {} ({}, {kb} KB, not shown)",
                        s("uri"),
                        s("mimeType")
                    ));
                }
            }
            Some("audio") => {
                let mime = part
                    .get("mimeType")
                    .and_then(Value::as_str)
                    .unwrap_or("audio");
                let kb = part
                    .get("data")
                    .and_then(Value::as_str)
                    .map_or(0, |d| d.len() * 3 / 4 / 1024);
                self.line(&format!("Audio ({mime}, {kb} KB, not played here)"));
            }
            _ => self.line(&part.to_string()),
        }
    }
}

/// A raster image's mime type by its first bytes.
pub(crate) fn image_mime(b: &[u8]) -> Option<&'static str> {
    if b.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if b.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else {
        None
    }
}

/// Confine an agent-reported tool image path to one of the allowed roots
/// (session cwd, workspace root, attachments dir, or OS temp dir). Returns the
/// canonical path only when it resolves inside a non-root allowed directory.
pub(crate) fn confine_tool_image(
    path: &std::path::Path,
    roots: &[&std::path::Path],
) -> Option<std::path::PathBuf> {
    let canon = std::fs::canonicalize(path).ok()?;
    for root in roots {
        if let Ok(canon_root) = std::fs::canonicalize(root) {
            if canon_root.parent().is_some() && canon.starts_with(&canon_root) {
                return Some(canon);
            }
        }
    }
    None
}

/// Convenience wrapper for `AgentRuntime::tool_image`: confines `path` to
/// `cwd`, optional `ws_root`, `attachments_dir`, or `std::env::temp_dir()`.
pub(crate) fn confine_session_image(
    path: &str,
    cwd: &str,
    ws_root: Option<&str>,
    attachments_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let tmp = std::env::temp_dir();
    let cwd_p = std::path::Path::new(cwd);
    let mut roots: Vec<&std::path::Path> = vec![cwd_p, attachments_dir, tmp.as_path()];
    if let Some(wr) = ws_root {
        roots.push(std::path::Path::new(wr));
    }
    confine_tool_image(std::path::Path::new(path), &roots)
}

/// Preserve mixed content and structured metadata without repeating base64 as text.
/// Intentionally inspect only the provider's known top-level result shapes.
pub(crate) fn extract(value: Option<&Value>) -> ToolOutput {
    let mut out = ToolOutput::default();
    let Some(value) = value.filter(|v| !v.is_null()) else {
        return out;
    };
    if let Some(parts) = value
        .as_array()
        .or_else(|| value.get("content").and_then(Value::as_array))
    {
        for part in parts {
            out.block(part);
        }
        if let Some(structured) = value
            .get("structuredContent")
            .or_else(|| value.get("structured_content"))
            .filter(|v| !v.is_null())
        {
            out.line(&structured.to_string());
        }
    } else if let Some(text) = value.as_str() {
        out.line(text);
    } else if let Some(attachments) = value.get("attachments").and_then(Value::as_array) {
        // OpenCode rawOutput can carry text output plus inline attachments.
        if let Some(text) = value.get("output").and_then(Value::as_str) {
            out.line(text);
        }
        for part in attachments {
            out.block(part);
        }
    } else {
        out.line(&value.to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    pub const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5V8AAAAASUVORK5CYII=";

    #[test]
    fn mixed_mcp_results_keep_images_text_and_structured_metadata() {
        let result = extract(Some(
            &json!({"content":[{"type":"text","text":"Screenshot"},
            {"type":"image","mimeType":"image/png","data":PNG},
            {"type":"image","mimeType":"image/png","data":PNG}],"structuredContent":{"width":1}}),
        ));
        assert_eq!(result.images.len(), 2);
        assert_eq!(result.text, "Screenshot\n{\"width\":1}");
        assert_eq!(result.images[0].data_base64, PNG);
    }
    #[test]
    fn claude_source_and_opencode_attachment_are_inline_only() {
        let result = extract(Some(
            &json!([{ "type":"image", "source":{"type":"base64","media_type":"image/png","data":PNG}}]),
        ));
        assert_eq!(result.images.len(), 1);
        let result = extract(Some(
            &json!({"output":"preview", "attachments":[{"type":"file","mime":"image/png","url":format!("data:image/png;base64,{PNG}")}, {"type":"file","mime":"image/png","url":"https://example.com/a.png"}]}),
        ));
        assert_eq!(result.images.len(), 1);
        assert!(result.text.starts_with("preview\n[Image omitted:"));
    }
    #[test]
    fn invalid_mime_base64_and_limits_are_reported_without_retaining_payload() {
        let mut result = ToolOutput::default();
        result.image("image/svg+xml", PNG);
        result.image("image/png", "!!!!");
        result.image("image/png", &"A".repeat(MAX_IMAGE_BYTES * 2));
        result.image("image/png", &STANDARD.encode(b"<html/>"));
        assert!(result.images.is_empty());
        assert_eq!(result.text.lines().count(), 4);
        for _ in 0..6 {
            result.image("image/png", PNG);
        }
        assert_eq!(result.images.len(), 4);
        assert_eq!(result.text.lines().count(), 6);
    }
    #[test]
    fn decoded_and_aggregate_byte_limits_apply() {
        let mut bytes = vec![0; MAX_IMAGE_BYTES];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        let mut result = ToolOutput::default();
        for _ in 0..3 {
            result.image("image/png", &STANDARD.encode(&bytes));
        }
        assert_eq!(result.images.len(), 2);
        bytes.push(0); // Same padded base64 length as the exact boundary.
        let mut result = ToolOutput::default();
        result.image("image/png", &STANDARD.encode(bytes));
        assert!(result.images.is_empty());
    }
    #[test]
    fn jpeg_webp_gif_and_legacy_text() {
        let mut result = ToolOutput::default();
        result.image("image/jpeg", &STANDARD.encode(b"\xff\xd8\xff\xe0"));
        result.image("image/webp", &STANDARD.encode(b"RIFFxxxxWEBP"));
        result.image("image/gif", &STANDARD.encode(b"GIF89a"));
        assert_eq!(result.images.len(), 3);
        assert_eq!(extract(Some(&json!("legacy"))).text, "legacy");
    }

    #[test]
    fn confine_tool_image_allows_under_root_and_blocks_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let allowed = dir.path().join("ok.png");
        let secret = outside.path().join("other.png");
        std::fs::write(&allowed, b"\x89PNG\r\n\x1a\n").unwrap();
        std::fs::write(&secret, b"\x89PNG\r\n\x1a\n").unwrap();
        let roots = [dir.path()];
        assert!(confine_tool_image(&allowed, &roots).is_some());
        assert!(confine_tool_image(&secret, &roots).is_none());
        #[cfg(unix)]
        {
            let link = dir.path().join("escape.png");
            std::os::unix::fs::symlink(&secret, &link).unwrap();
            assert!(confine_tool_image(&link, &roots).is_none());
        }
    }
}
