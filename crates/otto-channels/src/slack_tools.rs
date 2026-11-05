//! The Slack routes behind the agent tools (`otto.slack_*`) and the
//! "Act as me in Slack" settings. Paths are relative to `/api/v1`.
//!
//! **Tool routes** — `/workspaces/{id}/integrations/slack/…` — act either as
//! the workspace's **bot** (`as=bot`, the default: the integration's saved
//! `xoxb-` token) or as **the caller** (`as=user`: the caller's own `xoxp-`
//! token for the chosen Slack `team`, see [`crate::slack_user_tokens`]).
//! As-user calls also pass the host's [`AsMe`] gate: a person (or a session
//! they run directly) may use every team they connected; a personal agent
//! only the teams its owner allowed. Search, the activity digest and the
//! read cursor exist only as-user (Slack gives bots none of them).
//!
//! **Settings routes** — `/integrations/slack/me[/{team_id}[/test]]` — the
//! caller's own connected teams: list, add (paste a token), replace, test,
//! remove. A token is never returned.
//!
//! Outward-facing writes (post, react, mark) are reached by agents only
//! through approval-gated tools; the routes themselves are tagged Outward.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use otto_core::api::Problem;
use otto_core::auth::{AuthContext, AuthUser};
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ChannelsCtx;
use crate::slack_api::{self, HistoryReq, Hit, SlackApi};
use crate::slack_user_tokens::{self as store, AsMe, SlackMe, SlackMeView};

/// The write routes the Slack tools reach (the route-class `Outward` tag).
pub const OUTWARD_ROUTES: &[&str] = &[
    "/workspaces/{id}/integrations/slack/messages",
    "/workspaces/{id}/integrations/slack/reactions",
    "/workspaces/{id}/integrations/slack/mark",
];

pub(crate) struct ApiErr(Error);

impl From<Error> for ApiErr {
    fn from(e: Error) -> Self {
        Self(e)
    }
}

impl From<slack_api::SlackError> for ApiErr {
    fn from(e: slack_api::SlackError) -> Self {
        Self(e.into_otto())
    }
}

impl IntoResponse for ApiErr {
    fn into_response(self) -> Response {
        let status = match &self.0 {
            Error::NotFound(_) => StatusCode::NOT_FOUND,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Forbidden(_) => StatusCode::FORBIDDEN,
            Error::Conflict(_) => StatusCode::CONFLICT,
            Error::Invalid(_) => StatusCode::BAD_REQUEST,
            Error::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            Error::UnsupportedMedia(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            Error::Upstream(_) => StatusCode::BAD_GATEWAY,
            Error::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let body = Problem {
            code: self.0.code().to_string(),
            message: self.0.to_string(),
        };
        (status, Json(body)).into_response()
    }
}

type ApiResult<T> = Result<T, ApiErr>;

/// Mount the Slack routes onto the channels router.
pub(crate) fn routes<S: ChannelsCtx>(r: Router<S>) -> Router<S> {
    let r = r
        .route(
            "/integrations/slack/me",
            get(me_list::<S>).post(me_add::<S>),
        )
        .route(
            "/integrations/slack/me/{team_id}",
            put(me_replace::<S>).delete(me_remove::<S>),
        )
        .route("/integrations/slack/me/{team_id}/test", post(me_test::<S>))
        .route("/workspaces/{id}/integrations/slack/teams", get(teams::<S>))
        .route(
            "/workspaces/{id}/integrations/slack/conversations",
            get(conversations::<S>),
        )
        .route(
            "/workspaces/{id}/integrations/slack/messages",
            get(read_messages::<S>).post(post_message::<S>),
        )
        .route(
            "/workspaces/{id}/integrations/slack/users",
            get(find_users::<S>),
        )
        .route(
            "/workspaces/{id}/integrations/slack/search",
            get(search::<S>),
        )
        .route(
            "/workspaces/{id}/integrations/slack/activity",
            get(activity::<S>),
        )
        .route(
            "/workspaces/{id}/integrations/slack/reactions",
            post(react::<S>),
        )
        .route("/workspaces/{id}/integrations/slack/mark", post(mark::<S>));
    crate::slack_as_me::routes(r)
}

// ---------------------------------------------------------------------------
// Identity resolution
// ---------------------------------------------------------------------------

/// `as` = `bot` (default) | `user` (alias `me`).
fn as_user(v: Option<&str>) -> Result<bool, Error> {
    match v.map(str::trim).unwrap_or("") {
        "" | "bot" => Ok(false),
        "user" | "me" => Ok(true),
        other => Err(Error::Invalid(format!(
            "`as` must be bot or user (got '{other}')"
        ))),
    }
}

/// The Slack identity a call acts as: the workspace bot, or the caller's own
/// token for the picked team (gated by the host's [`AsMe`] verdict).
async fn identity<S: ChannelsCtx>(
    s: &S,
    ws_id: &Id,
    user: &AuthUser,
    auth: &AuthContext,
    as_user: bool,
    team: Option<&str>,
) -> Result<(SlackApi, Option<SlackMe>), Error> {
    let base = s.slack_api_base();
    if !as_user {
        let token = otto_core::secrets::get_async(s.secrets(), &format!("chan-bot-{ws_id}-slack"))
            .await?
            .filter(|t| !t.trim().is_empty())
            .ok_or_else(|| {
                Error::Invalid(
                    "this workspace has no Slack bot token — set up the Slack integration \
                     (Settings → Channels), or pass as=user to act as yourself"
                        .into(),
                )
            })?;
        return Ok((SlackApi::with_base(token, base), None));
    }
    store::refuse_impersonation(auth)?;
    let scope = s.slack_as_me(auth).await?;
    let teams = store::list(s.secrets(), &user.0.id).await?;
    let me = store::pick(&teams, team, &scope)?.clone();
    let token = store::token(s.secrets(), &user.0.id, &me.team_id).await?;
    Ok((SlackApi::with_base(token, base), Some(me)))
}

fn q<'a>(p: &'a HashMap<String, String>, k: &str) -> Option<&'a str> {
    p.get(k)
        .map(String::as_str)
        .filter(|v| !v.trim().is_empty())
}

fn num(p: &HashMap<String, String>, k: &str, default: u64, max: u64) -> u64 {
    q(p, k)
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
        .clamp(1, max)
}

fn when(v: Option<&str>) -> Result<Option<String>, Error> {
    v.map(|v| slack_api::parse_when(v, chrono::Utc::now()).map_err(Error::Invalid))
        .transpose()
}

fn team_json(me: &Option<SlackMe>) -> Value {
    match me {
        Some(m) => {
            json!({"as": "user", "team_id": m.team_id, "team": m.team, "user_id": m.user_id})
        }
        None => json!({"as": "bot"}),
    }
}

/// Merge `{as, team…}` into a serialized answer.
fn with_actor(mut body: Value, me: &Option<SlackMe>) -> Value {
    if let (Some(o), Value::Object(a)) = (body.as_object_mut(), team_json(me)) {
        o.extend(a);
    }
    body
}

fn to_value<T: Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

// ---------------------------------------------------------------------------
// Settings: the caller's own connected teams
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct TokenReq {
    pub token: String,
}

/// `GET /integrations/slack/me` — the caller's connected Slack workspaces.
async fn me_list<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
) -> ApiResult<Json<Vec<SlackMeView>>> {
    let teams = store::list(s.secrets(), &user.0.id).await?;
    Ok(Json(teams.into_iter().map(Into::into).collect()))
}

/// `POST /integrations/slack/me` — connect a team from a pasted token
/// (409 when that team is already connected: replace it instead).
async fn me_add<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Json(req): Json<TokenReq>,
) -> ApiResult<(StatusCode, Json<SlackMeView>)> {
    store::refuse_impersonation(&auth)?;
    let base = s.slack_api_base();
    let me = store::connect(s.secrets(), &base, &user.0.id, &req.token, false, None).await?;
    Ok((StatusCode::CREATED, Json(me.into())))
}

/// `PUT /integrations/slack/me/{team_id}` — swap a connected team's token.
async fn me_replace<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(team_id): Path<String>,
    Json(req): Json<TokenReq>,
) -> ApiResult<Json<SlackMeView>> {
    store::refuse_impersonation(&auth)?;
    let base = s.slack_api_base();
    let me = store::connect(
        s.secrets(),
        &base,
        &user.0.id,
        &req.token,
        true,
        Some(&team_id),
    )
    .await?;
    Ok(Json(me.into()))
}

/// `DELETE /integrations/slack/me/{team_id}`.
async fn me_remove<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(team_id): Path<String>,
) -> ApiResult<StatusCode> {
    store::refuse_impersonation(&auth)?;
    store::remove(s.secrets(), &user.0.id, &team_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /integrations/slack/me/{team_id}/test` — a live `auth.test` with the
/// stored token: refreshes the team / user / scopes it shows.
async fn me_test<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(team_id): Path<String>,
) -> ApiResult<Json<Value>> {
    store::refuse_impersonation(&auth)?;
    let base = s.slack_api_base();
    Ok(Json(
        match store::retest(s.secrets(), &base, &user.0.id, &team_id).await {
            Ok(me) => json!({"ok": true, "team": SlackMeView::from(me)}),
            Err(Error::NotFound(m)) => return Err(Error::NotFound(m).into()),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        },
    ))
}

// ---------------------------------------------------------------------------
// Tool routes
// ---------------------------------------------------------------------------

/// `GET /workspaces/{id}/integrations/slack/teams` — the Slack workspaces
/// the caller may act in as themselves (`otto.slack_my_workspaces`), plus
/// whether this workspace has a bot.
async fn teams<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws_id): Path<Id>,
) -> ApiResult<Json<Value>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Viewer)
        .await?;
    let scope = s.slack_as_me(&auth).await?;
    let teams: Vec<SlackMeView> = store::list(s.secrets(), &user.0.id)
        .await?
        .into_iter()
        .filter(|m| scope.allows(&m.team_id))
        .map(Into::into)
        .collect();
    let bot = otto_core::secrets::get_async(s.secrets(), &format!("chan-bot-{ws_id}-slack"))
        .await
        .ok()
        .flatten()
        .is_some_and(|t| !t.trim().is_empty());
    Ok(Json(json!({
        "teams": teams,
        "bot": bot,
        "as_me_restricted": matches!(scope, AsMe::Teams(_)),
    })))
}

/// `GET …/slack/conversations?as=&team=&types=&q=&limit=`.
async fn conversations<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws_id): Path<Id>,
    Query(p): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Viewer)
        .await?;
    let as_user = as_user(q(&p, "as"))?;
    let (api, me) = identity(&s, &ws_id, &user, &auth, as_user, q(&p, "team")).await?;
    let limit = num(&p, "limit", 200, 1000) as usize;
    let list = api
        .conversations(as_user, q(&p, "types"), q(&p, "q"), limit)
        .await?;
    Ok(Json(with_actor(to_value(&list), &me)))
}

/// `GET …/slack/messages?as=&team=&channel=&thread_ts=&oldest=&latest=&limit=&cursor=`.
async fn read_messages<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws_id): Path<Id>,
    Query(p): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Viewer)
        .await?;
    let as_user = as_user(q(&p, "as"))?;
    let channel = q(&p, "channel").ok_or_else(|| Error::Invalid("`channel` is required".into()))?;
    let thread_ts = q(&p, "thread_ts").map(str::to_string);
    if thread_ts
        .as_deref()
        .is_some_and(|t| !slack_api::valid_ts(t))
    {
        return Err(Error::Invalid("`thread_ts` is not a Slack timestamp".into()).into());
    }
    let (api, me) = identity(&s, &ws_id, &user, &auth, as_user, q(&p, "team")).await?;
    let req = HistoryReq {
        channel: api.resolve_channel(as_user, channel).await?,
        thread_ts,
        oldest: when(q(&p, "oldest"))?,
        latest: when(q(&p, "latest"))?,
        cursor: q(&p, "cursor").map(str::to_string),
        limit: num(&p, "limit", 50, 200),
    };
    let page = api.messages(&req).await?;
    Ok(Json(with_actor(to_value(&page), &me)))
}

/// `GET …/slack/users?as=&team=&q=` — find members by name / handle / id.
async fn find_users<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws_id): Path<Id>,
    Query(p): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Viewer)
        .await?;
    let (api, me) = identity(
        &s,
        &ws_id,
        &user,
        &auth,
        as_user(q(&p, "as"))?,
        q(&p, "team"),
    )
    .await?;
    let needle = q(&p, "q")
        .map(|v| v.trim().trim_start_matches('@').to_lowercase())
        .unwrap_or_default();
    let users = api.users().await;
    let hits: Vec<_> = users
        .iter()
        .filter(|u| {
            needle.is_empty()
                || u.id.to_lowercase() == needle
                || u.name.to_lowercase().contains(&needle)
                || u.handle.to_lowercase().contains(&needle)
        })
        .take(50)
        .cloned()
        .collect();
    Ok(Json(with_actor(json!({ "users": hits }), &me)))
}

/// `GET …/slack/search?team=&query=&count=&page=&sort=` — as the caller.
async fn search<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws_id): Path<Id>,
    Query(p): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Viewer)
        .await?;
    let query = q(&p, "query").ok_or_else(|| Error::Invalid("`query` is required".into()))?;
    let sort = match q(&p, "sort").unwrap_or("timestamp") {
        s @ ("timestamp" | "score") => s,
        other => {
            return Err(
                Error::Invalid(format!("`sort` is timestamp or score (got '{other}')")).into(),
            )
        }
    };
    let (api, me) = identity(&s, &ws_id, &user, &auth, true, q(&p, "team")).await?;
    let res = api
        .search(
            query,
            num(&p, "count", 20, 100),
            num(&p, "page", 1, 100),
            sort,
        )
        .await?;
    Ok(Json(with_actor(to_value(&res), &me)))
}

/// `GET …/slack/activity?team=&since=&limit=` — the caller's day digest:
/// messages that mention them, and their DM / group-DM traffic, since
/// `since` (default 24h).
async fn activity<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws_id): Path<Id>,
    Query(p): Query<HashMap<String, String>>,
) -> ApiResult<Json<Value>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Viewer)
        .await?;
    let since = when(Some(q(&p, "since").unwrap_or("24h")))?.unwrap_or_default();
    let limit = num(&p, "limit", 100, 300) as usize;
    let (api, me) = identity(&s, &ws_id, &user, &auth, true, q(&p, "team")).await?;
    let me_id = me.as_ref().map(|m| m.user_id.clone()).unwrap_or_default();
    let after = slack_api::search_after(&since);
    let mentions = api
        .search(&format!("<@{me_id}> {after}"), 100, 1, "timestamp")
        .await?;
    // Every message the caller can see since `since`, kept to DMs / group
    // DMs: Slack search has no "DMs only" modifier, so filter by kind (up to
    // three pages, newest first).
    let mut direct = Vec::new();
    let mut scanned_all = false;
    for page in 1..=3 {
        let res = api.search(&after, 100, page, "timestamp").await?;
        direct.extend(
            res.matches
                .into_iter()
                .filter(|h| matches!(h.kind, "im" | "mpim")),
        );
        if page >= res.pages {
            scanned_all = true;
            break;
        }
    }
    let digest = build_digest(&since, &me_id, mentions.matches, direct, limit);
    Ok(Json(with_actor(
        json!({
            "since": since,
            "since_time": slack_api::ts_time(&since),
            "mentions": digest.0,
            "direct": digest.1,
            "complete": scanned_all,
            "note": "Search-backed: Slack's index can lag a few minutes, and only \
                     conversations you are in are searched. Read a conversation in full \
                     with slack_read_messages.",
        }),
        &me,
    )))
}

/// Keep hits at/after `since`, flag the caller's own, newest first, capped.
fn build_digest(
    since: &str,
    me_id: &str,
    mentions: Vec<Hit>,
    direct: Vec<Hit>,
    limit: usize,
) -> (Vec<Hit>, Vec<Hit>) {
    let floor: f64 = since.parse().unwrap_or(0.0);
    let keep = |hits: Vec<Hit>| -> Vec<Hit> {
        let mut out: Vec<Hit> = hits
            .into_iter()
            .filter(|h| h.ts.parse::<f64>().unwrap_or(0.0) >= floor)
            .map(|mut h| {
                h.from_me = h.user.as_deref() == Some(me_id);
                h
            })
            .collect();
        out.sort_by(|a, b| b.ts.cmp(&a.ts));
        out.dedup_by(|a, b| a.ts == b.ts && a.channel_id == b.channel_id);
        out.truncate(limit);
        out
    };
    (keep(mentions), keep(direct))
}

#[derive(Deserialize)]
pub struct PostReq {
    #[serde(rename = "as", default)]
    pub as_: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
    pub channel: String,
    pub text: String,
    #[serde(default)]
    pub thread_ts: Option<String>,
}

/// `POST …/slack/messages` — post (or reply in a thread) as the bot or as
/// the caller.
async fn post_message<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws_id): Path<Id>,
    Json(req): Json<PostReq>,
) -> ApiResult<Json<Value>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Editor)
        .await?;
    if req.text.trim().is_empty() {
        return Err(Error::Invalid("`text` is empty".into()).into());
    }
    if req.text.chars().count() > slack_api::MAX_POST_CHARS {
        return Err(Error::Invalid(format!(
            "`text` is over {} characters",
            slack_api::MAX_POST_CHARS
        ))
        .into());
    }
    let thread = req.thread_ts.as_deref().filter(|t| !t.trim().is_empty());
    if thread.is_some_and(|t| !slack_api::valid_ts(t)) {
        return Err(Error::Invalid("`thread_ts` is not a Slack timestamp".into()).into());
    }
    let as_user = as_user(req.as_.as_deref())?;
    let (api, me) = identity(&s, &ws_id, &user, &auth, as_user, req.team.as_deref()).await?;
    let channel = api.resolve_channel(as_user, &req.channel).await?;
    let posted = api.post_message(&channel, &req.text, thread).await?;
    Ok(Json(with_actor(to_value(&posted), &me)))
}

#[derive(Deserialize)]
pub struct ReactReq {
    #[serde(rename = "as", default)]
    pub as_: Option<String>,
    #[serde(default)]
    pub team: Option<String>,
    pub channel: String,
    pub ts: String,
    pub name: String,
}

/// `POST …/slack/reactions` — add an emoji reaction.
async fn react<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws_id): Path<Id>,
    Json(req): Json<ReactReq>,
) -> ApiResult<Json<Value>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Editor)
        .await?;
    if !slack_api::valid_ts(&req.ts) {
        return Err(Error::Invalid("`ts` is not a Slack timestamp".into()).into());
    }
    if req.name.trim().trim_matches(':').is_empty() {
        return Err(Error::Invalid("`name` (the emoji) is empty".into()).into());
    }
    let as_user = as_user(req.as_.as_deref())?;
    let (api, me) = identity(&s, &ws_id, &user, &auth, as_user, req.team.as_deref()).await?;
    let channel = api.resolve_channel(as_user, &req.channel).await?;
    api.react(&channel, &req.ts, &req.name).await?;
    Ok(Json(with_actor(
        json!({"ok": true, "channel": channel, "ts": req.ts}),
        &me,
    )))
}

#[derive(Deserialize)]
pub struct MarkReq {
    #[serde(default)]
    pub team: Option<String>,
    pub channel: String,
    pub ts: String,
    /// `true` (default): leave `ts` and everything after it unread; `false`:
    /// mark read up to and including `ts`.
    #[serde(default)]
    pub unread: Option<bool>,
}

/// `POST …/slack/mark` — move the caller's read cursor (as-user only).
async fn mark<S: ChannelsCtx>(
    State(s): State<S>,
    Extension(user): Extension<AuthUser>,
    Extension(auth): Extension<AuthContext>,
    Path(ws_id): Path<Id>,
    Json(req): Json<MarkReq>,
) -> ApiResult<Json<Value>> {
    s.roles()
        .check(&user.0, &ws_id, WorkspaceRole::Editor)
        .await?;
    let unread = req.unread.unwrap_or(true);
    let cursor = if unread {
        slack_api::ts_just_before(&req.ts)
    } else {
        slack_api::valid_ts(&req.ts).then(|| req.ts.trim().to_string())
    }
    .ok_or_else(|| Error::Invalid("`ts` is not a Slack timestamp".into()))?;
    let (api, me) = identity(&s, &ws_id, &user, &auth, true, req.team.as_deref()).await?;
    let channel = api.resolve_channel(true, &req.channel).await?;
    api.mark(&channel, &cursor).await?;
    Ok(Json(with_actor(
        json!({"ok": true, "channel": channel, "ts": req.ts, "cursor": cursor, "unread": unread}),
        &me,
    )))
}

#[cfg(test)]
#[path = "slack_tools_tests.rs"]
mod tests;
