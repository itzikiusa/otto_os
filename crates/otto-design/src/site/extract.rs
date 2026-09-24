//! The `otto-site` indexer (`extract::extract` hands `otto-site` here):
//!
//! - **links** — every `otto://design/…` string, related by its key like
//!   every JSON format ([`rel_for_key`]: `src` / `poster` embed, `brand`
//!   uses_tokens, `derived_from` …) plus `image` (a picture the page renders
//!   → `embeds`); `src_node` = the nearest enclosing section / block `id`;
//! - **text** — the copy people search sites for: titles, headlines, body
//!   copy, FAQ questions and answers, tier names, perks, captions, alt text.
//!
//! Pure; unit-tested.

use serde_json::Value;

use crate::extract::{rel_for_key, Extraction, RawRef, Target, MAX_REFS, MAX_TEXT};
use crate::uri::{scan, PREFIX};

const MAX_DEPTH: usize = 64;

/// Keys whose string values are copy worth indexing.
const TEXT_KEYS: &[&str] = &[
    "title",
    "name",
    "description",
    "eyebrow",
    "headline",
    "subhead",
    "heading",
    "subheading",
    "body",
    "trust",
    "label",
    "quote",
    "role",
    "question",
    "answer",
    "blurb",
    "features",
    "price",
    "value",
    "caption",
    "alt",
    "tagline",
    "legal",
    "logo",
    "note",
    "badge",
    "primary_label",
    "secondary_label",
    "cta_label",
    "button_label",
];

fn push_ref(out: &mut Extraction, r: RawRef) {
    if out.refs.len() < MAX_REFS && !out.refs.contains(&r) {
        out.refs.push(r);
    }
}

fn push_text(out: &mut Extraction, s: &str) {
    let s = s.trim();
    if s.is_empty() || out.text.len() >= MAX_TEXT {
        return;
    }
    if !out.text.is_empty() {
        out.text.push(' ');
    }
    let room = MAX_TEXT.saturating_sub(out.text.len());
    let mut end = s.len().min(room);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    out.text.push_str(&s[..end]);
}

fn scan_into(out: &mut Extraction, text: &str, rel: &'static str, node: Option<&str>) {
    for hit in scan(text) {
        let target = match hit.parsed {
            Ok(u) => Target::Uri(u),
            Err(raw) => Target::Malformed(raw),
        };
        push_ref(
            out,
            RawRef {
                target,
                rel,
                src_node: node.map(str::to_string),
            },
        );
    }
}

/// The relation a site reference implies from its key.
fn rel_for(key: &str) -> &'static str {
    match key {
        "image" => "embeds",
        other => rel_for_key("otto-site", other),
    }
}

fn walk(v: &Value, key: &str, node: Option<&str>, depth: usize, out: &mut Extraction) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Object(map) => {
            let node = map.get("id").and_then(Value::as_str).or(node);
            for (k, child) in map {
                walk(child, k, node, depth + 1, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, key, node, depth + 1, out);
            }
        }
        Value::String(s) => {
            if s.contains(PREFIX) {
                scan_into(out, s, rel_for(key), node);
            } else if TEXT_KEYS.contains(&key) {
                push_text(out, s);
            }
        }
        _ => {}
    }
}

/// Index an `otto-site` document.
pub fn extract(text: &str) -> Extraction {
    let mut out = Extraction::default();
    match serde_json::from_str::<Value>(text) {
        Ok(v) => walk(&v, "", None, 0, &mut out),
        // Not JSON (a legacy row) — still index its references.
        Err(_) => scan_into(&mut out, text, "references", None),
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::uri::VersionSel;
    use serde_json::json;

    fn uris(ex: &Extraction) -> Vec<(String, &'static str, Option<String>, VersionSel)> {
        ex.refs
            .iter()
            .filter_map(|r| match &r.target {
                Target::Uri(u) => Some((
                    u.artifact_id.clone(),
                    r.rel,
                    r.src_node.clone(),
                    u.version.clone(),
                )),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn embeds_brand_images_and_provenance_are_linked_to_their_section() {
        let doc = json!({
            "type": "otto-site", "version": 1, "title": "Rewards+",
            "brand": "otto://design/KIT",
            "pages": [{ "id": "home", "title": "Home", "sections": [
                { "id": "hero", "block": "hero/split",
                  "props": { "headline": "Every purchase moves you up.", "primary_href": "page:tiers" },
                  "blocks": [{ "id": "card", "block": "embed/3d", "props": { "src": "otto://design/CARD@approved", "alt": "A rotating card" } }] },
                { "id": "faq2", "block": "faq/accordion", "derived_from": "otto://design/SPRING@v12#faq",
                  "blocks": [{ "id": "q1", "block": "item/faq", "props": { "question": "Do points expire?", "answer": "Never." } }] },
                { "id": "feat", "block": "features/alternating",
                  "blocks": [{ "id": "f1", "block": "item/feature", "props": { "title": "Fast", "image": "otto://design/SHOT@v3" } }] }
            ]}]
        });
        let ex = extract(&doc.to_string());
        let got = uris(&ex);
        assert!(
            got.contains(&("KIT".into(), "uses_tokens", None, VersionSel::Default)),
            "{got:?}"
        );
        assert!(
            got.contains(&(
                "CARD".into(),
                "embeds",
                Some("card".into()),
                VersionSel::Approved
            )),
            "{got:?}"
        );
        assert!(
            got.contains(&(
                "SPRING".into(),
                "derived_from",
                Some("faq2".into()),
                VersionSel::Seq(12)
            )),
            "{got:?}"
        );
        assert!(
            got.contains(&(
                "SHOT".into(),
                "embeds",
                Some("f1".into()),
                VersionSel::Seq(3)
            )),
            "{got:?}"
        );
        for want in [
            "Rewards+",
            "Every purchase moves you up.",
            "Do points expire?",
            "Never.",
            "A rotating card",
            "Fast",
        ] {
            assert!(ex.text.contains(want), "{want} missing in {}", ex.text);
        }
        assert!(!ex.text.contains("page:tiers"), "links are not copy");
        assert!(!ex.text.contains("hero/split"), "block names are not copy");
    }

    #[test]
    fn malformed_references_are_reported_and_garbage_never_panics() {
        let ex = extract(
            r#"{"type":"otto-site","pages":[{"id":"p","sections":[{"id":"s2","block":"cta/band","props":{"link":"otto://design/@nope"}}]}]}"#,
        );
        assert!(ex.refs.iter().any(
            |r| matches!(r.target, Target::Malformed(_)) && r.src_node.as_deref() == Some("s2")
        ));
        let ex = extract("not json otto://design/X1");
        assert_eq!(uris(&ex).len(), 1);
    }
}
