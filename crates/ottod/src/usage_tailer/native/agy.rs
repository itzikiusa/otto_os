//! Read Antigravity's authoritative per-conversation SQLite usage snapshots.
//! Connections are read-only and never migrate or modify a provider database.
use otto_transcript::agy::{
    default_agy_conversation_dirs, parse_agy_conversation_blobs_from, parse_agy_trajectory_lineage,
    AgyParseSeed, AgyUsageRecord,
};
use sqlx::{Connection, Row};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Rows at or below a high-water mark that are still re-read on every pass:
/// the provider revises a live step's usage in place (and can fill a step's
/// metadata after later rows landed), but only near the conversation's tail.
const REVISABLE_ROWS: i64 = 32;

/// `(db, wal)` size + mtime, as one compact string (one checkpoint leaf).
pub(super) type Fingerprint = String;

fn stamp(meta: &std::fs::Metadata) -> String {
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map_or_else(|| "-".to_owned(), |d| d.as_nanos().to_string());
    format!("{}@{mtime}", meta.len())
}

/// What the tailer remembers per Antigravity DB between reads.
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub(super) struct FileState {
    pub fp: Fingerprint,
    #[serde(default)]
    pub mark: Mark,
}

/// Incremental read position: the highest `steps.idx` / `gen_metadata.idx`
/// already parsed, plus the parse seed the rows below it established.
#[derive(Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct Mark {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    steps: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    gens: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ts: Option<i64>,
}

#[allow(clippy::disallowed_methods)] // Called only from spawn_blocking.
pub(super) fn discover(home: &Path, extra: Option<&Path>) -> Vec<(PathBuf, Fingerprint)> {
    let mut files = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let dirs = extra
        .map(Path::to_path_buf)
        .into_iter()
        .chain(default_agy_conversation_dirs(home));
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "db") {
                continue;
            }
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            if !meta.is_file() || meta.len() < 100 {
                continue;
            }
            if !seen.insert(path.file_stem().unwrap_or_default().to_os_string()) {
                continue;
            }
            let wal = std::fs::metadata(format!("{}-wal", path.display()))
                .map_or_else(|_| "-".to_owned(), |m| stamp(&m));
            files.push((path, format!("{}|{wal}", stamp(&meta))));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

pub(super) struct Snapshot {
    pub conversation: String,
    pub parent: Option<String>,
    pub root: Option<String>,
    pub rows: Vec<AgyUsageRecord>,
    /// Where the next read of this DB resumes.
    pub mark: Mark,
}

/// Read the rows past `mark` (minus [`REVISABLE_ROWS`]); `Mark::default()`
/// reads everything. Rows below the window were billed by an earlier read:
/// a merge they would join is resolved through the persisted aliases, and
/// their "first seen" model/timestamp travel in the mark's parse seed.
pub(super) async fn read(path: &Path, mark: &Mark) -> Result<Snapshot, sqlx::Error> {
    let conversation = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let options = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(path)
        .read_only(true)
        .create_if_missing(false)
        .busy_timeout(std::time::Duration::from_secs(2));
    let mut connection = sqlx::SqliteConnection::connect_with(&options).await?;
    let result = async {
        let mut tx = connection.begin().await?;
        // A DB whose rows now end below the mark was recreated: start over.
        let (max_step, max_gen): (Option<i64>, Option<i64>) = sqlx::query_as(
            "SELECT (SELECT MAX(idx) FROM steps), (SELECT MAX(idx) FROM gen_metadata)",
        )
        .fetch_one(&mut *tx)
        .await?;
        let regressed = |seen: Option<i64>, now: Option<i64>| {
            seen.is_some_and(|seen| now.is_none_or(|now| now < seen))
        };
        let mark = if regressed(mark.steps, max_step) || regressed(mark.gens, max_gen) {
            Mark::default()
        } else {
            mark.clone()
        };
        let floor = |hw: Option<i64>| hw.map_or(i64::MIN, |h| h.saturating_sub(REVISABLE_ROWS));
        let generations =
            sqlx::query("SELECT idx, data FROM gen_metadata WHERE idx > ? ORDER BY idx")
                .bind(floor(mark.gens))
                .fetch_all(&mut *tx)
                .await?
                .iter()
                .map(|row| Ok((row.try_get("idx")?, row.try_get("data")?)))
                .collect::<Result<Vec<(i64, Vec<u8>)>, sqlx::Error>>()?;
        let trajectories: Vec<Vec<u8>> =
            sqlx::query_scalar("SELECT data FROM trajectory_metadata_blob ORDER BY rowid ASC")
                .fetch_all(&mut *tx)
                .await?;
        let steps = sqlx::query(
            "SELECT idx, metadata FROM steps WHERE metadata IS NOT NULL AND idx > ? ORDER BY idx",
        )
        .bind(floor(mark.steps))
        .fetch_all(&mut *tx)
        .await?
        .iter()
        .map(|row| Ok((row.try_get("idx")?, row.try_get("metadata")?)))
        .collect::<Result<Vec<(i64, Vec<u8>)>, sqlx::Error>>()?;
        let mut seed = AgyParseSeed {
            fallback_model: mark.model.clone(),
            first_step_ts_ms: mark.ts,
        };
        let rows = parse_agy_conversation_blobs_from(
            &conversation,
            &trajectories,
            &generations,
            &steps,
            &mut seed,
        );
        let (parent, root) = parse_agy_trajectory_lineage(&trajectories);
        tx.commit().await?;
        let high = |seen: Option<i64>, batch: &[(i64, Vec<u8>)]| {
            batch.iter().map(|(idx, _)| *idx).max().max(seen)
        };
        Ok(Snapshot {
            conversation,
            parent,
            root,
            rows,
            mark: Mark {
                steps: high(mark.steps, &steps),
                gens: high(mark.gens, &generations),
                model: seed.fallback_model,
                ts: seed.first_step_ts_ms,
            },
        })
    }
    .await;
    connection.close().await?;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint(mut n: u64) -> Vec<u8> {
        let mut bytes = Vec::new();
        while n >= 128 {
            bytes.push((n as u8 & 127) | 128);
            n >>= 7;
        }
        bytes.push(n as u8);
        bytes
    }
    fn number(field: u64, n: u64) -> Vec<u8> {
        [varint(field << 3), varint(n)].concat()
    }
    fn message(field: u64, bytes: &[u8]) -> Vec<u8> {
        [
            varint((field << 3) | 2),
            varint(bytes.len() as u64),
            bytes.to_vec(),
        ]
        .concat()
    }
    fn step(output: u64) -> Vec<u8> {
        let tokens = [
            number(1, 1418),
            number(2, 100),
            number(3, output),
            number(5, 400),
        ]
        .concat();
        [message(1, &number(1, 1785585600)), message(9, &tokens)].concat()
    }

    /// Bill `rows` the way `scan_agy` does; returns (input, output,
    /// cache_read, cost) owed by this batch.
    fn bill(
        state: &mut super::super::NativeState,
        rows: &[AgyUsageRecord],
    ) -> (u64, u64, u64, f64) {
        let mut owed = (0, 0, 0, 0.0);
        for row in rows {
            let cost = otto_usage::estimate_cost(
                &row.usage.model,
                row.usage.input,
                row.usage.output,
                row.usage.cache_read,
                row.usage.cache_write,
            );
            if let Some((delta, cost)) =
                state.delta_aliased(&row.dedup_key, &row.aliases, &row.usage, Some(cost))
            {
                owed.0 += delta.input;
                owed.1 += delta.output;
                owed.2 += delta.cache_read;
                owed.3 += cost.unwrap_or(0.0);
            }
        }
        owed
    }

    #[tokio::test]
    async fn incremental_reads_bill_exactly_what_one_full_read_does() {
        let temp = std::env::temp_dir().join(format!("otto-agy-incr-{}", otto_core::new_id()));
        std::fs::create_dir_all(&temp).unwrap();
        let path = temp.join("conversation.db");
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let mut writer = sqlx::SqliteConnection::connect_with(&options)
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TABLE steps(idx INTEGER, metadata BLOB); CREATE TABLE gen_metadata(idx INTEGER,data BLOB); CREATE TABLE trajectory_metadata_blob(data BLOB);").execute(&mut writer).await.unwrap();
        let mut incremental = super::super::NativeState::default();
        let mut mark = Mark::default();
        let mut owed = (0, 0, 0, 0.0);
        let mut next = 1_i64;
        let mut last_batch = 0;
        for round in 0..6_u64 {
            // The live tail step is revised in place (usage grows)…
            if next > 1 {
                sqlx::query("UPDATE steps SET metadata=? WHERE idx=?")
                    .bind(step(500 + round))
                    .bind(next - 1)
                    .execute(&mut writer)
                    .await
                    .unwrap();
            }
            // …and new steps land after it.
            for _ in 0..25 {
                sqlx::query("INSERT INTO steps VALUES(?,?)")
                    .bind(next)
                    .bind(step(10 + next as u64))
                    .execute(&mut writer)
                    .await
                    .unwrap();
                next += 1;
            }
            let snapshot = read(&path, &mark).await.unwrap();
            last_batch = snapshot.rows.len();
            let batch = bill(&mut incremental, &snapshot.rows);
            owed = (
                owed.0 + batch.0,
                owed.1 + batch.1,
                owed.2 + batch.2,
                owed.3 + batch.3,
            );
            mark = snapshot.mark;
        }
        assert_eq!(mark.steps, Some(next - 1));
        assert!(
            last_batch < (next - 1) as usize,
            "a later read must not re-parse the whole history ({last_batch} rows)"
        );
        let full = read(&path, &Mark::default()).await.unwrap();
        assert_eq!(full.rows.len(), (next - 1) as usize);
        let whole = bill(&mut super::super::NativeState::default(), &full.rows);
        assert_eq!((owed.0, owed.1, owed.2), (whole.0, whole.1, whole.2));
        assert!((owed.3 - whole.3).abs() < 1e-9, "{} vs {}", owed.3, whole.3);
        // A recreated DB (rows now end below the mark) is read from scratch.
        sqlx::query("DELETE FROM steps WHERE idx > 3")
            .execute(&mut writer)
            .await
            .unwrap();
        let reset = read(&path, &mark).await.unwrap();
        assert_eq!(reset.rows.len(), 3);
        assert_eq!(reset.mark.steps, Some(3));
        writer.close().await.unwrap();
        tokio::fs::remove_dir_all(temp).await.unwrap();
    }

    #[tokio::test]
    async fn historical_sqlite_rows_keep_disjoint_cache_and_revised_steps() {
        let temp = std::env::temp_dir().join(format!("otto-agy-reader-{}", otto_core::new_id()));
        std::fs::create_dir_all(&temp).unwrap();
        let home = temp.join("home");
        let path = home.join(".gemini/antigravity-cli/conversations/conversation.db");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true);
        let mut writer = sqlx::SqliteConnection::connect_with(&options)
            .await
            .unwrap();
        sqlx::raw_sql("CREATE TABLE steps(idx INTEGER, metadata BLOB); CREATE TABLE gen_metadata(idx INTEGER,data BLOB); CREATE TABLE trajectory_metadata_blob(data BLOB);").execute(&mut writer).await.unwrap();
        sqlx::query("INSERT INTO steps VALUES(1,?)")
            .bind(step(20))
            .execute(&mut writer)
            .await
            .unwrap();
        sqlx::query("INSERT INTO trajectory_metadata_blob VALUES(?)")
            .bind(message(5, b"parent"))
            .execute(&mut writer)
            .await
            .unwrap();
        let first = read(&path, &Mark::default()).await.unwrap();
        assert_eq!(first.parent.as_deref(), Some("parent"));
        assert_eq!(first.rows.len(), 1);
        let row = &first.rows[0];
        assert_eq!(
            (row.input_tokens, row.output_tokens, row.cache_read_tokens),
            (100, 20, 400)
        );
        assert_eq!(row.timestamp_ms, 1785585600000);
        let key = row.key.clone();
        sqlx::query("UPDATE steps SET metadata=? WHERE idx=1")
            .bind(step(30))
            .execute(&mut writer)
            .await
            .unwrap();
        let updated = read(&path, &Mark::default()).await.unwrap();
        assert_eq!(
            updated.rows[0].key, key,
            "in-place usage growth must retain durable identity"
        );
        assert_eq!(updated.rows[0].output_tokens, 30);
        let pool = otto_state::open(&temp.join("state.db")).await.unwrap();
        let now = "2026-08-01T12:00:00Z";
        sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('u','u','x','U',0,?)").bind(now).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO workspaces(id,name,root_path,created_at) VALUES('w','w','/repo',?)",
        )
        .bind(now)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO sessions(id,workspace_id,kind,provider,title,status,cwd,provider_session_id,created_by,created_at,last_active_at,meta_json) VALUES('s','w','agent','agy','t','running','/repo','parent','u',?,?,'{}')").bind(now).bind(now).execute(&pool).await.unwrap();
        let engine = otto_usage::UsageEngine::start(
            otto_usage::UsageConfig {
                enabled: false,
                ..Default::default()
            },
            temp.join("usage"),
        )
        .await;
        let mut tailer = super::super::super::UsageTailer::new(
            engine.clone(),
            pool.clone(),
            temp.join("data"),
            home.clone(),
        );
        // A broken provider root must not block another provider or the
        // shared checkpoint commit (the original failure was propagated).
        #[cfg(unix)]
        {
            let grok = home.join(".grok");
            std::fs::create_dir_all(&grok).unwrap();
            std::os::unix::fs::symlink("sessions", grok.join("sessions")).unwrap();
        }
        tailer.scan_native().await.unwrap();
        assert_eq!(tailer.pending_events.len(), 1);
        assert_eq!(tailer.pending_events[0].session_id, "s");
        assert_eq!(tailer.pending_events[0].workspace_id, "w");
        assert_eq!(tailer.pending_events[0].provider, "agy");
        tailer.persist().await.unwrap();
        let mut recovered = super::super::super::UsageTailer::new(
            engine.clone(),
            pool.clone(),
            temp.join("data"),
            home,
        );
        recovered
            .restore_checkpoint(engine.ingestion_checkpoint().await.unwrap().unwrap())
            .unwrap();
        recovered.scan_native().await.unwrap();
        assert!(
            recovered.pending_events.is_empty(),
            "unchanged history must not replay after restart: {:?}",
            recovered.pending_events
        );
        sqlx::query("UPDATE steps SET metadata=? WHERE idx=1")
            .bind(step(40))
            .execute(&mut writer)
            .await
            .unwrap();
        recovered.scan_native().await.unwrap();
        assert_eq!(recovered.pending_events.len(), 1);
        assert_eq!(recovered.pending_events[0].output_tokens, 10);
        assert_eq!(recovered.pending_events[0].cache_read_tokens, 0);
        pool.close().await;
        writer.close().await.unwrap();
        tokio::fs::remove_dir_all(temp).await.unwrap();
    }
}
