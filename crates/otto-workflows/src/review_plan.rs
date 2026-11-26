//! Pure halves of planning a `review_run` step, moved out of otto-server's
//! `workflow_engine/nodes/review_run.rs`: the review mode it was told to use,
//! the progress-post label and the prior run context its reviewers get.

use crate::context::RunContextFiles;
use otto_core::domain::ReviewMode;
use serde_json::Value;

/// Cap on the prior-step context handed to reviewers. Their prompt already
/// carries the whole diff; the briefs are there to stop them re-deriving the
/// system, not to become the bulk of the prompt.
pub const REVIEW_CONTEXT_CAP: usize = 48_000;

/// R2: the review mode a `review_run` step was TOLD to use, with the source for
/// its log line — the run's own input first (the run dialog's override, which
/// must beat a node that sets its own), then the node's `params.mode`. `None`
/// when neither names a valid mode; the caller then falls back to the repo's
/// stored config. Unparseable values are treated as absent (never an error).
///
/// Note the signature: there is no node-INPUT parameter. Node inputs lose
/// run-level keys hop by hop, so a `review_mode` riding one is ignored by
/// construction.
pub fn resolve_review_mode_source(
    run_input: &Value,
    params: &Value,
) -> Option<(ReviewMode, &'static str)> {
    if let Some(m) = run_input
        .get("review_mode")
        .and_then(Value::as_str)
        .and_then(ReviewMode::parse)
    {
        return Some((m, "run override"));
    }
    params
        .get("mode")
        .and_then(Value::as_str)
        .and_then(ReviewMode::parse)
        .map(|m| (m, "node"))
}

/// Progress-post label fragments shared by every target:
/// " · lenses: …", " · providers: …", " · N check(s) …" (each may be empty).
pub fn label_tail(
    reviewers: &[Value],
    lenses: &[String],
    providers: &[String],
    check_specs: &[(String, String)],
) -> String {
    let lens_list: Vec<String> = if reviewers.is_empty() {
        lenses.to_vec()
    } else {
        reviewers
            .iter()
            .filter_map(|r| {
                r.get("lens")
                    .or_else(|| r.get("skill"))
                    .or_else(|| r.get("name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
            .collect()
    };
    let mut tail = String::new();
    if !lens_list.is_empty() {
        tail.push_str(&format!(" · lenses: {}", lens_list.join(", ")));
    }
    if !providers.is_empty() {
        tail.push_str(&format!(" · providers: {}", providers.join(", ")));
    }
    if !check_specs.is_empty() {
        let n = check_specs.len();
        tail.push_str(&format!(" · {n} check(s) delegated to the reviewer"));
    }
    tail
}

/// What this run already established about the change, handed to the
/// reviewers: `(ticket, step briefs, log line)`.
///
/// Every OTHER node kind gets the run context directory named in its
/// preamble and reads it; `review_run` is not an agent node, so its
/// reviewers used to see the diff and nothing else — no ticket, no brief
/// from the steps that ran before them. They then re-derive the system from
/// raw hunks, and behavior that is documented, ticketed and intentional
/// comes back as a defect of whichever change happened to touch its line.
/// The engine already renders both blocks into every reviewer prompt
/// (`run_review_core`); only this caller withheld them.
///
/// `include_context: false` opts out.
pub fn prior_run_context(
    files: &RunContextFiles,
    p: &Value,
) -> (Option<String>, Option<String>, Option<String>) {
    let want_context = p
        .get("include_context")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !want_context {
        return (None, None, None);
    }
    let jira = files.read_jira_md();
    // The step handoffs, oldest first, capped — the reviewer prompt
    // also carries the whole diff, so this must not grow unbounded.
    let mut brief = String::new();
    for name in files.list_step_mds() {
        let Some(body) = files.read_named(&name) else {
            continue;
        };
        if brief.len() + body.len() > REVIEW_CONTEXT_CAP {
            brief.push_str(
                "\n\n[remaining step files omitted — context cap reached; \
                         they are in the run context directory]\n",
            );
            break;
        }
        brief.push_str(&format!("\n\n### {name}\n\n{body}"));
    }
    if let Some(previous) = files.read_named("previous-review.md") {
        brief.push_str(&format!(
            "\n\nHISTORICAL PRIOR ITERATION (not current authority):\n{previous}"
        ));
    }
    let run = (!brief.trim().is_empty()).then(|| {
        format!(
            "The current-run steps below ran BEFORE this review over this same \
                     change. The explicitly marked HISTORICAL PRIOR ITERATION is different: verify its findings against current code and do not treat it as current authority. Treat them as established fact about the system — not as claims \
                     to re-verify from the diff. In particular: behavior they record as \
                     existing, documented or intentional was NOT introduced by this change, \
                     and impact they already assessed is not yours to re-litigate. Use them \
                     so you do not reconstruct the system from hunks alone.{brief}"
        )
    });
    let log = (jira.is_some() || run.is_some()).then(|| {
        format!(
            "review_run: reviewers given prior run context ({}{}{} chars)",
            if jira.is_some() { "ticket + " } else { "" },
            if run.is_some() { "step briefs, " } else { "" },
            jira.as_ref().map_or(0, |s| s.len()) + run.as_ref().map_or(0, |s| s.len()),
        )
    });
    (jira, run, log)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_label_names_reviewers_providers_and_checks() {
        let checks = vec![("go".to_string(), "go test".to_string())];
        let provs = vec!["claude".to_string()];
        let lenses = vec!["security".to_string()];
        assert_eq!(label_tail(&[], &[], &[], &[]), "");
        assert_eq!(
            label_tail(&[], &lenses, &provs, &checks),
            " · lenses: security · providers: claude · 1 check(s) delegated to the reviewer"
        );
        // Declared reviewers win over the lens list; lens > skill > name.
        let reviewers = [
            json!({"lens": "perf", "name": "x"}),
            json!({"skill": "tests"}),
            json!({}),
        ];
        assert_eq!(
            label_tail(&reviewers, &lenses, &[], &[]),
            " · lenses: perf, tests"
        );
    }

    #[test]
    fn reviewers_get_the_step_briefs_capped_unless_opted_out() {
        let dir = tempfile::tempdir().unwrap();
        let files = RunContextFiles::create(dir.path(), "run-1");
        assert_eq!(prior_run_context(&files, &json!({})), (None, None, None));
        let run_dir = dir.path().join("workflow-context/run-1");
        std::fs::write(run_dir.join("step1-plan.md"), "the plan").unwrap();
        std::fs::write(run_dir.join("step2-big.md"), "x".repeat(REVIEW_CONTEXT_CAP)).unwrap();
        let (jira, run, log) = prior_run_context(&files, &json!({}));
        let run = run.expect("briefs given");
        assert!(jira.is_none());
        assert!(run.contains("### step1-plan.md\n\nthe plan"), "{run}");
        assert!(
            run.contains("context cap reached") && !run.contains("step2-big"),
            "{run}"
        );
        assert!(log.unwrap().contains("step briefs"));
        let off = prior_run_context(&files, &json!({"include_context": false}));
        assert_eq!(off, (None, None, None));
    }
}
