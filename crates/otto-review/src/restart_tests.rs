use super::*;
use crate::verification::INCOMPLETE_PREFIX;

/// A review whose reviewer rows have `statuses` (each with one finding when
/// `findings`), plus a summarizer row, and a scope plan with `resumes`
/// restart notes (`None` ⇒ no plan).
fn review(status: &str, statuses: &[&str], findings: bool, resumes: Option<usize>) -> Review {
    let mut agents: Vec<serde_json::Value> = statuses
        .iter()
        .map(|s| {
            let note = if *s == "error" {
                "timed out with no result"
            } else {
                ""
            };
            let stored = if findings {
                serde_json::json!([{"severity": "bug", "body": "auth.rs:2 bypass"}])
            } else {
                serde_json::json!([])
            };
            serde_json::json!({"name": format!("lens-{s}"), "provider": "claude", "model": "",
                "status": s, "note": note, "comment_count": 0, "findings": stored})
        })
        .collect();
    agents.push(
        serde_json::json!({"name": "Summarizer", "provider": "claude", "model": "",
        "status": "running", "note": "", "comment_count": 0}),
    );
    let mut r: Review = serde_json::from_value(serde_json::json!({
        "id": "rv", "repo_id": "repo", "pr_number": 0, "status": status,
        "agents": agents, "comments": [], "created_at": "2026-11-12T00:00:00Z"
    }))
    .expect("review");
    r.scope_plan = resumes.map(|n| {
        let mut p = crate::scope::documentation_plan("--- a/docs/a.md\n+++ b/docs/a.md\n+x", "h")
            .expect("plan");
        p.verification_notes = (0..n).map(|i| format!("{RESUME_NOTE} ({i})")).collect();
        p
    });
    r
}

#[test]
fn a_restart_resumes_only_a_review_whose_reviewers_all_finished() {
    let r = review("running", &["done", "skipped"], true, Some(0));
    assert_eq!(on_restart(&r), RestartAction::ResumeSummarizer);
    // A reviewer still running, failed, or partial: re-run from scratch.
    for s in ["running", "pending", "error"] {
        let r = review("running", &["done", s], true, Some(0));
        assert_eq!(on_restart(&r), RestartAction::Fail(INTERRUPTED), "{s}");
    }
    let mut partial = review("running", &["done"], true, Some(0));
    partial.agents[0].note = "partial — 1 lens missing".into();
    assert_eq!(on_restart(&partial), RestartAction::Fail(INTERRUPTED));
    // Still on the scope step (one row): nothing to resume.
    let mut scoping = review("running", &[], true, Some(0));
    scoping.agents.truncate(1);
    assert_eq!(on_restart(&scoping), RestartAction::Fail(INTERRUPTED));
    // Reviewers finished with nothing stored: nothing to summarize.
    let empty = review("running", &["done"], false, Some(0));
    assert_eq!(on_restart(&empty), RestartAction::Fail(INTERRUPTED));
}

#[test]
fn a_crash_loop_stops_after_two_resumes() {
    let r = review("running", &["done"], true, Some(MAX_RESTART_RESUMES - 1));
    assert_eq!(on_restart(&r), RestartAction::ResumeSummarizer);
    let r = review("running", &["done"], true, Some(MAX_RESTART_RESUMES));
    assert_eq!(on_restart(&r), RestartAction::Fail(RESUMES_EXHAUSTED));
    // No plan to count in: never resumed (it could loop for ever).
    let r = review("running", &["done"], true, None);
    assert_eq!(on_restart(&r), RestartAction::Fail(RESUMES_EXHAUSTED));
    assert!(RESUMES_EXHAUSTED.contains(&MAX_RESTART_RESUMES.to_string()));
}

#[test]
fn a_step_retry_reruns_only_the_summarizer_when_only_it_failed() {
    let verifier = format!("invalid: {INCOMPLETE_PREFIX}the final verifier agent failed (timed out); 4 reviewer findings are stored unverified. Retry the summarizer to verify them.");
    let mut r = review("error", &["done", "done"], true, Some(0));
    for error in [
        verifier.as_str(),
        crate::engine::REVIEW_STALLED_MSG,
        INTERRUPTED_SUMMARIZING,
    ] {
        r.error = Some(error.into());
        assert_eq!(adoption(&r), Adoption::RerunSummarizer, "{error}");
    }
    // Not the summarizer's failure, or a crash loop: a new review.
    for error in [
        format!("invalid: {INCOMPLETE_PREFIX}the reviewed change is not fully covered (partial diff (files omitted)); …"),
        RESUMES_EXHAUSTED.to_string(),
        INTERRUPTED.to_string(),
        "internal: database is locked".to_string(),
    ] {
        r.error = Some(error.clone());
        assert_eq!(adoption(&r), Adoption::Launch, "{error}");
    }
    // A failed reviewer: the summarizer alone cannot finish it.
    let mut failed = review("error", &["done", "error"], true, Some(0));
    failed.error = Some(verifier.clone());
    assert_eq!(adoption(&failed), Adoption::Launch);
    let mut cancelled = review("cancelled", &["done"], true, Some(0));
    cancelled.error = Some(verifier);
    assert_eq!(adoption(&cancelled), Adoption::Launch);
}

/// The adoption rule the step used before (now [`adoption`]).
#[test]
fn running_and_complete_reviews_are_adopted_as_they_are() {
    assert_eq!(
        adoption(&review("running", &["running"], false, None)),
        Adoption::Adopt
    );
    let mut done = review("done", &["done"], true, None);
    done.agents[1].status = "done".into();
    assert_eq!(adoption(&done), Adoption::Adopt);
    done.summary_fallback = Some(true);
    assert_eq!(adoption(&done), Adoption::Launch, "summary fell back");
    assert_eq!(
        adoption(&review("done", &["error"], true, None)),
        Adoption::Launch
    );
}

#[test]
fn rerun_block_names_why_the_summarizer_cannot_rerun() {
    assert!(rerun_block(&review("running", &["done"], true, None))
        .is_some_and(|w| w.contains("still running")));
    let mut lone = review("error", &[], true, None);
    lone.agents.truncate(1);
    assert!(rerun_block(&lone).is_some_and(|w| w.contains("no summarizer")));
    assert!(rerun_block(&review("error", &["done"], false, None))
        .is_some_and(|w| w.contains("re-run the review")));
    assert_eq!(rerun_block(&review("error", &["done"], true, None)), None);
}

#[test]
fn an_unfinished_reviewer_is_named_with_why_it_failed() {
    let r = review("error", &["done", "error"], true, None);
    let rows = with_failure_reasons(&r.agents[..2]);
    assert_eq!(rows[0].name, "lens-done");
    assert_eq!(rows[1].name, "lens-error (timed out with no result)");
    let missing = crate::verification::missing_lenses(&rows);
    assert_eq!(missing, vec!["lens-error (timed out with no result)"]);
}

#[tokio::test]
async fn boot_settles_running_reviews_and_returns_the_ones_to_resume() {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::db::open(&dir.path().join("state.sqlite"))
        .await
        .unwrap();
    let reviews = otto_state::ReviewsRepo::new(pool);
    let repo: Id = "repo".into();
    let make = |template: Review| {
        let reviews = reviews.clone();
        let repo = repo.clone();
        async move {
            let row = reviews.create_review(&repo, 0).await.unwrap();
            reviews.set_agents(&row.id, &template.agents).await.unwrap();
            if let Some(plan) = &template.scope_plan {
                reviews.set_scope_plan(&row.id, plan).await.unwrap();
            }
            row.id
        }
    };
    let resumable = make(review("running", &["done"], true, Some(0))).await;
    let unfinished = make(review("running", &["running"], true, Some(0))).await;
    let looping = make(review("running", &["done"], true, Some(2))).await;
    let resume = settle_after_restart(&reviews).await.unwrap();
    assert_eq!(resume, vec![resumable.clone()]);
    let after = |id: Id| {
        let reviews = reviews.clone();
        async move { reviews.get_review(&id).await.unwrap() }
    };
    let r = after(resumable).await;
    assert_eq!(
        r.status,
        ReviewStatus::Error,
        "settled so the rerun can claim it"
    );
    assert_eq!(r.error.as_deref(), Some(INTERRUPTED_SUMMARIZING));
    assert_eq!(restart_resumes(r.scope_plan.as_ref().unwrap()), 1);
    assert_eq!(adoption(&r), Adoption::RerunSummarizer);
    let u = after(unfinished).await;
    assert_eq!(u.error.as_deref(), Some(INTERRUPTED));
    assert_eq!(u.agents[0].status, "error", "orphaned live row settled");
    assert_eq!(
        after(looping).await.error.as_deref(),
        Some(RESUMES_EXHAUSTED)
    );
    assert!(settle_after_restart(&reviews).await.unwrap().is_empty());
}
