//! Learning v1 — the DETERMINISTIC candidate extractor (proposal §6.2 step 1
//! "Deterministic aggregation"). Groups repeated design signals into candidate
//! team rules; nothing here calls a model or writes anything. The server feeds
//! the ready candidates to `otto-improve`'s `design` evidence source, which
//! proposes them (suggest-only) as pending edits to the `design-team-style`
//! skill for a human to approve / reject / roll back.
//!
//! Patterns (each needs ≥ [`MIN_SIGNALS`] signals across ≥ [`MIN_ARTIFACTS`]
//! artifacts to be `ready`):
//! - **variant preference** — the same variant direction chosen repeatedly
//!   (`variant_accepted` / `variant_chosen` with `payload.direction`), and
//!   chosen more often than it was rejected;
//! - **reject reason** — the same reason chip on rejected variants
//!   (`variant_rejected` with `payload.reason`, `other` ignored);
//! - **edit after draft** — humans keep changing the same property right after
//!   an agent draft (`edit_after_draft` `payload.summary.changed_paths`,
//!   normalized to the last two path segments, per format);
//! - **a11y fix** — an accessibility fix accepted repeatedly (`a11y_fix` with
//!   `payload.rule` and a `accepted`/`fixed` disposition).
//!
//! Output order is deterministic: ready first, then by signal count, then key.
//! Pure; unit-tested.

use std::collections::{BTreeMap, BTreeSet};

use otto_core::Id;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::DesignSignal;

/// Repetitions before a pattern becomes a candidate rule.
pub const MIN_SIGNALS: usize = 3;
/// Distinct artifacts a pattern must span (one artifact's quirk is not a
/// team rule).
pub const MIN_ARTIFACTS: usize = 2;
/// Evidence ids kept per candidate.
pub const MAX_EVIDENCE: usize = 20;
/// Candidates returned per extraction.
pub const MAX_CANDIDATES: usize = 50;
const MAX_SLUG: usize = 48;

/// One candidate team rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleCandidate {
    /// Stable identity (`variant_direction:bold`, `reject_reason:too_busy`,
    /// `edit_property:scene3d:material.color`, `a11y:color-contrast`) — the
    /// dedup key of the `design` evidence source.
    pub key: String,
    /// `variant_preference` | `reject_reason` | `edit_after_draft` | `a11y`.
    pub kind: String,
    /// The one-line rule an agent would follow.
    pub rule: String,
    /// Why it was proposed (counts), for the reviewer.
    pub rationale: String,
    /// Evidence: the signal ids behind it (≤ [`MAX_EVIDENCE`], newest first).
    pub signal_ids: Vec<Id>,
    pub signal_count: usize,
    pub artifact_count: usize,
    /// Meets the thresholds (only ready candidates are proposed).
    pub ready: bool,
}

/// A lowercase `[a-z0-9_-]` slug (spaces → `_`), capped; empty when nothing
/// usable remains.
pub fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.trim().chars().flat_map(char::to_lowercase) {
        if out.len() >= MAX_SLUG {
            break;
        }
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
            out.push(c);
        } else if (c.is_whitespace() || c == '.') && !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_string()
}

/// A JSON change path → the property it names: `+`/`-` markers dropped,
/// `[id]`/`[3]` → `[]`, the last two dotted segments kept
/// (`objects[hero].material.color` → `material.color`). `None` for the
/// document root.
pub fn property_of(path: &str) -> Option<String> {
    let p = path.trim().trim_start_matches(['+', '-']);
    if p.is_empty() || p == "$" {
        return None;
    }
    let mut norm = String::new();
    let mut depth = 0usize;
    for c in p.chars() {
        match c {
            '[' => {
                depth += 1;
                if depth == 1 {
                    norm.push_str("[]");
                }
            }
            ']' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            _ => norm.push(c),
        }
    }
    let segs: Vec<&str> = norm.split('.').filter(|s| !s.is_empty()).collect();
    let tail = if segs.len() > 2 {
        &segs[segs.len() - 2..]
    } else {
        &segs[..]
    };
    let out = tail.join(".");
    let out: String = out.chars().take(80).collect();
    (!out.is_empty() && out != "[]").then_some(out)
}

#[derive(Default)]
struct Group {
    label: String,
    extra: String,
    signals: Vec<Id>,
    artifacts: BTreeSet<Id>,
    against: usize,
}

impl Group {
    fn add(&mut self, s: &DesignSignal) {
        if !self.signals.contains(&s.id) {
            self.signals.push(s.id.clone());
            self.artifacts.insert(s.artifact_id.clone());
        }
    }
}

fn payload_str<'a>(s: &'a DesignSignal, key: &str) -> Option<&'a str> {
    s.payload
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

/// Aggregate `signals` (any order) into candidate rules.
pub fn extract(signals: &[DesignSignal]) -> Vec<RuleCandidate> {
    // Newest first, so evidence lists keep the most recent ids.
    let mut sorted: Vec<&DesignSignal> = signals.iter().collect();
    sorted.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));

    let mut groups: BTreeMap<String, (String, Group)> = BTreeMap::new();
    let mut rejected_dirs: BTreeMap<String, usize> = BTreeMap::new();

    for s in &sorted {
        match s.kind.as_str() {
            "variant_accepted" | "variant_chosen" => {
                if let Some(d) = payload_str(s, "direction") {
                    let key = slug(d);
                    if key.is_empty() {
                        continue;
                    }
                    let g = &mut groups
                        .entry(format!("variant_direction:{key}"))
                        .or_insert_with(|| ("variant_preference".into(), Group::default()))
                        .1;
                    if g.label.is_empty() {
                        g.label = d.chars().take(MAX_SLUG).collect();
                    }
                    g.add(s);
                }
            }
            "variant_rejected" => {
                if let Some(d) = payload_str(s, "direction") {
                    let key = slug(d);
                    if !key.is_empty() {
                        *rejected_dirs.entry(key).or_default() += 1;
                    }
                }
                if let Some(r) = payload_str(s, "reason") {
                    let key = slug(r);
                    if key.is_empty() || key == "other" {
                        continue;
                    }
                    let g = &mut groups
                        .entry(format!("reject_reason:{key}"))
                        .or_insert_with(|| ("reject_reason".into(), Group::default()))
                        .1;
                    if g.label.is_empty() {
                        g.label = key.replace('_', " ");
                    }
                    g.add(s);
                }
            }
            "edit_after_draft" => {
                let format = slug(payload_str(s, "format").unwrap_or("design"));
                let paths = s
                    .payload
                    .get("summary")
                    .and_then(|v| v.get("changed_paths"))
                    .and_then(Value::as_array);
                let Some(paths) = paths else {
                    continue;
                };
                let props: BTreeSet<String> = paths
                    .iter()
                    .filter_map(Value::as_str)
                    .filter_map(property_of)
                    .collect();
                for p in props {
                    let g = &mut groups
                        .entry(format!("edit_property:{format}:{p}"))
                        .or_insert_with(|| ("edit_after_draft".into(), Group::default()))
                        .1;
                    if g.label.is_empty() {
                        g.label = p.clone();
                        g.extra = format.clone();
                    }
                    g.add(s);
                }
            }
            "a11y_fix" => {
                let disposition = payload_str(s, "disposition").unwrap_or("accepted");
                if !matches!(disposition, "accepted" | "fixed" | "applied") {
                    continue;
                }
                let Some(rule) = payload_str(s, "rule") else {
                    continue;
                };
                let key = slug(rule);
                if key.is_empty() {
                    continue;
                }
                let g = &mut groups
                    .entry(format!("a11y:{key}"))
                    .or_insert_with(|| ("a11y".into(), Group::default()))
                    .1;
                if g.label.is_empty() {
                    g.label = rule.chars().take(80).collect();
                }
                g.add(s);
            }
            _ => {}
        }
    }

    let mut out: Vec<RuleCandidate> = groups
        .into_iter()
        .map(|(key, (kind, mut g))| {
            if kind == "variant_preference" {
                let dir = key.trim_start_matches("variant_direction:");
                g.against = rejected_dirs.get(dir).copied().unwrap_or(0);
            }
            let n = g.signals.len();
            let m = g.artifacts.len();
            let (rule, rationale) = phrase(&kind, &g, n, m);
            let mut ready = n >= MIN_SIGNALS && m >= MIN_ARTIFACTS;
            if kind == "variant_preference" {
                ready &= n > g.against;
            }
            RuleCandidate {
                key,
                kind,
                rule,
                rationale,
                signal_ids: g.signals.iter().take(MAX_EVIDENCE).cloned().collect(),
                signal_count: n,
                artifact_count: m,
                ready,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.ready
            .cmp(&a.ready)
            .then(b.signal_count.cmp(&a.signal_count))
            .then(a.key.cmp(&b.key))
    });
    out.truncate(MAX_CANDIDATES);
    out
}

fn phrase(kind: &str, g: &Group, n: usize, m: usize) -> (String, String) {
    let arts = if m == 1 { "artifact" } else { "artifacts" };
    match kind {
        "variant_preference" => (
            format!(
                "When offering design directions, lead with the \"{}\" direction — the team picks it most.",
                g.label
            ),
            format!(
                "Chosen {n}× across {m} {arts} (rejected {}× when offered).",
                g.against
            ),
        ),
        "reject_reason" => (
            format!(
                "Avoid drafts that read as \"{}\" — the team rejects variants for it.",
                g.label
            ),
            format!("Variants rejected as \"{}\" {n}× across {m} {arts}.", g.label),
        ),
        "edit_after_draft" => (
            format!(
                "In {} drafts, get `{}` right the first time (match the team's usual choice) — humans keep changing it after the agent's draft.",
                g.extra, g.label
            ),
            format!(
                "`{}` edited right after an agent draft {n}× across {m} {} {arts}.",
                g.label, g.extra
            ),
        ),
        _ => (
            format!(
                "Before finishing a design, check the \"{}\" accessibility rule — the team keeps accepting that fix.",
                g.label
            ),
            format!("Accessibility fix \"{}\" accepted {n}× across {m} {arts}.", g.label),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use serde_json::json;

    fn sig(id: &str, art: &str, kind: &str, payload: Value, age_min: i64) -> DesignSignal {
        DesignSignal {
            id: id.into(),
            workspace_id: "w1".into(),
            artifact_id: art.into(),
            version_id: None,
            kind: kind.into(),
            actor_kind: "user".into(),
            actor_id: "u1".into(),
            session_id: None,
            payload,
            created_at: Utc::now() - Duration::minutes(age_min),
        }
    }

    #[test]
    fn slugs_and_properties_normalize() {
        assert_eq!(slug("  Too Busy "), "too_busy");
        assert_eq!(slug("Off-brand!"), "off-brand");
        assert_eq!(slug("???"), "");
        assert_eq!(
            property_of("objects[hero].material.color").as_deref(),
            Some("material.color")
        );
        assert_eq!(
            property_of("+pages[0].sections[faq]").as_deref(),
            Some("pages[].sections[]")
        );
        assert_eq!(property_of("title").as_deref(), Some("title"));
        assert_eq!(property_of("$"), None);
        assert_eq!(property_of("-[x]"), None);
    }

    #[test]
    fn repeated_variant_choices_become_a_ready_preference() {
        let sigs = vec![
            sig(
                "s1",
                "a1",
                "variant_accepted",
                json!({"direction": "Bold"}),
                1,
            ),
            sig(
                "s2",
                "a2",
                "variant_accepted",
                json!({"direction": "bold"}),
                2,
            ),
            sig(
                "s3",
                "a1",
                "variant_chosen",
                json!({"direction": "bold"}),
                3,
            ),
            sig(
                "s4",
                "a3",
                "variant_rejected",
                json!({"direction": "bold"}),
                4,
            ),
            sig(
                "s5",
                "a3",
                "variant_accepted",
                json!({"direction": "calm"}),
                5,
            ),
        ];
        let c = extract(&sigs);
        let bold = c
            .iter()
            .find(|x| x.key == "variant_direction:bold")
            .unwrap();
        assert!(bold.ready, "{bold:?}");
        assert_eq!(bold.signal_count, 3);
        assert_eq!(bold.artifact_count, 2);
        assert_eq!(bold.signal_ids, vec!["s1", "s2", "s3"]);
        assert!(bold.rationale.contains("rejected 1×"));
        let calm = c
            .iter()
            .find(|x| x.key == "variant_direction:calm")
            .unwrap();
        assert!(!calm.ready);
        // Ready candidates sort first.
        assert_eq!(c[0].key, "variant_direction:bold");
        // Deterministic: same input, same output.
        assert_eq!(extract(&sigs), c);
    }

    #[test]
    fn one_artifact_or_a_losing_direction_is_not_a_team_rule() {
        let same_art: Vec<DesignSignal> = (0..4)
            .map(|i| {
                sig(
                    &format!("s{i}"),
                    "a1",
                    "variant_accepted",
                    json!({"direction": "bold"}),
                    i,
                )
            })
            .collect();
        assert!(!extract(&same_art)[0].ready);
        let mut losing = vec![
            sig(
                "w1",
                "a1",
                "variant_accepted",
                json!({"direction": "bold"}),
                1,
            ),
            sig(
                "w2",
                "a2",
                "variant_accepted",
                json!({"direction": "bold"}),
                2,
            ),
            sig(
                "w3",
                "a3",
                "variant_accepted",
                json!({"direction": "bold"}),
                3,
            ),
        ];
        for i in 0..3 {
            losing.push(sig(
                &format!("r{i}"),
                "a4",
                "variant_rejected",
                json!({"direction": "bold"}),
                10 + i,
            ));
        }
        let c = extract(&losing);
        assert!(
            !c.iter()
                .find(|x| x.kind == "variant_preference")
                .unwrap()
                .ready
        );
    }

    #[test]
    fn edits_reject_reasons_and_a11y_fixes_aggregate() {
        let edit = |id: &str, art: &str| {
            sig(
                id,
                art,
                "edit_after_draft",
                json!({"format": "scene3d", "summary": {"kind": "json",
                    "changed_paths": ["objects[a].material.color", "objects[b].material.color", "lights[k].intensity"]}}),
                1,
            )
        };
        let mut sigs = vec![edit("e1", "a1"), edit("e2", "a2"), edit("e3", "a2")];
        for (i, a) in ["a1", "a2", "a3"].iter().enumerate() {
            sigs.push(sig(
                &format!("r{i}"),
                a,
                "variant_rejected",
                json!({"reason": "too busy"}),
                2,
            ));
            sigs.push(sig(
                &format!("x{i}"),
                a,
                "variant_rejected",
                json!({"reason": "other"}),
                2,
            ));
            sigs.push(sig(
                &format!("y{i}"),
                a,
                "a11y_fix",
                json!({"rule": "color-contrast", "disposition": "accepted"}),
                2,
            ));
            sigs.push(sig(
                &format!("z{i}"),
                a,
                "a11y_fix",
                json!({"rule": "tap-target", "disposition": "dismissed"}),
                2,
            ));
        }
        let c = extract(&sigs);
        let color = c
            .iter()
            .find(|x| x.key == "edit_property:scene3d:material.color")
            .unwrap();
        // One signal touching the property twice counts once.
        assert_eq!(color.signal_count, 3);
        assert!(color.ready);
        assert!(color.rule.contains("scene3d") && color.rule.contains("material.color"));
        assert!(c
            .iter()
            .any(|x| x.key == "reject_reason:too_busy" && x.ready));
        assert!(!c.iter().any(|x| x.key == "reject_reason:other"));
        assert!(c.iter().any(|x| x.key == "a11y:color-contrast" && x.ready));
        assert!(!c.iter().any(|x| x.key == "a11y:tap-target"));
    }
}
