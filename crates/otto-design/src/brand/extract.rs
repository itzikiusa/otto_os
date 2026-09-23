//! The `otto-brand` indexer (`extract::extract` hands `otto-brand` here):
//!
//! - **links** — a logo's `asset: "otto://design/<id>"` is rendered by the kit
//!   (`embeds`, `src_node` = `logos.<kind>`); any other `otto://` string is
//!   related by its key like every JSON format (`extract::rel_for_key`);
//! - **text** — what people search a kit for: its name, token names
//!   (`primary`, `color.primary`), descriptions, font stacks, logo names and
//!   the voice / imagery copy.
//!
//! Pure; unit-tested.

use serde_json::Value;

use crate::extract::{rel_for_key, Extraction, RawRef, Target, MAX_REFS, MAX_TEXT};
use crate::uri::{scan, PREFIX};

const MAX_DEPTH: usize = 32;

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

/// Index an `otto-brand` document.
pub fn extract(text: &str) -> Extraction {
    let mut out = Extraction::default();
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        // Not JSON (a legacy row) — still index its references.
        scan_into(&mut out, text, "references", None);
        return out;
    };
    let Some(root) = v.as_object() else {
        return out;
    };
    if let Some(n) = root.get("name").and_then(Value::as_str) {
        push_text(&mut out, n);
    }
    // Token names + descriptions + font stacks.
    for g in super::doc::GROUPS {
        let Some(group) = root.get(*g).and_then(Value::as_object) else {
            continue;
        };
        for (name, tok) in group.iter().filter(|(k, _)| !k.starts_with('$')) {
            push_text(&mut out, name);
            push_text(&mut out, &format!("{g}.{name}"));
            if let Some(d) = tok.get("$description").and_then(Value::as_str) {
                push_text(&mut out, d);
            }
            if *g == "font" {
                if let Some(s) = tok.get("$value").and_then(Value::as_str) {
                    push_text(&mut out, s);
                }
            }
        }
    }
    // Logos: the asset is rendered by the kit → `embeds`.
    if let Some(logos) = root.get("logos").and_then(Value::as_array) {
        for l in logos.iter().filter_map(Value::as_object) {
            if let Some(n) = l.get("name").and_then(Value::as_str) {
                push_text(&mut out, n);
            }
            let kind = l.get("kind").and_then(Value::as_str).unwrap_or("full");
            if let Some(a) = l.get("asset").and_then(Value::as_str) {
                if a.contains(PREFIX) {
                    let node = format!("logos.{kind}");
                    scan_into(&mut out, a, "embeds", Some(&node));
                }
            }
        }
    }
    // Voice / imagery copy.
    for g in ["voice", "imagery"] {
        if let Some(o) = root.get(g).and_then(Value::as_object) {
            if let Some(s) = o.get("summary").and_then(Value::as_str) {
                push_text(&mut out, s);
            }
            for k in ["do", "dont"] {
                for s in o
                    .get(k)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                {
                    push_text(&mut out, s);
                }
            }
        }
    }
    // Any other `otto://` reference, related by its key (logos done above).
    for (k, child) in root.iter().filter(|(k, _)| k.as_str() != "logos") {
        walk(child, k, 0, &mut out);
    }
    out
}

fn walk(v: &Value, key: &str, depth: usize, out: &mut Extraction) {
    if depth > MAX_DEPTH {
        return;
    }
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                walk(child, k, depth + 1, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                walk(item, key, depth + 1, out);
            }
        }
        Value::String(s) if s.contains(PREFIX) => {
            scan_into(out, s, rel_for_key("otto-brand", key), None);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn uris(ex: &Extraction) -> Vec<(String, &'static str, Option<String>)> {
        ex.refs
            .iter()
            .filter_map(|r| match &r.target {
                Target::Uri(u) => Some((u.artifact_id.clone(), r.rel, r.src_node.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn logos_embed_and_text_has_tokens_and_voice() {
        let doc = json!({
            "$schema": "otto-brand/1",
            "name": "Acme Brand Kit",
            "color": { "primary": { "$value": "#5B3DF5", "$description": "Buttons and links" } },
            "font": { "display": { "$value": "Inter, system-ui" } },
            "logos": [
                { "name": "Acme wordmark", "kind": "full", "asset": "otto://design/LOGO@approved" },
                { "name": "Blob mark", "kind": "mark", "asset": format!("blob:{}", "a".repeat(64)) }
            ],
            "imagery": { "summary": "Product renders", "moodboard": "otto://design/MOOD" },
            "voice": { "summary": "Warm, confident, never salesy.", "do": ["Join free"], "dont": ["Submit"] }
        });
        let ex = extract(&doc.to_string());
        let refs = uris(&ex);
        assert!(
            refs.contains(&("LOGO".into(), "embeds", Some("logos.full".into()))),
            "{refs:?}"
        );
        assert!(
            refs.contains(&("MOOD".into(), "references", None)),
            "{refs:?}"
        );
        assert_eq!(refs.len(), 2, "blob assets are not links: {refs:?}");
        for needle in [
            "Acme Brand Kit",
            "primary",
            "color.primary",
            "Buttons and links",
            "Inter, system-ui",
            "Acme wordmark",
            "Warm, confident",
            "Join free",
            "Submit",
            "Product renders",
        ] {
            assert!(
                ex.text.contains(needle),
                "{needle:?} missing from {:?}",
                ex.text
            );
        }
    }

    #[test]
    fn malformed_and_non_json_are_reported_not_fatal() {
        let doc =
            json!({ "logos": [{ "name": "x", "kind": "mark", "asset": "otto://design/@bad" }] });
        let ex = extract(&doc.to_string());
        assert!(ex
            .refs
            .iter()
            .any(|r| matches!(r.target, Target::Malformed(_))));
        let ex = extract("not json otto://design/X1");
        assert_eq!(uris(&ex).len(), 1);
        assert_eq!(extract("[1,2]"), Extraction::default());
    }
}
