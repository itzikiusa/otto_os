//! The `otto-site` v1 document, typed leniently (every field defaults) for the
//! renderer and exporter. Saves are gated by [`super::validate`] on the raw
//! JSON first, so a stored version always deserializes. Mirrored by
//! `ui/src/modules/design-hall/site/engine/types.ts` + `catalog.ts`.

use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// `type` of every site document.
pub const SITE_TYPE: &str = "otto-site";

/// Section blocks (`<family>/<variant>`) the renderer knows — the UI catalog's
/// `SECTIONS` in the same order.
pub const SECTION_BLOCKS: &[&str] = &[
    "nav/bar",
    "hero/split",
    "hero/centered",
    "hero/fullbleed",
    "hero/stacked",
    "features/grid",
    "features/alternating",
    "features/bento",
    "features/steps",
    "features/stats",
    "social/logos",
    "social/testimonials",
    "social/quote",
    "pricing/tiers",
    "pricing/compare",
    "pricing/single",
    "faq/accordion",
    "faq/grid",
    "cta/band",
    "cta/split",
    "cta/card",
    "content/text",
    "media/3d-embed",
    "media/video",
    "media/gallery",
    "footer/columns",
    "footer/simple",
];

/// Child blocks (repeated items and media) — the UI catalog's `ITEMS`.
pub const ITEM_BLOCKS: &[&str] = &[
    "item/link",
    "item/feature",
    "item/step",
    "item/stat",
    "item/logo",
    "item/testimonial",
    "item/tier",
    "item/faq",
    "item/column",
    "embed/3d",
    "embed/image",
];

/// Child blocks a section renders in its media slot (first match wins).
pub fn media_slot(block: &str) -> &'static [&'static str] {
    match block {
        "hero/split" | "hero/fullbleed" => &["embed/3d", "embed/image"],
        "hero/centered" | "hero/stacked" => &["embed/image", "embed/3d"],
        "media/3d-embed" => &["embed/3d"],
        _ => &[],
    }
}

pub const MOTIONS: &[&str] = &["none", "fade-up", "scroll-reveal", "parallax", "tilt-hover"];
pub const SPACINGS: &[&str] = &["s", "m", "l", "xl"];
pub const ALIGNS: &[&str] = &["left", "center"];
pub const MIN_HEIGHTS: &[&str] = &["auto", "80vh", "100vh"];
pub const BREAKPOINTS: &[&str] = &["desktop", "tablet", "mobile"];
pub const STACKS: &[&str] = &["media-first", "media-last"];
pub const GRADIENTS: &[&str] = &["soft", "primary", "ink", "sunset"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SiteDoc {
    #[serde(rename = "type")]
    pub kind: String,
    pub version: i64,
    pub title: String,
    /// `otto://design/<brand kit>[@…]` (a `uses_tokens` link).
    pub brand: Option<String>,
    pub settings: Settings,
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub domain: String,
    pub lang: String,
    pub description: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Page {
    pub id: String,
    pub title: String,
    /// `""` = the home page.
    pub slug: String,
    pub description: String,
    pub sections: Vec<Section>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Section {
    pub id: String,
    pub block: String,
    pub name: String,
    pub props: Map<String, Value>,
    pub style: Style,
    pub responsive: Responsive,
    pub blocks: Vec<Block>,
    pub derived_from: Option<String>,
    pub hidden: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Block {
    pub id: String,
    pub block: String,
    pub props: Map<String, Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Style {
    /// `token:color.<name>` | `gradient:<preset>` | `#hex` | `""`.
    pub background: String,
    pub spacing: String,
    pub align: String,
    pub motion: String,
    pub min_height: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Responsive {
    pub hide: Vec<String>,
    pub stack: String,
    pub mobile_align: String,
}

impl SiteDoc {
    /// Validate, then type a stored document.
    pub fn parse(bytes: &[u8]) -> Result<SiteDoc> {
        let v: Value = serde_json::from_slice(bytes)
            .map_err(|e| Error::Invalid(format!("otto-site content is not valid JSON: {e}")))?;
        super::validate(&v)?;
        serde_json::from_value(v).map_err(|e| {
            Error::Invalid(format!("otto-site document does not match the schema: {e}"))
        })
    }

    /// Every `otto://design/…` reference the page RENDERS, with the role it
    /// plays (`embed` for 3D, `image` for pictures) — what a publish pins.
    pub fn rendered_refs(&self) -> Vec<(String, &'static str)> {
        let mut out: Vec<(String, &'static str)> = Vec::new();
        let mut add = |uri: &str, role: &'static str| {
            let u = uri.trim();
            if u.starts_with(crate::uri::PREFIX) && !out.iter().any(|(x, _)| x == u) {
                out.push((u.to_string(), role));
            }
        };
        for p in &self.pages {
            for s in p.sections.iter().filter(|s| !s.hidden) {
                for k in ["image", "poster"] {
                    if let Some(v) = s.props.get(k).and_then(Value::as_str) {
                        add(v, "image");
                    }
                }
                for b in &s.blocks {
                    let src = b.props.get("src").and_then(Value::as_str).unwrap_or("");
                    if b.block == "embed/3d" {
                        add(src, "embed");
                    } else if b.block == "embed/image" {
                        add(src, "image");
                    }
                    if let Some(v) = b.props.get("image").and_then(Value::as_str) {
                        add(v, "image");
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_names_are_unique_and_media_slots_use_known_blocks() {
        let mut seen = std::collections::HashSet::new();
        for b in SECTION_BLOCKS.iter().chain(ITEM_BLOCKS) {
            assert!(seen.insert(*b), "duplicate block {b}");
        }
        assert!(SECTION_BLOCKS.len() >= 25);
        for s in SECTION_BLOCKS {
            for m in media_slot(s) {
                assert!(ITEM_BLOCKS.contains(m), "{s} media {m}");
            }
        }
    }

    #[test]
    fn parse_is_lenient_on_missing_fields_and_collects_rendered_refs() {
        let doc = serde_json::json!({
            "type": "otto-site", "version": 1,
            "pages": [{ "id": "home", "sections": [
                { "id": "hero", "block": "hero/split", "props": { "headline": "Hi" },
                  "blocks": [{ "id": "card", "block": "embed/3d", "props": { "src": "otto://design/CARD@approved" } }] },
                { "id": "g", "block": "media/gallery",
                  "blocks": [{ "id": "i1", "block": "embed/image", "props": { "src": "otto://design/IMG" } }] },
                { "id": "off", "block": "media/video", "hidden": true, "props": { "poster": "otto://design/HIDDEN" } }
            ]}]
        });
        let d = SiteDoc::parse(doc.to_string().as_bytes()).unwrap();
        assert_eq!(d.pages[0].slug, "");
        assert_eq!(d.pages[0].sections[0].style.spacing, "");
        let refs = d.rendered_refs();
        assert_eq!(
            refs,
            vec![
                ("otto://design/CARD@approved".to_string(), "embed"),
                ("otto://design/IMG".to_string(), "image"),
            ],
            "hidden sections are never rendered, so never pinned"
        );
        assert!(SiteDoc::parse(b"{\"type\":\"nope\",\"version\":1}").is_err());
    }
}
