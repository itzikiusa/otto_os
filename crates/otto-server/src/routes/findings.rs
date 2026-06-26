//! Review **findings workflow** endpoints — the id-keyed action surface that
//! turns code-review output into a tracked security/code-quality workflow.
//!
//! Each finding (`crates/otto-state/review_findings`) carries the 11 required
//! fields + a 6-state workflow `status`. Every action here resolves the finding,
//! checks the workspace role, performs the transition / field patch (validated by
//! `FindingStatus::can_transition`), appends an immutable `finding_events` audit
//! row, emits `Event::FindingUpdated`, and returns the updated `Finding`.
//!
//! - `GET  /findings/{id}`                      — finding + its event timeline
//! - `POST /findings/{id}/accept`               — open → accepted
//! - `POST /findings/{id}/waive`                — → waived
//! - `POST /findings/{id}/false-positive`       — → false_positive
//! - `POST /findings/{id}/require-approval`     — set the human-approval gate
//! - `POST /findings/{id}/approve`              — approve (clear gate) / reject (→ FP)
//! - `POST /findings/{id}/jira`                 — convert to a Jira issue        (jira.rs pass)
//! - `POST /findings/{id}/repo-rule`            — generalize into a repo rule    (repo_rules pass)
//! - `POST /findings/{id}/fix|verify|regression-test` — agent-backed            (finding_agent pass)

use std::time::Duration;

use axum::extract::{Path, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use otto_core::domain::{User, WorkspaceRole};
use otto_core::event::Event;
use otto_core::finding::{Finding, FindingActionResp, FindingDetail, FindingStatus, RepoRule};
use otto_core::Error;
use otto_state::repo_rules::NewRepoRule;
use otto_state::FindingPatch;

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::finding_agent;
use crate::state::ServerCtx;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route("/findings/{id}", get(get_finding))
        .route("/findings/{id}/accept", post(accept))
        .route("/findings/{id}/waive", post(waive))
        .route("/findings/{id}/false-positive", post(false_positive))
        .route("/findings/{id}/require-approval", post(require_approval))
        .route("/findings/{id}/approve", post(approve))
        .route("/findings/{id}/jira", post(to_jira))
        .route("/findings/{id}/repo-rule", post(to_repo_rule))
        .route("/findings/{id}/fix", post(fix))
        .route("/findings/{id}/verify", post(verify))
        .route("/findings/{id}/regression-test", post(regression_test))
        // E2E-only seed (handler 404s unless OTTO_E2E; policy-exempt).
        .route("/workspaces/{ws}/__e2e/findings", post(e2e_seed))
}

/// The provider used for agent-backed finding actions. Kept simple (the daemon's
/// default agent CLI); the session is best-effort so a missing CLI just means no
/// live session, not a failed action.
fn finding_agent_provider() -> String {
    std::env::var("OTTO_DEFAULT_PROVIDER").unwrap_or_else(|_| "claude".to_string())
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// The label recorded as a finding's `reviewer` / an event `actor`.
pub(crate) fn actor_label(user: &User) -> String {
    if user.display_name.trim().is_empty() {
        user.id.clone()
    } else {
        user.display_name.clone()
    }
}

/// Load a finding by id and enforce the caller's workspace role (the finding row
/// carries its own `workspace_id`, so no review/repo join is needed).
pub(crate) async fn load_for_role(
    ctx: &ServerCtx,
    user: &User,
    id: &str,
    min: WorkspaceRole,
) -> ApiResult<Finding> {
    let f = ctx.findings_store.get_full(id).await.map_err(ApiError)?;
    require_ws_role(ctx, user, &f.workspace_id, min).await?;
    Ok(f)
}

/// Broadcast a `finding_updated` so the board refetches live.
pub(crate) fn emit_updated(ctx: &ServerCtx, f: &Finding) {
    let _ = ctx.events.send(Event::FindingUpdated {
        workspace_id: f.workspace_id.clone(),
        review_id: f.review_id.clone(),
        finding_id: f.id.clone(),
        status: f.status.as_str().to_string(),
    });
}

/// Append one audit-trail event (best-effort log on failure).
pub(crate) async fn audit(
    ctx: &ServerCtx,
    f: &Finding,
    kind: &str,
    actor: &str,
    from: Option<&str>,
    to: Option<&str>,
    detail: serde_json::Value,
) {
    if let Err(e) = ctx
        .finding_events_store
        .append(&f.id, &f.workspace_id, kind, actor, from, to, detail)
        .await
    {
        tracing::warn!("finding audit append ({kind}): {e}");
    }
}

// ---------------------------------------------------------------------------
// Read
// ---------------------------------------------------------------------------

/// `GET /findings/{id}` → the finding plus its full event timeline.
async fn get_finding(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<FindingDetail>> {
    let finding = load_for_role(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let events = ctx
        .finding_events_store
        .list_for_finding(&id)
        .await
        .map_err(ApiError)?;
    Ok(Json(FindingDetail { finding, events }))
}

// ---------------------------------------------------------------------------
// Pure-status triage actions
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ReasonReq {
    #[serde(default)]
    reason: Option<String>,
}

/// `POST /findings/{id}/accept` — open → accepted (this is a real issue to fix).
async fn accept(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Finding>> {
    let cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let f = ctx
        .findings_store
        .set_status(&id, FindingStatus::Accepted, &who)
        .await
        .map_err(ApiError)?;
    audit(&ctx, &f, "accepted", &who, Some(cur.status.as_str()), Some("accepted"), serde_json::json!({})).await;
    emit_updated(&ctx, &f);
    Ok(Json(f))
}

/// `POST /findings/{id}/waive` — accepted risk; intentionally not fixing.
async fn waive(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<ReasonReq>,
) -> ApiResult<Json<Finding>> {
    let cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let f = ctx
        .findings_store
        .set_status(&id, FindingStatus::Waived, &who)
        .await
        .map_err(ApiError)?;
    audit(&ctx, &f, "waived", &who, Some(cur.status.as_str()), Some("waived"),
        serde_json::json!({ "reason": body.reason })).await;
    emit_updated(&ctx, &f);
    Ok(Json(f))
}

/// `POST /findings/{id}/false-positive` — not a real issue.
async fn false_positive(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<ReasonReq>,
) -> ApiResult<Json<Finding>> {
    let cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let f = ctx
        .findings_store
        .set_status(&id, FindingStatus::FalsePositive, &who)
        .await
        .map_err(ApiError)?;
    audit(&ctx, &f, "false_positive", &who, Some(cur.status.as_str()), Some("false_positive"),
        serde_json::json!({ "reason": body.reason })).await;
    emit_updated(&ctx, &f);
    Ok(Json(f))
}

/// `POST /findings/{id}/require-approval` — flag that a human must sign off
/// before this finding can be auto-closed. Orthogonal to status (the gate).
async fn require_approval(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Finding>> {
    let _cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let f = ctx
        .findings_store
        .set_fields(&id, &FindingPatch { requires_human_approval: Some(true), ..Default::default() })
        .await
        .map_err(ApiError)?;
    audit(&ctx, &f, "approval_required", &who, None, None, serde_json::json!({})).await;
    emit_updated(&ctx, &f);
    Ok(Json(f))
}

#[derive(Deserialize)]
struct ApproveReq {
    /// "approve" | "reject"
    decision: String,
    #[serde(default)]
    note: Option<String>,
}

/// `POST /findings/{id}/approve` — resolve the human-approval gate. `approve`
/// clears the gate (and accepts an `open` finding); `reject` → `false_positive`.
async fn approve(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<ApproveReq>,
) -> ApiResult<Json<Finding>> {
    let cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let now = chrono::Utc::now().to_rfc3339();
    let approved = matches!(body.decision.as_str(), "approve" | "approved" | "accept");
    let decision = if approved { "approved" } else { "rejected" };

    // Record the decision + clear the gate.
    let mut f = ctx
        .findings_store
        .set_fields(
            &id,
            &FindingPatch {
                requires_human_approval: Some(false),
                approved_by: Some(who.clone()),
                approved_at: Some(now),
                approval_decision: Some(decision.to_string()),
                ..Default::default()
            },
        )
        .await
        .map_err(ApiError)?;

    // Apply the status effect.
    if approved {
        // Accept an untriaged finding; otherwise leave its disposition as-is.
        if cur.status == FindingStatus::Open {
            f = ctx
                .findings_store
                .set_status(&id, FindingStatus::Accepted, &who)
                .await
                .map_err(ApiError)?;
        }
    } else {
        f = ctx
            .findings_store
            .set_status(&id, FindingStatus::FalsePositive, &who)
            .await
            .map_err(ApiError)?;
    }
    audit(&ctx, &f, decision, &who, Some(cur.status.as_str()), Some(f.status.as_str()),
        serde_json::json!({ "note": body.note })).await;
    emit_updated(&ctx, &f);
    Ok(Json(f))
}

// ---------------------------------------------------------------------------
// Convert to Jira
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct JiraReq {
    project_key: String,
    #[serde(default)]
    issue_type: Option<String>,
    #[serde(default)]
    account_id: Option<String>,
}

/// Build the Jira issue description (Atlassian wiki markup) from a finding.
fn jira_description_md(f: &Finding) -> String {
    let range = match (&f.path, f.line, f.line_end) {
        (Some(p), Some(a), Some(b)) if b != a => format!("{p}:{a}-{b}"),
        (Some(p), Some(a), _) => format!("{p}:{a}"),
        (Some(p), _, _) => p.clone(),
        _ => "(no location)".to_string(),
    };
    format!(
        "*Otto code-review finding* (`{}`)\n\n*Severity:* {}\n*Location:* {{{{{}}}}}\n\n*Evidence:*\n{{code}}\n{}\n{{code}}\n\n*Reasoning:* {}\n\n*Suggested fix:* {}",
        f.id,
        f.severity.as_str(),
        range,
        f.evidence.trim(),
        f.agent_reasoning_summary.trim(),
        f.suggested_fix.clone().unwrap_or_else(|| "(none)".to_string()).trim(),
    )
}

/// `POST /findings/{id}/jira` — file the finding as a Jira issue. Idempotent: if
/// already filed, returns the finding unchanged. 400 if no Jira account exists.
async fn to_jira(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<JiraReq>,
) -> ApiResult<Json<Finding>> {
    let f = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    if f.jira_key.is_some() {
        return Ok(Json(f)); // already filed
    }
    let account = match &body.account_id {
        Some(aid) => ctx.issues_store.get_account(aid).await.map_err(ApiError)?,
        None => ctx
            .issues_store
            .list_accounts(&user.id)
            .await
            .map_err(ApiError)?
            .into_iter()
            .next()
            .ok_or_else(|| ApiError(Error::Invalid("no Jira account configured".to_string())))?,
    };
    otto_core::auth::authorize_owner(&account, &user).map_err(ApiError)?;
    let token = ctx
        .secrets
        .get(&account.token_ref)
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::Invalid("Jira token missing".to_string())))?;
    let client = otto_issues::JiraClient::new(&account.base_url, &account.email, &token);
    let issue_type = body.issue_type.clone().unwrap_or_else(|| "Bug".to_string());
    let created = client
        .create_issue(&body.project_key, &issue_type, &f.title, &jira_description_md(&f))
        .await
        .map_err(ApiError)?;

    let who = actor_label(&user);
    let f = ctx
        .findings_store
        .set_fields(
            &id,
            &FindingPatch {
                jira_key: Some(created.key.clone()),
                jira_url: Some(created.url.clone()),
                ..Default::default()
            },
        )
        .await
        .map_err(ApiError)?;
    audit(&ctx, &f, "jira_created", &who, None, None,
        serde_json::json!({ "jira_key": created.key, "jira_url": created.url })).await;
    emit_updated(&ctx, &f);
    Ok(Json(f))
}

// ---------------------------------------------------------------------------
// Add to repo rule (Context Engine)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RepoRuleReq {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    glob: Option<String>,
}

/// `POST /findings/{id}/repo-rule` — generalize the finding into a durable repo
/// rule and register it with the Context Engine (materialized into future
/// sessions' instruction files).
async fn to_repo_rule(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(body): Json<RepoRuleReq>,
) -> ApiResult<Json<RepoRule>> {
    let f = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let title = body
        .title
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| f.title.clone());
    let rule_body = body.body.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
        f.suggested_fix
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| f.agent_reasoning_summary.clone())
    });
    let rule = ctx
        .repo_rules_store
        .create(
            &f.workspace_id,
            &who,
            NewRepoRule {
                title: &title,
                body: &rule_body,
                category: f.category.as_deref(),
                severity: Some(f.severity.as_str()),
                glob: body.glob.as_deref(),
                source_finding_id: Some(&f.id),
            },
        )
        .await
        .map_err(ApiError)?;
    // Link the rule to the finding + re-render the workspace context block.
    let _ = ctx
        .findings_store
        .set_fields(&id, &FindingPatch { repo_rule_id: Some(rule.id.clone()), ..Default::default() })
        .await;
    let _ = crate::finding_context::apply_repo_rules_to_context(&ctx, &f.workspace_id).await;
    audit(&ctx, &f, "repo_rule_added", &who, None, None,
        serde_json::json!({ "repo_rule_id": rule.id })).await;
    if let Ok(updated) = ctx.findings_store.get_full(&id).await {
        emit_updated(&ctx, &updated);
    }
    Ok(Json(rule))
}

// ---------------------------------------------------------------------------
// Agent-backed actions (fix / verify / regression-test)
// ---------------------------------------------------------------------------

fn fix_prompt(f: &Finding) -> String {
    format!(
        "You are fixing a code-review finding in this repository.\n\nTitle: {}\nSeverity: {}\nLocation: {}\n\nEvidence:\n{}\n\nWhy it's a problem: {}\n\nSuggested fix: {}\n\nMake the MINIMAL change that resolves it, then commit it with a clear message. Do not change unrelated code.",
        f.title,
        f.severity.as_str(),
        f.path.clone().unwrap_or_default(),
        f.evidence,
        f.agent_reasoning_summary,
        f.suggested_fix.clone().unwrap_or_default(),
    )
}

fn verify_prompt(f: &Finding) -> String {
    format!(
        "Verify whether this code-review finding is resolved at the current HEAD.\n\nTitle: {}\nLocation: {}\nOriginal evidence:\n{}\n\nInspect the code and state clearly whether the issue is resolved.",
        f.title,
        f.path.clone().unwrap_or_default(),
        f.evidence,
    )
}

fn regression_prompt(f: &Finding) -> String {
    format!(
        "Write a regression test that FAILS on the bug described below and PASSES once it is fixed. Add it to this repo's test suite and commit it.\n\nTitle: {}\nLocation: {}\nEvidence:\n{}\n\nReasoning: {}",
        f.title,
        f.path.clone().unwrap_or_default(),
        f.evidence,
        f.agent_reasoning_summary,
    )
}

/// Background watcher: when the fix agent's worktree HEAD advances, stamp
/// `linked_commit` + transition `accepted → fixed` (respecting the approval gate).
fn spawn_fix_watcher(ctx: ServerCtx, finding_id: String, worktree: String, base_head: String) {
    tokio::spawn(async move {
        let dir = std::path::PathBuf::from(&worktree);
        let mut waited = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            waited += 5;
            if let Some(sha) = finding_agent::stamp_fix(Some(&base_head), &dir) {
                if let Ok(f) = ctx.findings_store.get_full(&finding_id).await {
                    // Gate: never auto-advance a finding awaiting human approval.
                    if f.requires_human_approval && f.approved_at.is_none() {
                        break;
                    }
                    let _ = ctx
                        .findings_store
                        .set_fields(&finding_id, &FindingPatch { linked_commit: Some(sha.clone()), ..Default::default() })
                        .await;
                    if matches!(f.status, FindingStatus::Accepted | FindingStatus::Open) {
                        if let Ok(f2) = ctx
                            .findings_store
                            .set_status(&finding_id, FindingStatus::Fixed, "fix-agent")
                            .await
                        {
                            audit(&ctx, &f2, "fix_applied", "fix-agent", Some(f.status.as_str()), Some("fixed"),
                                serde_json::json!({ "commit": sha })).await;
                            emit_updated(&ctx, &f2);
                        }
                    }
                }
                break;
            }
            if waited >= 360 {
                break; // ~6 min — give up watching
            }
        }
    });
}

/// Background watcher: when the regression agent adds a new test file, set
/// `linked_test` + append the `regression_test_added` event.
fn spawn_regression_watcher(
    ctx: ServerCtx,
    finding_id: String,
    worktree: String,
    before: std::collections::HashSet<String>,
) {
    tokio::spawn(async move {
        let dir = std::path::PathBuf::from(&worktree);
        let mut waited = 0u64;
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            waited += 5;
            if let Some(test) = finding_agent::detect_new_test(&before, &dir) {
                let _ = ctx
                    .findings_store
                    .set_fields(&finding_id, &FindingPatch { linked_test: Some(test.clone()), ..Default::default() })
                    .await;
                if let Ok(f) = ctx.findings_store.get_full(&finding_id).await {
                    audit(&ctx, &f, "regression_test_added", "test-agent", None, None,
                        serde_json::json!({ "test": test })).await;
                    emit_updated(&ctx, &f);
                }
                break;
            }
            if waited >= 360 {
                break;
            }
        }
    });
}

/// `POST /findings/{id}/fix` — accept the finding + spawn a fix agent in an
/// isolated worktree. Returns immediately with the session id; the watcher stamps
/// `fixed` + `linked_commit` when the agent commits.
async fn fix(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<FindingActionResp>> {
    let cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    // Accept it (we've decided it's a real issue to fix).
    let f = if matches!(cur.status, FindingStatus::Open | FindingStatus::Accepted) {
        ctx.findings_store
            .set_status(&id, FindingStatus::Accepted, &who)
            .await
            .map_err(ApiError)?
    } else {
        cur.clone()
    };
    let repo = ctx.git_store.get_repo(&f.repo_id).await.map_err(ApiError)?;
    let provider = finding_agent_provider();
    let session_id = match finding_agent::provision_worktree(&repo.path, &f.id).await {
        Ok((wt, base)) => {
            let sid = finding_agent::spawn_session(
                &ctx, &f.workspace_id, &user.id, &provider, &wt, &f.id, "fix", fix_prompt(&f),
            )
            .await;
            if let Some(ref s) = sid {
                let _ = ctx
                    .findings_store
                    .set_fields(&id, &FindingPatch { fix_session_id: Some(s.clone()), ..Default::default() })
                    .await;
                spawn_fix_watcher(ctx.clone(), id.clone(), wt, base);
            }
            sid
        }
        Err(e) => {
            tracing::warn!("fix worktree provision failed: {e}");
            None
        }
    };
    audit(&ctx, &f, "fix_requested", &who, Some(cur.status.as_str()), Some(f.status.as_str()),
        serde_json::json!({ "session_id": session_id })).await;
    let _ = ctx.events.send(Event::FindingActionStarted {
        workspace_id: f.workspace_id.clone(),
        review_id: f.review_id.clone(),
        finding_id: f.id.clone(),
        action: "fix".to_string(),
        session_id: session_id.clone(),
    });
    emit_updated(&ctx, &f);
    let finding = ctx.findings_store.get_full(&id).await.map_err(ApiError)?;
    Ok(Json(FindingActionResp { finding, session_id }))
}

/// `POST /findings/{id}/verify` — verify the finding is resolved. On pass →
/// `verified` + `linked_commit = HEAD`. Blocked by the approval gate.
async fn verify(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<FindingActionResp>> {
    let cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    if cur.requires_human_approval && cur.approved_at.is_none() {
        return Err(ApiError(Error::Invalid(
            "finding requires human approval before verification".to_string(),
        )));
    }
    if !matches!(
        cur.status,
        FindingStatus::Accepted | FindingStatus::Fixed | FindingStatus::Verified
    ) {
        return Err(ApiError(Error::Invalid(format!(
            "cannot verify a finding in status '{}'",
            cur.status.as_str()
        ))));
    }
    let repo = ctx.git_store.get_repo(&cur.repo_id).await.map_err(ApiError)?;
    let provider = finding_agent_provider();
    // Spawn an openable verify agent (best-effort) for the user to watch.
    let session_id = match finding_agent::provision_worktree(&repo.path, &cur.id).await {
        Ok((wt, _)) => {
            finding_agent::spawn_session(
                &ctx, &cur.workspace_id, &user.id, &provider, &wt, &cur.id, "verify", verify_prompt(&cur),
            )
            .await
        }
        Err(_) => None,
    };
    let _ = ctx.events.send(Event::FindingActionStarted {
        workspace_id: cur.workspace_id.clone(),
        review_id: cur.review_id.clone(),
        finding_id: cur.id.clone(),
        action: "verify".to_string(),
        session_id: session_id.clone(),
    });

    let pass = finding_agent::judge_verify(&cur, std::path::Path::new(&repo.path));
    let finding = if pass {
        if cur.linked_commit.is_none() {
            if let Some(head) = finding_agent::head_of(std::path::Path::new(&repo.path)) {
                let _ = ctx
                    .findings_store
                    .set_fields(&id, &FindingPatch { linked_commit: Some(head), ..Default::default() })
                    .await;
            }
        }
        let f = ctx
            .findings_store
            .set_status(&id, FindingStatus::Verified, &who)
            .await
            .map_err(ApiError)?;
        audit(&ctx, &f, "verified", &who, Some(cur.status.as_str()), Some("verified"),
            serde_json::json!({ "commit": f.linked_commit })).await;
        emit_updated(&ctx, &f);
        f
    } else {
        let f = ctx.findings_store.get_full(&id).await.map_err(ApiError)?;
        audit(&ctx, &f, "verify_failed", &who, None, None, serde_json::json!({})).await;
        emit_updated(&ctx, &f);
        f
    };
    Ok(Json(FindingActionResp { finding, session_id }))
}

/// `POST /findings/{id}/regression-test` — spawn an agent to add a guard test;
/// the watcher sets `linked_test` when a new test file appears.
async fn regression_test(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<FindingActionResp>> {
    let cur = load_for_role(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let who = actor_label(&user);
    let repo = ctx.git_store.get_repo(&cur.repo_id).await.map_err(ApiError)?;
    let provider = finding_agent_provider();
    let session_id = match finding_agent::provision_worktree(&repo.path, &cur.id).await {
        Ok((wt, _)) => {
            let before = finding_agent::list_test_files(std::path::Path::new(&wt));
            let sid = finding_agent::spawn_session(
                &ctx, &cur.workspace_id, &user.id, &provider, &wt, &cur.id, "regression_test",
                regression_prompt(&cur),
            )
            .await;
            if sid.is_some() {
                spawn_regression_watcher(ctx.clone(), id.clone(), wt, before);
            }
            sid
        }
        Err(_) => None,
    };
    let _ = ctx.events.send(Event::FindingActionStarted {
        workspace_id: cur.workspace_id.clone(),
        review_id: cur.review_id.clone(),
        finding_id: cur.id.clone(),
        action: "regression_test".to_string(),
        session_id: session_id.clone(),
    });
    audit(&ctx, &cur, "regression_test_requested", &who, None, None,
        serde_json::json!({ "session_id": session_id })).await;
    let finding = ctx.findings_store.get_full(&id).await.map_err(ApiError)?;
    emit_updated(&ctx, &finding);
    Ok(Json(FindingActionResp { finding, session_id }))
}

// ---------------------------------------------------------------------------
// E2E-only seed (deterministic preconditions; OTTO_E2E-gated, policy-exempt)
// ---------------------------------------------------------------------------

fn e2e_enabled() -> bool {
    matches!(std::env::var("OTTO_E2E").as_deref(), Ok("1") | Ok("true"))
}

/// Drive a freshly-seeded (open) finding to a target status via legal transitions
/// (every status is reachable from `open` in ≤2 legal steps — no machine bypass).
async fn drive_to_status(ctx: &ServerCtx, id: &str, target: FindingStatus, who: &str) -> ApiResult<Finding> {
    use FindingStatus::*;
    let steps: &[FindingStatus] = match target {
        Open => &[],
        Accepted => &[Accepted],
        FalsePositive => &[FalsePositive],
        Waived => &[Waived],
        Fixed => &[Accepted, Fixed],
        Verified => &[Accepted, Verified],
    };
    let mut last = ctx.findings_store.get_full(id).await.map_err(ApiError)?;
    for s in steps {
        last = ctx.findings_store.set_status(id, *s, who).await.map_err(ApiError)?;
    }
    Ok(last)
}

#[derive(Deserialize)]
struct SeedReq {
    review_id: String,
    repo_id: String,
    #[serde(default)]
    pr_number: Option<u64>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<i64>,
    #[serde(default)]
    line_end: Option<i64>,
    #[serde(default = "default_seed_sev")]
    severity: String,
    #[serde(default)]
    category: Option<String>,
    #[serde(default = "default_seed_title")]
    title: String,
    #[serde(default = "default_seed_body")]
    body: String,
    #[serde(default = "default_seed_evidence")]
    evidence: String,
    #[serde(default = "default_seed_reasoning")]
    reasoning: String,
    #[serde(default)]
    suggested_fix: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    linked_commit: Option<String>,
    #[serde(default)]
    linked_test: Option<String>,
    #[serde(default)]
    requires_human_approval: Option<bool>,
}
fn default_seed_sev() -> String { "high".into() }
fn default_seed_title() -> String { "Seeded finding".into() }
fn default_seed_body() -> String { "Seeded body".into() }
fn default_seed_evidence() -> String { "seeded evidence".into() }
fn default_seed_reasoning() -> String { "seeded reasoning".into() }

/// `POST /workspaces/{ws}/__e2e/findings` — insert a finding with arbitrary fields
/// for deterministic E2E preconditions. 404 unless `OTTO_E2E` is set.
async fn e2e_seed(
    Path(ws): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(b): Json<SeedReq>,
) -> ApiResult<Json<Finding>> {
    if !e2e_enabled() {
        return Err(ApiError(Error::NotFound("not found".to_string())));
    }
    let who = actor_label(&user);
    let fingerprint = otto_state::review_findings::compute_fingerprint(
        &b.repo_id,
        b.pr_number.unwrap_or(0),
        b.path.as_deref(),
        b.category.as_deref(),
        &format!("{}-{}", b.title, b.body),
    );
    let nf = otto_state::NewFinding {
        review_id: &b.review_id,
        workspace_id: &ws,
        repo_id: &b.repo_id,
        pr_number: b.pr_number,
        path: b.path.as_deref(),
        line: b.line,
        line_end: b.line_end,
        severity: &b.severity,
        category: b.category.as_deref(),
        title: &b.title,
        body: &b.body,
        evidence: &b.evidence,
        agent_reasoning_summary: &b.reasoning,
        suggested_fix: b.suggested_fix.as_deref(),
        produced_by_agent: Some("e2e"),
        reviewer: &who,
        fingerprint: &fingerprint,
        run_id: &b.review_id,
    };
    let (mut f, _) = ctx.findings_store.upsert(&nf).await.map_err(ApiError)?;
    // anchor a created event
    let _ = ctx
        .finding_events_store
        .append(&f.id, &ws, "created", "e2e", None, Some("open"), serde_json::json!({}))
        .await;
    // patch artifact/gate fields
    if b.linked_commit.is_some() || b.linked_test.is_some() || b.requires_human_approval.is_some() {
        f = ctx
            .findings_store
            .set_fields(
                &f.id,
                &FindingPatch {
                    linked_commit: b.linked_commit,
                    linked_test: b.linked_test,
                    requires_human_approval: b.requires_human_approval,
                    ..Default::default()
                },
            )
            .await
            .map_err(ApiError)?;
    }
    // drive to the requested status
    if let Some(s) = b.status.as_deref().and_then(FindingStatus::parse) {
        f = drive_to_status(&ctx, &f.id, s, &who).await?;
    }
    Ok(Json(f))
}
