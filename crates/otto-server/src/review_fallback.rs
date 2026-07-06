//! Deterministic summarizer FALLBACK for the multi-agent review engine.
//!
//! When the claude summarizer can't run (CLI missing, spawn failure), the
//! review used to degrade to raw concatenation of per-agent findings — no
//! dedupe, no ranking, no cap. This module is the Rust-side floor under that
//! degradation: it merges the per-agent batches deterministically and emits
//! JSON in the exact shape the summarizer would have produced, so the rest of
//! the pipeline (parse → comments → tracked findings → merge-readiness) is
//! untouched. It is strictly a floor — never a replacement for the claude
//! summarizer, which still owns the quality path.

use otto_core::domain::ReviewFinding;
use otto_core::finding::FindingSeverity;
use otto_state::review_findings::fallback_dedupe_key;

/// The summarizer enforces "at most 20 items"; the fallback honors the same cap.
const MAX_FINAL: usize = 20;

/// One merged finding, pre-serialization (summarizer output shape).
#[derive(serde::Serialize)]
struct MergedFinding {
    path: Option<String>,
    line: Option<u32>,
    severity: String,
    category: Option<String>,
    title: String,
    body: String,
    evidence: String,
    reasoning: String,
    suggested_fix: Option<String>,
}

/// Dedupe + rank the per-agent finding batches into the summarizer's JSON
/// output shape. Deterministic: same input → same output, always.
///
/// - **Dedupe** by [`fallback_dedupe_key`] (normalized path + 10-line bucket +
///   fingerprint body normalization); the highest-severity representative
///   wins and the agreement count is recorded in `reasoning`.
/// - **Rank** by normalized severity (blockers first), then cross-agent
///   agreement (more agents = higher confidence), then `(path, line)` for a
///   stable total order. Capped at the summarizer's ≤20.
pub fn deterministic_summary(batches: &[Vec<ReviewFinding>]) -> String {
    struct Slot {
        finding: ReviewFinding,
        rank: u8,
        agreement: usize,
        order: usize, // first-seen index — final tiebreaker for determinism
    }
    // Severity rank: 0 = critical … 4 = info (sortable ascending).
    fn sev_rank(s: &str) -> u8 {
        match FindingSeverity::normalize(s) {
            FindingSeverity::Critical => 0,
            FindingSeverity::High => 1,
            FindingSeverity::Medium => 2,
            FindingSeverity::Low => 3,
            FindingSeverity::Info => 4,
        }
    }

    let mut slots: Vec<Slot> = Vec::new();
    let mut by_key: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for f in batches.iter().flatten() {
        let key = fallback_dedupe_key(f.path.as_deref(), f.line, &f.body);
        match by_key.get(&key) {
            Some(&i) => {
                slots[i].agreement += 1;
                // Keep the most severe wording as the representative.
                let rank = sev_rank(&f.severity);
                if rank < slots[i].rank {
                    slots[i].rank = rank;
                    slots[i].finding = f.clone();
                }
            }
            None => {
                by_key.insert(key, slots.len());
                slots.push(Slot {
                    rank: sev_rank(&f.severity),
                    agreement: 1,
                    order: slots.len(),
                    finding: f.clone(),
                });
            }
        }
    }

    slots.sort_by(|a, b| {
        a.rank
            .cmp(&b.rank)
            .then(b.agreement.cmp(&a.agreement))
            .then(a.finding.path.cmp(&b.finding.path))
            .then(a.finding.line.cmp(&b.finding.line))
            .then(a.order.cmp(&b.order))
    });
    slots.truncate(MAX_FINAL);

    let merged: Vec<MergedFinding> = slots
        .into_iter()
        .map(|s| {
            let f = s.finding;
            let title: String = f
                .body
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(120)
                .collect();
            MergedFinding {
                evidence: f.body.clone(),
                reasoning: format!(
                    "deterministic fallback — reported by {} agent{}",
                    s.agreement,
                    if s.agreement == 1 { "" } else { "s" }
                ),
                title,
                path: f.path,
                line: f.line,
                severity: f.severity,
                category: None,
                body: f.body,
                suggested_fix: None,
            }
        })
        .collect();
    serde_json::to_string(&merged).unwrap_or_else(|_| "[]".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, line: u32, severity: &str, body: &str) -> ReviewFinding {
        ReviewFinding {
            path: Some(path.to_string()),
            line: Some(line),
            severity: severity.to_string(),
            body: body.to_string(),
        }
    }

    fn parse(json: &str) -> Vec<serde_json::Value> {
        serde_json::from_str(json).unwrap()
    }

    #[test]
    fn empty_batches_yield_empty_array() {
        assert_eq!(deterministic_summary(&[]), "[]");
        assert_eq!(deterministic_summary(&[vec![], vec![]]), "[]");
    }

    #[test]
    fn duplicates_across_agents_collapse_with_agreement() {
        // Same path, same 10-line bucket (13 and 17 → bucket 1), same body →
        // one merged finding that records both reporters; the higher-severity
        // wording wins.
        let out = parse(&deterministic_summary(&[
            vec![f("a.rs", 13, "warn", "Unchecked unwrap here")],
            vec![f("a.rs", 17, "bug", "Unchecked unwrap here")],
        ]));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["severity"], "bug");
        assert_eq!(out[0]["reasoning"], "deterministic fallback — reported by 2 agents");
        // Summarizer output shape: enriched fields present.
        assert_eq!(out[0]["title"], "Unchecked unwrap here");
        assert_eq!(out[0]["evidence"], "Unchecked unwrap here");
        assert!(out[0]["category"].is_null());
    }

    #[test]
    fn rank_is_severity_then_agreement_then_location_and_stable() {
        let batches = vec![
            vec![
                f("z.rs", 5, "info", "note z"),
                f("b.rs", 40, "warn", "warn b"),
                f("a.rs", 9, "warn", "agreed warn"),
            ],
            vec![f("a.rs", 9, "warn", "agreed warn"), f("c.rs", 1, "bug", "the bug")],
        ];
        let expect = |o: &[serde_json::Value]| {
            // bug first; among warns the 2-agent one beats the 1-agent one;
            // info last.
            assert_eq!(o[0]["body"], "the bug");
            assert_eq!(o[1]["body"], "agreed warn");
            assert_eq!(o[2]["body"], "warn b");
            assert_eq!(o[3]["body"], "note z");
        };
        let once = parse(&deterministic_summary(&batches));
        assert_eq!(once.len(), 4);
        expect(&once);
        // Deterministic: identical input → byte-identical output.
        assert_eq!(deterministic_summary(&batches), deterministic_summary(&batches));
    }

    #[test]
    fn caps_at_twenty_like_the_summarizer() {
        let many: Vec<ReviewFinding> = (0..25)
            .map(|i| f(&format!("f{i}.rs"), i, "warn", &format!("finding {i}")))
            .collect();
        let out = parse(&deterministic_summary(&[many]));
        assert_eq!(out.len(), 20);
    }
}
