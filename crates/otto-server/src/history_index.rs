//! History index scan (design §4.6). Walks `~/.claude/projects/*/*.jsonl` (top
//! level only — `subagents/` files are children, not sessions) and
//! `$CODEX_HOME/sessions/**/rollout-*.jsonl`, skips files whose `(mtime, size)`
//! the index already has, peeks head 64 KB + tail 16 KB of the rest, and
//! upserts `transcript_index`. Tolerates files appearing/vanishing mid-walk.
//! Runs at daemon boot (low priority: yields between files) and on
//! `POST /workspaces/{wid}/history/rescan`; progress goes out as
//! `history_index_progress` on the requesting workspace. One scan at a time.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use otto_core::event::Event;
use otto_core::Id;
use otto_state::{TranscriptIndexRepo, TranscriptIndexRow};
use otto_transcript::Provider;

use crate::state::ServerCtx;

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Emit progress every this many files.
const PROGRESS_EVERY: u64 = 25;

pub fn is_running() -> bool {
    RUNNING.load(Ordering::Relaxed)
}

/// Start a scan unless one is already running. Returns whether it started.
pub fn spawn_scan(ctx: ServerCtx, workspace_id: Option<Id>) -> bool {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return false;
    }
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        match scan(&ctx, workspace_id.as_ref()).await {
            Ok((scanned, refreshed)) => tracing::info!(
                scanned,
                refreshed,
                secs = started.elapsed().as_secs(),
                "history index: scan complete"
            ),
            Err(e) => tracing::warn!("history index: scan failed: {e}"),
        }
        RUNNING.store(false, Ordering::Release);
    });
    true
}

/// One transcript file found on disk.
#[derive(Debug, Clone)]
pub struct Found {
    pub path: PathBuf,
    pub provider: Provider,
    pub mtime: i64,
    pub size: i64,
}

fn stat(path: &Path, provider: Provider) -> Option<Found> {
    let m = std::fs::metadata(path).ok()?;
    if !m.is_file() {
        return None;
    }
    let mtime = m
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    Some(Found {
        path: path.to_path_buf(),
        provider,
        mtime,
        size: m.len() as i64,
    })
}

/// Every transcript file under the two roots, plus whether any directory
/// failed to list (a transient error must never look like "everything was
/// deleted" to the prune step).
pub fn list_transcripts(claude_root: &Path, codex_root: &Path) -> (Vec<Found>, bool) {
    let mut out = Vec::new();
    let mut errors = false;
    // Claude: `<root>/<slug>/<sid>.jsonl` — one level, no recursion into
    // `<sid>/subagents/`.
    match std::fs::read_dir(claude_root) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => errors = true,
        Err(_) => {}
        Ok(slugs) => for slug in slugs.flatten() {
            let Ok(files) = std::fs::read_dir(slug.path()) else {
                errors = true;
                continue;
            };
            for f in files.flatten() {
                let p = f.path();
                if p.extension().is_some_and(|e| e == "jsonl") {
                    if let Some(found) = stat(&p, Provider::Claude) {
                        out.push(found);
                    }
                }
            }
        }
    }
    // Codex: `<root>/YYYY/MM/DD/rollout-*.jsonl` (bounded walk).
    fn walk(dir: &Path, depth: usize, out: &mut Vec<Found>, errors: &mut bool) {
        if depth > 5 {
            return;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                if e.kind() != std::io::ErrorKind::NotFound {
                    *errors = true;
                }
                return;
            }
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, depth + 1, out, errors);
            } else if p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
            {
                if let Some(found) = stat(&p, Provider::Codex) {
                    out.push(found);
                }
            }
        }
    }
    walk(codex_root, 0, &mut out, &mut errors);
    (out, errors)
}

/// Peek one file into an index row (blocking IO).
pub fn index_row(found: &Found) -> std::io::Result<TranscriptIndexRow> {
    let peek = otto_transcript::peek(found.provider, &found.path)?;
    Ok(TranscriptIndexRow {
        path: found.path.to_string_lossy().into_owned(),
        provider: found.provider.as_str().to_string(),
        provider_session_id: peek.provider_session_id,
        cwd: peek.cwd,
        title: peek.title,
        first_prompt: peek.first_prompt,
        started_at: peek.started_at.or_else(|| iso_from_unix(found.mtime)),
        last_active_at: peek.last_active_at.or_else(|| iso_from_unix(found.mtime)),
        mtime: found.mtime,
        size: found.size,
        turns: peek.turns.map(|t| t as i64),
        indexed_at: String::new(),
    })
}

fn iso_from_unix(secs: i64) -> Option<String> {
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, 0).map(|d| d.to_rfc3339())
}

/// The scan proper. Returns `(files seen, files (re)indexed)`.
async fn scan(ctx: &ServerCtx, workspace_id: Option<&Id>) -> otto_core::Result<(u64, u64)> {
    let repo = TranscriptIndexRepo::new(ctx.pool.clone());
    let (claude_root, codex_root) = crate::routes::transcript::transcript_roots(&ctx.data_dir);
    let (files, list_errors) = tokio::task::spawn_blocking(move || list_transcripts(&claude_root, &codex_root))
        .await
        .map_err(|e| otto_core::Error::Internal(format!("list transcripts: {e}")))?;
    let total = files.len() as u64;
    let stamps = repo.stamps().await?;
    let mut seen: HashSet<String> = HashSet::with_capacity(files.len());
    let mut scanned = 0u64;
    let mut refreshed = 0u64;
    let progress = |scanned: u64, done: bool| {
        if let Some(wid) = workspace_id {
            let _ = ctx.events.send(Event::HistoryIndexProgress {
                workspace_id: wid.clone(),
                scanned,
                total,
                done,
            });
        }
    };
    for found in files {
        scanned += 1;
        let key = found.path.to_string_lossy().into_owned();
        seen.insert(key.clone());
        let unchanged = stamps.get(&key).is_some_and(|&(m, s)| m == found.mtime && s == found.size);
        if !unchanged {
            let f = found.clone();
            match tokio::task::spawn_blocking(move || index_row(&f)).await {
                Ok(Ok(row)) => {
                    if let Err(e) = repo.upsert(&row).await {
                        tracing::debug!(path = %key, "history index: upsert failed: {e}");
                    } else {
                        refreshed += 1;
                    }
                }
                // Vanished mid-walk / unreadable: skip, keep any old row.
                Ok(Err(e)) => tracing::debug!(path = %key, "history index: peek failed: {e}"),
                Err(_) => {}
            }
            // Low priority: give the interactive paths the SQLite writer back.
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        if scanned.is_multiple_of(PROGRESS_EVERY) {
            progress(scanned, false);
        }
    }
    // Prune only from a complete, non-empty walk: an unmounted/unreadable
    // root (or a brand-new machine) must not wipe the index.
    if total > 0 && !list_errors {
        let removed = repo.retain(&seen).await.unwrap_or(0);
        if removed > 0 {
            tracing::info!(removed, "history index: dropped rows for deleted transcripts");
        }
    } else if list_errors {
        tracing::warn!("history index: a root failed to list; skipping prune");
    }
    progress(scanned, true);
    Ok((scanned, refreshed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_top_level_claude_and_codex_rollouts_only() {
        let dir = tempfile::tempdir().unwrap();
        let claude = dir.path().join("projects");
        let slug = claude.join("-Users-u-repo");
        std::fs::create_dir_all(slug.join("sid1").join("subagents")).unwrap();
        std::fs::write(slug.join("sid1.jsonl"), "{\"type\":\"user\"}\n").unwrap();
        std::fs::write(slug.join("sid1").join("subagents").join("agent-x.jsonl"), "{}\n").unwrap();
        std::fs::write(slug.join("notes.txt"), "x").unwrap();
        let codex = dir.path().join("sessions");
        let day = codex.join("2026").join("07").join("01");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-2026-07-01T00-00-00-abc.jsonl"), "{}\n").unwrap();
        std::fs::write(day.join("other.jsonl"), "{}\n").unwrap();
        let (found, errors) = list_transcripts(&claude, &codex);
        assert!(!errors);
        let names: Vec<String> = found
            .iter()
            .map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found.len(), 2, "{names:?}");
        assert!(names.contains(&"sid1.jsonl".to_string()));
        assert!(names.iter().any(|n| n.starts_with("rollout-")));
        assert!(found.iter().all(|f| f.size > 0 && f.mtime > 0));
        // A missing root is simply empty.
        let (none, errors) = list_transcripts(&dir.path().join("nope"), &dir.path().join("nope2"));
        assert!(none.is_empty() && !errors, "a missing root is empty, not an error");
    }

    #[test]
    fn index_row_falls_back_to_mtime_for_dates() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s.jsonl");
        std::fs::write(&p, "{\"type\":\"user\",\"uuid\":\"u\",\"sessionId\":\"s\",\"cwd\":\"/r\",\"message\":{\"role\":\"user\",\"content\":\"hey\"}}\n").unwrap();
        let f = stat(&p, Provider::Claude).unwrap();
        let row = index_row(&f).unwrap();
        assert_eq!(row.provider_session_id.as_deref(), Some("s"));
        assert_eq!(row.first_prompt.as_deref(), Some("hey"));
        assert!(row.started_at.is_some(), "no timestamps in the file → mtime");
        assert_eq!(row.turns, Some(1));
    }
}
