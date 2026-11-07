//! Stable finding lineage for reruns, independent of server routes.
use crate::engine::ReviewBranches;
use otto_core::domain::ReviewStatus;
use otto_core::{Error, Id, Result};

/// Prior issues are verifier inputs even when every fresh lens overlooks them.
/// Rebuild from the persisted scope on retry, so restart cannot erase coverage.
pub async fn prior_claims(
    plan: &otto_core::domain::ReviewScopePlan,
    findings: &otto_state::ReviewFindingsRepo,
) -> Result<Vec<otto_core::domain::ReviewFinding>> {
    let mut claims = Vec::new();
    for prior in &plan.prior_findings {
        let original = findings.get_full(&prior.finding_id).await?;
        claims.push(otto_core::domain::ReviewFinding {
            path: original.path,
            line: original.line,
            line_end: original.line_end,
            severity: original.severity.as_str().into(),
            body: format!("Prior finding {}: {}\nEarlier scope disposition (re-verify independently): {} — {}\nOriginal evidence: {}", prior.finding_id, original.body, prior.disposition, prior.evidence, original.evidence),
            lens: Some("prior-finding-verification".into()),
        });
    }
    Ok(claims)
}

pub async fn prior_review_context(
    reviews: &otto_state::ReviewsRepo,
    findings_store: &otto_state::ReviewFindingsRepo,
    repo_id: &Id,
    pr: u64,
    current: &Id,
    branches: Option<&ReviewBranches>,
    explicit: Option<&Id>,
) -> Result<(String, Vec<String>)> {
    let previous = if let Some(id) = explicit {
        let prior = reviews.get_review(id).await?;
        if prior.repo_id != *repo_id || prior.pr_number != pr || prior.id == *current {
            return Err(Error::Invalid(
                "Previous review must belong to the same repository and PR".into(),
            ));
        }
        vec![prior]
    } else {
        reviews.list_for_pr(repo_id, pr as i64).await?
    };
    for review in previous {
        if review.id == *current || review.status != ReviewStatus::Done {
            continue;
        }
        if pr == 0 && explicit.is_none() {
            let recorded = reviews.get_run_context(&review.id).await?;
            let branch = branches
                .map(|b| b.source.as_str())
                .filter(|s| !s.is_empty());
            if branch.is_none()
                || recorded.as_ref().and_then(|c| c.source_branch.as_deref()) != branch
            {
                continue;
            }
        }
        let mut findings = findings_store.list_seen_for_review(&review.id).await?;
        if let Some(plan) = &review.scope_plan {
            for prior in &plan.prior_findings {
                if !findings.iter().any(|f| f.id == prior.finding_id) {
                    findings.push(findings_store.get_full(&prior.finding_id).await?);
                }
            }
        }
        let ids = findings.iter().map(|f| f.id.clone()).collect();
        return Ok((format!("\n## Prior review {} — verify before fresh review\nFor EACH prior finding, cite its ID and record fixed + concrete current-code/check evidence, still_open + trigger/evidence, or not_relevant + causal-scope reason. Missing from fresh output is NOT proof of a fix. Do not inherit earlier claims without rechecking. Then perform an additional fresh review of the current causal diff.\n{}\n",
            review.id, serde_json::to_string(&findings).map_err(|e| Error::Internal(e.to_string()))?), ids));
    }
    Ok((String::new(), vec![]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn three_run_lineage_keeps_an_inherited_issue_without_a_fresh_comment() {
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
                path: "/tmp/review-fixture".into(),
                remote_url: None,
                provider: None,
                git_account_id: None,
            })
            .await
            .unwrap();
        let reviews = otto_state::ReviewsRepo::new(pool.clone());
        let findings = otto_state::ReviewFindingsRepo::new(pool);
        let first = reviews.create_review(&repo.id, 7).await.unwrap();
        let mut original = otto_state::NewFinding {
            review_id: &first.id,
            workspace_id: &workspace.id,
            repo_id: &repo.id,
            pr_number: Some(7),
            path: Some("auth.rs"),
            line: Some(2),
            line_end: None,
            severity: "bug",
            category: Some("security"),
            title: "Permission bypass",
            body: "Anonymous access reaches protected data",
            evidence: "auth.rs:2 → api.rs:8",
            agent_reasoning_summary: "Reachable caller",
            suggested_fix: None,
            produced_by_agent: Some("review"),
            reviewer: "review",
            fingerprint: "stable",
            run_id: &first.id,
        };
        let (finding, _) = findings.upsert(&original).await.unwrap();
        reviews
            .set_status(&first.id, ReviewStatus::Done, None)
            .await
            .unwrap();
        let second = reviews.create_review(&repo.id, 7).await.unwrap();
        let mut plan =
            crate::scope::documentation_plan("--- a/docs/a.md\n+++ b/docs/a.md\n+change", "head")
                .unwrap();
        plan.prior_findings
            .push(otto_core::domain::ReviewFindingDisposition {
                finding_id: finding.id.clone(),
                disposition: "still_open".into(),
                evidence: "auth.rs:2 still bypasses the check".into(),
            });
        reviews.set_scope_plan(&second.id, &plan).await.unwrap();
        reviews
            .set_status(&second.id, ReviewStatus::Done, None)
            .await
            .unwrap();
        let third = reviews.create_review(&repo.id, 7).await.unwrap();
        assert!(findings
            .list_full_for_review(&second.id)
            .await
            .unwrap()
            .is_empty());
        let (context, ids) = prior_review_context(
            &reviews,
            &findings,
            &repo.id,
            7,
            &third.id,
            None,
            Some(&second.id),
        )
        .await
        .unwrap();
        assert_eq!(ids, vec![finding.id.clone()]);
        assert!(context.contains("Anonymous access reaches protected data"));
        let injected = prior_claims(&plan, &findings).await.unwrap();
        assert_eq!(injected.len(), 1);
        assert!(injected[0].body.contains(&finding.id));
        assert_eq!(injected[0].severity, "high");
        original.review_id = &third.id;
        findings.upsert(&original).await.unwrap();
        let (_, original_ids) = prior_review_context(
            &reviews,
            &findings,
            &repo.id,
            7,
            &third.id,
            None,
            Some(&first.id),
        )
        .await
        .unwrap();
        assert_eq!(original_ids, vec![finding.id]);
    }
}
