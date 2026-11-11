//! A person's default Slack team for "act as me" sends: a global default
//! (stored with `workspace_id = ''`) and per-workspace overrides; a
//! workspace's own choice wins over the global one. Only the team id lives
//! here — the tokens stay in the Keychain (otto-channels' `slack_user_tokens`).

use crate::convert::{dberr, fmt};
use crate::DbPool;
use chrono::Utc;
use otto_core::Result;
use sqlx::Row;

/// The stored row key for the global default.
const GLOBAL: &str = "";

#[derive(Clone)]
pub struct SlackAsMeDefaultsRepo {
    pool: DbPool,
}

/// A person's default team at both levels, and which one applies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlackAsMeDefaults {
    pub global: Option<String>,
    pub workspace: Option<String>,
}

impl SlackAsMeDefaults {
    /// The team a send without an explicit team uses: the workspace's
    /// override, else the global default.
    pub fn effective(&self) -> Option<&str> {
        self.workspace.as_deref().or(self.global.as_deref())
    }
}

impl SlackAsMeDefaultsRepo {
    pub fn new(pool: impl Into<DbPool>) -> Self {
        Self { pool: pool.into() }
    }

    /// Both levels for `user` (the workspace level only when `ws` is given).
    pub async fn get(&self, user: &str, ws: Option<&str>) -> Result<SlackAsMeDefaults> {
        let rows = sqlx::query(
            "SELECT workspace_id, team_id FROM slack_as_me_defaults
             WHERE user_id = ? AND (workspace_id = '' OR workspace_id = ?)",
        )
        .bind(user)
        .bind(ws.unwrap_or(GLOBAL))
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("slack as-me defaults"))?;
        let mut out = SlackAsMeDefaults::default();
        for r in rows {
            let w: String = r.get("workspace_id");
            let t: String = r.get("team_id");
            if w.is_empty() {
                out.global = Some(t);
            } else {
                out.workspace = Some(t);
            }
        }
        Ok(out)
    }

    /// Set (`Some`) or clear (`None`) the global default (`ws = None`) or a
    /// workspace override.
    pub async fn set(&self, user: &str, ws: Option<&str>, team: Option<&str>) -> Result<()> {
        let w = ws.unwrap_or(GLOBAL);
        match team.map(str::trim).filter(|t| !t.is_empty()) {
            Some(t) => sqlx::query(
                "INSERT INTO slack_as_me_defaults (user_id, workspace_id, team_id, updated_at)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT(user_id, workspace_id) DO UPDATE
                 SET team_id = excluded.team_id, updated_at = excluded.updated_at",
            )
            .bind(user)
            .bind(w)
            .bind(t)
            .bind(fmt(Utc::now()))
            .execute(&self.pool)
            .await
            .map_err(dberr("set slack as-me default"))?,
            None => sqlx::query(
                "DELETE FROM slack_as_me_defaults WHERE user_id = ? AND workspace_id = ?",
            )
            .bind(user)
            .bind(w)
            .execute(&self.pool)
            .await
            .map_err(dberr("clear slack as-me default"))?,
        };
        Ok(())
    }

    /// Drop every default pointing at `team` (the team was disconnected).
    pub async fn forget_team(&self, user: &str, team: &str) -> Result<()> {
        sqlx::query("DELETE FROM slack_as_me_defaults WHERE user_id = ? AND team_id = ?")
            .bind(user)
            .bind(team)
            .execute(&self.pool)
            .await
            .map_err(dberr("forget slack as-me team"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repo() -> SlackAsMeDefaultsRepo {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!().run(&pool).await.unwrap();
        SlackAsMeDefaultsRepo::new(pool)
    }

    #[tokio::test]
    async fn workspace_override_wins_over_global_and_clears_back_to_it() {
        let r = repo().await;
        assert_eq!(r.get("u1", Some("ws1")).await.unwrap().effective(), None);
        r.set("u1", None, Some("T_GLOBAL")).await.unwrap();
        assert_eq!(
            r.get("u1", Some("ws1")).await.unwrap().effective(),
            Some("T_GLOBAL")
        );
        r.set("u1", Some("ws1"), Some("T_WS")).await.unwrap();
        let d = r.get("u1", Some("ws1")).await.unwrap();
        assert_eq!(
            (d.global.as_deref(), d.workspace.as_deref(), d.effective()),
            (Some("T_GLOBAL"), Some("T_WS"), Some("T_WS"))
        );
        // Another workspace and another person are untouched.
        assert_eq!(
            r.get("u1", Some("ws2")).await.unwrap().effective(),
            Some("T_GLOBAL")
        );
        assert_eq!(r.get("u2", Some("ws1")).await.unwrap().effective(), None);
        // Clearing the override falls back to the global default.
        r.set("u1", Some("ws1"), None).await.unwrap();
        assert_eq!(
            r.get("u1", Some("ws1")).await.unwrap().effective(),
            Some("T_GLOBAL")
        );
    }

    #[tokio::test]
    async fn disconnecting_a_team_drops_defaults_that_point_at_it() {
        let r = repo().await;
        r.set("u1", None, Some("T1")).await.unwrap();
        r.set("u1", Some("ws1"), Some("T1")).await.unwrap();
        r.set("u1", Some("ws2"), Some("T2")).await.unwrap();
        r.forget_team("u1", "T1").await.unwrap();
        assert_eq!(r.get("u1", Some("ws1")).await.unwrap().effective(), None);
        assert_eq!(
            r.get("u1", Some("ws2")).await.unwrap().effective(),
            Some("T2")
        );
    }
}
