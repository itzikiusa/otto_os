//! Async off-lock filesystem & read-only SQLite sideband polling for
//! [`crate::runtime`].
//!
//! All disk/DB I/O executes outside the session mutex so [`crate::acp::AcpDriver`]
//! remains strictly sans-IO.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::{json, Value};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Connection, Row, SqliteConnection};
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::burst::now_ms;
use crate::driver::{
    SidebandPayload, SidebandPoll, SidebandResumeSeed, SidebandSource, StructuredDriver,
};

const SIDEBAND_READ_CAP: usize = 2 * 1024 * 1024;
const GEMINI_SNAPSHOT_CAP: u64 = 8 * 1024 * 1024;

pub(crate) async fn seed_resume(
    driver: &mut dyn StructuredDriver,
    resume: Option<&str>,
    cwd: &str,
) {
    let Some(sid) = resume else { return };
    let Some(source) = driver.sideband_resume_poll(sid, cwd) else {
        return;
    };
    match source {
        SidebandSource::AppendLines {
            path,
            fallback_path,
            scan_sessions_dir,
            ..
        } => {
            if let Ok(meta) = tokio::fs::metadata(&path).await {
                if meta.is_file() {
                    driver.init_sideband_resume(SidebandResumeSeed::AppendOffset {
                        resolved_path: Some(path),
                        using_fallback: false,
                        offset: meta.len(),
                    });
                    return;
                }
            }
            if let Some(fb) = fallback_path {
                if let Ok(meta) = tokio::fs::metadata(&fb).await {
                    if meta.is_file() {
                        driver.init_sideband_resume(SidebandResumeSeed::AppendOffset {
                            resolved_path: None,
                            using_fallback: true,
                            offset: meta.len(),
                        });
                        return;
                    }
                }
            }
            if let Some((sessions_dir, session_id)) = scan_sessions_dir {
                if let Some(found) = scan_grok_session_file(&sessions_dir, &session_id).await {
                    if let Ok(meta) = tokio::fs::metadata(&found).await {
                        driver.init_sideband_resume(SidebandResumeSeed::AppendOffset {
                            resolved_path: Some(found),
                            using_fallback: false,
                            offset: meta.len(),
                        });
                    }
                }
            }
        }
        SidebandSource::GeminiJson { .. } | SidebandSource::OpencodeSqlite { .. } => {
            driver.init_sideband_resume(SidebandResumeSeed::IdleSinceMs(now_ms()));
        }
    }
}

/// Fetch pending sideband updates off the session lock. Returns `(payload, has_more)`.
pub(crate) async fn fetch_sideband(poll: &SidebandPoll) -> (Option<SidebandPayload>, bool) {
    match &poll.source {
        SidebandSource::AppendLines {
            path,
            fallback_path,
            scan_sessions_dir,
            offset,
            using_fallback,
        } => {
            fetch_append_lines(
                path,
                fallback_path.as_deref(),
                scan_sessions_dir.as_ref(),
                *offset,
                *using_fallback,
            )
            .await
        }
        SidebandSource::GeminiJson {
            gemini_home,
            cwd,
            session_id,
            resolved_path,
            last_stamp,
        } => {
            let payload = fetch_gemini_json(
                gemini_home,
                cwd,
                session_id,
                resolved_path.as_deref(),
                *last_stamp,
            )
            .await;
            (payload, false)
        }
        SidebandSource::OpencodeSqlite {
            db_path,
            session_id,
            min_time_ms,
            last_stamp,
        } => {
            let payload =
                fetch_opencode_sqlite(db_path, session_id, *min_time_ms, *last_stamp).await;
            (payload, false)
        }
    }
}

async fn fetch_append_lines(
    path: &Path,
    fallback_path: Option<&Path>,
    scan_sessions_dir: Option<&(PathBuf, String)>,
    offset: u64,
    prev_using_fallback: bool,
) -> (Option<SidebandPayload>, bool) {
    let mut resolved_override: Option<PathBuf> = None;
    let (using_fallback, file_res) = match tokio::fs::File::open(path).await {
        Ok(f) => (false, Ok(f)),
        Err(_) => match fallback_path {
            Some(fb) => (true, tokio::fs::File::open(fb).await),
            None => match scan_sessions_dir {
                Some((sessions_dir, sid)) => {
                    let Some(found) = scan_grok_session_file(sessions_dir, sid).await else {
                        return (None, false);
                    };
                    let open_res = tokio::fs::File::open(&found).await;
                    resolved_override = Some(found);
                    (false, open_res)
                }
                None => return (None, false),
            },
        },
    };
    let Ok(mut file) = file_res else {
        return (None, false);
    };
    let Ok(meta) = file.metadata().await else {
        return (None, false);
    };
    let len = meta.len();

    let start_offset = if prev_using_fallback != using_fallback || len < offset {
        0
    } else {
        offset
    };
    if len == start_offset && prev_using_fallback == using_fallback {
        return (None, false);
    }
    if file
        .seek(std::io::SeekFrom::Start(start_offset))
        .await
        .is_err()
    {
        return (None, false);
    }
    let to_read = ((len - start_offset) as usize).min(SIDEBAND_READ_CAP);
    if to_read == 0 {
        return (None, false);
    }
    let mut buf = vec![0u8; to_read];
    let mut read_total = 0usize;
    while read_total < to_read {
        match file.read(&mut buf[read_total..]).await {
            Ok(0) => break,
            Ok(n) => read_total += n,
            Err(_) => return (None, false),
        }
    }
    if let Some(last_nl) = buf[..read_total].iter().rposition(|b| *b == b'\n') {
        buf.truncate(last_nl + 1);
        let next_offset = start_offset + buf.len() as u64;
        return (
            Some(SidebandPayload::AppendLines {
                resolved_path: resolved_override,
                using_fallback,
                next_offset,
                bytes: buf,
            }),
            next_offset < len,
        );
    }

    // If a single line exceeded SIDEBAND_READ_CAP (e.g. huge base64 tool blob),
    // skip forward to its terminating '\n' so the tailer never wedges at start_offset.
    if read_total == SIDEBAND_READ_CAP && start_offset + (read_total as u64) < len {
        let mut skipped = start_offset + read_total as u64;
        let mut scratch = vec![0u8; 64 * 1024];
        while skipped < len {
            let Ok(n) = file.read(&mut scratch).await else {
                return (None, false);
            };
            if n == 0 {
                break;
            }
            if let Some(pos) = scratch[..n].iter().position(|b| *b == b'\n') {
                let next_offset = skipped + (pos as u64) + 1;
                return (
                    Some(SidebandPayload::AppendLines {
                        resolved_path: resolved_override,
                        using_fallback,
                        next_offset,
                        bytes: Vec::new(),
                    }),
                    next_offset < len,
                );
            }
            skipped += n as u64;
        }
    }
    (None, false)
}

async fn scan_grok_session_file(sessions_dir: &Path, session_id: &str) -> Option<PathBuf> {
    let mut dir = tokio::fs::read_dir(sessions_dir).await.ok()?;
    while let Ok(Some(entry)) = dir.next_entry().await {
        let candidate = entry.path().join(session_id).join("updates.jsonl");
        if tokio::fs::metadata(&candidate)
            .await
            .is_ok_and(|m| m.is_file())
        {
            return Some(candidate);
        }
    }
    None
}

async fn fetch_gemini_json(
    gemini_home: &Path,
    cwd: &str,
    session_id: &str,
    resolved_path: Option<&Path>,
    last_stamp: (u64, u64),
) -> Option<SidebandPayload> {
    let target = match resolved_path {
        Some(p) if tokio::fs::metadata(p).await.is_ok_and(|m| m.is_file()) => p.to_path_buf(),
        _ => resolve_gemini_chat_file(gemini_home, cwd, session_id).await?,
    };
    let meta = tokio::fs::metadata(&target).await.ok()?;
    if !meta.is_file() || meta.len() > GEMINI_SNAPSHOT_CAP {
        return None;
    }
    let stamp = file_stamp(&meta);
    if stamp == last_stamp {
        return None;
    }
    let bytes = tokio::fs::read(&target).await.ok()?;
    Some(SidebandPayload::GeminiJson {
        resolved_path: target,
        stamp,
        bytes,
    })
}

async fn resolve_gemini_chat_file(
    gemini_home: &Path,
    cwd: &str,
    session_id: &str,
) -> Option<PathBuf> {
    let prefix: String = session_id.chars().take(8).collect();
    if prefix.is_empty() {
        return None;
    }
    let tmp_root = gemini_home.join("tmp");

    // 1. Fast path via `<gemini_home>/projects.json` mapping `cwd -> slug`.
    if let Ok(proj_bytes) = tokio::fs::read(gemini_home.join("projects.json")).await {
        if let Ok(proj_json) = serde_json::from_slice::<Value>(&proj_bytes) {
            if let Some(slug) = proj_json
                .get("projects")
                .and_then(|p| p.get(cwd))
                .and_then(Value::as_str)
            {
                if let Some(hit) =
                    find_matching_session_in_chats(&tmp_root.join(slug).join("chats"), &prefix)
                        .await
                {
                    return Some(hit);
                }
            }
        }
    }

    // 2. Scan `<gemini_home>/tmp/*/chats/session-*<prefix>*.json`.
    let mut dirs = tokio::fs::read_dir(&tmp_root).await.ok()?;
    while let Ok(Some(entry)) = dirs.next_entry().await {
        let chats = entry.path().join("chats");
        if let Some(hit) = find_matching_session_in_chats(&chats, &prefix).await {
            return Some(hit);
        }
    }
    None
}

async fn find_matching_session_in_chats(chats_dir: &Path, prefix: &str) -> Option<PathBuf> {
    let mut entries = tokio::fs::read_dir(chats_dir).await.ok()?;
    let mut newest: Option<(u64, PathBuf)> = None;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("session-") || !name.ends_with(".json") || !name.contains(prefix) {
            continue;
        }
        let path = entry.path();
        if let Ok(meta) = entry.metadata().await {
            let mtime = file_stamp(&meta).0;
            if newest.as_ref().is_none_or(|(prev, _)| mtime >= *prev) {
                newest = Some((mtime, path));
            }
        }
    }
    newest.map(|(_, p)| p)
}

async fn fetch_opencode_sqlite(
    db_path: &Path,
    session_id: &str,
    min_time_ms: i64,
    last_stamp: (u64, u64, u64, u64),
) -> Option<SidebandPayload> {
    let db_meta = tokio::fs::metadata(db_path).await.ok()?;
    if !db_meta.is_file() {
        return None;
    }
    let (db_m, db_l) = file_stamp(&db_meta);
    let wal_path = PathBuf::from(format!("{}-wal", db_path.display()));
    let (wal_m, wal_l) = tokio::fs::metadata(&wal_path)
        .await
        .ok()
        .map(|m| file_stamp(&m))
        .unwrap_or((0, 0));
    let stamp = (db_m, db_l, wal_m, wal_l);
    if stamp == last_stamp {
        return None;
    }

    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .read_only(true)
        .create_if_missing(false);
    let mut conn = SqliteConnection::connect_with(&opts).await.ok()?;
    let rows = sqlx::query(
        "SELECT p.id, p.message_id, p.time_created, p.time_updated, p.data, \
         COALESCE(json_extract(m.data, '$.role'), 'assistant') AS role \
         FROM part p \
         JOIN message m ON p.message_id = m.id \
         WHERE p.session_id = ?1 AND p.time_updated >= ?2 \
         ORDER BY p.time_created ASC, p.id ASC \
         LIMIT 256",
    )
    .bind(session_id)
    .bind(min_time_ms)
    .fetch_all(&mut conn)
    .await
    .ok()?;
    let _ = conn.close().await;

    let mut rows_jsonl = Vec::new();
    for r in rows {
        let rec = json!({
            "id": r.try_get::<String, _>(0).unwrap_or_default(),
            "message_id": r.try_get::<String, _>(1).unwrap_or_default(),
            "time_created": r.try_get::<i64, _>(2).unwrap_or(0),
            "time_updated": r.try_get::<i64, _>(3).unwrap_or(0),
            "data": r.try_get::<String, _>(4).unwrap_or_default(),
            "role": r.try_get::<String, _>(5).unwrap_or_else(|_| "assistant".into()),
        });
        rows_jsonl.extend_from_slice(rec.to_string().as_bytes());
        rows_jsonl.push(b'\n');
    }

    Some(SidebandPayload::OpencodeSqlite { stamp, rows_jsonl })
}

fn file_stamp(meta: &std::fs::Metadata) -> (u64, u64) {
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    (mtime_ns, meta.len())
}
