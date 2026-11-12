//! An agent's **own scope**, managed from its chat with no approval card —
//! except a routine that posts OUTSIDE the chat (Slack), which the owner
//! approves first ([`outward_delivery_reason`]):
//!
//! * **Routines** (its schedules): "every weekday at 9 Israel time, send me a
//!   Slack message with …" — create / list / edit / pause / delete. A routine
//!   delivers into the agent's chat (always — the feed posts every result)
//!   and, optionally, to Slack (as the Otto bot via the routine's own
//!   destination, or as the user when the agent may act as them in Slack).
//!   The main agent may manage its team's routines too.
//! * **Personality** (`AgentAutonomy::personality`): a preset from
//!   `docs/contracts/personal-agent-personalities.json` + a free-text tweak,
//!   rendered into the persona file. Tone only — never rules or approvals.
//! * **Instruction proposals**: the agent may PROPOSE a new version of its
//!   own instructions; the user sees From/To and applies, edits or dismisses
//!   it (owner-only), with one-step Undo. Agents never rewrite their
//!   instructions themselves.
//!
//! Everything here acts only on the calling agent (or, for the main agent,
//! its workspace's agents) — the identity comes from the calling session
//! (`personal_agent_team::session_agent`), never from the request.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use otto_core::{Error, Result};
use otto_state::{
    AgentSchedulePatch, NewAgentSchedule, PersonalAgent, PersonalAgentSchedule, Personality,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::personal_agent_feed::routine_label;
use crate::personal_agents_engine::{self as pa_engine, one_line};
use crate::AssistantCtx;

/// The personality catalog (one source with the UI).
pub const PERSONALITIES_JSON: &str =
    include_str!("../../../docs/contracts/personal-agent-personalities.json");

/// Most routines one agent may have.
const MAX_ROUTINES: usize = 50;
/// Longest routine instruction / proposal (bytes).
const MAX_TEXT: usize = 16 * 1024;
/// Upcoming runs a routine view lists.
const NEXT_RUNS: usize = 3;

/// Every personality preset `{id, label, description, prompt, sample, default_for?}`.
pub fn personalities() -> Vec<Value> {
    static ALL: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        serde_json::from_str::<Value>(PERSONALITIES_JSON)
            .ok()
            .and_then(|v| v.get("personalities").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    })
    .clone()
}

fn preset(id: &str) -> Option<Value> {
    personalities()
        .into_iter()
        .find(|p| p["id"].as_str() == Some(id))
}

/// The preset an agent without one gets: the main agent's, else any agent's.
fn default_preset(primary: bool) -> Option<Value> {
    let role = if primary { "main" } else { "agent" };
    personalities()
        .into_iter()
        .find(|p| p["default_for"].as_str() == Some(role))
}

/// The "Your personality" persona section.
pub fn render_personality(p: &Personality, primary: bool) -> String {
    let chosen = Some(p.preset.trim())
        .filter(|s| !s.is_empty())
        .and_then(preset)
        .or_else(|| default_preset(primary));
    let mut s = String::new();
    if let Some(v) = &chosen {
        s.push_str(&format!(
            "\n## Your personality\n{} ({}). This shapes your TONE in chat, reports and \
             messages to other agents — never your rules, permissions or approvals.\n",
            v["prompt"].as_str().unwrap_or(""),
            v["label"].as_str().unwrap_or("")
        ));
    }
    let note = one_line(&p.note);
    if !note.is_empty() {
        if s.is_empty() {
            s.push_str("\n## Your personality\n");
        }
        s.push_str(&format!(
            "The user's own words on how you should come across: {note}\n"
        ));
    }
    s
}

/// The persona section every agent gets about its own scope — routines,
/// personality and instructions — and how to learn.
pub fn render_self_section() -> &'static str {
    "\n## Your routines, personality and learning\n\
     - Routines are recurring tasks you run on a schedule. When the user asks for one \
     (\"every weekday at 9 Israel time, send me a Slack message with …\"), create it at once \
     with `otto_routine_create` and confirm in one line with its schedule and next run. A \
     routine that only reports in this chat needs no approval; one that posts to Slack waits \
     for the user's approval first (Otto asks them) — say so, never claim it is live before \
     it is. `otto_routine_list` / `otto_routine_update` (also pause: \
     enabled=false) / `otto_routine_delete` manage them. Turn the user's words into the \
     structured schedule yourself; omit the timezone to use the user's own. Always give it a \
     `name` the user would say (2–5 words, e.g. \"Morning PR brief\") and write the \
     instruction in the user's voice (\"Remind me to review stale PRs\", never \"Remind the \
     user …\"). Check \
     `otto_routine_list` first: if a routine already runs on that schedule, ask before adding \
     another. If the result has a `note`, pass it on. Always state times in the \
     user's own timezone (next_runs already are), never in UTC or another zone. Each result is posted into this chat automatically.\n\
     - `otto_personality_set` changes your personality when the user asks (\"be more \
     blunt\").\n\
     - Learning: keep `memory/notes.md` small and curated — one fact per line, rewrite \
     (never just append), drop stale lines, stay under ~60 lines. When the user corrects you \
     (\"no, do it like X\") or states a preference, record it there. If your standing \
     instructions themselves should change, PROPOSE it with `otto_instructions_propose` \
     (the full new text + why) — the user decides; never claim it is applied.\n"
}

// ---------------------------------------------------------------------------
// Routines
// ---------------------------------------------------------------------------

/// Where a routine's result goes besides the chat.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
pub struct Deliver {
    /// `chat` (default) | `slack`.
    #[serde(default, rename = "type")]
    pub kind: String,
    /// Slack channel id / name, or a user id for a DM.
    #[serde(default)]
    pub channel: String,
    /// `bot` (default — the Otto Slack app) | `me` (as the user).
    #[serde(default, rename = "as")]
    pub as_: String,
}

/// `otto.routine_create` / `_update` body.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct RoutineIn {
    /// `{cadence: daily|weekly|days|interval|cron|once, at, weekday, days, every_min, expr, run_at}`.
    #[serde(default)]
    pub schedule: Option<Value>,
    /// IANA zone; omitted = the user's (this machine's) timezone.
    #[serde(default)]
    pub timezone: Option<String>,
    /// What each run does.
    #[serde(default)]
    pub instruction: Option<String>,
    /// A short title (create only) — stored as the instruction's first line,
    /// which is what every list shows.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub deliver: Option<Deliver>,
    #[serde(default)]
    pub enabled: Option<bool>,
    /// `read_only` | `directed` (default).
    #[serde(default)]
    pub permission: Option<String>,
}

/// This machine's IANA timezone (`/etc/localtime` → `…/zoneinfo/Asia/Jerusalem`),
/// else `$TZ`, else UTC. Otto is local: the user's timezone is the Mac's.
pub fn local_timezone() -> String {
    let from_link = std::fs::read_link("/etc/localtime").ok().and_then(|p| {
        let s = p.to_string_lossy().to_string();
        s.split_once("zoneinfo/").map(|(_, z)| z.to_string())
    });
    from_link
        .or_else(|| std::env::var("TZ").ok())
        .filter(|z| z.parse::<chrono_tz::Tz>().is_ok())
        .unwrap_or_else(|| "UTC".into())
}

const DAY_NAMES: [&str; 7] = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

/// Cron day-of-week (0 = Sunday) for a day name or number.
fn day_num(v: &Value) -> Option<u32> {
    if let Some(n) = v.as_u64() {
        return (n <= 7).then_some((n % 7) as u32);
    }
    let s = v.as_str()?.trim().to_ascii_lowercase();
    DAY_NAMES
        .iter()
        .position(|d| s.starts_with(d))
        .map(|i| i as u32)
}

/// Days from `days`: a list (`["sun","thu"]`, `[0,4]`) or a range / alias
/// (`"sun-fri"`, `"weekdays"` = Mon–Fri, `"weekend"`).
fn days_of(v: &Value) -> Result<Vec<u32>> {
    let bad = || {
        Error::Invalid(
            "schedule.days must be day names/numbers (0=Sun) or a range like \"sun-thu\"".into(),
        )
    };
    let mut out: Vec<u32> = match v {
        Value::Array(a) => a
            .iter()
            .map(|d| day_num(d).ok_or_else(bad))
            .collect::<Result<_>>()?,
        Value::String(s) => {
            let s = s.trim().to_ascii_lowercase();
            match s.as_str() {
                "weekdays" | "workdays" => vec![1, 2, 3, 4, 5],
                "weekend" | "weekends" => vec![0, 6],
                "everyday" | "daily" | "all" => (0..7).collect(),
                _ => match s.split_once('-') {
                    Some((a, b)) => {
                        let (a, b) = (
                            day_num(&json!(a)).ok_or_else(bad)?,
                            day_num(&json!(b)).ok_or_else(bad)?,
                        );
                        let mut d = a;
                        let mut v = vec![d];
                        while d != b {
                            d = (d + 1) % 7;
                            v.push(d);
                        }
                        v
                    }
                    None => s
                        .split(',')
                        .map(|d| day_num(&json!(d)).ok_or_else(bad))
                        .collect::<Result<_>>()?,
                },
            }
        }
        _ => return Err(bad()),
    };
    out.sort_unstable();
    out.dedup();
    if out.is_empty() {
        return Err(bad());
    }
    Ok(out)
}

/// The routine's schedule in Otto's cadence format: `days` (or
/// `weekdays`) becomes a cron on those days at `at`; everything else passes
/// through to the cadence validator.
pub fn normalize_schedule(spec: &Value) -> Result<Value> {
    let cadence = spec.get("cadence").and_then(Value::as_str).unwrap_or("");
    if !matches!(cadence, "days" | "weekdays") {
        return Ok(spec.clone());
    }
    let at = spec.get("at").and_then(Value::as_str).unwrap_or("09:00");
    let (h, m) = at
        .split_once(':')
        .and_then(|(h, m)| Some((h.trim().parse::<u32>().ok()?, m.trim().parse::<u32>().ok()?)))
        .filter(|(h, m)| *h < 24 && *m < 60)
        .ok_or_else(|| Error::Invalid(format!("schedule.at must be HH:MM (got '{at}')")))?;
    let days = match spec.get("days") {
        Some(d) => days_of(d)?,
        None => vec![1, 2, 3, 4, 5],
    };
    let list = days
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    Ok(json!({"cadence": "cron", "expr": format!("{m} {h} * * {list}")}))
}

/// The routine's next `n` fire times.
pub fn next_runs<C: AssistantCtx>(
    spec: &Value,
    tz: &str,
    from: DateTime<Utc>,
    n: usize,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut at = from;
    // In the user's own clock, so an agent quoting them never says "UTC".
    let user_tz = local_timezone()
        .parse::<chrono_tz::Tz>()
        .unwrap_or(chrono_tz::UTC);
    for _ in 0..n {
        match C::cadence_next_run(spec, at, tz) {
            Some(t) if t > at => {
                out.push(t.with_timezone(&user_tz).to_rfc3339());
                at = t;
            }
            _ => break,
        }
    }
    out
}

/// A routine as its agent / the UI sees it.
pub fn routine_view<C: AssistantCtx>(agent: &PersonalAgent, s: &PersonalAgentSchedule) -> Value {
    json!({
        "id": s.id,
        "agent_id": agent.id,
        "agent_name": agent.name,
        "name": routine_label(&s.directive),
        "instruction": s.directive,
        "schedule": s.schedule,
        "timezone": s.timezone,
        "enabled": s.enabled && agent.enabled,
        "paused": !s.enabled,
        "agent_paused": !agent.enabled,
        "permission": s.permission,
        "deliver": deliver_of(s),
        "last_run_at": s.last_run_at,
        "next_runs": if s.enabled && agent.enabled {
            next_runs::<C>(&s.schedule, &s.timezone, Utc::now(), NEXT_RUNS)
        } else {
            Vec::new()
        },
    })
}

/// A routine's destination as `{type, channel?, as?}`.
fn deliver_of(s: &PersonalAgentSchedule) -> Value {
    if let Some(d) = &s.delivery {
        if d["type"] == "slack" {
            return json!({"type": "slack", "channel": d["chat_id"], "as": "bot"});
        }
    }
    match s.directive.contains(SLACK_AS_ME_MARK) {
        true => json!({"type": "slack", "as": "me"}),
        false => json!({"type": "chat"}),
    }
}

/// Marks a routine whose run posts to Slack as the user (the instruction
/// carries the channel).
const SLACK_AS_ME_MARK: &str = "[Deliver: post the result to Slack as the user";

/// The routine's destination (`delivery_json`) and the instruction it runs.
async fn plan_delivery<C: AssistantCtx>(
    ctx: &C,
    agent: &PersonalAgent,
    instruction: &str,
    deliver: Option<&Deliver>,
) -> Result<(Option<Value>, String)> {
    let base = match instruction.find(SLACK_AS_ME_MARK) {
        Some(i) => instruction[..i].trim_end().to_string(),
        None => instruction.trim().to_string(),
    };
    let Some(d) = deliver.filter(|d| d.kind == "slack") else {
        return Ok((Some(json!({"type": "none"})), base));
    };
    let channel = one_line(&d.channel);
    if channel.is_empty() {
        return Err(Error::Invalid(
            "deliver.channel is required for Slack (a channel id/name, or a user id for a DM)"
                .into(),
        ));
    }
    if d.as_ == "me" {
        let teams = pa_engine::repo(ctx)
            .autonomy(&agent.id)
            .await?
            .slack_as_me_teams;
        if teams.is_empty() {
            return Err(Error::Forbidden(
                "this agent may not act as the user in Slack — ask the user to turn on \"Can act \
                 as me in Slack\" in its Details, or deliver as the Otto bot (as: \"bot\")"
                    .into(),
            ));
        }
        let text = format!(
            "{base}\n\n{SLACK_AS_ME_MARK} in {channel} with the slack_post_message tool, as \
             yourself acting for the user. Post it once.]"
        );
        return Ok((Some(json!({"type": "none"})), text));
    }
    Ok((Some(json!({"type": "slack", "chat_id": channel})), base))
}

/// Why an `otto.routine_create` / `otto.routine_update` call (bare tool
/// name, the caller's arguments) needs the owner's approval: it sets or
/// changes a destination OUTSIDE the agent's chat (Slack as the Otto bot or
/// as the user, or any non-chat kind added later), which then posts
/// unattended on every run. `None` for a chat-only routine and for an
/// update that leaves the destination alone (no `deliver`) — "remind me
/// every morning" never waits on a card. The text names where it posts, as
/// whom, and on what schedule: the approval card shows it.
///
/// The governed tool pipeline (otto-server `personal_agent_policy::decide`)
/// turns `Some` into a forced approval.
pub fn outward_delivery_reason(bare: &str, args: &Value) -> Option<String> {
    let verb = match bare {
        "routine_create" => "sets up a routine that",
        "routine_update" => "changes a routine so it",
        _ => return None,
    };
    let d = args.get("deliver").filter(|d| !d.is_null())?;
    let kind = d
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if matches!(kind.as_str(), "" | "chat" | "none") {
        return None;
    }
    let field = |k: &str| {
        d.get(k)
            .and_then(Value::as_str)
            .map(one_line)
            .filter(|s| !s.is_empty())
    };
    let target = field("channel").unwrap_or_else(|| "(no channel given)".into());
    let whom = match field("as").as_deref() {
        Some("me") => "AS YOU, in the Slack workspaces you let this agent act in",
        _ => "as the Otto Slack bot",
    };
    let place = if kind == "slack" {
        format!("posts to Slack {target} {whom}")
    } else {
        format!("delivers to a '{kind}' destination ({target})")
    };
    let schedule = match args.get("schedule").filter(|s| !s.is_null()) {
        Some(s) => schedule_words(s),
        None if bare == "routine_update" => "schedule unchanged".into(),
        None => "no schedule given".into(),
    };
    let tz = args
        .get("timezone")
        .and_then(Value::as_str)
        .map(one_line)
        .filter(|t| !t.is_empty())
        .map(|t| format!(" ({t})"))
        .unwrap_or_default();
    let agent = args
        .get("agent")
        .and_then(Value::as_str)
        .map(one_line)
        .filter(|a| !a.is_empty())
        .map(|a| format!(" for agent \"{a}\""))
        .unwrap_or_default();
    Some(format!(
        "otto.{bare} {verb} {place}, unattended on every run{agent} — {schedule}{tz}"
    ))
}

/// A routine schedule spec in a few words for an approval card
/// (`daily at 09:00`, `cron 0 9 * * 1-5`, `days sun-thu at 09:00`, …).
fn schedule_words(s: &Value) -> String {
    let get = |k: &str| match s.get(k) {
        Some(Value::String(v)) => Some(one_line(v)),
        Some(Value::Null) | None => None,
        Some(v) => Some(v.to_string()),
    };
    let mut out = get("cadence").unwrap_or_else(|| "schedule".into());
    for (k, label) in [
        ("expr", ""),
        ("days", ""),
        ("weekday", "weekday "),
        ("at", "at "),
        ("every_min", "every min: "),
        ("run_at", "at "),
    ] {
        if let Some(v) = get(k) {
            out.push_str(&format!(" {label}{v}"));
        }
    }
    out.chars().take(160).collect()
}

/// The agent a routine call acts on: the caller, or — for the main agent —
/// one of its owner's agents in its workspace (`agent` = id or name).
pub async fn target<C: AssistantCtx>(
    ctx: &C,
    caller: &PersonalAgent,
    agent: Option<&str>,
) -> Result<PersonalAgent> {
    let Some(r) = agent.map(str::trim).filter(|r| !r.is_empty()) else {
        return Ok(caller.clone());
    };
    if r == caller.id || r.eq_ignore_ascii_case(caller.name.trim()) {
        return Ok(caller.clone());
    }
    if !pa_engine::repo(ctx).autonomy(&caller.id).await?.primary {
        return Err(Error::Forbidden(
            "you manage only your own routines — the main agent manages the others'".into(),
        ));
    }
    // Its OWNER's agents only — never another member's in a shared workspace.
    pa_engine::repo(ctx)
        .list_by_workspace(&caller.workspace_id)
        .await?
        .into_iter()
        .filter(|a| crate::personal_agent_team::same_owner(a, caller))
        .find(|a| a.id == r || a.name.trim().eq_ignore_ascii_case(r))
        .ok_or_else(|| Error::NotFound(format!("no agent \"{r}\"")))
}

/// `otto.routine_list`.
pub async fn list<C: AssistantCtx>(ctx: &C, agent: &PersonalAgent) -> Result<Value> {
    let schedules = pa_engine::repo(ctx).list_schedules(&agent.id).await?;
    Ok(json!({
        "agent": {"id": agent.id, "name": agent.name, "enabled": agent.enabled},
        "user_timezone": local_timezone(),
        "routines": schedules.iter().map(|s| routine_view::<C>(agent, s)).collect::<Vec<_>>(),
    }))
}

/// `otto.routine_create`: validated, armed now, announced.
pub async fn create<C: AssistantCtx>(
    ctx: &C,
    agent: &PersonalAgent,
    req: RoutineIn,
) -> Result<Value> {
    let repo = pa_engine::repo(ctx);
    let existing = repo.list_schedules(&agent.id).await?;
    if existing.len() >= MAX_ROUTINES {
        return Err(Error::Conflict(format!(
            "an agent can have at most {MAX_ROUTINES} routines"
        )));
    }
    let spec = normalize_schedule(
        req.schedule
            .as_ref()
            .ok_or_else(|| Error::Invalid("schedule is required".into()))?,
    )?;
    let instruction = req.instruction.unwrap_or_default();
    let instruction = match req.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => format!("{}\n\n{instruction}", one_line(name)),
        None => instruction,
    };
    if instruction.trim().is_empty() || instruction.len() > MAX_TEXT {
        return Err(Error::Invalid("instruction must be 1 byte – 16 KB".into()));
    }
    let tz = req
        .timezone
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(local_timezone);
    crate::personal_agent_admin::check_timezone(&tz)?;
    crate::personal_agent_admin::check_permission(req.permission.as_deref())?;
    C::cadence_validate(&spec)?;
    let (delivery, directive) =
        plan_delivery(ctx, agent, &instruction, req.deliver.as_ref()).await?;
    let s = repo
        .create_schedule_once(
            NewAgentSchedule {
                agent_id: agent.id.clone(),
                schedule: spec,
                timezone: tz,
                directive,
                enabled: req.enabled.unwrap_or(true),
            },
            None,
            req.permission.as_deref().unwrap_or("directed"),
        )
        .await?;
    repo.set_schedule_delivery(&s.id, delivery.as_ref()).await?;
    crate::personal_agent_admin::refresh_next_run::<C>(&repo, &s).await;
    let resumed =
        s.enabled && crate::personal_agent_admin::resume_for_routine(&repo, agent).await?;
    pa_engine::emit_changed(ctx, &agent.workspace_id, &agent.id, "schedules");
    let agent = PersonalAgent {
        enabled: agent.enabled || resumed,
        ..agent.clone()
    };
    let mut view = routine_view::<C>(&agent, &repo.get_schedule(&s.id).await?);
    // A near-duplicate is usually a misunderstanding — say so, don't block.
    if let Some(twin) = existing
        .iter()
        .find(|o| o.schedule == s.schedule && o.timezone == s.timezone)
    {
        view["note"] = json!(format!(
            "Another routine already runs on this schedule: \"{}\". Tell the user and ask \
             whether to keep both.",
            routine_label(&twin.directive)
        ));
    }
    if resumed {
        // Tell the agent so it can say so ("I was paused — I'm active again").
        view["agent_resumed"] = json!(true);
    }
    Ok(view)
}

/// The routine `id` of `agent` (never another agent's).
async fn own<C: AssistantCtx>(
    ctx: &C,
    agent: &PersonalAgent,
    id: &str,
) -> Result<PersonalAgentSchedule> {
    let s = pa_engine::repo(ctx).get_schedule(id).await?;
    if s.agent_id != agent.id {
        return Err(Error::NotFound(format!(
            "no routine {id} on \"{}\"",
            agent.name
        )));
    }
    Ok(s)
}

/// `otto.routine_update` (also pause / resume with `enabled`).
pub async fn update<C: AssistantCtx>(
    ctx: &C,
    agent: &PersonalAgent,
    id: &str,
    req: RoutineIn,
) -> Result<Value> {
    let repo = pa_engine::repo(ctx);
    let cur = own(ctx, agent, id).await?;
    let spec = req.schedule.as_ref().map(normalize_schedule).transpose()?;
    if let Some(s) = &spec {
        C::cadence_validate(s)?;
    }
    if let Some(tz) = req.timezone.as_deref() {
        crate::personal_agent_admin::check_timezone(tz)?;
    }
    crate::personal_agent_admin::check_permission(req.permission.as_deref())?;
    if let Some(p) = req.permission.as_deref() {
        repo.set_schedule_permission(id, p).await?;
    }
    let mut directive = None;
    if req.instruction.is_some() || req.deliver.is_some() {
        let text = req
            .instruction
            .clone()
            .unwrap_or_else(|| cur.directive.clone());
        if text.trim().is_empty() || text.len() > MAX_TEXT {
            return Err(Error::Invalid("instruction must be 1 byte – 16 KB".into()));
        }
        let deliver = match &req.deliver {
            Some(d) => Some(d.clone()),
            // An instruction edit keeps the routine's destination.
            None => serde_json::from_value::<Deliver>(deliver_of(&cur))
                .ok()
                .map(|mut d| {
                    if d.kind == "slack" && d.as_ == "me" {
                        d.channel = slack_as_me_channel(&cur.directive).unwrap_or_default();
                    }
                    d
                }),
        };
        let (delivery, text) = plan_delivery(ctx, agent, &text, deliver.as_ref()).await?;
        repo.set_schedule_delivery(id, delivery.as_ref()).await?;
        directive = Some(text);
    }
    let cadence_changed = spec.is_some() || req.timezone.is_some();
    let resumed = !cur.enabled && req.enabled == Some(true);
    let updated = repo
        .update_schedule(
            id,
            AgentSchedulePatch {
                schedule: spec,
                timezone: req.timezone,
                directive,
                enabled: req.enabled,
                rearm_at: (cadence_changed || resumed).then(|| Utc::now().to_rfc3339()),
                reset_once: cadence_changed,
            },
        )
        .await?;
    crate::personal_agent_admin::refresh_next_run::<C>(&repo, &updated).await;
    pa_engine::emit_changed(ctx, &agent.workspace_id, &agent.id, "schedules");
    Ok(routine_view::<C>(agent, &repo.get_schedule(id).await?))
}

fn slack_as_me_channel(directive: &str) -> Option<String> {
    let rest = &directive[directive.find(SLACK_AS_ME_MARK)? + SLACK_AS_ME_MARK.len()..];
    let rest = rest.trim_start().strip_prefix("in ")?;
    rest.split(" with the slack_post_message tool")
        .next()
        .map(str::to_string)
}

/// `otto.routine_delete`.
pub async fn delete<C: AssistantCtx>(ctx: &C, agent: &PersonalAgent, id: &str) -> Result<Value> {
    let s = own(ctx, agent, id).await?;
    pa_engine::repo(ctx).delete_schedule(&s.id).await?;
    pa_engine::emit_changed(ctx, &agent.workspace_id, &agent.id, "schedules");
    Ok(json!({"ok": true, "deleted": routine_label(&s.directive)}))
}

// ---------------------------------------------------------------------------
// Personality + instruction proposals
// ---------------------------------------------------------------------------

/// `otto.personality_set` (and the Details editor): a preset id ("" = the
/// default for its role) + a free-text tweak. Re-provisioned on the next
/// session.
pub async fn set_personality<C: AssistantCtx>(
    ctx: &C,
    agent: &PersonalAgent,
    preset_id: Option<&str>,
    note: Option<&str>,
) -> Result<Personality> {
    let repo = pa_engine::repo(ctx);
    let mut cfg = repo.autonomy(&agent.id).await?;
    if let Some(p) = preset_id.map(str::trim) {
        if !p.is_empty() && preset(p).is_none() {
            let ids: Vec<String> = personalities()
                .iter()
                .filter_map(|v| v["id"].as_str().map(str::to_string))
                .collect();
            return Err(Error::Invalid(format!(
                "unknown personality '{p}' (have: {})",
                ids.join(", ")
            )));
        }
        cfg.personality.preset = p.to_string();
    }
    if let Some(n) = note {
        if n.len() > 1024 {
            return Err(Error::Invalid("the personality note is over 1 KB".into()));
        }
        cfg.personality.note = n.trim().to_string();
    }
    repo.save_autonomy(&agent.id, &cfg).await?;
    pa_engine::emit_changed(ctx, &agent.workspace_id, &agent.id, "agent");
    Ok(cfg.personality)
}

fn proposal_path(dir: &Path) -> PathBuf {
    dir.join("proposals").join("instructions.json")
}
fn history_path(dir: &Path) -> PathBuf {
    dir.join("proposals").join("instructions-history.json")
}

/// The agent's pending instruction proposal, if any:
/// `{from, to, reason, proposed_at}`.
pub async fn proposal<C: AssistantCtx>(ctx: &C, agent: &PersonalAgent) -> Result<Option<Value>> {
    let dir = pa_engine::agent_directory(ctx, agent)?;
    match tokio::fs::read_to_string(proposal_path(&dir)).await {
        Ok(t) => Ok(serde_json::from_str(&t).ok()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Internal(format!("read proposal: {e}"))),
    }
}

/// `otto.instructions_propose`: record a proposed new version of the agent's
/// own instructions for the user to decide on (replaces an older one).
pub async fn propose<C: AssistantCtx>(
    ctx: &C,
    agent: &PersonalAgent,
    to: &str,
    reason: &str,
) -> Result<Value> {
    if to.trim().is_empty() || to.len() > 32 * 1024 {
        return Err(Error::Invalid("instructions must be 1 byte – 32 KB".into()));
    }
    if to.trim() == agent.soul_md.trim() {
        return Err(Error::Invalid(
            "that is the same as the current instructions".into(),
        ));
    }
    let dir = pa_engine::agent_directory(ctx, agent)?;
    let v = json!({
        "from": agent.soul_md,
        "to": to.trim(),
        "reason": one_line(reason).chars().take(500).collect::<String>(),
        "proposed_at": Utc::now().to_rfc3339(),
    });
    write_json(&proposal_path(&dir), &v).await?;
    pa_engine::emit_changed(ctx, &agent.workspace_id, &agent.id, "agent");
    Ok(json!({"ok": true, "status": "proposed — the user decides in Personal Agents › Details"}))
}

async fn write_json(path: &Path, v: &Value) -> Result<()> {
    if let Some(p) = path.parent() {
        tokio::fs::create_dir_all(p)
            .await
            .map_err(|e| Error::Internal(format!("proposal dir: {e}")))?;
    }
    tokio::fs::write(path, v.to_string())
        .await
        .map_err(|e| Error::Internal(format!("write proposal: {e}")))
}

/// Apply the pending proposal (optionally edited first: `text`), keeping the
/// previous instructions for Undo. Returns the new instructions.
pub async fn apply_proposal<C: AssistantCtx>(
    ctx: &C,
    agent: &PersonalAgent,
    text: Option<&str>,
) -> Result<String> {
    let p = proposal(ctx, agent)
        .await?
        .ok_or_else(|| Error::NotFound("no pending instruction proposal".into()))?;
    let to = text
        .map(str::to_string)
        .or_else(|| p["to"].as_str().map(str::to_string))
        .unwrap_or_default();
    if to.trim().is_empty() || to.len() > 32 * 1024 {
        return Err(Error::Invalid("instructions must be 1 byte – 32 KB".into()));
    }
    set_instructions(ctx, agent, to.trim()).await?;
    let dir = pa_engine::agent_directory(ctx, agent)?;
    let _ = tokio::fs::remove_file(proposal_path(&dir)).await;
    Ok(to.trim().to_string())
}

/// Write new instructions, remembering the old ones for one-step Undo.
async fn set_instructions<C: AssistantCtx>(ctx: &C, agent: &PersonalAgent, to: &str) -> Result<()> {
    let dir = pa_engine::agent_directory(ctx, agent)?;
    write_json(
        &history_path(&dir),
        &json!({"previous": agent.soul_md, "replaced_at": Utc::now().to_rfc3339()}),
    )
    .await?;
    pa_engine::repo(ctx)
        .update(
            &agent.id,
            otto_state::PersonalAgentPatch {
                soul_md: Some(to.to_string()),
                ..Default::default()
            },
        )
        .await?;
    pa_engine::emit_changed(ctx, &agent.workspace_id, &agent.id, "agent");
    Ok(())
}

/// Dismiss the pending proposal.
pub async fn dismiss_proposal<C: AssistantCtx>(ctx: &C, agent: &PersonalAgent) -> Result<()> {
    let dir = pa_engine::agent_directory(ctx, agent)?;
    let _ = tokio::fs::remove_file(proposal_path(&dir)).await;
    pa_engine::emit_changed(ctx, &agent.workspace_id, &agent.id, "agent");
    Ok(())
}

/// Undo the last applied instruction change (restores the previous text).
pub async fn undo_instructions<C: AssistantCtx>(ctx: &C, agent: &PersonalAgent) -> Result<String> {
    let dir = pa_engine::agent_directory(ctx, agent)?;
    let h: Value = tokio::fs::read_to_string(history_path(&dir))
        .await
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .ok_or_else(|| Error::NotFound("nothing to undo".into()))?;
    let prev = h["previous"].as_str().unwrap_or("").to_string();
    set_instructions(ctx, agent, &prev).await?;
    // One step: undoing again would re-apply the change, so forget it.
    let _ = tokio::fs::remove_file(history_path(&dir)).await;
    Ok(prev)
}

/// Whether an applied change can be undone.
pub async fn can_undo<C: AssistantCtx>(ctx: &C, agent: &PersonalAgent) -> bool {
    match pa_engine::agent_directory(ctx, agent) {
        Ok(dir) => tokio::fs::try_exists(history_path(&dir))
            .await
            .unwrap_or(false),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weekday_routines_become_crons_on_the_right_days() {
        let s = normalize_schedule(&json!({"cadence": "days", "at": "09:00", "days": "sun-thu"}))
            .unwrap();
        assert_eq!(s, json!({"cadence": "cron", "expr": "0 9 * * 0,1,2,3,4"}));
        let s =
            normalize_schedule(&json!({"cadence": "days", "at": "18:30", "days": "sunday-friday"}))
                .unwrap();
        assert_eq!(s["expr"], "30 18 * * 0,1,2,3,4,5");
        let s = normalize_schedule(&json!({"cadence": "weekdays", "at": "7:05"})).unwrap();
        assert_eq!(s["expr"], "5 7 * * 1,2,3,4,5");
        let s =
            normalize_schedule(&json!({"cadence": "days", "at": "09:00", "days": ["fri", "Sat"]}))
                .unwrap();
        assert_eq!(s["expr"], "0 9 * * 5,6");
        // A wrap-around range.
        let s = normalize_schedule(&json!({"cadence": "days", "at": "09:00", "days": "fri-mon"}))
            .unwrap();
        assert_eq!(s["expr"], "0 9 * * 0,1,5,6");
        assert!(normalize_schedule(&json!({"cadence": "days", "at": "25:00"})).is_err());
        assert!(normalize_schedule(&json!({"cadence": "days", "days": "someday"})).is_err());
        // Other cadences pass through to the validator.
        let d = json!({"cadence": "daily", "at": "09:00"});
        assert_eq!(normalize_schedule(&d).unwrap(), d);
    }

    #[test]
    fn personalities_render_tone_only_with_a_role_default() {
        let main = render_personality(&Personality::default(), true);
        assert!(main.contains("chief of staff"), "{main}");
        assert!(main.contains("never your rules"));
        let blunt = render_personality(
            &Personality {
                preset: "blunt".into(),
                note: "swear less\nplease".into(),
            },
            false,
        );
        assert!(blunt.contains("Be blunt"));
        assert!(blunt.contains("swear less please"));
        assert!(render_personality(&Personality::default(), false).contains("warm"));
        assert!(personalities().len() >= 8);
    }

    #[test]
    fn the_self_section_names_the_own_scope_tools() {
        let s = render_self_section();
        for t in [
            "otto_routine_create",
            "otto_routine_update",
            "otto_personality_set",
            "otto_instructions_propose",
            "one fact per line",
        ] {
            assert!(s.contains(t), "{t}");
        }
    }

    #[test]
    fn a_slack_as_me_channel_round_trips_through_the_instruction() {
        let d = format!("Brief me\n\n{SLACK_AS_ME_MARK} in #general with the slack_post_message tool, as yourself acting for the user. Post it once.]");
        assert_eq!(slack_as_me_channel(&d).as_deref(), Some("#general"));
        assert!(local_timezone().parse::<chrono_tz::Tz>().is_ok());
    }

    #[test]
    fn a_chat_only_routine_needs_no_approval() {
        let sched = json!({"cadence": "daily", "at": "09:00"});
        for args in [
            json!({"schedule": sched, "instruction": "Remind me to review PRs"}),
            json!({"schedule": sched, "instruction": "x", "deliver": {"type": "chat"}}),
            json!({"schedule": sched, "instruction": "x", "deliver": {}}),
            json!({"schedule": sched, "instruction": "x", "deliver": null}),
        ] {
            assert_eq!(
                outward_delivery_reason("routine_create", &args),
                None,
                "{args}"
            );
        }
        // Other tools are never this gate's business.
        let slack = json!({"deliver": {"type": "slack", "channel": "#general"}});
        assert_eq!(outward_delivery_reason("routine_delete", &slack), None);
        assert_eq!(outward_delivery_reason("routine_list", &slack), None);
    }

    #[test]
    fn a_slack_routine_needs_approval_naming_where_whom_and_when() {
        let args = json!({
            "schedule": {"cadence": "days", "at": "09:00", "days": "sun-thu"},
            "timezone": "Asia/Jerusalem",
            "instruction": "Morning PR brief",
            "deliver": {"type": "slack", "channel": "#eng-leads"},
        });
        let why = outward_delivery_reason("routine_create", &args).expect("approval");
        for part in [
            "otto.routine_create",
            "#eng-leads",
            "as the Otto Slack bot",
            "days sun-thu at 09:00",
            "Asia/Jerusalem",
            "unattended",
        ] {
            assert!(why.contains(part), "{part}: {why}");
        }
        // As the user: the card says so loudly.
        let me = json!({
            "schedule": {"cadence": "cron", "expr": "0 9 * * 1-5"},
            "deliver": {"type": "Slack", "channel": "C0123", "as": "me"},
            "agent": "Researcher",
        });
        let why = outward_delivery_reason("routine_create", &me).expect("approval");
        assert!(why.contains("AS YOU"), "{why}");
        assert!(
            why.contains("C0123") && why.contains("cron 0 9 * * 1-5"),
            "{why}"
        );
        assert!(why.contains("for agent \"Researcher\""), "{why}");
        // Any non-chat destination added later is outward too.
        let other = json!({"deliver": {"type": "email", "channel": "x@y"}});
        assert!(outward_delivery_reason("routine_create", &other).is_some());
    }

    #[test]
    fn an_update_needs_approval_only_when_it_sets_an_outward_destination() {
        // Adding (or re-pointing) a Slack delivery asks the owner.
        let add = json!({"routine_id": "s1", "deliver": {"type": "slack", "channel": "#ops"}});
        let why = outward_delivery_reason("routine_update", &add).expect("approval");
        assert!(
            why.contains("#ops") && why.contains("schedule unchanged"),
            "{why}"
        );
        // An approved routine's instruction / cadence / pause edit does not:
        // its destination stays what the owner approved.
        for args in [
            json!({"routine_id": "s1", "instruction": "Also list stale branches"}),
            json!({"routine_id": "s1", "schedule": {"cadence": "daily", "at": "10:00"}}),
            json!({"routine_id": "s1", "enabled": false}),
            json!({"routine_id": "s1", "deliver": {"type": "chat"}}),
        ] {
            assert_eq!(
                outward_delivery_reason("routine_update", &args),
                None,
                "{args}"
            );
        }
    }
}
