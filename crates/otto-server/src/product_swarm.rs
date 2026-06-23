//! Product → Swarm bridge: turn a refined Product story into a runnable swarm
//! project. The flagship cross-feature path — Product owns the AI plan/tasks
//! breakdown, Swarm owns the role-agent runtime; this is where they meet.
//!
//! `POST /api/v1/product/stories/{sid}/to-swarm` (ws editor):
//!   1. Resolve the story + role-check via its workspace.
//!   2. Idempotent: if a project already links back to this story, return it.
//!   3. Resolve the target swarm: request `swarm_id` (verified same-workspace),
//!      else the workspace's first swarm, else auto-create a default swarm.
//!   4. `goal_md` = the story's refined body (latest `suggested` version → its
//!      `source` → the title) so the project carries the picture forward.
//!   5. Seed tasks: reuse the story's existing `kind="plan"` version by parsing
//!      its `### Task N:` markdown; if there is no plan, fall back to the swarm
//!      planner (run_agent over the goal) to generate one.
//!   6. Create the project with the `story_id` back-link + seed the tasks.
//!
//! The route is registered in `orchestrator_routes()` (otto-server) because it
//! needs the product repo, the swarm repo, and the orchestrator together.

use std::time::Duration;

use axum::extract::{Path, State};
use axum::Json;
use otto_core::domain::WorkspaceRole;
use otto_core::{Error, Id};
// `NewTask` is qualified via `swarm::` because the otto-state root also
// re-exports an unrelated `activity::NewTask` (agent task tracker).
use otto_state::swarm::{NewTask, RunFilter};
use otto_state::{
    DiscoveryRun, NewDiscoveryRun, ProductAttachment, ProductStory, NewProject, Swarm,
    SwarmMessage, SwarmProject, SwarmTask,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tracing::warn;

use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::ServerCtx;

/// Request body for `POST /product/stories/{sid}/to-swarm`.
#[derive(Debug, Default, Deserialize)]
pub struct ToSwarmReq {
    /// Target swarm. When omitted, the workspace's first swarm is used, or a
    /// default swarm is auto-created if the workspace has none.
    #[serde(default)]
    pub swarm_id: Option<Id>,
    /// Override the new project's name (defaults to the story title).
    #[serde(default)]
    pub name: Option<String>,
}

/// Response: the swarm + the (possibly newly-created) project + its seeded tasks,
/// so the UI can navigate straight to the project's Kanban board.
#[derive(Debug, Serialize)]
pub struct ToSwarmResp {
    pub swarm: Swarm,
    pub project: SwarmProject,
    pub tasks: Vec<SwarmTask>,
    /// True when this call created the project; false when it returned a
    /// pre-existing linked project (idempotent re-send).
    pub created: bool,
}

/// One parsed task from a plan markdown body. `### Task N: <title>` heading →
/// task; the checkbox/Goal/Verify lines beneath it (until the next heading)
/// become the task description.
struct ParsedTask {
    title: String,
    description: String,
}

/// Parse the implementation-plan markdown produced by the `story-task-breakdown`
/// skill into seedable tasks. Mirrors `ui/src/modules/product/plan_parse.ts`:
/// any `##`–`####` ATX heading starts a task; everything up to the next heading
/// is its body. Lines before the first heading are ignored. The leading
/// `Task N: ` prefix is stripped from the title for a cleaner Kanban card.
fn parse_plan_tasks(md: &str) -> Vec<ParsedTask> {
    let mut tasks: Vec<ParsedTask> = Vec::new();
    let mut body = String::new();
    for line in md.lines() {
        if let Some(title) = heading_title(line) {
            if let Some(last) = tasks.last_mut() {
                last.description = body.trim().to_string();
            }
            body.clear();
            tasks.push(ParsedTask {
                title: strip_task_prefix(&title),
                description: String::new(),
            });
        } else if !tasks.is_empty() {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(last) = tasks.last_mut() {
        last.description = body.trim().to_string();
    }
    tasks.into_iter().filter(|t| !t.title.is_empty()).collect()
}

/// Return the heading text for a 2–4 level ATX heading line, else `None`.
fn heading_title(line: &str) -> Option<String> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (2..=4).contains(&hashes) {
        let rest = line[hashes..].trim();
        if !rest.is_empty() && line.as_bytes().get(hashes) == Some(&b' ') {
            return Some(rest.to_string());
        }
    }
    None
}

/// Strip a leading `Task N:` / `Task N -` prefix from a heading title.
fn strip_task_prefix(title: &str) -> String {
    let lower = title.to_ascii_lowercase();
    if let Some(rest) = lower.strip_prefix("task ") {
        // Skip the number, then a `:` / `-` / `.` separator.
        let digits = rest.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 {
            let after = title["task ".len() + digits..].trim_start();
            let after = after
                .trim_start_matches([':', '-', '.', ')'])
                .trim_start();
            if !after.is_empty() {
                return after.to_string();
            }
        }
    }
    title.to_string()
}

/// Build the project goal from the story's most refined content: prefer the
/// latest `suggested` (rewrite) body, then the `source` body, then the title.
async fn build_goal_md(ctx: &ServerCtx, story: &ProductStory) -> String {
    if let Ok(Some(v)) = ctx
        .product_repo
        .latest_version_of_kind(&story.id, "suggested")
        .await
    {
        if !v.body_md.trim().is_empty() {
            return v.body_md.trim().to_string();
        }
    }
    if let Ok(Some(v)) = ctx.product_repo.latest_source_version(&story.id).await {
        if !v.body_md.trim().is_empty() {
            return v.body_md.trim().to_string();
        }
    }
    story.title.clone()
}

/// Resolve the target swarm: explicit request id (verified same-workspace),
/// else the workspace's first swarm, else auto-create a paused default swarm.
async fn resolve_swarm(
    ctx: &ServerCtx,
    workspace_id: &Id,
    user_id: &Id,
    requested: Option<Id>,
) -> ApiResult<Swarm> {
    if let Some(sid) = requested {
        let swarm = ctx.swarm_repo.get_swarm(&sid).await.map_err(ApiError)?;
        if &swarm.workspace_id != workspace_id {
            return Err(ApiError(Error::NotFound(
                "swarm not found in workspace".into(),
            )));
        }
        return Ok(swarm);
    }
    let existing = ctx
        .swarm_repo
        .list_swarms(workspace_id)
        .await
        .map_err(ApiError)?;
    if let Some(first) = existing.into_iter().next() {
        return Ok(first);
    }
    // No swarm yet — auto-create a default one (paused; the user starts it).
    ctx.swarm_repo
        .create_swarm(otto_state::NewSwarm {
            workspace_id: workspace_id.clone(),
            name: "Default Swarm".into(),
            description: "Auto-created for Product → Swarm".into(),
            preset_slug: None,
            config: json!({}),
            // Budget guardrails left unset (unlimited) on an auto-created default
            // swarm; the user can set limits before starting it.
            max_total_runs: None,
            max_cost_usd: None,
            max_runtime_secs: None,
            max_attempts: None,
            created_by: user_id.clone(),
        })
        .await
        .map_err(ApiError)
}

/// Seed tasks for the new project. Reuse the story's existing plan markdown when
/// present; otherwise run the swarm planner over the goal to generate tasks.
/// Returns the list of created tasks (already persisted).
pub(crate) async fn seed_tasks(
    ctx: &ServerCtx,
    project: &SwarmProject,
    user_id: &Id,
    goal_md: &str,
) -> Vec<SwarmTask> {
    // 1. Reuse an existing plan if the story has one.
    if let Some(story_id) = &project.story_id {
        if let Ok(Some(plan)) = ctx.product_repo.latest_plan_version(story_id).await {
            let parsed = parse_plan_tasks(&plan.body_md);
            if !parsed.is_empty() {
                return create_tasks(ctx, project, user_id, parsed).await;
            }
        }
    }

    // 2. No usable plan — fall back to the swarm planner over the goal.
    let agents = ctx
        .swarm_repo
        .list_agents(&project.swarm_id)
        .await
        .unwrap_or_default();
    let preset_agents: Vec<otto_swarm::PresetAgent> = agents
        .iter()
        .map(|a| otto_swarm::PresetAgent {
            key: a.id.clone(),
            name: a.name.clone(),
            title: a.title.clone(),
            reports_to: None,
            provider: a.provider.clone(),
            specialization: a.specialization.clone(),
        })
        .collect();
    let prompt = otto_swarm::recruiter::planner_prompt(&project.name, goal_md, &preset_agents, "");
    let cwd = project
        .repo_path
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());
    // Planning is structured-extraction work — run it on the fast/cheap model
    // rather than the default, matching the other planner/recruiter paths.
    let reply = match ctx
        .orchestrator
        .run_agent(&prompt, &cwd, Some("haiku"), Duration::from_secs(150))
        .await
    {
        Ok(r) => r,
        Err(e) => {
            warn!("product_swarm: planner run_agent failed: {e}");
            return Vec::new();
        }
    };
    let Some(v) = otto_swarm::recruiter::extract_json(&reply) else {
        warn!("product_swarm: planner returned no parseable JSON");
        return Vec::new();
    };
    let tasks_json = v
        .get("tasks")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();
    let parsed: Vec<ParsedTask> = tasks_json
        .iter()
        .filter_map(|t| {
            let title = t.get("title").and_then(|v| v.as_str())?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let description = t
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ParsedTask { title, description })
        })
        .collect();
    create_tasks(ctx, project, user_id, parsed).await
}

/// Persist a list of parsed tasks as `todo` Kanban cards in `project`.
async fn create_tasks(
    ctx: &ServerCtx,
    project: &SwarmProject,
    user_id: &Id,
    parsed: Vec<ParsedTask>,
) -> Vec<SwarmTask> {
    let mut created = Vec::with_capacity(parsed.len());
    for (i, t) in parsed.into_iter().enumerate() {
        match ctx
            .swarm_repo
            .create_task(NewTask {
                project_id: project.id.clone(),
                swarm_id: project.swarm_id.clone(),
                workspace_id: project.workspace_id.clone(),
                title: t.title,
                description: t.description,
                assignee_agent_id: None,
                status: "todo".into(),
                priority: "medium".into(),
                parent_task_id: None,
                depends_on: json!([]),
                labels: json!(["product"]),
                order_idx: i as i64,
                created_by: user_id.clone(),
            })
            .await
        {
            Ok(task) => {
                let _ = ctx.events.send(otto_core::event::Event::SwarmTaskUpdated {
                    workspace_id: project.workspace_id.clone(),
                    swarm_id: project.swarm_id.clone(),
                    project_id: project.id.clone(),
                    task: serde_json::to_value(&task).unwrap_or(json!({})),
                });
                created.push(task);
            }
            Err(e) => warn!("product_swarm: create_task: {e}"),
        }
    }
    created
}

/// `POST /api/v1/product/stories/{sid}/to-swarm` — create a swarm project from a
/// Product story and seed its tasks from the story's plan. See module docs.
pub async fn story_to_swarm(
    Path(sid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<ToSwarmReq>>,
) -> ApiResult<Json<ToSwarmResp>> {
    let req = body.map(|b| b.0).unwrap_or_default();

    // 1. Resolve the story + role-check via its workspace.
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &story.workspace_id, WorkspaceRole::Editor).await?;

    // 2. Idempotent: a project already linked to this story → return it as-is.
    if let Ok(Some(project)) = ctx.swarm_repo.project_for_story(&story.id).await {
        let swarm = ctx
            .swarm_repo
            .get_swarm(&project.swarm_id)
            .await
            .map_err(ApiError)?;
        let tasks = ctx
            .swarm_repo
            .list_tasks(&project.id)
            .await
            .map_err(ApiError)?;
        return Ok(Json(ToSwarmResp {
            swarm,
            project,
            tasks,
            created: false,
        }));
    }

    // 3. Resolve target swarm (request → first → auto-create default).
    let swarm = resolve_swarm(&ctx, &story.workspace_id, &user.id, req.swarm_id).await?;

    // 4. Build goal_md from the story's refined content.
    let goal_md = build_goal_md(&ctx, &story).await;
    let name = req
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| story.title.clone());

    // 5. Create the project with the story back-link.
    let order_idx = ctx
        .swarm_repo
        .list_projects(&swarm.id)
        .await
        .map(|p| p.len() as i64)
        .unwrap_or(0);
    let project = match ctx
        .swarm_repo
        .create_project(NewProject {
            swarm_id: swarm.id.clone(),
            workspace_id: story.workspace_id.clone(),
            name,
            description: format!("From Product story {}", story.source_key),
            repo_path: story.cwd.clone(),
            goal_md: Some(goal_md.clone()),
            story_id: Some(story.id.clone()),
            order_idx,
            created_by: user.id.clone(),
        })
        .await
    {
        Ok(project) => project,
        // A concurrent hand-off won the race (unique `story_id`, migration 0037):
        // stay idempotent by returning that already-linked project as-is.
        Err(Error::Conflict(_)) => {
            if let Ok(Some(existing)) = ctx.swarm_repo.project_for_story(&story.id).await {
                let swarm = ctx
                    .swarm_repo
                    .get_swarm(&existing.swarm_id)
                    .await
                    .map_err(ApiError)?;
                let tasks = ctx
                    .swarm_repo
                    .list_tasks(&existing.id)
                    .await
                    .map_err(ApiError)?;
                return Ok(Json(ToSwarmResp {
                    swarm,
                    project: existing,
                    tasks,
                    created: false,
                }));
            }
            return Err(ApiError(Error::Conflict(
                "a swarm project is already linked to this story".into(),
            )));
        }
        Err(e) => return Err(ApiError(e)),
    };

    // 6. Seed tasks (reuse plan, else generate via planner).
    let tasks = seed_tasks(&ctx, &project, &user.id, &goal_md).await;

    // 7. Record a Product event so the story's history shows the hand-off.
    let _ = ctx
        .product_repo
        .add_event(otto_state::NewEvent {
            story_id: story.id.clone(),
            section: "plan".into(),
            kind: "sent_to_swarm".into(),
            summary: format!(
                "Sent to swarm “{}” as project “{}” ({} task(s))",
                swarm.name,
                project.name,
                tasks.len()
            ),
            actor_id: Some(user.id.clone()),
            meta_json: Some(
                json!({ "swarm_id": swarm.id, "project_id": project.id }).to_string(),
            ),
        })
        .await;

    Ok(Json(ToSwarmResp {
        swarm,
        project,
        tasks,
        created: true,
    }))
}

// ===========================================================================
// Discovery swarm — launch a repeatable INVESTIGATION swarm from a story.
//
// Distinct from the Product → Swarm hand-off above: discovery runs *before*
// implementation. It creates a non-story-linked swarm project (so it is
// repeatable — the unique `swarm_projects(story_id)` index is reserved for the
// single implementation project), seeds investigation tasks, records a
// `product_discovery_runs` row, and AUTO-STARTS the swarm so the discovery
// agents actually run (unlike `to-swarm`, which leaves the swarm paused).
//
//   POST /api/v1/product/stories/{sid}/discover        (ws editor) → DiscoverResp
//   GET  /api/v1/product/stories/{sid}/discovery-runs  (ws viewer) → Vec<DiscoveryRunSummary>
//   GET  /api/v1/product/discovery-runs/{rid}          (ws viewer) → DiscoveryRunDetail
// ===========================================================================

/// Request body for `POST /product/stories/{sid}/discover`.
#[derive(Debug, Default, Deserialize)]
pub struct DiscoverReq {
    /// Target swarm. When omitted, the workspace's first swarm is used, or a
    /// default swarm is auto-created if the workspace has none.
    #[serde(default)]
    pub swarm_id: Option<Id>,
    /// Override the new discovery project's name.
    #[serde(default)]
    pub name: Option<String>,
}

/// Response from launching discovery: the run row + the (auto-started) swarm +
/// the new discovery project + its seeded investigation tasks, so the UI can
/// navigate to the run and/or deep-link into the Kanban board.
#[derive(Debug, Serialize)]
pub struct DiscoverResp {
    pub run: DiscoveryRun,
    pub swarm: Swarm,
    pub project: SwarmProject,
    pub tasks: Vec<SwarmTask>,
}

/// One discovery run for the story's list view: the persisted row plus the
/// status derived from the discovery project's task statuses and a done/total
/// progress count.
#[derive(Debug, Serialize)]
pub struct DiscoveryRunSummary {
    pub run: DiscoveryRun,
    pub derived_status: String,
    pub task_count: i64,
    pub done_count: i64,
}

/// Full detail for one discovery run: the run, its derived status, the discovery
/// project's tasks, the latest run `summary` per task, and the discovery board
/// messages (`kind == "discovery"`). `report_md` lives on `run`.
#[derive(Debug, Serialize)]
pub struct DiscoveryRunDetail {
    pub run: DiscoveryRun,
    pub derived_status: String,
    pub tasks: Vec<SwarmTask>,
    /// `(task_id, latest run summary)` for each task — the live "findings so far".
    pub task_summaries: Vec<(Id, Option<String>)>,
    /// Board messages posted by agents with `kind == "discovery"`.
    pub messages: Vec<SwarmMessage>,
}

/// Derive the displayed status of a discovery run from its tasks (§6.3): if every
/// task is `done` the run is `"done"`; if any task errored, surface `"error"`;
/// otherwise fall back to the persisted column (`running`/`stopped`/…). An empty
/// task list keeps the persisted status (nothing has run yet).
fn derived_status(tasks: &[SwarmTask], persisted: &str) -> String {
    if tasks.iter().any(|t| t.status == "error") {
        return "error".to_string();
    }
    if !tasks.is_empty() && tasks.iter().all(|t| t.status == "done") {
        return "done".to_string();
    }
    persisted.to_string()
}

/// Build the discovery brief (§6.1): a DISCOVERY-framed superset of the goal
/// markdown. Sections: mission framing (investigate, do NOT implement), the
/// story header, the refined body, bounded context (analysis/notes/transcripts),
/// and the attachments listed by ABSOLUTE path (no files are copied at launch —
/// the agent opens them with its file tools; §6.4).
async fn build_discovery_brief(
    ctx: &ServerCtx,
    story: &ProductStory,
    atts: &[ProductAttachment],
) -> String {
    let mut s = String::new();

    // --- Mission framing ---------------------------------------------------
    s.push_str(DISCOVERY_MISSION_FRAMING);

    // --- Story header ------------------------------------------------------
    s.push_str("## Story\n");
    s.push_str(&format!("- **Title:** {}\n", story.title));
    s.push_str(&format!("- **Key:** {}\n", story.source_key));
    if !story.url.trim().is_empty() {
        s.push_str(&format!("- **URL:** {}\n", story.url));
    }
    if let Some(t) = &story.issue_type {
        s.push_str(&format!("- **Type:** {t}\n"));
    }
    s.push_str(&format!("- **Stage:** {}\n\n", story.stage));

    // --- Body (latest suggested → source → title) --------------------------
    s.push_str("## Body\n");
    s.push_str(&build_goal_md(ctx, story).await);
    s.push_str("\n\n");

    // --- Context (bounded) -------------------------------------------------
    let mut context = String::new();
    if let Ok(Some(v)) = ctx
        .product_repo
        .latest_version_of_kind(&story.id, "analysis")
        .await
    {
        let body = v.body_md.trim();
        if !body.is_empty() {
            context.push_str("### Analysis (latest)\n");
            context.push_str(&truncate_for_brief(body, 1200));
            context.push_str("\n\n");
        }
    }
    if let Ok(notes) = ctx.product_repo.list_notes(&story.id).await {
        let notes: Vec<&otto_state::ProductNote> =
            notes.iter().filter(|n| !n.body.trim().is_empty()).take(10).collect();
        if !notes.is_empty() {
            context.push_str("### Notes\n");
            for n in notes {
                context.push_str(&format!("- {}\n", truncate_for_brief(n.body.trim(), 200)));
            }
            context.push('\n');
        }
    }
    if let Ok(transcripts) = ctx.product_repo.list_transcripts(&story.id).await {
        let titles: Vec<&str> = transcripts
            .iter()
            .map(|t| t.title.trim())
            .filter(|t| !t.is_empty())
            .take(15)
            .collect();
        if !titles.is_empty() {
            context.push_str("### Transcripts\n");
            for t in titles {
                context.push_str(&format!("- {t}\n"));
            }
            context.push('\n');
        }
    }
    if !context.is_empty() {
        s.push_str("## Context\n");
        s.push_str(&context);
    }

    // --- Attachments (absolute paths; never copied) ------------------------
    s.push_str(&render_attachments_section(&ctx.data_dir, atts));

    s.trim_end().to_string()
}

/// Mission-framing header for the discovery brief (§6.1). DISCOVERY-before-
/// implementation framing + the expected-outputs list + the output contract.
const DISCOVERY_MISSION_FRAMING: &str = "# DISCOVERY\n\n\
     This is a **DISCOVERY** task that runs **before** implementation planning. \
     Do NOT write production code, do NOT open PRs. Investigate the story and \
     report your findings. For each task, your report should cover, where relevant:\n\
     - **Affected services / files** and the data flow involved\n\
     - **Dependencies & integration points** (APIs, contracts, schemas)\n\
     - **Risks & unknowns**\n\
     - **Open questions** for stakeholders\n\
     - **Prior art** / similar work already in the codebase\n\
     - A **recommended approach**\n\n\
     **Output contract:** post your findings to the board with `./otto-post` using \
     `kind: discovery`, and publish the consolidated discovery report with \
     `./otto-discovery-report <markdown>` (it is captured back into the story's \
     discovery run).\n\n";

/// Render the brief's Attachments section: each attachment listed by its
/// ABSOLUTE path (`data_dir/storage_path`) with an instruction to open it via
/// the agent's file tools. No files are copied (§6.4). Empty when there are no
/// attachments. Pure so it is unit-testable without a `ServerCtx`.
fn render_attachments_section(data_dir: &std::path::Path, atts: &[ProductAttachment]) -> String {
    if atts.is_empty() {
        return String::new();
    }
    let mut s = String::from("## Attachments\n");
    s.push_str(
        "Open these with your file tools (Read). Images/mockups are at the absolute paths below.\n",
    );
    for a in atts {
        let abs = data_dir.join(&a.storage_path);
        s.push_str(&format!(
            "- {} ({}) — {}\n",
            a.filename,
            a.mime,
            abs.to_string_lossy()
        ));
    }
    s.push('\n');
    s
}

/// Truncate `text` to at most `max` chars, appending an ellipsis when cut. Keeps
/// the brief bounded so a sprawling analysis/note doesn't blow the prompt budget.
fn truncate_for_brief(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}…")
}

/// The fixed fallback discovery tasks (§6.2) — used when the discovery planner
/// returns nothing, so a run is never empty.
fn fallback_discovery_tasks() -> Vec<ParsedTask> {
    [
        ("Map affected services & data flow",
         "Identify the services, modules and files this story touches and how data flows between them."),
        ("Identify integration & contract risks",
         "Surface the APIs, schemas and contracts involved and the risks of changing them."),
        ("Review prior art & similar work",
         "Find similar features/changes already in the codebase that inform the approach."),
        ("Compile open questions for stakeholders",
         "List the unknowns and decisions a stakeholder must resolve before implementation."),
    ]
    .into_iter()
    .map(|(title, description)| ParsedTask {
        title: title.to_string(),
        description: description.to_string(),
    })
    .collect()
}

/// Seed investigation tasks for the discovery project (§6.2). Runs the discovery
/// planner on the fast model (haiku, like `seed_tasks`); on empty/failure falls
/// back to a fixed default set. Returns the created tasks (already persisted).
pub(crate) async fn seed_discovery_tasks(
    ctx: &ServerCtx,
    project: &SwarmProject,
    user_id: &Id,
    brief: &str,
) -> Vec<SwarmTask> {
    let agents = ctx
        .swarm_repo
        .list_agents(&project.swarm_id)
        .await
        .unwrap_or_default();
    let preset_agents: Vec<otto_swarm::PresetAgent> = agents
        .iter()
        .map(|a| otto_swarm::PresetAgent {
            key: a.id.clone(),
            name: a.name.clone(),
            title: a.title.clone(),
            reports_to: None,
            provider: a.provider.clone(),
            specialization: a.specialization.clone(),
        })
        .collect();
    let prompt =
        otto_swarm::recruiter::discovery_planner_prompt(&project.name, brief, &preset_agents, "");
    let cwd = project
        .repo_path
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());
    let parsed = match ctx
        .orchestrator
        .run_agent(&prompt, &cwd, Some("haiku"), Duration::from_secs(150))
        .await
    {
        Ok(reply) => parse_discovery_tasks(&reply),
        Err(e) => {
            warn!("product_swarm: discovery planner run_agent failed: {e}");
            Vec::new()
        }
    };
    // Never launch an empty discovery run — fall back to the fixed task set.
    let parsed = if parsed.is_empty() {
        fallback_discovery_tasks()
    } else {
        parsed
    };
    create_tasks(ctx, project, user_id, parsed).await
}

/// Parse the discovery planner's `{"tasks":[{title,description}]}` reply into
/// seedable tasks; returns empty on no/invalid JSON (caller falls back).
fn parse_discovery_tasks(reply: &str) -> Vec<ParsedTask> {
    let Some(v) = otto_swarm::recruiter::extract_json(reply) else {
        warn!("product_swarm: discovery planner returned no parseable JSON");
        return Vec::new();
    };
    v.get("tasks")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter_map(|t| {
            let title = t.get("title").and_then(|v| v.as_str())?.trim().to_string();
            if title.is_empty() {
                return None;
            }
            let description = t
                .get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Some(ParsedTask { title, description })
        })
        .collect()
}

/// `POST /api/v1/product/stories/{sid}/discover` — launch a repeatable discovery
/// swarm for a story. See the section header for the flow.
pub async fn discover_story(
    Path(sid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    body: Option<Json<DiscoverReq>>,
) -> ApiResult<Json<DiscoverResp>> {
    let req = body.map(|b| b.0).unwrap_or_default();

    // 1. Resolve the story + Editor role-check via its workspace.
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &story.workspace_id, WorkspaceRole::Editor).await?;

    // 2. Resolve the target swarm (request → first → auto-create default).
    let swarm = resolve_swarm(&ctx, &story.workspace_id, &user.id, req.swarm_id).await?;

    // 3. Gather the story's attachments (referenced by absolute path in the brief).
    let atts = ctx
        .attachment_repo
        .list_for_story(&story.id)
        .await
        .unwrap_or_default();

    // 4. Build the discovery brief.
    let brief = build_discovery_brief(&ctx, &story, &atts).await;

    // 5. Create the discovery project — story_id MUST be None (the unique index
    //    reserves story_id for the single implementation project; discovery is
    //    repeatable, and its linkage lives in the discovery-run row).
    let existing_runs = ctx
        .discovery_repo
        .list_for_story(&story.id)
        .await
        .map(|r| r.len())
        .unwrap_or(0);
    let name = req
        .name
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| {
            format!("Discovery: {} (run {})", story.title, existing_runs + 1)
        });
    let order_idx = ctx
        .swarm_repo
        .list_projects(&swarm.id)
        .await
        .map(|p| p.len() as i64)
        .unwrap_or(0);
    let project = ctx
        .swarm_repo
        .create_project(NewProject {
            swarm_id: swarm.id.clone(),
            workspace_id: story.workspace_id.clone(),
            name: name.clone(),
            description: format!("Discovery for Product story {}", story.source_key),
            repo_path: story.cwd.clone(),
            goal_md: Some(brief.clone()),
            story_id: None,
            order_idx,
            created_by: user.id.clone(),
        })
        .await
        .map_err(ApiError)?;

    // 6. Seed investigation tasks (planner, else fixed fallback).
    let tasks = seed_discovery_tasks(&ctx, &project, &user.id, &brief).await;

    // 7. Record the discovery-run row (the only story↔project linkage).
    let run = ctx
        .discovery_repo
        .create(NewDiscoveryRun {
            story_id: story.id.clone(),
            workspace_id: story.workspace_id.clone(),
            swarm_id: swarm.id.clone(),
            project_id: project.id.clone(),
            brief_md: brief,
            created_by: user.id.clone(),
        })
        .await
        .map_err(ApiError)?;

    // 8. Record a Product event so the story history shows the discovery launch.
    let _ = ctx
        .product_repo
        .add_event(otto_state::NewEvent {
            story_id: story.id.clone(),
            section: "discovery".into(),
            kind: "discovery_started".into(),
            summary: format!(
                "Started discovery on swarm “{}” as project “{}” ({} task(s))",
                swarm.name,
                project.name,
                tasks.len()
            ),
            actor_id: Some(user.id.clone()),
            meta_json: Some(
                json!({
                    "swarm_id": swarm.id,
                    "project_id": project.id,
                    "run_id": run.id,
                })
                .to_string(),
            ),
        })
        .await;

    // 9. Auto-start the swarm so the discovery agents actually run. Unlike
    //    `to-swarm` (which leaves the swarm paused), discovery is fire-and-go.
    //    Replicates the `start` handler in swarm_runtime.rs: point-of-action
    //    budget gate first (same guard as `start`), then set status active,
    //    start the coordinator, emit the status event. (Starting the swarm runs
    //    ALL ready tasks, which now includes the discovery tasks — intended.)
    {
        let verdict = crate::routes::usage::check_budget(&ctx, &story.workspace_id, "").await;
        if verdict.blocked {
            return Err(ApiError(Error::Invalid(format!(
                "Budget exceeded — swarm blocked: {}",
                verdict.reason.unwrap_or_else(|| "cap reached".to_string())
            ))));
        }
    }
    ctx.swarm_repo
        .set_swarm_status(&swarm.id, "active")
        .await
        .map_err(ApiError)?;
    crate::swarm_runtime::start_coordinator(ctx.clone(), swarm.id.clone());
    crate::swarm_runtime::emit_status(&ctx, &story.workspace_id, &swarm.id, "active");

    // 10. Return the run + swarm + project + seeded tasks.
    let swarm = ctx.swarm_repo.get_swarm(&swarm.id).await.map_err(ApiError)?;
    Ok(Json(DiscoverResp {
        run,
        swarm,
        project,
        tasks,
    }))
}

/// `GET /api/v1/product/stories/{sid}/discovery-runs` — list a story's discovery
/// runs (newest first) with their derived status and progress.
pub async fn list_discovery_runs(
    Path(sid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<Vec<DiscoveryRunSummary>>> {
    let story = ctx.product_repo.get_story(&sid).await.map_err(ApiError)?;
    crate::auth::require_ws_role(&ctx, &user, &story.workspace_id, WorkspaceRole::Viewer).await?;

    let runs = ctx
        .discovery_repo
        .list_for_story(&story.id)
        .await
        .map_err(ApiError)?;
    let mut out = Vec::with_capacity(runs.len());
    for run in runs {
        let tasks = ctx
            .swarm_repo
            .list_tasks(&run.project_id)
            .await
            .unwrap_or_default();
        let done_count = tasks.iter().filter(|t| t.status == "done").count() as i64;
        out.push(DiscoveryRunSummary {
            derived_status: derived_status(&tasks, &run.status),
            task_count: tasks.len() as i64,
            done_count,
            run,
        });
    }
    Ok(Json(out))
}

/// `GET /api/v1/product/discovery-runs/{rid}` — full detail for one discovery run:
/// tasks, per-task latest run summaries, discovery board messages, derived status
/// (and `report_md` on the run itself).
pub async fn get_discovery_run(
    Path(rid): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<DiscoveryRunDetail>> {
    let run = ctx
        .discovery_repo
        .get(&rid)
        .await
        .map_err(ApiError)?
        .ok_or_else(|| ApiError(Error::NotFound(format!("discovery run {rid}"))))?;
    crate::auth::require_ws_role(&ctx, &user, &run.workspace_id, WorkspaceRole::Viewer).await?;

    let tasks = ctx
        .swarm_repo
        .list_tasks(&run.project_id)
        .await
        .unwrap_or_default();

    // Per-task latest run summary (the live "findings so far").
    let mut task_summaries: Vec<(Id, Option<String>)> = Vec::with_capacity(tasks.len());
    for t in &tasks {
        let runs = ctx
            .swarm_repo
            .list_runs(&RunFilter {
                swarm_id: Some(run.swarm_id.clone()),
                project_id: Some(run.project_id.clone()),
                agent_id: None,
                status: None,
            })
            .await
            .unwrap_or_default();
        // `list_runs` is ordered newest-first; first run for this task wins.
        let summary = runs
            .into_iter()
            .find(|r| r.task_id.as_ref() == Some(&t.id))
            .and_then(|r| r.summary);
        task_summaries.push((t.id.clone(), summary));
    }

    // Discovery board messages for this project.
    let messages = ctx
        .swarm_repo
        .list_board(&run.swarm_id, Some(&run.project_id), None, 200)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|m| m.kind == "discovery")
        .collect();

    let derived = derived_status(&tasks, &run.status);
    Ok(Json(DiscoveryRunDetail {
        run,
        derived_status: derived,
        tasks,
        task_summaries,
        messages,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal `SwarmTask` with a given status for `derived_status` tests.
    fn mk_task(status: &str) -> SwarmTask {
        let now = chrono::Utc::now();
        SwarmTask {
            id: "t".into(),
            project_id: "p".into(),
            swarm_id: "sw".into(),
            workspace_id: "w".into(),
            title: "t".into(),
            description: String::new(),
            assignee_agent_id: None,
            status: status.into(),
            priority: "medium".into(),
            parent_task_id: None,
            depends_on: json!([]),
            labels: json!([]),
            result_ref: None,
            delegated: false,
            attempts: 0,
            order_idx: 0,
            created_by: "u".into(),
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn derived_status_all_done() {
        let t = mk_task;
        assert_eq!(derived_status(&[t("done"), t("done")], "running"), "done");
    }

    #[test]
    fn derived_status_mixed_keeps_persisted() {
        let t = mk_task;
        assert_eq!(
            derived_status(&[t("done"), t("in_progress")], "running"),
            "running"
        );
        // Empty task list keeps the persisted status (nothing ran yet).
        assert_eq!(derived_status(&[], "running"), "running");
    }

    #[test]
    fn derived_status_any_error_wins() {
        let t = mk_task;
        // An error surfaces even alongside done tasks.
        assert_eq!(derived_status(&[t("done"), t("error")], "running"), "error");
        assert_eq!(derived_status(&[t("error")], "running"), "error");
    }

    #[test]
    fn brief_includes_discovery_framing_and_abs_attachment_paths() {
        let now = chrono::Utc::now();
        let att = ProductAttachment {
            id: "a1".into(),
            story_id: "s1".into(),
            workspace_id: "w1".into(),
            filename: "mock.png".into(),
            mime: "image/png".into(),
            size_bytes: 10,
            sha256: None,
            storage_path: "product/attachments/s1/a1.png".into(),
            kind: "image".into(),
            source: "user".into(),
            meta_json: None,
            created_by: "u1".into(),
            created_at: now,
            updated_at: now,
        };
        // The mission framing the brief opens with is DISCOVERY-flavoured and
        // forbids implementation.
        assert!(DISCOVERY_MISSION_FRAMING.contains("DISCOVERY"));
        assert!(DISCOVERY_MISSION_FRAMING.contains("Do NOT write production code"));

        // The attachments section lists the file at its ABSOLUTE path
        // (data_dir + storage_path) with an "open with your file tools" note.
        let data_dir = std::path::PathBuf::from("/var/otto-data");
        let section = render_attachments_section(&data_dir, std::slice::from_ref(&att));
        assert!(section.contains("## Attachments"));
        assert!(section.contains("Open these with your file tools"));
        assert!(section.contains("/var/otto-data/product/attachments/s1/a1.png"));
        assert!(section.contains("- mock.png (image/png) — /"));

        // No attachments → empty section (so the brief doesn't render a header).
        assert!(render_attachments_section(&data_dir, &[]).is_empty());
    }

    #[test]
    fn parses_task_headings_with_bodies() {
        let md = "intro line ignored\n\
            ### Task 1: Set up the database\n\
            **Goal:** create schema\n\
            - [ ] add migration\n\
            - [ ] index it\n\
            **Verify:** cargo test\n\
            ### Task 2: Wire the endpoint\n\
            - [ ] add route\n";
        let tasks = parse_plan_tasks(md);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].title, "Set up the database");
        assert!(tasks[0].description.contains("create schema"));
        assert!(tasks[0].description.contains("add migration"));
        assert_eq!(tasks[1].title, "Wire the endpoint");
        assert!(tasks[1].description.contains("add route"));
    }

    #[test]
    fn strips_various_task_prefixes() {
        assert_eq!(strip_task_prefix("Task 1: Do X"), "Do X");
        assert_eq!(strip_task_prefix("Task 12 - Do Y"), "Do Y");
        assert_eq!(strip_task_prefix("Plain heading"), "Plain heading");
        assert_eq!(strip_task_prefix("Task 3. Do Z"), "Do Z");
    }

    #[test]
    fn ignores_h1_and_keeps_h2_h4() {
        // H1 (#) is not a task; H2–H4 are.
        let md = "# Plan title\n## Phase A\n#### Subtask\n";
        let tasks = parse_plan_tasks(md);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].title, "Phase A");
        assert_eq!(tasks[1].title, "Subtask");
    }

    #[test]
    fn empty_plan_yields_no_tasks() {
        assert!(parse_plan_tasks("just some prose\nno headings here").is_empty());
    }
}
