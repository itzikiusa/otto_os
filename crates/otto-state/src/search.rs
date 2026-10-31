//! Narrow metadata projections for the command palette's cross-module search.
//! Stream candidates rather than materializing full stories/workflow graphs or
//! one project/task list per swarm. Substring matching stays in Rust to retain
//! Unicode lowercasing (SQLite's built-in lower only folds ASCII). A no-match
//! search remains O(N), but retained matches are bounded independently of N.
use crate::{convert::dberr, DbPool};
use futures_util::TryStreamExt;
use otto_core::{domain::Feature, Result};
use sqlx::Row;

#[derive(Clone, Copy)]
pub enum Source {
    Story,
    Workflow,
    ApiRequest,
    Repo,
    BrokerCluster,
    Canvas,
}
impl Source {
    pub const ALL: [Self; 6] = [
        Self::Story,
        Self::Workflow,
        Self::ApiRequest,
        Self::Repo,
        Self::BrokerCluster,
        Self::Canvas,
    ];
    pub fn kind(self) -> &'static str {
        match self {
            Self::Story => "story",
            Self::Workflow => "workflow",
            Self::ApiRequest => "api_request",
            Self::Repo => "repo",
            Self::BrokerCluster => "broker_cluster",
            Self::Canvas => "canvas",
        }
    }
    pub fn feature(self) -> Feature {
        match self {
            Self::Story => Feature::Product,
            Self::Workflow => Feature::Workflows,
            Self::ApiRequest => Feature::ApiClient,
            Self::Repo => Feature::Git,
            Self::BrokerCluster => Feature::Database,
            Self::Canvas => Feature::Canvas,
        }
    }
    fn sql(self) -> &'static str {
        match self {
        // Product is intentionally a global library; workspace is provenance.
        Self::Story => "SELECT id,title,source_key AS secondary,source_key AS subtitle,updated_at FROM product_stories WHERE ?1 IS NOT NULL ORDER BY created_at DESC",
        Self::Workflow => "SELECT id,name AS title,description AS secondary,NULLIF(description,'') AS subtitle,updated_at FROM workflows WHERE workspace_id=?1 ORDER BY updated_at DESC",
        Self::ApiRequest => "SELECT id,name AS title,method||' '||url AS secondary,method||' '||url AS subtitle,updated_at FROM api_requests WHERE workspace_id=?1 ORDER BY position,name",
        Self::Repo => "SELECT id,name AS title,remote_url AS secondary,remote_url AS subtitle,created_at AS updated_at FROM repos WHERE workspace_id=?1 ORDER BY name",
        Self::BrokerCluster => "SELECT id,name AS title,bootstrap_servers AS secondary,bootstrap_servers AS subtitle,created_at AS updated_at FROM broker_clusters WHERE workspace_id=?1 OR workspace_id IS NULL ORDER BY name",
        Self::Canvas => "SELECT id,title,section AS secondary,section AS subtitle,updated_at FROM canvas_scenes WHERE workspace_id=?1 ORDER BY updated_at DESC",
    }
    }
}

pub struct SearchRow {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub updated_at: String,
    pub score: i32,
}

/// Caller must authorize `source.feature()` before invoking this read. Each
/// query enforces the source's existing workspace/global visibility semantics.
pub async fn search(
    pool: &DbPool,
    source: Source,
    workspace: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<SearchRow>> {
    let limit = limit.min(100);
    if limit == 0 {
        return Ok(vec![]);
    }
    let query = query.to_lowercase();
    let mut stream = sqlx::query(source.sql()).bind(workspace).fetch(pool);
    let mut out = Vec::with_capacity(limit);
    while let Some(row) = stream.try_next().await.map_err(dberr("search metadata"))? {
        let title: String = row.get("title");
        let secondary: Option<String> = row.get("secondary");
        let title_match = title.to_lowercase().contains(&query);
        if !title_match
            && !secondary
                .as_deref()
                .unwrap_or_default()
                .to_lowercase()
                .contains(&query)
        {
            continue;
        }
        out.push(SearchRow {
            id: row.get("id"),
            title,
            subtitle: row.get("subtitle"),
            updated_at: row.get("updated_at"),
            score: if title_match { 2 } else { 1 },
        });
        if out.len() == limit {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn every_projection_is_valid_and_empty_for_a_new_database() {
        let pool = crate::db::test_pool().await;
        for source in Source::ALL {
            assert!(
                search(&pool, source, "empty", "needle", 5)
                    .await
                    .unwrap()
                    .is_empty(),
                "{}",
                source.kind()
            );
        }
    }
    #[tokio::test]
    async fn bounded_unicode_search_reaches_matches_after_many_nonmatches() {
        let pool = crate::db::test_pool().await;
        sqlx::query("INSERT INTO workspaces(id,name,root_path,created_at) VALUES('w','w','/tmp','2026-01-01T00:00:00Z'),('other','other','/tmp','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        for i in 0..10_000 {
            let title = if i >= 9980 {
                format!("ÉCOLE {i}")
            } else {
                format!("ordinary {i}")
            };
            sqlx::query("INSERT INTO repos(id,workspace_id,name,path,created_at) VALUES(?,?,?,?, '2026-01-01T00:00:00Z')")
                .bind(format!("r{i}")).bind(if i == 9999 { "other" } else { "w" }).bind(title).bind(format!("/tmp/fixture-r{i}")).execute(&mut *tx).await.unwrap();
        }
        tx.commit().await.unwrap();
        let start = std::time::Instant::now();
        let rows = search(&pool, Source::Repo, "w", "école", 5).await.unwrap();
        println!("10k rows Unicode late-match search: {:?}", start.elapsed());
        assert_eq!(rows.len(), 5);
        assert!(rows.iter().all(|r| r.title.starts_with("ÉCOLE")));
        assert!(search(&pool, Source::Repo, "w", "9999", 5)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(
            search(&pool, Source::Repo, "other", "école", 5)
                .await
                .unwrap()
                .len(),
            1
        );
        assert!(search(&pool, Source::Repo, "w", "", 0)
            .await
            .unwrap()
            .is_empty());
    }

    /// The per-keystroke canvas / API-request projections are answered from a
    /// covering index (0188) — they never read the multi-MB table rows.
    #[tokio::test]
    async fn heavy_row_sources_use_a_covering_index() {
        let pool = crate::db::test_pool().await;
        for (source, index) in [
            (Source::Canvas, "idx_canvas_scenes_search"),
            (Source::ApiRequest, "idx_api_requests_search"),
        ] {
            let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
                "EXPLAIN QUERY PLAN {}",
                source.sql()
            )))
            .bind("w")
            .fetch_all(&pool)
            .await
            .unwrap();
            let plan = rows
                .iter()
                .map(|r| r.get::<String, _>("detail"))
                .collect::<Vec<_>>()
                .join(" | ");
            assert!(
                plan.contains(&format!("COVERING INDEX {index}")),
                "{}: {plan}",
                source.kind()
            );
        }
    }
}
