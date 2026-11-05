//! Slack Web API client for the agent-facing Slack tools (`otto.slack_*`):
//! list conversations, read channel / DM / thread history, search, post,
//! react and move the read cursor — as the workspace's **bot** (its saved
//! `xoxb-` token) or as **the person** (their own `xoxp-` user token, see
//! [`crate::slack_tools`]).
//!
//! Every call is a form-encoded POST with the token in the `Authorization`
//! header — never in a URL — so no request line, reqwest error or log ever
//! carries a token. Errors name the Slack method and code (and, for
//! `missing_scope`, the scope Slack says is needed) and nothing else.
//!
//! Normalization (messages, conversations, search hits, the read-cursor
//! arithmetic, `since` parsing) is pure and unit-tested; the HTTP half is
//! tested against a loopback mock Slack via [`SlackApi::with_base`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, TimeZone, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::slack::build_http_client;
use crate::slack_directory::DirUser;

/// The real Web API root.
pub const SLACK_API: &str = "https://slack.com/api";

/// The user-token scopes the as-you tools use (Slack app → OAuth &
/// Permissions → **User Token Scopes**). The `*:write` ones are what
/// `conversations.mark` (mark unread) requires; Otto never calls the other
/// methods they would allow (create / archive / invite).
pub const USER_SCOPES: &[&str] = &[
    "channels:history",
    "groups:history",
    "im:history",
    "mpim:history",
    "channels:read",
    "groups:read",
    "im:read",
    "mpim:read",
    "users:read",
    "search:read",
    "chat:write",
    "reactions:write",
    "channels:write",
    "groups:write",
    "im:write",
    "mpim:write",
];

/// Longest message text returned per message (chars); longer text is cut
/// with an ellipsis so one pasted log cannot blow the tool response budget.
const MAX_TEXT_CHARS: usize = 4000;
/// Longest text Otto will post (Slack itself truncates past 40k).
pub const MAX_POST_CHARS: usize = 40_000;
/// Pages walked by a full conversation listing / user directory.
const MAX_PAGES: usize = 5;
/// Member-directory cache lifetime (users.list is Tier 2: 20/min).
const NAMES_TTL: Duration = Duration::from_secs(300);
/// A rate-limited call is retried once when Slack asks for at most this.
const RETRY_AFTER_MAX: u64 = 3;

/// A failed Web API call: the method, Slack's error code, and (for
/// `missing_scope`) the scope Slack says it needs. Never carries a token.
#[derive(Debug, Clone, PartialEq)]
pub struct SlackError {
    pub method: String,
    pub code: String,
    pub needed: Option<String>,
}

impl std::fmt::Display for SlackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "slack {}: {}", self.method, self.code)?;
        if let Some(n) = &self.needed {
            write!(f, " (the token needs the `{n}` scope)")?;
        }
        Ok(())
    }
}

impl std::error::Error for SlackError {}

impl SlackError {
    fn new(method: &str, code: impl Into<String>) -> Self {
        Self {
            method: method.to_string(),
            code: code.into(),
            needed: None,
        }
    }

    /// The Otto error a route answers with: a missing channel / message /
    /// user is a 404, a bad argument a 400, everything else (auth, scopes,
    /// rate limits, transport) a 502 naming Slack's reason.
    pub fn into_otto(self) -> otto_core::Error {
        use otto_core::Error;
        let msg = self.to_string();
        match self.code.as_str() {
            "channel_not_found" | "not_in_channel" | "message_not_found" | "thread_not_found"
            | "user_not_found" | "users_not_found" => Error::NotFound(msg),
            "invalid_ts_latest" | "invalid_ts_oldest" | "invalid_timestamp" | "invalid_name"
            | "bad_timestamp" | "no_text" | "msg_too_long" | "invalid_cursor"
            | "already_reacted" | "too_many_reactions" | "is_archived" | "cant_dm_bot" => {
                Error::Invalid(msg)
            }
            _ => Error::Upstream(msg),
        }
    }
}

type SlackResult<T> = Result<T, SlackError>;

/// One Slack Web API identity (a bot or a user token) against one API root.
#[derive(Clone)]
pub struct SlackApi {
    token: String,
    base: String,
    http: reqwest::Client,
}

impl std::fmt::Debug for SlackApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the token.
        f.debug_struct("SlackApi")
            .field("base", &self.base)
            .finish()
    }
}

/// `auth.test`: who the token is, plus the scopes Slack granted it (the
/// `x-oauth-scopes` response header).
#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct Identity {
    pub user_id: String,
    pub user: String,
    pub team: String,
    pub team_id: String,
    pub url: String,
    /// Present for a bot token — a user token has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bot_id: Option<String>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Reaction {
    pub name: String,
    pub count: u64,
}

/// One message, text decoded from Slack's wire format with `<@U…>`
/// mentions resolved to names.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Msg {
    pub ts: String,
    /// `ts` as RFC 3339 (UTC).
    pub time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtype: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_count: Option<u64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reactions: Vec<Reaction>,
    /// Attached files' names (content is not fetched).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

/// A page of messages, oldest → newest.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct MsgPage {
    pub channel: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    pub messages: Vec<Msg>,
    /// Pass back as `cursor` for the next (older) page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

/// One conversation: a channel, a private channel, a DM or a group DM.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Conv {
    pub id: String,
    /// The channel name; for a DM the other person's name (or id).
    pub name: String,
    /// `channel` | `private` | `im` | `mpim`.
    pub kind: &'static str,
    /// A DM's other member.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_member: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_members: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ConvList {
    pub conversations: Vec<Conv>,
    /// The listing stopped at its page cap or at `limit`.
    pub truncated: bool,
}

/// One `search.messages` match.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Hit {
    pub channel_id: String,
    pub channel_name: String,
    pub kind: &'static str,
    pub ts: String,
    pub time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_name: Option<String>,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permalink: Option<String>,
    /// Sent by the token's own user (set by the activity digest).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub from_me: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SearchResult {
    pub query: String,
    pub total: u64,
    pub page: u64,
    pub pages: u64,
    pub matches: Vec<Hit>,
}

/// A posted message.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Posted {
    pub channel: String,
    pub ts: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permalink: Option<String>,
}

/// Member id → display name, for mention / author resolution.
#[derive(Debug, Clone, Default)]
pub struct Names(HashMap<String, String>);

impl Names {
    pub fn from_users(users: &[DirUser]) -> Self {
        Self(
            users
                .iter()
                .map(|u| (u.id.clone(), u.name.clone()))
                .collect(),
        )
    }
    pub fn get(&self, id: &str) -> Option<&str> {
        self.0.get(id).map(String::as_str)
    }
}

type NamesCache = Mutex<HashMap<String, (Instant, Arc<Vec<DirUser>>)>>;

fn names_cache() -> &'static NamesCache {
    static C: std::sync::OnceLock<NamesCache> = std::sync::OnceLock::new();
    C.get_or_init(Default::default)
}

impl SlackApi {
    pub fn new(token: impl Into<String>) -> Self {
        Self::with_base(token, SLACK_API)
    }

    /// Against another API root (tests: a loopback mock Slack).
    pub fn with_base(token: impl Into<String>, base: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            base: base.into().trim_end_matches('/').to_string(),
            http: build_http_client(),
        }
    }

    /// One Web API call: a form POST, the decoded body and the granted
    /// scopes header. A 429 asking for ≤ [`RETRY_AFTER_MAX`] s is retried
    /// once; otherwise it is `ratelimited (retry after Ns)`.
    async fn call_raw(
        &self,
        method: &str,
        params: &[(&str, String)],
    ) -> SlackResult<(Value, Option<String>)> {
        let url = format!("{}/{method}", self.base);
        for attempt in 0..2 {
            let resp = self
                .http
                .post(&url)
                .bearer_auth(&self.token)
                .form(params)
                .send()
                .await
                // reqwest's Display names the URL (no token in it) — keep
                // just the kind so nothing else leaks either.
                .map_err(|e| SlackError::new(method, transport_reason(&e)))?;
            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let secs = resp
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.trim().parse::<u64>().ok())
                    .unwrap_or(1)
                    .max(1);
                if attempt == 0 && secs <= RETRY_AFTER_MAX {
                    tokio::time::sleep(Duration::from_secs(secs)).await;
                    continue;
                }
                return Err(SlackError::new(
                    method,
                    format!("ratelimited (retry after {secs}s)"),
                ));
            }
            if !resp.status().is_success() {
                return Err(SlackError::new(
                    method,
                    format!("http {}", resp.status().as_u16()),
                ));
            }
            let scopes = resp
                .headers()
                .get("x-oauth-scopes")
                .and_then(|v| v.to_str().ok())
                .map(str::to_string);
            let val: Value = resp
                .json()
                .await
                .map_err(|_| SlackError::new(method, "invalid JSON response"))?;
            return decode_body(method, val).map(|v| (v, scopes));
        }
        unreachable!("the loop returns on its second attempt")
    }

    async fn call(&self, method: &str, params: &[(&str, String)]) -> SlackResult<Value> {
        self.call_raw(method, params).await.map(|(v, _)| v)
    }

    /// `auth.test` — the token's identity and granted scopes.
    pub async fn auth_test(&self) -> SlackResult<Identity> {
        let (v, scopes) = self.call_raw("auth.test", &[]).await?;
        let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
        Ok(Identity {
            user_id: s("user_id"),
            user: s("user"),
            team: s("team"),
            team_id: s("team_id"),
            url: s("url"),
            bot_id: v["bot_id"].as_str().map(str::to_string),
            scopes: scopes
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
        })
    }

    /// The workspace's members (cached 5 minutes per token); empty when the
    /// token lacks `users:read` — names then stay ids.
    pub async fn users(&self) -> Arc<Vec<DirUser>> {
        let key = format!("{}\n{}", self.base, self.token);
        let hit = names_cache()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&key)
            .filter(|(at, _)| at.elapsed() < NAMES_TTL)
            .map(|(_, u)| u.clone());
        if let Some(u) = hit {
            return u;
        }
        let mut out = Vec::new();
        let mut cursor = String::new();
        for _ in 0..MAX_PAGES {
            let mut p = vec![("limit", "200".to_string())];
            if !cursor.is_empty() {
                p.push(("cursor", cursor.clone()));
            }
            let Ok(v) = self.call("users.list", &p).await else {
                return Arc::new(out);
            };
            out.extend(crate::slack_directory::parse_users(&v));
            cursor = next_cursor(&v).unwrap_or_default();
            if cursor.is_empty() {
                break;
            }
        }
        let users = Arc::new(out);
        let mut c = names_cache().lock().unwrap_or_else(|p| p.into_inner());
        c.retain(|_, (at, _)| at.elapsed() < NAMES_TTL);
        c.insert(key, (Instant::now(), users.clone()));
        users
    }

    pub async fn names(&self) -> Names {
        Names::from_users(&self.users().await)
    }

    /// Conversations: as the bot `conversations.list` (what the bot can
    /// see), as a person `users.conversations` (what they are a member of —
    /// their channels, DMs and group DMs). `q` filters by name, `limit` caps
    /// the answer.
    pub async fn conversations(
        &self,
        as_user: bool,
        types: Option<&str>,
        q: Option<&str>,
        limit: usize,
    ) -> SlackResult<ConvList> {
        let (method, default_types) = if as_user {
            (
                "users.conversations",
                "public_channel,private_channel,mpim,im",
            )
        } else {
            ("conversations.list", "public_channel,private_channel")
        };
        let types = types
            .filter(|t| !t.trim().is_empty())
            .unwrap_or(default_types);
        let names = self.names().await;
        let needle = q.map(|q| q.trim().trim_start_matches(['#', '@']).to_lowercase());
        let mut out = Vec::new();
        let mut cursor = String::new();
        let mut truncated = true;
        for _ in 0..MAX_PAGES {
            let mut p = vec![
                ("types", types.to_string()),
                ("exclude_archived", "true".to_string()),
                ("limit", "200".to_string()),
            ];
            if !cursor.is_empty() {
                p.push(("cursor", cursor.clone()));
            }
            let v = self.call(method, &p).await?;
            for c in v["channels"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
                if let Some(conv) = parse_conversation(c, &names) {
                    let keep = needle.as_deref().is_none_or(|n| {
                        conv.name.to_lowercase().contains(n) || conv.id.to_lowercase() == n
                    });
                    if keep {
                        out.push(conv);
                    }
                }
            }
            cursor = next_cursor(&v).unwrap_or_default();
            if cursor.is_empty() {
                truncated = false;
                break;
            }
            if out.len() >= limit {
                break;
            }
        }
        if out.len() > limit {
            out.truncate(limit);
            truncated = true;
        }
        Ok(ConvList {
            conversations: out,
            truncated,
        })
    }

    /// A channel's history (`conversations.history`) or, with `thread_ts`, a
    /// thread (`conversations.replies`) — oldest → newest.
    pub async fn messages(&self, req: &HistoryReq) -> SlackResult<MsgPage> {
        let mut p = vec![
            ("channel", req.channel.clone()),
            ("limit", req.limit.to_string()),
        ];
        for (k, v) in [
            ("oldest", &req.oldest),
            ("latest", &req.latest),
            ("cursor", &req.cursor),
        ] {
            if let Some(v) = v {
                p.push((k, v.clone()));
            }
        }
        let method = match &req.thread_ts {
            Some(t) => {
                p.push(("ts", t.clone()));
                "conversations.replies"
            }
            None => "conversations.history",
        };
        let v = self.call(method, &p).await?;
        let names = self.names().await;
        let mut messages: Vec<Msg> = v["messages"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .map(|m| parse_message(m, &names))
            .collect();
        // History comes newest-first, replies oldest-first: one order out.
        messages.sort_by(|a, b| ts_key(&a.ts).total_cmp(&ts_key(&b.ts)));
        Ok(MsgPage {
            channel: req.channel.clone(),
            thread_ts: req.thread_ts.clone(),
            messages,
            next_cursor: next_cursor(&v),
            has_more: v["has_more"].as_bool().unwrap_or(false),
        })
    }

    /// `search.messages` (user tokens only — Slack offers bots no search).
    pub async fn search(
        &self,
        query: &str,
        count: u64,
        page: u64,
        sort: &str,
    ) -> SlackResult<SearchResult> {
        let p = [
            ("query", query.to_string()),
            ("count", count.clamp(1, 100).to_string()),
            ("page", page.max(1).to_string()),
            ("sort", sort.to_string()),
            ("sort_dir", "desc".to_string()),
            ("highlight", "false".to_string()),
        ];
        let v = self.call("search.messages", &p).await?;
        let names = self.names().await;
        let m = &v["messages"];
        Ok(SearchResult {
            query: query.to_string(),
            total: m["total"].as_u64().unwrap_or(0),
            page: m["paging"]["page"].as_u64().unwrap_or(1),
            pages: m["paging"]["pages"].as_u64().unwrap_or(0),
            matches: m["matches"]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .filter_map(|h| parse_hit(h, &names))
                .collect(),
        })
    }

    /// `chat.postMessage` (+ `chat.getPermalink`, best effort).
    pub async fn post_message(
        &self,
        channel: &str,
        text: &str,
        thread_ts: Option<&str>,
    ) -> SlackResult<Posted> {
        let mut p = vec![("channel", channel.to_string()), ("text", text.to_string())];
        if let Some(t) = thread_ts {
            p.push(("thread_ts", t.to_string()));
        }
        let v = self.call("chat.postMessage", &p).await?;
        let channel = v["channel"].as_str().unwrap_or(channel).to_string();
        let ts = v["ts"].as_str().unwrap_or("").to_string();
        let permalink = self
            .call(
                "chat.getPermalink",
                &[("channel", channel.clone()), ("message_ts", ts.clone())],
            )
            .await
            .ok()
            .and_then(|v| v["permalink"].as_str().map(str::to_string));
        Ok(Posted {
            channel,
            ts,
            thread_ts: thread_ts.map(str::to_string),
            permalink,
        })
    }

    /// `reactions.add` (`name` without colons).
    pub async fn react(&self, channel: &str, ts: &str, name: &str) -> SlackResult<()> {
        let name = name.trim().trim_matches(':');
        self.call(
            "reactions.add",
            &[
                ("channel", channel.to_string()),
                ("timestamp", ts.to_string()),
                ("name", name.to_string()),
            ],
        )
        .await
        .map(|_| ())
    }

    /// `conversations.mark` — move the token's user's read cursor to `ts`.
    pub async fn mark(&self, channel: &str, ts: &str) -> SlackResult<()> {
        self.call(
            "conversations.mark",
            &[("channel", channel.to_string()), ("ts", ts.to_string())],
        )
        .await
        .map(|_| ())
    }

    /// `conversations.open` — the DM channel with one member.
    pub async fn open_dm(&self, user: &str) -> SlackResult<String> {
        let v = self
            .call("conversations.open", &[("users", user.to_string())])
            .await?;
        v["channel"]["id"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| SlackError::new("conversations.open", "no channel in response"))
    }

    /// A channel / DM argument → a conversation id: an id passes through,
    /// `@handle` / a member id opens the DM, `#name` / a bare name is looked
    /// up among the conversations the identity sees.
    pub async fn resolve_channel(&self, as_user: bool, arg: &str) -> SlackResult<String> {
        let arg = arg.trim();
        match classify_target(arg) {
            Target::Conversation(id) => Ok(id.to_string()),
            Target::Member(id) => self.open_dm(id).await,
            Target::Handle(h) => {
                let users = self.users().await;
                let h = h.to_lowercase();
                let hit = users
                    .iter()
                    .find(|u| u.handle.to_lowercase() == h || u.name.to_lowercase() == h)
                    .ok_or_else(|| SlackError::new("users.list", "user_not_found"))?;
                self.open_dm(&hit.id).await
            }
            Target::Name(n) => {
                let list = self
                    .conversations(as_user, Some("public_channel,private_channel"), Some(n), 50)
                    .await?;
                let n = n.to_lowercase();
                list.conversations
                    .into_iter()
                    .find(|c| c.name.to_lowercase() == n)
                    .map(|c| c.id)
                    .ok_or_else(|| SlackError::new("conversations.list", "channel_not_found"))
            }
        }
    }
}

/// `conversations.history` / `.replies` arguments.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HistoryReq {
    pub channel: String,
    pub thread_ts: Option<String>,
    pub oldest: Option<String>,
    pub latest: Option<String>,
    pub cursor: Option<String>,
    pub limit: u64,
}

/// What a channel / DM argument names.
#[derive(Debug, PartialEq)]
pub(crate) enum Target<'a> {
    /// `C…` / `G…` / `D…` conversation id.
    Conversation(&'a str),
    /// `U…` / `W…` member id → their DM.
    Member(&'a str),
    /// `@handle` → their DM.
    Handle(&'a str),
    /// `#name` / `name` → a channel.
    Name(&'a str),
}

fn looks_like_id(s: &str, prefixes: &[char]) -> bool {
    s.len() >= 9
        && s.starts_with(prefixes)
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

pub(crate) fn classify_target(arg: &str) -> Target<'_> {
    if let Some(h) = arg.strip_prefix('@') {
        return if looks_like_id(h, &['U', 'W']) {
            Target::Member(h)
        } else {
            Target::Handle(h)
        };
    }
    if let Some(n) = arg.strip_prefix('#') {
        return Target::Name(n);
    }
    if looks_like_id(arg, &['C', 'G', 'D']) {
        Target::Conversation(arg)
    } else if looks_like_id(arg, &['U', 'W']) {
        Target::Member(arg)
    } else {
        Target::Name(arg)
    }
}

/// The body of a Web API answer, or the [`SlackError`] its `ok: false`
/// names (with `needed` for `missing_scope`).
fn decode_body(method: &str, val: Value) -> SlackResult<Value> {
    if val["ok"].as_bool().unwrap_or(false) {
        return Ok(val);
    }
    Err(SlackError {
        method: method.to_string(),
        code: val["error"].as_str().unwrap_or("unknown").to_string(),
        needed: val["needed"].as_str().map(str::to_string),
    })
}

fn transport_reason(e: &reqwest::Error) -> String {
    if e.is_timeout() {
        "timed out".into()
    } else if e.is_connect() {
        "could not connect to Slack".into()
    } else {
        "request failed".into()
    }
}

fn next_cursor(v: &Value) -> Option<String> {
    v["response_metadata"]["next_cursor"]
        .as_str()
        .filter(|c| !c.is_empty())
        .map(str::to_string)
}

/// A Slack `ts` (`"1730800000.123456"`) as a sortable number.
fn ts_key(ts: &str) -> f64 {
    ts.parse().unwrap_or(0.0)
}

/// `ts` as RFC 3339 UTC (empty for a malformed one).
pub fn ts_time(ts: &str) -> String {
    let (secs, micros) = split_ts(ts).unwrap_or((0, 0));
    Utc.timestamp_opt(secs, (micros * 1000) as u32)
        .single()
        .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        .unwrap_or_default()
}

fn split_ts(ts: &str) -> Option<(i64, u64)> {
    let (s, frac) = ts.trim().split_once('.').unwrap_or((ts.trim(), ""));
    let secs: i64 = s.parse().ok()?;
    if frac.len() > 6 || !frac.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let micros: u64 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<6}").parse().ok()?
    };
    Some((secs, micros))
}

/// Is `ts` a well-formed Slack timestamp (`secs[.micros]`)?
pub fn valid_ts(ts: &str) -> bool {
    !ts.trim().is_empty() && split_ts(ts).is_some()
}

/// The read cursor that leaves the message at `ts` (and everything after
/// it) **unread**: one microsecond before it — Slack timestamps are unique
/// per channel at microsecond precision, so nothing sits in between.
pub fn ts_just_before(ts: &str) -> Option<String> {
    let (secs, micros) = split_ts(ts)?;
    let (secs, micros) = if micros == 0 {
        (secs.checked_sub(1)?, 999_999)
    } else {
        (secs, micros - 1)
    };
    (secs >= 0).then(|| format!("{secs}.{micros:06}"))
}

/// A `since` / `oldest` / `latest` argument → a Slack `ts`: a ts or unix
/// seconds as-is, `30m` / `6h` / `2d` / `1w` back from `now`, an RFC 3339
/// instant, or a `YYYY-MM-DD` day (UTC midnight).
pub fn parse_when(s: &str, now: DateTime<Utc>) -> Result<String, String> {
    let s = s.trim();
    if valid_ts(s) {
        return Ok(s.to_string());
    }
    let bad = || {
        format!("can't read the time '{s}' — use a Slack ts, 30m / 6h / 2d / 1w, RFC 3339 or YYYY-MM-DD")
    };
    if let Some(unit) = s.chars().last().filter(|c| "mhdw".contains(*c)) {
        let n: i64 = s[..s.len() - 1].trim().parse().map_err(|_| bad())?;
        let mins = match unit {
            'm' => n,
            'h' => n * 60,
            'd' => n * 1440,
            _ => n * 10080,
        };
        let at = now - chrono::Duration::minutes(mins.clamp(0, 366 * 1440));
        return Ok(format!("{}.000000", at.timestamp()));
    }
    if let Ok(t) = DateTime::parse_from_rfc3339(s) {
        return Ok(format!("{}.000000", t.timestamp()));
    }
    if let Ok(d) = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d") {
        let t = d.and_hms_opt(0, 0, 0).ok_or_else(bad)?.and_utc();
        return Ok(format!("{}.000000", t.timestamp()));
    }
    Err(bad())
}

/// Slack's `after:` search modifier for messages at/after `since_ts`: the
/// day before (the modifier is exclusive and day-granular, in the searcher's
/// own timezone), so callers re-filter by `ts`.
pub fn search_after(since_ts: &str) -> String {
    let secs = split_ts(since_ts).map(|(s, _)| s).unwrap_or(0);
    let day = Utc
        .timestamp_opt(secs, 0)
        .single()
        .unwrap_or_default()
        .date_naive()
        - chrono::Duration::days(1);
    format!("after:{}", day.format("%Y-%m-%d"))
}

fn cap_text(s: String) -> String {
    if s.chars().count() <= MAX_TEXT_CHARS {
        return s;
    }
    let mut cut: String = s.chars().take(MAX_TEXT_CHARS).collect();
    cut.push('…');
    cut
}

/// Slack wire text → plain text: `<@U…>` mentions resolved to names first,
/// then the shared decoder (links, channels, entities).
pub fn render_text(text: &str, names: &Names) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("<@") {
        out.push_str(&rest[..i]);
        let after = &rest[i + 2..];
        match after.find('>') {
            Some(end) if !after[..end].contains('|') => {
                let id = &after[..end];
                match names.get(id) {
                    Some(n) => out.push_str(&format!("<@{id}|{n}>")),
                    None => out.push_str(&format!("<@{id}>")),
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str("<@");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    cap_text(crate::slack::decode_slack_text(&out))
}

fn opt_str(v: &Value) -> Option<String> {
    v.as_str().filter(|s| !s.is_empty()).map(str::to_string)
}

/// One `messages[]` entry → [`Msg`].
pub fn parse_message(m: &Value, names: &Names) -> Msg {
    let ts = m["ts"].as_str().unwrap_or("").to_string();
    let user = opt_str(&m["user"]);
    let user_name = user
        .as_deref()
        .and_then(|u| names.get(u))
        .map(str::to_string)
        .or_else(|| opt_str(&m["user_profile"]["display_name"]))
        .or_else(|| opt_str(&m["username"]))
        .or_else(|| opt_str(&m["bot_profile"]["name"]));
    let thread_ts = opt_str(&m["thread_ts"]);
    Msg {
        time: ts_time(&ts),
        thread_ts,
        user,
        user_name,
        text: render_text(m["text"].as_str().unwrap_or(""), names),
        subtype: opt_str(&m["subtype"]),
        reply_count: m["reply_count"].as_u64().filter(|n| *n > 0),
        reactions: m["reactions"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|r| {
                Some(Reaction {
                    name: r["name"].as_str()?.to_string(),
                    count: r["count"].as_u64().unwrap_or(0),
                })
            })
            .collect(),
        files: m["files"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|f| opt_str(&f["name"]).or_else(|| opt_str(&f["title"])))
            .collect(),
        ts,
    }
}

fn conv_kind(c: &Value) -> &'static str {
    if c["is_im"].as_bool().unwrap_or(false) {
        "im"
    } else if c["is_mpim"].as_bool().unwrap_or(false) {
        "mpim"
    } else if c["is_private"].as_bool().unwrap_or(false) || c["is_group"].as_bool().unwrap_or(false)
    {
        "private"
    } else {
        "channel"
    }
}

/// One `channels[]` entry → [`Conv`].
pub fn parse_conversation(c: &Value, names: &Names) -> Option<Conv> {
    let id = opt_str(&c["id"])?;
    let kind = conv_kind(c);
    let user = (kind == "im").then(|| opt_str(&c["user"])).flatten();
    let name = match (&user, opt_str(&c["name"])) {
        (Some(u), _) => names
            .get(u)
            .map(str::to_string)
            .unwrap_or_else(|| u.clone()),
        (None, Some(n)) => n,
        (None, None) => id.clone(),
    };
    Some(Conv {
        id,
        name,
        kind,
        user,
        is_member: c["is_member"].as_bool(),
        topic: opt_str(&c["topic"]["value"]),
        num_members: c["num_members"].as_u64(),
    })
}

/// One `search.messages` match → [`Hit`].
pub fn parse_hit(h: &Value, names: &Names) -> Option<Hit> {
    let ch = &h["channel"];
    let channel_id = opt_str(&ch["id"])?;
    let kind = conv_kind(ch);
    let user = opt_str(&h["user"]);
    let channel_name = match kind {
        // A DM's "name" is the other member's id.
        "im" => opt_str(&ch["name"])
            .map(|n| names.get(&n).map(str::to_string).unwrap_or(n))
            .unwrap_or_else(|| channel_id.clone()),
        _ => opt_str(&ch["name"]).unwrap_or_else(|| channel_id.clone()),
    };
    let ts = h["ts"].as_str().unwrap_or("").to_string();
    Some(Hit {
        channel_id,
        channel_name,
        kind,
        time: ts_time(&ts),
        user_name: user
            .as_deref()
            .and_then(|u| names.get(u))
            .map(str::to_string)
            .or_else(|| opt_str(&h["username"])),
        user,
        text: render_text(h["text"].as_str().unwrap_or(""), names),
        permalink: opt_str(&h["permalink"]),
        from_me: false,
        ts,
    })
}

#[cfg(test)]
#[path = "slack_api_tests.rs"]
pub(crate) mod tests;
