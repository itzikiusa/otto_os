//! Thin async wrapper over the `clickhouse` binary in `local --path` mode.
//!
//! Every call shells out to a fresh `clickhouse local` process pointed at our
//! persistent data dir. `clickhouse local` takes an exclusive lock on its
//! `--path`, so a second concurrent process fails with an I/O error — we
//! therefore serialize *all* access through `lock` (held for the duration of
//! each spawned process). Process startup is ~tens of ms; usage writes are
//! batched by the engine and metric writes are infrequent, so this is cheap
//! enough while keeping the implementation dependency-free (no embedded C++).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use otto_core::{Error, Result};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex;

/// Handle to a local ClickHouse instance backed by an on-disk `--path`.
pub struct ClickHouse {
    bin: PathBuf,
    data_dir: PathBuf,
    /// Serializes process spawns — `clickhouse local` locks its `--path`.
    lock: Mutex<()>,
}

impl ClickHouse {
    /// Resolve the `clickhouse` binary in priority order: an explicit
    /// configured path, then `PATH`, then well-known install locations
    /// (Homebrew, `/usr/local/bin`, `~/clickhouse`, `~/.local/bin`, and the
    /// copy Otto's installer drops in its own `bin/`). Returns an absolute path
    /// so the daemon can run it regardless of the working directory or whether
    /// it's actually on `PATH`.
    pub fn locate(configured: Option<&str>) -> Option<PathBuf> {
        if let Some(p) = configured.map(str::trim).filter(|s| !s.is_empty()) {
            let pb = PathBuf::from(p);
            if pb.is_file() {
                return Some(pb);
            }
        }
        if let Ok(out) = std::process::Command::new("which").arg("clickhouse").output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !s.is_empty() && Path::new(&s).is_file() {
                    return Some(PathBuf::from(s));
                }
            }
        }
        let mut candidates = vec![
            PathBuf::from("/usr/local/bin/clickhouse"),
            PathBuf::from("/opt/homebrew/bin/clickhouse"),
        ];
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("clickhouse"));
            candidates.push(home.join(".local/bin/clickhouse"));
            candidates.push(home.join("Library/Application Support/Otto/bin/clickhouse"));
        }
        candidates.into_iter().find(|p| p.is_file())
    }

    /// Wrap an already-resolved binary + data dir. Does not touch disk.
    pub fn new(bin: PathBuf, data_dir: PathBuf) -> Self {
        Self {
            bin,
            data_dir,
            lock: Mutex::new(()),
        }
    }

    pub fn binary(&self) -> &Path {
        &self.bin
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    fn base_args(&self) -> Vec<String> {
        vec![
            "local".into(),
            "--path".into(),
            self.data_dir.to_string_lossy().into_owned(),
        ]
    }

    /// `clickhouse local --version` → the first output line, e.g.
    /// "ClickHouse local version 26.3.1.715 (official build).".
    pub async fn version(&self) -> Result<String> {
        let out = Command::new(&self.bin)
            .arg("local")
            .arg("--version")
            .output()
            .await
            .map_err(|e| Error::Internal(format!("clickhouse --version: {e}")))?;
        Ok(String::from_utf8_lossy(&out.stdout)
            .lines()
            .next()
            .unwrap_or("")
            .trim()
            .to_string())
    }

    /// Run one or more `;`-separated statements that produce no rows (DDL,
    /// `ALTER`, …). Creates the data dir on first use.
    pub async fn exec(&self, sql: &str) -> Result<()> {
        let _g = self.lock.lock().await;
        std::fs::create_dir_all(&self.data_dir)
            .map_err(|e| Error::Internal(format!("create clickhouse dir: {e}")))?;
        let mut args = self.base_args();
        args.push("--multiquery".into());
        args.push("--query".into());
        args.push(sql.to_string());
        let out = Command::new(&self.bin)
            .args(&args)
            .output()
            .await
            .map_err(|e| Error::Internal(format!("clickhouse exec spawn: {e}")))?;
        if !out.status.success() {
            return Err(Error::Internal(format!(
                "clickhouse exec failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }

    /// Run a `SELECT` and return each row as a JSON object. 64-bit integers are
    /// emitted as numbers (not strings) via
    /// `output_format_json_quote_64bit_integers=0` so they deserialize straight
    /// into `u64`/`f64`.
    pub async fn query_rows(&self, sql: &str) -> Result<Vec<serde_json::Value>> {
        let _g = self.lock.lock().await;
        let mut args = self.base_args();
        args.push("--output_format_json_quote_64bit_integers=0".into());
        // Resolve bare identifiers to columns, not same-named SELECT aliases, so
        // `sum(input_tokens + …) AS total_tokens` alongside `sum(input_tokens) AS
        // input_tokens` doesn't trip ILLEGAL_AGGREGATION (alias = column name).
        args.push("--prefer_column_name_to_alias=1".into());
        args.push("--multiquery".into());
        args.push("--query".into());
        args.push(sql.to_string());
        args.push("--format".into());
        args.push("JSONEachRow".into());
        let out = Command::new(&self.bin)
            .args(&args)
            .output()
            .await
            .map_err(|e| Error::Internal(format!("clickhouse query spawn: {e}")))?;
        if !out.status.success() {
            return Err(Error::Internal(format!(
                "clickhouse query failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut rows = Vec::new();
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let v: serde_json::Value = serde_json::from_str(line)
                .map_err(|e| Error::Internal(format!("parse clickhouse row: {e}")))?;
            rows.push(v);
        }
        Ok(rows)
    }

    /// Bulk insert into `table` from newline-delimited JSON (one object per
    /// line). Columns omitted from each object fall back to their DDL default
    /// (e.g. `ts`/`event_date`). A blank payload is a no-op.
    pub async fn insert_ndjson(&self, table: &str, ndjson: &str) -> Result<()> {
        if ndjson.trim().is_empty() {
            return Ok(());
        }
        let _g = self.lock.lock().await;
        std::fs::create_dir_all(&self.data_dir)
            .map_err(|e| Error::Internal(format!("create clickhouse dir: {e}")))?;
        let mut args = self.base_args();
        args.push("--query".into());
        args.push(format!("INSERT INTO {table} FORMAT JSONEachRow"));
        let mut child = Command::new(&self.bin)
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| Error::Internal(format!("clickhouse insert spawn: {e}")))?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(ndjson.as_bytes())
                .await
                .map_err(|e| Error::Internal(format!("clickhouse insert stdin: {e}")))?;
            stdin
                .shutdown()
                .await
                .map_err(|e| Error::Internal(format!("clickhouse insert close: {e}")))?;
        }
        let out = child
            .wait_with_output()
            .await
            .map_err(|e| Error::Internal(format!("clickhouse insert wait: {e}")))?;
        if !out.status.success() {
            return Err(Error::Internal(format!(
                "clickhouse insert failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(())
    }
}
