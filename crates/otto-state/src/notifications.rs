//! Notifications repository: persisted notices + singleton settings.
//!
//! Notices mirror [`otto_core::domain::Notice`]. `create` de-dupes on
//! `source_key`: a live (non-dismissed) notice with the same key is refreshed in
//! place instead of inserting a duplicate. Settings are stored as a single
//! JSON-encoded row (`NotificationSettings`), defaulting when unset.

use chrono::Utc;
use otto_core::api::NotificationSettings;
use otto_core::domain::{Notice, NoticeAction, NoticeKind, NoticeSeverity};
use otto_core::{new_id, Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

/// Input for [`NotificationsRepo::create`]. `created_at`/`read`/`id` are owned
/// by the repository.
pub struct NewNotice {
    pub kind: NoticeKind,
    pub severity: NoticeSeverity,
    pub title: String,
    pub body: String,
    /// Stable key for de-duping recurring notices. None = always a new row.
    pub source_key: Option<String>,
    pub action: Option<NoticeAction>,
    /// Owning user. `None` = a global / system notice visible to everyone
    /// (the credential monitor, session-event hooks and skill-eval producers all
    /// emit these). `Some(id)` scopes the notice to a single user.
    pub user_id: Option<Id>,
}

/// Who is reading or mutating notices, used to scope queries to the caller.
///
/// Visibility: a [`NoticeAccess::User`] sees global notices (`user_id IS NULL`)
/// plus their own; [`NoticeAccess::All`] (root / system) sees everything.
///
/// Mutation: a [`NoticeAccess::User`] may only mark-read / dismiss / clear their
/// OWN notices — global / shared notices are read-only to them so one user can't
/// alter another's (or the system's) state. [`NoticeAccess::All`] may mutate any
/// row.
#[derive(Debug, Clone)]
pub enum NoticeAccess {
    /// Unrestricted (root user or daemon-internal callers).
    All,
    /// Scoped to a single user id.
    User(Id),
}

#[derive(Clone)]
pub struct NotificationsRepo {
    pool: SqlitePool,
}

// --- enum <-> column string helpers ----------------------------------------
// NoticeKind / NoticeSeverity have no inherent parse/as_str (they live in the
// read-only otto-core contract), so map them here. Strings match the migration
// CHECK constraints and the serde snake_case wire form.

fn kind_str(k: NoticeKind) -> &'static str {
    match k {
        NoticeKind::Credential => "credential",
        NoticeKind::Session => "session",
        NoticeKind::System => "system",
    }
}

fn kind_parse(s: &str) -> Result<NoticeKind> {
    match s {
        "credential" => Ok(NoticeKind::Credential),
        "session" => Ok(NoticeKind::Session),
        "system" => Ok(NoticeKind::System),
        other => Err(Error::Internal(format!("bad notice kind '{other}'"))),
    }
}

fn severity_str(s: NoticeSeverity) -> &'static str {
    match s {
        NoticeSeverity::Info => "info",
        NoticeSeverity::Warn => "warn",
        NoticeSeverity::Error => "error",
    }
}

fn severity_parse(s: &str) -> Result<NoticeSeverity> {
    match s {
        "info" => Ok(NoticeSeverity::Info),
        "warn" => Ok(NoticeSeverity::Warn),
        "error" => Ok(NoticeSeverity::Error),
        other => Err(Error::Internal(format!("bad notice severity '{other}'"))),
    }
}

fn row_to_notice(r: &sqlx::sqlite::SqliteRow) -> Result<Notice> {
    let action = match r.get::<Option<String>, _>("action_json") {
        Some(s) => Some(
            serde_json::from_str::<NoticeAction>(&s)
                .map_err(|e| Error::Internal(format!("bad notice action: {e}")))?,
        ),
        None => None,
    };
    Ok(Notice {
        id: r.get("id"),
        created_at: ts(&r.get::<String, _>("created_at"))?,
        read: r.get::<i64, _>("read") != 0,
        kind: kind_parse(&r.get::<String, _>("kind"))?,
        severity: severity_parse(&r.get::<String, _>("severity"))?,
        title: r.get("title"),
        body: r.get("body"),
        source_key: r.get("source_key"),
        action,
    })
}

impl NotificationsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create a notice, de-duping on `source_key`.
    ///
    /// When `source_key` is set and a live (non-dismissed) notice already
    /// carries the same key, that row is refreshed in place — body, severity and
    /// `created_at` are updated and `read` is reset to false — instead of
    /// inserting a duplicate. Otherwise a fresh row is inserted. Returns the
    /// resulting notice.
    pub async fn create(&self, n: NewNotice) -> Result<Notice> {
        let now = fmt(Utc::now());
        let action_json = match &n.action {
            Some(a) => Some(
                serde_json::to_string(a)
                    .map_err(|e| Error::Internal(format!("encode notice action: {e}")))?,
            ),
            None => None,
        };

        if let Some(key) = &n.source_key {
            // De-dupe within the same owner: a NULL `user_id` (global notice)
            // must match other global rows, so use NULL-safe equality (`IS`)
            // rather than `=`, which never matches NULL in SQLite.
            let existing = sqlx::query(
                "SELECT id FROM notifications WHERE source_key = ? AND user_id IS ?",
            )
            .bind(key)
            .bind(&n.user_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("notification dedupe"))?;
            if let Some(row) = existing {
                let id: Id = row.get("id");
                sqlx::query(
                    "UPDATE notifications
                        SET created_at = ?, read = 0, kind = ?, severity = ?,
                            title = ?, body = ?, action_json = ?
                      WHERE id = ?",
                )
                .bind(&now)
                .bind(kind_str(n.kind))
                .bind(severity_str(n.severity))
                .bind(&n.title)
                .bind(&n.body)
                .bind(&action_json)
                .bind(&id)
                .execute(&self.pool)
                .await
                .map_err(dberr("refresh notification"))?;
                return self.get(&id).await;
            }
        }

        let id = new_id();
        sqlx::query(
            "INSERT INTO notifications
                (id, created_at, read, kind, severity, title, body, source_key,
                 action_json, user_id)
             VALUES (?, ?, 0, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&now)
        .bind(kind_str(n.kind))
        .bind(severity_str(n.severity))
        .bind(&n.title)
        .bind(&n.body)
        .bind(&n.source_key)
        .bind(&action_json)
        .bind(&n.user_id)
        .execute(&self.pool)
        .await
        .map_err(dberr("create notification"))?;
        self.get(&id).await
    }

    pub async fn get(&self, id: &Id) -> Result<Notice> {
        let r = sqlx::query("SELECT * FROM notifications WHERE id = ?")
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .map_err(dberr("notification"))?;
        row_to_notice(&r)
    }

    /// Most recent notices first, capped at `limit`, scoped to what `access`
    /// may see: a [`NoticeAccess::User`] sees global (`user_id IS NULL`) notices
    /// plus their own; [`NoticeAccess::All`] sees everything.
    pub async fn list(&self, limit: i64, access: &NoticeAccess) -> Result<Vec<Notice>> {
        let rows = match access {
            NoticeAccess::All => {
                sqlx::query(
                    "SELECT * FROM notifications
                     ORDER BY created_at DESC, id DESC LIMIT ?",
                )
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            NoticeAccess::User(uid) => {
                sqlx::query(
                    "SELECT * FROM notifications
                     WHERE user_id IS NULL OR user_id = ?
                     ORDER BY created_at DESC, id DESC LIMIT ?",
                )
                .bind(uid)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
        }
        .map_err(dberr("notifications"))?;
        rows.iter().map(row_to_notice).collect()
    }

    /// Count of unread notices that drive the caller's badge.
    ///
    /// For a [`NoticeAccess::User`] this counts ONLY their own unread notices —
    /// global / system notices are intentionally excluded. A non-root user cannot
    /// mark a global notice read (the `read` flag is shared, see [`Self::mark_read`]),
    /// so counting global unread would leave a badge they can never clear. Global
    /// notices still appear in their [`Self::list`]; they just don't inflate the
    /// unread badge. [`NoticeAccess::All`] (root) counts every unread notice.
    pub async fn unread_count(&self, access: &NoticeAccess) -> Result<i64> {
        let r = match access {
            NoticeAccess::All => {
                sqlx::query("SELECT COUNT(*) AS n FROM notifications WHERE read = 0")
                    .fetch_one(&self.pool)
                    .await
            }
            NoticeAccess::User(uid) => sqlx::query(
                "SELECT COUNT(*) AS n FROM notifications
                 WHERE read = 0 AND user_id = ?",
            )
            .bind(uid)
            .fetch_one(&self.pool)
            .await,
        }
        .map_err(dberr("unread count"))?;
        Ok(r.get::<i64, _>("n"))
    }

    /// Mark a notice read. A [`NoticeAccess::User`] may only flip their OWN
    /// notices; global / shared notices are read-only to a non-root user (the
    /// `read` flag is shared, so a user must not alter it for others).
    pub async fn mark_read(&self, id: &Id, access: &NoticeAccess) -> Result<()> {
        match access {
            NoticeAccess::All => {
                sqlx::query("UPDATE notifications SET read = 1 WHERE id = ?")
                    .bind(id)
                    .execute(&self.pool)
                    .await
            }
            NoticeAccess::User(uid) => sqlx::query(
                "UPDATE notifications SET read = 1 WHERE id = ? AND user_id = ?",
            )
            .bind(id)
            .bind(uid)
            .execute(&self.pool)
            .await,
        }
        .map_err(dberr("mark notification read"))?;
        Ok(())
    }

    /// Mark every notice the caller owns as read. A [`NoticeAccess::User`] only
    /// touches their own rows; global notices are left untouched.
    pub async fn mark_all_read(&self, access: &NoticeAccess) -> Result<()> {
        match access {
            NoticeAccess::All => {
                sqlx::query("UPDATE notifications SET read = 1 WHERE read = 0")
                    .execute(&self.pool)
                    .await
            }
            NoticeAccess::User(uid) => sqlx::query(
                "UPDATE notifications SET read = 1 WHERE read = 0 AND user_id = ?",
            )
            .bind(uid)
            .execute(&self.pool)
            .await,
        }
        .map_err(dberr("mark all notifications read"))?;
        Ok(())
    }

    /// Permanently remove a single notice the caller owns. A
    /// [`NoticeAccess::User`] cannot dismiss global / shared notices.
    pub async fn dismiss(&self, id: &Id, access: &NoticeAccess) -> Result<()> {
        match access {
            NoticeAccess::All => {
                sqlx::query("DELETE FROM notifications WHERE id = ?")
                    .bind(id)
                    .execute(&self.pool)
                    .await
            }
            NoticeAccess::User(uid) => {
                sqlx::query("DELETE FROM notifications WHERE id = ? AND user_id = ?")
                    .bind(id)
                    .bind(uid)
                    .execute(&self.pool)
                    .await
            }
        }
        .map_err(dberr("dismiss notification"))?;
        Ok(())
    }

    /// Permanently remove the caller's notices. A [`NoticeAccess::User`] clears
    /// only their own rows; global notices remain. [`NoticeAccess::All`] wipes
    /// everything.
    pub async fn clear(&self, access: &NoticeAccess) -> Result<()> {
        match access {
            NoticeAccess::All => {
                sqlx::query("DELETE FROM notifications")
                    .execute(&self.pool)
                    .await
            }
            NoticeAccess::User(uid) => {
                sqlx::query("DELETE FROM notifications WHERE user_id = ?")
                    .bind(uid)
                    .execute(&self.pool)
                    .await
            }
        }
        .map_err(dberr("clear notifications"))?;
        Ok(())
    }

    /// Current settings, falling back to [`NotificationSettings::default`] when
    /// none have been persisted.
    pub async fn get_settings(&self) -> Result<NotificationSettings> {
        let row = sqlx::query("SELECT settings_json FROM notification_settings WHERE id = 1")
            .fetch_optional(&self.pool)
            .await
            .map_err(dberr("notification settings"))?;
        match row {
            Some(r) => serde_json::from_str(&r.get::<String, _>("settings_json"))
                .map_err(|e| Error::Internal(format!("bad notification settings: {e}"))),
            None => Ok(NotificationSettings::default()),
        }
    }

    pub async fn put_settings(&self, s: &NotificationSettings) -> Result<NotificationSettings> {
        let encoded = serde_json::to_string(s)
            .map_err(|e| Error::Internal(format!("encode notification settings: {e}")))?;
        sqlx::query(
            "INSERT INTO notification_settings (id, settings_json) VALUES (1, ?)
             ON CONFLICT (id) DO UPDATE SET settings_json = excluded.settings_json",
        )
        .bind(&encoded)
        .execute(&self.pool)
        .await
        .map_err(dberr("put notification settings"))?;
        self.get_settings().await
    }
}
