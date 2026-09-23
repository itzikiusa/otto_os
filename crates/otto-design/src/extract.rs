//! Per-format indexer: pulls every outgoing reference and the searchable text
//! out of a document on each save. The Vault's link index is the precedent —
//! everything here is DERIVED from the bytes and rebuilt on every commit.
//!
//! | format                         | references                                         | text                            |
//! |--------------------------------|----------------------------------------------------|---------------------------------|
//! | `html` / `svg`                 | `otto://` in `src=` (embeds) / other attrs (refs)  | visible text (tags/scripts cut) |
//! | `mermaid` / `d2`               | `otto://` anywhere (describes); node from `click X` / `X.link` | the source            |
//! | `scene3d`                      | `gltf` objects' `attachment_id` (embeds) + `otto://` strings | names, text, notes    |
//! | `excalidraw` / `otto-canvas`   | `otto://` strings (describes) incl. inner source   | element text                    |
//! | `otto-site`/`-layout`/`-brand`/`-exhibit`, `gltf` | `otto://` strings, rel by key (see [`rel_for_key`]) | copy-ish string values (+ token names for brand) |
//! | binaries                       | —                                                  | —                               |
//!
//! Malformed references are reported (never dropped silently, never a crash).
//! Pure; unit-tested.

use std::collections::HashSet;

use serde_json::Value;

use crate::uri::{scan, DesignUri};

/// Cap on the extracted search text per document (64 KiB).
pub const MAX_TEXT: usize = 64 * 1024;
/// Cap on references per document (the link table stays bounded).
pub const MAX_REFS: usize = 500;
const MAX_JSON_DEPTH: usize = 64;

/// What a reference points at before resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A well-formed `otto://design/…` reference.
    Uri(DesignUri),
    /// A bare id from a legacy field (`scene3d` `gltf.attachment_id`): resolves
    /// to the design artifact with that id, else to the one imported from the
    /// product attachment with that id.
    LegacyId(String),
    /// A malformed `otto://design/…` string (reported as broken).
    Malformed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRef {
    pub target: Target,
    pub rel: &'static str,
    /// The node INSIDE this document the reference hangs off (an element /
    /// object / block id, a Mermaid node …).
    pub src_node: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Extraction {
    pub refs: Vec<RawRef>,
    pub text: String,
}

impl Extraction {
    fn push_ref(&mut self, r: RawRef) {
        if self.refs.len() < MAX_REFS && !self.refs.contains(&r) {
            self.refs.push(r);
        }
    }
    fn push_text(&mut self, s: &str) {
        let s = s.trim();
        if s.is_empty() || self.text.len() >= MAX_TEXT {
            return;
        }
        if !self.text.is_empty() {
            self.text.push(' ');
        }
        let room = MAX_TEXT - self.text.len();
        if s.len() <= room {
            self.text.push_str(s);
        } else {
            // Cut on a char boundary.
            let mut end = room;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            self.text.push_str(&s[..end]);
        }
    }
}

/// Is this a whiteboard format (its references *describe* frames/pages)?
fn is_whiteboard(format: &str) -> bool {
    matches!(format, "mermaid" | "d2" | "excalidraw" | "otto-canvas")
}

/// Extract references + search text from `bytes` of `format`.
pub fn extract(format: &str, bytes: &[u8]) -> Extraction {
    let mut out = Extraction::default();
    let Ok(text) = std::str::from_utf8(bytes) else {
        return out; // binary formats carry neither
    };
    match format {
        "html" | "svg" => extract_html(text, &mut out),
        "mermaid" | "d2" => extract_diagram(text, &mut out),
        "otto-brand" => out = crate::brand::extract(text),
        "png" | "jpeg" | "gif" | "webp" | "pdf" | "glb" => {}
        _ => match serde_json::from_str::<Value>(text) {
            Ok(v) => extract_json(format, &v, &mut out),
            // Not JSON after all (a legacy row) — still index its references.
            Err(_) => scan_into(text, "references", None, &mut out),
        },
    }
    out
}

/// Push every `otto://` hit in `text` with a fixed rel/node.
fn scan_into(text: &str, rel: &'static str, node: Option<&str>, out: &mut Extraction) {
    for hit in scan(text) {
        let target = match hit.parsed {
            Ok(u) => Target::Uri(u),
            Err(raw) => Target::Malformed(raw),
        };
        out.push_ref(RawRef {
            target,
            rel,
            src_node: node.map(str::to_string),
        });
    }
}

// ---------------------------------------------------------------------------
// HTML / SVG
// ---------------------------------------------------------------------------

fn extract_html(text: &str, out: &mut Extraction) {
    for hit in scan(text) {
        let before = &text[..hit.offset];
        let rel = if attr_name_before(before).is_some_and(|a| {
            matches!(
                a.as_str(),
                "src" | "data" | "poster" | "data-src" | "href:embed"
            )
        }) {
            "embeds"
        } else {
            "references"
        };
        let node = enclosing_tag_id(before);
        let target = match hit.parsed {
            Ok(u) => Target::Uri(u),
            Err(raw) => Target::Malformed(raw),
        };
        out.push_ref(RawRef {
            target,
            rel,
            src_node: node,
        });
    }
    out.push_text(&visible_text(text));
}

/// The attribute name right before a value that starts at the end of `before`
/// (`… src="` → `src`). `None` when the reference is not an attribute value.
fn attr_name_before(before: &str) -> Option<String> {
    let t = before.trim_end_matches(['"', '\'']).trim_end();
    let t = t.strip_suffix('=')?.trim_end();
    let name: String = t
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ':')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    (!name.is_empty()).then(|| name.to_ascii_lowercase())
}

/// The `id="…"` of the tag a reference sits in (text since the last `<`).
fn enclosing_tag_id(before: &str) -> Option<String> {
    let lt = before.rfind('<')?;
    let tag = &before[lt..];
    if tag.contains('>') {
        return None; // the reference is in text content, not in a tag
    }
    for q in ["id=\"", "id='"] {
        if let Some(i) = tag.find(q) {
            let rest = &tag[i + q.len()..];
            let end = rest.find(['"', '\''])?;
            let id = &rest[..end];
            // Guard `data-id=` etc.: the char before `id=` must not be part of a name.
            let prev = tag[..i].chars().last();
            if !id.is_empty() && !prev.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-') {
                return Some(id.to_string());
            }
        }
    }
    None
}

/// Visible text of an HTML document: tags removed, `<script>`/`<style>`
/// bodies skipped, the common entities decoded, whitespace collapsed.
pub fn visible_text(html: &str) -> String {
    let lower = html.to_ascii_lowercase();
    let mut out = String::with_capacity(html.len().min(MAX_TEXT));
    let mut i = 0usize;
    let bytes = html.as_bytes();
    while i < bytes.len() && out.len() < MAX_TEXT {
        if bytes[i] == b'<' {
            // Skip a whole <script>/<style> element.
            let skip_to = ["script", "style"].iter().find_map(|t| {
                if lower[i + 1..].starts_with(*t) {
                    lower[i..].find(&format!("</{t}")).map(|e| i + e)
                } else {
                    None
                }
            });
            let from = skip_to.unwrap_or(i);
            match lower[from..].find('>') {
                Some(e) => i = from + e + 1,
                None => break,
            }
            out.push(' ');
            continue;
        }
        let next = html[i..].find('<').map(|e| i + e).unwrap_or(html.len());
        out.push_str(&html[i..next]);
        i = next;
    }
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// Mermaid / D2
// ---------------------------------------------------------------------------

fn extract_diagram(text: &str, out: &mut Extraction) {
    for line in text.lines() {
        if !line.contains(crate::uri::PREFIX) {
            continue;
        }
        let node = diagram_node(line);
        scan_into(line, "describes", node.as_deref(), out);
    }
    out.push_text(text);
}

/// The diagram node a reference line belongs to: Mermaid `click Node "…"` /
/// `click Node href "…"`, or D2 `node.link: …`.
fn diagram_node(line: &str) -> Option<String> {
    let t = line.trim();
    if let Some(rest) = t.strip_prefix("click ") {
        let id: String = rest
            .trim_start()
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect();
        return (!id.is_empty()).then_some(id);
    }
    if let Some(i) = t.find(".link") {
        let id = t[..i].trim();
        if !id.is_empty() && !id.contains(char::is_whitespace) {
            return Some(id.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// JSON formats
// ---------------------------------------------------------------------------

/// Keys whose string values are human copy worth indexing.
const TEXT_KEYS: &[&str] = &[
    "text",
    "originalText",
    "title",
    "label",
    "name",
    "heading",
    "subheading",
    "headline",
    "copy",
    "body",
    "caption",
    "alt",
    "description",
    "notes",
    "content",
    "cta",
];

/// The relation a reference implies from the JSON key holding it.
pub fn rel_for_key(format: &str, key: &str) -> &'static str {
    match key {
        "src" | "embed" | "model" | "media" | "poster" => "embeds",
        "component" | "component_ref" | "instance_of" => "uses_component",
        "brand" | "brand_kit" | "tokens" | "theme" => "uses_tokens",
        "derived_from" | "fork_of" => "derived_from",
        "variant_of" => "variant_of",
        "resized_from" => "resized_from",
        _ if is_whiteboard(format) => "describes",
        _ => "references",
    }
}

fn extract_json(format: &str, v: &Value, out: &mut Extraction) {
    walk(format, v, "", None, 0, out);
    if format == "otto-canvas" {
        // The canvas wrapper's `source` is the inner document (Mermaid/D2 text
        // or Excalidraw JSON) — index it with the inner format's rules.
        let inner = v.get("format").and_then(Value::as_str).unwrap_or("mermaid");
        if let Some(src) = v.get("source").and_then(Value::as_str) {
            let inner_ex = match inner {
                "excalidraw" => match serde_json::from_str::<Value>(src) {
                    Ok(iv) => {
                        let mut e = Extraction::default();
                        walk("excalidraw", &iv, "", None, 0, &mut e);
                        e
                    }
                    Err(_) => Extraction::default(),
                },
                _ => {
                    let mut e = Extraction::default();
                    extract_diagram(src, &mut e);
                    e
                }
            };
            for r in inner_ex.refs {
                out.push_ref(r);
            }
            out.push_text(&inner_ex.text);
        }
    }
}

/// Depth-first walk carrying the nearest enclosing object's `id` as the
/// reference's `src_node`.
fn walk(
    format: &str,
    v: &Value,
    key: &str,
    node: Option<&str>,
    depth: usize,
    out: &mut Extraction,
) {
    if depth > MAX_JSON_DEPTH {
        return;
    }
    match v {
        Value::Object(map) => {
            let own_id = map.get("id").and_then(Value::as_str);
            let node = own_id.or(node);
            // scene3d: a `gltf` object embeds the model named by `attachment_id`.
            if format == "scene3d" && map.get("type").and_then(Value::as_str) == Some("gltf") {
                if let Some(aid) = map.get("attachment_id").and_then(Value::as_str) {
                    out.push_ref(RawRef {
                        target: Target::LegacyId(aid.to_string()),
                        rel: "embeds",
                        src_node: node.map(str::to_string),
                    });
                }
            }
            for (k, child) in map {
                // The canvas wrapper's `source` is indexed with the INNER
                // format's rules by `extract_json` — don't double-count it.
                if format == "otto-canvas" && depth == 0 && k == "source" {
                    continue;
                }
                // Brand kits: token NAMES are what people search for.
                if format == "otto-brand" && child.is_object() && k != "meta" {
                    out.push_text(k);
                }
                walk(format, child, k, node, depth + 1, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(format, item, key, node, depth + 1, out);
            }
        }
        Value::String(s) => {
            if s.contains(crate::uri::PREFIX) {
                scan_into(s, rel_for_key(format, key), node, out);
            } else if TEXT_KEYS.contains(&key) {
                out.push_text(s);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Node ids (for `#node` validation of a link target)
// ---------------------------------------------------------------------------

/// Every node id a document defines, when the format has addressable nodes:
/// JSON formats → every `"id"` string (objects, groups, elements, blocks);
/// HTML/SVG → every `id="…"`. `None` = the format has no node model, so a
/// `#node` reference is accepted as-is.
pub fn node_ids(format: &str, bytes: &[u8]) -> Option<HashSet<String>> {
    const MAX_IDS: usize = 20_000;
    let text = std::str::from_utf8(bytes).ok()?;
    match format {
        "html" | "svg" => {
            let mut ids = HashSet::new();
            let mut from = 0usize;
            while let Some(i) = text[from..].find("id=") {
                let at = from + i;
                from = at + 3;
                let prev = text[..at].chars().last();
                if prev.is_some_and(|c| c.is_ascii_alphanumeric() || c == '-') {
                    continue;
                }
                let rest = &text[at + 3..];
                let Some(q) = rest.chars().next().filter(|c| *c == '"' || *c == '\'') else {
                    continue;
                };
                if let Some(end) = rest[1..].find(q) {
                    ids.insert(rest[1..1 + end].to_string());
                }
                if ids.len() >= MAX_IDS {
                    break;
                }
            }
            Some(ids)
        }
        "mermaid" | "d2" | "png" | "jpeg" | "gif" | "webp" | "pdf" | "glb" => None,
        _ => {
            let v: Value = serde_json::from_str(text).ok()?;
            let mut ids = HashSet::new();
            collect_ids(&v, 0, &mut ids, MAX_IDS);
            Some(ids)
        }
    }
}

fn collect_ids(v: &Value, depth: usize, ids: &mut HashSet<String>, cap: usize) {
    if depth > MAX_JSON_DEPTH || ids.len() >= cap {
        return;
    }
    match v {
        Value::Object(map) => {
            if let Some(id) = map.get("id").and_then(Value::as_str) {
                ids.insert(id.to_string());
            }
            for child in map.values() {
                collect_ids(child, depth + 1, ids, cap);
            }
        }
        Value::Array(items) => {
            for item in items {
                collect_ids(item, depth + 1, ids, cap);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uri::VersionSel;

    fn uri_ids(ex: &Extraction) -> Vec<(String, &'static str, Option<String>)> {
        ex.refs
            .iter()
            .filter_map(|r| match &r.target {
                Target::Uri(u) => Some((u.artifact_id.clone(), r.rel, r.src_node.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn html_src_embeds_href_references_and_text_is_visible_only() {
        let html = r#"<html><head><style>.x{color:red}</style><script>var a="otto://design/SCRIPT";</script></head>
<body><section id="hero"><h1>Rewards &amp; more</h1>
<img id="card" src="otto://design/CARD@approved#view:hero">
<a data-id="nope" href="otto://design/FAQ">FAQ</a></section></body></html>"#;
        let ex = extract("html", html.as_bytes());
        let refs = uri_ids(&ex);
        assert!(
            refs.contains(&("CARD".into(), "embeds", Some("card".into()))),
            "{refs:?}"
        );
        assert!(
            refs.contains(&("FAQ".into(), "references", None)),
            "{refs:?}"
        );
        assert!(ex.text.contains("Rewards & more"), "{}", ex.text);
        assert!(!ex.text.contains("color"), "style bodies are not text");
        assert!(!ex.text.contains("var a"), "script bodies are not text");
    }

    #[test]
    fn mermaid_click_lines_describe_with_their_node() {
        let src = "flowchart TD\n  Join --> Pay\n  click Join \"otto://design/SIGNUP@latest\"\n";
        let ex = extract("mermaid", src.as_bytes());
        assert_eq!(
            uri_ids(&ex),
            vec![("SIGNUP".to_string(), "describes", Some("Join".to_string()))]
        );
        let d2 = "join: Join\njoin.link: otto://design/SIGNUP\n";
        let ex = extract("d2", d2.as_bytes());
        assert_eq!(uri_ids(&ex)[0].2.as_deref(), Some("join"));
    }

    #[test]
    fn scene3d_gltf_embeds_legacy_attachment_and_indexes_names() {
        let doc = serde_json::json!({
            "type": "otto-scene3d", "version": 1,
            "objects": [
                { "id": "floor", "type": "plane", "name": "Floor" },
                { "id": "hero", "type": "gltf", "attachment_id": "01ATT", "notes": "the gift box" }
            ]
        });
        let ex = extract("scene3d", doc.to_string().as_bytes());
        assert_eq!(
            ex.refs,
            vec![RawRef {
                target: Target::LegacyId("01ATT".into()),
                rel: "embeds",
                src_node: Some("hero".into()),
            }]
        );
        assert!(ex.text.contains("Floor") && ex.text.contains("gift box"));
    }

    #[test]
    fn site_blocks_use_key_based_rels_and_report_malformed() {
        let doc = serde_json::json!({
            "type": "otto-site",
            "brand": "otto://design/BRAND",
            "pages": [{ "id": "home", "sections": [
                { "id": "s1", "block": "media/3d-embed", "props": { "src": "otto://design/CARD@v3#view:hero", "headline": "Win more" } },
                { "id": "s2", "block": "custom/html", "props": { "link": "otto://design/@nope" } }
            ]}]
        });
        let ex = extract("otto-site", doc.to_string().as_bytes());
        let refs = uri_ids(&ex);
        assert!(
            refs.contains(&("BRAND".into(), "uses_tokens", None)),
            "{refs:?}"
        );
        assert!(
            refs.contains(&("CARD".into(), "embeds", Some("s1".into()))),
            "{refs:?}"
        );
        let card = ex
            .refs
            .iter()
            .find_map(|r| match &r.target {
                Target::Uri(u) if u.artifact_id == "CARD" => Some(u.clone()),
                _ => None,
            })
            .unwrap();
        assert_eq!(card.version, VersionSel::Seq(3));
        assert!(ex.refs.iter().any(
            |r| matches!(r.target, Target::Malformed(_)) && r.src_node.as_deref() == Some("s2")
        ));
        assert!(ex.text.contains("Win more"));
    }

    #[test]
    fn otto_canvas_indexes_its_inner_source() {
        let doc = serde_json::json!({
            "type": "otto-canvas", "version": 1, "format": "mermaid",
            "source": "flowchart LR\n A[Start] --> B\n click A \"otto://design/FRAME\"\n"
        });
        let ex = extract("otto-canvas", doc.to_string().as_bytes());
        assert_eq!(
            uri_ids(&ex),
            vec![("FRAME".to_string(), "describes", Some("A".to_string()))]
        );
        assert!(ex.text.contains("Start"));
    }

    #[test]
    fn binaries_and_garbage_never_panic() {
        assert_eq!(extract("png", &[0x89, 0x50, 0xff]), Extraction::default());
        let ex = extract("excalidraw", b"not json otto://design/X1");
        assert_eq!(uri_ids(&ex).len(), 1);
        assert!(node_ids("glb", b"glTF").is_none());
    }

    #[test]
    fn node_ids_cover_json_and_html() {
        let ids = node_ids(
            "scene3d",
            br#"{"objects":[{"id":"a"},{"id":"b"}],"groups":[{"id":"g"}]}"#,
        )
        .unwrap();
        assert!(ids.contains("a") && ids.contains("g"));
        let ids = node_ids("html", br#"<div id="hero"></div><p data-id="x" id='faq'>"#).unwrap();
        assert!(ids.contains("hero") && ids.contains("faq") && !ids.contains("x"));
        assert!(node_ids("mermaid", b"graph TD").is_none());
    }
}
