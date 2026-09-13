//! Bounded metadata candidates for the additive history page API.
use crate::convert::dberr;
use crate::TranscriptIndexRow;
use otto_core::{domain::Session, Result};
use sqlx::{Row, SqlitePool};

#[derive(Clone, Debug)]
pub struct Candidate {
    pub key: String,
    pub at: String,
    pub session: Option<Session>,
    pub indexed: Option<TranscriptIndexRow>,
    pub persisted_path: Option<String>,
}
#[derive(Default)]
pub struct PageRequest<'a> {
    pub workspace: &'a str,
    pub owner: &'a str,
    pub admin: bool,
    pub status: Option<&'a str>,
    pub cwd: Option<&'a str>,
    pub before: Option<(&'a str, &'a str)>,
    pub budget: usize,
}
pub async fn candidates(pool: &SqlitePool, req: PageRequest<'_>) -> Result<Vec<Candidate>> {
    let limit = req.budget.clamp(1, 4000) + 1;
    let (before_at, before_key) = req
        .before
        .map(|(a, k)| (Some(a), Some(k)))
        .unwrap_or_default();
    let cwd_prefix = req.cwd.map(|cwd| format!("{}/", cwd.trim_end_matches('/')));
    let rows = sqlx::query(
        "SELECT * FROM sessions WHERE workspace_id = ? AND kind = 'agent'
        AND (? OR created_by = ?) AND (? IS NULL OR status = ?)
        AND (? IS NULL OR cwd = ? OR instr(cwd, ?) = 1)
        AND (? IS NULL OR last_active_at < ? OR (last_active_at = ? AND id > ?))
        ORDER BY last_active_at DESC, id ASC LIMIT ?",
    )
    .bind(req.workspace)
    .bind(req.admin)
    .bind(req.owner)
    .bind(req.status)
    .bind(req.status)
    .bind(req.cwd)
    .bind(req.cwd)
    .bind(&cwd_prefix)
    .bind(before_at)
    .bind(before_at)
    .bind(before_at)
    .bind(before_key)
    .bind(limit as i64)
    .fetch_all(pool)
    .await
    .map_err(dberr("history candidates"))?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let session = crate::sessions::row_to_session(&row)?;
        out.push(Candidate {
            key: session.id.clone(),
            at: row.get("last_active_at"),
            session: Some(session),
            indexed: None,
            persisted_path: row.get("transcript_path"),
        });
    }
    if req.admin && req.status.is_none_or(|status| status == "on_disk") {
        let rows = sqlx::query("SELECT t.* FROM transcript_index t
            WHERE NOT EXISTS (SELECT 1 FROM sessions s WHERE s.transcript_path = t.path)
            AND (t.provider_session_id IS NULL OR NOT EXISTS (SELECT 1 FROM sessions s WHERE s.provider_session_id = t.provider_session_id))
            AND (? IS NULL OR COALESCE(t.cwd, '') = ? OR instr(COALESCE(t.cwd, ''), ?) = 1)
            AND (? IS NULL OR COALESCE(t.last_active_at,t.started_at,'') < ?
                OR (COALESCE(t.last_active_at,t.started_at,'') = ? AND 'path:' || t.path > ?))
            ORDER BY COALESCE(t.last_active_at,t.started_at,'') DESC, t.path ASC LIMIT ?")
            .bind(req.cwd).bind(req.cwd).bind(&cwd_prefix)
            .bind(before_at).bind(before_at).bind(before_at).bind(before_key).bind(limit as i64)
            .fetch_all(pool).await.map_err(dberr("history indexed candidates"))?;
        for row in rows {
            let indexed = crate::transcript_index::row(&row);
            out.push(Candidate {
                key: format!("path:{}", indexed.path),
                at: indexed
                    .last_active_at
                    .clone()
                    .or_else(|| indexed.started_at.clone())
                    .unwrap_or_default(),
                session: None,
                persisted_path: None,
                indexed: Some(indexed),
            });
        }
    }
    out.sort_by(|a, b| b.at.cmp(&a.at).then_with(|| a.key.cmp(&b.key)));
    out.truncate(limit);
    Ok(out)
}

/// One bounded batch of exact paths after filesystem resolution. At most20
/// parameter-safe queries for the maximum4000-candidate page, never N queries.
pub async fn indexed_paths(pool: &SqlitePool, paths: &[String]) -> Result<Vec<TranscriptIndexRow>> {
    let mut out = Vec::new();
    for chunk in paths.chunks(200) {
        let mut query = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT * FROM transcript_index WHERE path IN (",
        );
        let mut separated = query.separated(",");
        for path in chunk {
            separated.push_bind(path);
        }
        separated.push_unseparated(")");
        let rows = query
            .build()
            .fetch_all(pool)
            .await
            .map_err(dberr("history path metadata"))?;
        out.extend(rows.iter().map(crate::transcript_index::row));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
    async fn pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::new().in_memory(true))
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        sqlx::query("INSERT INTO users(id,username,password_hash,created_at) VALUES ('owner','owner','','2026-01-01T00:00:00Z'),('other','other','','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO workspaces(id,name,root_path,created_at) VALUES ('ws','ws','/tmp','2026-01-01T00:00:00Z')").execute(&pool).await.unwrap();
        pool
    }
    async fn seed(pool: &SqlitePool, count: usize) {
        let mut tx = pool.begin().await.unwrap();
        for n in 0..count {
            sqlx::query("INSERT INTO sessions(id,workspace_id,kind,provider,title,status,cwd,created_by,created_at,last_active_at) VALUES (?,'ws','agent','claude',?,'exited','/repo',?,'2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')")
                .bind(format!("s{n:05}")).bind(format!("title{n}")).bind(if n % 2 == 0 {"owner"} else {"other"}).execute(&mut *tx).await.unwrap();
        }
        tx.commit().await.unwrap();
    }
    #[tokio::test]
    async fn metadata_candidates_are_bounded_before_resolution_and_owner_scoped() {
        let pool = pool().await;
        seed(&pool, 1000).await;
        let rows = candidates(
            &pool,
            PageRequest {
                workspace: "ws",
                owner: "owner",
                budget: 400,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(
            rows.len() <= 401,
            "at most candidate budget plus one metadata lookahead"
        );
        assert!(rows
            .iter()
            .all(|r| r.session.as_ref().unwrap().created_by == "owner"));
    }
    #[tokio::test]
    async fn equal_timestamp_pages_are_lossless() {
        let pool = pool().await;
        seed(&pool, 9).await;
        let mut all = Vec::new();
        let mut cursor: Option<(String, String)> = None;
        loop {
            let mut rows = candidates(
                &pool,
                PageRequest {
                    workspace: "ws",
                    owner: "owner",
                    admin: true,
                    budget: 2,
                    before: cursor.as_ref().map(|(a, k)| (a.as_str(), k.as_str())),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
            let more = rows.len() > 2;
            rows.truncate(2);
            if let Some(last) = rows.last() {
                cursor = Some((last.at.clone(), last.key.clone()));
            }
            all.extend(rows.into_iter().map(|r| r.key));
            if !more || all.len() > 12 {
                break;
            }
        }
        assert_eq!(all, (0..9).map(|n| format!("s{n:05}")).collect::<Vec<_>>());
    }
    #[tokio::test]
    async fn disk_claims_span_providers_and_nonadmins_never_see_unclaimed_files() {
        let pool = pool().await;
        seed(&pool, 2).await;
        sqlx::query("UPDATE sessions SET provider='codex',provider_session_id='shared',transcript_path='/claimed' WHERE id='s00000'").execute(&pool).await.unwrap();
        for (path, psid) in [
            ("/different-provider", "shared"),
            ("/claimed", "other"),
            ("/free", "free"),
        ] {
            sqlx::query("INSERT INTO transcript_index(path,provider,provider_session_id,cwd,mtime,size,indexed_at,last_active_at) VALUES (?,'claude',?,'/repo',0,0,'now','2026-01-01T00:00:00Z')").bind(path).bind(psid).execute(&pool).await.unwrap();
        }
        let rows = candidates(
            &pool,
            PageRequest {
                workspace: "ws",
                owner: "owner",
                admin: true,
                budget: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            rows.iter()
                .filter_map(|r| r.indexed.as_ref().map(|i| i.path.as_str()))
                .collect::<Vec<_>>(),
            vec!["/free"]
        );
        let rows = candidates(
            &pool,
            PageRequest {
                workspace: "ws",
                owner: "owner",
                budget: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(rows.iter().all(|r| r.indexed.is_none()));
    }
    #[tokio::test]
    async fn cwd_filter_is_literal_and_does_not_match_siblings() {
        let pool = pool().await;
        seed(&pool, 4).await;
        for (id, cwd) in [
            ("s00000", "/repo_%"),
            ("s00001", "/repo_%/child"),
            ("s00002", "/repo_%sibling"),
            ("s00003", "/repoABC"),
        ] {
            sqlx::query("UPDATE sessions SET cwd=? WHERE id=?")
                .bind(cwd)
                .bind(id)
                .execute(&pool)
                .await
                .unwrap();
        }
        let rows = candidates(
            &pool,
            PageRequest {
                workspace: "ws",
                owner: "owner",
                admin: true,
                cwd: Some("/repo_%"),
                budget: 10,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(
            rows.into_iter().map(|r| r.key).collect::<Vec<_>>(),
            vec!["s00000", "s00001"]
        );
    }
}
