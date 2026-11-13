//! Review comments on a design (docs/features/design-hall.md § Comments).
//!
//! A comment belongs to an artifact and to the VERSION it was made on, so a
//! note about v3 still reads "made on v3" after v7 lands. A top-level comment
//! may carry a PIN — a point on the rendered stage as fractions (0..1) of the
//! stage's content box, plus an optional frame/page id — and a resolved state;
//! replies are one level deep and inherit the thread's version. A comment never
//! changes the design, so mirrored (read-only) designs take comments too.
//!
//! Rows live in `design_comments` (the `design_comments` migration); a hard
//! delete of the artifact removes them (`Store::delete_artifact`), archiving
//! keeps them. Routes: [`http::routes`]. The design-assist brief reads the open
//! ones through [`brief_lines`] — reviewer notes, fenced as data.

pub mod http;

use chrono::{DateTime, Utc};
use otto_core::{new_id, Error, Id, Result};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::store::{stamp, Store};

/// Longest comment body, in characters.
pub const MAX_BODY_CHARS: usize = 4_000;
/// Longest pin frame/page id.
pub const MAX_FRAME_CHARS: usize = 128;
/// `GET …/comments` page size: the default and the cap.
pub const DEFAULT_LIST_LIMIT: i64 = 500;
pub const MAX_LIST_LIMIT: i64 = 1_000;
/// Comments (threads + replies) one artifact may hold.
pub const MAX_PER_ARTIFACT: i64 = 2_000;
/// Open threads the assist brief lists, and how much of each body.
pub const MAX_IN_BRIEF: usize = 12;
const BRIEF_BODY_CHARS: usize = 300;

/// Where on the stage a comment points: fractions of the rendered stage's
/// content box (so the pin survives a resize or a device-frame change).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CommentPin {
    pub x: f64,
    pub y: f64,
    /// A frame / page id for multi-frame formats.
    #[serde(default)]
    pub frame: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignComment {
    pub id: Id,
    pub artifact_id: Id,
    pub workspace_id: Id,
    /// The version the comment was made on (`None`: the design had none yet).
    pub version_id: Option<Id>,
    pub version_seq: Option<i64>,
    /// The thread a reply belongs to (`None`: a top-level comment).
    pub parent_id: Option<Id>,
    /// `user` | `agent`.
    pub author_kind: String,
    pub author_id: Id,
    pub author_name: String,
    pub session_id: Option<Id>,
    pub body: String,
    pub pin: Option<CommentPin>,
    pub resolved: bool,
    pub resolved_by: Option<Id>,
    pub resolved_by_name: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    /// Set when the body was edited after posting.
    pub edited_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A row to insert (validated by the caller with [`clean_body`] / [`clean_pin`]).
#[derive(Debug, Clone)]
pub struct NewComment {
    pub artifact_id: Id,
    pub workspace_id: Id,
    pub version_id: Option<Id>,
    pub version_seq: Option<i64>,
    pub parent_id: Option<Id>,
    pub author_kind: String,
    pub author_id: Id,
    pub author_name: String,
    pub session_id: Option<Id>,
    pub body: String,
    pub pin: Option<CommentPin>,
}

/// `GET …/comments` filters. `version_id` / `resolved` select THREADS; a
/// matching thread brings its replies along.
#[derive(Debug, Clone, Default)]
pub struct CommentFilter {
    pub version_id: Option<Id>,
    pub resolved: Option<bool>,
    pub limit: Option<i64>,
}

/// Trim and bound a body; control characters other than newline/tab go.
pub fn clean_body(raw: &str) -> Result<String> {
    let kept: String = raw
        .chars()
        .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
        .collect();
    let t = kept.trim().to_string();
    if t.is_empty() {
        return Err(Error::Invalid("a comment needs some text".into()));
    }
    let n = t.chars().count();
    if n > MAX_BODY_CHARS {
        return Err(Error::Invalid(format!(
            "a comment is at most {MAX_BODY_CHARS} characters (this one is {n})"
        )));
    }
    Ok(t)
}

/// A pin must sit on the stage: finite fractions in 0..=1, a short frame id.
pub fn clean_pin(pin: Option<CommentPin>) -> Result<Option<CommentPin>> {
    let Some(p) = pin else { return Ok(None) };
    let on_stage = |v: f64| v.is_finite() && (0.0..=1.0).contains(&v);
    if !on_stage(p.x) || !on_stage(p.y) {
        return Err(Error::Invalid(
            "a pin's x and y are fractions of the stage, between 0 and 1".into(),
        ));
    }
    let frame = p
        .frame
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty());
    if frame
        .as_ref()
        .is_some_and(|f| f.chars().count() > MAX_FRAME_CHARS)
    {
        return Err(Error::Invalid(format!(
            "a pin's frame id is at most {MAX_FRAME_CHARS} characters"
        )));
    }
    Ok(Some(CommentPin {
        x: p.x,
        y: p.y,
        frame,
    }))
}

fn dberr(op: &'static str) -> impl Fn(sqlx::Error) -> Error {
    move |e| Error::Internal(format!("{op}: {e}"))
}

fn ts(s: &str) -> Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .map_err(|e| Error::Internal(format!("bad timestamp '{s}': {e}")))
}

fn opt_ts(s: Option<String>) -> Result<Option<DateTime<Utc>>> {
    s.filter(|s| !s.is_empty()).map(|s| ts(&s)).transpose()
}

/// Every comment select: the row plus the CURRENT display names of its
/// author (people only — an agent keeps the name it wrote under) and of
/// whoever resolved it.
const SELECT: &str = "SELECT c.*, \
     CASE WHEN c.author_kind = 'user' THEN (SELECT COALESCE(NULLIF(u.display_name, ''), u.username) \
       FROM users u WHERE u.id = c.author_id) END AS author_now, \
     (SELECT COALESCE(NULLIF(u.display_name, ''), u.username) FROM users u \
       WHERE u.id = c.resolved_by) AS resolver_now \
     FROM design_comments c";

fn row_comment(r: &sqlx::sqlite::SqliteRow) -> Result<DesignComment> {
    let author_id: String = r.get("author_id");
    let stored: String = r.get("author_name");
    let author_name = r
        .try_get::<Option<String>, _>("author_now")
        .ok()
        .flatten()
        .filter(|n| !n.is_empty())
        .or_else(|| (!stored.is_empty()).then_some(stored))
        .unwrap_or_else(|| author_id.clone());
    let pin = match (
        r.get::<Option<f64>, _>("pin_x"),
        r.get::<Option<f64>, _>("pin_y"),
    ) {
        (Some(x), Some(y)) => Some(CommentPin {
            x,
            y,
            frame: r.get("pin_frame"),
        }),
        _ => None,
    };
    let resolved_by_name = r
        .try_get::<Option<String>, _>("resolver_now")
        .ok()
        .flatten()
        .or_else(|| r.get("resolved_by_name"));
    Ok(DesignComment {
        id: r.get("id"),
        artifact_id: r.get("artifact_id"),
        workspace_id: r.get("workspace_id"),
        version_id: r.get("version_id"),
        version_seq: r.get("version_seq"),
        parent_id: r.get("parent_id"),
        author_kind: r.get("author_kind"),
        author_id,
        author_name,
        session_id: r.get("session_id"),
        body: r.get("body"),
        pin,
        resolved: r.get::<i64, _>("resolved") != 0,
        resolved_by: r.get("resolved_by"),
        resolved_by_name,
        resolved_at: opt_ts(r.get("resolved_at"))?,
        edited_at: opt_ts(r.get("edited_at"))?,
        created_at: ts(&r.get::<String, _>("created_at"))?,
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

impl Store {
    pub async fn insert_comment(&self, c: NewComment) -> Result<DesignComment> {
        let n: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM design_comments WHERE artifact_id = ?")
                .bind(&c.artifact_id)
                .fetch_one(self.pool())
                .await
                .map_err(dberr("design.comment.count"))?;
        if n >= MAX_PER_ARTIFACT {
            return Err(Error::Conflict(format!(
                "this design already holds {MAX_PER_ARTIFACT} comments; resolve and delete some first"
            )));
        }
        let id = new_id();
        let now = stamp(Utc::now());
        let (px, py, pf) = match &c.pin {
            Some(p) => (Some(p.x), Some(p.y), p.frame.clone()),
            None => (None, None, None),
        };
        sqlx::query(
            "INSERT INTO design_comments
             (id, artifact_id, workspace_id, version_id, version_seq, parent_id, author_kind,
              author_id, author_name, session_id, body, pin_x, pin_y, pin_frame, resolved,
              created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?, ?)",
        )
        .bind(&id)
        .bind(&c.artifact_id)
        .bind(&c.workspace_id)
        .bind(&c.version_id)
        .bind(c.version_seq)
        .bind(&c.parent_id)
        .bind(&c.author_kind)
        .bind(&c.author_id)
        .bind(&c.author_name)
        .bind(&c.session_id)
        .bind(&c.body)
        .bind(px)
        .bind(py)
        .bind(&pf)
        .bind(&now)
        .bind(&now)
        .execute(self.pool())
        .await
        .map_err(dberr("design.comment.insert"))?;
        self.require_comment(&id).await
    }

    pub async fn get_comment(&self, id: &str) -> Result<Option<DesignComment>> {
        let row = sqlx::query(sqlx::AssertSqlSafe(format!("{SELECT} WHERE c.id = ?")))
            .bind(id)
            .fetch_optional(self.pool())
            .await
            .map_err(dberr("design.comment.get"))?;
        row.as_ref().map(row_comment).transpose()
    }

    pub async fn require_comment(&self, id: &str) -> Result<DesignComment> {
        self.get_comment(id)
            .await?
            .ok_or_else(|| Error::NotFound(format!("design comment {id}")))
    }

    /// Comments of one artifact, oldest first. The page keeps the NEWEST
    /// `limit` rows (a long-lived design shows its recent discussion).
    pub async fn list_comments(
        &self,
        artifact_id: &str,
        f: &CommentFilter,
    ) -> Result<Vec<DesignComment>> {
        let limit = f
            .limit
            .unwrap_or(DEFAULT_LIST_LIMIT)
            .clamp(1, MAX_LIST_LIMIT);
        // Replies carry their thread's version; the resolved state is the
        // thread's (a reply's own flag is never set).
        let resolved = f.resolved.map(i64::from);
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECT} WHERE c.artifact_id = ?
               AND (? IS NULL OR c.version_id = ?)
               AND (? IS NULL OR (CASE WHEN c.parent_id IS NULL THEN c.resolved
                    ELSE (SELECT p.resolved FROM design_comments p WHERE p.id = c.parent_id) END) = ?)
             ORDER BY c.created_at DESC, c.id DESC LIMIT ?"
        )))
        .bind(artifact_id)
        .bind(&f.version_id)
        .bind(&f.version_id)
        .bind(resolved)
        .bind(resolved)
        .bind(limit)
        .fetch_all(self.pool())
        .await
        .map_err(dberr("design.comment.list"))?;
        let mut out = rows.iter().map(row_comment).collect::<Result<Vec<_>>>()?;
        out.reverse();
        Ok(out)
    }

    pub async fn update_comment_body(&self, id: &str, body: &str) -> Result<DesignComment> {
        let now = stamp(Utc::now());
        let res = sqlx::query(
            "UPDATE design_comments SET body = ?, edited_at = ?, updated_at = ? WHERE id = ?",
        )
        .bind(body)
        .bind(&now)
        .bind(&now)
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(dberr("design.comment.edit"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("design comment {id}")));
        }
        self.require_comment(id).await
    }

    /// Resolve (`by` = the person) or reopen (`None`) a top-level comment.
    pub async fn set_comment_resolved(
        &self,
        id: &str,
        by: Option<(&str, &str)>,
    ) -> Result<DesignComment> {
        let now = stamp(Utc::now());
        let (flag, who, name, at) = match by {
            Some((uid, uname)) => (1i64, Some(uid), Some(uname), Some(now.as_str())),
            None => (0, None, None, None),
        };
        let res = sqlx::query(
            "UPDATE design_comments SET resolved = ?, resolved_by = ?, resolved_by_name = ?,
               resolved_at = ?, updated_at = ? WHERE id = ? AND parent_id IS NULL",
        )
        .bind(flag)
        .bind(who)
        .bind(name)
        .bind(at)
        .bind(&now)
        .bind(id)
        .execute(self.pool())
        .await
        .map_err(dberr("design.comment.resolve"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("design comment thread {id}")));
        }
        self.require_comment(id).await
    }

    /// Delete a comment; a thread takes its replies with it. Returns the
    /// number of rows removed.
    pub async fn delete_comment(&self, id: &str) -> Result<u64> {
        let res = sqlx::query("DELETE FROM design_comments WHERE id = ? OR parent_id = ?")
            .bind(id)
            .bind(id)
            .execute(self.pool())
            .await
            .map_err(dberr("design.comment.delete"))?;
        if res.rows_affected() == 0 {
            return Err(Error::NotFound(format!("design comment {id}")));
        }
        Ok(res.rows_affected())
    }

    /// The newest open threads (top-level, unresolved), newest first.
    pub async fn open_threads(
        &self,
        artifact_id: &str,
        limit: usize,
    ) -> Result<Vec<DesignComment>> {
        let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
            "{SELECT} WHERE c.artifact_id = ? AND c.parent_id IS NULL AND c.resolved = 0
             ORDER BY c.created_at DESC, c.id DESC LIMIT ?"
        )))
        .bind(artifact_id)
        .bind(limit as i64)
        .fetch_all(self.pool())
        .await
        .map_err(dberr("design.comment.open"))?;
        rows.iter().map(row_comment).collect()
    }
}

/// A pin in words for an agent: "pinned at 42% across, 18% down (frame f2)".
pub fn pin_words(p: &CommentPin) -> String {
    let pct = |v: f64| (v * 100.0).round() as i64;
    let frame = p
        .frame
        .as_deref()
        .map(|f| format!(" of frame {f}"))
        .unwrap_or_default();
    format!("pinned at {}% across, {}% down{frame}", pct(p.x), pct(p.y))
}

/// One brief line per open thread, labelled `[C1]`… oldest first: version,
/// pin, author and the (single-line, capped) body. The caller fences them as
/// reviewer notes — data, never instructions.
pub fn brief_lines(open: &[DesignComment]) -> Vec<String> {
    let mut threads: Vec<&DesignComment> = open
        .iter()
        .filter(|c| c.parent_id.is_none() && !c.resolved)
        .take(MAX_IN_BRIEF)
        .collect();
    threads.sort_by_key(|c| c.created_at);
    threads
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let on = c
                .version_seq
                .map(|s| format!("on v{s}"))
                .unwrap_or_else(|| "on the draft".into());
            let pin = c
                .pin
                .as_ref()
                .map(pin_words)
                .unwrap_or_else(|| "no pin (the whole design)".into());
            let body: String = c
                .body
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .chars()
                .take(BRIEF_BODY_CHARS)
                .collect();
            format!(
                "[C{}] {on} · {pin} · {}: {body}",
                i + 1,
                c.author_name.replace(['\n', '\r'], " ")
            )
        })
        .collect()
}

/// The open threads of `artifact_id` as brief lines (empty on any error —
/// comments enrich the brief, they never block a turn).
pub async fn brief_for(store: &Store, artifact_id: &str) -> Vec<String> {
    match store.open_threads(artifact_id, MAX_IN_BRIEF).await {
        Ok(open) => brief_lines(&open),
        Err(e) => {
            tracing::warn!(artifact = %artifact_id, error = %e, "design: open comments for the brief");
            Vec::new()
        }
    }
}

#[cfg(test)]
mod tests;
