//! Personal-agent **autonomy** routes (dots-style): permission modes, standing
//! goals + budget, custom rules, the primary assistant, the live activity
//! feed, the memory inspector and "reset agent". Merged into
//! [`super::routes`]; every handler loads the agent and checks the caller's
//! workspace role on ITS workspace (IDOR guard), like the parent module.

use axum::extract::{Path, Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};

use otto_core::domain::WorkspaceRole;
use otto_core::Error;
use otto_state::{AgentAutonomy, AgentRule, PersonalAgentsRepo, ProactiveConfig, StandingGoal};

use crate::auth::{require_ws_role, CurrentUser};
use crate::error::{ApiError, ApiResult};
use crate::personal_agent_memory;
use crate::personal_agents_engine;
use crate::state::ServerCtx;

/// Caps that keep the persona file and the per-call rule scan small.
const MAX_GOALS: usize = 20;
const MAX_RULES: usize = 30;
const MAX_TEXT_CHARS: usize = 500;

pub fn routes() -> Router<ServerCtx> {
    Router::new()
        .route(
            "/personal-agents/{id}/autonomy",
            get(read_autonomy).put(save_autonomy),
        )
        .route("/personal-agents/{id}/goals/{goal_id}/run", post(run_goal))
        .route("/personal-agents/{id}/activity", get(activity))
        .route("/personal-agents/{id}/memories", get(list_memories))
        .route("/personal-agents/{id}/memories/edit", post(edit_memory))
        .route("/personal-agents/{id}/reset", post(reset))
}

fn agents(ctx: &ServerCtx) -> PersonalAgentsRepo {
    PersonalAgentsRepo::new(ctx.pool.clone())
}

async fn load(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    id: &str,
    role: WorkspaceRole,
) -> ApiResult<otto_state::PersonalAgent> {
    let agent = agents(ctx).get(id).await.map_err(ApiError)?;
    require_ws_role(ctx, user, &agent.workspace_id, role).await?;
    Ok(agent)
}

// --- Autonomy ----------------------------------------------------------------

/// `GET /personal-agents/{id}/autonomy`
async fn read_autonomy(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<AgentAutonomy>> {
    load(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    agents(&ctx).autonomy(&id).await.map(Json).map_err(ApiError)
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct GoalIn {
    id: Option<String>,
    text: String,
    enabled: Option<bool>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RuleIn {
    id: Option<String>,
    text: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SaveAutonomyReq {
    proactive: Option<ProactiveConfig>,
    goals: Option<Vec<GoalIn>>,
    rules: Option<Vec<RuleIn>>,
    primary: Option<bool>,
}

fn clean_text(raw: &str, what: &str) -> Result<String, ApiError> {
    let t = raw.trim();
    if t.chars().count() > MAX_TEXT_CHARS {
        return Err(ApiError(Error::Invalid(format!(
            "{what} is longer than {MAX_TEXT_CHARS} characters"
        ))));
    }
    Ok(t.to_string())
}

/// Merge a save request into the stored config. Server-owned fields are never
/// taken from the client: a goal's `last_run_at` is kept by id, a rule's
/// `enforce` is re-derived from its text. Pure — unit-tested.
fn merge_autonomy(mut cfg: AgentAutonomy, req: SaveAutonomyReq) -> Result<AgentAutonomy, ApiError> {
    if let Some(p) = req.proactive {
        cfg.proactive = ProactiveConfig {
            enabled: p.enabled,
            runs_per_day: p.runs_per_day.clamp(1, 24),
            max_minutes: p.max_minutes.clamp(1, 60),
        };
    }
    if let Some(goals) = req.goals {
        if goals.len() > MAX_GOALS {
            return Err(ApiError(Error::Invalid(format!(
                "at most {MAX_GOALS} standing goals"
            ))));
        }
        let mut next = Vec::new();
        for g in goals {
            let text = clean_text(&g.text, "a goal")?;
            if text.is_empty() {
                continue;
            }
            let id =
                g.id.filter(|i| !i.trim().is_empty())
                    .unwrap_or_else(otto_core::new_id);
            let last_run_at = cfg
                .goals
                .iter()
                .find(|o| o.id == id)
                .and_then(|o| o.last_run_at.clone());
            next.push(StandingGoal {
                id,
                text,
                enabled: g.enabled.unwrap_or(true),
                last_run_at,
            });
        }
        cfg.goals = next;
    }
    if let Some(rules) = req.rules {
        if rules.len() > MAX_RULES {
            return Err(ApiError(Error::Invalid(format!(
                "at most {MAX_RULES} rules"
            ))));
        }
        let mut next = Vec::new();
        for r in rules {
            let text = clean_text(&r.text, "a rule")?;
            if text.is_empty() {
                continue;
            }
            next.push(AgentRule {
                id: r
                    .id
                    .filter(|i| !i.trim().is_empty())
                    .unwrap_or_else(otto_core::new_id),
                enforce: crate::personal_agent_policy::derive_enforcement(&text),
                text,
            });
        }
        cfg.rules = next;
    }
    if let Some(primary) = req.primary {
        cfg.primary = primary;
    }
    Ok(cfg)
}

/// `PUT /personal-agents/{id}/autonomy` — partial: omitted sections are kept.
/// Making an agent primary clears the flag on every other agent of the
/// workspace (one "your agent" per workspace).
async fn save_autonomy(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<SaveAutonomyReq>,
) -> ApiResult<Json<AgentAutonomy>> {
    let agent = load(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let repo = agents(&ctx);
    let current = repo.autonomy(&id).await.map_err(ApiError)?;
    let becomes_primary = req.primary == Some(true) && !current.primary;
    let next = merge_autonomy(current, req)?;
    repo.save_autonomy(&id, &next).await.map_err(ApiError)?;
    if becomes_primary {
        for other in repo
            .list_by_workspace(&agent.workspace_id)
            .await
            .map_err(ApiError)?
        {
            if other.id == id {
                continue;
            }
            let mut cfg = repo.autonomy(&other.id).await.map_err(ApiError)?;
            if cfg.primary {
                cfg.primary = false;
                repo.save_autonomy(&other.id, &cfg)
                    .await
                    .map_err(ApiError)?;
            }
        }
    }
    Ok(Json(next))
}

/// `POST /personal-agents/{id}/goals/{goal_id}/run` — work on a standing goal
/// now (a proactive, read-only run; counts against the daily budget).
async fn run_goal(
    Path((id, goal_id)): Path<(String, String)>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<otto_state::PersonalAgentRun>> {
    let agent = load(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    personal_agents_engine::spawn_proactive_run(&ctx, &agent, &goal_id)
        .await
        .map(Json)
        .map_err(ApiError)
}

// --- Activity ----------------------------------------------------------------

/// `GET /personal-agents/{id}/activity` query (perf W4).
#[derive(Deserialize, Default)]
struct ActivityQuery {
    /// Only ring entries with `seq > after_seq` (the client appends them).
    #[serde(default)]
    after_seq: Option<u64>,
    /// Include `runs` (recent history). Default true; the client asks only
    /// when a run changed.
    #[serde(default)]
    runs: Option<bool>,
}

/// Longest run `summary` the activity feed carries (the run page has it all).
const ACTIVITY_SUMMARY_CLIP: usize = 280;

/// `GET /personal-agents/{id}/activity` — what the agent is doing now (the
/// running run + its session's live status), its recent tool calls (allowed /
/// blocked / needing approval), the approvals it is waiting on (with their
/// current status), and recent run history.
///
/// Incremental (perf W4): `?after_seq=N` returns only newer `items`, and
/// `?runs=false` leaves `runs` out (`null`). `seq` is the cursor for the next
/// call. Approvals are read in one statement; run summaries are clipped.
async fn activity(
    Path(id): Path<String>,
    Query(q): Query<ActivityQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    load(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let (current, runs) = if q.runs.unwrap_or(true) {
        let mut runs = agents(&ctx).list_runs(&id, 20).await.map_err(ApiError)?;
        for r in &mut runs {
            clip_in_place(&mut r.summary, ACTIVITY_SUMMARY_CLIP);
        }
        (
            runs.iter().find(|r| r.status == "running").cloned(),
            Some(runs),
        )
    } else {
        let mut cur = agents(&ctx).running_run(&id).await.map_err(ApiError)?;
        if let Some(r) = cur.as_mut() {
            clip_in_place(&mut r.summary, ACTIVITY_SUMMARY_CLIP);
        }
        (cur, None)
    };
    let session_status = match current.as_ref().and_then(|r| r.session_id.clone()) {
        Some(sid) => otto_state::SessionsRepo::new(ctx.pool.clone())
            .get(&sid)
            .await
            .ok()
            .map(|s| json!(s.status)),
        None => None,
    };
    let items = match q.after_seq {
        Some(after) => crate::personal_agent_activity::recent_after(&id, after, 100),
        None => crate::personal_agent_activity::recent(&id, 100),
    };
    // Every waiting approval still in the ring (not just the new items), in
    // one statement — their statuses change independently of the ring.
    let waiting = crate::personal_agent_activity::waiting_approval_ids(&id);
    let waiting_items = crate::personal_agent_activity::recent(&id, 200);
    let rows = ctx
        .mcp
        .approvals()
        .get_many(&waiting)
        .await
        .unwrap_or_default();
    let mut approvals = Vec::new();
    for aid in &waiting {
        let Some(a) = rows.iter().find(|a| &a.id == aid) else {
            continue;
        };
        let Some(item) = waiting_items
            .iter()
            .find(|i| i.approval_id.as_deref() == Some(aid.as_str()))
        else {
            continue;
        };
        approvals.push(json!({
            "approval_id": aid,
            "tool": item.tool,
            "at": item.at,
            "status": a.status,
            "title": a.title,
            "detail": a.detail,
            "risk_label": a.risk_label,
        }));
    }
    let seq = items
        .first()
        .map(|i| i.seq)
        .unwrap_or(q.after_seq.unwrap_or(0));
    Ok(Json(json!({
        "now": { "run": current, "session_status": session_status },
        "items": items,
        "approvals": approvals,
        "runs": runs,
        "seq": seq,
    })))
}

/// Truncate `s` to at most `max` bytes on a char boundary, adding `…`.
fn clip_in_place(s: &mut String, max: usize) {
    if s.len() <= max {
        return;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push('…');
}

// --- Memory inspector ----------------------------------------------------------

/// `GET /personal-agents/{id}/memories` — the memory file split into items
/// (with source + section) plus the document version edits must quote.
async fn list_memories(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &user, &id, WorkspaceRole::Viewer).await?;
    let root = personal_agents_engine::agent_directory(&ctx, &agent).map_err(ApiError)?;
    let doc = crate::personal_agent_documents::read_memory(&root)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({
        "version": doc.version,
        "exists": doc.exists,
        "path": doc.path,
        "items": personal_agent_memory::parse_items(&doc.content),
    })))
}

#[derive(Deserialize)]
struct EditMemoryReq {
    version: String,
    line: usize,
    /// The raw line the client saw (`MemoryItem.raw`).
    raw: String,
    /// New text, or `null` to forget the item.
    #[serde(default)]
    text: Option<String>,
}

/// `POST /personal-agents/{id}/memories/edit` — edit or forget one item
/// (version- and content-checked; 409 when the agent changed it meanwhile).
async fn edit_memory(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<EditMemoryReq>,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    let root = personal_agents_engine::agent_directory(&ctx, &agent).map_err(ApiError)?;
    let doc = crate::personal_agent_documents::read_memory(&root)
        .await
        .map_err(ApiError)?;
    if doc.version != req.version {
        return Err(ApiError(Error::Conflict(
            "the agent's memory changed since you opened it — reload and try again".into(),
        )));
    }
    let next =
        personal_agent_memory::edit_item(&doc.content, req.line, &req.raw, req.text.as_deref())
            .map_err(ApiError)?;
    let saved = crate::personal_agent_documents::save_memory(&root, &req.version, &next)
        .await
        .map_err(ApiError)?;
    Ok(Json(json!({
        "version": saved.version,
        "exists": saved.exists,
        "path": saved.path,
        "items": personal_agent_memory::parse_items(&saved.content),
    })))
}

// --- Reset -----------------------------------------------------------------------

#[derive(Deserialize)]
struct ResetReq {
    /// Must equal the agent's name — the server-side half of the confirm.
    confirm: String,
}

/// `POST /personal-agents/{id}/reset` — forget everything the agent learned
/// and did: its memory file is re-seeded, its chat session is stopped and
/// unpinned, its schedules and run history (+ report files) are deleted, its
/// goals' cursors and live activity are cleared. Persona, rules, goals, model
/// and delivery are kept. Refused while a run is in progress.
async fn reset(
    Path(id): Path<String>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Json(req): Json<ResetReq>,
) -> ApiResult<Json<Value>> {
    let agent = load(&ctx, &user, &id, WorkspaceRole::Editor).await?;
    if req.confirm.trim() != agent.name.trim() {
        return Err(ApiError(Error::Invalid(
            "type the agent's name to confirm the reset".into(),
        )));
    }
    let Some(_guard) = personal_agents_engine::claim_idle(&agent.id) else {
        return Err(ApiError(Error::Conflict(
            "a run of this agent is in progress — stop it first".into(),
        )));
    };
    let repo = agents(&ctx);
    if let Some(sid) = agent.chat_session_id.as_deref().filter(|s| !s.is_empty()) {
        let _ = ctx.manager.kill_session(&sid.to_string()).await;
    }
    let reports = repo.reset_agent(&id).await.map_err(ApiError)?;
    for p in reports {
        let _ = tokio::fs::remove_file(&p).await;
    }
    let root = personal_agents_engine::agent_directory(&ctx, &agent).map_err(ApiError)?;
    let doc = crate::personal_agent_documents::read_memory(&root)
        .await
        .map_err(ApiError)?;
    crate::personal_agent_documents::save_memory(
        &root,
        &doc.version,
        &personal_agents_engine::seed_notes(&agent.name),
    )
    .await
    .map_err(ApiError)?;
    let mut cfg = repo.autonomy(&id).await.map_err(ApiError)?;
    for g in &mut cfg.goals {
        g.last_run_at = None;
    }
    repo.save_autonomy(&id, &cfg).await.map_err(ApiError)?;
    crate::personal_agent_activity::clear(&id);
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_server_fields_and_derives_enforcement() {
        let mut cur = AgentAutonomy::default();
        cur.goals.push(StandingGoal {
            id: "g1".into(),
            text: "old".into(),
            enabled: true,
            last_run_at: Some("2026-10-01T00:00:00Z".into()),
        });
        let req = SaveAutonomyReq {
            proactive: Some(ProactiveConfig {
                enabled: true,
                runs_per_day: 99,
                max_minutes: 0,
            }),
            goals: Some(vec![
                GoalIn {
                    id: Some("g1".into()),
                    text: "  Watch CI  ".into(),
                    enabled: None,
                },
                GoalIn {
                    id: None,
                    text: "   ".into(),
                    enabled: None,
                },
                GoalIn {
                    id: None,
                    text: "Track releases".into(),
                    enabled: Some(false),
                },
            ]),
            rules: Some(vec![RuleIn {
                id: None,
                text: "Ask before touching prod".into(),
            }]),
            primary: Some(true),
        };
        let next = merge_autonomy(cur, req).unwrap();
        assert_eq!(next.proactive.runs_per_day, 24);
        assert_eq!(next.proactive.max_minutes, 1);
        assert_eq!(next.goals.len(), 2);
        assert_eq!(next.goals[0].text, "Watch CI");
        assert_eq!(
            next.goals[0].last_run_at.as_deref(),
            Some("2026-10-01T00:00:00Z")
        );
        assert!(!next.goals[1].id.is_empty());
        assert!(!next.goals[1].enabled);
        let enf = next.rules[0].enforce.as_ref().unwrap();
        assert_eq!(enf.kind, "approval");
        assert!(next.primary);
    }

    #[test]
    fn merge_enforces_caps_and_keeps_omitted_sections() {
        let mut cur = AgentAutonomy::default();
        cur.rules.push(AgentRule {
            id: "r".into(),
            text: "Be brief".into(),
            enforce: None,
        });
        let next = merge_autonomy(cur.clone(), SaveAutonomyReq::default()).unwrap();
        assert_eq!(next, cur);
        let too_many = SaveAutonomyReq {
            goals: Some(
                (0..=MAX_GOALS)
                    .map(|i| GoalIn {
                        id: None,
                        text: format!("g{i}"),
                        enabled: None,
                    })
                    .collect(),
            ),
            ..Default::default()
        };
        assert!(merge_autonomy(cur.clone(), too_many).is_err());
        let long = SaveAutonomyReq {
            rules: Some(vec![RuleIn {
                id: None,
                text: "x".repeat(MAX_TEXT_CHARS + 1),
            }]),
            ..Default::default()
        };
        assert!(merge_autonomy(cur, long).is_err());
    }
}
