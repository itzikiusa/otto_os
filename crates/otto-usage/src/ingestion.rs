//! Transactional source checkpoints and replayable usage outbox.
//!
//! SQLite FULL synchronous commits on sqlx's dedicated worker thread precede
//! source advancement and ClickHouse visibility. Never delete/recreate a corrupt
//! journal: a failure must stop ingestion, preserving evidence for recovery.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};
use sqlx::SqlitePool;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct DurableEvent {
    pub event_id: String,
    #[serde(flatten)]
    pub event: crate::UsageEvent,
}

pub(crate) struct IngestionJournal {
    pool: SqlitePool,
    parts: tokio::sync::Mutex<std::collections::BTreeMap<String, String>>,
}

impl IngestionJournal {
    pub async fn open(path: &Path) -> Result<Self, sqlx::Error> {
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Full);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await?;
        // Additive, idempotent schema in its OWN DB, not otto-state's migration
        // namespace. A read validates integrity before accepting source state.
        sqlx::query("CREATE TABLE IF NOT EXISTS checkpoint_parts (path TEXT PRIMARY KEY, payload TEXT NOT NULL)")
            .execute(&pool).await?;
        sqlx::query("CREATE TABLE IF NOT EXISTS outbox (sequence INTEGER PRIMARY KEY AUTOINCREMENT, event_id TEXT NOT NULL UNIQUE, payload TEXT NOT NULL)")
            .execute(&pool).await?;
        let check: String = sqlx::query_scalar("PRAGMA quick_check")
            .fetch_one(&pool)
            .await?;
        if check != "ok" {
            return Err(sqlx::Error::Protocol(format!(
                "usage journal integrity: {check}"
            )));
        }
        let rows: Vec<(String, String)> =
            sqlx::query_as("SELECT path,payload FROM checkpoint_parts")
                .fetch_all(&pool)
                .await?;
        Ok(Self {
            pool,
            parts: tokio::sync::Mutex::new(rows.into_iter().collect()),
        })
    }

    pub async fn checkpoint(&self) -> Result<Option<Value>, sqlx::Error> {
        let parts = self.parts.lock().await;
        if parts.is_empty() {
            return Ok(None);
        }
        inflate(&parts).map(Some)
    }

    pub async fn commit(
        &self,
        checkpoint: &Value,
        events: &[crate::UsageEvent],
    ) -> Result<(), sqlx::Error> {
        // Only changed leaves/chunks hit disk. Flattening a bounded checkpoint
        // runs on a blocking pool, not on a Tokio worker; idle scans skip commit.
        let snapshot = checkpoint.clone();
        let next = tokio::task::spawn_blocking(move || flatten(&snapshot))
            .await
            .map_err(|e| sqlx::Error::Protocol(format!("checkpoint worker: {e}")))?;
        let mut previous = self.parts.lock().await;
        let mut tx = self.pool.begin().await?;
        let pending: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
            .fetch_one(&mut *tx)
            .await?;
        // Backpressure keeps disk bounded; never drop accepted spend. Caller
        // retains this source batch and retries without reading further bytes.
        if pending.saturating_add(events.len() as i64) > 100_000 {
            return Err(sqlx::Error::Protocol(
                "usage outbox full; source checkpoint retained".into(),
            ));
        }
        for key in previous.keys().filter(|key| !next.contains_key(*key)) {
            sqlx::query("DELETE FROM checkpoint_parts WHERE path=?")
                .bind(key)
                .execute(&mut *tx)
                .await?;
        }
        for (key, value) in &next {
            if previous.get(key) != Some(value) {
                sqlx::query("INSERT INTO checkpoint_parts(path,payload) VALUES(?,?) ON CONFLICT(path) DO UPDATE SET payload=excluded.payload")
                    .bind(key).bind(value).execute(&mut *tx).await?;
            }
        }
        for event in events {
            let row = DurableEvent {
                event_id: otto_core::new_id(),
                event: event.clone(),
            };
            let payload =
                serde_json::to_string(&row).map_err(|e| sqlx::Error::Encode(Box::new(e)))?;
            sqlx::query("INSERT INTO outbox(event_id,payload) VALUES(?,?)")
                .bind(&row.event_id)
                .bind(payload)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        *previous = next;
        Ok(())
    }

    pub async fn pending_count(&self) -> Result<u64, sqlx::Error> {
        let n: i64 = sqlx::query_scalar("SELECT count(*) FROM outbox")
            .fetch_one(&self.pool)
            .await?;
        Ok(n as u64)
    }

    pub async fn pending(&self, limit: u32) -> Result<Vec<DurableEvent>, sqlx::Error> {
        let rows: Vec<String> =
            sqlx::query_scalar("SELECT payload FROM outbox ORDER BY sequence LIMIT ?")
                .bind(limit)
                .fetch_all(&self.pool)
                .await?;
        rows.into_iter()
            .map(|s| serde_json::from_str(&s).map_err(|e| sqlx::Error::Decode(Box::new(e))))
            .collect()
    }

    pub async fn ack(&self, rows: &[DurableEvent]) -> Result<(), sqlx::Error> {
        let mut tx = self.pool.begin().await?;
        for row in rows {
            sqlx::query("DELETE FROM outbox WHERE event_id=?")
                .bind(&row.event_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await
    }
}

// Maps become independent checkpoint rows; arrays/scalars remain leaves.
// JSON-encoded path arrays are unambiguous even for paths containing slashes.
fn flatten(value: &Value) -> std::collections::BTreeMap<String, String> {
    fn visit(
        value: &Value,
        path: &mut Vec<String>,
        out: &mut std::collections::BTreeMap<String, String>,
    ) {
        if let Value::Object(map) = value {
            if !map.is_empty() {
                for (key, value) in map {
                    path.push(key.clone());
                    visit(value, path, out);
                    path.pop();
                }
                return;
            }
        }
        out.insert(
            serde_json::to_string(path).expect("string path"),
            value.to_string(),
        );
    }
    let mut out = Default::default();
    visit(value, &mut Vec::new(), &mut out);
    out
}

fn inflate(parts: &std::collections::BTreeMap<String, String>) -> Result<Value, sqlx::Error> {
    let mut root = serde_json::json!({});
    for (path, payload) in parts {
        let path: Vec<String> =
            serde_json::from_str(path).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let value = serde_json::from_str(payload).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let mut cursor = &mut root;
        for key in path {
            if !cursor.is_object() {
                *cursor = serde_json::json!({});
            }
            cursor = cursor
                .as_object_mut()
                .expect("object")
                .entry(key)
                .or_insert(Value::Null);
        }
        *cursor = value;
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn checkpoint_parts_roundtrip_paths_chunks_and_empty_containers() {
        let state = json!({"cursors":{"/tmp/a~b/\"c":123},"seen":{"chunks":{"0":["a","b"],"1":["c"]}},"held":{},"order":[]});
        assert_eq!(inflate(&flatten(&state)).unwrap(), state);
        let mut changed = state.clone();
        changed["seen"]["chunks"]["1"] = json!(["c", "d"]);
        let old = flatten(&state);
        let new = flatten(&changed);
        assert_eq!(
            new.iter()
                .filter(|(key, value)| old.get(*key) != Some(*value))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn checkpoint_removes_obsolete_parts_and_rejects_malformed_payload() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.sqlite3");
        let journal = IngestionJournal::open(&path).await.unwrap();
        journal
            .commit(&json!({"old":{"key":1},"kept":[1]}), &[])
            .await
            .unwrap();
        journal.commit(&json!({"kept":[2]}), &[]).await.unwrap();
        journal.pool.close().await;
        let recovered = IngestionJournal::open(&path).await.unwrap();
        assert_eq!(
            recovered.checkpoint().await.unwrap(),
            Some(json!({"kept":[2]}))
        );
        sqlx::query("UPDATE checkpoint_parts SET payload='invalid JSON'")
            .execute(&recovered.pool)
            .await
            .unwrap();
        recovered.pool.close().await;
        let corrupt = IngestionJournal::open(&path).await.unwrap();
        assert!(corrupt.checkpoint().await.is_err());
    }

    fn event() -> crate::UsageEvent {
        crate::UsageEvent {
            ts: Some("2026-10-18T01:00:00Z".into()),
            input_tokens: 1000,
            session_id: "session".into(),
            workspace_id: "workspace".into(),
            provider: "codex".into(),
            kind: "completion".into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn checkpoint_and_original_ids_survive_every_delivery_crash_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.sqlite3");
        let journal = IngestionJournal::open(&path).await.unwrap();
        assert!(journal.checkpoint().await.unwrap().is_none());
        journal
            .commit(&json!({"offset": 123}), &[event()])
            .await
            .unwrap();
        let before = journal.pending(100).await.unwrap();
        assert_eq!(before.len(), 1);
        journal.pool.close().await;
        // Crash after journal commit / before INSERT: source progress and its
        // owed event are recovered together. Crash after INSERT / before ack:
        // replay carries the EXACT same ID (CH FINAL test covers count-once).
        let recovered = IngestionJournal::open(&path).await.unwrap();
        assert_eq!(
            recovered.checkpoint().await.unwrap(),
            Some(json!({"offset": 123}))
        );
        let replay = recovered.pending(100).await.unwrap();
        assert_eq!(before[0].event_id, replay[0].event_id);
        assert_eq!(replay[0].event.input_tokens, 1000);
        recovered.ack(&replay).await.unwrap();
        recovered.pool.close().await;
        let recovered = IngestionJournal::open(&path).await.unwrap();
        assert!(recovered.pending(100).await.unwrap().is_empty());
        assert_eq!(
            recovered.checkpoint().await.unwrap(),
            Some(json!({"offset": 123}))
        );
    }

    #[tokio::test]
    async fn failed_outbox_write_cannot_advance_checkpoint() {
        let dir = tempfile::tempdir().unwrap();
        let journal = IngestionJournal::open(&dir.path().join("journal.sqlite3"))
            .await
            .unwrap();
        journal.commit(&json!({"offset": 0}), &[]).await.unwrap();
        sqlx::query("CREATE TRIGGER fail_outbox BEFORE INSERT ON outbox BEGIN SELECT RAISE(ABORT, 'simulated disk full'); END")
            .execute(&journal.pool).await.unwrap();
        assert!(journal
            .commit(&json!({"offset": 123}), &[event()])
            .await
            .is_err());
        assert_eq!(
            journal.checkpoint().await.unwrap(),
            Some(json!({"offset": 0}))
        );
        assert!(journal.pending(100).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn corrupt_journal_is_an_error_never_an_empty_reset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.sqlite3");
        std::fs::write(&path, b"not a SQLite database").unwrap();
        assert!(IngestionJournal::open(&path).await.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"not a SQLite database");
    }
}
