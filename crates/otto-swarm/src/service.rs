//! SwarmService — CRUD façade over `SwarmRepo` plus composite assembly. Holds no
//! session/LLM dependencies (those live in the otto-server runtime).

use otto_core::{Id, Result};
use otto_state::swarm::NewTask;
use otto_state::{
    NewAgent, NewMessage, NewProject, NewSwarm, ProjectPatch, RunFilter, Swarm, SwarmAgent,
    SwarmMessage, SwarmPatch, SwarmProject, SwarmRepo, SwarmRun, SwarmTask, TaskPatch,
};
use serde_json::{json, Value};

use crate::types::*;

#[derive(Clone)]
pub struct SwarmService {
    pub repo: SwarmRepo,
}

impl SwarmService {
    pub fn new(repo: SwarmRepo) -> Self {
        Self { repo }
    }

    fn default_config() -> Value {
        json!({
            "provider": "claude",
            "max_parallel_sessions": 4,
            "cwd_mode": "scratch",
            "auto_submit": false
        })
    }

    // -- Swarms -------------------------------------------------------------

    pub async fn list_swarms(&self, ws: &Id) -> Result<Vec<Swarm>> {
        self.repo.list_swarms(ws).await
    }

    pub async fn get_swarm(&self, id: &Id) -> Result<Swarm> {
        self.repo.get_swarm(id).await
    }

    pub async fn create_swarm(
        &self,
        ws: &Id,
        user: &Id,
        req: CreateSwarmReq,
    ) -> Result<Swarm> {
        let config = req.config.unwrap_or_else(Self::default_config);
        self.repo
            .create_swarm(NewSwarm {
                workspace_id: ws.clone(),
                name: req.name,
                description: req.description.unwrap_or_default(),
                preset_slug: req.preset_slug,
                config,
                max_total_runs: req.max_total_runs,
                max_cost_usd: req.max_cost_usd,
                max_runtime_secs: req.max_runtime_secs,
                max_attempts: req.max_attempts,
                created_by: user.clone(),
            })
            .await
    }

    pub async fn update_swarm(&self, id: &Id, req: UpdateSwarmReq) -> Result<Swarm> {
        self.repo
            .update_swarm(
                id,
                SwarmPatch {
                    name: req.name,
                    description: req.description,
                    status: req.status,
                    config: req.config,
                    max_total_runs: req.max_total_runs,
                    max_cost_usd: req.max_cost_usd,
                    max_runtime_secs: req.max_runtime_secs,
                    max_attempts: req.max_attempts,
                },
            )
            .await
    }

    pub async fn delete_swarm(&self, id: &Id) -> Result<()> {
        self.repo.delete_swarm(id).await
    }

    pub async fn detail(&self, id: &Id) -> Result<SwarmDetail> {
        let swarm = self.repo.get_swarm(id).await?;
        let agents = self.repo.list_agents(id).await?;
        let projects = self.repo.list_projects(id).await?;
        let tasks = self.repo.list_tasks_for_swarm(id).await?;
        let running_runs = self.repo.running_count(id).await?;
        let spend = self.repo.swarm_spend(id).await?;
        let counts = SwarmCounts {
            agents: agents.len(),
            projects: projects.len(),
            tasks: tasks.len(),
            running_runs,
            total_runs: spend.total_runs,
            cost_usd: spend.cost_usd,
        };
        Ok(SwarmDetail {
            swarm,
            agents,
            projects,
            counts,
        })
    }

    // -- Agents -------------------------------------------------------------

    pub async fn list_agents(&self, swarm_id: &Id) -> Result<Vec<SwarmAgent>> {
        self.repo.list_agents(swarm_id).await
    }

    pub async fn get_agent(&self, id: &Id) -> Result<SwarmAgent> {
        self.repo.get_agent(id).await
    }

    pub async fn create_agent(
        &self,
        swarm: &Swarm,
        user: &Id,
        req: CreateAgentReq,
    ) -> Result<SwarmAgent> {
        self.repo
            .create_agent(NewAgent {
                swarm_id: swarm.id.clone(),
                workspace_id: swarm.workspace_id.clone(),
                name: req.name,
                title: req.title.unwrap_or_default(),
                reports_to: req.reports_to,
                provider: req.provider,
                model: req.model,
                soul_name: req.soul_name,
                soul_md: req.soul_md,
                specialization: req.specialization.unwrap_or_default(),
                scope_md: req.scope_md.unwrap_or_default(),
                skills: req.skills.unwrap_or_else(|| json!([])),
                schedule: req.schedule,
                cwd_mode: req.cwd_mode,
                avatar: req.avatar.unwrap_or_default(),
                order_idx: req.order_idx.unwrap_or(0),
                created_by: user.clone(),
            })
            .await
    }

    pub async fn update_agent(&self, id: &Id, req: UpdateAgentReq) -> Result<SwarmAgent> {
        use otto_state::AgentPatch;
        self.repo
            .update_agent(
                id,
                AgentPatch {
                    name: req.name,
                    title: req.title,
                    reports_to: req.reports_to.map(Some),
                    provider: req.provider,
                    model: req.model.map(Some),
                    soul_name: req.soul_name.map(Some),
                    soul_md: req.soul_md.map(Some),
                    specialization: req.specialization,
                    scope_md: req.scope_md,
                    skills: req.skills,
                    schedule: req.schedule.map(Some),
                    cwd_mode: req.cwd_mode.map(Some),
                    avatar: None,
                    status: req.status,
                    order_idx: req.order_idx,
                },
            )
            .await
    }

    pub async fn delete_agent(&self, id: &Id) -> Result<()> {
        self.repo.delete_agent(id).await
    }

    // -- Projects -----------------------------------------------------------

    pub async fn list_projects(&self, swarm_id: &Id) -> Result<Vec<SwarmProject>> {
        self.repo.list_projects(swarm_id).await
    }

    pub async fn get_project(&self, id: &Id) -> Result<SwarmProject> {
        self.repo.get_project(id).await
    }

    pub async fn create_project(
        &self,
        swarm: &Swarm,
        user: &Id,
        req: CreateProjectReq,
    ) -> Result<SwarmProject> {
        self.repo
            .create_project(NewProject {
                swarm_id: swarm.id.clone(),
                workspace_id: swarm.workspace_id.clone(),
                name: req.name,
                description: req.description.unwrap_or_default(),
                repo_path: req.repo_path,
                goal_md: req.goal_md,
                story_id: None,
                order_idx: 0,
                created_by: user.clone(),
            })
            .await
    }

    pub async fn update_project(&self, id: &Id, req: UpdateProjectReq) -> Result<SwarmProject> {
        self.repo
            .update_project(
                id,
                ProjectPatch {
                    name: req.name,
                    description: req.description,
                    repo_path: req.repo_path.map(Some),
                    goal_md: req.goal_md.map(Some),
                    // story_id is an internal Plan → Swarm back-link, not editable
                    // via the project PATCH endpoint — leave it unchanged.
                    story_id: None,
                    status: req.status,
                    order_idx: req.order_idx,
                },
            )
            .await
    }

    pub async fn delete_project(&self, id: &Id) -> Result<()> {
        self.repo.delete_project(id).await
    }

    // -- Tasks --------------------------------------------------------------

    pub async fn list_tasks(&self, project_id: &Id) -> Result<Vec<SwarmTask>> {
        self.repo.list_tasks(project_id).await
    }

    pub async fn get_task(&self, id: &Id) -> Result<SwarmTask> {
        self.repo.get_task(id).await
    }

    pub async fn create_task(
        &self,
        project: &SwarmProject,
        user: &Id,
        req: CreateTaskReq,
    ) -> Result<SwarmTask> {
        self.repo
            .create_task(NewTask {
                project_id: project.id.clone(),
                swarm_id: project.swarm_id.clone(),
                workspace_id: project.workspace_id.clone(),
                title: req.title,
                description: req.description.unwrap_or_default(),
                assignee_agent_id: req.assignee_agent_id,
                // Default to "todo" so a hand-added task (UI "Add task") is
                // immediately schedulable — `ready_tasks` only picks up "todo".
                // A caller that explicitly sets a status (e.g. parks it in
                // "backlog") is preserved.
                status: req.status.unwrap_or_else(|| "todo".into()),
                priority: req.priority.unwrap_or_else(|| "medium".into()),
                parent_task_id: None,
                depends_on: req.depends_on.unwrap_or_else(|| json!([])),
                labels: req.labels.unwrap_or_else(|| json!([])),
                order_idx: req.order_idx.unwrap_or(0),
                created_by: user.clone(),
            })
            .await
    }

    pub async fn update_task(&self, id: &Id, req: UpdateTaskReq) -> Result<SwarmTask> {
        self.repo
            .update_task(
                id,
                TaskPatch {
                    title: req.title,
                    description: req.description,
                    assignee_agent_id: req.assignee_agent_id.map(Some),
                    status: req.status,
                    priority: req.priority,
                    depends_on: req.depends_on,
                    labels: req.labels,
                    result_ref: None,
                    delegated: None,
                    order_idx: req.order_idx,
                },
            )
            .await
    }

    pub async fn delete_task(&self, id: &Id) -> Result<()> {
        self.repo.delete_task(id).await
    }

    // -- Runs ---------------------------------------------------------------

    pub async fn list_runs(&self, f: &RunFilter) -> Result<Vec<SwarmRun>> {
        self.repo.list_runs(f).await
    }

    pub async fn get_run(&self, id: &Id) -> Result<SwarmRun> {
        self.repo.get_run(id).await
    }

    // -- Board --------------------------------------------------------------

    pub async fn list_board(
        &self,
        swarm_id: &Id,
        project_id: Option<&Id>,
        task_id: Option<&Id>,
    ) -> Result<Vec<SwarmMessage>> {
        self.repo
            .list_board(swarm_id, project_id, task_id, 300)
            .await
    }

    pub async fn post_human_message(
        &self,
        swarm: &Swarm,
        user: &Id,
        req: PostMessageReq,
    ) -> Result<SwarmMessage> {
        self.repo
            .create_message(NewMessage {
                swarm_id: swarm.id.clone(),
                workspace_id: swarm.workspace_id.clone(),
                project_id: req.project_id,
                task_id: req.task_id,
                run_id: None,
                author_agent_id: None,
                author_user_id: Some(user.clone()),
                to_agent_id: req.to_agent_id,
                kind: req.kind.unwrap_or_else(|| "message".into()),
                body: req.body,
                meta: json!({}),
            })
            .await
    }

    // -- Run graph ----------------------------------------------------------

    pub async fn graph(&self, swarm_id: &Id) -> Result<SwarmGraph> {
        let tasks = self.repo.list_tasks_for_swarm(swarm_id).await?;
        let mut nodes = Vec::new();
        let mut edges = Vec::new();
        for t in &tasks {
            // Surface the latest run's session for this task (if any).
            nodes.push(GraphNode {
                id: format!("task:{}", t.id),
                kind: "task".into(),
                label: t.title.clone(),
                status: t.status.clone(),
                agent_id: t.assignee_agent_id.clone(),
                session_id: None,
                project_id: Some(t.project_id.clone()),
            });
            if let Some(deps) = t.depends_on.as_array() {
                for d in deps.iter().filter_map(|d| d.as_str()) {
                    edges.push(GraphEdge {
                        from: format!("task:{d}"),
                        to: format!("task:{}", t.id),
                        kind: "depends".into(),
                    });
                }
            }
            if let Some(parent) = &t.parent_task_id {
                edges.push(GraphEdge {
                    from: format!("task:{parent}"),
                    to: format!("task:{}", t.id),
                    kind: "handoff".into(),
                });
            }
        }
        // Attach the most recent session per task from runs.
        let runs = self.repo.runs_for_swarm(swarm_id, 500).await?;
        for n in nodes.iter_mut() {
            if let Some(tid) = n.id.strip_prefix("task:") {
                if let Some(run) = runs
                    .iter()
                    .find(|r| r.task_id.as_deref() == Some(tid) && r.session_id.is_some())
                {
                    n.session_id = run.session_id.clone();
                }
            }
        }
        Ok(SwarmGraph { nodes, edges })
    }
}
