//! Async off-lock filesystem & read-only SQLite sideband polling for
//! [`crate::runtime`].
//!
//! All disk/DB I/O executes outside the session mutex so [`crate::acp::AcpDriver`]
//! remains strictly sans-IO.

use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde_json::{json, Value};
use sqlx::Row;
use tokio::io::{AsyncReadExt, AsyncSeekExt};

use super::burst::now_ms;
use super::confinement::{
    is_forbidden_tail_target, validate_confined_tail_path_scoped,
    validate_existing_or_pending_tail_path_scoped,
};
use crate::driver::{
    ActiveTailChunk, ActiveTailKind, ActiveTailSource, SidebandPayload, SidebandPoll,
    SidebandResumeSeed, SidebandSource, StructuredDriver,
};

const SIDEBAND_READ_CAP: usize = 2 * 1024 * 1024;
const ACTIVE_TAIL_READ_CAP: usize = 64 * 1024;
const MAX_ACTIVE_TAILS: usize = 8;
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
        SidebandSource::None => {}
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
    let (primary, mut has_more) = fetch_primary_sideband(&poll.source).await;
    if poll.active_tails.is_empty() && !poll.timer_expired {
        return (primary, has_more);
    }

    let mut tails = Vec::new();
    for tail in poll.active_tails.iter().take(MAX_ACTIVE_TAILS) {
        let (chunk, more) = fetch_active_tail(tail).await;
        if let Some(c) = chunk {
            tails.push(c);
        }
        has_more |= more;
    }

    if primary.is_none() && tails.is_empty() && !poll.timer_expired {
        return (None, false);
    }

    (
        Some(SidebandPayload::Batch {
            primary: primary.map(Box::new),
            tails,
            timer_tick: poll.timer_expired,
        }),
        has_more,
    )
}

async fn fetch_primary_sideband(source: &SidebandSource) -> (Option<SidebandPayload>, bool) {
    match source {
        SidebandSource::None => (None, false),
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
                SIDEBAND_READ_CAP,
            )
            .await
        }
        SidebandSource::GeminiJson {
            gemini_home,
            cwd,
            session_id,
            resolved_path,
            last_stamp,
            allow_scan,
        } => {
            let payload = fetch_gemini_json(
                gemini_home,
                cwd,
                session_id,
                resolved_path.as_deref(),
                *last_stamp,
                *allow_scan,
            )
            .await;
            (payload, false)
        }
        SidebandSource::OpencodeSqlite {
            db_path,
            session_id,
            min_time_ms,
            min_part_id,
            last_stamp,
        } => {
            let payload =
                fetch_opencode_sqlite(db_path, session_id, *min_time_ms, min_part_id, *last_stamp)
                    .await;
            let more = matches!(
                &payload,
                Some(SidebandPayload::OpencodeSqlite { has_more: true, .. })
            );
            (payload, more)
        }
    }
}

async fn fetch_active_tail(tail: &ActiveTailSource) -> (Option<ActiveTailChunk>, bool) {
    match tail.kind {
        ActiveTailKind::AgySubagentTranscript => {
            let tail_cl = tail.clone();
            let validated = tokio::task::spawn_blocking(move || {
                let valid_primary = validate_existing_or_pending_tail_path_scoped(
                    &tail_cl.path,
                    tail_cl.kind,
                    tail_cl.allowed_cwd.as_deref(),
                    tail_cl.agy_brain_dir.as_deref(),
                    tail_cl.allowed_agy_conv_ids.as_deref(),
                    tail_cl.otto_session_id.as_deref(),
                )?;
                let valid_fallback = tail_cl.fallback_path.as_deref().and_then(|fb| {
                    validate_existing_or_pending_tail_path_scoped(
                        fb,
                        tail_cl.kind,
                        tail_cl.allowed_cwd.as_deref(),
                        tail_cl.agy_brain_dir.as_deref(),
                        tail_cl.allowed_agy_conv_ids.as_deref(),
                        tail_cl.otto_session_id.as_deref(),
                    )
                });
                Some((valid_primary, valid_fallback))
            })
            .await
            .ok()
            .flatten();
            let Some((valid_primary, valid_fallback)) = validated else {
                return (None, false);
            };
            let (payload, more) = fetch_append_lines(
                &valid_primary,
                valid_fallback.as_deref(),
                None,
                tail.offset,
                tail.using_fallback,
                ACTIVE_TAIL_READ_CAP,
            )
            .await;
            match payload {
                Some(SidebandPayload::AppendLines {
                    using_fallback,
                    next_offset,
                    bytes,
                    ..
                }) => (
                    Some(ActiveTailChunk {
                        task_id: tail.task_id.clone(),
                        kind: tail.kind,
                        using_fallback,
                        next_offset,
                        bytes,
                    }),
                    more,
                ),
                _ => (None, false),
            }
        }
        ActiveTailKind::CommandLog => fetch_raw_log_chunk(tail).await,
    }
}

async fn fetch_raw_log_chunk(tail: &ActiveTailSource) -> (Option<ActiveTailChunk>, bool) {
    let tail_cl = tail.clone();
    let Some(canon_path) = tokio::task::spawn_blocking(move || {
        validate_confined_tail_path_scoped(
            &tail_cl.path,
            ActiveTailKind::CommandLog,
            tail_cl.allowed_cwd.as_deref(),
            tail_cl.agy_brain_dir.as_deref(),
            tail_cl.allowed_agy_conv_ids.as_deref(),
            tail_cl.otto_session_id.as_deref(),
        )
    })
    .await
    .ok()
    .flatten() else {
        return (None, false);
    };
    let Ok(mut file) = tokio::fs::File::open(&canon_path).await else {
        return (None, false);
    };
    let Ok(meta) = file.metadata().await else {
        return (None, false);
    };
    let len = meta.len();
    let start_offset = if len < tail.offset { 0 } else { tail.offset };
    if len == start_offset {
        return (None, false);
    }
    if file
        .seek(std::io::SeekFrom::Start(start_offset))
        .await
        .is_err()
    {
        return (None, false);
    }
    let to_read = ((len - start_offset) as usize).min(ACTIVE_TAIL_READ_CAP);
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
    if read_total == 0 {
        return (None, false);
    }
    let more_after_read = start_offset + (read_total as u64) < len;
    let safe_len = utf8_safe_prefix_len(&buf[..read_total], more_after_read);
    if safe_len == 0 {
        return (None, false);
    }
    buf.truncate(safe_len);
    let next_offset = start_offset + safe_len as u64;
    (
        Some(ActiveTailChunk {
            task_id: tail.task_id.clone(),
            kind: ActiveTailKind::CommandLog,
            using_fallback: false,
            next_offset,
            bytes: buf,
        }),
        next_offset < len,
    )
}

fn utf8_safe_prefix_len(bytes: &[u8], has_more: bool) -> usize {
    match std::str::from_utf8(bytes) {
        Ok(_) => bytes.len(),
        Err(e) => {
            if e.error_len().is_none() {
                // Incomplete multi-byte sequence at end of buffer: hold those
                // 1..3 trailing bytes for the next read unless nothing was valid.
                if e.valid_up_to() > 0 || has_more {
                    e.valid_up_to()
                } else {
                    bytes.len()
                }
            } else {
                // Non-UTF-8 byte inside log output: decode lossily across full slice.
                bytes.len()
            }
        }
    }
}

async fn fetch_append_lines(
    path: &Path,
    fallback_path: Option<&Path>,
    scan_sessions_dir: Option<&(PathBuf, String)>,
    offset: u64,
    prev_using_fallback: bool,
    read_cap: usize,
) -> (Option<SidebandPayload>, bool) {
    let mut resolved_override: Option<PathBuf> = None;
    let open_nonsymlink = |p: &Path| {
        let ok = std::fs::symlink_metadata(p)
            .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
            && !is_forbidden_tail_target(p);
        let p = p.to_path_buf();
        async move {
            if !ok {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "denied tail target",
                ));
            }
            tokio::fs::File::open(&p).await
        }
    };
    let (using_fallback, file_res) = match open_nonsymlink(path).await {
        Ok(f) => (false, Ok(f)),
        Err(_) => match fallback_path {
            Some(fb) => (true, open_nonsymlink(fb).await),
            None => match scan_sessions_dir {
                Some((sessions_dir, sid)) => {
                    let Some(found) = scan_grok_session_file(sessions_dir, sid).await else {
                        return (Some(SidebandPayload::Unresolved), false);
                    };
                    let open_res = open_nonsymlink(&found).await;
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
    let to_read = ((len - start_offset) as usize).min(read_cap);
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

    // If a single line exceeded `read_cap` (e.g. huge base64 tool blob),
    // skip forward to its terminating '\n' so the tailer never wedges at start_offset.
    if read_total == read_cap && start_offset + (read_total as u64) < len {
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
    allow_scan: bool,
) -> Option<SidebandPayload> {
    let target = match resolved_path {
        Some(p) if tokio::fs::metadata(p).await.is_ok_and(|m| m.is_file()) => p.to_path_buf(),
        _ => {
            if !allow_scan {
                return None;
            }
            match resolve_gemini_chat_file(gemini_home, cwd, session_id).await {
                Some(found) => found,
                None => return Some(SidebandPayload::Unresolved),
            }
        }
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
    let doc = if bytes.len() > 64 * 1024 {
        tokio::task::spawn_blocking(move || serde_json::from_slice::<Value>(&bytes))
            .await
            .ok()?
            .ok()?
    } else {
        serde_json::from_slice::<Value>(&bytes).ok()?
    };
    Some(SidebandPayload::GeminiJson {
        resolved_path: target,
        stamp,
        doc,
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
    min_part_id: &str,
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

    const PAGE_LIMIT: usize = 256;
    let pool = super::opencode::readonly_opencode_pool(db_path).await?;
    let rows_res = sqlx::query(
        "SELECT p.id, p.message_id, p.time_created, p.time_updated, p.data, \
         COALESCE(json_extract(m.data, '$.role'), 'assistant') AS role, \
         COALESCE(m.time_created, CAST(json_extract(m.data, '$.time.created') AS INTEGER), p.time_created, 0) AS msg_time_created, \
         p.session_id \
         FROM part p \
         JOIN message m ON p.message_id = m.id \
         WHERE (p.session_id = ?1 OR p.session_id IN (SELECT id FROM session WHERE parent_id = ?1)) \
           AND (p.time_updated > ?2 OR (p.time_updated = ?2 AND p.id > ?3)) \
         ORDER BY p.time_updated ASC, p.id ASC \
         LIMIT 257",
    )
    .bind(session_id)
    .bind(min_time_ms)
    .bind(min_part_id)
    .fetch_all(&pool)
    .await;

    let rows = match rows_res {
        Ok(rows) => rows,
        Err(_) => {
            let fallback_res = sqlx::query(
                "SELECT p.id, p.message_id, p.time_created, p.time_updated, p.data, \
                 COALESCE(json_extract(m.data, '$.role'), 'assistant') AS role, \
                 COALESCE(CAST(json_extract(m.data, '$.time.created') AS INTEGER), p.time_created, 0) AS msg_time_created, \
                 p.session_id \
                 FROM part p \
                 JOIN message m ON p.message_id = m.id \
                 WHERE p.session_id = ?1 \
                   AND (p.time_updated > ?2 OR (p.time_updated = ?2 AND p.id > ?3)) \
                 ORDER BY p.time_updated ASC, p.id ASC \
                 LIMIT 257",
            )
            .bind(session_id)
            .bind(min_time_ms)
            .bind(min_part_id)
            .fetch_all(&pool)
            .await;
            match fallback_res {
                Ok(rows) => rows,
                Err(_) => {
                    super::opencode::evict_opencode_pool(db_path).await;
                    return None;
                }
            }
        }
    };

    let has_more = rows.len() > PAGE_LIMIT;
    let mut rows_jsonl = Vec::new();
    for r in rows.into_iter().take(PAGE_LIMIT) {
        let rec = json!({
            "id": r.try_get::<String, _>(0).unwrap_or_default(),
            "message_id": r.try_get::<String, _>(1).unwrap_or_default(),
            "time_created": r.try_get::<i64, _>(2).unwrap_or(0),
            "time_updated": r.try_get::<i64, _>(3).unwrap_or(0),
            "data": r.try_get::<String, _>(4).unwrap_or_default(),
            "role": r.try_get::<String, _>(5).unwrap_or_else(|_| "assistant".into()),
            "msg_time_created": r.try_get::<i64, _>(6).unwrap_or(0),
            "session_id": r.try_get::<String, _>(7).unwrap_or_else(|_| session_id.to_string()),
        });
        rows_jsonl.extend_from_slice(rec.to_string().as_bytes());
        rows_jsonl.push(b'\n');
    }

    Some(SidebandPayload::OpencodeSqlite {
        stamp,
        rows_jsonl,
        has_more,
    })
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
