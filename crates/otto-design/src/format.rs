//! The artifact formats the graph stores, and the cheap, format-agnostic checks
//! every save passes (size cap, UTF-8 for text, a JSON object for JSON
//! formats, a magic-byte sniff for binaries). Deep schema validation that
//! needs a host crate (e.g. `scene3d`, owned by otto-server) is the
//! `DesignCtx::validate_content` hook. Pure; unit-tested.

use otto_core::{Error, Result};

/// Raw content cap per version (matches the product-attachment cap).
pub const MAX_CONTENT_BYTES: usize = 25 * 1024 * 1024;
/// Above this a `design_artifact_updated` event carries `content: null`.
pub const MAX_EVENT_CONTENT: usize = 4 * 1024 * 1024;

/// How a format's bytes are encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    /// UTF-8 source text (HTML, Mermaid, D2, SVG).
    Text,
    /// A UTF-8 JSON document whose top level is an object.
    Json,
    /// Opaque bytes (images, GLB, PDF) — never edited in place by an agent.
    Binary,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FormatSpec {
    /// Wire name stored in `design_artifacts.format`.
    pub name: &'static str,
    pub mime: &'static str,
    /// File name of the working copy (`<data>/design/<id>/work/<file_name>`).
    pub file_name: &'static str,
    /// The studio an artifact of this format lands in by default.
    pub studio: &'static str,
    pub encoding: Encoding,
}

const fn f(
    name: &'static str,
    mime: &'static str,
    file_name: &'static str,
    studio: &'static str,
    encoding: Encoding,
) -> FormatSpec {
    FormatSpec {
        name,
        mime,
        file_name,
        studio,
        encoding,
    }
}

/// Every known format. `otto-site` / `otto-layout` / `otto-brand` /
/// `otto-exhibit` are Phase 1+ documents: stored and link-indexed today (as
/// generic JSON), with their own validators landing with their studios.
pub const FORMATS: &[FormatSpec] = &[
    f("html", "text/html", "design.html", "frames", Encoding::Text),
    f(
        "mermaid",
        "text/vnd.mermaid",
        "design.mmd",
        "whiteboard",
        Encoding::Text,
    ),
    f(
        "d2",
        "text/vnd.d2",
        "design.d2",
        "whiteboard",
        Encoding::Text,
    ),
    f(
        "excalidraw",
        "application/vnd.excalidraw+json",
        "design.excalidraw",
        "whiteboard",
        Encoding::Json,
    ),
    f(
        "scene3d",
        "application/vnd.otto.scene3d+json",
        "scene.json",
        "3d",
        Encoding::Json,
    ),
    // An imported Canvas scene document (`{type:"otto-canvas",format,source,…}`)
    // kept verbatim so the Whiteboard studio renders it losslessly.
    f(
        "otto-canvas",
        "application/vnd.otto.canvas+json",
        "canvas.json",
        "whiteboard",
        Encoding::Json,
    ),
    f(
        "otto-site",
        "application/vnd.otto.site+json",
        "site.json",
        "site",
        Encoding::Json,
    ),
    f(
        "otto-layout",
        "application/vnd.otto.layout+json",
        "layout.json",
        "frames",
        Encoding::Json,
    ),
    f(
        "otto-brand",
        "application/vnd.otto.brand+json",
        "brand.json",
        "brand",
        Encoding::Json,
    ),
    f(
        "otto-exhibit",
        "application/vnd.otto.exhibit+json",
        "exhibit.json",
        "spatial",
        Encoding::Json,
    ),
    f(
        "svg",
        "image/svg+xml",
        "image.svg",
        "graphics",
        Encoding::Text,
    ),
    f(
        "png",
        "image/png",
        "image.png",
        "graphics",
        Encoding::Binary,
    ),
    f(
        "jpeg",
        "image/jpeg",
        "image.jpg",
        "graphics",
        Encoding::Binary,
    ),
    f(
        "gif",
        "image/gif",
        "image.gif",
        "graphics",
        Encoding::Binary,
    ),
    f(
        "webp",
        "image/webp",
        "image.webp",
        "graphics",
        Encoding::Binary,
    ),
    f(
        "pdf",
        "application/pdf",
        "document.pdf",
        "graphics",
        Encoding::Binary,
    ),
    f(
        "glb",
        "model/gltf-binary",
        "model.glb",
        "3d",
        Encoding::Binary,
    ),
    f(
        "gltf",
        "model/gltf+json",
        "model.gltf",
        "3d",
        Encoding::Json,
    ),
];

/// Look a format up by its wire name.
pub fn spec(name: &str) -> Option<&'static FormatSpec> {
    FORMATS.iter().find(|s| s.name == name)
}

/// Look a format up by MIME (the legacy attachment rows carry a mime).
pub fn from_mime(mime: &str) -> Option<&'static FormatSpec> {
    FORMATS.iter().find(|s| s.mime == mime)
}

/// The cheap save-time checks: size cap, then per-encoding shape.
pub fn validate(spec: &FormatSpec, bytes: &[u8]) -> Result<()> {
    if bytes.len() > MAX_CONTENT_BYTES {
        return Err(Error::PayloadTooLarge(format!(
            "design content exceeds the {} MB cap",
            MAX_CONTENT_BYTES / (1024 * 1024)
        )));
    }
    match spec.encoding {
        Encoding::Text => {
            std::str::from_utf8(bytes)
                .map_err(|_| Error::Invalid(format!("{} content must be UTF-8", spec.name)))?;
        }
        Encoding::Json => {
            let v: serde_json::Value = serde_json::from_slice(bytes).map_err(|e| {
                Error::Invalid(format!("{} content is not valid JSON: {e}", spec.name))
            })?;
            if !v.is_object() {
                return Err(Error::Invalid(format!(
                    "{} content must be a JSON object",
                    spec.name
                )));
            }
            if spec.name == "otto-brand" {
                crate::brand::validate(&v)?;
            }
        }
        Encoding::Binary => {
            if bytes.is_empty() {
                return Err(Error::Invalid(format!("{} content is empty", spec.name)));
            }
            if !sniff_binary(spec.name, bytes) {
                return Err(Error::Invalid(format!(
                    "content does not look like {} (magic-byte check failed)",
                    spec.name
                )));
            }
        }
    }
    Ok(())
}

/// Magic-byte sniff for the binary formats (mirrors `product_media::sniff_ok`).
fn sniff_binary(name: &str, b: &[u8]) -> bool {
    match name {
        "png" => b.starts_with(&[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A]),
        "jpeg" => b.starts_with(&[0xFF, 0xD8, 0xFF]),
        "gif" => b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a"),
        "webp" => b.len() >= 12 && &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP",
        "pdf" => b.starts_with(b"%PDF-"),
        "glb" => b.len() >= 12 && &b[0..4] == b"glTF",
        _ => false,
    }
}

/// The document a brand-new artifact starts with when the caller sent no
/// content. `None` for binary formats (they need real bytes).
pub fn default_content(spec: &FormatSpec) -> Option<Vec<u8>> {
    let s: String = match spec.name {
        "html" => "<!doctype html>\n<html>\n<head><meta charset=\"utf-8\"></head>\n<body></body>\n</html>\n".into(),
        "mermaid" => "flowchart TD\n".into(),
        "d2" => String::new(),
        "svg" => "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"800\" height=\"600\"></svg>\n".into(),
        "excalidraw" => serde_json::json!({
            "type": "excalidraw", "version": 2, "elements": [], "appState": {}, "files": {}
        })
        .to_string(),
        "scene3d" => serde_json::json!({
            "type": "otto-scene3d", "version": 1, "objects": [], "lights": [], "groups": []
        })
        .to_string(),
        "otto-canvas" => serde_json::json!({
            "type": "otto-canvas", "version": 1, "format": "mermaid", "source": ""
        })
        .to_string(),
        "otto-brand" => crate::brand::default_doc("Brand kit").to_string(),
        "gltf" => return None,
        other if spec.encoding == Encoding::Json => {
            serde_json::json!({ "type": other, "version": 1 }).to_string()
        }
        _ => return None,
    };
    Some(s.into_bytes())
}

/// The `content` a live event may carry: the UTF-8 source of a text/JSON
/// format up to [`MAX_EVENT_CONTENT`]; `None` otherwise (clients re-fetch).
pub fn event_content(spec: &FormatSpec, bytes: &[u8]) -> Option<String> {
    if spec.encoding == Encoding::Binary || bytes.len() > MAX_EVENT_CONTENT {
        return None;
    }
    String::from_utf8(bytes.to_vec()).ok()
}

/// Is `studio` a known studio?
pub fn valid_studio(studio: &str) -> bool {
    crate::types::STUDIOS.contains(&studio)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_is_unique_and_homed_in_a_real_studio() {
        let mut names = std::collections::HashSet::new();
        let mut mimes = std::collections::HashSet::new();
        for s in FORMATS {
            assert!(names.insert(s.name), "duplicate format {}", s.name);
            assert!(mimes.insert(s.mime), "duplicate mime {}", s.mime);
            assert!(valid_studio(s.studio), "{} homed in unknown studio", s.name);
        }
        assert_eq!(from_mime("model/gltf-binary").unwrap().name, "glb");
        assert_eq!(spec("scene3d").unwrap().studio, "3d");
    }

    #[test]
    fn validate_checks_shape_per_encoding() {
        let html = spec("html").unwrap();
        assert!(validate(html, b"<p>hi</p>").is_ok());
        assert!(validate(html, &[0xff, 0xfe]).is_err());

        let ex = spec("excalidraw").unwrap();
        assert!(validate(ex, br#"{"elements":[]}"#).is_ok());
        assert!(
            validate(ex, b"[1,2]").is_err(),
            "top level must be an object"
        );
        assert!(validate(ex, b"{nope").is_err());

        let png = spec("png").unwrap();
        assert!(validate(png, &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0]).is_ok());
        assert!(validate(png, b"GIF89a").is_err());
        assert!(validate(png, b"").is_err());

        let big = vec![b'a'; MAX_CONTENT_BYTES + 1];
        assert!(matches!(
            validate(html, &big),
            Err(Error::PayloadTooLarge(_))
        ));
    }

    #[test]
    fn default_content_is_valid_for_every_non_binary_format() {
        for s in FORMATS {
            match default_content(s) {
                Some(bytes) => assert!(validate(s, &bytes).is_ok(), "{} default invalid", s.name),
                None => assert!(
                    s.encoding == Encoding::Binary || s.name == "gltf",
                    "{} has no default",
                    s.name
                ),
            }
        }
    }

    #[test]
    fn event_content_skips_binaries_and_oversized_payloads() {
        assert_eq!(
            event_content(spec("mermaid").unwrap(), b"graph TD").as_deref(),
            Some("graph TD")
        );
        assert!(event_content(spec("png").unwrap(), b"x").is_none());
        let big = vec![b'a'; MAX_EVENT_CONTENT + 1];
        assert!(event_content(spec("html").unwrap(), &big).is_none());
    }
}
