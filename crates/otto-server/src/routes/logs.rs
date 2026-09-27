//! `GET /api/v1/logs/daemon` — safe read access to Otto daemon logs.
//! `POST /api/v1/client/errors` — the UI reports a fatal client-side error
//! (see the handler) so it lands in the daemon log next to what the daemon
//! was doing at that moment.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::{require_root, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;
use otto_core::Error;

const ALL_FILES: &str = "__all__";
const DEFAULT_TAIL_LINES: usize = 500;
const MAX_TAIL_LINES: usize = 50_000;
/// Upper bound on `content` per response. "All log files" used to return every
/// `ottod.log*` in full (11 MB measured) on every Live tick; anything over the
/// cap is dropped from the OLD end (at a line boundary) and `truncated` is set.
const MAX_CONTENT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct LogsParams {
    /// File name from `files[].name`, `__all__`, or empty for latest.
    file: Option<String>,
    /// `all`, `tail`, or `since`. `since` reads from byte `offset`.
    mode: Option<String>,
    /// Tail line count when `mode=tail`.
    lines: Option<usize>,
    /// Byte offset used by `mode=since`.
    offset: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogFileEntry {
    pub name: String,
    pub size: u64,
    pub modified_ms: u128,
}

#[derive(Debug, Clone, Serialize)]
pub struct DaemonLogs {
    pub log_dir: String,
    pub files: Vec<LogFileEntry>,
    pub selected: String,
    pub mode: String,
    pub content: String,
    /// Byte offset in the live file where `content` starts (see `next_offset`).
    pub offset: u64,
    /// Byte offset to pass as `offset` with `mode=since` to get only what was
    /// written after this response. It always refers to the LIVE file: the
    /// selected file, or for `__all__` the newest file (`files[last]`).
    pub next_offset: u64,
    /// True when older content was dropped to honour the response byte cap.
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadMode {
    All,
    Tail,
    Since,
}

impl ReadMode {
    fn parse(raw: Option<&str>) -> Self {
        match raw.unwrap_or("all") {
            "tail" => Self::Tail,
            "since" => Self::Since,
            _ => Self::All,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Tail => "tail",
            Self::Since => "since",
        }
    }
}

pub async fn daemon_logs(
    State(_ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(params): Query<LogsParams>,
) -> ApiResult<Json<DaemonLogs>> {
    require_root(&user)?;
    let log_dir = daemon_log_dir();
    // Directory walk + multi-MB reads: keep them off the async workers.
    tokio::task::spawn_blocking(move || read_daemon_logs(&log_dir, params))
        .await
        .map_err(|e| ApiError(Error::Internal(format!("read logs task: {e}"))))?
        .map(Json)
        .map_err(ApiError)
}

/// Same resolution order as `ottod`'s `Config::log_dir`: `$OTTO_LOG_DIR`, then
/// `$OTTO_DATA_DIR/logs` (test/dev daemons), then `~/Library/Logs/Otto`.
fn daemon_log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OTTO_LOG_DIR") {
        return PathBuf::from(dir);
    }
    if let Some(data) = std::env::var_os("OTTO_DATA_DIR") {
        return PathBuf::from(data).join("logs");
    }
    dirs::home_dir()
        .map(|h| h.join("Library/Logs/Otto"))
        .unwrap_or_else(|| {
            std::env::var_os("OTTO_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."))
                .join("logs")
        })
}

fn read_daemon_logs(log_dir: &Path, params: LogsParams) -> Result<DaemonLogs, Error> {
    let files = list_log_files(log_dir)?;
    let mode = ReadMode::parse(params.mode.as_deref());
    let selected = params
        .file
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| files.last().map(|f| f.name.as_str()).unwrap_or(ALL_FILES))
        .to_string();

    let chunk = if selected == ALL_FILES {
        read_all_files(log_dir, &files)?
    } else {
        let path = safe_log_path(log_dir, &selected, &files)?;
        match mode {
            ReadMode::All => read_since(&path, 0)
                .map_err(|e| Error::Internal(format!("read log {}: {e}", selected)))?,
            ReadMode::Tail => {
                let lines = params
                    .lines
                    .unwrap_or(DEFAULT_TAIL_LINES)
                    .clamp(1, MAX_TAIL_LINES);
                tail_lines(&path, lines)
                    .map_err(|e| Error::Internal(format!("tail log {}: {e}", selected)))?
            }
            ReadMode::Since => {
                let offset = params.offset.unwrap_or(0);
                read_since(&path, offset)
                    .map_err(|e| Error::Internal(format!("read log update {}: {e}", selected)))?
            }
        }
    };

    Ok(DaemonLogs {
        log_dir: log_dir.to_string_lossy().into_owned(),
        files,
        selected,
        mode: mode.as_str().to_string(),
        content: chunk.content,
        offset: chunk.offset,
        next_offset: chunk.next_offset,
        truncated: chunk.truncated,
    })
}

/// One read: `content` = bytes `[offset, next_offset)` of the live file (for
/// `__all__`, older files' text is prepended and `offset` is 0).
struct Chunk {
    content: String,
    offset: u64,
    next_offset: u64,
    truncated: bool,
}

/// Read `path[start..size]`, keeping at most `MAX_CONTENT_BYTES` from the end
/// (the cut moves forward to the next line start). Returns the text, the real
/// start offset, the size read up to, and whether anything was dropped.
fn read_capped(path: &Path, start: u64, cap: u64) -> std::io::Result<Chunk> {
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    let mut start = start.min(size);
    let mut truncated = false;
    if size - start > cap {
        start = size - cap;
        truncated = true;
    }
    file.seek(SeekFrom::Start(start))?;
    // Grown by read_to_end; `take` bounds it to `cap` (never pre-sized from the request).
    let mut bytes = Vec::new();
    (&mut file).take(size - start).read_to_end(&mut bytes)?;
    let mut skip = 0usize;
    if truncated {
        // Don't start mid-line (or mid-UTF-8 sequence).
        if let Some(nl) = bytes.iter().position(|&b| b == b'\n') {
            skip = nl + 1;
        }
    }
    Ok(Chunk {
        content: String::from_utf8_lossy(&bytes[skip..]).into_owned(),
        offset: start + skip as u64,
        next_offset: start + bytes.len() as u64,
        truncated,
    })
}

fn list_log_files(log_dir: &Path) -> Result<Vec<LogFileEntry>, Error> {
    let entries = std::fs::read_dir(log_dir)
        .map_err(|e| Error::Internal(format!("read log directory {}: {e}", log_dir.display())))?;
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("ottod.log") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map(|d| d.as_millis())
            .unwrap_or(0);
        files.push(LogFileEntry {
            name: name.to_string(),
            size: meta.len(),
            modified_ms,
        });
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

fn safe_log_path(log_dir: &Path, selected: &str, files: &[LogFileEntry]) -> Result<PathBuf, Error> {
    if selected.contains('/') || selected.contains('\\') || selected.contains("..") {
        return Err(Error::Invalid("invalid log file name".into()));
    }
    if !files.iter().any(|f| f.name == selected) {
        return Err(Error::NotFound(format!("log file '{selected}'")));
    }
    Ok(log_dir.join(selected))
}

/// Every `ottod.log*` concatenated (oldest first, each under a `=====` header),
/// newest-first within the byte budget: once `MAX_CONTENT_BYTES` is spent the
/// older files are left out and `truncated` is set. `next_offset` is the byte
/// position in the NEWEST file, so a client can follow it with `mode=since`.
fn read_all_files(log_dir: &Path, files: &[LogFileEntry]) -> Result<Chunk, Error> {
    let mut parts: Vec<(String, String)> = Vec::new();
    let mut budget = MAX_CONTENT_BYTES;
    let mut truncated = false;
    let mut next_offset = 0u64;
    for (i, file) in files.iter().enumerate().rev() {
        if budget == 0 {
            truncated = true;
            break;
        }
        let path = safe_log_path(log_dir, &file.name, files)?;
        let chunk = read_capped(&path, 0, budget)
            .map_err(|e| Error::Internal(format!("read log {}: {e}", file.name)))?;
        if i + 1 == files.len() {
            next_offset = chunk.next_offset;
        }
        budget = budget.saturating_sub(chunk.content.len() as u64);
        truncated |= chunk.truncated;
        parts.push((file.name.clone(), chunk.content));
    }
    let mut out = String::new();
    for (name, content) in parts.iter().rev() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("===== {name} =====\n"));
        out.push_str(content);
    }
    Ok(Chunk {
        content: out,
        offset: 0,
        next_offset,
        truncated,
    })
}

/// The last `lines` lines, reading at most `MAX_CONTENT_BYTES` from the end.
fn tail_lines(path: &Path, lines: usize) -> std::io::Result<Chunk> {
    let mut chunk = read_capped(path, 0, MAX_CONTENT_BYTES)?;
    let parts: Vec<&str> = chunk.content.split_inclusive('\n').collect();
    let start = parts.len().saturating_sub(lines);
    if start > 0 {
        let dropped: usize = parts[..start].iter().map(|p| p.len()).sum();
        chunk.offset += dropped as u64;
        chunk.content = parts[start..].concat();
    }
    Ok(chunk)
}

/// Bytes from `offset` to EOF (capped; see [`read_capped`]).
fn read_since(path: &Path, offset: u64) -> std::io::Result<Chunk> {
    read_capped(path, offset, MAX_CONTENT_BYTES)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_log_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    #[test]
    fn reads_full_log_without_line_cap() {
        let dir = temp_log_dir();
        let body = (0..800)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.path().join("ottod.log.2026-06-13"), &body).unwrap();

        let logs = read_daemon_logs(
            dir.path(),
            LogsParams {
                file: Some("ottod.log.2026-06-13".into()),
                mode: Some("all".into()),
                lines: None,
                offset: None,
            },
        )
        .unwrap();

        assert!(logs.content.contains("line 0"));
        assert!(logs.content.contains("line 799"));
    }

    #[test]
    fn tails_requested_number_of_lines() {
        let dir = temp_log_dir();
        std::fs::write(dir.path().join("ottod.log.2026-06-13"), "a\nb\nc\nd\n").unwrap();

        let logs = read_daemon_logs(
            dir.path(),
            LogsParams {
                file: Some("ottod.log.2026-06-13".into()),
                mode: Some("tail".into()),
                lines: Some(2),
                offset: None,
            },
        )
        .unwrap();

        assert_eq!(logs.content, "c\nd\n");
    }

    #[test]
    fn rejects_path_traversal() {
        let dir = temp_log_dir();
        std::fs::write(dir.path().join("ottod.log.2026-06-13"), "ok").unwrap();

        let err = read_daemon_logs(
            dir.path(),
            LogsParams {
                file: Some("../secret".into()),
                mode: Some("all".into()),
                lines: None,
                offset: None,
            },
        )
        .unwrap_err();

        assert!(matches!(err, Error::Invalid(_)));
    }

    #[test]
    fn reads_updates_from_offset() {
        let dir = temp_log_dir();
        std::fs::write(dir.path().join("ottod.log.2026-06-13"), "first\nsecond\n").unwrap();

        let logs = read_daemon_logs(
            dir.path(),
            LogsParams {
                file: Some("ottod.log.2026-06-13".into()),
                mode: Some("since".into()),
                lines: None,
                offset: Some(6),
            },
        )
        .unwrap();

        assert_eq!(logs.content, "second\n");
        assert_eq!(logs.offset, 6);
        assert_eq!(logs.next_offset, 13);
        assert!(!logs.truncated);
    }

    #[test]
    fn capped_read_keeps_newest_whole_lines() {
        let dir = temp_log_dir();
        let path = dir.path().join("ottod.log.2026-06-13");
        std::fs::write(&path, "aaaa\nbbbb\ncccc\n").unwrap();
        let c = read_capped(&path, 0, 7).unwrap();
        // Last 7 bytes are "b\ncccc\n" → cut forward to the line start.
        assert!(c.truncated);
        assert_eq!(c.content, "cccc\n");
        assert_eq!(c.offset, 10);
        assert_eq!(c.next_offset, 15);
        let full = read_capped(&path, 0, 100).unwrap();
        assert!(!full.truncated);
        assert_eq!(full.content.len(), 15);
    }

    #[test]
    fn all_files_next_offset_tracks_newest_file() {
        let dir = temp_log_dir();
        std::fs::write(dir.path().join("ottod.log.2026-06-12"), "old\n").unwrap();
        std::fs::write(dir.path().join("ottod.log.2026-06-13"), "new1\nnew2\n").unwrap();
        let logs = read_daemon_logs(
            dir.path(),
            LogsParams {
                file: Some(ALL_FILES.into()),
                mode: Some("all".into()),
                lines: None,
                offset: None,
            },
        )
        .unwrap();
        assert_eq!(
            logs.content,
            "===== ottod.log.2026-06-12 =====\nold\n\n===== ottod.log.2026-06-13 =====\nnew1\nnew2\n"
        );
        assert_eq!(logs.next_offset, 10, "offset within the newest file");
        assert!(!logs.truncated);
    }

    #[test]
    fn tail_reports_offset_of_first_kept_line() {
        let dir = temp_log_dir();
        std::fs::write(dir.path().join("ottod.log.2026-06-13"), "a\nb\nc\nd\n").unwrap();
        let logs = read_daemon_logs(
            dir.path(),
            LogsParams {
                file: Some("ottod.log.2026-06-13".into()),
                mode: Some("tail".into()),
                lines: Some(2),
                offset: None,
            },
        )
        .unwrap();
        assert_eq!(logs.content, "c\nd\n");
        assert_eq!((logs.offset, logs.next_offset), (4, 8));
    }
}

/// Body of `POST /client/errors`.
#[derive(Debug, Deserialize)]
pub struct ClientErrorReq {
    /// Short classifier the UI assigns (`effect_loop`, `unhandled_rejection`, …).
    pub kind: String,
    pub message: String,
    #[serde(default)]
    pub stack: String,
    /// The hash route that was on screen (`#/agents/<id>`), for correlation.
    #[serde(default)]
    pub route: String,
    /// What the UI did about it (`reloaded`, `reload_suppressed`, `none`).
    #[serde(default)]
    pub action: String,
}

fn clip(s: &str, max: usize) -> String {
    // Char-boundary safe: a stack from a minified bundle can be one huge line.
    let mut out: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        out.push('…');
    }
    out
}

/// `POST /api/v1/client/errors` — 204. The UI's last-resort error hook (see
/// `ui/src/main.ts`) posts here before it self-heals, so a fatal client-side
/// failure — e.g. Svelte's `effect_update_depth_exceeded`, which aborts the
/// reactive flush and leaves the whole shell frozen until a reload — is
/// recorded in the daemon log with a timestamp, the user, the route and the
/// (minified) stack, next to whatever the daemon was doing. Without this the
/// only evidence was a devtools console the user usually did not have open.
/// Any authenticated user may report; fields are clipped, never interpreted.
pub async fn client_error(
    CurrentUser(user): CurrentUser,
    Json(req): Json<ClientErrorReq>,
) -> ApiResult<StatusCode> {
    tracing::error!(
        target: "otto_client",
        user = %user.username,
        kind = %clip(&req.kind, 64),
        route = %clip(&req.route, 256),
        action = %clip(&req.action, 32),
        stack = %clip(&req.stack, 4000),
        "UI fatal error: {}",
        clip(&req.message, 2000)
    );
    Ok(StatusCode::NO_CONTENT)
}
