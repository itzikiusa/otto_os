//! DB Assistant — a managed, resumable, FILE-BACKED database agent for the DB
//! Explorer. The session-backed replacement for the old "Ask in English" / "Ask
//! AI" drafter, which ran `claude` in a bare `std::env::temp_dir()` with no trust
//! grant (→ hung on the folder-trust dialog → 502) and seeded an EMPTY schema.
//!
//! Two root causes it fixes:
//!   - RC1: the agent now runs as a real Otto **session** (`agent_session::
//!     run_session_turn`) in an Otto-owned, **trusted** directory, so the PTY never
//!     stalls on a first-run trust prompt.
//!   - RC2: it is seeded with the COMPLETE schema (`DbViewerService::
//!     schema_context`, whose node→db derivation was fixed) instead of `(no tables
//!     introspected)`.
//!
//! ## How it works
//! Each assist owns an ephemeral directory `db_assist/<assist_id>` the daemon
//! seeds with:
//!   - `SCHEMA.md`  — the full schema (tables, columns, PK/FK).
//!   - `CONTEXT.md` — connection details, the question, and the working rules.
//!   - `RESULT.md`  — (investigate mode) the statement + a sample of its result.
//!   - `q`          — an executable that POSTs read-only SQL to the loopback
//!     `/db-assist/<id>/query` endpoint (assist-key authed) and prints the rows.
//!     This is the agent↔Otto **interaction loop**: the agent cannot reach any DB
//!     directly, so it runs `./q '<SELECT …>'` and Otto executes it READ-ONLY.
//!
//! The agent writes its FINAL query to `ANSWER.sql` and a one-line note to
//! `NOTE.txt`. While the turn runs we poll `ANSWER.sql` and broadcast each change
//! LIVE (`Event::DbAssistUpdated`), and surface the session at turn start
//! (`Event::DbAssistSessionStarted`) so the Database page can attach the live
//! shell immediately. The session carries `meta.source = "db_assist"`, which hides
//! it from the Agents list.
//!
//! Per-assist state (dir, key, session id, connection, workspace, provider, …)
//! lives in an in-memory registry on [`ServerCtx::db_assist`] — ephemeral by
//! design: a resume reuses the stored session id; a close discards everything.
//!
//! Routes (registered in `modules.rs`; the query tool is a PUBLIC route in
//! `routes/mod.rs` — assist-key authed, not user-bearer, like `/ingest/*`):
//!   POST   /connections/{id}/db/assist                 → one turn  → AssistResp
//!   POST   /connections/{id}/db/assist/{aid}/summary   → SUMMARY.md → { markdown }
//!   DELETE /connections/{id}/db/assist/{aid}           → discard everything
//!   POST   /db-assist/{aid}/query                      → the `q` tool backend

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::event::Event;
use otto_core::{Error, Id};
use otto_dbviewer::types::{statement_is_write, Engine, QueryRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Live `ANSWER.sql` poll cadence while the agent works.
const POLL: Duration = Duration::from_millis(900);
/// Row cap for the read-only `q` tool — enough to ground the agent, bounded so a
/// stray `SELECT *` can't pull a whole table back through the loop.
const Q_MAX_ROWS: usize = 200;

// ---------------------------------------------------------------------------
// In-memory registry
// ---------------------------------------------------------------------------

/// One live assist's state. Cheap to clone (clones out of the registry lock so no
/// lock is held across an await). Ephemeral — never persisted (discarded on close
/// or daemon restart), matching the throwaway working dir.
#[derive(Clone, Debug)]
pub struct DbAssistEntry {
    /// The assist's working directory (`db_assist/<assist_id>`).
    pub dir: PathBuf,
    /// Secret the `q` tool presents in `x-assist-key`. Random per assist.
    pub key: String,
    /// The managed agent session id (set after the first turn; resumed on later
    /// turns / summary).
    pub session_id: Id,
    pub connection_id: Id,
    pub workspace_id: Id,
    /// Agent provider chosen at start (sticky across resumes).
    pub provider: String,
    /// Caller — recorded against the `q` tool's history rows.
    pub user_id: Id,
    /// The active-db node (scopes both the schema seed and the `q` tool's runs).
    pub node: Option<String>,
}

/// The registry type stored on [`ServerCtx`]: `assist_id → entry`.
pub type DbAssistRegistry = Arc<Mutex<HashMap<Id, DbAssistEntry>>>;

/// Construct an empty registry (wired into `ServerCtx` at boot).
pub fn new_registry() -> DbAssistRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

fn lookup(ctx: &ServerCtx, aid: &Id) -> Option<DbAssistEntry> {
    ctx.db_assist.lock().ok().and_then(|m| m.get(aid).cloned())
}

// ---------------------------------------------------------------------------
// Request / response bodies
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct AssistReq {
    /// The user's question / instruction for this turn.
    pub question: String,
    /// `nl` (default) — produce a runnable query; `ask` — free-form Q&A;
    /// `investigate` — examine the current statement + result.
    #[serde(default)]
    pub mode: Option<String>,
    /// The active-db node from the Database tree (scopes the schema + `q` runs).
    #[serde(default)]
    pub node: Option<String>,
    /// Agent provider to run (sticky after the first turn). Defaults to the
    /// workspace/global default.
    #[serde(default)]
    pub provider: Option<String>,
    /// `investigate` mode: the current statement + a sample of its result, dropped
    /// into `RESULT.md` so the agent sees what the user is looking at.
    #[serde(default)]
    pub result_context: Option<String>,
    /// Resume an existing assist (reuses its dir / key / session). Omit to start a
    /// new one.
    #[serde(default)]
    pub assist_id: Option<Id>,
    /// Needed only for global connections (no workspace of their own), mirroring
    /// `db_explain_with_agent`.
    #[serde(default)]
    pub workspace_id: Option<Id>,
}

#[derive(Debug, Serialize)]
pub struct AssistResp {
    pub assist_id: Id,
    pub session_id: Id,
    /// The agent's proposed FINAL query (`ANSWER.sql`, or a ```sql fence fallback).
    pub sql: String,
    /// A one-line explanation (`NOTE.txt`, or the reply's prose fallback).
    pub note: String,
}

#[derive(Debug, Serialize)]
pub struct SummaryResp {
    /// The investigation summary the agent wrote to `SUMMARY.md` (the UI downloads
    /// it).
    pub markdown: String,
}

#[derive(Debug, Deserialize)]
pub struct QueryToolReq {
    pub sql: String,
}

#[derive(Debug, Default, Serialize)]
pub struct QueryToolResp {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    /// Set when the query was rejected (write/DDL) or the engine returned an error;
    /// surfaced to the agent so it can correct its SQL (the loop).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// POST /connections/{id}/db/assist — run one turn
// ---------------------------------------------------------------------------

/// Run ONE assist turn against a managed, file-backed agent session and return its
/// proposed SQL. The first call mints the assist (dir + key + session); later
/// calls (same `assist_id`) RESUME it so the conversation continues.
pub async fn assist(
    Path(conn_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<AssistReq>,
) -> ApiResult<Json<AssistResp>> {
    let conn = ctx
        .db_explorer
        .get_connection(&conn_id)
        .await
        .map_err(ApiError)?;
    let ws_id = conn
        .workspace_id
        .clone()
        .or_else(|| req.workspace_id.clone())
        .ok_or_else(|| ApiError(Error::Invalid("a workspace_id is required".into())))?;
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Editor).await?;
    let ws = ctx.workspaces.get(&ws_id).await.map_err(ApiError)?;
    let engine = Engine::from_kind(conn.kind)
        .ok_or_else(|| ApiError(Error::Invalid(format!("{} is not a browsable database", conn.name))))?;

    // Resume an existing assist (same dir / key / session / provider / node) or
    // mint a fresh one.
    let resumed = req.assist_id.as_ref().and_then(|aid| lookup(&ctx, aid));
    let assist_id = match &resumed {
        Some(_) => req.assist_id.clone().expect("resumed implies assist_id present"),
        None => otto_core::new_id(),
    };
    let dir = resumed
        .as_ref()
        .map(|r| r.dir.clone())
        .unwrap_or_else(|| ctx.data_dir.join("db_assist").join(&assist_id));
    let key = resumed
        .as_ref()
        .map(|r| r.key.clone())
        .unwrap_or_else(|| otto_core::new_id().to_string());
    let provider = match &resumed {
        Some(r) => r.provider.clone(),
        None => resolve_provider(&ctx, &ws, req.provider.as_deref()).await,
    };
    let node = req.node.clone().or_else(|| resumed.as_ref().and_then(|r| r.node.clone()));
    let mode = normalize_mode(req.mode.as_deref());

    // Seed the working dir: SCHEMA.md (RC2 — the full schema), CONTEXT.md, the `q`
    // tool, and (investigate) RESULT.md.
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        return Err(ApiError(Error::Internal(format!("db_assist dir: {e}"))));
    }
    let schema_md = ctx
        .db_explorer
        .schema_context(&conn_id, node.as_deref())
        .await
        .unwrap_or_else(|e| format!("(schema unavailable — use ./q to introspect: {e})"));
    let active_db = node_label(node.as_deref());
    let _ = tokio::fs::write(dir.join("SCHEMA.md"), &schema_md).await;
    let _ = tokio::fs::write(
        dir.join("CONTEXT.md"),
        build_context_md(&conn.name, engine.as_str(), &active_db, &req.question, mode),
    )
    .await;
    if mode == "investigate" {
        if let Some(rc) = req.result_context.as_deref() {
            let _ = tokio::fs::write(dir.join("RESULT.md"), build_result_md(rc)).await;
        }
    }
    let q_path = dir.join("q");
    let _ = tokio::fs::write(&q_path, q_script_body(daemon_port(), &assist_id, &key)).await;
    make_executable(&q_path).await;

    let dir_str = dir.to_string_lossy().to_string();
    // RC1 fix: trust the dir BEFORE the turn so the PTY never stalls on the
    // first-run folder-trust dialog.
    otto_sessions::trust::ensure_trusted(&provider, &dir_str);

    // Live preview: broadcast each ANSWER.sql change while the turn runs.
    let answer_path = dir.join("ANSWER.sql");
    let note_path = dir.join("NOTE.txt");
    let poll = spawn_answer_poll(&ctx, &ws_id, &conn_id, &assist_id, &answer_path, &note_path);

    let base = build_assist_prompt(&conn.name, engine.as_str(), &active_db, &req.question, mode);
    // Inject the per-engine DB skill (no-op until the skills lane seeds the slug).
    let skill = crate::modules::resolve_skill_inline(
        &ctx.context_library,
        &format!("db-{}", engine.as_str()),
    );
    let prompt = crate::modules::compose_draft_prompt(&skill, &base);
    let meta = serde_json::json!({
        "source": "db_assist", "assist_id": assist_id, "connection_id": conn_id,
    });

    // Surface the session the MOMENT it exists (turn start) so the Database page
    // attaches the live shell immediately, not after the turn.
    let ready_events = ctx.events.clone();
    let ready_ws = ws_id.clone();
    let ready_conn = conn_id.clone();
    let ready_aid = assist_id.clone();
    let on_ready = move |sid: &Id| {
        let _ = ready_events.send(Event::DbAssistSessionStarted {
            workspace_id: ready_ws.clone(),
            connection_id: ready_conn.clone(),
            assist_id: ready_aid.clone(),
            session_id: sid.clone(),
        });
    };
    let existing_sid = resumed.as_ref().map(|r| r.session_id.clone());
    let turn = crate::agent_session::run_session_turn(
        &ctx,
        &ws,
        &user,
        existing_sid.as_ref(),
        &format!("DB Assistant: {}", conn.name),
        &dir_str,
        &provider,
        meta,
        &prompt,
        on_ready,
    )
    .await;
    poll.abort();
    let (raw, sid) = turn?;

    // Final answer = the agent's ANSWER.sql, else a ```sql fence in the reply.
    let sql = read_trimmed(&answer_path)
        .await
        .filter(|s| !s.is_empty())
        .or_else(|| extract_fenced(&raw, "sql"))
        .unwrap_or_default();
    let note = read_trimmed(&note_path)
        .await
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| first_line(&raw));

    // Persist the entry so summary/turn can resume and `q` can authenticate.
    if let Ok(mut map) = ctx.db_assist.lock() {
        map.insert(
            assist_id.clone(),
            DbAssistEntry {
                dir: dir.clone(),
                key,
                session_id: sid.clone(),
                connection_id: conn_id.clone(),
                workspace_id: ws_id.clone(),
                provider,
                user_id: user.id.clone(),
                node,
            },
        );
    }

    // One final live broadcast with the committed answer.
    let _ = ctx.events.send(Event::DbAssistUpdated {
        workspace_id: ws_id,
        connection_id: conn_id,
        assist_id: assist_id.clone(),
        sql: sql.clone(),
        note: note.clone(),
    });

    Ok(Json(AssistResp {
        assist_id,
        session_id: sid,
        sql,
        note,
    }))
}

// ---------------------------------------------------------------------------
// POST /connections/{id}/db/assist/{aid}/summary — write + return SUMMARY.md
// ---------------------------------------------------------------------------

/// Resume the assist's session and ask it to write a concise `SUMMARY.md` of the
/// investigation, then return it for the UI to download.
pub async fn summary(
    Path((conn_id, aid)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<SummaryResp>> {
    let entry = lookup(&ctx, &aid)
        .filter(|e| e.connection_id == conn_id)
        .ok_or_else(|| ApiError(Error::NotFound(format!("db assist {aid}"))))?;
    crate::auth::require_ws_role(&ctx, &user, &entry.workspace_id, WorkspaceRole::Editor).await?;
    let ws = ctx.workspaces.get(&entry.workspace_id).await.map_err(ApiError)?;

    let dir_str = entry.dir.to_string_lossy().to_string();
    let summary_path = entry.dir.join("SUMMARY.md");
    // Clear any stale SUMMARY.md so we don't return a previous turn's file if the
    // agent declines to rewrite it.
    let _ = tokio::fs::remove_file(&summary_path).await;
    let prompt = summary_prompt();
    let meta = serde_json::json!({
        "source": "db_assist", "assist_id": aid, "connection_id": conn_id,
    });
    let (raw, _sid) = crate::agent_session::run_session_turn(
        &ctx,
        &ws,
        &user,
        Some(&entry.session_id),
        "DB Assistant: summary",
        &dir_str,
        &entry.provider,
        meta,
        &prompt,
        |_| {},
    )
    .await?;

    let markdown = read_trimmed(&summary_path)
        .await
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| raw.trim().to_string());
    Ok(Json(SummaryResp { markdown }))
}

// ---------------------------------------------------------------------------
// DELETE /connections/{id}/db/assist/{aid} — discard everything
// ---------------------------------------------------------------------------

/// Close the assist: kill the session, remove the working dir, and drop the
/// registry entry. (Close = discard — the assist is ephemeral by design.)
pub async fn close(
    Path((conn_id, aid)): Path<(Id, Id)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let entry = lookup(&ctx, &aid)
        .filter(|e| e.connection_id == conn_id)
        .ok_or_else(|| ApiError(Error::NotFound(format!("db assist {aid}"))))?;
    crate::auth::require_ws_role(&ctx, &user, &entry.workspace_id, WorkspaceRole::Editor).await?;

    let _ = ctx.manager.kill_session(&entry.session_id).await;
    let _ = tokio::fs::remove_dir_all(&entry.dir).await;
    if let Ok(mut map) = ctx.db_assist.lock() {
        map.remove(&aid);
    }
    Ok(Json(serde_json::json!({ "ok": true })))
}

// ---------------------------------------------------------------------------
// POST /db-assist/{aid}/query — the `q` tool backend (PUBLIC, assist-key authed)
// ---------------------------------------------------------------------------

/// Run a READ-ONLY query for the assist's `q` tool. Authenticated by the assist
/// key in `x-assist-key` (NOT a user bearer token — this is a sidecar route, like
/// `/ingest/*`). Writes/DDL are refused; rows are capped. A rejected statement or
/// engine error is returned in `error` (not an HTTP error) so the agent can read
/// it and correct course — that is the agent↔Otto loop.
pub async fn query_tool(
    Path(aid): Path<Id>,
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<QueryToolReq>,
) -> ApiResult<Json<QueryToolResp>> {
    let entry = lookup(&ctx, &aid)
        .ok_or_else(|| ApiError(Error::NotFound(format!("db assist {aid}"))))?;
    let presented = headers
        .get("x-assist-key")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if presented.is_empty() || presented != entry.key {
        return Err(ApiError(Error::Unauthorized));
    }

    let conn = ctx
        .db_explorer
        .get_connection(&entry.connection_id)
        .await
        .map_err(ApiError)?;
    let engine = match Engine::from_kind(conn.kind) {
        Some(e) => e,
        None => {
            return Ok(Json(QueryToolResp {
                error: Some(format!("{} is not a browsable database", conn.name)),
                ..Default::default()
            }))
        }
    };
    // Hard read-only guard — independent of the connection's own write-guard, so
    // even an unguarded dev DB can't be mutated through the assist loop.
    if statement_is_write(engine, &req.sql) {
        return Ok(Json(QueryToolResp {
            error: Some("read-only: writes/DDL are not allowed from the DB Assistant".into()),
            ..Default::default()
        }));
    }

    let qreq = QueryRequest {
        statement: req.sql.clone(),
        max_rows: Some(Q_MAX_ROWS),
        node: entry.node.clone(),
        ..Default::default()
    };
    match ctx
        .db_explorer
        .run(&entry.connection_id, &entry.user_id, &qreq)
        .await
    {
        Ok(res) => Ok(Json(QueryToolResp {
            columns: res.columns.into_iter().map(|c| c.name).collect(),
            rows: res.rows,
            error: None,
        })),
        Err(e) => Ok(Json(QueryToolResp {
            error: Some(e.to_string()),
            ..Default::default()
        })),
    }
}

// ---------------------------------------------------------------------------
// Provider / port helpers
// ---------------------------------------------------------------------------

/// Resolve the agent provider: explicit body value wins, else the workspace
/// default, else the global default, else `claude` (mirrors `db_explain_with_agent`).
async fn resolve_provider(ctx: &ServerCtx, ws: &otto_core::domain::Workspace, body: Option<&str>) -> String {
    if let Some(p) = body.map(str::trim).filter(|p| !p.is_empty()) {
        return p.to_string();
    }
    let global_default = otto_state::SettingsRepo::new(ctx.pool.clone())
        .get("default_provider")
        .await
        .ok()
        .flatten();
    otto_core::provider::resolve_provider(&[
        otto_core::provider::workspace_default(&ws.settings),
        otto_core::provider::global_default(global_default.as_ref()),
    ])
}

/// The loopback port the daemon listens on (`$OTTO_PORT`, default 7700) — baked
/// into the `q` script's base URL.
fn daemon_port() -> u16 {
    std::env::var("OTTO_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(7700)
}

/// `chmod 0755` the `q` script so the agent can run `./q`.
async fn make_executable(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).await;
    }
    #[cfg(not(unix))]
    let _ = path;
}

async fn read_trimmed(path: &std::path::Path) -> Option<String> {
    tokio::fs::read_to_string(path).await.ok().map(|s| s.trim().to_string())
}

// ---------------------------------------------------------------------------
// Live ANSWER.sql poll
// ---------------------------------------------------------------------------

/// Broadcast `DbAssistUpdated` on each `ANSWER.sql` change while the agent works,
/// so the panel shows the proposed query forming live. Aborted when the turn
/// returns.
#[allow(clippy::too_many_arguments)]
fn spawn_answer_poll(
    ctx: &ServerCtx,
    workspace_id: &Id,
    connection_id: &Id,
    assist_id: &Id,
    answer_path: &std::path::Path,
    note_path: &std::path::Path,
) -> tokio::task::JoinHandle<()> {
    let events = ctx.events.clone();
    let workspace_id = workspace_id.clone();
    let connection_id = connection_id.clone();
    let assist_id = assist_id.clone();
    let answer_path = answer_path.to_path_buf();
    let note_path = note_path.to_path_buf();
    let mut last = String::new();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(POLL).await;
            if let Ok(sql) = tokio::fs::read_to_string(&answer_path).await {
                let sql = sql.trim().to_string();
                if sql != last && !sql.is_empty() {
                    last = sql.clone();
                    let note = tokio::fs::read_to_string(&note_path)
                        .await
                        .map(|s| s.trim().to_string())
                        .unwrap_or_default();
                    let _ = events.send(Event::DbAssistUpdated {
                        workspace_id: workspace_id.clone(),
                        connection_id: connection_id.clone(),
                        assist_id: assist_id.clone(),
                        sql,
                        note,
                    });
                }
            }
        }
    })
}

// ---------------------------------------------------------------------------
// Pure builders (unit-tested — no DB / no agent)
// ---------------------------------------------------------------------------

/// Map a requested mode to one of `nl` | `ask` | `investigate` (default `nl`).
fn normalize_mode(mode: Option<&str>) -> &'static str {
    match mode.unwrap_or("nl") {
        "ask" => "ask",
        "investigate" => "investigate",
        _ => "nl",
    }
}

/// Human label for the active-db node (`db:shop/…` → `shop`; bare → itself).
fn node_label(node: Option<&str>) -> String {
    let s = otto_dbviewer::types::NodePath::parse(node.unwrap_or_default())
        .get("db")
        .map(str::to_string)
        .unwrap_or_else(|| node.unwrap_or_default().to_string());
    if s.is_empty() {
        "(default)".to_string()
    } else {
        s
    }
}

/// The mode-specific task line in the prompt + CONTEXT.md.
fn mode_task(mode: &str) -> &'static str {
    match mode {
        "ask" => "Answer the user's free-form question about the data or schema. \
                  If a SQL query best answers it, also write that query to ANSWER.sql.",
        "investigate" => "Examine the statement and result described in RESULT.md. Explain what it \
                          shows, spot anything notable, and write a refined or follow-up query to \
                          ANSWER.sql.",
        // nl
        _ => "Produce ONE runnable SQL query that answers the question. Validate it with ./q \
              before finalizing, then write it to ANSWER.sql.",
    }
}

/// Build the file-edit prompt. The `OTTO_TASK: db_assist` sentinel routes the
/// offline E2E stub; the rest tells the agent how to use the seeded files + tool.
fn build_assist_prompt(conn_name: &str, engine: &str, active_db: &str, question: &str, mode: &str) -> String {
    let result_line = if mode == "investigate" {
        "- RESULT.md — the statement + result sample you are asked to examine.\n"
    } else {
        ""
    };
    format!(
        "OTTO_TASK: db_assist\n\
         You are a database expert helping a user explore the `{conn_name}` ({engine}) connection \
         in Otto. Active database: {active_db}.\n\n\
         Your working directory contains:\n\
         - SCHEMA.md — the COMPLETE schema (tables, columns, primary keys, foreign keys). READ IT \
         FIRST.\n\
         - CONTEXT.md — the connection details and the user's question.\n\
         {result_line}\
         - q — an executable tool that runs READ-ONLY SQL against the LIVE database and prints the \
         rows.\n\n\
         You CANNOT connect to any database yourself. To inspect real data, run:\n\
         \x20\x20./q '<read-only SQL>'\n\
         It executes the SQL read-only (writes/DDL are rejected) and prints the resulting rows. Use \
         it to check real values, counts, and distributions, and to VALIDATE your query before you \
         finalize it.\n\n\
         TASK: {task}\n\n\
         When you have the answer:\n\
         1. Write your FINAL SQL query to a file named `ANSWER.sql` (the raw query only — no ``` \
         fences, no prose).\n\
         2. Write a single-line plain-English explanation to a file named `NOTE.txt`.\n\
         Never run, write, or suggest INSERT/UPDATE/DELETE/DDL — this is strictly read-only.\n\n\
         Question: {question}\n",
        conn_name = conn_name,
        engine = engine,
        active_db = active_db,
        result_line = result_line,
        task = mode_task(mode),
        question = question,
    )
}

/// The `CONTEXT.md` seed file.
fn build_context_md(conn_name: &str, engine: &str, active_db: &str, question: &str, mode: &str) -> String {
    format!(
        "# DB Assistant context\n\n\
         - Connection: {conn_name}\n\
         - Engine: {engine}\n\
         - Active database: {active_db}\n\
         - Mode: {mode}\n\n\
         ## Question\n{question}\n\n\
         ## How to work\n\
         1. Read `SCHEMA.md` for the full schema.\n\
         2. Inspect real data with `./q '<read-only SQL>'` (read-only; it prints rows).\n\
         3. {task}\n\
         4. Write the final query to `ANSWER.sql` and a one-line explanation to `NOTE.txt`.\n\n\
         You cannot connect to any database directly — only the `./q` tool reaches it, read-only.\n",
        conn_name = conn_name,
        engine = engine,
        active_db = active_db,
        mode = mode,
        question = question,
        task = mode_task(mode),
    )
}

/// The `RESULT.md` seed for investigate mode.
fn build_result_md(result_context: &str) -> String {
    format!(
        "# Statement + result under examination\n\n{result_context}\n\n\
         Examine this and write a refined/follow-up query to `ANSWER.sql`.\n",
    )
}

/// The summary-turn prompt.
fn summary_prompt() -> String {
    "OTTO_TASK: db_assist\n\
     Write a concise SUMMARY.md of this investigation to a file named `SUMMARY.md`. Cover: what was \
     asked, the final query (from ANSWER.sql), and the key findings — in clean Markdown. Reply with \
     one short sentence confirming you wrote it."
        .to_string()
}

/// The executable `q` script: POSTs read-only SQL to the loopback query endpoint
/// with the assist key and prints the rows (or the error). Robustly JSON-encodes
/// the SQL via `jq`/`python3` when present, else a conservative shell escape.
fn q_script_body(port: u16, assist_id: &Id, key: &str) -> String {
    format!(
        "#!/usr/bin/env bash\n\
         # Otto DB Assistant — READ-ONLY query tool. Usage: ./q 'SELECT ...'\n\
         set -euo pipefail\n\
         SQL=\"$*\"\n\
         if [ -z \"$SQL\" ]; then echo 'usage: ./q \"<read-only SQL>\"' >&2; exit 2; fi\n\
         BASE=\"http://127.0.0.1:{port}\"\n\
         AID=\"{aid}\"\n\
         KEY=\"{key}\"\n\
         if command -v jq >/dev/null 2>&1; then\n\
         \x20\x20BODY=$(printf '%s' \"$SQL\" | jq -Rs '{{sql: .}}')\n\
         elif command -v python3 >/dev/null 2>&1; then\n\
         \x20\x20BODY=$(SQL=\"$SQL\" python3 -c 'import json,os;print(json.dumps({{\"sql\":os.environ[\"SQL\"]}}))')\n\
         else\n\
         \x20\x20ESC=$(printf '%s' \"$SQL\" | sed 's/\\\\/\\\\\\\\/g; s/\"/\\\\\"/g' | tr '\\n' ' ')\n\
         \x20\x20BODY=\"{{\\\"sql\\\":\\\"$ESC\\\"}}\"\n\
         fi\n\
         RESP=$(curl -s -X POST \"$BASE/api/v1/db-assist/$AID/query\" -H \"x-assist-key: $KEY\" -H 'content-type: application/json' -d \"$BODY\")\n\
         if command -v jq >/dev/null 2>&1; then\n\
         \x20\x20echo \"$RESP\" | jq -r 'if .error then \"ERROR: \" + .error else ((([.columns | join(\"\\t\")]) + ([.rows[]? | map(tostring) | join(\"\\t\")])) | join(\"\\n\")) end'\n\
         else\n\
         \x20\x20echo \"$RESP\"\n\
         fi\n",
        port = port,
        aid = assist_id,
        key = key,
    )
}

/// Extract the contents of the first ```<lang> ... ``` fenced block.
fn extract_fenced(raw: &str, lang: &str) -> Option<String> {
    let open = format!("```{lang}");
    let start = raw.find(&open)?;
    let after = &raw[start + open.len()..];
    let after = after.strip_prefix('\n').unwrap_or(after);
    let end = after.find("```")?;
    let body = after[..end].trim();
    if body.is_empty() {
        None
    } else {
        Some(body.to_string())
    }
}

/// The first non-empty line of the reply, as a note fallback.
fn first_line(raw: &str) -> String {
    raw.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("Done.")
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompt_has_sentinel_schema_q_and_question() {
        let p = build_assist_prompt("shop_db", "mysql", "shop", "top customers by spend", "nl");
        assert!(p.contains("OTTO_TASK: db_assist"));
        assert!(p.contains("SCHEMA.md"));
        assert!(p.contains("./q '<read-only SQL>'"));
        assert!(p.contains("ANSWER.sql"));
        assert!(p.contains("NOTE.txt"));
        assert!(p.contains("top customers by spend"));
        assert!(p.contains("(mysql)"));
        // nl mode → no RESULT.md reference.
        assert!(!p.contains("RESULT.md"));
    }

    #[test]
    fn investigate_prompt_references_result_md() {
        let p = build_assist_prompt("c", "clickhouse", "logs", "why is this slow", "investigate");
        assert!(p.contains("RESULT.md"));
        assert!(p.contains("examine"));
    }

    #[test]
    fn mode_normalizes_and_drives_task() {
        assert_eq!(normalize_mode(None), "nl");
        assert_eq!(normalize_mode(Some("nl")), "nl");
        assert_eq!(normalize_mode(Some("ask")), "ask");
        assert_eq!(normalize_mode(Some("investigate")), "investigate");
        assert_eq!(normalize_mode(Some("weird")), "nl");
        assert!(mode_task("nl").contains("ONE runnable SQL"));
        assert!(mode_task("ask").contains("free-form"));
        assert!(mode_task("investigate").contains("RESULT.md"));
    }

    #[test]
    fn context_md_carries_details() {
        let md = build_context_md("shop_db", "mysql", "shop", "count orders", "nl");
        assert!(md.contains("Connection: shop_db"));
        assert!(md.contains("Engine: mysql"));
        assert!(md.contains("Active database: shop"));
        assert!(md.contains("count orders"));
        assert!(md.contains("./q"));
        assert!(md.contains("ANSWER.sql"));
    }

    #[test]
    fn node_label_handles_tagged_bare_and_empty() {
        assert_eq!(node_label(Some("db:shop/table:orders")), "shop");
        assert_eq!(node_label(Some("player_details")), "player_details");
        assert_eq!(node_label(None), "(default)");
        assert_eq!(node_label(Some("")), "(default)");
    }

    #[test]
    fn q_script_bakes_port_id_key_and_endpoint() {
        let body = q_script_body(7700, &Id::from("aid-123"), "secret-key");
        assert!(body.starts_with("#!/usr/bin/env bash"));
        assert!(body.contains("http://127.0.0.1:7700"));
        assert!(body.contains("AID=\"aid-123\""));
        assert!(body.contains("KEY=\"secret-key\""));
        assert!(body.contains("/api/v1/db-assist/$AID/query"));
        assert!(body.contains("x-assist-key: $KEY"));
        // Reads SQL from all args (so unquoted multi-word SQL still works).
        assert!(body.contains("SQL=\"$*\""));
    }

    #[test]
    fn extract_fenced_sql_block() {
        let raw = "Here is the query.\n\n```sql\nSELECT * FROM t\n```";
        assert_eq!(extract_fenced(raw, "sql").as_deref(), Some("SELECT * FROM t"));
        assert!(extract_fenced("no fence here", "sql").is_none());
    }

    #[test]
    fn first_line_picks_first_nonempty() {
        assert_eq!(first_line("\n\n  hello \nworld"), "hello");
        assert_eq!(first_line("   "), "Done.");
    }
}
