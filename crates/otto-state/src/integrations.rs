//! Workspace integrations repository (Slack / Telegram config, no tokens).

use chrono::Utc;
use otto_core::domain::{Channel, Integration};
use otto_core::{Error, Id, Result};
use sqlx::{Row, SqlitePool};

use crate::convert::{dberr, fmt, ts};

#[derive(Clone)]
pub struct IntegrationsRepo {
    pool: SqlitePool,
}

/// Internal row that also carries the keychain references (for delete / lookup).
struct IntegrationRow {
    workspace_id: Id,
    channel: Channel,
    enabled: bool,
    bot_token_ref: Option<String>,
    app_token_ref: Option<String>,
    allowed_users: String,
    agent_reply: bool,
    reply_instructions: String,
    channel_id: String,
    preferred_cli: String,
    updated_at: chrono::DateTime<Utc>,
}

fn parse_row(r: &sqlx::sqlite::SqliteRow) -> Result<IntegrationRow> {
    let channel_str: String = r.get("channel");
    let channel = Channel::parse(&channel_str)
        .ok_or_else(|| Error::Internal(format!("unknown channel '{channel_str}'")))?;
    Ok(IntegrationRow {
        workspace_id: r.get("workspace_id"),
        channel,
        enabled: r.get::<i64, _>("enabled") != 0,
        bot_token_ref: r.get("bot_token_ref"),
        app_token_ref: r.get("app_token_ref"),
        allowed_users: r.get("allowed_users"),
        agent_reply: r.get::<i64, _>("agent_reply") != 0,
        reply_instructions: r.get("reply_instructions"),
        channel_id: r.get("channel_id"),
        preferred_cli: r.get("preferred_cli"),
        updated_at: ts(&r.get::<String, _>("updated_at"))?,
    })
}

fn row_to_integration(row: &IntegrationRow) -> Integration {
    Integration {
        workspace_id: row.workspace_id.clone(),
        channel: row.channel,
        enabled: row.enabled,
        allowed_users: row.allowed_users.clone(),
        agent_reply: row.agent_reply,
        reply_instructions: row.reply_instructions.clone(),
        channel_id: row.channel_id.clone(),
        preferred_cli: row.preferred_cli.clone(),
        has_bot_token: row
            .bot_token_ref
            .as_deref()
            .map_or(false, |s| !s.is_empty()),
        has_app_token: row
            .app_token_ref
            .as_deref()
            .map_or(false, |s| !s.is_empty()),
        updated_at: row.updated_at,
    }
}

impl IntegrationsRepo {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// List all integrations for a workspace.
    pub async fn list(&self, workspace_id: &Id) -> Result<Vec<Integration>> {
        let rows = sqlx::query(
            "SELECT * FROM workspace_integrations WHERE workspace_id = ? ORDER BY channel",
        )
        .bind(workspace_id)
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("workspace integrations"))?;
        rows.iter()
            .map(|r| parse_row(r).map(|row| row_to_integration(&row)))
            .collect()
    }

    /// Get one integration row including the token refs (internal use).
    async fn get_row(&self, workspace_id: &Id, channel: Channel) -> Result<Option<IntegrationRow>> {
        let result = sqlx::query(
            "SELECT * FROM workspace_integrations WHERE workspace_id = ? AND channel = ?",
        )
        .bind(workspace_id)
        .bind(channel.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(dberr("workspace integration row"))?;
        result.map(|r| parse_row(&r)).transpose()
    }

    /// Get the public Integration struct (used after upsert to return fresh data).
    pub async fn get(&self, workspace_id: &Id, channel: Channel) -> Result<Option<Integration>> {
        self.get_row(workspace_id, channel)
            .await
            .map(|opt| opt.map(|row| row_to_integration(&row)))
    }

    /// Return `(bot_token_ref, app_token_ref)` for the given row, if it exists.
    pub async fn get_refs(
        &self,
        workspace_id: &Id,
        channel: Channel,
    ) -> Result<Option<(Option<String>, Option<String>)>> {
        Ok(self
            .get_row(workspace_id, channel)
            .await?
            .map(|row| (row.bot_token_ref, row.app_token_ref)))
    }

    /// Upsert an integration row.
    ///
    /// When `bot_token_ref` / `app_token_ref` is `None`, the existing ref is
    /// preserved (read-then-write). The caller is responsible for storing/deleting
    /// the actual secret in the keychain before calling this.
    pub async fn upsert(
        &self,
        workspace_id: &Id,
        channel: Channel,
        enabled: bool,
        bot_token_ref: Option<String>,
        app_token_ref: Option<String>,
        allowed_users: &str,
        agent_reply: bool,
        reply_instructions: &str,
        channel_id: &str,
        preferred_cli: &str,
    ) -> Result<()> {
        // If a ref arg is None, keep whatever is already stored.
        let (existing_bot, existing_app) = self
            .get_refs(workspace_id, channel)
            .await?
            .map(|(b, a)| (b, a))
            .unwrap_or((None, None));

        let bot_ref = bot_token_ref.or(existing_bot);
        let app_ref = app_token_ref.or(existing_app);

        let now = fmt(Utc::now());
        sqlx::query(
            "INSERT INTO workspace_integrations
             (workspace_id, channel, enabled, bot_token_ref, app_token_ref,
              allowed_users, agent_reply, reply_instructions, channel_id,
              preferred_cli, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(workspace_id, channel) DO UPDATE SET
               enabled            = excluded.enabled,
               bot_token_ref      = excluded.bot_token_ref,
               app_token_ref      = excluded.app_token_ref,
               allowed_users      = excluded.allowed_users,
               agent_reply        = excluded.agent_reply,
               reply_instructions = excluded.reply_instructions,
               channel_id         = excluded.channel_id,
               preferred_cli      = excluded.preferred_cli,
               updated_at         = excluded.updated_at",
        )
        .bind(workspace_id)
        .bind(channel.as_str())
        .bind(enabled as i64)
        .bind(&bot_ref)
        .bind(&app_ref)
        .bind(allowed_users)
        .bind(agent_reply as i64)
        .bind(reply_instructions)
        .bind(channel_id)
        .bind(preferred_cli)
        .bind(&now)
        .bind(&now)
        .execute(&self.pool)
        .await
        .map_err(dberr("upsert workspace integration"))?;
        Ok(())
    }

    /// List all enabled integrations across ALL workspaces (used by ChannelManager on startup).
    pub async fn list_all_enabled(&self) -> Result<Vec<Integration>> {
        let rows = sqlx::query(
            "SELECT * FROM workspace_integrations WHERE enabled = 1 ORDER BY workspace_id, channel",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(dberr("list all enabled integrations"))?;
        rows.iter()
            .map(|r| parse_row(r).map(|row| row_to_integration(&row)))
            .collect()
    }

    /// Delete a single integration row.
    pub async fn delete(&self, workspace_id: &Id, channel: Channel) -> Result<()> {
        sqlx::query("DELETE FROM workspace_integrations WHERE workspace_id = ? AND channel = ?")
            .bind(workspace_id)
            .bind(channel.as_str())
            .execute(&self.pool)
            .await
            .map_err(dberr("delete workspace integration"))?;
        Ok(())
    }
}
