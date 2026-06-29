//! Run with Otto — webhook result callback delivery.
//!
//! A run launched from the webhook entry (`POST /api/v1/webhooks/{ws}/run`)
//! carries an optional `callback_url`. This module POSTs a compact result
//! summary back to that URL at the milestones a caller can act on: the approval
//! gate and every terminal state. It is the Run-with-Otto analogue of
//! `otto_channels::WebhookAdapter` (which delivers a *generic* webhook agent's
//! reply) and shares its SSRF posture verbatim: the URL is validated through
//! [`otto_netguard::check_url`] and a redirect can't bounce the POST onto a
//! private/loopback/metadata address ([`otto_netguard::redirect_policy`]).
//!
//! Delivery is strictly best-effort: a missing URL is a silent no-op, and a
//! blocked or failed POST is recorded as a `delivery` timeline event but never
//! fails the run. The body is the run's public read shape — never a secret.
//!
//! Taking `&RunsRepo` (not the full `ServerCtx`) keeps the unit testable and
//! reflects that delivery only ever reads the run + appends a timeline event.

use std::time::Duration;

use otto_core::run::{OttoRun, RunStatus};
use otto_state::runs::NewRunEvent;
use otto_state::RunsRepo;
use serde_json::{json, Value};

/// How long a single callback POST may take before we give up.
const CALLBACK_TIMEOUT: Duration = Duration::from_secs(20);

/// Build the compact JSON result a webhook caller receives. Pure (no I/O) so it
/// is unit-testable. Carries the run's public shape: enough to drive an external
/// automation (approve, open a PR, file a ticket) without a follow-up fetch.
pub(crate) fn build_payload(run: &OttoRun) -> Value {
    json!({
        "run_id": run.id,
        "workspace_id": run.workspace_id,
        "status": run.status.as_str(),
        "awaiting_approval": run.status == RunStatus::AwaitingApproval,
        "terminal": run.status.is_terminal(),
        "title": run.title,
        "source_kind": run.source_kind.as_str(),
        "source_ref": run.source_ref,
        "source_url": run.source_url,
        "mode": run.mode.as_str(),
        "proof_status": run.proof_status,
        "risk_score": run.risk_score,
        "findings_total": run.findings_total,
        "findings_blocking": run.findings_blocking,
        "has_pr_draft": run.pr_draft_json.is_some(),
        "pr_url": run.pr_url,
        "approval_decision": run.approval_decision,
        "error": run.error,
    })
}

/// Best-effort POST of the run's result to its `callback_url`. No-op when the run
/// has none (every non-webhook origin). SSRF-guarded; records a `delivery`
/// timeline event (delivered / blocked / failed) for transparency.
pub(crate) async fn deliver(runs: &RunsRepo, run: &OttoRun) {
    let Some(url) = run
        .callback_url
        .as_deref()
        .map(str::trim)
        .filter(|u| !u.is_empty())
    else {
        return;
    };

    if let Err(reason) = otto_netguard::check_url(url).await {
        record(
            runs,
            run,
            &format!("Callback blocked (SSRF guard): {reason}"),
        )
        .await;
        return;
    }

    let client = match reqwest::Client::builder()
        .timeout(CALLBACK_TIMEOUT)
        .redirect(otto_netguard::redirect_policy())
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            record(runs, run, &format!("Callback client error: {e}")).await;
            return;
        }
    };

    match client.post(url).json(&build_payload(run)).send().await {
        Ok(resp) if resp.status().is_success() => {
            record(
                runs,
                run,
                &format!("Callback delivered ({})", resp.status()),
            )
            .await;
        }
        Ok(resp) => {
            record(runs, run, &format!("Callback returned {}", resp.status())).await;
        }
        Err(e) => {
            record(runs, run, &format!("Callback failed: {e}")).await;
        }
    }
}

/// Append a `delivery` timeline event (best-effort; swallows store errors).
async fn record(runs: &RunsRepo, run: &OttoRun, message: &str) {
    let _ = runs
        .add_event(NewRunEvent {
            run_id: run.id.clone(),
            workspace_id: run.workspace_id.clone(),
            kind: "delivery".to_string(),
            status: Some(run.status.as_str().to_string()),
            message: message.to_string(),
            detail: None,
        })
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use otto_core::run::{RunMode, RunOrigin, SourceKind};
    use otto_state::runs::NewRun;

    fn run_fixture(status: RunStatus) -> OttoRun {
        OttoRun {
            id: "run-1".to_string(),
            workspace_id: "ws-1".to_string(),
            title: "Fix the thing".to_string(),
            source_kind: SourceKind::Channel,
            source_ref: "thread:abc".to_string(),
            source_url: None,
            goal: "do it".to_string(),
            mode: RunMode::SingleAgent,
            provider: "claude".to_string(),
            repo_id: Some("repo-1".to_string()),
            repo_path: Some("/tmp/repo".to_string()),
            base_branch: Some("main".to_string()),
            branch: Some("otto-run/run-1".to_string()),
            worktree_path: Some("/tmp/wt".to_string()),
            base_commit: Some("abc123".to_string()),
            status,
            error: None,
            origin_kind: RunOrigin::Webhook,
            origin_chat: None,
            origin_thread: None,
            origin_user: None,
            callback_url: Some("https://example.test/cb".to_string()),
            goal_loop_id: None,
            review_id: None,
            proof_pack_id: Some("pp-1".to_string()),
            proof_status: Some("partial".to_string()),
            risk_score: Some(12),
            findings_total: 3,
            findings_blocking: 1,
            pr_draft_json: None,
            pr_url: None,
            auto_open_pr: false,
            approval_decision: None,
            approved_by: None,
            approved_at: None,
            result_summary: None,
            context_summary: None,
            created_by: "u-1".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn payload_marks_the_approval_gate() {
        let p = build_payload(&run_fixture(RunStatus::AwaitingApproval));
        assert_eq!(p["status"], "awaiting_approval");
        assert_eq!(p["awaiting_approval"], true);
        assert_eq!(p["terminal"], false);
        assert_eq!(p["run_id"], "run-1");
        assert_eq!(p["source_kind"], "channel");
        assert_eq!(p["findings_total"], 3);
        assert_eq!(p["findings_blocking"], 1);
        assert_eq!(p["has_pr_draft"], false);
        assert_eq!(p["proof_status"], "partial");
    }

    #[test]
    fn payload_marks_completed_with_draft() {
        let mut run = run_fixture(RunStatus::Completed);
        run.pr_draft_json = Some(r#"{"title":"x"}"#.to_string());
        run.approval_decision = Some("approved".to_string());
        let p = build_payload(&run);
        assert_eq!(p["status"], "completed");
        assert_eq!(p["terminal"], true);
        assert_eq!(p["awaiting_approval"], false);
        assert_eq!(p["has_pr_draft"], true);
        assert_eq!(p["approval_decision"], "approved");
    }

    #[test]
    fn payload_carries_error_for_failed_runs() {
        let mut run = run_fixture(RunStatus::Failed);
        run.error = Some("boom".to_string());
        let p = build_payload(&run);
        assert_eq!(p["status"], "failed");
        assert_eq!(p["terminal"], true);
        assert_eq!(p["error"], "boom");
    }

    // --- deliver() end-to-end against a real RunsRepo --------------------------

    async fn seed_repo() -> (RunsRepo, otto_core::Id) {
        let pool = otto_state::db::test_pool().await;
        let ws = otto_core::new_id();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'ws', '/tmp', ?)",
        )
        .bind(&ws)
        .bind(Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
        (RunsRepo::new(pool), ws)
    }

    fn new_run(ws: &str, callback_url: Option<String>) -> NewRun {
        NewRun {
            workspace_id: ws.to_string(),
            title: "t".into(),
            source_kind: SourceKind::Channel,
            source_ref: "x".into(),
            source_url: None,
            goal: "g".into(),
            mode: RunMode::SingleAgent,
            provider: "claude".into(),
            repo_id: None,
            origin_kind: RunOrigin::Webhook,
            origin_chat: None,
            origin_thread: None,
            origin_user: None,
            callback_url,
            auto_open_pr: false,
            context_summary: None,
            created_by: "root".into(),
        }
    }

    /// The SSRF guard refuses a loopback callback target, and `deliver` records
    /// that as a `delivery` timeline event — proving the path runs end to end and
    /// the guard is enforced (a key-holder can't turn Otto into an SSRF proxy).
    #[tokio::test]
    async fn deliver_records_ssrf_block_for_loopback_callback() {
        let (runs, ws) = seed_repo().await;
        let run = runs
            .create(new_run(&ws, Some("http://127.0.0.1:9/cb".into())))
            .await
            .unwrap();
        deliver(&runs, &run).await;
        let events = runs.list_events(&run.id).await.unwrap();
        let d = events
            .iter()
            .find(|e| e.kind == "delivery")
            .expect("a delivery event was recorded");
        assert!(
            d.message.contains("SSRF") || d.message.to_lowercase().contains("blocked"),
            "unexpected message: {}",
            d.message
        );
    }

    /// With no `callback_url`, `deliver` is a pure no-op — no timeline noise.
    #[tokio::test]
    async fn deliver_is_noop_without_callback_url() {
        let (runs, ws) = seed_repo().await;
        let run = runs.create(new_run(&ws, None)).await.unwrap();
        deliver(&runs, &run).await;
        let events = runs.list_events(&run.id).await.unwrap();
        assert!(
            !events.iter().any(|e| e.kind == "delivery"),
            "no delivery event expected without a callback_url"
        );
    }
}
