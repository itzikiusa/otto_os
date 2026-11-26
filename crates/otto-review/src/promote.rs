//! Promoting a summarized draft comment into a tracked workflow Finding (the
//! board and Proof Pack read these): the stable fingerprint, the line span
//! and the `created` audit event that links the originating comment.

use otto_core::Id;

/// Where a finding lands: `(review_id, workspace_id, repo_id, pr_number,
/// repo_path)` — the checkout is read for the flagged line's text.
pub type FindingAt<'a> = (&'a Id, &'a str, &'a Id, u64, &'a str);

/// Promote one draft comment into a tracked workflow Finding (created →
/// audit event linking the comment). Returns its fingerprint.
pub async fn promote_finding(
    (findings, events): (
        &otto_state::ReviewFindingsRepo,
        &otto_state::FindingEventsRepo,
    ),
    (review_id, workspace_id, repo_id, pr_number, repo_path): FindingAt<'_>,
    c: &crate::engine::DraftComment,
    comment_id: &Id,
    seen_fingerprints: &[String],
    anchor_files: &mut std::collections::HashMap<String, Option<Vec<String>>>,
) -> String {
    // PR #0 is the local-review sentinel → dedup within the review, not across PRs.
    let pr_opt = if pr_number == 0 {
        None
    } else {
        Some(pr_number)
    };
    // Derive the enriched fields from the comment when the summarizer didn't
    // emit them (back-compat; §15.9).
    let title = c
        .title
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| c.body.lines().next().unwrap_or("").trim().to_string());
    let evidence = c
        .evidence
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| c.body.clone());
    let reasoning = c.reasoning.clone().unwrap_or_default();
    // Stable (v2) fingerprint anchored on the flagged code line's text (or
    // the title), NOT the summarizer's re-worded body — see
    // `compute_finding_fingerprint`. The legacy body hash rides along so
    // rows stored before v2 are recognized and re-keyed, not duplicated.
    let legacy_fp = otto_state::review_findings::compute_fingerprint(
        repo_id,
        pr_number,
        c.path.as_deref(),
        c.category.as_deref(),
        &c.body,
    );
    let line_text = match (c.path.as_deref(), c.line) {
        (Some(p), Some(l)) => {
            crate::engine::anchored_line_text(repo_path, p, l, anchor_files).await
        }
        _ => None,
    };
    let anchor = otto_state::review_findings::finding_anchor(line_text.as_deref(), &title, &c.body);
    let mut fp = otto_state::review_findings::compute_finding_fingerprint(
        repo_id,
        pr_number,
        c.path.as_deref(),
        c.category.as_deref(),
        &anchor,
    );
    if seen_fingerprints.contains(&fp) {
        // A second, distinct finding on the same line/category in this run:
        // disambiguate by title so it is not folded into the first one.
        let titled = format!(
            "{anchor}|{}",
            otto_state::review_findings::normalize_anchor_text(&title)
        );
        fp = otto_state::review_findings::compute_finding_fingerprint(
            repo_id,
            pr_number,
            c.path.as_deref(),
            c.category.as_deref(),
            &titled,
        );
    }
    // Anchor the finding to a sane (line, line_end) span — an end without a
    // start, or an inverted/degenerate range, collapses appropriately.
    let (line, line_end) = otto_core::finding::normalize_line_range(c.line, c.line_end);
    let nf = otto_state::NewFinding {
        review_id,
        workspace_id,
        repo_id,
        pr_number: pr_opt,
        path: c.path.as_deref(),
        line: line.map(|l| l as i64),
        line_end: line_end.map(|l| l as i64),
        severity: &c.severity,
        category: c.category.as_deref(),
        title: &title,
        body: &c.body,
        evidence: &evidence,
        agent_reasoning_summary: &reasoning,
        suggested_fix: c.suggested_fix.as_deref(),
        produced_by_agent: Some("review"),
        reviewer: "review",
        fingerprint: &fp,
        run_id: review_id,
    };
    match findings.upsert_tracked(&nf, Some(&legacy_fp)).await {
        Ok((f, created)) => {
            if created {
                // Anchor the audit trail + link the originating comment id.
                let _ = events
                    .append(
                        &f.id,
                        &f.workspace_id,
                        "created",
                        "review",
                        None,
                        Some("open"),
                        serde_json::json!({ "comment_id": comment_id }),
                    )
                    .await;
            }
        }
        Err(e) => tracing::warn!(review = %review_id, "persist finding failed: {e}"),
    }
    fp
}
