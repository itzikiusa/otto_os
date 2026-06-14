//! `GET /api/v1/logs/daemon` — safe read access to Otto daemon logs.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::{require_root, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;
use otto_core::Error;

const ALL_FILES: &str = "__all__";
const DEFAULT_TAIL_LINES: usize = 500;
const MAX_TAIL_LINES: usize = 50_000;

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
    pub offset: u64,
    pub next_offset: u64,
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
    read_daemon_logs(&log_dir, params)
        .map(Json)
        .map_err(ApiError)
}

fn daemon_log_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("OTTO_LOG_DIR") {
        return PathBuf::from(dir);
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

    let (content, offset, next_offset) = if selected == ALL_FILES {
        read_all_files(log_dir, &files)?
    } else {
        let path = safe_log_path(log_dir, &selected, &files)?;
        match mode {
            ReadMode::All => {
                let content = std::fs::read_to_string(&path)
                    .map_err(|e| Error::Internal(format!("read log {}: {e}", selected)))?;
                let next_offset = std::fs::metadata(&path)
                    .map(|m| m.len())
                    .unwrap_or(content.len() as u64);
                (content, 0, next_offset)
            }
            ReadMode::Tail => {
                let lines = params
                    .lines
                    .unwrap_or(DEFAULT_TAIL_LINES)
                    .clamp(1, MAX_TAIL_LINES);
                let content = tail_lines(&path, lines)
                    .map_err(|e| Error::Internal(format!("tail log {}: {e}", selected)))?;
                let next_offset = std::fs::metadata(&path)
                    .map(|m| m.len())
                    .unwrap_or(content.len() as u64);
                (content, 0, next_offset)
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
        content,
        offset,
        next_offset,
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

fn read_all_files(log_dir: &Path, files: &[LogFileEntry]) -> Result<(String, u64, u64), Error> {
    let mut out = String::new();
    for file in files {
        let path = safe_log_path(log_dir, &file.name, files)?;
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("===== {} =====\n", file.name));
        let content = std::fs::read_to_string(&path)
            .map_err(|e| Error::Internal(format!("read log {}: {e}", file.name)))?;
        out.push_str(&content);
    }
    let next_offset = out.len() as u64;
    Ok((out, 0, next_offset))
}

fn tail_lines(path: &Path, lines: usize) -> std::io::Result<String> {
    let content = std::fs::read_to_string(path)?;
    let parts: Vec<&str> = content.split_inclusive('\n').collect();
    let start = parts.len().saturating_sub(lines);
    Ok(parts[start..].concat())
}

fn read_since(path: &Path, offset: u64) -> std::io::Result<(String, u64, u64)> {
    let mut file = std::fs::File::open(path)?;
    let size = file.metadata()?.len();
    let start = offset.min(size);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok((String::from_utf8_lossy(&bytes).into_owned(), start, size))
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
    }
}
