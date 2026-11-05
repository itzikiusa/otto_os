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
        .unwrap_or_else(|| "UTC".into());
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

/// Make `agent` the workspace's main agent (one per workspace: every other
/// agent's flag is cleared).
pub async fn make_primary(repo: &PersonalAgentsRepo, agent: &PersonalAgent) -> Result<()> {
    let mut cfg = repo.autonomy(&agent.id).await?;
    if !cfg.primary {
        cfg.primary = true;
        repo.save_autonomy(&agent.id, &cfg).await?;
    }
    clear_other_primaries(repo, agent).await
}

/// Clear `primary` on every other agent of `agent`'s workspace.
pub async fn clear_other_primaries(repo: &PersonalAgentsRepo, agent: &PersonalAgent) -> Result<()> {
    for other in repo.list_by_workspace(&agent.workspace_id).await? {
        if other.id == agent.id {
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
    if req.primary == Some(true) {
        make_primary(&repo, &agent).await?;
    }
    let mut out = serde_json::to_value(&agent).map_err(|e| Error::Internal(e.to_string()))?;
    out["schedules"] = json!(made);
    if let Some(p) = req.primary {
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
            assert!(t["schedule"]["cadence"].is_string());
        }
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
}
