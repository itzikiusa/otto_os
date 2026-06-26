//! Proof Pack endpoints — the assembled evidence bundle for a review, the
//! literal "closing the loop with evidence" artifact.
//!
//! - `GET  /reviews/{review_id}/proof-pack`         — assemble live (JSON)
//! - `POST /reviews/{review_id}/proof-pack/export`  — render markdown, persist a
//!   snapshot, and ingest VERIFIED findings into the memory store (durable recall)
//!
//! Namespaced `ReviewProofPack*` to avoid colliding with a parallel
//! `feat/proof-packs` branch. Both routes ride the existing `/reviews/` RBAC rule
//! (GET=View, POST=Edit).

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use otto_core::event::Event;
use otto_core::finding::{
    Finding, FindingStatus, ReviewProofPack, ReviewProofPackEntry, ReviewProofPackExport,
    ReviewProofPackSummary,
};
use otto_core::domain::WorkspaceRole;
use otto_state::memory::{NewMemory, Scope};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::routes::findings::actor_label;
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/reviews/{review_id}/proof-pack", get(get_proof_pack))
        .route("/reviews/{review_id}/proof-pack/export", post(export_proof_pack))
}

/// Resolve a review's workspace id (for the role check) via its repo.
async fn review_workspace(ctx: &ServerCtx, review_id: &str) -> ApiResult<String> {
    let review = ctx
        .reviews_store
        .get_review(&review_id.to_string())
        .await
        .map_err(ApiError)?;
    let repo = ctx
        .git_store
        .get_repo(&review.repo_id)
        .await
        .map_err(ApiError)?;
    Ok(repo.workspace_id)
}

/// Assemble the live Proof Pack: every finding with its full event timeline +
/// artifacts, summary counts, and the repo rules generated for the workspace.
pub async fn assemble(ctx: &ServerCtx, review_id: &str, workspace_id: &str) -> ApiResult<ReviewProofPack> {
    let findings = ctx
        .findings_store
        .list_full_for_review(review_id)
        .await
        .map_err(ApiError)?;
    let mut summary = ReviewProofPackSummary::default();
    let mut entries: Vec<ReviewProofPackEntry> = Vec::with_capacity(findings.len());
    for f in findings {
        summary.total += 1;
        *summary.by_status.entry(f.status.as_str().to_string()).or_insert(0) += 1;
        *summary.by_severity.entry(f.severity.as_str().to_string()).or_insert(0) += 1;
        match f.status {
            FindingStatus::Verified => summary.verified += 1,
            FindingStatus::Fixed => summary.fixed += 1,
            FindingStatus::Open => summary.open += 1,
            _ => {}
        }
        if f.linked_commit.as_deref().filter(|s| !s.is_empty()).is_some() {
            summary.with_commit += 1;
        }
        if f.linked_test.as_deref().filter(|s| !s.is_empty()).is_some() {
            summary.with_test += 1;
        }
        let events = ctx
            .finding_events_store
            .list_for_finding(&f.id)
            .await
            .unwrap_or_default();
        entries.push(ReviewProofPackEntry { finding: f, events });
    }
    let repo_rules = ctx
        .repo_rules_store
        .list(workspace_id)
        .await
        .unwrap_or_default();
    Ok(ReviewProofPack {
        review_id: review_id.to_string(),
        workspace_id: workspace_id.to_string(),
        generated_at: chrono::Utc::now().to_rfc3339(),
        summary,
        findings: entries,
        repo_rules,
    })
}

/// `GET /reviews/{review_id}/proof-pack`
async fn get_proof_pack(
    Path(review_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<ReviewProofPack>> {
    let ws = review_workspace(&ctx, &review_id).await?;
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Viewer).await?;
    Ok(Json(assemble(&ctx, &review_id, &ws).await?))
}

#[derive(Deserialize, Default)]
struct ExportReq {
    #[serde(default)]
    format: Option<String>,
}

/// `POST /reviews/{review_id}/proof-pack/export` — persist a snapshot + ingest
/// verified findings into memory for durable recall.
async fn export_proof_pack(
    Path(review_id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<ExportReq>>,
) -> ApiResult<Json<ReviewProofPackExport>> {
    let ws = review_workspace(&ctx, &review_id).await?;
    require_ws_role(&ctx, &user, &ws, WorkspaceRole::Editor).await?;
    let _format = body.map(|b| b.0.format).unwrap_or_default();

    let pack = assemble(&ctx, &review_id, &ws).await?;
    let markdown = render_markdown(&pack);
    let summary_json = serde_json::to_string(&pack.summary).unwrap_or_else(|_| "{}".to_string());
    let who = actor_label(&user);

    let snapshot = ctx
        .proof_packs_store
        .create(&review_id, &ws, "markdown", &markdown, &summary_json, &who)
        .await
        .map_err(ApiError)?;

    // Ingest VERIFIED findings into the memory store for durable hybrid recall
    // (the Context-Engine recall side of the loop).
    let verified: Vec<&Finding> = pack
        .findings
        .iter()
        .map(|e| &e.finding)
        .filter(|f| f.status == FindingStatus::Verified)
        .collect();
    for f in verified {
        let mem = finding_to_memory(f);
        if let Err(e) = ctx.memory.save(&ws, &who, vec![mem]).await {
            tracing::warn!("proof-pack memory ingest failed: {e}");
        }
    }

    let _ = ctx.events.send(Event::ProofPackExported {
        workspace_id: ws,
        review_id,
        proof_pack_id: snapshot.id.clone(),
    });
    Ok(Json(snapshot))
}

/// Build a memory record from a verified finding (commit + test linked as refs).
fn finding_to_memory(f: &Finding) -> NewMemory {
    let mut refs = Vec::new();
    if let Some(c) = f.linked_commit.as_deref().filter(|s| !s.is_empty()) {
        refs.push(otto_state::memory::MemoryRef {
            kind: "commit".into(),
            reference: c.into(),
            url: None,
            label: None,
        });
    }
    if let Some(t) = f.linked_test.as_deref().filter(|s| !s.is_empty()) {
        refs.push(otto_state::memory::MemoryRef {
            kind: "test".into(),
            reference: t.into(),
            url: None,
            label: None,
        });
    }
    let mut tags = vec![f.severity.as_str().to_string(), "finding".to_string()];
    if let Some(c) = &f.category {
        tags.push(c.clone());
    }
    NewMemory {
        collection: "findings".into(),
        record_type: "note".into(),
        scope: Scope::Workspace,
        story_id: f.jira_key.clone(),
        kind: "finding".into(),
        title: f.title.clone(),
        body: format!(
            "{}\n\nEvidence:\n{}\n\nReasoning:\n{}\n\nFix:\n{}",
            f.body,
            f.evidence,
            f.agent_reasoning_summary,
            f.suggested_fix.clone().unwrap_or_default()
        ),
        entities: Vec::new(),
        tags,
        source_kind: "analysis".into(),
        source_ref: f.linked_commit.clone(),
        refs,
        confidence: None,
        salience: None,
        visibility: "shared".into(),
    }
}

/// Render the Proof Pack as a human-readable markdown artifact.
pub fn render_markdown(pack: &ReviewProofPack) -> String {
    let s = &pack.summary;
    let mut md = String::new();
    md.push_str(&format!("# Proof Pack — review {}\n\n", pack.review_id));
    md.push_str(&format!("_Generated {}_\n\n", pack.generated_at));
    md.push_str("## Summary\n\n");
    md.push_str(&format!(
        "- Findings: **{}** (verified {}, fixed {}, open {})\n- With fixing commit: {} · With regression test: {}\n\n",
        s.total, s.verified, s.fixed, s.open, s.with_commit, s.with_test
    ));
    md.push_str("## Findings\n\n");
    for entry in &pack.findings {
        let f = &entry.finding;
        md.push_str(&format!(
            "### [{}] {} — {}\n\n",
            f.severity.as_str(),
            f.title,
            f.status.as_str()
        ));
        if let Some(p) = &f.path {
            let range = match (f.line, f.line_end) {
                (Some(a), Some(b)) if b != a => format!("{p}:{a}-{b}"),
                (Some(a), _) => format!("{p}:{a}"),
                _ => p.clone(),
            };
            md.push_str(&format!("- Location: `{range}`\n"));
        }
        if let Some(c) = f.linked_commit.as_deref().filter(|s| !s.is_empty()) {
            md.push_str(&format!("- Fixed by commit: `{c}`\n"));
        }
        if let Some(t) = f.linked_test.as_deref().filter(|s| !s.is_empty()) {
            md.push_str(&format!("- Guarded by test: `{t}`\n"));
        }
        if let Some(j) = &f.jira_key {
            md.push_str(&format!("- Jira: {j}\n"));
        }
        if !f.evidence.trim().is_empty() {
            md.push_str(&format!("\n**Evidence**\n\n```\n{}\n```\n", f.evidence.trim()));
        }
        if !f.agent_reasoning_summary.trim().is_empty() {
            md.push_str(&format!("\n**Reasoning:** {}\n", f.agent_reasoning_summary.trim()));
        }
        if let Some(fix) = f.suggested_fix.as_deref().filter(|s| !s.trim().is_empty()) {
            md.push_str(&format!("\n**Suggested fix:** {}\n", fix.trim()));
        }
        // Event timeline (the audit trail).
        if !entry.events.is_empty() {
            md.push_str("\n**Timeline**\n\n");
            for ev in &entry.events {
                md.push_str(&format!("- `{}` {} by {}\n", ev.created_at, ev.kind, ev.actor));
            }
        }
        md.push('\n');
    }
    if !pack.repo_rules.is_empty() {
        md.push_str("## Repo rules generated\n\n");
        for r in &pack.repo_rules {
            md.push_str(&format!("- **{}** — {}\n", r.title, r.body.trim()));
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::finding::{FindingSeverity, ReviewProofPackEntry};

    fn finding(id: &str, status: FindingStatus, commit: Option<&str>, test: Option<&str>) -> Finding {
        Finding {
            id: id.into(),
            review_id: "rev".into(),
            workspace_id: "ws".into(),
            repo_id: "repo".into(),
            pr_number: Some(1),
            fingerprint: "fp".into(),
            severity: FindingSeverity::High,
            category: Some("security".into()),
            path: Some("src/db.rs".into()),
            line: Some(10),
            line_end: Some(12),
            title: "SQLi".into(),
            body: "body".into(),
            evidence: "format!()".into(),
            agent_reasoning_summary: "tainted".into(),
            suggested_fix: Some("parameterize".into()),
            status,
            linked_commit: commit.map(String::from),
            linked_test: test.map(String::from),
            reviewer: "alice".into(),
            state: "open".into(),
            regressed: false,
            requires_human_approval: false,
            approval_decision: None,
            approved_by: None,
            approved_at: None,
            jira_key: None,
            jira_url: None,
            produced_by_agent: Some("grill".into()),
            repo_rule_id: None,
            fix_session_id: None,
            occurrence_count: 1,
            created_at: "t".into(),
            updated_at: "t".into(),
        }
    }

    fn pack_of(findings: Vec<Finding>) -> ReviewProofPack {
        let mut summary = ReviewProofPackSummary::default();
        let mut entries = vec![];
        for f in findings {
            summary.total += 1;
            *summary.by_status.entry(f.status.as_str().to_string()).or_insert(0) += 1;
            match f.status {
                FindingStatus::Verified => summary.verified += 1,
                FindingStatus::Fixed => summary.fixed += 1,
                FindingStatus::Open => summary.open += 1,
                _ => {}
            }
            if f.linked_commit.is_some() {
                summary.with_commit += 1;
            }
            if f.linked_test.is_some() {
                summary.with_test += 1;
            }
            entries.push(ReviewProofPackEntry { finding: f, events: vec![] });
        }
        ReviewProofPack {
            review_id: "rev".into(),
            workspace_id: "ws".into(),
            generated_at: "t".into(),
            summary,
            findings: entries,
            repo_rules: vec![],
        }
    }

    #[test]
    fn summary_counts_and_markdown() {
        let pack = pack_of(vec![
            finding("a", FindingStatus::Verified, Some("abc"), Some("t.rs")),
            finding("b", FindingStatus::Open, None, None),
            finding("c", FindingStatus::Fixed, Some("def"), None),
        ]);
        assert_eq!(pack.summary.total, 3);
        assert_eq!(pack.summary.verified, 1);
        assert_eq!(pack.summary.fixed, 1);
        assert_eq!(pack.summary.open, 1);
        assert_eq!(pack.summary.with_commit, 2);
        assert_eq!(pack.summary.with_test, 1);
        let md = render_markdown(&pack);
        assert!(md.contains("Proof Pack — review rev"));
        assert!(md.contains("Fixed by commit: `abc`"));
        assert!(md.contains("Guarded by test: `t.rs`"));
    }

    #[test]
    fn memory_record_links_commit_and_test() {
        let f = finding("a", FindingStatus::Verified, Some("abc123"), Some("t.rs"));
        let mem = finding_to_memory(&f);
        assert_eq!(mem.kind, "finding");
        assert_eq!(mem.refs.len(), 2);
        assert!(mem.refs.iter().any(|r| r.kind == "commit" && r.reference == "abc123"));
        assert!(mem.refs.iter().any(|r| r.kind == "test"));
        assert!(mem.tags.contains(&"security".to_string()));
    }
}
