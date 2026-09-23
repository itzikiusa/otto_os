//! Reference citations for agent turns (proposal §6.1 "Agents cite"). A
//! design-assist turn offers the agent a numbered set of references —
//! `[R1] <title> (<studio>, <status>, v12)` — and the agent names what it
//! borrowed, either inline in its reply (`… layout rhythm from [R1]`, also
//! `[R1, R3]`) or in an optional `provenance.json` (`{"refs": ["R1",
//! "<artifact>@v12", "otto://design/<id>@v3"], "why": "…"}`). The server
//! VERIFIES every citation against the offered set: a marker that was never
//! offered, an unknown artifact, or a version that differs from the offered
//! one is dropped from `cited` and reported in `unverified` — provenance never
//! records a reference the agent was not actually given. Pure; unit-tested.

use otto_core::Id;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::uri::{DesignUri, VersionSel};

/// Cap on citations read from one turn (markers + provenance refs).
pub const MAX_CITATIONS: usize = 64;
const MAX_WHY_CHARS: usize = 300;
const MAX_UNVERIFIED_CHARS: usize = 120;

/// One reference offered to an agent turn as `[R<n>]`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OfferedRef {
    /// `R1`, `R2`, … (1-based, stable order).
    pub label: String,
    pub artifact_id: Id,
    /// The exact version offered (approved, else head, else the pinned one).
    pub version_id: Option<Id>,
    pub seq: Option<i64>,
    pub title: String,
    pub studio: String,
    pub format: String,
    pub status: String,
    /// Why it was offered: `explicit` (the request's `references`), `link`
    /// (an explicit `references`/`derived_from` link), `search` (library FTS).
    pub source: String,
}

/// One verified citation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CitedRef {
    pub label: String,
    pub artifact_id: Id,
    pub version_id: Option<Id>,
    pub seq: Option<i64>,
}

/// What a turn cited, after verification.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CitationReport {
    /// Verified citations, in first-mention order, deduplicated.
    pub cited: Vec<CitedRef>,
    /// Citations that could NOT be verified (never offered / wrong version /
    /// unparseable) — dropped from `cited`, kept here for transparency.
    pub unverified: Vec<String>,
    /// The agent's one-line "why" from `provenance.json`, if any (capped).
    pub why: Option<String>,
}

/// `[R1]`, `[R1, R3]`, `[r2]` → `R1`, `R3`, `R2` (first-mention order,
/// deduplicated, capped). Only bracket groups made ENTIRELY of `R<digits>`
/// tokens count, so prose like `[see R1 above]` or a Markdown link isn't
/// mistaken for a citation.
pub fn scan_markers(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(']') else {
            break;
        };
        let inner = &after[..close];
        rest = &after[close + 1..];
        if inner.is_empty() || inner.len() > 48 {
            continue;
        }
        let tokens: Vec<&str> = inner
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|t| !t.is_empty())
            .collect();
        let labels: Option<Vec<String>> = tokens.iter().map(|t| marker_label(t)).collect();
        if let Some(labels) = labels.filter(|l| !l.is_empty()) {
            for l in labels {
                if !out.contains(&l) && out.len() < MAX_CITATIONS {
                    out.push(l);
                }
            }
        }
    }
    out
}

/// `R12` / `r12` → `R12` (1..=999); anything else → `None`.
fn marker_label(t: &str) -> Option<String> {
    let digits = t.strip_prefix('R').or_else(|| t.strip_prefix('r'))?;
    if digits.is_empty() || digits.len() > 3 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let n: u32 = digits.parse().ok()?;
    (n >= 1).then(|| format!("R{n}"))
}

/// Verify the agent's citations — reply markers plus an optional
/// `provenance.json` value — against the offered references.
pub fn verify(offered: &[OfferedRef], reply: &str, provenance: Option<&Value>) -> CitationReport {
    let mut report = CitationReport::default();
    let mut raw: Vec<String> = scan_markers(reply);
    if let Some(p) = provenance {
        if let Some(refs) = p.get("refs").and_then(Value::as_array) {
            for r in refs.iter().take(MAX_CITATIONS) {
                if let Some(s) = r.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    let s = s.trim_start_matches('[').trim_end_matches(']').to_string();
                    if !raw.contains(&s) {
                        raw.push(s);
                    }
                }
            }
        }
        report.why = p
            .get("why")
            .and_then(Value::as_str)
            .map(|w| w.trim().chars().take(MAX_WHY_CHARS).collect::<String>())
            .filter(|w| !w.is_empty());
    }
    for c in raw.into_iter().take(MAX_CITATIONS) {
        match resolve(offered, &c) {
            Some(o) => {
                if !report.cited.iter().any(|x| x.label == o.label) {
                    report.cited.push(CitedRef {
                        label: o.label.clone(),
                        artifact_id: o.artifact_id.clone(),
                        version_id: o.version_id.clone(),
                        seq: o.seq,
                    });
                }
            }
            None => {
                let c: String = c.chars().take(MAX_UNVERIFIED_CHARS).collect();
                if !report.unverified.contains(&c) {
                    report.unverified.push(c);
                }
            }
        }
    }
    report
}

/// Match one citation (`R2`, `<artifact_id>`, `<artifact_id>@v12`,
/// `otto://design/<id>[@v12]`) to an offered reference. A version that is
/// named must be the offered one; `@approved`/`@latest` count as "the one
/// offered".
fn resolve<'a>(offered: &'a [OfferedRef], c: &str) -> Option<&'a OfferedRef> {
    if let Some(label) = marker_label(c) {
        return offered.iter().find(|o| o.label == label);
    }
    let uri = if c.starts_with("otto://") {
        DesignUri::parse(c)?
    } else {
        DesignUri::parse(&format!("otto://design/{c}"))?
    };
    let o = offered.iter().find(|o| o.artifact_id == uri.artifact_id)?;
    match uri.version {
        VersionSel::Seq(n) => (o.seq == Some(n)).then_some(o),
        _ => Some(o),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn offered() -> Vec<OfferedRef> {
        ["A1", "B2"]
            .iter()
            .enumerate()
            .map(|(i, id)| OfferedRef {
                label: format!("R{}", i + 1),
                artifact_id: id.to_string(),
                version_id: Some(format!("v-{id}")),
                seq: Some(12),
                title: format!("T{id}"),
                studio: "site".into(),
                format: "html".into(),
                status: "shipped".into(),
                source: "search".into(),
            })
            .collect()
    }

    #[test]
    fn markers_need_a_pure_bracket_group() {
        assert_eq!(
            scan_markers("rhythm from [R1], palette [R2, r3] and [R1] again"),
            vec!["R1", "R2", "R3"]
        );
        assert!(scan_markers("see [the R1 layout] and [link](x) [R0] [R1000]").is_empty());
        assert!(scan_markers("unclosed [R1").is_empty());
    }

    #[test]
    fn verify_keeps_offered_drops_the_rest() {
        let r = verify(
            &offered(),
            "Borrowed the hero from [R1] and the grid from [R7].",
            Some(&json!({
                "refs": ["R2", "A1@v12", "B2@v3", "ZZZ", "otto://design/B2@approved"],
                "why": "  layout rhythm  "
            })),
        );
        let labels: Vec<&str> = r.cited.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["R1", "R2"]);
        assert_eq!(r.cited[0].version_id.as_deref(), Some("v-A1"));
        // R7 never offered; B2@v3 names a version other than the offered v12;
        // ZZZ is no offered artifact.
        assert!(r.unverified.contains(&"R7".to_string()));
        assert!(r.unverified.contains(&"B2@v3".to_string()));
        assert!(r.unverified.contains(&"ZZZ".to_string()));
        assert_eq!(r.why.as_deref(), Some("layout rhythm"));
    }

    #[test]
    fn nothing_offered_means_nothing_verifies() {
        let r = verify(&[], "from [R1]", None);
        assert!(r.cited.is_empty());
        assert_eq!(r.unverified, vec!["R1"]);
        assert_eq!(
            verify(&offered(), "plain reply", None),
            CitationReport::default()
        );
    }
}
