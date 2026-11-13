//! Personal-agent **administration** shared by the person's UI and an agent
//! acting for them through the governed `otto.*_personal_agent*` tools: the
//! templates (`docs/contracts/personal-agent-templates.json` — the same file
//! the New-agent sheet imports), field validation, create-in-one-call (the
//! agent, its schedules and "main agent"), the overview an agent reads, and the
//! approval-card text that names exactly what a governed write changes.
//!
//! The HTTP handlers stay in `otto-server` (`routes::personal_agents`): they
//! authenticate, check the workspace role and call in here.

use chrono::Utc;
use otto_core::{Error, Result};
use otto_state::{
    NewAgentSchedule, NewPersonalAgent, PersonalAgent, PersonalAgentSchedule, PersonalAgentsRepo,
};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::personal_agents_engine::{self as pa_engine, validate_agent_cwd};
use crate::AssistantCtx;

/// The template catalog (one source with the UI).
pub const TEMPLATES_JSON: &str =
    include_str!("../../../docs/contracts/personal-agent-templates.json");

/// The main agent's persona (the seeded first preset and the `main-agent`
/// template): one point of contact that routes work and reports back.
pub const MAIN_AGENT_PERSONA: &str = "I'm your main agent: your one point of contact. I can \
manage your other agents. Task me with anything — I'll make sure it gets to the right agent, and \
I come back with what they found.\n\n\
- I answer quick things myself, in short, direct replies.\n\
- Specialist or long-running work goes to the agent that fits; I tell you who has it, and when it \
reports back I tell you the substance and what needs you.\n\
- When no agent fits a recurring kind of work, I create one and tell you its name.\n\
- I ask only when I need information, a judgment call or your approval.";

/// Every template: `{id, title, description, avatar, name, soul_md, schedule,
/// directive}`.
pub fn templates() -> Vec<Value> {
    static ALL: std::sync::OnceLock<Vec<Value>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        serde_json::from_str::<Value>(TEMPLATES_JSON)
            .ok()
            .and_then(|v| v.get("templates").and_then(Value::as_array).cloned())
            .unwrap_or_default()
    })
    .clone()
}

/// One template by id (or exact title, case-insensitive).
pub fn template(reference: &str) -> Option<Value> {
    let r = reference.trim();
    templates().into_iter().find(|t| {
        t["id"].as_str() == Some(r)
            || t["title"]
                .as_str()
                .is_some_and(|x| x.eq_ignore_ascii_case(r))
    })
}

// --- Validation ------------------------------------------------------------

/// Same slug rule as scheduled tasks — a non-empty custom provider is allowed.
pub fn check_provider(p: &str) -> Result<()> {
    let p = p.trim();
    if p.is_empty()
        || p.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        Ok(())
    } else {
        Err(Error::Invalid(format!(
            "provider '{p}' is not a valid provider name"
        )))
    }
}

pub fn check_timezone(tz: &str) -> Result<()> {
    let t = tz.trim();
    if t.is_empty() || t.parse::<chrono_tz::Tz>().is_ok() {
        Ok(())
    } else {
        Err(Error::Invalid(format!("unknown timezone '{tz}'")))
    }
}

pub fn check_delivery(d: &Value) -> Result<()> {
    match d.get("type").and_then(Value::as_str).unwrap_or("none") {
        "none" | "slack" | "telegram" | "email" | "webhook" => Ok(()),
        other => Err(Error::Invalid(format!(
            "delivery type must be none|slack|telegram|email|webhook (got '{other}')"
        ))),
    }
}

pub fn check_permission(p: Option<&str>) -> Result<()> {
    match p {
        None | Some("read_only") | Some("directed") => Ok(()),
        Some(other) => Err(Error::Invalid(format!(
            "schedule permission must be read_only or directed (got '{other}')"
        ))),
    }
}

// --- Create ----------------------------------------------------------------

fn default_true() -> bool {
    true
}

/// One schedule of a create (`POST /personal-agents/{id}/schedules` body).
#[derive(Deserialize, Debug, Clone)]
pub struct ScheduleIn {
    #[serde(default)]
    pub idempotency_key: Option<String>,
    pub schedule: Value,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub directive: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// `read_only` | `directed` (default).
    #[serde(default)]
    pub permission: Option<String>,
}

/// `POST /workspaces/{id}/personal-agents`. `template` pre-fills name /
/// avatar / persona / the first schedule (explicit fields win); `schedules`
/// replaces the template's schedule (an empty directive takes the
/// template's); `timezone` is the template schedule's; `primary` makes it the
/// workspace's main agent.
#[derive(Deserialize, Debug, Default)]
pub struct CreateReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default)]
    pub soul_md: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub browser: Option<bool>,
    #[serde(default)]
    pub delivery: Option<Value>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub schedules: Option<Vec<ScheduleIn>>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub primary: Option<bool>,
}

// --- Person-only fields (an agent's own credential) -------------------------

/// The fields of a create only a PERSON may set: what the new agent runs and
/// where its reports go — the fields `PATCH /personal-agents/{id}` and a
/// schedule create keep to the owner (instructions differing from the
/// template's, a provider other than the default, a model, a folder, browser
/// access, an outward delivery, a schedule's own directive or a permission
/// other than `directed`). A template's persona and schedule are catalog
/// text, not the caller's, so a bare or template create stays open to an
/// agent. `primary` is not here: it stays Editor on the autonomy route too.
pub fn create_owner_only_fields(req: &CreateReq) -> Vec<&'static str> {
    let tpl = req.template.as_deref().and_then(template);
    let tpl_soul = tpl
        .as_ref()
        .and_then(|t| t["soul_md"].as_str())
        .unwrap_or("");
    let given = |v: &Option<String>| v.as_deref().is_some_and(|v| !v.trim().is_empty());
    let schedules = req.schedules.as_deref().unwrap_or_default();
    let mut out = Vec::new();
    if req.soul_md.as_deref().is_some_and(|s| s != tpl_soul) {
        out.push("instructions");
    }
    if req
        .provider
        .as_deref()
        .is_some_and(|p| !matches!(p.trim(), "" | "claude"))
    {
        out.push("provider");
    }
    if given(&req.model) {
        out.push("model");
    }
    if given(&req.cwd) {
        out.push("folder");
    }
    if req.browser == Some(true) {
        out.push("browser access");
    }
    if req
        .delivery
        .as_ref()
        .is_some_and(|d| d.get("type").and_then(Value::as_str).unwrap_or("none") != "none")
    {
        out.push("delivery");
    }
    if schedules.iter().any(|s| !s.directive.trim().is_empty()) {
        out.push("schedule directive");
    }
    if schedules
        .iter()
        .any(|s| s.permission.as_deref().is_some_and(|p| p != "directed"))
    {
        out.push("schedule permission");
    }
    out
}

/// Refuse an AGENT credential setting owner-only `fields` itself: the
/// governed `otto.{tool}` asks a person, and the approved call replays as
/// them (`otto_mcp::outward::sets_owner_only_agent_field`).
fn person_only_via(is_person: bool, fields: &[&str], tool: &str) -> Result<()> {
    if is_person || fields.is_empty() {
        return Ok(());
    }
    Err(Error::Forbidden(format!(
        "an agent's own token cannot set a personal agent's {} — that needs a person: use \
         otto.{tool} (it asks for approval) or the Personal Agents page",
        fields.join(", ")
    )))
}

/// `POST /workspaces/{id}/personal-agents` by `is_person` (the request's
/// credential): [`create_owner_only_fields`] are a person's to set.
pub fn check_creator(is_person: bool, req: &CreateReq) -> Result<()> {
    person_only_via(
        is_person,
        &create_owner_only_fields(req),
        "create_personal_agent",
    )
}

/// `PUT /personal-agents/self/personality` by `is_person`: the note is
/// injected into every future prompt as the user's own words, so only a
/// person sets (or clears) it, as on `PUT /personal-agents/{id}/personality`;
/// a preset (tone only) stays the agent's.
pub fn check_note_by(is_person: bool, sets_note: bool) -> Result<()> {
    let fields: &[&str] = if sets_note {
        &["personality note"]
    } else {
        &[]
    };
    person_only_via(is_person, fields, "personality_set")
}

/// Is THIS governed call a person's to make when an AGENT makes it?
/// `otto_mcp::outward::call_is_person_only`, or a routine posting outward
/// (`personal_agent_routines::outward_delivery_reason`) — both judged, as
/// the routes judge them, against the STORED destination when an update
/// names none (a routine's via `with_stored_deliver`, a scheduled task's
/// from its row). Otherwise a person approves an edit of an already-outward
/// routine or task, the call replays bound to the agent's session, and the
/// route refuses it (F15).
pub async fn governed_call_is_person_only<C: AssistantCtx>(
    ctx: &C,
    bare: &str,
    args: &Value,
) -> bool {
    use crate::personal_agent_routines::{outward_delivery_reason, with_stored_deliver};
    let filled = with_stored_deliver(ctx, bare, None, args).await;
    let args = filled.as_ref().unwrap_or(args);
    if otto_mcp::outward::call_is_person_only(bare, args)
        || outward_delivery_reason(bare, args).is_some()
    {
        return true;
    }
    let stored_task = match args.get("task_id").and_then(Value::as_str) {
        Some(id)
            if bare == "update_scheduled_task"
                && args.get("destination").is_none_or(Value::is_null) =>
        {
            otto_state::ScheduledTasksRepo::new(ctx.pool().clone())
                .get(id)
                .await
                .ok()
        }
        _ => None,
    };
    stored_task.is_some_and(|t| otto_mcp::outward::task_update_is_person_only(&t.destination, args))
}

/// The schedules a create adds: the request's, else the template's (in
/// `timezone`). An empty directive takes the template's.
fn planned_schedules(req: &CreateReq, tpl: Option<&Value>) -> Vec<ScheduleIn> {
    let tpl_directive = tpl
        .and_then(|t| t["directive"].as_str())
        .unwrap_or("")
        .to_string();
    match (&req.schedules, tpl) {
        (Some(list), _) => list
            .iter()
            .cloned()
            .map(|mut s| {
                if s.directive.trim().is_empty() {
                    s.directive = tpl_directive.clone();
                }
                s
            })
            .collect(),
        (None, Some(t)) if t["schedule"].is_object() => vec![ScheduleIn {
            idempotency_key: None,
            schedule: t["schedule"].clone(),
            timezone: req.timezone.clone(),
            directive: tpl_directive,
            enabled: true,
            permission: None,
        }],
        _ => Vec::new(),
    }
}

/// Validate a schedule against the cadence rules (`C::cadence_validate`).
fn check_schedule<C: AssistantCtx>(s: &ScheduleIn) -> Result<()> {
    C::cadence_validate(&s.schedule)?;
    check_timezone(s.timezone.as_deref().unwrap_or(""))?;
    check_permission(s.permission.as_deref())
}

/// Add one schedule to `agent_id` and show its next run.
pub async fn add_schedule<C: AssistantCtx>(
    repo: &PersonalAgentsRepo,
    agent_id: &str,
    s: ScheduleIn,
) -> Result<PersonalAgentSchedule> {
    check_schedule::<C>(&s)?;
    let timezone = s
        .timezone
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(crate::personal_agent_routines::local_timezone);
    let schedule = repo
        .create_schedule_once(
            NewAgentSchedule {
                agent_id: agent_id.to_string(),
                schedule: s.schedule,
                timezone,
                directive: s.directive,
                enabled: s.enabled,
            },
            s.idempotency_key.as_deref(),
            s.permission.as_deref().unwrap_or("directed"),
        )
        .await?;
    refresh_next_run::<C>(repo, &schedule).await;
    repo.get_schedule(&schedule.id).await
}

/// Recompute `next_run_at` for display (best-effort; cursor untouched).
pub async fn refresh_next_run<C: AssistantCtx>(
    repo: &PersonalAgentsRepo,
    s: &PersonalAgentSchedule,
) {
    let next = C::cadence_next_run(&s.schedule, Utc::now(), &s.timezone).map(|d| d.to_rfc3339());
    let _ = repo
        .set_schedule_runtime(&s.id, None, next.as_deref())
        .await;
}

/// Make `agent` its owner's main agent in its workspace (one per person per
/// workspace: the flag is cleared on the owner's other agents there, never
/// on another member's).
pub async fn make_primary(repo: &PersonalAgentsRepo, agent: &PersonalAgent) -> Result<()> {
    let mut cfg = repo.autonomy(&agent.id).await?;
    if !cfg.primary {
        cfg.primary = true;
        repo.save_autonomy(&agent.id, &cfg).await?;
    }
    clear_other_primaries(repo, agent).await
}

/// `agent`'s owner's main agent in its workspace, if one is set (the oldest
/// flagged, as `PersonalAgentsRepo::primary_agent` picks per workspace) — the
/// lead `agent` reports to.
pub async fn primary_of(
    repo: &PersonalAgentsRepo,
    agent: &PersonalAgent,
) -> Result<Option<PersonalAgent>> {
    // `list_by_workspace` is oldest first; the autonomy read is cached.
    for a in repo.list_by_workspace(&agent.workspace_id).await? {
        if crate::personal_agent_team::same_owner(&a, agent) && repo.autonomy(&a.id).await?.primary
        {
            return Ok(Some(a));
        }
    }
    Ok(None)
}

/// Setting up a routine means "run this": a paused agent is resumed, or the
/// routine would silently never fire while showing a next run. `true` when
/// it was paused.
pub async fn resume_for_routine(repo: &PersonalAgentsRepo, agent: &PersonalAgent) -> Result<bool> {
    if agent.enabled {
        return Ok(false);
    }
    repo.update(
        &agent.id,
        otto_state::PersonalAgentPatch {
            enabled: Some(true),
            ..Default::default()
        },
    )
    .await?;
    Ok(true)
}

/// Clear `primary` on every other agent of `agent`'s owner in its
/// workspace. "Main agent" is per (workspace, owner): one member choosing
/// theirs must not demote another member's.
pub async fn clear_other_primaries(repo: &PersonalAgentsRepo, agent: &PersonalAgent) -> Result<()> {
    for other in repo.list_by_workspace(&agent.workspace_id).await? {
        if other.id == agent.id || !crate::personal_agent_team::same_owner(&other, agent) {
            continue;
        }
        let mut cfg = repo.autonomy(&other.id).await?;
        if cfg.primary {
            cfg.primary = false;
            repo.save_autonomy(&other.id, &cfg).await?;
        }
    }
    Ok(())
}

/// Create an agent owned by `owner` in `ws` — the agent, then its schedules,
/// then "main agent". Everything is validated BEFORE the agent row exists, so
/// a bad cadence never leaves a half-made agent. Returns the agent with its
/// `schedules` (and `primary`) added.
pub async fn create<C: AssistantCtx>(
    ctx: &C,
    ws: &str,
    owner: &str,
    req: CreateReq,
) -> Result<Value> {
    let tpl = match req.template.as_deref().filter(|t| !t.trim().is_empty()) {
        Some(id) => Some(template(id).ok_or_else(|| {
            let ids: Vec<String> = templates()
                .iter()
                .filter_map(|t| t["id"].as_str().map(str::to_string))
                .collect();
            Error::Invalid(format!(
                "unknown template '{id}' (have: {})",
                ids.join(", ")
            ))
        })?),
        None => None,
    };
    let from_tpl = |k: &str| {
        tpl.as_ref()
            .and_then(|t| t[k].as_str())
            .unwrap_or("")
            .to_string()
    };
    let name = req
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| from_tpl("name"));
    if name.trim().is_empty() {
        return Err(Error::Invalid("name is required".into()));
    }
    let provider = req.provider.clone().unwrap_or_default();
    check_provider(&provider)?;
    let delivery = req
        .delivery
        .clone()
        .unwrap_or_else(|| json!({"type":"none"}));
    check_delivery(&delivery)?;
    if let Some(c) = req.cwd.as_deref() {
        validate_agent_cwd(c)?;
    }
    let schedules = planned_schedules(&req, tpl.as_ref());
    for s in &schedules {
        check_schedule::<C>(s)?;
    }
    let repo = pa_engine::repo(ctx);
    let agent = repo
        .create(NewPersonalAgent {
            avatar: req.avatar.clone().unwrap_or_else(|| from_tpl("avatar")),
            soul_md: req.soul_md.clone().unwrap_or_else(|| from_tpl("soul_md")),
            provider: if provider.trim().is_empty() {
                "claude".into()
            } else {
                provider
            },
            model: req.model.clone().unwrap_or_default(),
            cwd: req.cwd.clone().unwrap_or_default(),
            browser: req.browser.unwrap_or(false),
            delivery,
            enabled: req.enabled.unwrap_or(true),
            created_by: Some(owner.to_string()),
            ..NewPersonalAgent::defaults(ws.to_string(), name.trim().to_string())
        })
        .await?;
    let mut made = Vec::new();
    for s in schedules {
        made.push(add_schedule::<C>(&repo, &agent.id, s).await?);
    }
    // A template may make its agent the main agent (`main-agent`); an
    // explicit `primary` wins.
    let primary = req
        .primary
        .or_else(|| tpl.as_ref().and_then(|t| t["primary"].as_bool()));
    if primary == Some(true) {
        make_primary(&repo, &agent).await?;
    }
    let mut out = serde_json::to_value(&agent).map_err(|e| Error::Internal(e.to_string()))?;
    out["schedules"] = json!(made);
    if let Some(p) = primary {
        out["primary"] = json!(p);
    }
    Ok(out)
}

/// `GET /personal-agents/{id}/overview` — the agent, its schedules, autonomy
/// (incl. the Slack act-as-me teams) and its 5 latest runs: what an agent
/// reads before changing anything.
pub async fn overview(repo: &PersonalAgentsRepo, agent: &PersonalAgent) -> Result<Value> {
    let runs = repo.list_runs(&agent.id, 5).await?;
    Ok(json!({
        "agent": agent,
        "schedules": repo.list_schedules(&agent.id).await?,
        "autonomy": repo.autonomy(&agent.id).await?,
        "recent_runs": runs,
    }))
}

// --- Approval cards ----------------------------------------------------------

fn s<'a>(args: &'a Value, k: &str) -> Option<&'a str> {
    args.get(k).and_then(Value::as_str)
}

/// A persona as the approver reads it: its size and opening lines.
fn persona_summary(text: &str) -> String {
    let body: String = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with("<!--"))
        .take(4)
        .collect::<Vec<_>>()
        .join(" / ");
    let clip: String = body.chars().take(240).collect();
    format!("{} chars: \"{clip}\"", text.chars().count())
}

/// The persona change as the approver reads it: lines removed / added.
pub fn persona_diff(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let removed: Vec<&str> = old_lines
        .iter()
        .filter(|l| !l.trim().is_empty() && !new_lines.contains(l))
        .copied()
        .collect();
    let added: Vec<&str> = new_lines
        .iter()
        .filter(|l| !l.trim().is_empty() && !old_lines.contains(l))
        .copied()
        .collect();
    let show = |v: &[&str], sign: char| -> String {
        let mut out: Vec<String> = v
            .iter()
            .take(6)
            .map(|l| format!("{sign} {}", l.chars().take(140).collect::<String>()))
            .collect();
        if v.len() > 6 {
            out.push(format!("{sign} … {} more", v.len() - 6));
        }
        out.join("\n")
    };
    match (removed.is_empty(), added.is_empty()) {
        (true, true) => "persona: whitespace / order only".into(),
        _ => format!(
            "persona diff (-{} / +{} lines):\n{}\n{}",
            removed.len(),
            added.len(),
            show(&removed, '-'),
            show(&added, '+')
        )
        .trim_end()
        .to_string(),
    }
}

/// One schedule as the approver reads it.
fn schedule_line(sched: &Value) -> String {
    let spec = sched.get("schedule").unwrap_or(sched);
    let cadence = spec["cadence"].as_str().unwrap_or("?");
    let when = match cadence {
        "cron" => format!("cron `{}`", spec["expr"].as_str().unwrap_or("?")),
        "daily" => format!("daily at {}", spec["at"].as_str().unwrap_or("?")),
        "weekly" => format!(
            "weekly {} at {}",
            spec["weekday"].as_str().unwrap_or("?"),
            spec["at"].as_str().unwrap_or("?")
        ),
        "interval" => format!("every {} min", spec["every_min"]),
        "once" => format!("once at {}", spec["run_at"].as_str().unwrap_or("?")),
        other => other.to_string(),
    };
    let tz = s(sched, "timezone")
        .filter(|t| !t.is_empty())
        .unwrap_or("UTC");
    let perm = s(sched, "permission").unwrap_or("directed");
    let perm = if perm == "read_only" {
        "read-only"
    } else {
        "may act (directed)"
    };
    let directive: String = s(sched, "directive")
        .unwrap_or("")
        .chars()
        .take(160)
        .collect();
    format!("{when} ({tz}), {perm}: \"{directive}\"")
}

/// The approval-card text for a governed personal-agent write — exactly what
/// is created / changed (persona summary or diff against the stored one,
/// schedules, permission, main agent, Slack act-as-me teams). `None` = not a
/// personal-agent admin tool (the generic card applies).
pub async fn approval_detail(
    pool: &otto_state::DbPool,
    tool: &str,
    args: &Value,
) -> Option<String> {
    let short = tool.strip_prefix("otto.").unwrap_or(tool);
    if !short.contains("_personal_agent") {
        return None;
    }
    let repo = PersonalAgentsRepo::new(pool.clone());
    let agent = match s(args, "agent_id") {
        Some(id) => repo.get(id).await.ok(),
        None => None,
    };
    let label = |a: &Option<PersonalAgent>| match a {
        Some(a) => format!("'{}'", a.name),
        None => format!("'{}'", s(args, "agent_id").unwrap_or("?")),
    };
    Some(match short {
        "create_personal_agent" => {
            let tpl = s(args, "template").and_then(template);
            let name = s(args, "name")
                .or_else(|| tpl.as_ref().and_then(|t| t["name"].as_str()))
                .unwrap_or("?");
            let mut parts = vec![format!("Create personal agent '{name}'")];
            if let Some(t) = &tpl {
                parts.push(format!(
                    "from template '{}'",
                    t["title"].as_str().unwrap_or("?")
                ));
            }
            parts.push(format!(
                "provider {}{}",
                s(args, "provider")
                    .filter(|p| !p.is_empty())
                    .unwrap_or("claude"),
                s(args, "model")
                    .filter(|m| !m.is_empty())
                    .map(|m| format!(" ({m})"))
                    .unwrap_or_default()
            ));
            let persona = match (s(args, "soul_md"), &tpl) {
                (Some(p), Some(t)) => persona_diff(t["soul_md"].as_str().unwrap_or(""), p),
                (Some(p), None) => format!("persona {}", persona_summary(p)),
                (None, Some(t)) => format!(
                    "persona: the template's ({})",
                    persona_summary(t["soul_md"].as_str().unwrap_or(""))
                ),
                (None, None) => "no persona".into(),
            };
            parts.push(persona);
            let req: CreateReq = serde_json::from_value(args.clone()).unwrap_or_default();
            let scheds = planned_schedules(&req, tpl.as_ref());
            if scheds.is_empty() {
                parts.push("no schedule".into());
            }
            for sc in scheds {
                parts.push(format!(
                    "schedule: {}",
                    schedule_line(&json!({"schedule": sc.schedule, "timezone": sc.timezone,
                        "directive": sc.directive, "permission": sc.permission}))
                ));
            }
            if args.get("delivery").is_some_and(Value::is_object) {
                parts.push(format!(
                    "delivers reports to {}",
                    args["delivery"]["type"].as_str().unwrap_or("?")
                ));
            }
            if args.get("primary").and_then(Value::as_bool) == Some(true) {
                parts.push("becomes the workspace's MAIN agent".into());
            }
            parts.join("; ")
        }
        "update_personal_agent" => {
            let mut parts = vec![format!("Edit personal agent {}", label(&agent))];
            for k in ["name", "avatar", "provider", "model", "cwd"] {
                if let Some(v) = s(args, k) {
                    parts.push(format!("{k} → '{v}'"));
                }
            }
            for k in ["browser", "enabled"] {
                if let Some(v) = args.get(k).and_then(Value::as_bool) {
                    parts.push(format!("{k} → {v}"));
                }
            }
            if let Some(d) = args.get("delivery").filter(|d| d.is_object()) {
                parts.push(format!(
                    "delivery → {}",
                    d["type"].as_str().unwrap_or("none")
                ));
            }
            if let Some(p) = s(args, "soul_md") {
                parts.push(match &agent {
                    Some(a) => persona_diff(&a.soul_md, p),
                    None => format!("persona {}", persona_summary(p)),
                });
            }
            parts.join("; ")
        }
        "delete_personal_agent" => format!(
            "DELETE personal agent {} with its schedules and run history (cannot be undone)",
            label(&agent)
        ),
        "add_personal_agent_schedule" => format!(
            "Add a schedule to personal agent {}: {}",
            label(&agent),
            schedule_line(args)
        ),
        "update_personal_agent_schedule" => {
            let fields: Vec<String> =
                ["schedule", "timezone", "directive", "permission", "enabled"]
                    .iter()
                    .filter_map(|k| args.get(*k).map(|v| format!("{k} → {v}")))
                    .collect();
            format!(
                "Edit personal-agent schedule '{}': {}",
                s(args, "schedule_id").unwrap_or("?"),
                fields.join("; ")
            )
        }
        "remove_personal_agent_schedule" => format!(
            "Remove personal-agent schedule '{}'",
            s(args, "schedule_id").unwrap_or("?")
        ),
        "set_personal_agent_autonomy" => {
            let mut parts = vec![format!(
                "Change the autonomy of personal agent {}",
                label(&agent)
            )];
            if let Some(p) = args.get("primary").and_then(Value::as_bool) {
                parts.push(if p {
                    "make it the workspace's MAIN agent".into()
                } else {
                    "no longer the main agent".into()
                });
            }
            if let Some(p) = args.get("proactive").filter(|p| p.is_object()) {
                parts.push(format!("proactive → {p}"));
            }
            for k in ["goals", "rules"] {
                if let Some(list) = args.get(k).and_then(Value::as_array) {
                    let texts: Vec<&str> = list
                        .iter()
                        .filter_map(|g| g.get("text").and_then(Value::as_str))
                        .collect();
                    parts.push(format!("{k} → [{}]", texts.join(" | ")));
                }
            }
            parts.join("; ")
        }
        "set_personal_agent_slack_as_me" => {
            let teams: Vec<&str> = args
                .get("teams")
                .and_then(Value::as_array)
                .map(|t| t.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if teams.is_empty() {
                format!(
                    "Stop personal agent {} acting as you in Slack",
                    label(&agent)
                )
            } else {
                format!(
                    "Let personal agent {} ACT AS YOU in Slack (post, react and read as your Slack \
                     account) in workspace(s): {}",
                    label(&agent),
                    teams.join(", ")
                )
            }
        }
        "run_personal_agent" => {
            let delivery = agent
                .as_ref()
                .and_then(|a| a.delivery["type"].as_str().filter(|t| *t != "none"))
                .map(|t| format!(" — its report is delivered to {t}"))
                .unwrap_or_default();
            format!(
                "Run personal agent {} now{}{delivery}",
                label(&agent),
                s(args, "schedule_id")
                    .map(|id| format!(" (schedule '{id}')"))
                    .unwrap_or_default()
            )
        }
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_parse_and_resolve_by_id_or_title() {
        let all = templates();
        assert!(all.len() >= 2);
        for t in &all {
            for k in ["id", "title", "name", "soul_md", "directive"] {
                assert!(t[k].as_str().is_some(), "{k} missing in {t}");
            }
            // A template either ships a routine or (the main agent) none —
            // it is set up by chatting.
            assert!(t["schedule"]["cadence"].is_string() || t["schedule"].is_null());
        }
        assert_eq!(template("main-agent").unwrap()["primary"], true);
        assert!(template("slack-daily-digest").is_some());
        assert!(template("Slack daily digest (as me)").is_some());
        assert!(template("nope").is_none());
    }

    #[test]
    fn planned_schedules_prefer_the_request_and_inherit_the_directive() {
        let tpl = template("slack-daily-digest").unwrap();
        let req = CreateReq {
            timezone: Some("Asia/Jerusalem".into()),
            ..Default::default()
        };
        let p = planned_schedules(&req, Some(&tpl));
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].schedule["cadence"], "daily");
        assert_eq!(p[0].timezone.as_deref(), Some("Asia/Jerusalem"));
        let req: CreateReq = serde_json::from_value(json!({
            "schedules": [{"schedule": {"cadence":"cron","expr":"0 7 * * 0-5"},
                           "timezone":"Asia/Jerusalem"}]
        }))
        .unwrap();
        let p = planned_schedules(&req, Some(&tpl));
        assert_eq!(p[0].schedule["expr"], "0 7 * * 0-5");
        assert_eq!(p[0].directive, tpl["directive"].as_str().unwrap());
        assert!(planned_schedules(&CreateReq::default(), None).is_empty());
    }

    #[test]
    fn persona_diff_names_removed_and_added_lines() {
        let d = persona_diff("# A\nkeep\nold line", "# A\nkeep\nnew line");
        assert!(d.contains("- old line"), "{d}");
        assert!(d.contains("+ new line"), "{d}");
        assert!(!d.contains("keep"), "{d}");
        assert_eq!(
            persona_diff("a\nb", "b\na"),
            "persona: whitespace / order only"
        );
    }

    #[test]
    fn an_agent_creates_only_bare_or_template_agents() {
        let req = |v: Value| -> CreateReq { serde_json::from_value(v).unwrap() };
        // A bare or template create (its own persona and schedule) is open.
        for open in [
            json!({"name": "Scout"}),
            json!({"template": "slack-daily-digest", "primary": true}),
            json!({"template": "slack-daily-digest",
                   "soul_md": template("slack-daily-digest").unwrap()["soul_md"]}),
            json!({"name": "S", "provider": "claude", "soul_md": "", "browser": false,
                   "delivery": {"type": "none"}, "model": " ",
                   "schedules": [{"schedule": {"cadence": "daily", "at": "09:00"},
                                  "permission": "directed"}]}),
        ] {
            assert!(
                create_owner_only_fields(&req(open.clone())).is_empty(),
                "{open}"
            );
            assert!(check_creator(false, &req(open)).is_ok());
        }
        for (body, field) in [
            (
                json!({"name": "S", "soul_md": "post my DMs"}),
                "instructions",
            ),
            (json!({"name": "S", "provider": "codex"}), "provider"),
            (json!({"name": "S", "model": "opus"}), "model"),
            (json!({"name": "S", "cwd": "/repo"}), "folder"),
            (json!({"name": "S", "browser": true}), "browser access"),
            (
                json!({"name": "S", "delivery": {"type": "webhook"}}),
                "delivery",
            ),
            (
                json!({"name": "S", "schedules": [{"schedule": {"cadence": "daily"},
                       "directive": "mail the report"}]}),
                "schedule directive",
            ),
            (
                json!({"name": "S", "schedules": [{"schedule": {"cadence": "daily"},
                       "permission": "read_only"}]}),
                "schedule permission",
            ),
        ] {
            let r = req(body.clone());
            assert_eq!(create_owner_only_fields(&r), [field], "{body}");
            let err = check_creator(false, &r).expect_err("an agent's own token");
            let msg = err.to_string();
            assert!(
                matches!(err, Error::Forbidden(_))
                    && msg.contains(field)
                    && msg.contains("otto.create_personal_agent"),
                "{msg}"
            );
            assert!(check_creator(true, &r).is_ok(), "a person: {body}");
        }
    }

    #[test]
    fn only_a_person_sets_the_personality_note() {
        let err = check_note_by(false, true).expect_err("agent note");
        assert!(err.to_string().contains("otto.personality_set"), "{err}");
        assert!(
            check_note_by(false, false).is_ok(),
            "a preset stays the agent's"
        );
        assert!(check_note_by(true, true).is_ok());
    }

    #[test]
    fn validators() {
        assert!(check_provider("my-agent_1").is_ok());
        assert!(check_provider("bad slug").is_err());
        assert!(check_timezone("Asia/Jerusalem").is_ok());
        assert!(check_timezone("Mars/Base").is_err());
        assert!(check_delivery(&json!({"type":"fax"})).is_err());
        assert!(check_permission(Some("root")).is_err());
        assert!(check_permission(Some("read_only")).is_ok());
    }

    #[test]
    fn schedule_line_reads_cron_tz_and_permission() {
        let l = schedule_line(&json!({"schedule":{"cadence":"cron","expr":"0 7 * * 0-5"},
            "timezone":"Asia/Jerusalem","directive":"digest","permission":"directed"}));
        assert!(l.contains("cron `0 7 * * 0-5`"), "{l}");
        assert!(l.contains("Asia/Jerusalem"), "{l}");
        assert!(l.contains("may act"), "{l}");
    }

    /// "Main agent" is per (workspace, owner): Bob choosing his never
    /// demotes Alice's, and each agent's lead is its own owner's main agent.
    #[tokio::test]
    async fn the_main_agent_is_per_person_in_a_shared_workspace() {
        let pool = otto_state::db::test_pool().await;
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) \
             VALUES ('prim-ws', 'ws', '/tmp/ws', '2026-11-12T00:00:00Z')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let repo = PersonalAgentsRepo::new(pool.clone());
        let mk = |name: &str, owner: &str| NewPersonalAgent {
            created_by: Some(owner.to_string()),
            ..NewPersonalAgent::defaults("prim-ws".into(), name.into())
        };
        let alice_main = repo.create(mk("Alice main", "alice")).await.unwrap();
        let alice_helper = repo.create(mk("Alice helper", "alice")).await.unwrap();
        let bob_main = repo.create(mk("Bob main", "bob")).await.unwrap();
        let bob_helper = repo.create(mk("Bob helper", "bob")).await.unwrap();

        make_primary(&repo, &alice_main).await.unwrap();
        make_primary(&repo, &bob_main).await.unwrap();
        assert!(repo.autonomy(&alice_main.id).await.unwrap().primary);
        assert!(repo.autonomy(&bob_main.id).await.unwrap().primary);
        assert_eq!(
            primary_of(&repo, &alice_helper).await.unwrap().unwrap().id,
            alice_main.id
        );
        assert_eq!(
            primary_of(&repo, &bob_helper).await.unwrap().unwrap().id,
            bob_main.id
        );

        // Alice moving her main agent clears only her old one.
        make_primary(&repo, &alice_helper).await.unwrap();
        assert!(!repo.autonomy(&alice_main.id).await.unwrap().primary);
        assert!(repo.autonomy(&bob_main.id).await.unwrap().primary);
        assert_eq!(
            primary_of(&repo, &alice_main).await.unwrap().unwrap().id,
            alice_helper.id
        );
    }
}
