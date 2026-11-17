//! Find the newest Codex limit snapshot under one sessions root.
//!
//! The rollouts already hold it (every `token_count` line), so nothing is
//! stored: a read walks the newest day directories, opens the most recently
//! written rollouts and reads each one's tail backwards until a snapshot turns
//! up. A rollout whose turn the plan refused carries no numbers, only the
//! `usage_limit_exceeded` error: that is kept as the newest limit hit and the
//! walk goes on to older rollouts for the last real snapshot (an hourly job
//! failing for days writes dozens of such rollouts). A short per-root cache
//! keeps a page that polls from re-reading files.
//! Blocking by contract — the async entry point runs it on the blocking pool.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};

use otto_transcript::limits::{parse_codex_rate_limits, CodexLimitsLine, LimitSnapshot};

/// Day directories (`YYYY/MM/DD`) looked at, newest first.
const MAX_DAY_DIRS: usize = 10;
/// Rollouts opened per read at most, most recently written first; the walk
/// stops at the first one holding a main snapshot.
const MAX_FILES: usize = 64;
/// Bytes read from the end of each rollout; token_count lines are frequent,
/// so the newest one sits well inside this.
const TAIL_BYTES: u64 = 1024 * 1024;
/// Bytes read per walk in all (refused turns' rollouts are a few KB each).
const MAX_BYTES: u64 = 16 * TAIL_BYTES;
/// How long one root's answer is reused.
const CACHE_TTL: Duration = Duration::from_secs(15);

/// What one root's newest rollouts say.
#[derive(Debug, Clone, Default)]
pub(crate) struct Found {
    /// The newest main-allowance snapshot.
    pub main: Option<LimitSnapshot>,
    /// Model-specific buckets seen beside it (one per `limit_id`).
    pub extra: Vec<LimitSnapshot>,
    /// At least one rollout exists under the root.
    pub any_rollout: bool,
    /// A `token_count` line said `rate_limits: null` (API-key login).
    pub saw_no_limits: bool,
    /// The newest turn refused with `usage_limit_exceeded`.
    pub limit_hit: Option<LimitHit>,
}

/// A turn the plan refused.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LimitHit {
    pub at: DateTime<Utc>,
    /// From the error's "try again at …", when it parsed.
    pub resets_at: Option<DateTime<Utc>>,
}

/// [`scan_root`] behind a [`CACHE_TTL`] per-root cache.
pub(crate) fn latest_cached(root: &Path) -> Arc<Found> {
    type Cache = Mutex<HashMap<PathBuf, (Instant, Arc<Found>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((at, found)) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(root) {
        if at.elapsed() < CACHE_TTL {
            return found.clone();
        }
    }
    let found = Arc::new(scan_root(root));
    let mut map = cache.lock().unwrap_or_else(|e| e.into_inner());
    if map.len() > 64 {
        map.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
    }
    map.insert(root.to_path_buf(), (Instant::now(), found.clone()));
    found
}

/// The newest snapshot per `limit_id` under `root` (no cache).
pub(crate) fn scan_root(root: &Path) -> Found {
    let mut found = Found::default();
    let files = newest_rollouts(root);
    found.any_rollout = !files.is_empty();
    let mut read = 0;
    for file in files {
        read += scan_file(&file, &mut found);
        if found.main.is_some() || read >= MAX_BYTES {
            break;
        }
    }
    found
}

/// Fold one rollout's tail into `found`: lines newest first, the first
/// snapshot per `limit_id` wins (an older file never overrides a newer one).
/// Returns the bytes read.
fn scan_file(file: &Path, found: &mut Found) -> u64 {
    let Some(text) = read_tail(file) else {
        return 0;
    };
    for line in text.lines().rev() {
        match parse_codex_rate_limits(line) {
            Some(CodexLimitsLine::Snapshot(s)) => {
                if s.is_main() {
                    if found.main.is_none() {
                        found.main = Some(s);
                    }
                } else if !found.extra.iter().any(|e| e.limit_id == s.limit_id) {
                    found.extra.push(s);
                }
            }
            Some(CodexLimitsLine::NoLimits) => found.saw_no_limits = true,
            Some(CodexLimitsLine::LimitHit { at, resets_at })
                if found.limit_hit.is_none_or(|h| at > h.at) =>
            {
                found.limit_hit = Some(LimitHit { at, resets_at });
            }
            _ => {}
        }
    }
    text.len() as u64
}

/// The last [`TAIL_BYTES`] of `file`, starting at a line boundary.
fn read_tail(file: &Path) -> Option<String> {
    let mut f = std::fs::File::open(file).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut buf).ok()?;
    let body = if start > 0 {
        // Drop the partial first line.
        let cut = buf
            .iter()
            .position(|b| *b == b'\n')
            .map_or(buf.len(), |i| i + 1);
        &buf[cut..]
    } else {
        &buf[..]
    };
    Some(String::from_utf8_lossy(body).into_owned())
}

/// Rollouts under the newest [`MAX_DAY_DIRS`] day directories, most recently
/// written first, at most [`MAX_FILES`]. A long session started days ago
/// still sorts first by its write time.
fn newest_rollouts(root: &Path) -> Vec<PathBuf> {
    let mut days = Vec::new();
    'walk: for year in numeric_subdirs(root) {
        for month in numeric_subdirs(&year) {
            for day in numeric_subdirs(&month) {
                days.push(day);
                if days.len() >= MAX_DAY_DIRS {
                    break 'walk;
                }
            }
        }
    }
    // Flat roots (an isolated e2e fixture dir) hold rollouts directly.
    days.push(root.to_path_buf());
    let mut files: Vec<(SystemTime, PathBuf)> = days
        .iter()
        .flat_map(|d| list_dir(d.as_path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
        })
        .filter_map(|p| {
            let meta = std::fs::metadata(&p).ok()?;
            meta.is_file()
                .then(|| (meta.modified().unwrap_or(SystemTime::UNIX_EPOCH), p))
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.truncate(MAX_FILES);
    files.into_iter().map(|(_, p)| p).collect()
}

/// All-digit subdirectories (`2026`, `11`, `12`), highest first.
fn numeric_subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<(u32, PathBuf)> = list_dir(dir)
        .into_iter()
        .filter(|p| p.is_dir())
        .filter_map(|p| {
            let n: u32 = p.file_name()?.to_str()?.parse().ok()?;
            Some((n, p))
        })
        .collect();
    out.sort_by_key(|d| std::cmp::Reverse(d.0));
    out.into_iter().map(|(_, p)| p).collect()
}

#[allow(clippy::disallowed_methods)] // blocking by contract: run on the blocking pool (limits::remaining_limits)
fn list_dir(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .map(|rd| rd.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token_line(ts: &str, limit_id: &str, used: f64) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":null,"rate_limits":{{"limit_id":"{limit_id}","limit_name":null,"primary":{{"used_percent":{used},"window_minutes":300,"resets_at":1794983830}},"secondary":null,"credits":null,"plan_type":"pro","rate_limit_reached_type":null}}}}}}"#
        )
    }

    fn write(path: &Path, lines: &[String]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    #[test]
    fn newest_line_of_the_newest_rollout_wins() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("2026/11/10/rollout-a.jsonl");
        write(&old, &[token_line("2026-11-10T10:00:00Z", "codex", 90.0)]);
        let new = dir.path().join("2026/11/12/rollout-b.jsonl");
        write(
            &new,
            &[
                token_line("2026-11-12T10:00:00Z", "codex", 10.0),
                token_line("2026-11-12T10:05:00Z", "codex", 12.0),
                token_line("2026-11-12T10:05:01Z", "codex_other", 3.0),
            ],
        );
        // The older day's file was written first.
        let earlier = SystemTime::now() - Duration::from_secs(3600);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(earlier)
            .unwrap();
        let found = scan_root(dir.path());
        assert!(found.any_rollout);
        let main = found.main.unwrap();
        assert_eq!(main.windows[0].used_percent, 12.0);
        assert_eq!(found.extra.len(), 1);
        assert_eq!(found.extra[0].limit_id.as_deref(), Some("codex_other"));
    }

    #[test]
    fn api_key_rollouts_and_empty_roots_are_told_apart() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!scan_root(dir.path()).any_rollout);
        write(
            &dir.path().join("2026/11/12/rollout-c.jsonl"),
            &[r#"{"timestamp":"2026-11-12T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{},"rate_limits":null}}"#.to_string()],
        );
        let found = scan_root(dir.path());
        assert!(found.any_rollout && found.saw_no_limits && found.main.is_none());
    }

    /// What a `codex exec` turn writes when the plan is used up.
    fn refused_turn(ts: &str) -> Vec<String> {
        vec![
            format!(
                r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":null,"rate_limits":{{"limit_id":"premium","limit_name":null,"primary":null,"secondary":null,"credits":{{"has_credits":false,"unlimited":false,"balance":"0"}},"plan_type":null,"rate_limit_reached_type":null}}}}}}"#
            ),
            format!(
                r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"task_complete","last_agent_message":null,"error":{{"message":"You’ve hit your usage limit. Visit https://chatgpt.com/settings/usage to purchase more credits or try again at Nov 18th, 2026 8:42 AM.","codex_error_info":"usage_limit_exceeded"}}}}}}"#
            ),
        ]
    }

    #[test]
    fn refused_turns_do_not_hide_the_last_real_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("2026/11/13/rollout-real.jsonl");
        write(&real, &[token_line("2026-11-13T09:33:37Z", "codex", 100.0)]);
        let base = SystemTime::now() - Duration::from_secs(40 * 3600);
        std::fs::File::options()
            .write(true)
            .open(&real)
            .unwrap()
            .set_modified(base)
            .unwrap();
        // More failed hourly runs than the old 16-file walk looked at.
        for i in 0..30u64 {
            let f = dir
                .path()
                .join(format!("2026/11/16/rollout-fail-{i:02}.jsonl"));
            write(
                &f,
                &refused_turn(&format!("2026-11-16T{:02}:00:00Z", i % 24)),
            );
            std::fs::File::options()
                .write(true)
                .open(&f)
                .unwrap()
                .set_modified(base + Duration::from_secs(3600 * (i + 1)))
                .unwrap();
        }
        let found = scan_root(dir.path());
        let main = found.main.expect("the walk reached the real snapshot");
        assert_eq!(main.windows[0].used_percent, 100.0);
        assert!(
            found.extra.is_empty(),
            "the windowless placeholders are skipped"
        );
        assert!(
            !found.saw_no_limits,
            "a refused turn is not an API-key login"
        );
        let hit = found.limit_hit.unwrap();
        assert!(hit.at > main.taken_at);
        assert!(hit.resets_at.is_some());
    }

    #[test]
    fn a_large_rollout_is_read_from_its_tail() {
        let dir = tempfile::tempdir().unwrap();
        let filler = format!(
            r#"{{"timestamp":"2026-11-12T09:00:00Z","type":"response_item","payload":{{"text":"{}"}}}}"#,
            "x".repeat(4096)
        );
        let mut lines = vec![token_line("2026-11-12T08:00:00Z", "codex", 50.0)];
        lines.extend(std::iter::repeat_n(filler, 400));
        lines.push(token_line("2026-11-12T10:00:00Z", "codex", 7.0));
        write(&dir.path().join("rollout-flat.jsonl"), &lines);
        let main = scan_root(dir.path()).main.unwrap();
        assert_eq!(main.windows[0].used_percent, 7.0);
    }
}
