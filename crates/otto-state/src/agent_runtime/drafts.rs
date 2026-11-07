//! Bounded, revision-checked draft storage. Expiry removes bytes but retains
//! revision tombstones until the thread is deleted.
use super::*;
use otto_core::runtime_drafts::{RuntimeDraftContent, RuntimeDraftRecord};

const MAX_DRAFT_BYTES: usize = 24 * 1024 * 1024;
const MAX_SCOPE_BYTES: i64 = 64 * 1024 * 1024;
const RETENTION_SECONDS: i64 = 30 * 24 * 60 * 60;

impl AgentRuntimeRepo {
    pub async fn expire_drafts(&self) -> Result<()> {
        sqlx::query("UPDATE runtime_composer_drafts SET payload_json = NULL, revision = revision + 1 WHERE payload_json IS NOT NULL AND updated_at < ?")
            .bind(Utc::now().timestamp() - RETENTION_SECONDS)
            .execute(&self.pool).await.map_err(dberr)?;
        Ok(())
    }
    pub async fn composer_draft(
        &self,
        user: &Id,
        workspace: &Id,
        session: &Id,
    ) -> Result<RuntimeDraftRecord> {
        self.expire_drafts().await?;
        let row = sqlx::query("SELECT revision, payload_json FROM runtime_composer_drafts WHERE user_id=? AND workspace_id=? AND session_id=?")
            .bind(user).bind(workspace).bind(session).fetch_optional(&self.pool).await.map_err(dberr)?;
        let Some(row) = row else {
            return Ok(RuntimeDraftRecord {
                revision: 0,
                draft: None,
            });
        };
        let payload: Option<String> = row.get("payload_json");
        Ok(RuntimeDraftRecord {
            revision: row.get("revision"),
            draft: payload
                .map(|v| serde_json::from_str(&v))
                .transpose()
                .map_err(|e| Error::Internal(e.to_string()))?,
        })
    }
    pub async fn save_composer_draft(
        &self,
        user: &Id,
        workspace: &Id,
        session: &Id,
        revision: i64,
        draft: &RuntimeDraftContent,
    ) -> Result<RuntimeDraftRecord> {
        let json = serde_json::to_string(draft).map_err(|e| Error::Internal(e.to_string()))?;
        if json.len() > MAX_DRAFT_BYTES {
            return Err(Error::PayloadTooLarge(
                "Draft exceeds 24 MiB. Remove attachments and retry.".into(),
            ));
        }
        if revision < 0 || revision == i64::MAX {
            return Err(Error::Invalid("invalid draft revision".into()));
        }
        self.expire_drafts().await?;
        let has_content = !draft.text.is_empty()
            || !draft.images.is_empty()
            || !draft.context.is_empty()
            || draft.fresh_context;
        let has_preferences =
            draft.model.is_some() || draft.effort.is_some() || draft.access_mode.is_some();
        let payload = (has_content || has_preferences).then_some(json.as_str());
        let mut tx = self.pool.begin().await.map_err(dberr)?;
        // The first write acquires SQLite's writer lock before reading quota/revision.
        sqlx::query("INSERT OR IGNORE INTO runtime_composer_drafts (user_id,workspace_id,session_id,revision,payload_json,updated_at) VALUES (?,?,?,0,NULL,?)")
            .bind(user).bind(workspace).bind(session).bind(Utc::now().timestamp()).execute(&mut *tx).await.map_err(dberr)?;
        let used: i64 = sqlx::query_scalar("SELECT COALESCE(SUM(length(CAST(payload_json AS BLOB))),0) FROM runtime_composer_drafts WHERE user_id=? AND workspace_id=? AND session_id != ?")
            .bind(user).bind(workspace).bind(session).fetch_one(&mut *tx).await.map_err(dberr)?;
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_composer_drafts WHERE user_id=? AND workspace_id=? AND payload_json IS NOT NULL AND (json_extract(payload_json,'$.text') != '' OR json_array_length(payload_json,'$.images') > 0 OR json_array_length(payload_json,'$.context') > 0 OR json_extract(payload_json,'$.fresh_context') = 1) AND session_id != ?")
            .bind(user).bind(workspace).bind(session).fetch_one(&mut *tx).await.map_err(dberr)?;
        if has_content && (used + json.len() as i64 > MAX_SCOPE_BYTES || count >= 128) {
            return Err(Error::PayloadTooLarge(
                "Draft storage is full (64 MiB / 128 drafts). Clear an older draft and retry."
                    .into(),
            ));
        }
        let changed = sqlx::query("UPDATE runtime_composer_drafts SET revision=revision+1,payload_json=?,updated_at=? WHERE user_id=? AND workspace_id=? AND session_id=? AND revision=?")
            .bind(payload).bind(Utc::now().timestamp()).bind(user).bind(workspace).bind(session).bind(revision)
            .execute(&mut *tx).await.map_err(dberr)?.rows_affected();
        if changed != 1 {
            return Err(Error::Conflict(
                "Draft changed in another window. Reload its revision before saving.".into(),
            ));
        }
        tx.commit().await.map_err(dberr)?;
        Ok(RuntimeDraftRecord {
            revision: revision + 1,
            draft: payload.map(|_| draft.clone()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    async fn seeded() -> (AgentRuntimeRepo, DbPool) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(
                sqlx::sqlite::SqliteConnectOptions::new()
                    .in_memory(true)
                    .foreign_keys(true),
            )
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        let pool: DbPool = pool.into();
        for (id, username) in [("u1", "one"), ("u2", "two")] {
            sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES (?,?,'x','U',0,'2026-11-07T00:00:00Z')")
                .bind(id).bind(username).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO workspaces(id,name,root_path,created_at) VALUES ('w1','W','/tmp','2026-11-07T00:00:00Z')").execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO agent_runtime_sessions(id,workspace_id,provider,title,cwd,created_by,created_at,last_active_at) VALUES ('r1','w1','claude','T','/tmp','u1','2026-11-07T00:00:00Z','2026-11-07T00:00:00Z')").execute(&pool).await.unwrap();
        (AgentRuntimeRepo::new(pool.clone()), pool)
    }
    #[tokio::test]
    async fn draft_revision_owner_expiry_and_thread_cascade() {
        let (repo, pool) = seeded().await;
        let (u, w, r) = ("u1".into(), "w1".into(), "r1".into());
        let value = RuntimeDraftContent {
            text: "private unsent".into(),
            ..Default::default()
        };
        assert_eq!(
            repo.save_composer_draft(&u, &w, &r, 0, &value)
                .await
                .unwrap()
                .revision,
            1
        );
        assert!(matches!(
            repo.save_composer_draft(&u, &w, &r, 0, &value).await,
            Err(Error::Conflict(_))
        ));
        assert!(repo
            .composer_draft(&"u2".into(), &w, &r)
            .await
            .unwrap()
            .draft
            .is_none());
        let recovered = AgentRuntimeRepo::new(pool.clone())
            .composer_draft(&u, &w, &r)
            .await
            .unwrap();
        assert_eq!(recovered.draft.unwrap().text, "private unsent");
        sqlx::query("UPDATE runtime_composer_drafts SET updated_at=0")
            .execute(&pool)
            .await
            .unwrap();
        let expired = repo.composer_draft(&u, &w, &r).await.unwrap();
        assert_eq!(expired.revision, 2);
        assert!(expired.draft.is_none());
        assert!(matches!(
            repo.save_composer_draft(&u, &w, &r, 1, &value).await,
            Err(Error::Conflict(_))
        ));
        repo.save_composer_draft(&u, &w, &r, 2, &value)
            .await
            .unwrap();
        repo.delete(&r).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_composer_drafts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    #[tokio::test]
    async fn clear_is_a_revision_tombstone_and_choices_survive_without_content() {
        let (repo, _) = seeded().await;
        let (u, w, r) = ("u1".into(), "w1".into(), "r1".into());
        let value = RuntimeDraftContent {
            text: "sent".into(),
            ..Default::default()
        };
        repo.save_composer_draft(&u, &w, &r, 0, &value)
            .await
            .unwrap();
        let cleared = repo
            .save_composer_draft(&u, &w, &r, 1, &RuntimeDraftContent::default())
            .await
            .unwrap();
        assert_eq!(cleared.revision, 2);
        assert!(cleared.draft.is_none());
        let choices = RuntimeDraftContent {
            model: Some("picked".into()),
            ..Default::default()
        };
        repo.save_composer_draft(&u, &w, &r, 2, &choices)
            .await
            .unwrap();
        assert_eq!(
            repo.composer_draft(&u, &w, &r)
                .await
                .unwrap()
                .draft
                .unwrap()
                .model
                .as_deref(),
            Some("picked")
        );
        let oversized = RuntimeDraftContent {
            text: "x".repeat(MAX_DRAFT_BYTES),
            ..Default::default()
        };
        assert!(matches!(
            repo.save_composer_draft(&u, &w, &r, 3, &oversized).await,
            Err(Error::PayloadTooLarge(_))
        ));
    }
    #[tokio::test]
    async fn full_draft_quota_still_allows_clear_and_preferences() {
        let (repo, pool) = seeded().await;
        sqlx::query("WITH RECURSIVE n(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM n WHERE x<128) INSERT INTO agent_runtime_sessions(id,workspace_id,provider,title,cwd,created_by,created_at,last_active_at) SELECT 'extra-'||x,'w1','claude','T','/tmp','u1','2026-11-07T00:00:00Z','2026-11-07T00:00:00Z' FROM n").execute(&pool).await.unwrap();
        let value = RuntimeDraftContent {
            text: "draft".into(),
            ..Default::default()
        };
        sqlx::query("INSERT INTO runtime_composer_drafts(user_id,workspace_id,session_id,revision,payload_json,updated_at) SELECT 'u1','w1',id,1,?,? FROM agent_runtime_sessions WHERE id LIKE 'extra-%'")
            .bind(serde_json::to_string(&value).unwrap()).bind(Utc::now().timestamp()).execute(&pool).await.unwrap();
        let (u, w, r) = ("u1".into(), "w1".into(), "r1".into());
        assert!(matches!(
            repo.save_composer_draft(&u, &w, &r, 0, &value).await,
            Err(Error::PayloadTooLarge(_))
        ));
        // Rejected inserts roll back. Clearing never needs a free content slot.
        repo.save_composer_draft(&u, &w, &r, 0, &RuntimeDraftContent::default())
            .await
            .unwrap();
        let prefs = RuntimeDraftContent {
            model: Some("picked".into()),
            effort: Some("high".into()),
            access_mode: Some(RuntimeAccessMode::FullAccess),
            ..Default::default()
        };
        repo.save_composer_draft(&u, &w, &r, 1, &prefs)
            .await
            .unwrap();
        repo.save_composer_draft(
            &u,
            &w,
            &"extra-1".into(),
            1,
            &RuntimeDraftContent::default(),
        )
        .await
        .unwrap();
        repo.save_composer_draft(&u, &w, &r, 2, &value)
            .await
            .unwrap();
    }
}
