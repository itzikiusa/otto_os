//! Seed workspace channel integrations from a `.loom.env` file.
//!
//! The file groups credentials per loom workspace under section headers:
//!   `# ===== Workspace: <Name>  (id=...) =====`
//! and KEY=VALUE lines like:
//!   `LOOM_<NAME>_SLACK_TOKEN`, `LOOM_<NAME>_SLACK_APP_TOKEN`,
//!   `LOOM_<NAME>_SLACK_ALLOWED_USERS`, `LOOM_<NAME>_TELEGRAM_TOKEN`,
//!   `LOOM_<NAME>_TELEGRAM_ALLOWED_USERS`.
//!
//! Match strategy: uppercase the Otto workspace name and look for the matching
//! `LOOM_<UPPERNAME>_...` keys; fall back to `DEFAULT` if nothing matches.

use otto_core::domain::{Channel, Integration};
use otto_core::{Error, Id, Result};

use crate::http::ChannelsCtx;

const DEFAULT_SLACK_REPLY_INSTRUCTIONS: &str =
    "When you finish, reply in this Slack thread with a concise summary and \
     attach a markdown file (investigation.md) with full details.";

const DEFAULT_TELEGRAM_REPLY_INSTRUCTIONS: &str =
    "When you finish, reply with a concise summary of the results.";

/// Parse KEY=VALUE lines from the file content.
/// Ignores blank lines and lines starting with `#`.
fn parse_env(content: &str) -> std::collections::HashMap<String, String> {
    let mut map = std::collections::HashMap::new();
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(pos) = trimmed.find('=') {
            let key = trimmed[..pos].trim().to_string();
            let value = trimmed[pos + 1..].trim().to_string();
            if !key.is_empty() {
                map.insert(key, value);
            }
        }
    }
    map
}

/// Locate the `.loom.env` file: `$OTTO_LOOM_ENV` env var first, then
/// `$HOME/claude_ade/.loom.env`. Returns `None` if not found.
fn find_loom_env() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("OTTO_LOOM_ENV") {
        let path = std::path::PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        let path = std::path::PathBuf::from(home)
            .join("claude_ade")
            .join(".loom.env");
        if path.exists() {
            return Some(path);
        }
    }
    None
}

/// Seed the given workspace's Slack/Telegram integrations from the loom env file.
/// Returns the list of integrations that were created/updated (may be empty).
pub async fn seed_from_loom<S: ChannelsCtx>(ctx: &S, ws_id: &Id) -> Result<Vec<Integration>> {
    // Find the file; silently return empty if missing.
    let path = match find_loom_env() {
        Some(p) => p,
        None => {
            tracing::debug!("seed_from_loom: no .loom.env file found, skipping");
            return Ok(vec![]);
        }
    };

    let content = std::fs::read_to_string(&path)
        .map_err(|e| Error::Internal(format!("read {}: {e}", path.display())))?;

    let env = parse_env(&content);

    // Determine the workspace name prefix to look up.
    let ws = ctx.workspaces().get(ws_id).await?;
    let upper_name = ws.name.to_uppercase();

    // Helper: get a key for this workspace or fall back to DEFAULT.
    let get_key = |suffix: &str| -> Option<String> {
        let primary = format!("LOOM_{upper_name}_{suffix}");
        if let Some(v) = env.get(&primary) {
            if !v.is_empty() {
                return Some(v.clone());
            }
        }
        let fallback = format!("LOOM_DEFAULT_{suffix}");
        env.get(&fallback).filter(|v| !v.is_empty()).cloned()
    };

    let mut results = Vec::new();

    // --- Slack ---
    if let Some(slack_token) = get_key("SLACK_TOKEN") {
        let bot_ref = format!("chan-bot-{}-slack", ws_id);
        ctx.secrets().put(&bot_ref, &slack_token)?;

        let app_ref_opt = if let Some(app_tok) = get_key("SLACK_APP_TOKEN") {
            let r = format!("chan-app-{}-slack", ws_id);
            ctx.secrets().put(&r, &app_tok)?;
            Some(r)
        } else {
            None
        };

        let allowed_users = get_key("SLACK_ALLOWED_USERS").unwrap_or_default();

        ctx.integrations()
            .upsert(
                ws_id,
                Channel::Slack,
                false, // enabled=false; operator enables manually
                Some(bot_ref),
                app_ref_opt,
                &allowed_users,
                true,
                DEFAULT_SLACK_REPLY_INSTRUCTIONS,
                "",
                "", // preferred_cli: use the default agent
            )
            .await?;

        if let Some(integration) = ctx.integrations().get(ws_id, Channel::Slack).await? {
            results.push(integration);
        }
    }

    // --- Telegram ---
    if let Some(tg_token) = get_key("TELEGRAM_TOKEN") {
        let bot_ref = format!("chan-bot-{}-telegram", ws_id);
        ctx.secrets().put(&bot_ref, &tg_token)?;

        let allowed_users = get_key("TELEGRAM_ALLOWED_USERS").unwrap_or_default();

        ctx.integrations()
            .upsert(
                ws_id,
                Channel::Telegram,
                false,
                Some(bot_ref),
                None,
                &allowed_users,
                true,
                DEFAULT_TELEGRAM_REPLY_INSTRUCTIONS,
                "",
                "", // preferred_cli: use the default agent
            )
            .await?;

        if let Some(integration) = ctx.integrations().get(ws_id, Channel::Telegram).await? {
            results.push(integration);
        }
    }

    Ok(results)
}
