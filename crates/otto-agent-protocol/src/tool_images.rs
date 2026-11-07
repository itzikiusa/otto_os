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
                && raster_signature(mime, b)
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

    fn block(&mut self, part: &Value) {
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
            _ => self.line(&part.to_string()),
        }
    }
}

fn raster_signature(mime: &str, b: &[u8]) -> bool {
    match mime {
        "image/png" => b.starts_with(b"\x89PNG\r\n\x1a\n"),
        "image/jpeg" => b.starts_with(b"\xff\xd8\xff"),
        "image/gif" => b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a"),
        "image/webp" => b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP"),
        _ => false,
    }
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
}
