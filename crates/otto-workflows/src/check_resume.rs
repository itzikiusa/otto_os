//! Restart-recovery helpers for the `check` workflow node: identifying which
//! check phase (`criteria` vs `completion_review`) a held thread belongs to
//! without trusting mutable or workflow-prefixed titles, and persisting an
//! accepted evaluator verdict before entering completion review.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use otto_core::workflows::StepThread;
use otto_core::Id;
use serde_json::Value;

use crate::criteria::{self, Evaluation};

/// In-memory map of steps a restart caught on an open thread:
/// `(run_id, node_id)` -> `(thread_id, start_ordinal)`.
type Resumes = Mutex<HashMap<(Id, String), (Id, u32)>>;

fn resumes() -> &'static Resumes {
    static R: OnceLock<Resumes> = OnceLock::new();
    R.get_or_init(Default::default)
}

pub fn mark_resume(run_id: &Id, node_id: &str, t: &StepThread) {
    let ordinal = t.start_ordinal.unwrap_or(0) as u32;
    if let Ok(mut m) = resumes().lock() {
        m.insert(
            (run_id.clone(), node_id.to_string()),
            (t.thread_id.clone(), ordinal),
        );
    }
}

pub fn peek_resume(run_id: &Id, node_id: &str) -> Option<(Id, u32)> {
    resumes()
        .lock()
        .ok()
        .and_then(|m| m.get(&(run_id.clone(), node_id.to_string())).cloned())
}

pub fn take_resume(run_id: &Id, node_id: &str) -> Option<(Id, u32)> {
    resumes()
        .lock()
        .ok()
        .and_then(|mut m| m.remove(&(run_id.clone(), node_id.to_string())))
}

pub fn restore_resume(run_id: &Id, node_id: &str, entry: (Id, u32)) {
    if let Ok(mut m) = resumes().lock() {
        m.insert((run_id.clone(), node_id.to_string()), entry);
    }
}

pub fn forget_run(run_id: &Id) {
    if let Ok(mut m) = resumes().lock() {
        m.retain(|(id, _), _| id != run_id);
    }
}

/// Thread metadata key tagging which phase of a two-turn `check` step a
/// runtime thread belongs to (`criteria` | `completion_review`).
pub const CHECK_PHASE_META_KEY: &str = "check_phase";
pub const PHASE_CRITERIA: &str = "criteria";
pub const PHASE_COMPLETION_REVIEW: &str = "completion_review";

/// Whether a held check thread belongs to Turn 2 (`Completion review`).
///
/// Prefers explicit `meta.check_phase` so neither renaming a thread in the UI
/// nor naming a workflow "Completion review" can misclassify the phase. For
/// older unrenamed threads created before `check_phase` existed, inspects only
/// the generated `<label>` segment of `"WF <wf> · <label> · run <short_id>"`.
pub fn is_completion_review_thread(meta: Option<&Value>, title: &str) -> bool {
    if let Some(phase) = meta
        .and_then(|m| m.get(CHECK_PHASE_META_KEY))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return phase == PHASE_COMPLETION_REVIEW;
    }
    generated_title_label(title) == Some("Completion review")
}

/// Extract `<label>` from `"WF <wf_name> · <label> · run <short_id>"`.
fn generated_title_label(title: &str) -> Option<&str> {
    let (prefix, _) = title.rsplit_once(" · run ")?;
    let (_, label) = prefix.rsplit_once(" · ")?;
    Some(label)
}

/// Companion path beside `output.json` where an accepted evaluator `Evaluation`
/// (including one parsed from the agent's reply after malformed `output.json`)
/// is saved before Turn 2 (`Completion review`) begins.
pub fn accepted_eval_path(output_path: &Path) -> PathBuf {
    output_path.with_extension("accepted.json")
}

/// Persist an accepted Turn-1 evaluation so a restart during Turn 2 reloads it
/// without re-running the evaluator (even if `output.json` was malformed).
pub async fn save_accepted_evaluation(output_path: Option<&Path>, ev: &Evaluation) {
    let Some(path) = output_path else { return };
    if let Ok(json) = serde_json::to_string(ev) {
        let _ = tokio::fs::write(accepted_eval_path(path), json).await;
    }
}

/// Clear any previous attempt's accepted evaluation before a fresh Turn 1.
pub async fn clear_accepted_evaluation(output_path: Option<&Path>) {
    if let Some(path) = output_path {
        let _ = tokio::fs::remove_file(accepted_eval_path(path)).await;
    }
}

/// Reload the accepted Turn-1 evaluation during a Turn-2 restart resume:
/// prefers the persisted canonical snapshot, falling back to `output.json` for
/// pre-patch runs.
pub async fn load_accepted_evaluation(output_path: Option<&Path>) -> Option<Evaluation> {
    let path = output_path?;
    if let Ok(raw) = tokio::fs::read_to_string(accepted_eval_path(path)).await {
        if let Ok(ev) = serde_json::from_str::<Evaluation>(&raw) {
            return Some(ev);
        }
    }
    let raw = tokio::fs::read_to_string(path).await.ok()?;
    criteria::parse_evaluation(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::criteria::{Assessed, Verdict};
    use serde_json::json;

    #[test]
    fn completion_review_detection_uses_meta_and_never_confuses_workflow_name() {
        // BUG-9: workflow itself named "Completion review" while Turn 1 ("Check criteria") runs.
        let wf_named_title = "WF Completion review · Check criteria · run 01abc123";
        assert!(
            !is_completion_review_thread(
                Some(&json!({"check_phase": PHASE_CRITERIA})),
                wf_named_title
            ),
            "criteria meta must never match completion review"
        );
        assert!(
            !is_completion_review_thread(None, wf_named_title),
            "legacy title parser must inspect only the generated label, not the workflow name"
        );

        // BUG-9: user renamed a real completion-review thread in Threads.
        assert!(
            is_completion_review_thread(
                Some(&json!({"check_phase": PHASE_COMPLETION_REVIEW})),
                "My renamed check thread"
            ),
            "completion_review meta must survive thread renames"
        );

        // Legacy unrenamed completion-review thread without metadata still matches.
        assert!(is_completion_review_thread(
            None,
            "WF Build feature · Completion review · run 01abc123"
        ));
    }

    #[tokio::test]
    async fn accepted_evaluation_survives_malformed_output_json_on_restart() {
        // DATA-1: evaluator wrote malformed output.json, but returned valid structured reply.
        let dir = tempfile::tempdir().unwrap();
        let output_json = dir.path().join("step-check.output.json");
        tokio::fs::write(&output_json, "{ malformed json")
            .await
            .unwrap();
        let reply_eval = Evaluation {
            criteria: vec![Assessed {
                id: "c1".into(),
                met: true,
                evidence: "verified in reply".into(),
                verified_by: Some("agent".into()),
                progress: None,
            }],
            verdict: Verdict::Achieved,
            feedback: String::new(),
            rationale: "all met".into(),
        };
        save_accepted_evaluation(Some(&output_json), &reply_eval).await;
        let reloaded = load_accepted_evaluation(Some(&output_json))
            .await
            .expect("persisted accepted evaluation must reload even when output.json is malformed");
        assert_eq!(reloaded, reply_eval);
    }
}
