//! Persistence for **Run with Otto** (migration `0085_run_with_otto.sql`).
//!
//! Two tables: `otto_runs` (one row per run; its `status` is the pipeline stage
//! machine [`otto_core::run::RunStatus`]) and `otto_run_events` (an append-only
//! timeline used for the audit trail and the Slack/feed mirror). Pure storage —
//! the stage logic lives in `otto-server::run_engine`. Status transitions go
//! through [`RunsRepo::set_status_cas`] (compare-and-set) so a late boot reaper or
//! a double-approve can never double-advance a run.

use chrono::Utc;
use otto_core::run::{OttoRun, RunEvent, RunMode, RunOrigin, RunStatus, SourceKind};
use otto_core::{new_id, Error, Id, Result};
use serde_json::Value;
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, json, ts};

#[derive(Clone)]
pub struct RunsRepo {
    pool: SqlitePool,
}

/// Fields for creating a run (status starts `queued`).
#[derive(Clone, Debug)]
pub struct NewRun {
    pub workspace_id: String,
    pub title: String,
    pub source_kind: SourceKind,
    pub source_ref: String,
    pub source_url: Option<String>,
    pub goal: String,
    pub mode: RunMode,
    pub provider: String,
    pub repo_id: Option<String>,
    pub origin_kind: RunOrigin,
    pub origin_chat: Option<String>,
    pub origin_thread: Option<String>,
    pub origin_user: Option<String>,
    pub callback_url: Option<String>,
    pub auto_open_pr: bool,
    pub context_summary: Option<String>,
    pub created_by: String,
}

/// Partial update of the server-set columns. Every `Some` field is written
/// (`None` leaves it unchanged via `COALESCE`). Runs only ever move forward, so
/// fields are never cleared back to NULL here.
#[derive(Clone, Debug, Default)]
pub struct RunPatch {
    pub title: Option<String>,
    pub goal: Option<String>,
    pub source_url: Option<String>,
    pub repo_id: Option<String>,
    pub repo_path: Option<String>,
    pub base_branch: Option<String>,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub base_commit: Option<String>,
    pub goal_loop_id: Option<String>,
    pub review_id: Option<String>,
    pub proof_pack_id: Option<String>,
    pub proof_status: Option<String>,
    pub risk_score: Option<i64>,
    pub findings_total: Option<i64>,
    pub findings_blocking: Option<i64>,
    pub pr_draft_json: Option<String>,
    pub pr_url: Option<String>,
    pub approval_decision: Option<String>,
    pub approved_by: Option<String>,
    pub approved_at: Option<String>,
    pub result_summary: Option<String>,
    pub context_summary: Option<String>,
}

/// Fields for appending a timeline event.
#[derive(Clone, Debug)]
pub struct NewRunEvent {
    pub run_id: String,
    pub workspace_id: String,
    pub kind: String,
    pub status: Option<String>,
    pub message: String,
    pub detail: Option<Value>,
}

fn row_to_run(r: &sqlx::sqlite::SqliteRow) -> Result<OttoRun> {
    let bad = |what: &str| Error::Internal(format!("otto_runs: unparsable {what}"));
    let source_kind: String = r.get("source_kind");
    let mode: String = r.get("mode");
    let origin_kind: String = r.get("origin_kind");
    let status: String = r.get("status");
    Ok(OttoRun {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        title: r.get("title"),
        source_kind: SourceKind::parse(&source_kind).ok_or_else(|| bad("source_kind"))?,
        source_ref: r.get("source_ref"),
        source_url: r.get("source_url"),
        goal: r.get("goal"),
        mode: RunMode::parse(&mode).unwrap_or_default(),
        provider: r.get("provider"),
        repo_id: r.get("repo_id"),
        repo_path: r.get("repo_path"),
        base_branch: r.get("base_branch"),
        branch: r.get("branch"),
        worktree_path: r.get("worktree_path"),
        base_commit: r.get("base_commit"),
        status: RunStatus::parse(&status).ok_or_else(|| bad("status"))?,
        error: r.get("error"),
        origin_kind: RunOrigin::parse(&origin_kind).ok_or_else(|| bad("origin_kind"))?,
        origin_chat: r.get("origin_chat"),
        origin_thread: r.get("origin_thread"),
        origin_user: r.get("origin_user"),
        callback_url: r.get("callback_url"),
        goal_loop_id: r.get("goal_loop_id"),
        review_id: r.get("review_id"),
        proof_pack_id: r.get("proof_pack_id"),
        proof_status: r.get("proof_status"),
        risk_score: r.get("risk_score"),
        findings_total: r.get("findings_total"),
        findings_blocking: r.get("findings_blocking"),
        pr_draft_json: r.get("pr_draft_json"),
        pr_url: r.get("pr_url"),
        auto_open_pr: r.get::<i64, _>("auto_open_pr") != 0,
        approval_decision: r.get("approval_decision"),
        approved_by: r.get("approved_by"),
        approved_at: r.get("approved_at"),
        result_summary: r.get("result_summary"),
        context_summary: r.get("context_summary"),
        created_by: r.get("created_by"),
        created_at: ts(r.get("created_at"))?,
        updated_at: ts(r.get("updated_at"))?,
    })
}

fn row_to_event(r: &sqlx::sqlite::SqliteRow) -> Result<RunEvent> {
    let detail_raw: Option<String> = r.get("detail_json");
    Ok(RunEvent {
        id: r.get("id"),
        run_id: r.get("run_id"),
        workspace_id: r.get("workspace_id"),
        kind: r.get("kind"),
        status: r.get("status"),
        message: r.get("message"),
        detail: detail_raw.and_then(|s| json(&s).ok()),
        created_at: ts(r.get("created_at"))?,
    })
}

/// The stage-machine statuses that are safe to re-drive after a daemon restart.
const RESUMABLE_STATUSES: &[&str] = &[
    "queued",
    "resolving_source",
    "building_context",
    "provisioning",
    "proving",
    "drafting_pr",
];
/// The statuses that spawned live background work now gone (must be failed on boot).
const INTERRUPTED_STATUSES: &[&str] = &["executing", "reviewing"];

impl RunsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, n: NewRun) -> Result<OttoRun> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO otto_runs (id, workspace_id, title, source_kind, source_ref, source_url, \
             goal, mode, provider, repo_id, status, origin_kind, origin_chat, origin_thread, \
             origin_user, callback_url, auto_open_pr, context_summary, created_by, created_at, updated_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&n.workspace_id)
        .bind(&n.title)
        .bind(n.source_kind.as_str())
        .bind(&n.source_ref)
        .bind(&n.source_url)
        .bind(&n.goal)
        .bind(n.mode.as_str())
        .bind(&n.provider)
        .bind(&n.repo_id)
        .bind(n.origin_kind.as_str())
        .bind(&n.origin_chat)
        .bind(&n.origin_thread)
        .bind(&n.origin_user)
        .bind(&n.callback_url)
        .bind(i64::from(n.auto_open_pr))
        .bind(&n.context_summary)
        .bind(&n.created_by)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("create run"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<OttoRun> {
        let row = sqlx::query("SELECT * FROM otto_runs WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("run not found"))?;
        row_to_run(&row)
    }

    pub async fn list_by_workspace(&self, ws: &Id, limit: i64) -> Result<Vec<OttoRun>> {
        let rows =
            sqlx::query("SELECT * FROM otto_runs WHERE workspace_id = ? ORDER BY updated_at DESC LIMIT ?")
                .bind(ws)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("list runs"))?;
        rows.iter().map(row_to_run).collect()
    }

    /// Compare-and-set the status. Returns `true` iff the run was in `from` (so the
    /// transition actually happened). The engine uses this to make every stage
    /// transition idempotent against a racing reaper / second approve.
    pub async fn set_status_cas(&self, id: &Id, from: RunStatus, to: RunStatus) -> Result<bool> {
        let now = fmt(Utc::now());
        let res = sqlx::query(
            "UPDATE otto_runs SET status = ?, updated_at = ? WHERE id = ? AND status = ?",
        )
        .bind(to.as_str())
        .bind(&now)
        .bind(id)
        .bind(from.as_str())
        .execute(&self.pool)
        .await
        .map_err(dberr("set run status (cas)"))?;
        Ok(res.rows_affected() == 1)
    }

    pub async fn set_status(&self, id: &Id, to: RunStatus) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query("UPDATE otto_runs SET status = ?, updated_at = ? WHERE id = ?")
            .bind(to.as_str())
            .bind(&now)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set run status"))?;
        Ok(())
    }

    pub async fn set_error(&self, id: &Id, err: &str) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE otto_runs SET status = 'failed', error = ?, updated_at = ? WHERE id = ?",
        )
        .bind(err)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("set run error"))?;
        Ok(())
    }

    /// Write the `Some` fields of a patch (others unchanged via `COALESCE`).
    pub async fn set_fields(&self, id: &Id, p: &RunPatch) -> Result<()> {
        let now = fmt(Utc::now());
        sqlx::query(
            "UPDATE otto_runs SET \
               title = COALESCE(?, title), \
               goal = COALESCE(?, goal), \
               source_url = COALESCE(?, source_url), \
               repo_id = COALESCE(?, repo_id), \
               repo_path = COALESCE(?, repo_path), \
               base_branch = COALESCE(?, base_branch), \
               branch = COALESCE(?, branch), \
               worktree_path = COALESCE(?, worktree_path), \
               base_commit = COALESCE(?, base_commit), \
               goal_loop_id = COALESCE(?, goal_loop_id), \
               review_id = COALESCE(?, review_id), \
               proof_pack_id = COALESCE(?, proof_pack_id), \
               proof_status = COALESCE(?, proof_status), \
               risk_score = COALESCE(?, risk_score), \
               findings_total = COALESCE(?, findings_total), \
               findings_blocking = COALESCE(?, findings_blocking), \
               pr_draft_json = COALESCE(?, pr_draft_json), \
               pr_url = COALESCE(?, pr_url), \
               approval_decision = COALESCE(?, approval_decision), \
               approved_by = COALESCE(?, approved_by), \
               approved_at = COALESCE(?, approved_at), \
               result_summary = COALESCE(?, result_summary), \
               context_summary = COALESCE(?, context_summary), \
               updated_at = ? \
             WHERE id = ?",
        )
        .bind(&p.title)
        .bind(&p.goal)
        .bind(&p.source_url)
        .bind(&p.repo_id)
        .bind(&p.repo_path)
        .bind(&p.base_branch)
        .bind(&p.branch)
        .bind(&p.worktree_path)
        .bind(&p.base_commit)
        .bind(&p.goal_loop_id)
        .bind(&p.review_id)
        .bind(&p.proof_pack_id)
        .bind(&p.proof_status)
        .bind(p.risk_score)
        .bind(p.findings_total)
        .bind(p.findings_blocking)
        .bind(&p.pr_draft_json)
        .bind(&p.pr_url)
        .bind(&p.approval_decision)
        .bind(&p.approved_by)
        .bind(&p.approved_at)
        .bind(&p.result_summary)
        .bind(&p.context_summary)
        .bind(&now)
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("update run fields"))?;
        Ok(())
    }

    pub async fn add_event(&self, e: NewRunEvent) -> Result<RunEvent> {
        let id = new_id();
        let now = fmt(Utc::now());
        let detail_raw = e.detail.as_ref().map(|v| v.to_string());
        sqlx::query(
            "INSERT INTO otto_run_events (id, run_id, workspace_id, kind, status, message, detail_json, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&e.run_id)
        .bind(&e.workspace_id)
        .bind(&e.kind)
        .bind(&e.status)
        .bind(&e.message)
        .bind(&detail_raw)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("add run event"))?;
        let row = sqlx::query("SELECT * FROM otto_run_events WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("run event not found"))?;
        row_to_event(&row)
    }

    pub async fn list_events(&self, run_id: &Id) -> Result<Vec<RunEvent>> {
        let rows = sqlx::query(
            "SELECT * FROM otto_run_events WHERE run_id = ? ORDER BY created_at ASC, id ASC",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list run events"))?;
        rows.iter().map(row_to_event).collect()
    }

    /// The single run currently `awaiting_approval` bound to `(ws, chat, thread)`,
    /// newest first — what a Slack/Telegram `approve`/`reject` reply resolves.
    pub async fn find_awaiting_for_thread(
        &self,
        ws: &Id,
        chat: &str,
        thread: Option<&str>,
    ) -> Result<Option<OttoRun>> {
        let row = match thread {
            Some(t) => sqlx::query(
                "SELECT * FROM otto_runs WHERE workspace_id = ? AND origin_chat = ? \
                 AND origin_thread = ? AND status = 'awaiting_approval' \
                 ORDER BY updated_at DESC LIMIT 1",
            )
            .bind(ws)
            .bind(chat)
            .bind(t)
            .fetch_optional(&self.pool)
            .await,
            None => sqlx::query(
                "SELECT * FROM otto_runs WHERE workspace_id = ? AND origin_chat = ? \
                 AND origin_thread IS NULL AND status = 'awaiting_approval' \
                 ORDER BY updated_at DESC LIMIT 1",
            )
            .bind(ws)
            .bind(chat)
            .fetch_optional(&self.pool)
            .await,
        }
        .map_err(dberr("find awaiting run"))?;
        row.as_ref().map(row_to_run).transpose()
    }

    /// Active runs whose stage is safe to re-drive on boot.
    pub async fn list_resumable(&self) -> Result<Vec<OttoRun>> {
        self.list_by_statuses(RESUMABLE_STATUSES).await
    }

    /// Active runs interrupted mid live-work (must be failed on boot).
    pub async fn list_interrupted(&self) -> Result<Vec<OttoRun>> {
        self.list_by_statuses(INTERRUPTED_STATUSES).await
    }

    async fn list_by_statuses(&self, statuses: &[&str]) -> Result<Vec<OttoRun>> {
        let placeholders = vec!["?"; statuses.len()].join(",");
        let sql = format!("SELECT * FROM otto_runs WHERE status IN ({placeholders})");
        let mut q = sqlx::query(&sql);
        for s in statuses {
            q = q.bind(*s);
        }
        let rows = q
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list runs by status"))?;
        rows.iter().map(row_to_run).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_pool() -> SqlitePool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool
    }

    async fn seed_ws(pool: &SqlitePool) -> Id {
        let ws = new_id();
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'ws', '/tmp', ?)")
            .bind(&ws)
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
        ws
    }

    fn sample(ws: &str) -> NewRun {
        NewRun {
            workspace_id: ws.to_string(),
            title: "Fix login".into(),
            source_kind: SourceKind::Finding,
            source_ref: "f1".into(),
            source_url: None,
            goal: "Fix the finding".into(),
            mode: RunMode::SingleAgent,
            provider: "claude".into(),
            repo_id: None,
            origin_kind: RunOrigin::Slack,
            origin_chat: Some("C1".into()),
            origin_thread: Some("T1".into()),
            origin_user: Some("U1".into()),
            callback_url: None,
            auto_open_pr: false,
            context_summary: None,
            created_by: "root".into(),
        }
    }

    #[tokio::test]
    async fn create_get_roundtrip() {
        let pool = mem_pool().await;
        let ws = seed_ws(&pool).await;
        let repo = RunsRepo::new(pool);
        let run = repo.create(sample(&ws)).await.unwrap();
        assert_eq!(run.status, RunStatus::Queued);
        assert_eq!(run.source_kind, SourceKind::Finding);
        assert_eq!(run.mode, RunMode::SingleAgent);
        let got = repo.get(&run.id).await.unwrap();
        assert_eq!(got.id, run.id);
        assert_eq!(got.title, "Fix login");
        assert_eq!(got.origin_chat.as_deref(), Some("C1"));
    }

    #[tokio::test]
    async fn cas_blocks_stale_transition() {
        let pool = mem_pool().await;
        let ws = seed_ws(&pool).await;
        let repo = RunsRepo::new(pool);
        let run = repo.create(sample(&ws)).await.unwrap();
        // Correct precondition advances.
        assert!(repo
            .set_status_cas(&run.id, RunStatus::Queued, RunStatus::ResolvingSource)
            .await
            .unwrap());
        // Stale precondition (still claims Queued) does NOT.
        assert!(!repo
            .set_status_cas(&run.id, RunStatus::Queued, RunStatus::BuildingContext)
            .await
            .unwrap());
        assert_eq!(
            repo.get(&run.id).await.unwrap().status,
            RunStatus::ResolvingSource
        );
    }

    #[tokio::test]
    async fn set_fields_and_events() {
        let pool = mem_pool().await;
        let ws = seed_ws(&pool).await;
        let repo = RunsRepo::new(pool);
        let run = repo.create(sample(&ws)).await.unwrap();
        repo.set_fields(
            &run.id,
            &RunPatch {
                branch: Some("otto-run/x".into()),
                risk_score: Some(42),
                findings_total: Some(3),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let got = repo.get(&run.id).await.unwrap();
        assert_eq!(got.branch.as_deref(), Some("otto-run/x"));
        assert_eq!(got.risk_score, Some(42));
        assert_eq!(got.findings_total, 3);
        // title untouched (None in patch)
        assert_eq!(got.title, "Fix login");

        repo.add_event(NewRunEvent {
            run_id: run.id.clone(),
            workspace_id: ws.clone(),
            kind: "stage_ok".into(),
            status: Some("provisioning".into()),
            message: "worktree ready".into(),
            detail: None,
        })
        .await
        .unwrap();
        let evs = repo.list_events(&run.id).await.unwrap();
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].message, "worktree ready");
    }

    #[tokio::test]
    async fn find_awaiting_only_matches_awaiting() {
        let pool = mem_pool().await;
        let ws = seed_ws(&pool).await;
        let repo = RunsRepo::new(pool);
        let run = repo.create(sample(&ws)).await.unwrap();
        // Not yet awaiting → no match.
        assert!(repo
            .find_awaiting_for_thread(&ws, "C1", Some("T1"))
            .await
            .unwrap()
            .is_none());
        repo.set_status(&run.id, RunStatus::AwaitingApproval)
            .await
            .unwrap();
        let found = repo
            .find_awaiting_for_thread(&ws, "C1", Some("T1"))
            .await
            .unwrap();
        assert_eq!(found.map(|r| r.id), Some(run.id.clone()));
        // Wrong thread → no match.
        assert!(repo
            .find_awaiting_for_thread(&ws, "C1", Some("OTHER"))
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn resumable_vs_interrupted_partition() {
        let pool = mem_pool().await;
        let ws = seed_ws(&pool).await;
        let repo = RunsRepo::new(pool);
        let a = repo.create(sample(&ws)).await.unwrap(); // queued → resumable
        let b = repo.create(sample(&ws)).await.unwrap();
        repo.set_status(&b.id, RunStatus::Executing).await.unwrap(); // interrupted
        let c = repo.create(sample(&ws)).await.unwrap();
        repo.set_status(&c.id, RunStatus::Completed).await.unwrap(); // terminal → neither
        let resumable: Vec<_> = repo
            .list_resumable()
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        let interrupted: Vec<_> = repo
            .list_interrupted()
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect();
        assert!(resumable.contains(&a.id));
        assert!(!resumable.contains(&b.id));
        assert!(interrupted.contains(&b.id));
        assert!(!interrupted.contains(&c.id));
        assert!(!resumable.contains(&c.id));
    }
}
