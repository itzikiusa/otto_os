//! Audit ledger for first-party Otto MCP tool calls (Task B2b).
//!
//! Every invocation of an `otto_*` tool exposed to an agent session through
//! `.mcp.json` appends one row via [`McpAuditRepo::record`]. Append-only — there
//! is no update or delete path. The `ottod mcp-tools` subprocess writes these
//! best-effort: a failed audit insert must never fail the tool call, so callers
//! log and swallow the error. `args_json` is redacted (`otto_core::redact`)
//! before it reaches here, so no raw secret is persisted. It is also SHAPED
//! here ([`cap_args_json`]) so the ledger stays bounded WITHOUT hiding what
//! was called (r3-10-04 — a 200-char prefix let a destructive statement
//! padded past 1 KB vanish from the audit):
//! - any string value over [`ARG_STRING_CAP`] chars keeps its first AND last
//!   [`ARG_STRING_EDGE`] chars around a `…[truncated: N chars, sha256 H]…`
//!   marker (N = the original length, H = the sha256 of the full value), so
//!   a padded tail stays visible and the full value stays verifiable;
//! - the whole document is capped at [`ARGS_JSON_MAX_BYTES`]: past it the
//!   row stores `{"_otto_truncated":true,"bytes":N,"sha256":H,"head":…,
//!   "tail":…}` (head/tail of the shaped JSON text).
//!
//! The ledger records *that* a tool was called and with what shape — not full
//! note/file bodies (`otto_vault_write*` alone stored 120 MB).

use chrono::Utc;
use otto_core::{new_id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt};

/// String values longer than this (in chars) are truncated before insert.
pub const ARG_STRING_CAP: usize = 1024;
/// Chars kept from EACH end (head and tail) of a truncated string value.
pub const ARG_STRING_EDGE: usize = 512;
/// Hard cap on the stored `args_json` (bytes) after per-string shaping —
/// 10k short strings would otherwise still store megabytes.
pub const ARGS_JSON_MAX_BYTES: usize = 16 * 1024;
/// Bytes of the shaped document kept at the head / tail (on char
/// boundaries) when the whole document is over [`ARGS_JSON_MAX_BYTES`]. The
/// stored marker is ≈ these + JSON escaping of the kept text.
const DOC_HEAD_BYTES: usize = 8 * 1024;
const DOC_TAIL_BYTES: usize = 4 * 1024;

fn sha256_hex(s: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// Shape `args_json` for storage (see the module doc). The result is always
/// valid JSON: input that doesn't parse and is itself over the cap is stored
/// as a truncated JSON string. Also used for `mcp_call_log.args_redacted_json`.
pub fn cap_args_json(args_json: &str) -> String {
    fn shape(v: &mut serde_json::Value) -> bool {
        match v {
            serde_json::Value::String(s) => match truncate_str(s) {
                Some(t) => {
                    *s = t;
                    true
                }
                None => false,
            },
            serde_json::Value::Array(a) => a.iter_mut().fold(false, |acc, x| shape(x) | acc),
            serde_json::Value::Object(o) => o.values_mut().fold(false, |acc, x| shape(x) | acc),
            _ => false,
        }
    }
    // Fast path: nothing can exceed the cap in a short document.
    if args_json.len() <= ARG_STRING_CAP {
        return args_json.to_string();
    }
    let shaped = match serde_json::from_str::<serde_json::Value>(args_json) {
        Ok(mut v) => {
            if shape(&mut v) {
                serde_json::to_string(&v).unwrap_or_else(|_| args_json.to_string())
            } else {
                args_json.to_string()
            }
        }
        Err(_) => {
            let t = truncate_str(args_json).unwrap_or_else(|| args_json.to_string());
            serde_json::Value::String(t).to_string()
        }
    };
    cap_doc(shaped, args_json)
}

/// Whole-document bound: over [`ARGS_JSON_MAX_BYTES`] → a marker object with
/// the byte length and sha256 of the ORIGINAL input plus head/tail of the
/// shaped text (so both ends of a huge argument list stay readable).
fn cap_doc(shaped: String, original: &str) -> String {
    if shaped.len() <= ARGS_JSON_MAX_BYTES {
        return shaped;
    }
    let mut h = DOC_HEAD_BYTES.min(shaped.len());
    while !shaped.is_char_boundary(h) {
        h -= 1;
    }
    let mut t = shaped.len().saturating_sub(DOC_TAIL_BYTES);
    while !shaped.is_char_boundary(t) {
        t += 1;
    }
    let (head, tail) = (&shaped[..h], &shaped[t..]);
    serde_json::json!({
        "_otto_truncated": true,
        "bytes": original.len(),
        "sha256": sha256_hex(original),
        "head": head,
        "tail": tail,
    })
    .to_string()
}

/// `Some(head + marker + tail)` when `s` is over the cap, else `None`.
fn truncate_str(s: &str) -> Option<String> {
    let total = s.chars().count();
    if total <= ARG_STRING_CAP {
        return None;
    }
    let head: String = s.chars().take(ARG_STRING_EDGE).collect();
    let tail: String = s.chars().skip(total - ARG_STRING_EDGE).collect();
    Some(format!(
        "{head}…[truncated: {total} chars, sha256 {}]…{tail}",
        sha256_hex(s)
    ))
}

/// Input for [`McpAuditRepo::record`]. `id`/`created_at` are owned by the repo.
#[derive(Debug, Clone)]
pub struct NewMcpToolCall {
    /// The calling session's workspace, when known.
    pub workspace_id: Option<String>,
    /// The agent session that invoked the tool, when known.
    pub session_id: Option<String>,
    /// Tool name, e.g. `"otto_db_schema"`.
    pub tool: String,
    /// Redacted JSON of the call arguments.
    pub args_json: String,
    /// Whether the call succeeded.
    pub ok: bool,
    /// Row/item count returned (tool-specific; `None` when not meaningful).
    pub rows: Option<i64>,
}

/// One persisted audit row (read side, for a future governance view).
#[derive(Debug, Clone)]
pub struct McpToolCallRow {
    pub id: String,
    pub workspace_id: Option<String>,
    pub session_id: Option<String>,
    pub tool: String,
    pub args_json: String,
    pub ok: bool,
    pub rows: Option<i64>,
    pub created_at: String,
}

fn row_to_call(r: &sqlx::sqlite::SqliteRow) -> McpToolCallRow {
    McpToolCallRow {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        session_id: r.get("session_id"),
        tool: r.get("tool"),
        args_json: r.get("args_json"),
        ok: r.get::<i64, _>("ok") != 0,
        rows: r.get("rows"),
        created_at: r.get("created_at"),
    }
}

#[derive(Clone)]
pub struct McpAuditRepo {
    pool: SqlitePool,
}

impl McpAuditRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Append one tool-call audit row.
    pub async fn record(&self, e: NewMcpToolCall) -> Result<()> {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO mcp_tool_calls
               (id, workspace_id, session_id, tool, args_json, ok, rows, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&e.workspace_id)
        .bind(&e.session_id)
        .bind(&e.tool)
        .bind(cap_args_json(&e.args_json))
        .bind(e.ok as i64)
        .bind(e.rows)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("mcp tool audit insert"))?;
        Ok(())
    }

    /// Recent tool calls for a session (newest first, capped at `limit`).
    pub async fn recent_for_session(
        &self,
        session_id: &str,
        limit: i64,
    ) -> Result<Vec<McpToolCallRow>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, session_id, tool, args_json, ok, rows, created_at
             FROM mcp_tool_calls
             WHERE session_id = ?
             ORDER BY created_at DESC
             LIMIT ?",
        )
        .bind(session_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("mcp tool audit query"))?;
        Ok(rows.iter().map(row_to_call).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

    async fn mem_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[test]
    fn cap_args_json_truncates_long_strings_only() {
        let body = "x".repeat(5_000);
        let arg = serde_json::json!({"path": "a.md", "content": body, "n": 3,
            "nested": [{"k": "é".repeat(1_500)}]})
        .to_string();
        let out: serde_json::Value = serde_json::from_str(&cap_args_json(&arg)).unwrap();
        assert_eq!(out["path"], "a.md");
        assert_eq!(out["n"], 3);
        let c = out["content"].as_str().unwrap();
        assert!(c.starts_with(&"x".repeat(ARG_STRING_EDGE)));
        assert!(c.ends_with(&"x".repeat(ARG_STRING_EDGE)));
        assert!(
            c.contains(&format!("…[truncated: 5000 chars, sha256 {}]…", sha256_hex(&body))),
            "{c}"
        );
        let k = out["nested"][0]["k"].as_str().unwrap();
        assert!(
            k.contains("…[truncated: 1500 chars,"),
            "multi-byte counted in chars"
        );
        // Short / at-cap values are untouched byte-for-byte.
        let small = r#"{"a":"b"}"#;
        assert_eq!(cap_args_json(small), small);
        let at_cap = serde_json::json!({"a": "y".repeat(ARG_STRING_CAP)}).to_string();
        assert_eq!(cap_args_json(&at_cap), at_cap);
        // Non-JSON over the cap is still stored as valid JSON.
        let raw = "z".repeat(3_000);
        let v: serde_json::Value = serde_json::from_str(&cap_args_json(&raw)).unwrap();
        assert!(v.as_str().unwrap().contains("…[truncated: 3000 chars,"));
    }

    /// r3-10-04: a destructive statement padded past the cap must stay
    /// visible in the ledger (the old 200-char prefix dropped it).
    #[test]
    fn padded_payload_keeps_its_tail_visible() {
        let sql = format!("{}\nDROP TABLE users;", "-- padding\n".repeat(200));
        let arg = serde_json::json!({"connection_id": "c1", "sql": sql}).to_string();
        let out: serde_json::Value = serde_json::from_str(&cap_args_json(&arg)).unwrap();
        let stored = out["sql"].as_str().unwrap();
        assert!(stored.len() < sql.len());
        assert!(stored.ends_with("DROP TABLE users;"), "{stored}");
        assert!(stored.contains(&sha256_hex(&sql)));
    }

    #[test]
    fn whole_document_is_byte_capped_with_both_ends() {
        // 10k short strings: no single value is over the cap, but the row is.
        let many: Vec<String> = (0..10_000).map(|i| format!("item-{i:05}-{}", "p".repeat(900))).collect();
        let arg = serde_json::json!({"items": many, "last": "DROP"}).to_string();
        let stored = cap_args_json(&arg);
        assert!(stored.len() <= ARGS_JSON_MAX_BYTES + 1024, "{}", stored.len());
        let v: serde_json::Value = serde_json::from_str(&stored).unwrap();
        assert_eq!(v["_otto_truncated"], true);
        assert_eq!(v["bytes"], arg.len());
        assert_eq!(v["sha256"], sha256_hex(&arg));
        assert!(v["head"].as_str().unwrap().starts_with("{\"items\":[\"item-00000"));
        assert!(v["tail"].as_str().unwrap().contains("\"last\":\"DROP\""));
    }

    #[tokio::test]
    async fn record_and_read_back() {
        let pool = mem_pool().await;
        let repo = McpAuditRepo::new(pool.clone());
        repo.record(NewMcpToolCall {
            workspace_id: Some("ws1".into()),
            session_id: Some("sess1".into()),
            tool: "otto_db_schema".into(),
            args_json: r#"{"connection_id":"c1"}"#.into(),
            ok: true,
            rows: Some(7),
        })
        .await
        .unwrap();
        // A second call that failed (e.g. denied / not found).
        repo.record(NewMcpToolCall {
            workspace_id: Some("ws1".into()),
            session_id: Some("sess1".into()),
            tool: "otto_git_pr_review".into(),
            args_json: r#"{"repo_id":"r1","pr_number":9}"#.into(),
            ok: false,
            rows: None,
        })
        .await
        .unwrap();

        let recent = repo.recent_for_session("sess1", 10).await.unwrap();
        assert_eq!(recent.len(), 2);
        // Newest first: the failed git call is most recent.
        assert_eq!(recent[0].tool, "otto_git_pr_review");
        assert!(!recent[0].ok);
        assert_eq!(recent[0].rows, None);
        assert_eq!(recent[1].tool, "otto_db_schema");
        assert!(recent[1].ok);
        assert_eq!(recent[1].rows, Some(7));
    }
}
