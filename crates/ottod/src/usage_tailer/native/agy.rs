//! Read Antigravity's authoritative per-conversation SQLite usage snapshots.
//! Connections are read-only and never migrate or modify a provider database.
use otto_transcript::agy::{
    default_agy_conversation_dirs, parse_agy_conversation_blobs, parse_agy_trajectory_lineage,
    AgyUsageRecord,
};
use sqlx::{Connection, Row};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, PartialEq, Eq)]
pub(super) struct Fingerprint {
    db: (u64, Option<SystemTime>),
    wal: Option<(u64, Option<SystemTime>)>,
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
                .ok()
                .map(|m| (m.len(), m.modified().ok()));
            files.push((
                path,
                Fingerprint {
                    db: (meta.len(), meta.modified().ok()),
                    wal,
                },
            ));
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
}

pub(super) async fn read(path: &Path) -> Result<Snapshot, sqlx::Error> {
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
        let generations = sqlx::query("SELECT idx, data FROM gen_metadata ORDER BY idx")
            .fetch_all(&mut *tx)
            .await?
            .iter()
            .map(|row| Ok((row.try_get("idx")?, row.try_get("data")?)))
            .collect::<Result<Vec<(i64, Vec<u8>)>, sqlx::Error>>()?;
        let trajectories: Vec<Vec<u8>> =
            sqlx::query_scalar("SELECT data FROM trajectory_metadata_blob ORDER BY rowid ASC")
                .fetch_all(&mut *tx)
                .await?;
        let steps =
            sqlx::query("SELECT idx, metadata FROM steps WHERE metadata IS NOT NULL ORDER BY idx")
                .fetch_all(&mut *tx)
                .await?
                .iter()
                .map(|row| Ok((row.try_get("idx")?, row.try_get("metadata")?)))
                .collect::<Result<Vec<(i64, Vec<u8>)>, sqlx::Error>>()?;
        let rows = parse_agy_conversation_blobs(&conversation, &trajectories, &generations, &steps);
        let (parent, root) = parse_agy_trajectory_lineage(&trajectories);
        tx.commit().await?;
        Ok(Snapshot {
            conversation,
            parent,
            root,
            rows,
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
        let first = read(&path).await.unwrap();
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
        let updated = read(&path).await.unwrap();
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
