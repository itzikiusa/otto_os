//! Background usage tailer — mines *real* token usage from the agent CLIs'
//! on-disk transcript files and records it into the embedded ClickHouse usage
//! store via [`otto_usage::UsageEngine`].
//!
//! Why a tailer (rather than instrumenting the PTY): the CLIs already write
//! exact, per-turn token counts (input/output/cache) and the model id to JSONL
//! transcripts. Tailing those files is the single source of truth and survives
//! resumes, channel sessions, and restarts.
//!
//! Supported providers:
//!   * **Claude Code** — `~/.claude/projects/<enc_cwd>/<uuid>.jsonl`. Attributed
//!     by transcript filename stem (= `provider_session_id` in `sessions`).
//!   * **Codex** — `~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`.
//!     Attributed by `cwd` (from the file's `session_meta` line) → the unique
//!     codex session with that cwd, if exactly one.
//!   * **agy** — unsupported (token usage is encrypted on disk); logged once.
//!
//! Correctness invariants:
//!   * **No double-counting.** A per-file byte-offset cursor is persisted to
//!     `<data_dir>/usage_tailer.json` (atomic write). ClickHouse has no
//!     idempotency column, so the cursor is the *only* guard — including across
//!     restarts. Only complete lines (up to the last `\n`) are consumed; a
//!     trailing partial line is left for the next scan.
//!   * **No misdated backfill.** ClickHouse stamps `ts = now()` on insert, so
//!     replaying historical turns would misdate them. At startup every existing
//!     transcript is seeded with `cursor = file size`, skipping pre-existing
//!     history. Files that appear *later* (new real-time sessions) start at 0
//!     and are captured in full.
//!   * **Crash-resilient.** A bad file/line logs and is skipped; the loop never
//!     panics.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use otto_state::{SessionsRepo, SqlitePool};
use otto_usage::{
    estimate_cost, parse_claude_line, parse_codex_line, parse_codex_session_meta, CursorStore,
    UsageEngine, UsageEvent, EXTERNAL_WORKSPACE,
};
use tokio::task::JoinHandle;

/// How often to scan for new transcript bytes.
const SCAN_INTERVAL: Duration = Duration::from_secs(20);

/// Default model label for Codex turns when the rollout file carries no model.
/// `estimate_cost` prices this at the gpt tier (substring match on "codex").
const CODEX_FALLBACK_MODEL: &str = "codex";

// ---------------------------------------------------------------------------
// Public handle
// ---------------------------------------------------------------------------

/// Handle returned by [`UsageTailer::start`]. Keep it alive for the process
/// lifetime; dropping it sets the cancel flag and stops the loop.
pub struct UsageTailerHandle {
    cancel: Arc<AtomicBool>,
    _task: JoinHandle<()>,
}

impl Drop for UsageTailerHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Tailer
// ---------------------------------------------------------------------------

pub struct UsageTailer {
    usage: Arc<UsageEngine>,
    pool: SqlitePool,
    /// Home directory — the root of `~/.claude` and `~/.codex`.
    home: PathBuf,
    cursors: CursorStore,
}

/// One Otto session, projected for attribution.
#[derive(Clone)]
struct SessionRef {
    otto_session_id: String,
    workspace_id: String,
    provider: String,
}

/// Attribution indexes, rebuilt fresh each scan from the `sessions` table.
#[derive(Default)]
struct Attribution {
    /// claude: `provider_session_id` (= transcript filename stem) → session.
    by_provider_session: HashMap<String, SessionRef>,
    /// codex: `cwd` → all sessions in that directory (used only when unique).
    by_cwd: HashMap<String, Vec<SessionRef>>,
}

impl UsageTailer {
    /// Build the tailer. `data_dir` holds the persisted cursor file; `home` is
    /// the root for the `~/.claude` and `~/.codex` transcript trees.
    pub fn new(usage: Arc<UsageEngine>, pool: SqlitePool, data_dir: PathBuf, home: PathBuf) -> Self {
        let cursor_path = data_dir.join("usage_tailer.json");
        let cursors = CursorStore::load(cursor_path);
        Self {
            usage,
            pool,
            home,
            cursors,
        }
    }

    /// Spawn the background loop. Returns immediately.
    pub fn start(mut self) -> UsageTailerHandle {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_task = Arc::clone(&cancel);
        let task = tokio::spawn(async move {
            // Skip pre-existing history so old turns aren't replayed with a
            // now() timestamp (ClickHouse can't backdate them).
            self.seed_existing_files().await;
            loop {
                if cancel_task.load(Ordering::Relaxed) {
                    return;
                }
                if let Err(e) = self.scan_once().await {
                    tracing::warn!("usage tailer: scan failed: {e}");
                }
                // Sleep in short slices so cancellation is responsive.
                let mut slept = Duration::ZERO;
                while slept < SCAN_INTERVAL {
                    if cancel_task.load(Ordering::Relaxed) {
                        return;
                    }
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    slept += Duration::from_millis(500);
                }
            }
        });
        UsageTailerHandle {
            cancel,
            _task: task,
        }
    }

    /// Seed the cursor for every transcript file that isn't already tracked,
    /// setting it to the file's current size so existing history is skipped.
    async fn seed_existing_files(&mut self) {
        let files: Vec<PathBuf> = self
            .claude_files()
            .into_iter()
            .chain(self.codex_files())
            .collect();
        let mut seeded = 0usize;
        for f in files {
            if self.cursors.contains(&f) {
                continue;
            }
            let size = file_size(&f).await.unwrap_or(0);
            self.cursors.set(&f, size);
            seeded += 1;
        }
        if seeded > 0 {
            if let Err(e) = self.cursors.save() {
                tracing::warn!("usage tailer: failed to persist seeded cursors: {e}");
            }
        }
        tracing::info!("usage tailer: seeded {seeded} pre-existing transcript file(s)");
    }

    /// One full scan: rebuild attribution, tail both providers, persist cursors.
    async fn scan_once(&mut self) -> Result<(), String> {
        let attr = self.build_attribution().await;

        for file in self.claude_files() {
            if let Err(e) = self.tail_claude_file(&file, &attr).await {
                tracing::debug!("usage tailer: claude file {} skipped: {e}", file.display());
            }
        }
        for file in self.codex_files() {
            if let Err(e) = self.tail_codex_file(&file, &attr).await {
                tracing::debug!("usage tailer: codex file {} skipped: {e}", file.display());
            }
        }

        if let Err(e) = self.cursors.save() {
            tracing::warn!("usage tailer: failed to persist cursors: {e}");
        }
        Ok(())
    }

    /// Rebuild the claude (by provider-session-id) and codex (by cwd)
    /// attribution indexes from the current `sessions` table.
    async fn build_attribution(&self) -> Attribution {
        let repo = SessionsRepo::new(self.pool.clone());
        let rows = match repo.list_usage_attribution().await {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("usage tailer: attribution query failed: {e}");
                return Attribution::default();
            }
        };
        let mut attr = Attribution::default();
        for r in rows {
            let sref = SessionRef {
                otto_session_id: r.id,
                workspace_id: r.workspace_id,
                provider: r.provider,
            };
            if let Some(psid) = r.provider_session_id {
                if !psid.is_empty() {
                    attr.by_provider_session.insert(psid, sref.clone());
                }
            }
            if !r.cwd.is_empty() {
                attr.by_cwd.entry(r.cwd).or_default().push(sref);
            }
        }
        attr
    }

    // ── Claude ────────────────────────────────────────────────────────────────

    /// All claude transcript files: `~/.claude/projects/*/*.jsonl`.
    fn claude_files(&self) -> Vec<PathBuf> {
        let root = self.home.join(".claude").join("projects");
        let mut out = Vec::new();
        for project in read_subdirs(&root) {
            out.extend(read_files_with_ext(&project, "jsonl"));
        }
        out
    }

    async fn tail_claude_file(&mut self, file: &Path, attr: &Attribution) -> Result<(), String> {
        let (chunk, new_offset) = match self.read_new_bytes(file).await? {
            Some(v) => v,
            None => return Ok(()),
        };

        // Filename stem is the CLI's session uuid (= provider_session_id).
        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let sref = attr.by_provider_session.get(&stem);

        for line in chunk.lines() {
            let Some(parsed) = parse_claude_line(line) else {
                continue;
            };
            let (workspace_id, session_id) = match sref {
                Some(s) => (s.workspace_id.clone(), s.otto_session_id.clone()),
                None => (EXTERNAL_WORKSPACE.to_string(), stem.clone()),
            };
            let cost = estimate_cost(
                &parsed.model,
                parsed.input,
                parsed.output,
                parsed.cache_read,
                parsed.cache_write,
            );
            self.usage.record(UsageEvent {
                workspace_id,
                session_id,
                provider: "claude".to_string(),
                model: parsed.model,
                kind: "completion".to_string(),
                input_tokens: parsed.input,
                output_tokens: parsed.output,
                cache_read_tokens: parsed.cache_read,
                cache_write_tokens: parsed.cache_write,
                cost_usd: cost,
                duration_ms: 0,
            });
        }

        self.cursors.set(file, new_offset);
        Ok(())
    }

    // ── Codex ─────────────────────────────────────────────────────────────────

    /// All codex transcript files:
    /// `~/.codex/sessions/*/*/*/rollout-*.jsonl`.
    fn codex_files(&self) -> Vec<PathBuf> {
        let root = self.home.join(".codex").join("sessions");
        let mut out = Vec::new();
        for y in read_subdirs(&root) {
            for m in read_subdirs(&y) {
                for d in read_subdirs(&m) {
                    for f in read_files_with_ext(&d, "jsonl") {
                        if f
                            .file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with("rollout-"))
                            .unwrap_or(false)
                        {
                            out.push(f);
                        }
                    }
                }
            }
        }
        out
    }

    async fn tail_codex_file(&mut self, file: &Path, attr: &Attribution) -> Result<(), String> {
        // The session_meta (cwd + model) lives on the first line; read it once
        // (cheaply, just the head) so we can attribute and price every turn.
        let meta = read_codex_meta(file).await;
        let cwd = meta.as_ref().and_then(|m| m.cwd.clone());
        let model = meta
            .and_then(|m| m.model)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| CODEX_FALLBACK_MODEL.to_string());

        // Attribute by cwd → only when exactly one codex session matches.
        let sref = cwd.as_deref().and_then(|c| {
            attr.by_cwd.get(c).and_then(|sessions| {
                let codex: Vec<&SessionRef> =
                    sessions.iter().filter(|s| s.provider == "codex").collect();
                if codex.len() == 1 {
                    Some(codex[0].clone())
                } else {
                    None
                }
            })
        });

        // thread-uuid from the filename: rollout-<ts>-<uuid>.jsonl → last uuid.
        let thread_uuid = codex_thread_uuid(file);

        let (chunk, new_offset) = match self.read_new_bytes(file).await? {
            Some(v) => v,
            None => return Ok(()),
        };

        for line in chunk.lines() {
            let Some(parsed) = parse_codex_line(line, &model) else {
                continue;
            };
            let (workspace_id, session_id) = match &sref {
                Some(s) => (s.workspace_id.clone(), s.otto_session_id.clone()),
                None => (EXTERNAL_WORKSPACE.to_string(), thread_uuid.clone()),
            };
            let cost = estimate_cost(
                &parsed.model,
                parsed.input,
                parsed.output,
                parsed.cache_read,
                parsed.cache_write,
            );
            self.usage.record(UsageEvent {
                workspace_id,
                session_id,
                provider: "codex".to_string(),
                model: parsed.model,
                kind: "completion".to_string(),
                input_tokens: parsed.input,
                output_tokens: parsed.output,
                cache_read_tokens: parsed.cache_read,
                cache_write_tokens: parsed.cache_write,
                cost_usd: cost,
                duration_ms: 0,
            });
        }

        self.cursors.set(file, new_offset);
        Ok(())
    }

    // ── Shared I/O ──────────────────────────────────────────────────────────

    /// Read the new bytes of `file` from the persisted cursor to EOF, returning
    /// the complete-lines slice and the byte offset of the last consumed
    /// newline. Returns `Ok(None)` when there's nothing new (or only a partial
    /// trailing line). Handles truncation/rotation by resetting the cursor to 0.
    async fn read_new_bytes(&mut self, file: &Path) -> Result<Option<(String, u64)>, String> {
        let size = match file_size(file).await {
            Some(s) => s,
            None => return Ok(None), // file vanished mid-scan
        };
        let mut cursor = self.cursors.get(file).unwrap_or(0);
        if cursor > size {
            // Truncated / rotated under us — restart from the top.
            cursor = 0;
        }
        if size <= cursor {
            return Ok(None);
        }

        let bytes = read_range(file, cursor, size).await?;
        // Only consume up to the last newline; bytes after it are an incomplete
        // line still being written — leave them for the next scan.
        let last_nl = match bytes.iter().rposition(|&b| b == b'\n') {
            Some(pos) => pos,
            None => return Ok(None), // no complete line yet
        };
        let complete = &bytes[..=last_nl];
        let consumed = cursor + complete.len() as u64;
        let text = String::from_utf8_lossy(complete).into_owned();
        Ok(Some((text, consumed)))
    }
}

// ---------------------------------------------------------------------------
// Free helpers (filesystem, run on blocking threads to keep the loop snappy)
// ---------------------------------------------------------------------------

/// Immediate subdirectories of `dir` (empty on any error).
fn read_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
        .map(|e| e.path())
        .collect()
}

/// Files in `dir` with the given extension (non-recursive; empty on any error).
fn read_files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some(ext))
        .collect()
}

/// File size in bytes, or `None` if it can't be stat'd.
async fn file_size(file: &Path) -> Option<u64> {
    tokio::fs::metadata(file).await.ok().map(|m| m.len())
}

/// Read `file[start..end]` on a blocking thread (files can be large; we only
/// ever read the new slice, never the whole file).
async fn read_range(file: &Path, start: u64, end: u64) -> Result<Vec<u8>, String> {
    use std::io::{Read, Seek, SeekFrom};
    let path = file.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let mut f = std::fs::File::open(&path).map_err(|e| e.to_string())?;
        f.seek(SeekFrom::Start(start)).map_err(|e| e.to_string())?;
        let len = end.saturating_sub(start) as usize;
        let mut buf = vec![0u8; len];
        f.read_exact(&mut buf).map_err(|e| e.to_string())?;
        Ok(buf)
    })
    .await
    .map_err(|e| format!("join: {e}"))?
}

/// Read just the first line of a codex rollout file and parse its session_meta.
/// Reads a bounded head (the meta line is small) to avoid loading large files.
async fn read_codex_meta(file: &Path) -> Option<otto_usage::CodexMeta> {
    use std::io::Read;
    let path = file.to_path_buf();
    let head = tokio::task::spawn_blocking(move || -> Option<String> {
        let mut f = std::fs::File::open(&path).ok()?;
        // Session meta is the first line; 64 KiB is far more than enough.
        let mut buf = vec![0u8; 64 * 1024];
        let n = f.read(&mut buf).ok()?;
        buf.truncate(n);
        Some(String::from_utf8_lossy(&buf).into_owned())
    })
    .await
    .ok()??;
    let first = head.lines().next()?;
    parse_codex_session_meta(first)
}

/// Extract the thread uuid from a codex rollout filename:
/// `rollout-<ISO-ts>-<uuid>.jsonl`. The uuid is the tail after the timestamp;
/// since the ts itself contains `-`, we take the canonical 5-group uuid (last
/// 5 dash-separated segments of the stem). Falls back to the whole stem.
fn codex_thread_uuid(file: &Path) -> String {
    let stem = file
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() >= 5 {
        // Last 5 segments form the uuid (8-4-4-4-12).
        parts[parts.len() - 5..].join("-")
    } else {
        stem
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_thread_uuid_from_rollout_filename() {
        let f = Path::new(
            "/x/rollout-2026-06-18T08-53-25-019ed94a-994a-7010-b01f-9b840c5b7068.jsonl",
        );
        assert_eq!(
            codex_thread_uuid(f),
            "019ed94a-994a-7010-b01f-9b840c5b7068"
        );
    }
}
