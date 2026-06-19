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
use otto_state::swarm::NewTask;
use otto_state::{NewProject, ProductStory, Swarm, SwarmProject, SwarmTask};
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
            created_by: user_id.clone(),
        })
        .await
        .map_err(ApiError)
}

/// Seed tasks for the new project. Reuse the story's existing plan markdown when
/// present; otherwise run the swarm planner over the goal to generate tasks.
/// Returns the list of created tasks (already persisted).
async fn seed_tasks(
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
    let prompt = otto_swarm::recruiter::planner_prompt(&project.name, goal_md, &preset_agents);
    let cwd = project
        .repo_path
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string());
    let reply = match ctx
        .orchestrator
        .run_agent(&prompt, &cwd, None, Duration::from_secs(150))
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
    let project = ctx
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
        .map_err(ApiError)?;

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

#[cfg(test)]
mod tests {
    use super::*;

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
