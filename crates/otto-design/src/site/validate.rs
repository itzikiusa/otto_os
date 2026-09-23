//! The `otto-site` v1 validator — run on every content write through
//! `format::validate`, so an invalid document (an agent's bad edit included)
//! is refused with the JSON path of each problem. Structural only: ids,
//! known blocks, enums, bounded sizes and URL hygiene; copy is never judged.
//! Mirrored by `ui/src/modules/design-hall/site/engine/validate.ts`.

use std::collections::HashSet;

use otto_core::{Error, Result};
use serde_json::{Map, Value};

use super::schema::{
    ALIGNS, BREAKPOINTS, GRADIENTS, ITEM_BLOCKS, MIN_HEIGHTS, MOTIONS, SECTION_BLOCKS, SITE_TYPE,
    SPACINGS, STACKS,
};
use crate::uri::DesignUri;

pub const MAX_PAGES: usize = 50;
pub const MAX_SECTIONS_PER_PAGE: usize = 200;
pub const MAX_BLOCKS_PER_SECTION: usize = 100;
pub const MAX_NODES: usize = 5_000;
pub const MAX_TEXT: usize = 20_000;
pub const MAX_LIST_ITEMS: usize = 100;
const MAX_ISSUES: usize = 20;

/// Refuse the document when it has any issue (all of them in the message).
pub fn validate(v: &Value) -> Result<()> {
    let issues = issues(v);
    if issues.is_empty() {
        Ok(())
    } else {
        Err(Error::Invalid(format!("otto-site: {}", issues.join("; "))))
    }
}

struct Acc {
    out: Vec<String>,
}

impl Acc {
    fn add(&mut self, path: &str, msg: &str) {
        if self.out.len() < MAX_ISSUES {
            if path.is_empty() {
                self.out.push(msg.to_string());
            } else {
                self.out.push(format!("{path}: {msg}"));
            }
        }
    }
}

/// `[A-Za-z0-9_-]{1,64}`.
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Lower-case words joined by single dashes (`tiers`, `about-us`), ≤ 64.
pub fn valid_slug(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && !s.starts_with('-')
        && !s.ends_with('-')
        && !s.contains("--")
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn is_hex_color(s: &str) -> bool {
    let Some(h) = s.strip_prefix('#') else {
        return false;
    };
    matches!(h.len(), 3 | 6 | 8) && h.chars().all(|c| c.is_ascii_hexdigit())
}

/// `token:color.<name>` with a Brand Kit token name.
fn is_color_token(s: &str) -> bool {
    let Some(name) = s.strip_prefix("token:color.") else {
        return false;
    };
    crate::brand::doc::valid_token_name(name)
}

/// The URL scheme of `v` (lower-case, controls/spaces ignored), `//` for a
/// protocol-relative URL, `None` for a relative one.
fn scheme(v: &str) -> Option<String> {
    let probe: String = v
        .chars()
        .filter(|c| (*c as u32) > 0x20)
        .collect::<String>()
        .to_ascii_lowercase();
    if probe.starts_with("//") || probe.starts_with('\\') {
        return Some("//".into());
    }
    let mut chars = probe.chars();
    let first = chars.next()?;
    if !first.is_ascii_lowercase() {
        return None;
    }
    let mut name = String::from(first);
    for c in chars {
        if c == ':' {
            return Some(name);
        }
        if c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-') {
            name.push(c);
        } else {
            return None;
        }
    }
    None
}

/// `data:image/(png|jpeg|gif|webp);base64,<base64>`.
pub fn is_data_image(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("data:image/") else {
        return false;
    };
    let Some(payload) = ["png", "jpeg", "gif", "webp"].iter().find_map(|t| {
        rest.strip_prefix(t)
            .and_then(|r| r.strip_prefix(";base64,"))
    }) else {
        return false;
    };
    !payload.is_empty()
        && payload
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '='))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UrlKind {
    Href,
    Src,
    Video,
}

/// Why a URL-ish prop is refused (`None` = fine).
fn url_problem(kind: UrlKind, v: &str) -> Option<String> {
    let s = v.trim();
    if s.is_empty() {
        return None;
    }
    if kind == UrlKind::Href {
        if let Some(id) = s.strip_prefix("page:") {
            return if valid_id(id) {
                None
            } else {
                Some("page links look like page:<page id>".into())
            };
        }
    }
    if s.starts_with("otto://") {
        // A design reference (extracted as a link; a page link to another
        // design renders as `#` — it is a reference, not navigation).
        return if DesignUri::parse(s).is_some() {
            None
        } else {
            Some("not a valid otto://design/<id>[@approved|@latest|@vN] reference".into())
        };
    }
    let sc = scheme(s)?;
    let ok = match kind {
        UrlKind::Href => matches!(sc.as_str(), "http" | "https" | "mailto" | "tel"),
        UrlKind::Src => sc == "https" || (sc == "data" && is_data_image(s)),
        UrlKind::Video => sc == "https",
    };
    if ok {
        None
    } else if sc == "//" {
        Some("the protocol-relative scheme is not allowed".into())
    } else {
        Some(format!("the {sc}: scheme is not allowed"))
    }
}

fn url_kind(key: &str) -> Option<UrlKind> {
    if key == "href" || key.ends_with("_href") || key == "form_action" {
        Some(UrlKind::Href)
    } else if key == "image" || key == "poster" {
        Some(UrlKind::Src)
    } else {
        None
    }
}

/// `path.key` (just `key` at the root).
fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

fn check_string(o: &Map<String, Value>, key: &str, max: usize, path: &str, acc: &mut Acc) {
    match o.get(key) {
        None => {}
        Some(Value::String(s)) if s.chars().count() <= max => {}
        Some(_) => acc.add(
            &join(path, key),
            &format!("must be a string of at most {max} characters"),
        ),
    }
}

fn check_enum(o: &Map<String, Value>, key: &str, allowed: &[&str], path: &str, acc: &mut Acc) {
    match o.get(key) {
        None => {}
        Some(Value::String(s)) if s.is_empty() || allowed.contains(&s.as_str()) => {}
        Some(_) => acc.add(
            &join(path, key),
            &format!("must be one of {}", allowed.join(", ")),
        ),
    }
}

fn check_id(v: Option<&Value>, path: &str, seen: &mut HashSet<String>, acc: &mut Acc) {
    match v.and_then(Value::as_str) {
        Some(id) if valid_id(id) => {
            if !seen.insert(id.to_string()) {
                acc.add(
                    path,
                    &format!(
                        "duplicate id \"{id}\" (section and block ids are unique per document)"
                    ),
                );
            }
        }
        _ => acc.add(path, "must be 1–64 letters, digits, _ or -"),
    }
}

fn check_props(props: Option<&Value>, path: &str, media: bool, acc: &mut Acc) {
    let Some(props) = props else {
        return;
    };
    let Some(map) = props.as_object() else {
        acc.add(path, "must be an object");
        return;
    };
    for (k, v) in map {
        let p = format!("{path}.{k}");
        match v {
            Value::String(s) => {
                if s.chars().count() > MAX_TEXT {
                    acc.add(&p, &format!("at most {MAX_TEXT} characters"));
                }
                let kind = if k == "src" {
                    Some(if media { UrlKind::Src } else { UrlKind::Video })
                } else {
                    url_kind(k)
                };
                if let Some(kind) = kind {
                    if let Some(why) = url_problem(kind, s) {
                        acc.add(&p, &why);
                    }
                }
            }
            Value::Number(_) | Value::Bool(_) | Value::Null => {}
            Value::Array(items) => {
                if items.len() > MAX_LIST_ITEMS {
                    acc.add(&p, &format!("at most {MAX_LIST_ITEMS} entries"));
                }
                for (i, x) in items.iter().enumerate() {
                    match x {
                        Value::String(s) => {
                            if s.chars().count() > 2_000 {
                                acc.add(&format!("{p}[{i}]"), "at most 2000 characters");
                            }
                        }
                        Value::Object(o)
                            if o.get("label").is_some_and(Value::is_string)
                                && o.get("href").is_none_or(Value::is_string) =>
                        {
                            if let Some(h) = o.get("href").and_then(Value::as_str) {
                                if let Some(why) = url_problem(UrlKind::Href, h) {
                                    acc.add(&format!("{p}[{i}].href"), &why);
                                }
                            }
                        }
                        _ => acc.add(
                            &format!("{p}[{i}]"),
                            "list entries are strings or {label, href} links",
                        ),
                    }
                }
            }
            Value::Object(_) => acc.add(&p, "props are strings, numbers, booleans or lists"),
        }
    }
}

fn check_style(style: Option<&Value>, path: &str, acc: &mut Acc) {
    let Some(style) = style else {
        return;
    };
    let Some(o) = style.as_object() else {
        acc.add(path, "must be an object");
        return;
    };
    match o.get("background") {
        None => {}
        Some(Value::String(bg)) if bg.is_empty() => {}
        Some(Value::String(bg))
            if is_hex_color(bg)
                || is_color_token(bg)
                || bg
                    .strip_prefix("gradient:")
                    .is_some_and(|g| GRADIENTS.contains(&g)) => {}
        Some(_) => acc.add(
            &format!("{path}.background"),
            "must be token:color.<name>, gradient:<soft|primary|ink|sunset> or #hex",
        ),
    }
    check_enum(o, "spacing", SPACINGS, path, acc);
    check_enum(o, "align", ALIGNS, path, acc);
    check_enum(o, "motion", MOTIONS, path, acc);
    check_enum(o, "min_height", MIN_HEIGHTS, path, acc);
}

fn check_responsive(r: Option<&Value>, path: &str, acc: &mut Acc) {
    let Some(r) = r else {
        return;
    };
    let Some(o) = r.as_object() else {
        acc.add(path, "must be an object");
        return;
    };
    if let Some(h) = o.get("hide") {
        let ok = h.as_array().is_some_and(|a| {
            a.iter()
                .all(|x| x.as_str().is_some_and(|s| BREAKPOINTS.contains(&s)))
        });
        if !ok {
            acc.add(
                &format!("{path}.hide"),
                &format!("must be a list of {}", BREAKPOINTS.join(", ")),
            );
        }
    }
    check_enum(o, "stack", STACKS, path, acc);
    check_enum(o, "mobile_align", ALIGNS, path, acc);
}

fn check_uri(v: Option<&Value>, path: &str, what: &str, acc: &mut Acc) {
    match v {
        None => {}
        Some(Value::String(s)) if s.is_empty() || DesignUri::parse(s).is_some() => {}
        Some(_) => acc.add(path, what),
    }
}

/// Every issue of a document (≤ 20), as `path: message`.
pub fn issues(v: &Value) -> Vec<String> {
    let mut acc = Acc { out: Vec::new() };
    let Some(root) = v.as_object() else {
        acc.add("", "the document must be a JSON object");
        return acc.out;
    };
    if root.get("type").and_then(Value::as_str) != Some(SITE_TYPE) {
        acc.add("type", "must be \"otto-site\"");
    }
    // `version` may be omitted (read as 1); anything else is a future format.
    if root.get("version").is_some_and(|v| v.as_i64() != Some(1)) {
        acc.add("version", "must be 1");
    }
    check_string(root, "title", 300, "", &mut acc);
    check_uri(
        root.get("brand"),
        "brand",
        "must be an otto://design/<brand kit id>[@approved|@latest|@vN] reference",
        &mut acc,
    );
    if let Some(s) = root.get("settings") {
        match s.as_object() {
            Some(o) => {
                for k in ["domain", "lang", "description"] {
                    check_string(o, k, 1_000, "settings", &mut acc);
                }
            }
            None => acc.add("settings", "must be an object"),
        }
    }
    let Some(pages) = root.get("pages") else {
        return acc.out;
    };
    let Some(pages) = pages.as_array() else {
        acc.add("pages", "must be an array");
        return acc.out;
    };
    if pages.len() > MAX_PAGES {
        acc.add("pages", &format!("at most {MAX_PAGES} pages"));
    }
    let mut page_ids: HashSet<String> = HashSet::new();
    let mut slugs: HashSet<String> = HashSet::new();
    let mut node_ids: HashSet<String> = HashSet::new();
    let mut nodes = 0usize;
    for (pi, page) in pages.iter().enumerate() {
        let pp = format!("pages[{pi}]");
        let Some(page) = page.as_object() else {
            acc.add(&pp, "must be an object");
            continue;
        };
        match page.get("id").and_then(Value::as_str) {
            Some(id) if valid_id(id) => {
                if !page_ids.insert(id.to_string()) {
                    acc.add(&format!("{pp}.id"), &format!("duplicate page id \"{id}\""));
                }
            }
            _ => acc.add(&format!("{pp}.id"), "must be 1–64 letters, digits, _ or -"),
        }
        check_string(page, "title", 200, &pp, &mut acc);
        check_string(page, "description", 1_000, &pp, &mut acc);
        match page.get("slug") {
            None => {}
            Some(Value::String(s)) if s.is_empty() => {}
            Some(Value::String(s)) if valid_slug(s) => {
                if s == "index" {
                    acc.add(
                        &format!("{pp}.slug"),
                        "\"index\" is reserved for the home page",
                    );
                } else if !slugs.insert(s.clone()) {
                    acc.add(&format!("{pp}.slug"), &format!("duplicate slug \"{s}\""));
                }
            }
            Some(_) => acc.add(
                &format!("{pp}.slug"),
                "must be empty (home) or lowercase words joined by -",
            ),
        }
        let Some(sections) = page.get("sections") else {
            continue;
        };
        let Some(sections) = sections.as_array() else {
            acc.add(&format!("{pp}.sections"), "must be an array");
            continue;
        };
        if sections.len() > MAX_SECTIONS_PER_PAGE {
            acc.add(
                &format!("{pp}.sections"),
                &format!("at most {MAX_SECTIONS_PER_PAGE} sections per page"),
            );
        }
        for (si, sec) in sections.iter().enumerate() {
            let sp = format!("{pp}.sections[{si}]");
            let Some(sec) = sec.as_object() else {
                acc.add(&sp, "must be an object");
                continue;
            };
            nodes += 1;
            check_id(sec.get("id"), &format!("{sp}.id"), &mut node_ids, &mut acc);
            // A section without a block is kept (and renders nothing); a
            // named block must be one the renderer knows.
            match sec.get("block").map(|b| b.as_str()) {
                None => {}
                Some(Some(b)) if SECTION_BLOCKS.contains(&b) => {}
                Some(other) => acc.add(
                    &format!("{sp}.block"),
                    &format!(
                        "unknown section block {} (known: {})",
                        other
                            .map(|s| format!("\"{s}\""))
                            .unwrap_or_else(|| "(none)".into()),
                        SECTION_BLOCKS.join(", ")
                    ),
                ),
            }
            check_string(sec, "name", 120, &sp, &mut acc);
            if sec.get("hidden").is_some_and(|h| !h.is_boolean()) {
                acc.add(&format!("{sp}.hidden"), "must be true or false");
            }
            check_uri(
                sec.get("derived_from"),
                &format!("{sp}.derived_from"),
                "must be an otto://design/<site>@v<n>#<section> reference",
                &mut acc,
            );
            check_props(sec.get("props"), &format!("{sp}.props"), false, &mut acc);
            check_style(sec.get("style"), &format!("{sp}.style"), &mut acc);
            check_responsive(sec.get("responsive"), &format!("{sp}.responsive"), &mut acc);
            let Some(blocks) = sec.get("blocks") else {
                continue;
            };
            let Some(blocks) = blocks.as_array() else {
                acc.add(&format!("{sp}.blocks"), "must be an array");
                continue;
            };
            if blocks.len() > MAX_BLOCKS_PER_SECTION {
                acc.add(
                    &format!("{sp}.blocks"),
                    &format!("at most {MAX_BLOCKS_PER_SECTION} blocks per section"),
                );
            }
            for (bi, b) in blocks.iter().enumerate() {
                let bp = format!("{sp}.blocks[{bi}]");
                let Some(b) = b.as_object() else {
                    acc.add(&bp, "must be an object");
                    continue;
                };
                nodes += 1;
                check_id(b.get("id"), &format!("{bp}.id"), &mut node_ids, &mut acc);
                let kind = b.get("block").and_then(Value::as_str);
                match b.get("block").map(|x| x.as_str()) {
                    None => {}
                    Some(Some(k)) if ITEM_BLOCKS.contains(&k) => {}
                    Some(other) => acc.add(
                        &format!("{bp}.block"),
                        &format!(
                            "unknown block {} (known: {})",
                            other
                                .map(|s| format!("\"{s}\""))
                                .unwrap_or_else(|| "(none)".into()),
                            ITEM_BLOCKS.join(", ")
                        ),
                    ),
                }
                let media = matches!(kind, Some("embed/3d") | Some("embed/image"));
                check_props(b.get("props"), &format!("{bp}.props"), media, &mut acc);
            }
        }
    }
    if nodes > MAX_NODES {
        acc.add(
            "pages",
            &format!("at most {MAX_NODES} sections and blocks in total"),
        );
    }
    acc.out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn doc(sections: Value) -> Value {
        json!({ "type": "otto-site", "version": 1, "title": "T",
                "pages": [{ "id": "home", "title": "Home", "slug": "", "sections": sections }] })
    }

    #[test]
    fn the_stored_default_and_a_real_page_are_valid() {
        assert!(validate(&json!({ "type": "otto-site", "version": 1 })).is_ok());
        let d = doc(json!([
            { "id": "hero", "block": "hero/split",
              "props": { "headline": "Every purchase moves you up.", "primary_href": "page:tiers",
                         "secondary_href": "https://acme.example", "trust": "★ 4.8" },
              "style": { "background": "token:color.surface-alt", "spacing": "l", "motion": "fade-up" },
              "responsive": { "hide": ["mobile"], "stack": "media-first" },
              "blocks": [{ "id": "card", "block": "embed/3d",
                           "props": { "src": "otto://design/CARD@approved", "auto_rotate": true } }] },
            { "id": "faq", "block": "faq/accordion", "derived_from": "otto://design/SITE2@v12#faq",
              "blocks": [{ "id": "q1", "block": "item/faq", "props": { "question": "Free?", "answer": "Yes." } }] },
            { "id": "foot", "block": "footer/simple",
              "props": { "links": [{ "label": "Privacy", "href": "#" }, { "label": "Mail", "href": "mailto:a@b.c" }] } }
        ]));
        assert_eq!(issues(&d), Vec::<String>::new());
    }

    #[test]
    fn loose_documents_other_tools_write_still_save() {
        // No version, sections without a block, design references in href —
        // what the graph's own tests and agents without the schema produce.
        let d = json!({ "type": "otto-site", "brand": "otto://design/KIT",
            "pages": [{ "id": "home", "sections": [
                { "id": "hero", "props": { "src": "otto://design/CARD@approved", "bg": "token:color.primary" } },
                { "id": "faq", "props": { "href": "otto://design/MISSING01" } },
                { "id": "bad", "props": { "href": "otto://design/CARD@v9" } }
            ]}] });
        assert_eq!(issues(&d), Vec::<String>::new());
        assert!(!issues(&json!({ "type": "otto-site", "version": 2 })).is_empty());
    }

    #[test]
    fn structure_ids_blocks_and_enums_are_checked() {
        let bad = issues(&json!({ "type": "nope", "version": 2 }));
        assert!(
            bad[0].starts_with("type:") && bad[1].starts_with("version:"),
            "{bad:?}"
        );
        let dup = doc(json!([
            { "id": "a", "block": "cta/band" },
            { "id": "a", "block": "hero/fancy" }
        ]));
        let got = issues(&dup).join("\n");
        assert!(got.contains("duplicate id \"a\""), "{got}");
        assert!(
            got.contains("sections[1].block: unknown section block \"hero/fancy\""),
            "{got}"
        );
        let style = doc(json!([{ "id": "s", "block": "cta/band",
            "style": { "background": "red", "motion": "spin", "spacing": "xxl" },
            "responsive": { "hide": ["watch"] } }]));
        let got = issues(&style).join("\n");
        for p in [
            "style.background",
            "style.motion",
            "style.spacing",
            "responsive.hide",
        ] {
            assert!(got.contains(p), "{p} in {got}");
        }
        let slugs = json!({ "type": "otto-site", "version": 1, "pages": [
            { "id": "a", "slug": "" }, { "id": "b", "slug": "Tiers!" }, { "id": "c", "slug": "index" },
            { "id": "d", "slug": "faq" }, { "id": "e", "slug": "faq" } ] });
        let got = issues(&slugs).join("\n");
        assert!(
            got.contains("pages[1].slug")
                && got.contains("pages[2].slug")
                && got.contains("duplicate slug"),
            "{got}"
        );
    }

    #[test]
    fn unsafe_urls_are_refused_with_their_path() {
        let d = doc(json!([{ "id": "h", "block": "hero/centered",
            "props": { "primary_href": " java\tscript:alert(1)", "secondary_href": "//evil.example" },
            "blocks": [{ "id": "img", "block": "embed/image",
                         "props": { "src": "data:text/html;base64,PHNjcmlwdD4=" } }] },
            { "id": "v", "block": "media/video", "props": { "src": "http://insecure.example/v.mp4", "poster": "data:image/png;base64,iVBORw0KGgo=" } },
            { "id": "f", "block": "footer/simple", "props": { "links": [{ "label": "x", "href": "vbscript:x" }] } }
        ]));
        let got = issues(&d).join("\n");
        assert!(
            got.contains("props.primary_href: the javascript: scheme"),
            "{got}"
        );
        assert!(
            got.contains("props.secondary_href: the protocol-relative"),
            "{got}"
        );
        assert!(got.contains("blocks[0].props.src"), "{got}");
        assert!(
            got.contains("sections[1].props.src: the http: scheme"),
            "{got}"
        );
        assert!(
            !got.contains("props.poster"),
            "a data:image poster is fine: {got}"
        );
        assert!(got.contains("links[0].href"), "{got}");
        assert!(validate(&d).is_err());
    }

    #[test]
    fn scheme_and_data_image_helpers() {
        assert_eq!(scheme("HTTPS://x").as_deref(), Some("https"));
        assert_eq!(scheme("/tiers").as_deref(), None);
        assert_eq!(scheme("#faq").as_deref(), None);
        assert_eq!(scheme("tiers.html").as_deref(), None);
        assert_eq!(scheme("//cdn").as_deref(), Some("//"));
        assert!(is_data_image("data:image/webp;base64,UklGRg=="));
        assert!(!is_data_image("data:image/svg+xml;base64,PHN2Zz4="));
    }
}
