use super::*;
use serde_json::{json, Value};

fn review(status: &str, error: Option<&str>, comments: Value) -> Review {
    serde_json::from_value(json!({
        "id": "r2", "repo_id": "repo", "pr_number": 7, "status": status, "error": error,
        "comments": comments, "agents": [], "created_at": "2026-11-12T00:00:00Z",
    }))
    .unwrap()
}

fn comment(id: &str, review: &str, line: u32, body: &str) -> Value {
    json!({"id": id, "review_id": review, "path": "src/a.rs", "line": line, "severity": "bug",
        "body": body, "state": "draft", "posted": false, "created_at": "2026-11-12T00:00:00Z"})
}

fn comments(v: Value) -> Vec<ReviewComment> {
    serde_json::from_value(v).unwrap()
}

fn finding(id: &str, review: &str, line: u32, sev: &str, status: &str, title: &str) -> Finding {
    serde_json::from_value(json!({
        "id": id, "review_id": review, "workspace_id": "w", "repo_id": "repo", "pr_number": 7,
        "fingerprint": format!("fp-{id}"), "severity": sev, "path": "./src/a.rs", "line": line,
        "title": title, "body": title, "evidence": "", "agent_reasoning_summary": "",
        "status": status, "reviewer": "review", "state": "open", "regressed": false,
        "requires_human_approval": false, "occurrence_count": 1,
        "created_at": "2026-11-12T00:00:00Z", "updated_at": "2026-11-12T00:00:00Z",
    }))
    .unwrap()
}

#[test]
fn verdict_reads_blockers_incomplete_and_unverified() {
    let done = review(
        "done",
        None,
        json!([comment("c1", "r2", 1, "[unverified] maybe a race")]),
    );
    let open_high = finding("f1", "r2", 1, "high", "open", "SQL injection");
    let dismissed = finding("f2", "r2", 2, "critical", "false_positive", "Nope");
    let low = finding("f3", "r2", 3, "low", "accepted", "Naming");
    let v = verdict(&done, &[open_high.clone(), dismissed.clone(), low.clone()]);
    assert_eq!(v.verdict.as_deref(), Some("changes_requested"));
    assert_eq!(
        v.blocker_count,
        Some(1),
        "a dismissed critical is not a blocker"
    );
    assert_eq!(
        v.summary.as_deref(),
        Some("2 open findings: 1 high, 1 low · 1 unverified")
    );

    let clean = verdict(&done, &[dismissed.clone(), low]);
    assert_eq!(clean.verdict.as_deref(), Some("approved"));
    assert_eq!(clean.blocker_count, Some(0));

    // An incomplete review is never approved, even with nothing open.
    let incomplete = review(
        "error",
        Some("invalid: Review incomplete — not a verdict: the final verifier agent failed (x)"),
        json!([]),
    );
    let v = verdict(&incomplete, &[dismissed]);
    assert_eq!(v.verdict.as_deref(), Some("incomplete"));
    assert_eq!(v.summary.as_deref(), Some("No open findings"));

    // Running / plain failure: no verdict at all.
    assert_eq!(
        verdict(
            &review("running", None, json!([])),
            std::slice::from_ref(&open_high)
        ),
        Verdict::default()
    );
    assert_eq!(
        verdict(&review("error", Some("boom"), json!([])), &[open_high]),
        Verdict::default()
    );
}

#[test]
fn comments_link_by_audit_trail_then_anchor_and_text() {
    let cs = comments(json!([
        comment("c1", "r2", 10, "SQL injection: format! into a query"),
        comment("c2", "r2", 10, "[unverified] Unbounded loop on retry"),
        comment("c3", "r2", 20, "Off by one"),
        comment("c4", "r2", 30, "No finding here"),
    ]));
    let findings = vec![
        // Two findings on one line: text decides, whatever the order.
        finding(
            "f-loop",
            "r2",
            10,
            "medium",
            "open",
            "Unbounded loop on retry",
        ),
        finding("f-sql", "r2", 10, "high", "open", "SQL injection"),
        // Same anchor, but last seen by ANOTHER review: not this comment's.
        finding("f-other", "r1", 30, "high", "open", "No finding here"),
        // Moved line, but the audit trail names the comment.
        finding("f-moved", "r2", 99, "low", "open", "Off by one"),
    ];
    let created = HashMap::from([("f-moved".to_string(), "c3".to_string())]);
    let links = link_comments(&cs, &findings, &created);
    assert_eq!(links.get("c1").map(String::as_str), Some("f-sql"));
    assert_eq!(links.get("c2").map(String::as_str), Some("f-loop"));
    assert_eq!(links.get("c3").map(String::as_str), Some("f-moved"));
    assert_eq!(links.get("c4"), None);
}

#[test]
fn dismissal_and_follow_rules() {
    assert_eq!(dismissal(None).unwrap(), FindingStatus::FalsePositive);
    assert_eq!(dismissal(Some("waived")).unwrap(), FindingStatus::Waived);
    assert!(dismissal(Some("later")).is_err());
    use FindingStatus::*;
    assert!(follows(Open, FalsePositive));
    assert!(follows(Accepted, Waived));
    assert!(!follows(Verified, FalsePositive), "a verified fix stays");
    assert!(!follows(Waived, FalsePositive), "already dismissed");
    assert!(follows(FalsePositive, Open));
    assert!(
        !follows(Accepted, Open),
        "restore only reopens a dismissed finding"
    );
}

#[test]
fn a_redrafted_comment_already_posted_by_an_earlier_review_is_flagged() {
    let current = comments(json!([
        comment("n1", "r2", 10, "Reworded: SQL injection here"),
        comment("n2", "r2", 20, "[unverified] Off by one"),
        comment("n3", "r2", 30, "Brand new"),
    ]));
    let mut earlier = comments(json!([
        comment("o1", "r1", 12, "SQL injection"),
        comment("o2", "r1", 20, "Off  by one!"),
        comment("o3", "r1", 30, "Brand new"),
    ]));
    earlier[0].posted = true;
    earlier[1].posted = true;
    // o3 was never posted: n3 is not on the PR.
    let links = HashMap::from([("n1".to_string(), "f-sql".to_string())]);
    let posted: Vec<(ReviewComment, Option<Id>)> = vec![
        (earlier[0].clone(), Some("f-sql".into())),
        (earlier[1].clone(), None),
        (earlier[2].clone(), None),
    ];
    let copies = posted_copies(&current, &links, &posted);
    assert_eq!(copies["n1"].comment_id, "o1", "same tracked finding");
    assert_eq!(copies["n2"].comment_id, "o2", "same path + line + text");
    assert_eq!(copies["n2"].review_id, "r1");
    assert!(!copies.contains_key("n3"));
}

/// Declining a comment dismisses its finding; dismissing a finding declines
/// its other draft; restoring the comment reopens the finding.
#[tokio::test]
async fn decline_and_dismiss_move_together() {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::db::open(&dir.path().join("state.sqlite"))
        .await
        .unwrap();
    let user = otto_state::UsersRepo::new(pool.clone())
        .create("reviewer", "hash", "Reviewer", true)
        .await
        .unwrap();
    let workspace = otto_state::WorkspacesRepo::new(pool.clone())
        .create("Review", "/tmp", &user.id)
        .await
        .unwrap();
    let repo = otto_state::GitStore::new(pool.clone())
        .create_repo(otto_state::NewRepo {
            workspace_id: workspace.id.clone(),
            name: "Repo".into(),
            path: "/tmp/triage-fixture".into(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let reviews = otto_state::ReviewsRepo::new(pool.clone());
    let findings = otto_state::ReviewFindingsRepo::new(pool.clone());
    let events = otto_state::FindingEventsRepo::new(pool);
    let run = reviews.create_review(&repo.id, 7).await.unwrap();
    let sev = otto_core::domain::CommentSeverity::Bug;
    let a = reviews
        .add_comment(&run.id, Some("a.rs"), Some(3), sev, "Null deref")
        .await
        .unwrap();
    let b = reviews
        .add_comment(&run.id, Some("b.rs"), Some(9), sev, "Leaked handle")
        .await
        .unwrap();
    for (c, fp) in [(&a, "fp-a"), (&b, "fp-b")] {
        let nf = otto_state::NewFinding {
            review_id: &run.id,
            workspace_id: &workspace.id,
            repo_id: &repo.id,
            pr_number: Some(7),
            path: c.path.as_deref(),
            line: c.line.map(i64::from),
            line_end: None,
            severity: "bug",
            category: None,
            title: &c.body,
            body: &c.body,
            evidence: "",
            agent_reasoning_summary: "",
            suggested_fix: None,
            produced_by_agent: Some("review"),
            reviewer: "review",
            fingerprint: fp,
            run_id: &run.id,
        };
        findings.upsert(&nf).await.unwrap();
    }
    let run = reviews.get_review(&run.id).await.unwrap();
    let (bus, mut rx) = broadcast::channel(16);
    let l = Links {
        reviews: &reviews,
        findings: &findings,
        events: &events,
        bus: &bus,
        actor: "u".into(),
    };
    // Decline A as "Won't fix" → its finding is waived, and the board is told.
    assert!(follow_comment(&l, &run, &a.id, FindingStatus::Waived, None).await);
    assert_eq!(
        linked(&findings, &events, &run, &a.id).await.status,
        FindingStatus::Waived
    );
    assert!(matches!(rx.try_recv(), Ok(Event::FindingUpdated { .. })));
    // Declining again changes nothing (already dismissed).
    assert!(!follow_comment(&l, &run, &a.id, FindingStatus::FalsePositive, None).await);
    // Restore A → the finding reopens.
    assert!(follow_comment(&l, &run, &a.id, FindingStatus::Open, None).await);
    assert_eq!(
        linked(&findings, &events, &run, &a.id).await.status,
        FindingStatus::Open
    );

    // Mark B's finding a false positive → B's draft is declined.
    let fb = linked(&findings, &events, &run, &b.id).await;
    let fb = findings
        .set_status(&fb.id, FindingStatus::FalsePositive, "u")
        .await
        .unwrap();
    assert!(follow_finding(&l, &fb).await);
    let after = reviews.get_review(&run.id).await.unwrap();
    let state = |id: &str| after.comments.iter().find(|c| c.id == id).unwrap().state;
    assert_eq!(state(&b.id), CommentState::Declined);
    assert_eq!(state(&a.id), CommentState::Draft, "only the linked comment");

    // The triage view carries the link and the dismissed status.
    let t = triage(&l, &after).await.unwrap();
    let tb = t.comments.iter().find(|c| c.comment_id == b.id).unwrap();
    assert_eq!(tb.finding_status, Some(FindingStatus::FalsePositive));
    assert!(t.prior.is_empty());
}

async fn linked(
    findings: &ReviewFindingsRepo,
    events: &FindingEventsRepo,
    run: &Review,
    comment_id: &str,
) -> Finding {
    let (seen, links) = review_links(findings, events, run).await.unwrap();
    seen.into_iter()
        .find(|f| Some(&f.id) == links.get(comment_id))
        .unwrap()
}
