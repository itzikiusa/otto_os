//! The ONE design-artifact format enum shared by the mockup/design assist route
//! (`mockup_assist.rs`), the swarm `otto-mockup` ingest (`routes/swarm_ingest.rs`)
//! and the content PUT (`product_media.rs`). Mirrored in TS as `DesignFormat`.
//!
//! Every agent-editable artifact in the Design arena is one of these four text
//! formats; uploaded binaries (`glb` / `gltf` / images) are attachments with a
//! mime but no `DesignFormat` — they are never agent-edited in place. Parsing an
//! unknown format is a **400** (`Error::Invalid`), never a silent fallback to
//! HTML: a typo in `--format` must surface, not file a mislabeled artifact.

use std::fmt;
use std::str::FromStr;

use otto_core::Error;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DesignFormat {
    /// A self-contained HTML screen (sandboxed iframe).
    Html,
    /// A Mermaid diagram.
    Mermaid,
    /// An Excalidraw board (JSON scene).
    Excalidraw,
    /// An `otto-scene3d` document (see `design_scene3d.rs`).
    Scene3d,
}

impl DesignFormat {
    pub const ALL: [DesignFormat; 4] = [
        DesignFormat::Html,
        DesignFormat::Mermaid,
        DesignFormat::Excalidraw,
        DesignFormat::Scene3d,
    ];

    /// Wire name (`html` | `mermaid` | `excalidraw` | `scene3d`) — also the
    /// `meta_json.format` value and the `MockupUpdated.format` payload.
    pub fn as_str(self) -> &'static str {
        match self {
            DesignFormat::Html => "html",
            DesignFormat::Mermaid => "mermaid",
            DesignFormat::Excalidraw => "excalidraw",
            DesignFormat::Scene3d => "scene3d",
        }
    }

    /// Served content type (must stay in `product_media::allowed_mime`).
    pub fn mime(self) -> &'static str {
        match self {
            DesignFormat::Html => "text/html",
            DesignFormat::Mermaid => "text/vnd.mermaid",
            DesignFormat::Excalidraw => "application/vnd.excalidraw+json",
            DesignFormat::Scene3d => "application/vnd.otto.scene3d+json",
        }
    }

    /// Reverse of `mime` — which format an existing attachment row is, if it is
    /// an agent-editable text artifact at all.
    pub fn from_mime(mime: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|f| f.mime() == mime)
    }

    /// Storage extension (with the dot), consistent with `mime`.
    pub fn ext(self) -> &'static str {
        match self {
            DesignFormat::Html => ".html",
            DesignFormat::Mermaid => ".mmd",
            DesignFormat::Excalidraw => ".excalidraw",
            DesignFormat::Scene3d => ".json",
        }
    }

    /// The working-file name the assist agent edits in its cwd.
    pub fn file_name(self) -> &'static str {
        match self {
            DesignFormat::Html => "design.html",
            DesignFormat::Mermaid => "design.mmd",
            DesignFormat::Excalidraw => "design.excalidraw",
            DesignFormat::Scene3d => "scene.json",
        }
    }

    /// Display filename for a freshly agent-minted artifact.
    pub fn title(self) -> String {
        match self {
            DesignFormat::Html => "AI screen.html".into(),
            DesignFormat::Mermaid => "AI diagram.mmd".into(),
            DesignFormat::Excalidraw => "AI board.excalidraw".into(),
            DesignFormat::Scene3d => "AI scene.json".into(),
        }
    }

    /// Language of the fenced code block the agent may print instead of editing
    /// the file (fallback source in `mockup_assist::resolve_source`).
    pub fn fence_lang(self) -> &'static str {
        match self {
            DesignFormat::Html => "html",
            DesignFormat::Mermaid => "mermaid",
            // Both JSON formats are fenced as ```json.
            DesignFormat::Excalidraw | DesignFormat::Scene3d => "json",
        }
    }

    /// `product_attachments.kind` for an artifact of this format minted by the
    /// assist / ingest paths. The two legacy formats keep `mockup` (existing rows,
    /// specs and the Assistant panel filter); the arena-native formats are
    /// `design`. The arena lists `mockup`, `design` and `image` together.
    pub fn attachment_kind(self) -> &'static str {
        match self {
            DesignFormat::Html | DesignFormat::Mermaid => "mockup",
            DesignFormat::Excalidraw | DesignFormat::Scene3d => "design",
        }
    }

    /// The arena's default asset group for this format (`meta_json.group`).
    pub fn default_group(self) -> &'static str {
        match self {
            DesignFormat::Html => "Screens",
            DesignFormat::Mermaid => "Diagrams",
            DesignFormat::Excalidraw => "Boards",
            DesignFormat::Scene3d => "3D",
        }
    }

    /// A minimal VALID placeholder so a brand-new artifact renders before the
    /// agent commits real content.
    pub fn base_stub(self, story_title: &str) -> String {
        match self {
            DesignFormat::Mermaid => "flowchart TD\n  A([\"Generating…\"])\n".to_string(),
            DesignFormat::Html => format!(
                "<!doctype html><html><head><meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{title}</title></head>\
<body style=\"font:15px/1.5 system-ui;padding:40px;color:#334155\">\
<p>Generating a screen for <strong>{title}</strong>…</p></body></html>\n",
                title = html_escape(story_title)
            ),
            DesignFormat::Excalidraw => {
                serde_json::json!({
                    "type": "excalidraw",
                    "version": 2,
                    "source": "otto",
                    "elements": [],
                    "appState": { "viewBackgroundColor": "#ffffff", "gridSize": 20 },
                    "files": {}
                })
                .to_string()
                    + "\n"
            }
            DesignFormat::Scene3d => crate::design_scene3d::empty_scene_json(story_title) + "\n",
        }
    }
}

impl FromStr for DesignFormat {
    type Err = Error;

    /// Case-insensitive, whitespace-trimmed; the empty string is NOT a format
    /// (callers default explicitly so "absent" and "wrong" stay distinguishable).
    fn from_str(s: &str) -> Result<Self, Error> {
        match s.trim().to_ascii_lowercase().as_str() {
            "html" => Ok(DesignFormat::Html),
            "mermaid" => Ok(DesignFormat::Mermaid),
            "excalidraw" => Ok(DesignFormat::Excalidraw),
            "scene3d" => Ok(DesignFormat::Scene3d),
            other => Err(Error::Invalid(format!(
                "unknown design format {other:?} (expected html | mermaid | excalidraw | scene3d)"
            ))),
        }
    }
}

impl fmt::Display for DesignFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parse an optional request field: absent / blank → `default`, otherwise a
/// strict parse (unknown → `Error::Invalid`, i.e. HTTP 400).
pub fn parse_or_default(raw: Option<&str>, default: DesignFormat) -> Result<DesignFormat, Error> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(default),
        Some(s) => s.parse(),
    }
}

pub(crate) fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_four_formats_case_insensitively() {
        assert_eq!("html".parse::<DesignFormat>().unwrap(), DesignFormat::Html);
        assert_eq!(
            " Mermaid ".parse::<DesignFormat>().unwrap(),
            DesignFormat::Mermaid
        );
        assert_eq!(
            "EXCALIDRAW".parse::<DesignFormat>().unwrap(),
            DesignFormat::Excalidraw
        );
        assert_eq!(
            "scene3d".parse::<DesignFormat>().unwrap(),
            DesignFormat::Scene3d
        );
    }

    #[test]
    fn unknown_format_is_invalid_not_html() {
        for bad in ["weird", "svg", "python", "", "  "] {
            let err = bad.parse::<DesignFormat>().unwrap_err();
            assert!(matches!(err, Error::Invalid(_)), "{bad:?} → {err:?}");
        }
        // Absent/blank defaults; present-but-wrong still rejects.
        assert_eq!(
            parse_or_default(None, DesignFormat::Html).unwrap(),
            DesignFormat::Html
        );
        assert_eq!(
            parse_or_default(Some("  "), DesignFormat::Mermaid).unwrap(),
            DesignFormat::Mermaid
        );
        assert!(parse_or_default(Some("other"), DesignFormat::Html).is_err());
    }

    #[test]
    fn mime_ext_and_names_are_consistent() {
        for f in DesignFormat::ALL {
            assert_eq!(DesignFormat::from_mime(f.mime()), Some(f));
            assert!(
                f.file_name().ends_with(f.ext()),
                "{f}: {} vs {}",
                f.file_name(),
                f.ext()
            );
            assert!(f.title().ends_with(f.ext()));
            assert_eq!(f.as_str().parse::<DesignFormat>().unwrap(), f);
            assert_eq!(
                serde_json::to_string(&f).unwrap(),
                format!("\"{}\"", f.as_str())
            );
        }
        assert_eq!(DesignFormat::from_mime("image/png"), None);
        assert_eq!(
            DesignFormat::Excalidraw.mime(),
            "application/vnd.excalidraw+json"
        );
        assert_eq!(
            DesignFormat::Scene3d.mime(),
            "application/vnd.otto.scene3d+json"
        );
        assert_eq!(DesignFormat::Scene3d.ext(), ".json");
        assert_eq!(DesignFormat::Excalidraw.ext(), ".excalidraw");
    }

    #[test]
    fn stubs_render_before_the_agent_commits() {
        assert!(DesignFormat::Html
            .base_stub("T<x>")
            .contains("<!doctype html>"));
        assert!(DesignFormat::Html.base_stub("T<x>").contains("T&lt;x&gt;"));
        assert!(DesignFormat::Mermaid
            .base_stub("T")
            .starts_with("flowchart"));
        let board: serde_json::Value =
            serde_json::from_str(&DesignFormat::Excalidraw.base_stub("T")).unwrap();
        assert_eq!(board["type"], "excalidraw");
        let scene: serde_json::Value =
            serde_json::from_str(&DesignFormat::Scene3d.base_stub("T")).unwrap();
        assert_eq!(scene["type"], "otto-scene3d");
        assert!(crate::design_scene3d::validate(&scene).is_ok());
    }

    #[test]
    fn legacy_formats_keep_kind_mockup() {
        assert_eq!(DesignFormat::Html.attachment_kind(), "mockup");
        assert_eq!(DesignFormat::Mermaid.attachment_kind(), "mockup");
        assert_eq!(DesignFormat::Excalidraw.attachment_kind(), "design");
        assert_eq!(DesignFormat::Scene3d.attachment_kind(), "design");
    }
}
