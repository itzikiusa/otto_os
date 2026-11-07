//! Persistence for the structured agent runtime (migrations 0197–0201): runtime
//! session rows and the entity projection (the latest JSON state of every
//! provider session / thread, turn, turn item, runtime request and plan —
//! the T3 Code projection model). The live event ring lives in memory
//! (`otto-agent-protocol`); this is what survives a daemon restart.

mod drafts;

use chrono::Utc;
use otto_core::agent_runtime::{
    RuntimeAccessMode, RuntimeProvider, RuntimeSession, RuntimeSessionStatus,
};
use otto_core::{Error, Id, Result};
use sqlx::Row;

use crate::convert::{dberr, fmt, ts};
use crate::DbPool;

#[derive(Clone)]
pub struct AgentRuntimeRepo {
    pool: DbPool,
}

pub struct NewRuntimeSession {
    pub id: Id,
    pub workspace_id: Id,
    pub provider: RuntimeProvider,
    pub title: String,
    pub cwd: String,
    pub model: Option<String>,
    pub access_mode: RuntimeAccessMode,
    pub created_by: Id,
    /// The linked group's root thread (see `RuntimeSession::parent_id`).
    pub parent_id: Option<Id>,
    /// Server-owned meta (see `RuntimeSession::meta`).
    pub meta: Option<serde_json::Value>,
}

/// A stored session plus the sequence floor a restarted runtime resumes above.
#[derive(Debug, Clone)]
pub struct RuntimeSessionRecord {
    /// `status` is `closed` or `dormant` (the live status is the runtime's).
    pub session: RuntimeSession,
    pub closed: bool,
    pub seq_reserved: u64,
}

/// One projected entity: `kind` is the runtime's entity kind tag
/// (`provider_session`, `turn`, `turn_item`, …), `payload` its JSON.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeEntityRow {
    pub kind: String,
    pub entity_id: String,
    pub sequence: u64,
    pub payload: serde_json::Value,
}

/// Column list (a macro so the queries stay `&'static str`, as sqlx requires).
macro_rules! session_cols {
    () => {
        "id, workspace_id, provider, title, cwd, model, native_thread_id, closed, \
         created_by, created_at, last_active_at, last_seq, seq_reserved, access_mode, parent_id, ui_control, meta"
    };
}

fn u(v: i64) -> u64 {
    v.max(0) as u64
}

fn i(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

fn row_to_session(r: &sqlx::sqlite::SqliteRow) -> Result<RuntimeSessionRecord> {
    let provider: String = r.get("provider");
    let provider = RuntimeProvider::parse(&provider)
        .ok_or_else(|| Error::Internal(format!("bad runtime provider '{provider}'")))?;
    let closed = r.get::<i64, _>("closed") != 0;
    Ok(RuntimeSessionRecord {
        session: RuntimeSession {
            id: r.get("id"),
            workspace_id: r.get("workspace_id"),
            provider,
            title: r.get("title"),
            cwd: r.get("cwd"),
            model: r.get("model"),
            native_thread_id: r.get("native_thread_id"),
            status: if closed {
                RuntimeSessionStatus::Closed
            } else {
                RuntimeSessionStatus::Dormant
            },
            access_mode: RuntimeAccessMode::parse(&r.get::<String, _>("access_mode"))
                .unwrap_or_default(),
            created_by: r.get("created_by"),
            created_at: ts(&r.get::<String, _>("created_at"))?,
            last_active_at: ts(&r.get::<String, _>("last_active_at"))?,
            last_sequence: u(r.get("last_seq")),
            parent_id: r.get("parent_id"),
            ui_control: r
                .get::<Option<String>, _>("ui_control")
                .and_then(|j| serde_json::from_str(&j).ok()),
            background_tasks: Vec::new(),
            meta: r
                .get::<Option<String>, _>("meta")
                .and_then(|j| serde_json::from_str(&j).ok()),
        },
        closed,
        seq_reserved: u(r.get("seq_reserved")),
    })
}

impl AgentRuntimeRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, new: NewRuntimeSession) -> Result<RuntimeSessionRecord> {
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO agent_runtime_sessions (id, workspace_id, provider, title, cwd, model, \
             access_mode, created_by, created_at, last_active_at, parent_id, meta) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&new.id)
        .bind(&new.workspace_id)
        .bind(new.provider.as_str())
        .bind(&new.title)
        .bind(&new.cwd)
        .bind(&new.model)
        .bind(new.access_mode.as_str())
        .bind(&new.created_by)
        .bind(&now)
        .bind(&now)
        .bind(&new.parent_id)
        .bind(new.meta.as_ref().map(|m| m.to_string()))
        .execute(&self.pool)
        .await
        .map_err(dberr("create runtime session"))?;
        self.get(&new.id).await
    }

    pub async fn get(&self, id: &Id) -> Result<RuntimeSessionRecord> {
        let row = sqlx::query(concat!(
            "SELECT ",
            session_cols!(),
            " FROM agent_runtime_sessions WHERE id = ?"
        ))
        .bind(id)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("runtime session not found"))?;
        row_to_session(&row)
    }

    /// Sessions of `workspace_id`, newest activity first; `owner` narrows to
    /// one creator (non-admin callers).
    pub async fn list(
        &self,
        workspace_id: &Id,
        owner: Option<&Id>,
    ) -> Result<Vec<RuntimeSessionRecord>> {
        let rows = sqlx::query(concat!(
            "SELECT ",
            session_cols!(),
            " FROM agent_runtime_sessions WHERE workspace_id = ? \
             AND (? IS NULL OR created_by = ?) ORDER BY last_active_at DESC LIMIT 500"
        ))
        .bind(workspace_id)
        .bind(owner)
        .bind(owner)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list runtime sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Every session that has an entity a previous life left open — boot
    /// recovery's work list.
    pub async fn session_ids(&self) -> Result<Vec<Id>> {
        sqlx::query_scalar("SELECT id FROM agent_runtime_sessions WHERE closed = 0")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("list runtime session ids"))
    }

    pub async fn set_native_thread_id(&self, id: &Id, native: &str) -> Result<()> {
        sqlx::query("UPDATE agent_runtime_sessions SET native_thread_id = ? WHERE id = ?")
            .bind(native)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set native thread id"))?;
        Ok(())
    }

    pub async fn set_title(&self, id: &Id, title: &str) -> Result<()> {
        sqlx::query("UPDATE agent_runtime_sessions SET title = ? WHERE id = ?")
            .bind(title)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("rename runtime session"))?;
        Ok(())
    }

    pub async fn set_model(&self, id: &Id, model: &str) -> Result<()> {
        sqlx::query("UPDATE agent_runtime_sessions SET model = ? WHERE id = ?")
            .bind(model)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set runtime session model"))?;
        Ok(())
    }

    pub async fn set_access_mode(&self, id: &Id, mode: RuntimeAccessMode) -> Result<()> {
        sqlx::query("UPDATE agent_runtime_sessions SET access_mode = ? WHERE id = ?")
            .bind(mode.as_str())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set runtime session access mode"))?;
        Ok(())
    }

    /// Store the agent-UI-control grant record (see `RuntimeSession::ui_control`).
    pub async fn set_ui_control(&self, id: &Id, record: &serde_json::Value) -> Result<()> {
        let res = sqlx::query("UPDATE agent_runtime_sessions SET ui_control = ? WHERE id = ?")
            .bind(record.to_string())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("set runtime session ui control"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("runtime session {id}")));
        }
        Ok(())
    }

    pub async fn set_closed(&self, id: &Id) -> Result<()> {
        sqlx::query("UPDATE agent_runtime_sessions SET closed = 1 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("close runtime session"))?;
        Ok(())
    }

    /// Reopen a closed session (it takes prompts again).
    pub async fn set_open(&self, id: &Id) -> Result<()> {
        let res = sqlx::query("UPDATE agent_runtime_sessions SET closed = 0 WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("reopen runtime session"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("runtime session {id}")));
        }
        Ok(())
    }

    /// Clear the provider's resume key (a resume it refused: the next spawn
    /// starts a fresh provider thread).
    pub async fn clear_native_thread_id(&self, id: &Id) -> Result<()> {
        sqlx::query("UPDATE agent_runtime_sessions SET native_thread_id = NULL WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("clear runtime native thread id"))?;
        Ok(())
    }

    /// Remove a session row (its entities cascade) — a create whose first
    /// spawn was refused, or a person's Delete.
    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM agent_runtime_sessions WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete runtime session"))?;
        Ok(())
    }

    /// Raise the sequence floor a restarted runtime resumes above (never lowers it).
    pub async fn reserve_seq(&self, id: &Id, upto: u64) -> Result<()> {
        sqlx::query(
            "UPDATE agent_runtime_sessions SET seq_reserved = MAX(seq_reserved, ?) WHERE id = ?",
        )
        .bind(i(upto))
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("reserve runtime seq"))?;
        Ok(())
    }

    /// Upsert entities and advance the session's `last_seq` in one write.
    pub async fn upsert_entities(&self, session_id: &Id, rows: &[RuntimeEntityRow]) -> Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let now = fmt(Utc::now());
        let mut tx = self.pool.begin().await.map_err(dberr("upsert entities"))?;
        let mut max = 0u64;
        for r in rows {
            max = max.max(r.sequence);
            sqlx::query(
                "INSERT INTO agent_runtime_entities (session_id, kind, entity_id, sequence, \
                 payload_json) VALUES (?, ?, ?, ?, ?) ON CONFLICT (session_id, kind, entity_id) \
                 DO UPDATE SET sequence = excluded.sequence, payload_json = excluded.payload_json \
                 WHERE excluded.sequence >= agent_runtime_entities.sequence",
            )
            .bind(session_id)
            .bind(&r.kind)
            .bind(&r.entity_id)
            .bind(i(r.sequence))
            .bind(r.payload.to_string())
            .execute(&mut *tx)
            .await
            .map_err(dberr("upsert entity"))?;
        }
        sqlx::query(
            "UPDATE agent_runtime_sessions SET last_seq = MAX(last_seq, ?), \
             seq_reserved = MAX(seq_reserved, ?), last_active_at = ? WHERE id = ?",
        )
        .bind(i(max))
        .bind(i(max))
        .bind(&now)
        .bind(session_id)
        .execute(&mut *tx)
        .await
        .map_err(dberr("upsert entities"))?;
        tx.commit().await.map_err(dberr("upsert entities"))?;
        Ok(())
    }

    /// Every entity of a session, oldest update first.
    pub async fn entities(&self, session_id: &Id) -> Result<Vec<RuntimeEntityRow>> {
        let rows = sqlx::query(
            "SELECT kind, entity_id, sequence, payload_json FROM agent_runtime_entities \
             WHERE session_id = ? ORDER BY sequence ASC",
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list runtime entities"))?;
        rows.iter()
            .map(|r| {
                Ok(RuntimeEntityRow {
                    kind: r.get("kind"),
                    entity_id: r.get("entity_id"),
                    sequence: u(r.get("sequence")),
                    payload: serde_json::from_str(&r.get::<String, _>("payload_json"))
                        .map_err(|e| Error::Internal(format!("bad runtime entity: {e}")))?,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool.into()
    }

    async fn seed(pool: &DbPool) {
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) \
             VALUES ('u1', 'u', 'x', 'U', 0, ?)",
        )
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('w1', 'W', '/tmp', ?)",
        )
        .bind(&now)
        .execute(pool)
        .await
        .unwrap();
    }

    fn row(kind: &str, id: &str, seq: u64, v: serde_json::Value) -> RuntimeEntityRow {
        RuntimeEntityRow {
            kind: kind.into(),
            entity_id: id.into(),
            sequence: seq,
            payload: v,
        }
    }

    #[tokio::test]
    async fn entities_upsert_newest_wins_and_advance_the_floor() {
        let pool = mem_pool().await;
        seed(&pool).await;
        let repo = AgentRuntimeRepo::new(pool);
        let rec = repo
            .create(NewRuntimeSession {
                id: "r1".into(),
                workspace_id: "w1".into(),
                provider: RuntimeProvider::Gemini,
                title: "t".into(),
                cwd: "/tmp".into(),
                model: None,
                access_mode: RuntimeAccessMode::AutoAcceptEdits,
                parent_id: None,
                meta: Some(serde_json::json!({"personal_agent": "a1"})),
                created_by: "u1".into(),
            })
            .await
            .unwrap();
        assert_eq!(rec.session.status, RuntimeSessionStatus::Dormant);
        assert_eq!(rec.session.access_mode, RuntimeAccessMode::AutoAcceptEdits);
        assert_eq!(
            rec.session.meta,
            Some(serde_json::json!({"personal_agent": "a1"}))
        );
        let id: Id = "r1".into();
        repo.set_access_mode(&id, RuntimeAccessMode::FullAccess)
            .await
            .unwrap();
        assert_eq!(
            repo.get(&id).await.unwrap().session.access_mode,
            RuntimeAccessMode::FullAccess
        );
        repo.upsert_entities(&id, &[row("turn", "t1", 2, json!({"v":1}))])
            .await
            .unwrap();
        repo.upsert_entities(
            &id,
            &[
                row("turn", "t1", 5, json!({"v":2})),
                row("turn_item", "i1", 4, json!({"x":1})),
            ],
        )
        .await
        .unwrap();
        // An older write never overwrites a newer state.
        repo.upsert_entities(&id, &[row("turn", "t1", 3, json!({"v":0}))])
            .await
            .unwrap();
        let got = repo.entities(&id).await.unwrap();
        assert_eq!(
            got,
            vec![
                row("turn_item", "i1", 4, json!({"x":1})),
                row("turn", "t1", 5, json!({"v":2})),
            ]
        );
        let rec = repo.get(&id).await.unwrap();
        assert_eq!((rec.session.last_sequence, rec.seq_reserved), (5, 5));
        repo.reserve_seq(&id, 100).await.unwrap();
        repo.reserve_seq(&id, 50).await.unwrap();
        assert_eq!(repo.get(&id).await.unwrap().seq_reserved, 100);
        repo.set_native_thread_id(&id, "acp-1").await.unwrap();
        assert_eq!(
            repo.get(&id)
                .await
                .unwrap()
                .session
                .native_thread_id
                .as_deref(),
            Some("acp-1")
        );
        assert_eq!(repo.session_ids().await.unwrap(), vec![id.clone()]);
        repo.set_closed(&id).await.unwrap();
        assert!(repo.session_ids().await.unwrap().is_empty());
        assert_eq!(
            repo.list(&"w1".into(), Some(&"u1".into()))
                .await
                .unwrap()
                .len(),
            1
        );
        repo.delete(&id).await.unwrap();
        assert!(repo.entities(&id).await.unwrap().is_empty());
    }
}
