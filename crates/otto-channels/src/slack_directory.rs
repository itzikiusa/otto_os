//! Slack directory lookups for the integration settings: the workspace's
//! members and channels, so the Slack card can show "Dana Levi (@dana)" next
//! to `U0123…` and pick people / the default channel by name instead of
//! pasting ids.
//!
//! Read-only Web API calls with the integration's bot token: `users.list`
//! (scope `users:read`) and `conversations.list` (`channels:read`, plus
//! `groups:read` for private channels). A missing scope is reported per part
//! instead of failing the whole lookup — the settings page then keeps showing
//! bare ids for that part and says which scope to add.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::Value;

use crate::slack::{api_json, build_http_client};

/// Pages (of 200) walked per list — 2,000 members / channels. A workspace
/// bigger than that gets `truncated` and the rest still shows as ids.
const MAX_PAGES: usize = 10;
const PAGE: &str = "200";
/// One lookup per bot per 5 minutes: the settings page re-opens often and
/// `users.list` is a Tier-2 (20/min) method.
const TTL: Duration = Duration::from_secs(300);

/// One Slack member.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DirUser {
    pub id: String,
    /// Display name, else real name, else the handle.
    pub name: String,
    /// The `@handle` (Slack's legacy `name`).
    pub handle: String,
    pub is_bot: bool,
}

/// One Slack channel the bot can see.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DirChannel {
    pub id: String,
    pub name: String,
    pub is_private: bool,
}

/// `GET /workspaces/{id}/integrations/slack/directory`.
#[derive(Debug, Clone, Default, Serialize)]
pub struct SlackDirectory {
    pub users: Vec<DirUser>,
    pub channels: Vec<DirChannel>,
    /// Why members couldn't be listed (e.g. `missing_scope` → add `users:read`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub users_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels_error: Option<String>,
    /// A list stopped at its page cap.
    pub truncated: bool,
}

type Cache = Mutex<HashMap<String, (Instant, SlackDirectory)>>;

fn cache() -> &'static Cache {
    static C: std::sync::OnceLock<Cache> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

/// The directory for `bot_token`, from the 5-minute cache unless `refresh`.
pub async fn lookup(bot_token: &str, refresh: bool) -> SlackDirectory {
    if !refresh {
        let hit = cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(bot_token)
            .filter(|(at, _)| at.elapsed() < TTL)
            .map(|(_, d)| d.clone());
        if let Some(d) = hit {
            return d;
        }
    }
    let dir = fetch(bot_token).await;
    // Only a complete answer is worth caching — a transient error (rate
    // limit, network) should not stick for five minutes.
    if dir.users_error.is_none() && dir.channels_error.is_none() {
        let mut c = cache().lock().unwrap_or_else(|p| p.into_inner());
        c.retain(|_, (at, _)| at.elapsed() < TTL);
        c.insert(bot_token.to_string(), (Instant::now(), dir.clone()));
    }
    dir
}

async fn fetch(bot_token: &str) -> SlackDirectory {
    let (users, channels) = tokio::join!(
        walk(bot_token, "users.list", &[], parse_users),
        channels(bot_token),
    );
    let mut dir = SlackDirectory::default();
    match users {
        Ok((u, more)) => {
            dir.users = u;
            dir.truncated |= more;
        }
        Err(e) => dir.users_error = Some(explain(&e, "users:read")),
    }
    match channels {
        Ok((c, more)) => {
            dir.channels = c;
            dir.truncated |= more;
        }
        Err(e) => dir.channels_error = Some(explain(&e, "channels:read")),
    }
    dir
}

/// Public + private channels; a bot without `groups:read` gets
/// `missing_scope` for the mixed query, so retry with public ones only.
async fn channels(bot_token: &str) -> anyhow::Result<(Vec<DirChannel>, bool)> {
    let all = [
        ("types", "public_channel,private_channel"),
        ("exclude_archived", "true"),
    ];
    match walk(bot_token, "conversations.list", &all, parse_channels).await {
        Err(e) if e.to_string().contains("missing_scope") => {
            let public = [("types", "public_channel"), ("exclude_archived", "true")];
            walk(bot_token, "conversations.list", &public, parse_channels).await
        }
        other => other,
    }
}

/// Walk a cursor-paginated list method; `(items, stopped_at_cap)`.
async fn walk<T>(
    bot_token: &str,
    method: &str,
    params: &[(&str, &str)],
    parse: fn(&Value) -> Vec<T>,
) -> anyhow::Result<(Vec<T>, bool)> {
    let http = build_http_client();
    let url = format!("https://slack.com/api/{method}");
    let mut out = Vec::new();
    let mut cursor = String::new();
    for _ in 0..MAX_PAGES {
        let mut req = http
            .get(&url)
            .header("Authorization", format!("Bearer {bot_token}"))
            .query(params)
            .query(&[("limit", PAGE)]);
        if !cursor.is_empty() {
            req = req.query(&[("cursor", cursor.as_str())]);
        }
        let val = api_json(req.send().await?, method).await?;
        out.extend(parse(&val));
        cursor = val["response_metadata"]["next_cursor"]
            .as_str()
            .unwrap_or("")
            .to_string();
        if cursor.is_empty() {
            return Ok((out, false));
        }
    }
    Ok((out, true))
}

/// Turn `slack users.list: missing_scope` into an actionable sentence.
fn explain(e: &anyhow::Error, scope: &str) -> String {
    let msg = e.to_string();
    if msg.contains("missing_scope") {
        format!("The Slack app needs the `{scope}` scope to show names (OAuth & Permissions → Bot Token Scopes, then reinstall).")
    } else {
        msg
    }
}

/// `users.list` members → [`DirUser`]s, skipping deactivated accounts and
/// Slackbot's placeholder entry.
fn parse_users(val: &Value) -> Vec<DirUser> {
    let s = |v: &Value| v.as_str().map(str::trim).unwrap_or("").to_string();
    val["members"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|m| !m["deleted"].as_bool().unwrap_or(false))
        .filter_map(|m| {
            let id = s(&m["id"]);
            if id.is_empty() || id == "USLACKBOT" {
                return None;
            }
            let handle = s(&m["name"]);
            let name = [
                &m["profile"]["display_name"],
                &m["profile"]["real_name"],
                &m["real_name"],
            ]
            .into_iter()
            .map(s)
            .find(|n| !n.is_empty())
            .unwrap_or_else(|| handle.clone());
            Some(DirUser {
                id,
                name,
                handle,
                is_bot: m["is_bot"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

/// `conversations.list` channels → [`DirChannel`]s.
fn parse_channels(val: &Value) -> Vec<DirChannel> {
    val["channels"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter_map(|c| {
            let id = c["id"].as_str()?.to_string();
            Some(DirChannel {
                id,
                name: c["name"].as_str().unwrap_or("").to_string(),
                is_private: c["is_private"].as_bool().unwrap_or(false),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn users_prefer_display_name_and_skip_deleted() {
        let v = json!({"members": [
            {"id": "U1", "name": "dana", "profile": {"display_name": "Dana", "real_name": "Dana Levi"}},
            {"id": "U2", "name": "avi", "profile": {"display_name": "", "real_name": "Avi Cohen"}},
            {"id": "U3", "name": "ghost", "deleted": true, "profile": {"real_name": "Gone"}},
            {"id": "B1", "name": "zendesk", "is_bot": true, "profile": {}},
            {"id": "USLACKBOT", "name": "slackbot", "profile": {"real_name": "Slackbot"}}
        ]});
        let u = parse_users(&v);
        let names: Vec<_> = u.iter().map(|u| (u.id.as_str(), u.name.as_str())).collect();
        assert_eq!(
            names,
            [("U1", "Dana"), ("U2", "Avi Cohen"), ("B1", "zendesk")]
        );
        assert!(u[2].is_bot && !u[0].is_bot);
        assert_eq!(u[0].handle, "dana");
    }

    #[test]
    fn channels_keep_privacy_flag() {
        let v = json!({"channels": [
            {"id": "C1", "name": "general", "is_private": false},
            {"id": "G1", "name": "ops", "is_private": true},
            {"name": "no-id"}
        ]});
        let c = parse_channels(&v);
        assert_eq!(c.len(), 2);
        assert!(c[1].is_private);
        assert_eq!(c[0].name, "general");
    }

    #[test]
    fn missing_scope_names_the_scope_to_add() {
        let e = anyhow::anyhow!("slack users.list: missing_scope");
        assert!(explain(&e, "users:read").contains("`users:read`"));
        let e = anyhow::anyhow!("slack users.list: ratelimited (retry after 3s)");
        assert_eq!(explain(&e, "users:read"), e.to_string());
    }
}
