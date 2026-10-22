//! The Excalidraw adapter shared by the assist engines (architecture-4-02,
//! phase 1): split a stored scene into the simplified form an agent can edit
//! plus what it can't express (images, freehand, elements bound to those, and
//! every top-level key but `elements`), and merge the agent's result back so
//! nothing it couldn't see is lost. Moved from `otto-canvas`'s assist (the
//! Canvas module uses it today); the Design Hall engine adopts it with the
//! phase-2 `AssistTarget` merge. Pure; unit-tested by its callers. Lives in
//! `otto-design::format` (no agent/session deps) so any engine can use it
//! cheaply; `otto_design_assist::excalidraw` re-exports it.

use serde_json::Value;

/// Element types the agent's simplified form can express.
const AGENT_TYPES: &[&str] = &["rectangle", "ellipse", "diamond", "arrow", "text"];
/// Shape types (carry an inline `label` in the simplified form).
const SHAPE_TYPES: &[&str] = &["rectangle", "ellipse", "diamond"];

/// What an Excalidraw turn sets aside: the full elements the agent can't
/// express (or that are bound to such elements), plus every top-level key of
/// the scene except `elements` (`files`, `appState`, …).
#[derive(Debug, Clone, Default)]
pub struct ExcalidrawKeep {
    pub elements: Vec<Value>,
    pub extras: serde_json::Map<String, Value>,
}

impl ExcalidrawKeep {
    pub fn is_empty(&self) -> bool {
        self.elements.is_empty() && self.extras.is_empty()
    }
}

pub fn el_id(e: &Value) -> Option<&str> {
    e.get("id").and_then(|v| v.as_str())
}

fn el_type(e: &Value) -> &str {
    e.get("type").and_then(|v| v.as_str()).unwrap_or("")
}

/// An arrow's bound endpoint ids (simplified `start.id` or full
/// `startBinding.elementId`).
fn arrow_ends(e: &Value) -> (Option<&str>, Option<&str>) {
    let end = |simple: &str, full: &str| {
        e.get(simple)
            .and_then(|v| v.get("id"))
            .or_else(|| e.get(full).and_then(|v| v.get("elementId")))
            .and_then(|v| v.as_str())
    };
    (end("start", "startBinding"), end("end", "endBinding"))
}

/// Copy the listed keys (when present) from `src` into a fresh object.
fn pick(src: &Value, keys: &[&str]) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    for k in keys {
        if let Some(v) = src.get(*k) {
            if !v.is_null() {
                m.insert((*k).to_string(), v.clone());
            }
        }
    }
    m
}

/// Split a stored Excalidraw scene into (the agent's simplified view, what to
/// set aside). Unparseable / non-object sources pass through untouched with
/// nothing kept — the turn then behaves exactly as before.
pub fn split_excalidraw(source: &str) -> (String, ExcalidrawKeep) {
    let Ok(Value::Object(mut root)) = serde_json::from_str::<Value>(source) else {
        return (source.to_string(), ExcalidrawKeep::default());
    };
    let Some(Value::Array(all)) = root.remove("elements") else {
        return (source.to_string(), ExcalidrawKeep::default());
    };
    let live: Vec<Value> = all
        .into_iter()
        .filter(|e| {
            !e.get("isDeleted")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        })
        .collect();

    // 1. Decide which elements are set aside. Unknown types, free (unbound)
    //    arrows (their geometry can't be id-routed), and — to a fixed point —
    //    arrows bound to a kept element and texts contained in one.
    let mut kept_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut kept_idx: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for (i, e) in live.iter().enumerate() {
        let t = el_type(e);
        let unbound_arrow = t == "arrow" && arrow_ends(e) == (None, None);
        if !AGENT_TYPES.contains(&t) || unbound_arrow {
            kept_idx.insert(i);
            if let Some(id) = el_id(e) {
                kept_ids.insert(id.to_string());
            }
        }
    }
    loop {
        let mut grew = false;
        for (i, e) in live.iter().enumerate() {
            if kept_idx.contains(&i) {
                continue;
            }
            let bound_to_kept = match el_type(e) {
                "arrow" => {
                    let (a, b) = arrow_ends(e);
                    [a, b].into_iter().flatten().any(|id| kept_ids.contains(id))
                }
                "text" => e
                    .get("containerId")
                    .and_then(|v| v.as_str())
                    .is_some_and(|id| kept_ids.contains(id)),
                _ => false,
            };
            if bound_to_kept {
                kept_idx.insert(i);
                if let Some(id) = el_id(e) {
                    kept_ids.insert(id.to_string());
                }
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }

    // 2. Bound labels of the elements the agent DOES see fold into their
    //    container's simplified `label`.
    let mut labels: std::collections::HashMap<&str, &Value> = std::collections::HashMap::new();
    for (i, e) in live.iter().enumerate() {
        if kept_idx.contains(&i) || el_type(e) != "text" {
            continue;
        }
        if let Some(cid) = e.get("containerId").and_then(|v| v.as_str()) {
            labels.insert(cid, e);
        }
    }
    let label_of = |e: &Value| -> Option<Value> {
        if let Some(l) = e.get("label").filter(|l| l.is_object()) {
            return Some(l.clone());
        }
        let t = labels.get(el_id(e)?)?;
        let text = t
            .get("originalText")
            .or_else(|| t.get("text"))
            .and_then(|v| v.as_str())?;
        let mut l = serde_json::Map::new();
        l.insert("text".into(), Value::from(text));
        for k in ["fontSize", "fontFamily"] {
            if let Some(v) = t.get(k) {
                l.insert(k.into(), v.clone());
            }
        }
        Some(Value::Object(l))
    };

    // 3. The simplified view.
    let mut view = Vec::new();
    for (i, e) in live.iter().enumerate() {
        if kept_idx.contains(&i) {
            continue;
        }
        let t = el_type(e);
        let mut out = match t {
            _ if SHAPE_TYPES.contains(&t) => pick(
                e,
                &[
                    "type",
                    "id",
                    "x",
                    "y",
                    "width",
                    "height",
                    "backgroundColor",
                    "strokeColor",
                    "fillStyle",
                    "roundness",
                ],
            ),
            "arrow" => {
                let mut m = pick(e, &["type", "id", "strokeColor"]);
                let (a, b) = arrow_ends(e);
                if let Some(a) = a {
                    m.insert("start".into(), serde_json::json!({ "id": a }));
                }
                if let Some(b) = b {
                    m.insert("end".into(), serde_json::json!({ "id": b }));
                }
                m
            }
            _ => {
                // text: a container's label was folded above.
                if e.get("containerId")
                    .and_then(|v| v.as_str())
                    .is_some_and(|c| !c.is_empty())
                {
                    continue;
                }
                let mut m = pick(
                    e,
                    &[
                        "type",
                        "id",
                        "x",
                        "y",
                        "fontSize",
                        "fontFamily",
                        "strokeColor",
                    ],
                );
                let text = e
                    .get("originalText")
                    .or_else(|| e.get("text"))
                    .cloned()
                    .unwrap_or_else(|| Value::from(""));
                m.insert("text".into(), text);
                m
            }
        };
        if t != "text" {
            if let Some(l) = label_of(e) {
                out.insert("label".into(), l);
            }
        }
        view.push(Value::Object(out));
    }

    let keep = ExcalidrawKeep {
        elements: live
            .into_iter()
            .enumerate()
            .filter(|(i, _)| kept_idx.contains(i))
            .map(|(_, e)| e)
            .collect(),
        extras: root,
    };
    let view_doc = serde_json::json!({
        "type": "excalidraw",
        "version": 2,
        "source": "otto",
        "elements": view,
    });
    let view_src = serde_json::to_string_pretty(&view_doc).unwrap_or_else(|_| view_doc.to_string());
    (view_src, keep)
}

/// Fold the set-aside elements + top-level keys back into the agent's scene.
/// Kept elements win over an agent element echoing the same id (it can't have
/// expressed them faithfully); `files` entries are unioned. An unparseable
/// agent scene is returned as-is (nothing to merge into).
pub fn merge_excalidraw(agent_src: &str, keep: &ExcalidrawKeep) -> String {
    if keep.is_empty() {
        return agent_src.to_string();
    }
    let Ok(Value::Object(mut root)) = serde_json::from_str::<Value>(agent_src) else {
        return agent_src.to_string();
    };
    let kept_ids: std::collections::HashSet<&str> =
        keep.elements.iter().filter_map(el_id).collect();
    let mut elements: Vec<Value> = match root.remove("elements") {
        Some(Value::Array(a)) => a
            .into_iter()
            .filter(|e| el_id(e).is_none_or(|id| !kept_ids.contains(id)))
            .collect(),
        _ => Vec::new(),
    };
    elements.extend(keep.elements.iter().cloned());
    root.insert("elements".into(), Value::Array(elements));
    for (k, v) in &keep.extras {
        match (root.get_mut(k), v) {
            (Some(Value::Object(mine)), Value::Object(theirs)) if k == "files" => {
                for (fk, fv) in theirs {
                    mine.entry(fk.clone()).or_insert_with(|| fv.clone());
                }
            }
            (Some(_), _) => {}
            (None, _) => {
                root.insert(k.clone(), v.clone());
            }
        }
    }
    Value::Object(root).to_string()
}
