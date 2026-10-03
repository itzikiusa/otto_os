//! Sessions repository.

use crate::DbPool;
use chrono::Utc;
use otto_core::domain::{Session, SessionKind, SessionStatus};
use otto_core::{new_id, Error, Id, Result};
use sqlx::Row;

use crate::convert::{dberr, fmt, json, ts};

#[derive(Clone)]
pub struct SessionsRepo {
    pool: DbPool,
}

/// What [`SessionsRepo::mark_dormant_except`] changed at boot.
#[derive(Debug, Clone, Default)]
pub struct DormantPass {
    /// `(id, workspace_id, meta)` of the rows that were live and lost their
    /// process with the previous daemon (now `reconnectable`, stamped
    /// `meta.suspended.reason = "restart"`; `meta` is the updated object).
    pub suspended: Vec<(Id, Id, serde_json::Value)>,
    /// `(id, workspace_id)` of exited, resumable agent rows flipped to
    /// `reconnectable`.
    pub resumable: Vec<(Id, Id)>,
}

/// Minimal read-only projection used by the usage tailer to attribute on-disk
/// transcript turns back to Otto sessions. Deliberately *not* filtered by
/// status or `archived`: analysis/agent sessions finish quickly (status
/// `exited`) yet their transcripts keep growing as the user resumes them, and
/// usage from those turns still belongs to the original workspace/session.
#[derive(Debug, Clone)]
pub struct UsageAttrRow {
    pub id: String,
    pub workspace_id: String,
    pub provider: String,
    pub cwd: String,
    /// The CLI's own session uuid (= Claude transcript filename stem). `None`
    /// for sessions that never got a provider id.
    pub provider_session_id: Option<String>,
}

/// Which rows a filtered listing may see in ONE workspace: all of them (root /
/// workspace Admin) or only `owner`'s (the owner-scoped non-admin view, #L1).
#[derive(Debug, Clone)]
pub struct SessionScope {
    pub workspace_id: Id,
    /// `Some(user)` → only sessions `created_by` that user.
    pub owner: Option<Id>,
}

/// Narrowing filters for [`SessionsRepo::list_filtered`], all applied IN SQL
/// (the list used to decode a workspace's whole history — archived rows too —
/// and filter in Rust; 5 k rows was 4.8 MB and held a pool connection for
/// ~100 ms per call). `None` = no constraint.
#[derive(Debug, Clone, Default)]
pub struct SessionListFilter {
    /// `true` → only archived rows; `false` → only active rows.
    pub archived: Option<bool>,
    /// `"agent"` | `"connection"`.
    pub kind: Option<String>,
    /// Exact status string (`running`, `idle`, `working`, `exited`, …).
    pub status: Option<String>,
    /// Exact `meta.source` (a JSON string); the literal `"none"` matches rows
    /// whose `meta.source` is absent or not a string (the foreground ones).
    pub source: Option<String>,
    /// Keep only the NEWEST `limit` matching rows (still returned oldest-first,
    /// like the unpaged list). `None` = unbounded.
    pub limit: Option<u32>,
    /// Paging cursor: only rows created strictly before this RFC 3339 instant
    /// (pass the `created_at` of the oldest row of the previous page).
    pub before: Option<String>,
    /// `Some(true)` → only the rows the sidebar lists: every connection
    /// session plus the agent sessions [`Session::is_foreground_agent`]
    /// accepts (no `meta.source` in `BACKGROUND_SESSION_SOURCES`), plus any
    /// background source named in [`Self::with_sources`]. `Some(false)` → the
    /// complement (background agents only). The same rule, in SQL — a main
    /// workspace with 1.9 k hidden review agents used to ship them all.
    pub foreground: Option<bool>,
    /// With `foreground = Some(true)`: background sources to keep anyway
    /// (e.g. `channel` for the sidebar's Slack/Telegram groups).
    pub with_sources: Vec<String>,
    /// Only these ids (fetch-by-id for open tabs). `Some(empty)` → no rows.
    pub ids: Option<Vec<Id>>,
}

/// Push the SQL form of [`Session::is_foreground_agent`] for an agent row:
/// `meta.source` is absent / not a JSON string / not a background source.
fn push_foreground_agent(q: &mut sqlx::QueryBuilder<sqlx::Sqlite>) {
    q.push(
        "(COALESCE(json_type(meta_json, '$.source'), '') <> 'text' \
          OR json_extract(meta_json, '$.source') NOT IN (",
    );
    let mut sep = q.separated(", ");
    for src in otto_core::domain::BACKGROUND_SESSION_SOURCES {
        sep.push_bind(src);
    }
    q.push("))");
}

/// Insert payload for a new session row.
pub struct NewSession {
    pub workspace_id: Id,
    pub kind: SessionKind,
    pub provider: String,
    pub title: String,
    pub cwd: String,
    pub provider_session_id: Option<String>,
    pub connection_id: Option<Id>,
    pub created_by: Id,
    pub meta: serde_json::Value,
}

pub(crate) fn row_to_session(r: &sqlx::sqlite::SqliteRow) -> Result<Session> {
    Ok(Session {
        id: r.get("id"),
        workspace_id: r.get("workspace_id"),
        kind: SessionKind::parse(&r.get::<String, _>("kind"))
            .ok_or_else(|| Error::Internal("bad session kind".into()))?,
        provider: r.get("provider"),
        title: r.get("title"),
        status: SessionStatus::parse(&r.get::<String, _>("status"))
            .ok_or_else(|| Error::Internal("bad session status".into()))?,
        cwd: r.get("cwd"),
        provider_session_id: r.get("provider_session_id"),
        connection_id: r.get("connection_id"),
        created_by: r.get("created_by"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        last_active_at: ts(&r.get::<String, _>("last_active_at"))?,
        archived: r.get::<i64, _>("archived") != 0,
        meta: json(&r.get::<String, _>("meta_json"))?,
    })
}

/// Test-only latency injection on the hot [`SessionsRepo::get`] path.
///
/// `OTTO_TEST_DB_SLEEP_MS=<ms>` makes every `get` sleep that long before it
/// touches SQLite, so a test can prove a caller does NOT sit on this repo (see
/// `otto-sessions/tests/terminal_latency.rs`, which types into a real PTY
/// through the terminal WS while every session read costs 2 s). Read ONCE per
/// process: with the variable unset — how the daemon always runs — this is a
/// single `OnceLock` load and no sleep at all.
///
/// COMPILED OUT of release builds (`#[cfg(debug_assertions)]`): a shipped
/// `ottod` must not honour the variable even if it somehow lands in the launchd
/// environment. `cargo test` and the e2e daemon are dev-profile builds, so the
/// hook is present exactly where tests need it.
#[cfg(debug_assertions)]
fn injected_delay() -> Option<std::time::Duration> {
    static DELAY: std::sync::OnceLock<Option<std::time::Duration>> = std::sync::OnceLock::new();
    *DELAY.get_or_init(|| {
        std::env::var("OTTO_TEST_DB_SLEEP_MS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|ms| *ms > 0)
            .map(std::time::Duration::from_millis)
    })
}

/// Release twin of [`injected_delay`] — a strict no-op, so the hook cannot be
/// reached in a shipped daemon.
#[cfg(not(debug_assertions))]
#[inline(always)]
fn injected_delay() -> Option<std::time::Duration> {
    None
}

impl SessionsRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        let pool: DbPool = pool.into();
        Self { pool }
    }

    pub fn pool(&self) -> DbPool {
        self.pool.clone()
    }

    pub async fn network_profile(
        &self,
        workspace: &Id,
        meta: &serde_json::Value,
    ) -> Result<Option<otto_core::network_profiles::NetworkProfile>> {
        crate::network_profiles::NetworkProfilesRepo::new(self.pool.clone())
            .selected(workspace, meta)
            .await
    }

    pub async fn workspace(&self, workspace_id: &Id) -> Result<otto_core::domain::Workspace> {
        crate::WorkspacesRepo::new(self.pool.clone())
            .get(workspace_id)
            .await
    }

    /// Resolve curated project context with workspace validation, regardless of
    /// session provider. The launch layer decides how its adapter consumes it.
    pub async fn project_context(
        &self,
        workspace_id: &Id,
        meta: &serde_json::Value,
    ) -> Result<Option<String>> {
        crate::projects::ProjectsRepo::new(self.pool.clone())
            .context(workspace_id, meta)
            .await
    }

    pub async fn create(&self, s: NewSession) -> Result<Session> {
        self.project_context(&s.workspace_id, &s.meta).await?;
        self.network_profile(&s.workspace_id, &s.meta).await?;
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO sessions (id, workspace_id, kind, provider, title, status, cwd,
                                   provider_session_id, connection_id, created_by,
                                   created_at, last_active_at, meta_json)
             VALUES (?, ?, ?, ?, ?, 'running', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&s.workspace_id)
        .bind(s.kind.as_str())
        .bind(&s.provider)
        .bind(&s.title)
        .bind(&s.cwd)
        .bind(&s.provider_session_id)
        .bind(&s.connection_id)
        .bind(&s.created_by)
        .bind(&now)
        .bind(&now)
        .bind(s.meta.to_string())
        .execute(&self.pool)
        .await
        .map_err(dberr("create session"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<Session> {
        if let Some(d) = injected_delay() {
            tokio::time::sleep(d).await;
        }
        let r = sqlx::query("SELECT * FROM sessions WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("session"))?;
        row_to_session(&r)
    }

    pub async fn list_by_workspace(&self, ws: &Id) -> Result<Vec<Session>> {
        let rows = sqlx::query("SELECT * FROM sessions WHERE workspace_id = ? ORDER BY created_at")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Sessions of a workspace **owned by** `user_id` (the `created_by` creator).
    ///
    /// Used to owner-scope the session list for non-admin callers so user A's
    /// sessions never appear in user B's list (leak #L1). Admins/root keep the
    /// unfiltered [`list_by_workspace`].
    pub async fn list_by_workspace_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT * FROM sessions WHERE workspace_id = ? AND created_by = ? ORDER BY created_at",
        )
        .bind(ws)
        .bind(user_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Sessions across `scopes` (one or many workspaces, each full or
    /// owner-scoped) narrowed by `filter` — every predicate runs in SQL, so a
    /// caller asking for the live rows never decodes the archived history.
    /// Oldest first (`created_at, id`), matching [`Self::list_by_workspace`].
    /// Empty `scopes` → empty list (never "every workspace").
    pub async fn list_filtered(
        &self,
        scopes: &[SessionScope],
        filter: &SessionListFilter,
    ) -> Result<Vec<Session>> {
        if scopes.is_empty()
            || filter.limit == Some(0)
            || filter.ids.as_ref().is_some_and(|ids| ids.is_empty())
        {
            return Ok(Vec::new());
        }
        // With a limit: the newest `limit` rows in a subquery, re-sorted
        // oldest-first outside it.
        let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new(if filter.limit.is_some() {
            "SELECT * FROM (SELECT * FROM sessions WHERE ("
        } else {
            "SELECT * FROM sessions WHERE ("
        });
        for (i, scope) in scopes.iter().enumerate() {
            if i > 0 {
                q.push(" OR ");
            }
            q.push("(workspace_id = ")
                .push_bind(scope.workspace_id.clone());
            if let Some(owner) = &scope.owner {
                q.push(" AND created_by = ").push_bind(owner.clone());
            }
            q.push(")");
        }
        q.push(")");
        if let Some(archived) = filter.archived {
            q.push(" AND archived = ").push_bind(archived as i64);
        }
        if let Some(kind) = &filter.kind {
            q.push(" AND kind = ").push_bind(kind.clone());
        }
        if let Some(status) = &filter.status {
            q.push(" AND status = ").push_bind(status.clone());
        }
        if let Some(source) = &filter.source {
            // Parity with the old Rust filter: `meta.source` counts only when it
            // is a JSON string.
            if source == "none" {
                q.push(" AND COALESCE(json_type(meta_json, '$.source'), '') <> 'text'");
            } else {
                q.push(" AND json_type(meta_json, '$.source') = 'text' AND json_extract(meta_json, '$.source') = ")
                    .push_bind(source.clone());
            }
        }
        if let Some(before) = &filter.before {
            q.push(" AND created_at < ").push_bind(before.clone());
        }
        if let Some(fg) = filter.foreground {
            if fg {
                q.push(" AND (kind <> 'agent' OR ");
                push_foreground_agent(&mut q);
                if !filter.with_sources.is_empty() {
                    q.push(" OR (json_type(meta_json, '$.source') = 'text' AND json_extract(meta_json, '$.source') IN (");
                    let mut sep = q.separated(", ");
                    for src in &filter.with_sources {
                        sep.push_bind(src.clone());
                    }
                    q.push("))");
                }
                q.push(")");
            } else {
                q.push(" AND kind = 'agent' AND NOT ");
                push_foreground_agent(&mut q);
            }
        }
        if let Some(ids) = &filter.ids {
            q.push(" AND id IN (");
            let mut sep = q.separated(", ");
            for id in ids {
                sep.push_bind(id.clone());
            }
            q.push(")");
        }
        if let Some(limit) = filter.limit {
            q.push(" ORDER BY created_at DESC, id DESC LIMIT ")
                .push_bind(limit as i64)
                .push(") ORDER BY created_at, id");
        } else {
            q.push(" ORDER BY created_at, id");
        }
        let rows = q
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Ids of the sessions the sidebar shows in `ws` (non-archived,
    /// [`SessionListFilter::foreground`] rule incl. channel rows), optionally
    /// owner-scoped — the id set the workspace usage rollup is filtered by.
    /// Selects only `id`: the rollup used to decode every row (meta included)
    /// of a 2 k-session workspace just to build this set.
    pub async fn visible_ids(&self, ws: &Id, owner: Option<&Id>) -> Result<Vec<Id>> {
        let mut q = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
            "SELECT id FROM sessions WHERE workspace_id = ",
        );
        q.push_bind(ws.clone());
        if let Some(o) = owner {
            q.push(" AND created_by = ").push_bind(o.clone());
        }
        q.push(" AND archived = 0 AND (kind <> 'agent' OR ");
        push_foreground_agent(&mut q);
        q.push(" OR (json_type(meta_json, '$.source') = 'text' AND json_extract(meta_json, '$.source') = 'channel'))");
        let rows = q
            .build()
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("session ids"))?;
        Ok(rows.iter().map(|r| r.get::<String, _>("id")).collect())
    }

    /// Pre-filtered candidates for the provider-title auto-namer: live-ish
    /// (not exited), unarchived agent sessions on a title-bearing provider with
    /// a captured provider id and no settled `meta.title_source`. A SUPERSET of
    /// what `title_eligible` (otto-sessions) accepts — the caller still applies
    /// it (e.g. the background-source check) — but it keeps the 20 s sweep off
    /// the whole table (it used to decode every session ever, archived too).
    pub async fn list_title_candidates(&self) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT * FROM sessions \
             WHERE archived = 0 AND status <> 'exited' AND kind = 'agent' \
               AND provider IN ('claude', 'codex') \
               AND provider_session_id IS NOT NULL \
               AND NOT (COALESCE(json_type(meta_json, '$.title_source'), '') = 'text' \
                        AND json_extract(meta_json, '$.title_source') IN ('user', 'provider')) \
             ORDER BY created_at DESC",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("title candidates"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// **Every** session across **all** workspaces, newest first.
    ///
    /// The sanctioned cross-user, cross-workspace read backing the admin
    /// active-sessions overview (`GET /api/v1/admin/sessions`, Task 4.2). It is
    /// deliberately unfiltered by owner, workspace, status or `archived` — the
    /// admin overview wants the whole picture and enriches each row with live
    /// state from the `SessionManager`. Gated by `Users:Admin`/root at the route,
    /// not here.
    pub async fn list_all(&self) -> Result<Vec<Session>> {
        let rows = sqlx::query("SELECT * FROM sessions ORDER BY created_at DESC")
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("all sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Sessions that should be revived or marked reconnectable on daemon boot.
    ///
    /// Includes exited agent sessions that have a `provider_session_id` — those
    /// can be resumed with `--resume` even after the daemon restarts.
    pub async fn list_all_restorable(&self) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT * FROM sessions \
             WHERE archived = 0 \
               AND (status != 'exited' \
                    OR (kind = 'agent' AND provider_session_id IS NOT NULL))",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    pub async fn update_status(&self, id: &Id, status: SessionStatus) -> Result<()> {
        sqlx::query("UPDATE sessions SET status = ?, last_active_at = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update session status"))?;
        Ok(())
    }

    /// [`Self::update_status`] WITHOUT stamping `last_active_at`: for status
    /// corrections that are not activity — re-adopting a held PTY after a
    /// daemon restart must not reset the session's idle clock (review A14).
    pub async fn update_status_keep_activity(&self, id: &Id, status: SessionStatus) -> Result<()> {
        sqlx::query("UPDATE sessions SET status = ? WHERE id = ?")
            .bind(status.as_str())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update session status"))?;
        Ok(())
    }

    /// Daemon-boot dormant pass (review P1), set-based: every restorable,
    /// non-archived session NOT in `keep` (the ones just re-adopted from their
    /// PTY holders) becomes `reconnectable` in two statements instead of one
    /// UPDATE per row. `last_active_at` is left alone — a restart is not
    /// activity (stamping it broke auto-archive and recency ordering).
    ///
    /// - Rows that were live (`status` not `reconnectable`/`exited`) lost their
    ///   process with the old daemon: they also get
    ///   `meta.suspended = {reason: "restart", at}`.
    /// - Exited agent rows with a `provider_session_id` are resumable with
    ///   `--resume`, so they flip to `reconnectable` too (no stamp: their
    ///   process ended on its own).
    ///
    /// Rows already `reconnectable` are not touched at all. Returns the rows
    /// that changed, so the caller broadcasts only those.
    pub async fn mark_dormant_except(&self, keep: &[Id], at: &str) -> Result<DormantPass> {
        let keep_json = serde_json::to_string(keep).unwrap_or_else(|_| "[]".into());
        let stamp = serde_json::json!({ "suspended": { "reason": "restart", "at": at } });
        let live = sqlx::query(
            "UPDATE sessions SET status = 'reconnectable',
                 meta_json = json_patch(
                     CASE WHEN meta_json IS NOT NULL AND json_valid(meta_json)
                               AND json_type(meta_json) = 'object'
                          THEN meta_json ELSE '{}' END,
                     ?)
             WHERE archived = 0
               AND status NOT IN ('reconnectable', 'exited')
               AND id NOT IN (SELECT value FROM json_each(?))
             RETURNING id, workspace_id, meta_json",
        )
        .bind(stamp.to_string())
        .bind(&keep_json)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("mark sessions dormant"))?;
        let resumable = sqlx::query(
            "UPDATE sessions SET status = 'reconnectable'
             WHERE archived = 0 AND status = 'exited'
               AND kind = 'agent' AND provider_session_id IS NOT NULL
               AND id NOT IN (SELECT value FROM json_each(?))
             RETURNING id, workspace_id",
        )
        .bind(&keep_json)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("mark exited sessions resumable"))?;
        Ok(DormantPass {
            suspended: live
                .iter()
                .map(|r| {
                    let meta: Option<String> = r.get("meta_json");
                    (
                        r.get::<String, _>("id"),
                        r.get::<String, _>("workspace_id"),
                        meta.and_then(|m| serde_json::from_str(&m).ok())
                            .unwrap_or(serde_json::Value::Null),
                    )
                })
                .collect(),
            resumable: resumable
                .iter()
                .map(|r| (r.get::<String, _>("id"), r.get::<String, _>("workspace_id")))
                .collect(),
        })
    }

    pub async fn set_provider_session(&self, id: &Id, provider_session_id: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET provider_session_id = ? WHERE id = ?")
            .bind(provider_session_id)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update session"))?;
        Ok(())
    }

    /// Persist the resolved on-disk transcript path (conversation view, design
    /// §4.2). Written the moment a capture / first transcript read resolves it
    /// so later lookups are O(1) — Codex rollouts are otherwise only found by
    /// scanning `~/.codex/sessions`.
    pub async fn set_transcript_path(&self, id: &Id, path: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET transcript_path = ? WHERE id = ?")
            .bind(path)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update session transcript path"))?;
        Ok(())
    }

    /// The persisted transcript path for `id` (`None` when never resolved).
    pub async fn transcript_path(&self, id: &Id) -> Result<Option<String>> {
        let r = sqlx::query("SELECT transcript_path FROM sessions WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("session"))?;
        Ok(r.get("transcript_path"))
    }

    /// `(session id, transcript_path)` for every session in `ws` — the History
    /// merge joins these onto the workspace's session list.
    pub async fn transcript_paths_for_workspace(
        &self,
        ws: &Id,
    ) -> Result<Vec<(Id, Option<String>)>> {
        let rows = sqlx::query("SELECT id, transcript_path FROM sessions WHERE workspace_id = ?")
            .bind(ws)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("sessions"))?;
        Ok(rows
            .iter()
            .map(|r| {
                (
                    r.get::<String, _>("id"),
                    r.get::<Option<String>, _>("transcript_path"),
                )
            })
            .collect())
    }

    /// Every non-null `transcript_path` across all sessions — with
    /// [`Self::provider_session_ids`] this is the "claimed" set that keeps an
    /// indexed on-disk transcript from ALSO showing up as an `on_disk` History
    /// entry.
    pub async fn transcript_paths(&self) -> Result<Vec<String>> {
        let rows =
            sqlx::query("SELECT transcript_path FROM sessions WHERE transcript_path IS NOT NULL")
                .fetch_all(&self.pool)
                .await
                .map_err(dberr("sessions"))?;
        Ok(rows.iter().map(|r| r.get("transcript_path")).collect())
    }

    /// The session (any workspace) that already owns `provider_session_id`, if
    /// one does — History import returns it instead of minting a duplicate.
    pub async fn find_by_provider_session(
        &self,
        provider_session_id: &str,
    ) -> Result<Option<Session>> {
        let r = sqlx::query("SELECT * FROM sessions WHERE provider_session_id = ? LIMIT 1")
            .bind(provider_session_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("session"))?;
        r.as_ref().map(row_to_session).transpose()
    }

    /// Every non-null `provider_session_id` currently recorded — the "claimed"
    /// set. Used by the codex session-id capture task to avoid two sessions
    /// grabbing the same on-disk rollout (a provider id is unique to one session).
    pub async fn provider_session_ids(&self) -> Result<Vec<String>> {
        let rows = sqlx::query(
            "SELECT provider_session_id FROM sessions \
             WHERE provider_session_id IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("sessions"))?;
        Ok(rows.iter().map(|r| r.get("provider_session_id")).collect())
    }

    pub async fn set_title(&self, id: &Id, title: &str) -> Result<()> {
        sqlx::query("UPDATE sessions SET title = ? WHERE id = ?")
            .bind(title)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update session"))?;
        Ok(())
    }

    pub async fn set_meta(&self, id: &Id, meta: &serde_json::Value) -> Result<()> {
        sqlx::query("UPDATE sessions SET meta_json = ? WHERE id = ?")
            .bind(meta.to_string())
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("update session meta"))?;
        Ok(())
    }

    /// Atomically merge `patch` (a JSON object) into `sessions.meta_json` in a
    /// single UPDATE — RFC-7396 semantics via SQLite's `json_patch`: null values
    /// remove keys, everything else upserts. Unlike a read-modify-write through
    /// `get`+`set_meta`, two concurrent merges (e.g. a resize racing a
    /// keep-alive toggle) can never overwrite each other with a stale snapshot.
    /// Non-object / NULL / invalid existing meta is treated as `{}`.
    pub async fn merge_meta(&self, id: &Id, patch: &serde_json::Value) -> Result<()> {
        sqlx::query(
            "UPDATE sessions SET meta_json = json_patch(
                 CASE WHEN meta_json IS NOT NULL AND json_valid(meta_json)
                           AND json_type(meta_json) = 'object'
                      THEN meta_json ELSE '{}' END,
                 ?)
             WHERE id = ?",
        )
        .bind(patch.to_string())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("merge session meta"))?;
        Ok(())
    }

    /// Replace object-valued top-level keys without exposing a missing-key
    /// intermediate state. Both merge patches belong to one atomic UPDATE.
    pub async fn replace_meta_keys(&self, id: &Id, patch: &serde_json::Value) -> Result<()> {
        if patch.get("account_id").is_some() || patch.get("account_label").is_some() {
            return Err(Error::Invalid(
                "session account is immutable; create a new session to choose another account"
                    .into(),
            ));
        }
        if patch.get("project_id").is_some() {
            let session = self.get(id).await?;
            self.project_context(&session.workspace_id, patch).await?;
        }
        if patch.get("network_profile_id").is_some() {
            let session = self.get(id).await?;
            self.network_profile(&session.workspace_id, patch).await?;
        }
        let Some(object) = patch.as_object() else {
            return Ok(());
        };
        let nulls: serde_json::Map<String, serde_json::Value> = object
            .iter()
            .filter(|(_, value)| value.is_object())
            .map(|(key, _)| (key.clone(), serde_json::Value::Null))
            .collect();
        sqlx::query(
            "UPDATE sessions SET meta_json = json_patch(json_patch(
                 CASE WHEN meta_json IS NOT NULL AND json_valid(meta_json)
                           AND json_type(meta_json) = 'object'
                      THEN meta_json ELSE '{}' END, ?), ?)
             WHERE id = ?",
        )
        .bind(serde_json::Value::Object(nulls).to_string())
        .bind(patch.to_string())
        .bind(id)
        .execute(&self.pool)
        .await
        .map_err(dberr("replace session meta keys"))?;
        Ok(())
    }

    pub async fn set_archived(&self, id: &Id, archived: bool) -> Result<()> {
        sqlx::query("UPDATE sessions SET archived = ? WHERE id = ?")
            .bind(archived as i64)
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("archive session"))?;
        Ok(())
    }

    pub async fn touch(&self, id: &Id) -> Result<()> {
        sqlx::query("UPDATE sessions SET last_active_at = ? WHERE id = ?")
            .bind(fmt(Utc::now()))
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("touch session"))?;
        Ok(())
    }

    pub async fn delete(&self, id: &Id) -> Result<()> {
        sqlx::query("DELETE FROM sessions WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await
            .map_err(dberr("delete session"))?;
        Ok(())
    }

    /// Unarchived sessions active at or after `since` (RFC3339), newest first.
    pub async fn list_active_since(&self, ws: &Id, since: &str) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT * FROM sessions \
             WHERE workspace_id = ? AND archived = 0 AND last_active_at >= ? \
             ORDER BY last_active_at DESC",
        )
        .bind(ws)
        .bind(since)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Non-archived, channel-spawned agent sessions idle longer than `max_idle`
    /// — used to auto-archive stale ticket/chat sessions so they don't pile up
    /// in the sidebar. Oldest first. A `working` session is never idle, whatever
    /// its `last_active_at` says: that column only moves on a status transition,
    /// so a long turn (or a reply that arrived while the agent was already
    /// working) would otherwise get archived mid-answer — and the thread's next
    /// message would spawn a stranger.
    pub async fn list_idle_channel_sessions(
        &self,
        max_idle: std::time::Duration,
    ) -> Result<Vec<Session>> {
        let cutoff = fmt(Utc::now()
            - chrono::Duration::from_std(max_idle).unwrap_or_else(|_| chrono::Duration::zero()));
        let before = cutoff.as_str();
        let rows = sqlx::query(
            "SELECT * FROM sessions \
             WHERE archived = 0 AND kind = 'agent' AND status != 'working' AND last_active_at < ? \
               AND json_extract(meta_json, '$.source') = 'channel' \
             ORDER BY last_active_at",
        )
        .bind(before)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("idle channel sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Archived, channel-spawned agent sessions whose last activity is older
    /// than `max_age` — used to permanently delete closed ticket/chat sessions
    /// so the DB doesn't grow without bound at ticketing volume. Oldest first.
    pub async fn list_archived_channel_sessions_older_than(
        &self,
        max_age: std::time::Duration,
    ) -> Result<Vec<Session>> {
        let cutoff = fmt(Utc::now()
            - chrono::Duration::from_std(max_age).unwrap_or_else(|_| chrono::Duration::zero()));
        let before = cutoff.as_str();
        let rows = sqlx::query(
            "SELECT * FROM sessions \
             WHERE archived = 1 AND kind = 'agent' AND last_active_at < ? \
               AND json_extract(meta_json, '$.source') = 'channel' \
             ORDER BY last_active_at",
        )
        .bind(before)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("old archived channel sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// Agent sessions that are candidates for the existence-check pruner:
    /// non-running (status `exited` or `reconnectable`) agent sessions that
    /// carry a `provider_session_id` (so they could in principle be resumed).
    ///
    /// Includes archived rows — an archived session whose provider transcript
    /// is gone is also un-resumable and should be cleaned up. The pruner then
    /// verifies each against the provider's on-disk transcript before deleting;
    /// rows it cannot verify are kept.
    pub async fn list_prunable_agent_sessions(&self) -> Result<Vec<Session>> {
        let rows = sqlx::query(
            "SELECT * FROM sessions \
             WHERE kind = 'agent' \
               AND provider_session_id IS NOT NULL \
               AND status IN ('exited', 'reconnectable')",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("prunable agent sessions"))?;
        rows.iter().map(row_to_session).collect()
    }

    /// All sessions projected to the fields the usage tailer needs to attribute
    /// on-disk transcript turns. Read-only and unfiltered (see [`UsageAttrRow`]).
    pub async fn list_usage_attribution(&self) -> sqlx::Result<Vec<UsageAttrRow>> {
        let rows = sqlx::query(
            "SELECT id, workspace_id, provider, cwd, provider_session_id FROM sessions",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .iter()
            .map(|r| UsageAttrRow {
                id: r.get("id"),
                workspace_id: r.get("workspace_id"),
                provider: r.get("provider"),
                cwd: r.get("cwd"),
                provider_session_id: r.get("provider_session_id"),
            })
            .collect())
    }

    /// Count of sessions in a workspace for a provider (for "claude #N" titles).
    pub async fn count_by_provider(&self, ws: &Id, provider: &str) -> Result<i64> {
        let r = sqlx::query(
            "SELECT COUNT(*) AS n FROM sessions WHERE workspace_id = ? AND provider = ?",
        )
        .bind(ws)
        .bind(provider)
        .fetch_one(&self.pool)
        .await
        .map_err(dberr("count sessions"))?;
        Ok(r.get("n"))
    }
}

// --- Targeted live-session lookups (perf §15 F7) ---------------------------
impl SessionsRepo {
    /// Non-archived, non-exited sessions of `ws` (optionally of one `kind`)
    /// whose `meta.<meta_key>` equals `value` — filtered in SQL so a lookup
    /// never decodes the workspace's whole session history (the swarm's
    /// per-turn agent-session reuse used to load ~2k rows / 345 KB of
    /// `meta_json`). `meta_key` is a compile-time identifier, never input.
    pub async fn list_live_by_meta(
        &self,
        ws: &Id,
        kind: Option<&str>,
        meta_key: &'static str,
        value: &str,
    ) -> Result<Vec<Session>> {
        debug_assert!(meta_key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_'));
        let q = format!(
            "SELECT * FROM sessions WHERE workspace_id = ? AND archived = 0 \
             AND status != 'exited' AND (? IS NULL OR kind = ?) \
             AND json_extract(meta_json, '$.{meta_key}') = ? ORDER BY created_at"
        );
        let rows = sqlx::query(sqlx::AssertSqlSafe(q.as_str()))
            .bind(ws)
            .bind(kind)
            .bind(kind)
            .bind(value)
            .fetch_all(&self.pool)
            .await
            .map_err(dberr("sessions by meta"))?;
        rows.iter().map(row_to_session).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        pool.into()
    }

    async fn seed_user_ws(pool: &DbPool) -> (String, String) {
        let user = new_id();
        let ws = new_id();
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, ?, ?, 0, ?)")
            .bind(&user).bind("u").bind("x").bind("U").bind(&now)
            .execute(pool).await.unwrap();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
            .bind(&ws)
            .bind("w")
            .bind("/tmp")
            .bind(&now)
            .execute(pool)
            .await
            .unwrap();
        (user, ws)
    }

    async fn insert_session(
        pool: &DbPool,
        ws: &str,
        user: &str,
        last_active: &str,
        meta: &str,
        archived: i64,
    ) -> String {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO sessions (id, workspace_id, kind, provider, title, status, cwd,
                                   created_by, created_at, last_active_at, archived, meta_json)
             VALUES (?, ?, 'agent', 'claude', 't', 'idle', '/tmp', ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(ws)
        .bind(user)
        .bind(&now)
        .bind(last_active)
        .bind(archived)
        .bind(meta)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    #[allow(clippy::too_many_arguments)]
    async fn insert_session_full(
        pool: &DbPool,
        ws: &str,
        user: &str,
        provider: &str,
        status: &str,
        provider_session_id: Option<&str>,
        archived: i64,
    ) -> String {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO sessions (id, workspace_id, kind, provider, title, status, cwd,
                                   provider_session_id, created_by, created_at, last_active_at,
                                   archived, meta_json)
             VALUES (?, ?, 'agent', ?, 't', ?, '/tmp', ?, ?, ?, ?, ?, '{}')",
        )
        .bind(&id)
        .bind(ws)
        .bind(provider)
        .bind(status)
        .bind(provider_session_id)
        .bind(user)
        .bind(&now)
        .bind(&now)
        .bind(archived)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    #[tokio::test]
    async fn merge_meta_is_atomic_and_preserves_other_keys() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());
        let s = repo
            .create(NewSession {
                workspace_id: ws.clone(),
                kind: SessionKind::Agent,
                provider: "claude".into(),
                title: "merge-meta-test".into(),
                cwd: "/tmp".into(),
                provider_session_id: None,
                connection_id: None,
                created_by: user.clone(),
                meta: serde_json::json!({}),
            })
            .await
            .unwrap();

        // Two independent merges (a keep-alive toggle racing a resize persist):
        // neither may clobber the other's key — the old get→set read-modify-
        // write lost one of them.
        repo.merge_meta(&s.id, &serde_json::json!({ "keep_alive": true }))
            .await
            .unwrap();
        repo.merge_meta(
            &s.id,
            &serde_json::json!({ "pty_cols": 120, "pty_rows": 40 }),
        )
        .await
        .unwrap();
        let got = repo.get(&s.id).await.unwrap();
        assert_eq!(
            got.meta.get("keep_alive"),
            Some(&serde_json::Value::Bool(true))
        );
        assert_eq!(got.meta.get("pty_cols"), Some(&serde_json::json!(120)));

        // Null removes a key (RFC-7396); scalars replace.
        repo.merge_meta(
            &s.id,
            &serde_json::json!({ "keep_alive": null, "pty_cols": 80 }),
        )
        .await
        .unwrap();
        let got = repo.get(&s.id).await.unwrap();
        assert!(got.meta.get("keep_alive").is_none());
        assert_eq!(got.meta.get("pty_cols"), Some(&serde_json::json!(80)));
        assert_eq!(got.meta.get("pty_rows"), Some(&serde_json::json!(40)));
    }

    #[tokio::test]
    async fn replacing_saved_delivery_is_one_atomic_update() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());
        let session = repo.create(NewSession {
            workspace_id: ws, kind: SessionKind::Agent, provider: "test".into(),
            title: "atomic metadata".into(), cwd: "/tmp".into(),
            provider_session_id: None, connection_id: None, created_by: user,
            meta: serde_json::json!({"handover":{"state":"preparing","obsolete":true},"pty_cols":120}),
        }).await.unwrap();
        // This trigger observes EVERY write, including the old implementation's
        // removal pass. A crash at that point used to lose the durable record.
        sqlx::raw_sql("CREATE TRIGGER keep_delivery BEFORE UPDATE OF meta_json ON sessions WHEN json_type(NEW.meta_json, '$.handover') IS NULL BEGIN SELECT RAISE(ABORT, 'saved delivery disappeared'); END;")
            .execute(&pool).await.unwrap();
        repo.replace_meta_keys(
            &session.id,
            &serde_json::json!({"handover":{"state":"sent","brief":"keep this"}}),
        )
        .await
        .unwrap();
        let saved = repo.get(&session.id).await.unwrap();
        assert_eq!(
            saved.meta["handover"],
            serde_json::json!({"state":"sent","brief":"keep this"})
        );
        assert_eq!(saved.meta["pty_cols"], 120);
    }

    #[tokio::test]
    async fn prunable_agent_sessions_query_filters_correctly() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());

        // Matches: exited + has provider_session_id.
        let exited =
            insert_session_full(&pool, &ws, &user, "claude", "exited", Some("sid-1"), 0).await;
        // Matches: reconnectable + has provider_session_id (archived still counts).
        let recon = insert_session_full(
            &pool,
            &ws,
            &user,
            "claude",
            "reconnectable",
            Some("sid-2"),
            1,
        )
        .await;
        // Excluded: still running.
        insert_session_full(&pool, &ws, &user, "claude", "running", Some("sid-3"), 0).await;
        // Excluded: working.
        insert_session_full(&pool, &ws, &user, "claude", "working", Some("sid-4"), 0).await;
        // Excluded: exited but no provider_session_id (can't be resumed/verified).
        insert_session_full(&pool, &ws, &user, "shell", "exited", None, 0).await;

        let got = repo.list_prunable_agent_sessions().await.unwrap();
        let mut ids: Vec<&str> = got.iter().map(|s| s.id.as_str()).collect();
        ids.sort();
        let mut want = vec![exited.as_str(), recon.as_str()];
        want.sort();
        assert_eq!(ids, want);
    }

    #[tokio::test]
    async fn list_usage_attribution_returns_all_sessions_unfiltered() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());

        // A live claude session with a provider id.
        let live =
            insert_session_full(&pool, &ws, &user, "claude", "running", Some("psid-1"), 0).await;
        // An exited+archived codex session with NO provider id — must still be
        // returned (analysis sessions finish fast but transcripts keep growing).
        let exited = insert_session_full(&pool, &ws, &user, "codex", "exited", None, 1).await;

        let got = repo.list_usage_attribution().await.unwrap();
        let mut ids: Vec<&str> = got.iter().map(|r| r.id.as_str()).collect();
        ids.sort();
        let mut want = vec![live.as_str(), exited.as_str()];
        want.sort();
        assert_eq!(ids, want);

        let live_row = got.iter().find(|r| r.id == live).unwrap();
        assert_eq!(live_row.provider, "claude");
        assert_eq!(live_row.workspace_id, ws);
        assert_eq!(live_row.provider_session_id.as_deref(), Some("psid-1"));

        let exited_row = got.iter().find(|r| r.id == exited).unwrap();
        assert_eq!(exited_row.provider, "codex");
        assert_eq!(exited_row.provider_session_id, None);
    }

    /// Seed an extra user into an existing pool and return its id.
    async fn seed_extra_user(pool: &DbPool, username: &str) -> String {
        let id = new_id();
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, ?, ?, 0, ?)")
            .bind(&id).bind(username).bind("x").bind(username).bind(&now)
            .execute(pool).await.unwrap();
        id
    }

    #[tokio::test]
    async fn list_by_workspace_for_user_filters_by_owner() {
        let pool = mem_pool().await;
        let (alice, ws) = seed_user_ws(&pool).await;
        let bob = seed_extra_user(&pool, "bob").await;
        let repo = SessionsRepo::new(pool.clone());

        // Two sessions for alice, one for bob — same workspace.
        let a1 = insert_session_full(&pool, &ws, &alice, "claude", "running", None, 0).await;
        let a2 = insert_session_full(&pool, &ws, &alice, "shell", "running", None, 0).await;
        let b1 = insert_session_full(&pool, &ws, &bob, "claude", "running", None, 0).await;

        // alice sees only her two; bob sees only his one.
        let alice_ids: std::collections::HashSet<String> = repo
            .list_by_workspace_for_user(&ws, &alice)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(
            alice_ids,
            [a1.clone(), a2.clone()].into_iter().collect(),
            "alice must see only her own sessions"
        );
        assert!(!alice_ids.contains(&b1), "alice must not see bob's session");

        let bob_ids: Vec<String> = repo
            .list_by_workspace_for_user(&ws, &bob)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(bob_ids, vec![b1.clone()], "bob sees only his own");

        // The unscoped list still returns all three (admin/root path).
        let all = repo.list_by_workspace(&ws).await.unwrap();
        assert_eq!(all.len(), 3, "unscoped list returns every session");
    }

    #[tokio::test]
    async fn list_all_returns_every_session_across_owners_and_workspaces() {
        let pool = mem_pool().await;
        let (alice, ws1) = seed_user_ws(&pool).await;
        let bob = seed_extra_user(&pool, "bob").await;
        // A second workspace so we prove the listing crosses workspaces too.
        let ws2 = new_id();
        let now = fmt(Utc::now());
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
            .bind(&ws2)
            .bind("w2")
            .bind("/tmp2")
            .bind(&now)
            .execute(&pool)
            .await
            .unwrap();
        let repo = SessionsRepo::new(pool.clone());

        let a1 = insert_session_full(&pool, &ws1, &alice, "claude", "running", None, 0).await;
        let b1 = insert_session_full(&pool, &ws1, &bob, "shell", "exited", None, 0).await;
        // bob in a different workspace, archived — still part of "all".
        let b2 = insert_session_full(&pool, &ws2, &bob, "codex", "exited", None, 1).await;

        let all: std::collections::HashSet<String> = repo
            .list_all()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(
            all,
            [a1, b1, b2].into_iter().collect(),
            "list_all must return every session regardless of owner/workspace/status/archived"
        );
    }

    #[tokio::test]
    async fn idle_channel_sessions_query_filters_correctly() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());

        let old = fmt(Utc::now() - ChronoDuration::hours(20));
        let recent = fmt(Utc::now());
        // The only row that should match: old + channel + not archived.
        let idle = insert_session(
            &pool,
            &ws,
            &user,
            &old,
            r#"{"source":"channel","channel":"telegram"}"#,
            0,
        )
        .await;
        // Excluded: recent channel session.
        insert_session(&pool, &ws, &user, &recent, r#"{"source":"channel"}"#, 0).await;
        // Excluded: old but not a channel session.
        insert_session(&pool, &ws, &user, &old, "{}", 0).await;
        // Excluded: old channel session but already archived.
        insert_session(&pool, &ws, &user, &old, r#"{"source":"channel"}"#, 1).await;
        // Excluded: old timestamp but mid-turn — `last_active_at` only moves on a
        // status transition, so a working session can look stale while answering.
        let working = insert_session(
            &pool,
            &ws,
            &user,
            &old,
            r#"{"source":"channel","channel":"slack"}"#,
            0,
        )
        .await;
        sqlx::query("UPDATE sessions SET status = 'working' WHERE id = ?")
            .bind(&working)
            .execute(&pool)
            .await
            .unwrap();

        let got = repo
            .list_idle_channel_sessions(std::time::Duration::from_secs(12 * 60 * 60))
            .await
            .unwrap();
        let ids: Vec<&str> = got.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec![idle.as_str()]);
    }

    /// Insert a row with every filterable column chosen by the caller.
    #[allow(clippy::too_many_arguments)]
    async fn insert_row(
        pool: &DbPool,
        ws: &str,
        user: &str,
        kind: &str,
        status: &str,
        archived: i64,
        meta: &str,
        created_at: &str,
    ) -> String {
        let id = new_id();
        sqlx::query(
            "INSERT INTO sessions (id, workspace_id, kind, provider, title, status, cwd,
                                   created_by, created_at, last_active_at, archived, meta_json)
             VALUES (?, ?, ?, 'claude', 't', ?, '/tmp', ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(ws)
        .bind(kind)
        .bind(status)
        .bind(user)
        .bind(created_at)
        .bind(created_at)
        .bind(archived)
        .bind(meta)
        .execute(pool)
        .await
        .unwrap();
        id
    }

    /// The pre-SQL behaviour of `GET /workspaces/{id}/sessions`: fetch all,
    /// then filter in Rust. `list_filtered` must return exactly this set.
    fn rust_filter(all: Vec<Session>, f: &SessionListFilter) -> Vec<String> {
        all.into_iter()
            .filter(|s| f.archived.is_none_or(|a| s.archived == a))
            .filter(|s| f.kind.as_deref().is_none_or(|k| s.kind.as_str() == k))
            .filter(|s| {
                f.source.as_deref().is_none_or(|want| {
                    let src = s.meta.get("source").and_then(|v| v.as_str());
                    if want == "none" {
                        src.is_none()
                    } else {
                        src == Some(want)
                    }
                })
            })
            .filter(|s| f.status.as_deref().is_none_or(|st| s.status.as_str() == st))
            .map(|s| s.id)
            .collect()
    }

    #[tokio::test]
    async fn list_filtered_matches_the_old_rust_filter() {
        let pool = mem_pool().await;
        let (alice, ws) = seed_user_ws(&pool).await;
        let bob = seed_extra_user(&pool, "bob").await;
        let repo = SessionsRepo::new(pool.clone());
        let metas = [
            "{}",
            r#"{"source":"channel"}"#,
            r#"{"source":"review"}"#,
            r#"{"source":7}"#,
            r#"{"source":null}"#,
            r#"{"other":"x"}"#,
        ];
        let mut n = 0;
        for kind in ["agent", "connection"] {
            for status in ["running", "idle", "working", "exited"] {
                for archived in [0, 1] {
                    for meta in metas {
                        let user = if n % 3 == 0 { &bob } else { &alice };
                        let at = format!("2026-01-01T00:{:02}:{:02}+00:00", n / 60, n % 60);
                        insert_row(&pool, &ws, user, kind, status, archived, meta, &at).await;
                        n += 1;
                    }
                }
            }
        }
        let opt = |v: &str| (!v.is_empty()).then(|| v.to_string());
        let mut checked = 0;
        for archived in [None, Some(false), Some(true)] {
            for kind in ["", "agent", "connection"] {
                for status in ["", "working", "exited"] {
                    for source in ["", "none", "channel", "review"] {
                        let f = SessionListFilter {
                            archived,
                            kind: opt(kind),
                            status: opt(status),
                            source: opt(source),
                            ..Default::default()
                        };
                        // Full scope (admin) and owner scope (non-admin).
                        let full = [SessionScope {
                            workspace_id: ws.clone(),
                            owner: None,
                        }];
                        let got: Vec<String> = repo
                            .list_filtered(&full, &f)
                            .await
                            .unwrap()
                            .into_iter()
                            .map(|s| s.id)
                            .collect();
                        let want = rust_filter(repo.list_by_workspace(&ws).await.unwrap(), &f);
                        let mut got_sorted = got.clone();
                        got_sorted.sort();
                        let mut want_sorted = want.clone();
                        want_sorted.sort();
                        assert_eq!(got_sorted, want_sorted, "admin scope, filter {f:?}");
                        let mine = [SessionScope {
                            workspace_id: ws.clone(),
                            owner: Some(alice.clone()),
                        }];
                        let got: Vec<String> = repo
                            .list_filtered(&mine, &f)
                            .await
                            .unwrap()
                            .into_iter()
                            .map(|s| s.id)
                            .collect();
                        let want = rust_filter(
                            repo.list_by_workspace_for_user(&ws, &alice).await.unwrap(),
                            &f,
                        );
                        assert_eq!(got, want, "owner scope (same order), filter {f:?}");
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 108);
    }

    #[tokio::test]
    async fn list_filtered_spans_workspaces_and_pages_newest_first() {
        let pool = mem_pool().await;
        let (alice, ws1) = seed_user_ws(&pool).await;
        let bob = seed_extra_user(&pool, "bob").await;
        let ws2 = new_id();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'w2', '/tmp', ?)",
        )
        .bind(&ws2)
        .bind(fmt(Utc::now()))
        .execute(&pool)
        .await
        .unwrap();
        let repo = SessionsRepo::new(pool.clone());
        let mut ids = Vec::new();
        for i in 0..6 {
            let at = format!("2026-01-01T00:00:{i:02}+00:00");
            let ws = if i % 2 == 0 { &ws1 } else { &ws2 };
            let user = if i == 5 { &bob } else { &alice };
            ids.push(insert_row(&pool, ws, user, "agent", "idle", 0, "{}", &at).await);
        }
        // An archived row the live-only listing must never return.
        insert_row(
            &pool,
            &ws1,
            &alice,
            "agent",
            "idle",
            1,
            "{}",
            "2026-01-01T00:00:09+00:00",
        )
        .await;
        let scopes = [
            SessionScope {
                workspace_id: ws1.clone(),
                owner: None,
            },
            // ws2 owner-scoped to alice: bob's row (i = 5) is hidden.
            SessionScope {
                workspace_id: ws2.clone(),
                owner: Some(alice.clone()),
            },
        ];
        let live = SessionListFilter {
            archived: Some(false),
            ..Default::default()
        };
        let got: Vec<String> = repo
            .list_filtered(&scopes, &live)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(
            got,
            ids[..5].to_vec(),
            "both workspaces, oldest first, owner scope honoured"
        );

        // Page 1: the newest 2 (still oldest-first); page 2 via `before`.
        let page1 = SessionListFilter {
            archived: Some(false),
            limit: Some(2),
            ..Default::default()
        };
        let p1 = repo.list_filtered(&scopes, &page1).await.unwrap();
        assert_eq!(
            p1.iter().map(|s| s.id.clone()).collect::<Vec<_>>(),
            ids[3..5].to_vec()
        );
        let page2 = SessionListFilter {
            before: Some(fmt(p1[0].created_at)),
            ..page1.clone()
        };
        let p2: Vec<String> = repo
            .list_filtered(&scopes, &page2)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(p2, ids[1..3].to_vec());
        assert!(
            repo.list_filtered(&[], &live).await.unwrap().is_empty(),
            "no scope, no rows"
        );
    }

    /// F1 parity: `foreground` in SQL is exactly `Session::is_foreground_agent`
    /// (plus every connection row), across every background source, odd
    /// `meta.source` shapes, `with_sources` and the `ids` filter.
    #[tokio::test]
    async fn foreground_filter_matches_is_foreground_agent() {
        let pool = mem_pool().await;
        let (alice, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());
        let mut metas: Vec<String> = otto_core::domain::BACKGROUND_SESSION_SOURCES
            .iter()
            .map(|s| format!(r#"{{"source":"{s}"}}"#))
            .collect();
        metas.extend(
            [
                "{}",
                r#"{"source":7}"#,
                r#"{"source":null}"#,
                r#"{"source":""}"#,
                r#"{"source":"manual"}"#,
                r#"{"source":"Review"}"#,
                r#"{"source":["review"]}"#,
                r#"{"other":"review"}"#,
            ]
            .map(String::from),
        );
        let mut n = 0;
        for kind in ["agent", "connection"] {
            for archived in [0, 1] {
                for meta in &metas {
                    let at = format!("2026-01-01T00:{:02}:{:02}+00:00", n / 60, n % 60);
                    insert_row(&pool, &ws, &alice, kind, "idle", archived, meta, &at).await;
                    n += 1;
                }
            }
        }
        let all = repo.list_by_workspace(&ws).await.unwrap();
        let scope = [SessionScope {
            workspace_id: ws.clone(),
            owner: None,
        }];
        let ids_of = |v: Vec<Session>| {
            let mut v: Vec<String> = v.into_iter().map(|s| s.id).collect();
            v.sort();
            v
        };
        let src = |s: &Session| {
            s.meta
                .get("source")
                .and_then(|v| v.as_str())
                .map(String::from)
        };
        for with in [
            vec![],
            vec!["channel".to_string()],
            vec!["channel".into(), "swarm".into()],
        ] {
            for archived in [None, Some(false)] {
                let f = SessionListFilter {
                    archived,
                    foreground: Some(true),
                    with_sources: with.clone(),
                    ..Default::default()
                };
                let want = ids_of(
                    all.iter()
                        .filter(|s| archived.is_none_or(|a| s.archived == a))
                        .filter(|s| {
                            s.kind != SessionKind::Agent
                                || s.is_foreground_agent()
                                || src(s).is_some_and(|x| with.contains(&x))
                        })
                        .cloned()
                        .collect(),
                );
                let got = ids_of(repo.list_filtered(&scope, &f).await.unwrap());
                assert_eq!(got, want, "foreground=true {f:?}");
            }
        }
        let bg = SessionListFilter {
            foreground: Some(false),
            ..Default::default()
        };
        let want = ids_of(
            all.iter()
                .filter(|s| s.kind == SessionKind::Agent && !s.is_foreground_agent())
                .cloned()
                .collect(),
        );
        assert_eq!(
            want.len(),
            2 * otto_core::domain::BACKGROUND_SESSION_SOURCES.len()
        );
        assert_eq!(ids_of(repo.list_filtered(&scope, &bg).await.unwrap()), want);

        // visible_ids = non-archived foreground + channel rows.
        let mut vis = repo.visible_ids(&ws, None).await.unwrap();
        vis.sort();
        let want = ids_of(
            all.iter()
                .filter(|s| !s.archived)
                .filter(|s| {
                    s.kind != SessionKind::Agent
                        || s.is_foreground_agent()
                        || src(s).as_deref() == Some("channel")
                })
                .cloned()
                .collect(),
        );
        assert_eq!(vis, want);

        // ids: exact subset; empty → nothing.
        let pick: Vec<String> = all.iter().step_by(7).map(|s| s.id.clone()).collect();
        let f = SessionListFilter {
            ids: Some(pick.clone()),
            ..Default::default()
        };
        let mut pick_sorted = pick.clone();
        pick_sorted.sort();
        assert_eq!(
            ids_of(repo.list_filtered(&scope, &f).await.unwrap()),
            pick_sorted
        );
        let none = SessionListFilter {
            ids: Some(vec![]),
            ..Default::default()
        };
        assert!(repo.list_filtered(&scope, &none).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn title_candidates_are_the_sql_side_of_title_eligible() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());
        let ok = insert_session_full(&pool, &ws, &user, "claude", "idle", Some("p1"), 0).await;
        let codex = insert_session_full(&pool, &ws, &user, "codex", "working", Some("p2"), 0).await;
        let _archived =
            insert_session_full(&pool, &ws, &user, "claude", "idle", Some("p3"), 1).await;
        let _exited =
            insert_session_full(&pool, &ws, &user, "claude", "exited", Some("p4"), 0).await;
        let _shell = insert_session_full(&pool, &ws, &user, "shell", "idle", Some("p5"), 0).await;
        let _no_psid = insert_session_full(&pool, &ws, &user, "claude", "idle", None, 0).await;
        let named = insert_session_full(&pool, &ws, &user, "claude", "idle", Some("p6"), 0).await;
        repo.merge_meta(&named, &serde_json::json!({"title_source": "user"}))
            .await
            .unwrap();
        let auto = insert_session_full(&pool, &ws, &user, "claude", "idle", Some("p7"), 0).await;
        repo.merge_meta(&auto, &serde_json::json!({"title_source": "provider"}))
            .await
            .unwrap();
        let other = insert_session_full(&pool, &ws, &user, "claude", "idle", Some("p8"), 0).await;
        repo.merge_meta(&other, &serde_json::json!({"title_source": "first_prompt"}))
            .await
            .unwrap();
        let mut got: Vec<String> = repo
            .list_title_candidates()
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        got.sort();
        let mut want = vec![ok, codex, other];
        want.sort();
        assert_eq!(got, want);
    }

    #[tokio::test]
    async fn mark_dormant_except_keeps_last_active_and_skips_dormant_rows() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());
        let old = "2026-07-22T10:00:00+00:00";
        let set = |id: String, status: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query("UPDATE sessions SET status = ?, last_active_at = ? WHERE id = ?")
                    .bind(status)
                    .bind(old)
                    .bind(&id)
                    .execute(&pool)
                    .await
                    .unwrap();
                id
            }
        };
        let dormant = set(
            insert_session_full(&pool, &ws, &user, "claude", "idle", Some("p1"), 0).await,
            "reconnectable",
        )
        .await;
        let live = set(
            insert_session_full(&pool, &ws, &user, "claude", "idle", Some("p2"), 0).await,
            "running",
        )
        .await;
        let kept = set(
            insert_session_full(&pool, &ws, &user, "claude", "idle", Some("p3"), 0).await,
            "running",
        )
        .await;
        let exited = set(
            insert_session_full(&pool, &ws, &user, "claude", "idle", Some("p4"), 0).await,
            "exited",
        )
        .await;
        let exited_shell = set(
            insert_session_full(&pool, &ws, &user, "claude", "idle", None, 0).await,
            "exited",
        )
        .await;
        let archived = set(
            insert_session_full(&pool, &ws, &user, "claude", "idle", Some("p5"), 1).await,
            "running",
        )
        .await;

        let at = "2026-10-03T09:00:00+00:00";
        let pass = repo
            .mark_dormant_except(std::slice::from_ref(&kept), at)
            .await
            .unwrap();
        assert_eq!(pass.suspended.len(), 1);
        assert_eq!(pass.suspended[0].0, live);
        assert_eq!(pass.suspended[0].2["suspended"]["reason"], "restart");
        assert_eq!(pass.suspended[0].2["suspended"]["at"], at);
        assert_eq!(
            pass.resumable
                .iter()
                .map(|r| r.0.clone())
                .collect::<Vec<_>>(),
            vec![exited.clone()]
        );

        for (id, status) in [
            (&dormant, SessionStatus::Reconnectable),
            (&live, SessionStatus::Reconnectable),
            (&kept, SessionStatus::Running),
            (&exited, SessionStatus::Reconnectable),
            (&exited_shell, SessionStatus::Exited),
            (&archived, SessionStatus::Running),
        ] {
            let s = repo.get(id).await.unwrap();
            assert_eq!(s.status, status, "status of {id}");
            // A restart is not activity: the idle clock is untouched.
            assert_eq!(
                fmt(s.last_active_at),
                fmt(ts(old).unwrap()),
                "last_active_at of {id}"
            );
        }
        // The already-dormant row got no restart stamp.
        assert!(repo
            .get(&dormant)
            .await
            .unwrap()
            .meta
            .get("suspended")
            .is_none());
        // A second boot changes nothing.
        let again = repo.mark_dormant_except(&[], at).await.unwrap();
        assert!(again.suspended.len() == 1 && again.suspended[0].0 == kept);
        assert!(again.resumable.is_empty());
    }

    #[tokio::test]
    async fn update_status_keep_activity_leaves_last_active() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let repo = SessionsRepo::new(pool.clone());
        let old = "2026-07-22T10:00:00+00:00";
        let id = insert_session(&pool, &ws, &user, old, "{}", 0).await;
        repo.update_status_keep_activity(&id, SessionStatus::Running)
            .await
            .unwrap();
        let s = repo.get(&id).await.unwrap();
        assert_eq!(s.status, SessionStatus::Running);
        assert_eq!(fmt(s.last_active_at), fmt(ts(old).unwrap()));
    }

    /// Perf §15 F7: the targeted lookup matches on `meta.<key>` in SQL and
    /// skips archived and exited rows.
    #[tokio::test]
    async fn list_live_by_meta_filters_in_sql() {
        let pool = mem_pool().await;
        let (user, ws) = seed_user_ws(&pool).await;
        let now = fmt(Utc::now());
        let live = insert_session(&pool, &ws, &user, &now, r#"{"agent_id":"a1"}"#, 0).await;
        insert_session(&pool, &ws, &user, &now, r#"{"agent_id":"a1"}"#, 1).await;
        let exited = insert_session(&pool, &ws, &user, &now, r#"{"agent_id":"a1"}"#, 0).await;
        sqlx::query("UPDATE sessions SET status = 'exited' WHERE id = ?")
            .bind(&exited)
            .execute(&pool)
            .await
            .unwrap();
        insert_session(&pool, &ws, &user, &now, r#"{"agent_id":"a2"}"#, 0).await;
        let repo = SessionsRepo::new(pool.clone());
        let ws: Id = ws;
        let got = repo
            .list_live_by_meta(&ws, Some("agent"), "agent_id", "a1")
            .await
            .unwrap();
        assert_eq!(
            got.into_iter().map(|s| s.id).collect::<Vec<_>>(),
            vec![live]
        );
        assert!(repo
            .list_live_by_meta(&ws, Some("connection"), "agent_id", "a1")
            .await
            .unwrap()
            .is_empty());
    }
}
