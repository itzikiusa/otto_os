//! The Workflows tab's status column (engine.md §10.1): each workflow's
//! latest run and how many of its runs are in flight, in one grouped query.

use crate::convert::{dberr, ts};
use crate::DbPool;
use otto_core::{Id, Result};
use serde::Serialize;
use sqlx::Row;

/// `last_run` of a `GET /workspaces/{wid}/workflows` row.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct LastRun {
    pub id: Id,
    pub status: String,
    /// `overview.state` when the stored one is fresh (its `rev` is the row's).
    pub overview_state: Option<String>,
    pub attention: bool,
    pub ended_at: Option<String>,
    pub duration_ms: Option<i64>,
}

/// Per workflow of `workspace_id`: `(workflow_id, latest run, running_count)`.
pub async fn last_runs(pool: &DbPool, workspace_id: &Id) -> Result<Vec<(Id, LastRun, u32)>> {
    let rows = sqlx::query(
        "SELECT * FROM (SELECT workflow_id, id, status, started_at, finished_at,
                CASE WHEN status = 'running' THEN attention ELSE 0 END AS attention,
                CASE WHEN json_extract(overview_json, '$.rev') = rev
                     THEN json_extract(overview_json, '$.state') END AS overview_state,
                ROW_NUMBER() OVER (PARTITION BY workflow_id ORDER BY started_at DESC, id DESC) AS rn,
                SUM(status IN ('pending', 'running')) OVER (PARTITION BY workflow_id) AS running
             FROM workflow_runs WHERE workspace_id = ?) WHERE rn = 1",
    )
    .bind(workspace_id)
    .fetch_all(pool)
    .await
    .map_err(dberr("last runs"))?;
    Ok(rows
        .iter()
        .map(|r| {
            let ended: Option<String> = r.get("finished_at");
            let started: String = r.get("started_at");
            let duration_ms = match (ts(&started), ended.as_deref().map(ts)) {
                (Ok(s), Some(Ok(e))) => Some((e - s).num_milliseconds()),
                _ => None,
            };
            let run = LastRun {
                id: r.get("id"),
                status: r.get("status"),
                overview_state: r.get("overview_state"),
                attention: r.get::<i64, _>("attention") != 0,
                ended_at: ended,
                duration_ms,
            };
            (r.get("workflow_id"), run, r.get::<i64, _>("running") as u32)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkflowsRepo;

    #[tokio::test]
    async fn latest_run_and_running_count_per_workflow() {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        let pool: DbPool = pool.into();
        let repo = WorkflowsRepo::new(pool.clone());
        let graph = Default::default();
        let (ws, u) = (Id::from("ws"), Id::from("u"));
        let a = repo.create(&ws, "A", "", "", &graph, &u).await.unwrap();
        let b = repo.create(&ws, "B", "", "", &graph, &u).await.unwrap();
        let mut ids = vec![];
        for (i, status) in ["success", "running", "pending"].iter().enumerate() {
            let run = repo
                .create_run(&a.id, &ws, &serde_json::json!({}), None)
                .await
                .unwrap();
            sqlx::query("UPDATE workflow_runs SET status = ?, started_at = ? WHERE id = ?")
                .bind(status)
                .bind(format!("2026-11-0{}T00:00:00Z", i + 1))
                .bind(&run.id)
                .execute(&pool)
                .await
                .unwrap();
            ids.push(run.id);
        }
        let done = repo
            .create_run(&b.id, &ws, &serde_json::json!({}), None)
            .await
            .unwrap();
        sqlx::query("UPDATE workflow_runs SET status = 'error', started_at = '2026-11-01T00:00:00Z', finished_at = '2026-11-01T00:01:30Z', rev = rev + 1, overview_json = json_object('state', 'failed', 'rev', rev + 1) WHERE id = ?")
            .bind(&done.id)
            .execute(&pool)
            .await
            .unwrap();
        let mut got = last_runs(&pool, &ws).await.unwrap();
        got.sort_by(|x, y| x.0.cmp(&y.0));
        let by = |id: &Id| got.iter().find(|g| &g.0 == id).unwrap().clone();
        let (_, last, running) = by(&a.id);
        assert_eq!(
            (last.id, last.status.as_str(), running),
            (ids[2].clone(), "pending", 2)
        );
        let (_, last, running) = by(&b.id);
        assert_eq!(running, 0);
        assert_eq!(
            (last.overview_state.as_deref(), last.duration_ms),
            (Some("failed"), Some(90_000))
        );
        // A stale overview (another rev: a status write, an older build's
        // write-back) and the attention of a run no longer running are not read.
        let c = repo.create(&ws, "C", "", "", &graph, &u).await.unwrap();
        let stale = repo
            .create_run(&c.id, &ws, &serde_json::json!({}), None)
            .await
            .unwrap();
        sqlx::query("UPDATE workflow_runs SET status = 'error', attention = 1, overview_json = json_object('state', 'needs_you', 'rev', rev - 1) WHERE id = ?")
            .bind(&stale.id)
            .execute(&pool)
            .await
            .unwrap();
        let got = last_runs(&pool, &ws).await.unwrap();
        let (_, last, _) = got.iter().find(|g| g.0 == c.id).unwrap();
        assert_eq!(
            (last.overview_state.as_deref(), last.attention),
            (None, false)
        );
        assert!(last_runs(&pool, &"other".into()).await.unwrap().is_empty());
    }
}
