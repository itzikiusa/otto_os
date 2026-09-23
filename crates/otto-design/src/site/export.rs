//! Static export + loopback preview of a site version. The http layer
//! resolves what the document references (brand kit, 3D embeds, library
//! images — each to ONE concrete version) into [`Resolved`]; this module turns
//! that into files: one HTML document per page, a single `site.css` (the
//! brand-token block first, then the shared stylesheet), the assets, and
//! `otto-publish.json` — the pinned version set the publish row records.
//! Pure; unit-tested.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::render::{page_file_name, render_document, Css, EmbedInfo, RenderCtx};
use super::schema::SiteDoc;
use super::theme::Theme;
use super::SITE_CSS;

/// One entry of a publish's pinned set — `design_publishes.pinned_set_json`
/// is an ARRAY of these (the `[{artifact_id, version_id}]` shape the
/// retention prune reads to protect every version a publish rendered): the
/// site itself (`role: "site"`), its brand kit (`brand`), then every 3D
/// embed (`embed`) and library image (`image`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PinnedRef {
    /// `site` | `brand` | `embed` | `image`.
    pub role: String,
    /// The reference as the document wrote it (the site: `…@v<seq>`).
    pub uri: String,
    pub artifact_id: String,
    pub version_id: Option<String>,
    pub seq: Option<i64>,
    pub title: String,
    /// What the document asked for: `follow_approved` | `follow_latest` | `pinned`.
    pub policy: String,
    /// Unresolvable (missing artifact / version, no access, wrong format).
    pub missing: bool,
}

/// The entry for `role` (`site` / `brand`), if any.
pub fn pinned_role<'a>(set: &'a [PinnedRef], role: &str) -> Option<&'a PinnedRef> {
    set.iter().find(|r| r.role == role)
}

/// The version a publish pinned for reference `uri` (embeds / images).
pub fn pinned_version(set: &[PinnedRef], uri: &str) -> Option<String> {
    set.iter()
        .find(|r| r.role != "site" && r.role != "brand" && r.uri == uri)
        .and_then(|r| r.version_id.clone())
}

/// Everything a render needs beyond the document.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub theme: Theme,
    pub embeds: HashMap<String, EmbedInfo>,
    /// `otto://design/…` image → URL (an `assets/…` path, or a data URI).
    pub assets: HashMap<String, String>,
    /// Asset files for the archive (`assets/<file>`, bytes).
    pub files: Vec<(String, Vec<u8>)>,
    /// The pinned set (site first).
    pub pinned: Vec<PinnedRef>,
    pub warnings: Vec<String>,
}

/// The one stylesheet: brand tokens, page basics, then the shared site.css.
pub fn site_css(theme: &Theme) -> String {
    format!(
        "/* Otto Site Studio: brand tokens first (--brand-<group>-<name>), then the site stylesheet. */\n{}{}{}",
        theme.css(),
        theme.document_css(),
        SITE_CSS
    )
}

fn ctx_for<'a>(r: &'a Resolved, pages: HashMap<String, String>, home: String) -> RenderCtx<'a> {
    RenderCtx {
        theme: &r.theme,
        pages,
        home_href: home,
        assets: r.assets.clone(),
        embeds: r.embeds.clone(),
    }
}

/// The static site: `index.html` + `<slug>.html` per page, `site.css`, the
/// assets and `otto-publish.json`.
pub fn static_files(doc: &SiteDoc, r: &Resolved) -> Vec<(String, Vec<u8>)> {
    let pages: HashMap<String, String> = doc
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| (p.id.clone(), page_file_name(doc, i)))
        .collect();
    let ctx = ctx_for(r, pages, "index.html".into());
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..doc.pages.len() {
        let name = page_file_name(doc, i);
        if out.iter().any(|(n, _)| *n == name) {
            continue;
        }
        let html = render_document(doc, i, &ctx, Css::Link("site.css"));
        out.push((name, html.into_bytes()));
    }
    out.push(("site.css".into(), site_css(&r.theme).into_bytes()));
    for (name, bytes) in &r.files {
        if !out.iter().any(|(n, _)| n == name) {
            out.push((name.clone(), bytes.clone()));
        }
    }
    let manifest = serde_json::to_vec_pretty(&r.pinned).unwrap_or_default();
    out.push(("otto-publish.json".into(), manifest));
    out
}

/// The URL segment of a page in the loopback preview (slug, else id).
pub fn preview_segment(doc: &SiteDoc, idx: usize) -> Option<String> {
    let p = doc.pages.get(idx)?;
    if idx == 0 {
        return None;
    }
    Some(if p.slug.is_empty() {
        p.id.clone()
    } else {
        p.slug.clone()
    })
}

/// The page a preview path segment names (slug or id; none = home).
pub fn page_index(doc: &SiteDoc, segment: Option<&str>) -> Option<usize> {
    match segment.map(str::trim).filter(|s| !s.is_empty()) {
        None => (!doc.pages.is_empty()).then_some(0),
        Some(seg) => doc
            .pages
            .iter()
            .position(|p| (!p.slug.is_empty() && p.slug == seg) || p.id == seg),
    }
}

/// One page as a self-contained document (inline CSS, data-URI assets) whose
/// page links stay inside the preview route: `{base}` is
/// `/api/v1/design/artifacts/<id>/preview`, `{query}` e.g. `?publish=<id>`.
pub fn preview_html(doc: &SiteDoc, idx: usize, r: &Resolved, base: &str, query: &str) -> String {
    let pages: HashMap<String, String> = doc
        .pages
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let url = match preview_segment(doc, i) {
                Some(seg) => format!("{base}/{seg}{query}"),
                None => format!("{base}{query}"),
            };
            (p.id.clone(), url)
        })
        .collect();
    let ctx = ctx_for(r, pages, format!("{base}{query}"));
    let css = site_css(&r.theme);
    render_document(doc, idx, &ctx, Css::Inline(&css))
}

/// `application/zip` download name: `<site-slug>-v<seq>.zip`.
pub fn zip_name(title: &str, seq: i64) -> String {
    let slug = if title.chars().any(|c| c.is_ascii_alphanumeric()) {
        crate::brand::export::slug(title)
    } else {
        "site".to_string()
    };
    format!("{slug}-v{seq}.zip")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn resolved() -> Resolved {
        let mut embeds = HashMap::new();
        embeds.insert(
            "otto://design/CARD@approved".to_string(),
            EmbedInfo {
                title: "Rewards Card 3D".into(),
                seq: Some(7),
                policy: "follow_approved".into(),
                poster: Some("assets/CARD-v7-poster.png".into()),
                broken: false,
            },
        );
        let kit = json!({ "$schema": "otto-brand/1", "name": "Acme",
            "color": { "primary": { "$value": "#5B3DF5" }, "accent": { "$value": "#FFB547" } } });
        Resolved {
            theme: Theme::from_kit(Some(&kit)),
            embeds,
            assets: HashMap::new(),
            files: vec![(
                "assets/CARD-v7-poster.png".into(),
                vec![0x89, 0x50, 0x4E, 0x47],
            )],
            pinned: vec![
                PinnedRef {
                    role: "site".into(),
                    uri: "otto://design/SITE@v14".into(),
                    artifact_id: "SITE".into(),
                    version_id: Some("V14".into()),
                    seq: Some(14),
                    title: "Rewards+".into(),
                    policy: "pinned".into(),
                    missing: false,
                },
                PinnedRef {
                    role: "embed".into(),
                    uri: "otto://design/CARD@approved".into(),
                    artifact_id: "CARD".into(),
                    version_id: Some("CARD-V7".into()),
                    seq: Some(7),
                    title: "Rewards Card 3D".into(),
                    policy: "follow_approved".into(),
                    missing: false,
                },
            ],
            warnings: vec![],
        }
    }

    fn doc() -> SiteDoc {
        serde_json::from_value(json!({
            "type": "otto-site", "version": 1, "title": "Rewards+",
            "pages": [
                { "id": "home", "title": "Home", "slug": "", "sections": [
                    { "id": "nav", "block": "nav/bar", "props": { "logo": "acme" },
                      "blocks": [{ "id": "l1", "block": "item/link", "props": { "label": "Tiers", "href": "page:tiers" } }] },
                    { "id": "hero", "block": "hero/split", "props": { "headline": "Every purchase moves you up." },
                      "blocks": [{ "id": "card", "block": "embed/3d", "props": { "src": "otto://design/CARD@approved" } }] }
                ]},
                { "id": "tiers", "title": "Tiers", "slug": "tiers", "sections": [] },
                { "id": "about", "title": "About", "slug": "", "sections": [] }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn the_static_site_has_pages_one_stylesheet_with_tokens_assets_and_the_manifest() {
        let files = static_files(&doc(), &resolved());
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "index.html",
                "tiers.html",
                "about.html",
                "site.css",
                "assets/CARD-v7-poster.png",
                "otto-publish.json"
            ],
            "a page without a slug is exported under its id"
        );
        let get = |n: &str| {
            String::from_utf8(files.iter().find(|(x, _)| x == n).unwrap().1.clone()).unwrap()
        };
        let index = get("index.html");
        assert!(index.contains("<link rel=\"stylesheet\" href=\"site.css\">"));
        assert!(
            index.contains("href=\"tiers.html\""),
            "page links become files"
        );
        assert!(
            index.contains("src=\"assets/CARD-v7-poster.png\""),
            "embeds use the exported poster"
        );
        let css = get("site.css");
        let tokens_at = css
            .find("--brand-color-primary: #5B3DF5;")
            .expect("brand tokens as CSS variables");
        let shared_at = css.find(".os-site *").expect("the shared stylesheet");
        assert!(tokens_at < shared_at, "tokens come first");
        assert!(css.contains("--os-primary: var(--brand-color-primary);"));
        assert!(css.contains("body { margin: 0;"));
        let manifest: Vec<PinnedRef> = serde_json::from_str(&get("otto-publish.json")).unwrap();
        assert_eq!(pinned_role(&manifest, "site").unwrap().seq, Some(14));
        assert_eq!(
            pinned_version(&manifest, "otto://design/CARD@approved").as_deref(),
            Some("CARD-V7")
        );
        assert_eq!(
            pinned_version(&manifest, "otto://design/SITE@v14"),
            None,
            "the site entry is not a reference"
        );
    }

    #[test]
    fn previews_inline_the_css_and_keep_links_inside_the_route() {
        let d = doc();
        let base = "/api/v1/design/artifacts/SITE/preview";
        let html = preview_html(&d, 0, &resolved(), base, "?publish=P1");
        assert!(html.contains("<style>\n/* Otto Site Studio"));
        assert!(
            html.contains("href=\"/api/v1/design/artifacts/SITE/preview/tiers?publish=P1\""),
            "{html}"
        );
        assert!(html.contains(
            "class=\"os-logo\" href=\"/api/v1/design/artifacts/SITE/preview?publish=P1\""
        ));
        assert_eq!(page_index(&d, None), Some(0));
        assert_eq!(page_index(&d, Some("tiers")), Some(1));
        assert_eq!(
            page_index(&d, Some("about")),
            Some(2),
            "a page without a slug is addressed by id"
        );
        assert_eq!(page_index(&d, Some("nope")), None);
        assert_eq!(preview_segment(&d, 2).as_deref(), Some("about"));
        assert_eq!(
            zip_name("Rewards+ landing page", 14),
            "rewards-landing-page-v14.zip"
        );
    }
}
