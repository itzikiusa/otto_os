//! Stable finding lineage for reruns, independent of server routes.
use crate::engine::ReviewBranches;
use otto_core::domain::ReviewStatus;
use otto_core::{Error, Id, Result};

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
