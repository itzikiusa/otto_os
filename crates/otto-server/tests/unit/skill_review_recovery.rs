use super::*;

#[test]
fn partial_skill_findings_are_preserved_until_clean_result_is_complete() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("findings.json");
    std::fs::write(&path, "[").unwrap();
    assert!(read_skill_findings(&path).is_none());
    assert!(path.exists());
    std::fs::write(&path, "[]").unwrap();
    assert!(read_skill_findings(&path).unwrap().is_empty());
    assert!(!path.exists());
}

#[test]
fn clean_skill_transcript_is_a_complete_verdict() {
    assert!(completed_skill_findings("[]").unwrap().is_empty());
    assert!(completed_skill_findings("[broken]").is_none());
}

#[test]
fn skill_review_attempt_paths_are_unique() {
    assert_ne!(findings_path("review", 1), findings_path("review", 1));
}

#[test]
fn incomplete_fixer_output_is_not_consumed_as_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("fix.json");
    for text in ["", "{", "{}"] {
        std::fs::write(&path, text).unwrap();
        assert!(read_fix_note(&path).is_none());
        assert!(path.exists());
    }
    std::fs::write(
        &path,
        r#"{"applied":[],"skipped":[],"notes":"nothing needed"}"#,
    )
    .unwrap();
    assert!(read_fix_note(&path).unwrap().contains("nothing needed"));
}

#[tokio::test]
async fn retried_findings_replace_the_summary_used_by_apply() {
    use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
    let dir = tempfile::tempdir().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, "skill-retry-ws").await;
    let ctx = test_ctx(&pool, dir.path().to_path_buf()).await;
    let review = ctx
        .skill_reviews_store
        .create(
            &"skill-retry-ws".into(),
            "fixture",
            "library",
            "agents",
            "",
            None,
        )
        .await
        .unwrap();
    let report = SkillStaticReport {
        verdict: "Ready".into(),
        average_score: 5.0,
        scorecard: vec![],
        findings: vec![],
    };
    ctx.skill_reviews_store
        .set_static(&review.id, &report)
        .await
        .unwrap();
    let finding = |title: &str| SkillFinding {
        severity: "High".into(),
        code: "FIX".into(),
        title: title.into(),
        evidence: "SKILL.md".into(),
        why: "reason".into(),
        fix: title.into(),
    };
    ctx.skill_reviews_store
        .set_summary(
            &review.id,
            &merge_summary(&report, &[finding("old finding")]),
        )
        .await
        .unwrap();
    ctx.skill_reviews_store
        .set_agents(
            &review.id,
            &[SkillReviewAgent {
                name: "reviewer".into(),
                provider: "fixture".into(),
                model: String::new(),
                status: "done".into(),
                note: String::new(),
                session_id: None,
                findings: vec![finding("new finding")],
            }],
        )
        .await
        .unwrap();
    refresh_after_retry(&ctx, &review.id).await.unwrap();
    let after = ctx.skill_reviews_store.get(&review.id).await.unwrap();
    let summary = after.summary.unwrap();
    assert_eq!(summary.findings.len(), 1);
    assert_eq!(summary.findings[0].title, "new finding");
    assert_eq!(summary.patch_plan, vec!["new finding"]);
    assert_eq!(after.status, "done");
}
