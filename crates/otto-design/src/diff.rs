//! Bounded, deterministic change summaries between two versions — the payload
//! of an `edit_after_draft` signal ("what did the human change after the
//! agent's draft"). Never the raw content: JSON formats report up to
//! [`MAX_PATHS`] changed paths (depth ≤ [`MAX_DEPTH`]), text formats report line
//! counts, binaries report sizes. Phase 1's per-format structural diff (the
//! Compare "Changes" view) will replace the JSON heuristic; the payload shape
//! stays additive. Pure; unit-tested.

use serde_json::{json, Map, Value};

use crate::format::Encoding;

pub const MAX_PATHS: usize = 20;
pub const MAX_DEPTH: usize = 4;
const MAX_PATH_CHARS: usize = 120;

/// Summarize `old` → `new` for a format of `encoding`.
pub fn summarize(encoding: Encoding, old: &[u8], new: &[u8]) -> Value {
    let size = json!({ "old_bytes": old.len(), "new_bytes": new.len() });
    match encoding {
        Encoding::Binary => json!({ "kind": "binary", "size": size }),
        Encoding::Json => {
            let (Ok(a), Ok(b)) = (
                serde_json::from_slice::<Value>(old),
                serde_json::from_slice::<Value>(new),
            ) else {
                return json!({ "kind": "json", "size": size, "unparsed": true });
            };
            let mut paths = Vec::new();
            let mut total = 0usize;
            diff_values(&a, &b, "", 0, &mut paths, &mut total);
            json!({
                "kind": "json",
                "size": size,
                "changed_paths": paths,
                "changed_total": total,
            })
        }
        Encoding::Text => {
            let a = String::from_utf8_lossy(old);
            let b = String::from_utf8_lossy(new);
            let (added, removed) = line_delta(&a, &b);
            json!({
                "kind": "text",
                "size": size,
                "old_lines": a.lines().count(),
                "new_lines": b.lines().count(),
                "lines_added": added,
                "lines_removed": removed,
            })
        }
    }
}

fn push_path(paths: &mut Vec<String>, total: &mut usize, path: &str) {
    *total += 1;
    if paths.len() < MAX_PATHS {
        let p = if path.is_empty() { "$" } else { path };
        paths.push(p.chars().take(MAX_PATH_CHARS).collect());
    }
}

/// Recursive JSON diff: arrays of objects that carry an `id` are matched by id
/// (so moving an object isn't "everything changed"), other arrays by index.
fn diff_values(
    a: &Value,
    b: &Value,
    path: &str,
    depth: usize,
    paths: &mut Vec<String>,
    total: &mut usize,
) {
    if a == b {
        return;
    }
    if depth >= MAX_DEPTH {
        push_path(paths, total, path);
        return;
    }
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, va) in x {
                let p = join_key(path, k);
                match y.get(k) {
                    Some(vb) => diff_values(va, vb, &p, depth + 1, paths, total),
                    None => push_path(paths, total, &format!("-{p}")),
                }
            }
            for k in y.keys() {
                if !x.contains_key(k) {
                    push_path(paths, total, &format!("+{}", join_key(path, k)));
                }
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            let ids = |v: &Vec<Value>| -> Option<Map<String, Value>> {
                let mut m = Map::new();
                for item in v {
                    let id = item.get("id")?.as_str()?;
                    m.insert(id.to_string(), item.clone());
                }
                Some(m)
            };
            match (ids(x), ids(y)) {
                (Some(mx), Some(my)) if !mx.is_empty() || !my.is_empty() => {
                    for (id, va) in &mx {
                        let p = format!("{path}[{id}]");
                        match my.get(id) {
                            Some(vb) => diff_values(va, vb, &p, depth + 1, paths, total),
                            None => push_path(paths, total, &format!("-{p}")),
                        }
                    }
                    for id in my.keys() {
                        if !mx.contains_key(id) {
                            push_path(paths, total, &format!("+{path}[{id}]"));
                        }
                    }
                }
                _ => {
                    let n = x.len().max(y.len());
                    for i in 0..n {
                        let p = format!("{path}[{i}]");
                        match (x.get(i), y.get(i)) {
                            (Some(va), Some(vb)) => {
                                diff_values(va, vb, &p, depth + 1, paths, total)
                            }
                            (Some(_), None) => push_path(paths, total, &format!("-{p}")),
                            (None, Some(_)) => push_path(paths, total, &format!("+{p}")),
                            (None, None) => {}
                        }
                    }
                }
            }
        }
        _ => push_path(paths, total, path),
    }
}

fn join_key(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// Multiset line delta: lines present in `b` but not `a` (added) and vice
/// versa (removed). Order-insensitive, O(n) — enough for a learning signal.
fn line_delta(a: &str, b: &str) -> (usize, usize) {
    use std::collections::HashMap;
    let mut counts: HashMap<&str, i64> = HashMap::new();
    for l in a.lines() {
        *counts.entry(l.trim_end()).or_default() += 1;
    }
    for l in b.lines() {
        *counts.entry(l.trim_end()).or_default() -= 1;
    }
    let mut added = 0usize;
    let mut removed = 0usize;
    for c in counts.values() {
        if *c > 0 {
            removed += *c as usize;
        } else if *c < 0 {
            added += (-*c) as usize;
        }
    }
    (added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_diff_matches_objects_by_id_and_is_bounded() {
        let old = br##"{"objects":[{"id":"a","x":1},{"id":"b","x":2}],"bg":"#fff"}"##;
        let new = br##"{"objects":[{"id":"b","x":2},{"id":"a","x":5},{"id":"c"}],"bg":"#fff"}"##;
        let s = summarize(Encoding::Json, old, new);
        let paths: Vec<&str> = s["changed_paths"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p.as_str().unwrap())
            .collect();
        assert!(paths.contains(&"objects[a].x"), "{paths:?}");
        assert!(paths.contains(&"+objects[c]"), "{paths:?}");
        assert!(!paths.iter().any(|p| p.contains("[b]")), "b only moved");

        // Many changes: the path list is capped, the total is not.
        let old = serde_json::to_vec(&json!({ "k": (0..50).collect::<Vec<_>>() })).unwrap();
        let new = serde_json::to_vec(&json!({ "k": (100..150).collect::<Vec<_>>() })).unwrap();
        let s = summarize(Encoding::Json, &old, &new);
        assert_eq!(s["changed_paths"].as_array().unwrap().len(), MAX_PATHS);
        assert_eq!(s["changed_total"], 50);
    }

    #[test]
    fn text_and_binary_summaries_carry_no_content() {
        let s = summarize(Encoding::Text, b"a\nb\nc\n", b"a\nc\nd\ne\n");
        assert_eq!(s["lines_added"], 2);
        assert_eq!(s["lines_removed"], 1);
        assert!(!s.to_string().contains("\"d\""));
        let s = summarize(Encoding::Binary, b"xx", b"yyy");
        assert_eq!(s["size"]["new_bytes"], 3);
        let s = summarize(Encoding::Json, b"{", b"{}");
        assert_eq!(s["unparsed"], true);
    }
}
