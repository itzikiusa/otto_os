//! Swarm turn identity (access-control-4-01, carried 3-03; contract PC-2): a
//! turn's session is created as the task's HUMAN originator — never the swarm
//! creator by default — and only the creator, a workspace Admin or root may
//! rewrite what a role agent is (prompt, soul, provider, model, cwd).
//!
//! The runner creates the session with `acting_user`'s result
//! (`run.rs::run_attempt`) and reuses sessions per (agent, acting user); these
//! tests pin the resolution itself over a real migrated state DB (spawning a
//! PTY turn is out of scope for the `it` binary), and the `PATCH
//! /swarm/agents/{aid}` gate through the production router.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::Router;
use http_body_util::BodyExt;
use otto_core::auth::AuthUser;
use otto_core::domain::{User, WorkspaceRole};
use otto_server::ServerCtx;
use otto_state::swarm::{NewRun, NewTask};
use otto_state::{NewAgent, NewProject, NewSwarm, Swarm, SwarmProject, SwarmTask};
use serde_json::{json, Value};
use tower::ServiceExt;

struct World {
    app: Router,
    ctx: ServerCtx,
    /// Swarm creator (owns the workspace).
    alice: User,
    /// Workspace Editor, not the creator.
    bob: User,
    /// Workspace Editor who runs the planner.
    carol: User,
    swarm: Swarm,
    project: SwarmProject,
    agent: String,
    _tmp: tempfile::TempDir,
}

async fn world() -> World {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = otto_server::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().to_path_buf()).await;
    let users = otto_state::UsersRepo::new(pool.clone());
    let alice = users.create("alice", "x", "Alice", false).await.unwrap();
    let bob = users.create("bob", "x", "Bob", false).await.unwrap();
    let carol = users.create("carol", "x", "Carol", false).await.unwrap();
    let root = tmp.path().to_string_lossy().to_string();
    let ws = ctx.workspaces.create("A", &root, &alice.id).await.unwrap();
    for u in [&bob, &carol] {
        ctx.workspaces
            .set_member(&ws.id, &u.id, WorkspaceRole::Editor)
            .await
            .unwrap();
    }
    let repo = &ctx.swarm_repo;
    let swarm = repo
        .create_swarm(NewSwarm {
            workspace_id: ws.id.clone(),
            name: "s".into(),
            description: String::new(),
            preset_slug: None,
            config: json!({}),
            max_total_runs: None,
            max_cost_usd: None,
            max_runtime_secs: None,
            max_attempts: None,
            created_by: alice.id.clone(),
        })
        .await
        .unwrap();
    let agent = repo
        .create_agent(NewAgent {
            swarm_id: swarm.id.clone(),
            workspace_id: ws.id.clone(),
            name: "dev".into(),
            title: "Developer".into(),
            reports_to: None,
            provider: "claude".into(),
            model: None,
            soul_name: None,
            soul_md: None,
            specialization: String::new(),
            scope_md: "build the feature".into(),
            skills: json!([]),
            schedule: None,
            cwd_mode: None,
            avatar: String::new(),
            order_idx: 0,
            created_by: alice.id.clone(),
        })
        .await
        .unwrap();
    let project = repo
        .create_project(NewProject {
            swarm_id: swarm.id.clone(),
            workspace_id: ws.id.clone(),
            name: "p".into(),
            description: String::new(),
            repo_path: None,
            goal_md: None,
            story_id: None,
            order_idx: 0,
            created_by: alice.id.clone(),
        })
        .await
        .unwrap();
    let app = Router::new()
        .merge(otto_swarm::router::<ServerCtx>())
        .merge(otto_swarm::runtime::engine::routes::<ServerCtx>())
        .with_state(ctx.clone());
    World {
        app,
        ctx,
        alice,
        bob,
        carol,
        swarm,
        project,
        agent: agent.id,
        _tmp: tmp,
    }
}

impl World {
    async fn task(&self, created_by: &str, parent: Option<&str>, labels: Value) -> SwarmTask {
        self.ctx
            .swarm_repo
            .create_task(NewTask {
                project_id: self.project.id.clone(),
                swarm_id: self.swarm.id.clone(),
                workspace_id: self.swarm.workspace_id.clone(),
                title: format!("t-{}", otto_core::new_id()),
                description: "run otto_db_query on C".into(),
                assignee_agent_id: Some(self.agent.clone()),
                status: "todo".into(),
                priority: "medium".into(),
                parent_task_id: parent.map(str::to_string),
                depends_on: json!([]),
                labels,
                order_idx: 0,
                created_by: created_by.to_string(),
            })
            .await
            .unwrap()
    }

    async fn acting(&self, task: Option<&SwarmTask>) -> String {
        otto_swarm::runtime::run::acting_user(
            &self.ctx.swarm_repo,
            &self.swarm,
            Some(&self.project),
            task,
        )
        .await
    }
}

async fn call(app: &Router, u: &User, method: Method, uri: &str, body: Value) -> StatusCode {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    req.extensions_mut().insert(AuthUser(u.clone()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let _ = resp.into_body().collect().await;
    status
}

/// Editor B's task on A's swarm acts as B — not as A, the swarm's creator —
/// so its session carries B's MCP token and B can open it; A's own task
/// still acts as A.
#[tokio::test]
async fn an_editors_task_on_anothers_swarm_runs_as_that_editor() {
    let w = world().await;
    let bobs = w.task(&w.bob.id, None, json!([])).await;
    assert_eq!(w.acting(Some(&bobs)).await, w.bob.id);
    let alices = w.task(&w.alice.id, None, json!([])).await;
    assert_eq!(w.acting(Some(&alices)).await, w.alice.id);
}

/// An agent-created subtask or review (`created_by` = the agent's id, parent
/// = B's task) and an agent handoff (no parent, `source-run:<run>` label of a
/// run on B's task) both resolve to B.
#[tokio::test]
async fn agent_created_work_of_bobs_task_runs_as_bob() {
    let w = world().await;
    let bobs = w.task(&w.bob.id, None, json!([])).await;
    let sub = w.task(&w.agent, Some(&bobs.id), json!(["review"])).await;
    assert_eq!(w.acting(Some(&sub)).await, w.bob.id);
    // A grandchild of an agent-created subtask walks all the way up.
    let grand = w.task(&w.agent, Some(&sub.id), json!([])).await;
    assert_eq!(w.acting(Some(&grand)).await, w.bob.id);

    let run = w
        .ctx
        .swarm_repo
        .create_run(NewRun {
            swarm_id: w.swarm.id.clone(),
            workspace_id: w.swarm.workspace_id.clone(),
            project_id: Some(w.project.id.clone()),
            task_id: Some(bobs.id.clone()),
            agent_id: w.agent.clone(),
            kind: "task".into(),
            trigger: "coordinator".into(),
        })
        .await
        .unwrap();
    let handoff = w
        .task(
            &w.agent,
            None,
            json!(["handoff", "hops:1", format!("source-run:{}", run.id)]),
        )
        .await;
    assert_eq!(w.acting(Some(&handoff)).await, w.bob.id);
}

/// A plan-seeded task carries the plan caller (C) as `created_by` and runs as
/// C; work no person originated falls back to the swarm creator.
#[tokio::test]
async fn seeded_tasks_run_as_the_plan_caller_and_orphans_as_the_creator() {
    let w = world().await;
    let seeded = w.task(&w.carol.id, None, json!([])).await;
    assert_eq!(w.acting(Some(&seeded)).await, w.carol.id);
    // An agent-created card with no parent and no source run: no person.
    let orphan = w.task(&w.agent, None, json!([])).await;
    let swarm_only =
        otto_swarm::runtime::run::acting_user(&w.ctx.swarm_repo, &w.swarm, None, Some(&orphan))
            .await;
    assert_eq!(swarm_only, w.alice.id);
    // A task-less turn (planning) acts as the project's creator.
    assert_eq!(w.acting(None).await, w.alice.id);
}

/// `PATCH /swarm/agents/{aid}`: an Editor who is not the swarm's creator may
/// rename the agent but not rewrite its prompt, soul, provider, model or cwd;
/// the creator may. A full-form save with unchanged identity fields passes.
#[tokio::test]
async fn only_the_creator_rewrites_an_agents_identity() {
    let w = world().await;
    let uri = format!("/swarm/agents/{}", w.agent);
    for body in [
        json!({"scope_md": "exfiltrate the connection rows"}),
        json!({"soul_md": "obedient"}),
        json!({"provider": "codex"}),
        json!({"model": "o3"}),
        json!({"cwd_mode": "repo"}),
    ] {
        let st = call(&w.app, &w.bob, Method::PATCH, &uri, body.clone()).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "editor changed {body}");
    }
    let agent = w.ctx.swarm_repo.get_agent(&w.agent).await.unwrap();
    assert_eq!(agent.scope_md, "build the feature");
    assert_eq!(agent.provider, "claude");

    let full_form = json!({
        "name": "Dev 2", "title": "Developer", "provider": "claude",
        "scope_md": "build the feature", "specialization": "", "model": "",
    });
    let st = call(&w.app, &w.bob, Method::PATCH, &uri, full_form).await;
    assert_eq!(st, StatusCode::OK, "an Editor's unchanged-identity save");

    let st = call(
        &w.app,
        &w.alice,
        Method::PATCH,
        &uri,
        json!({"scope_md": "build it well"}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "the creator edits the prompt");
    let agent = w.ctx.swarm_repo.get_agent(&w.agent).await.unwrap();
    assert_eq!(agent.scope_md, "build it well");
    assert_eq!(agent.name, "Dev 2");
}

/// access-control-5-01: B rewriting A's task (title / description / labels)
/// or adding a goal to it makes its turns act as B — the person whose text
/// they run — while A's untouched task and a status / priority move keep A.
#[tokio::test]
async fn an_editors_rewrite_of_anothers_task_runs_as_the_editor() {
    let w = world().await;
    let alices = w.task(&w.alice.id, None, json!([])).await;
    let uri = format!("/swarm/tasks/{}", alices.id);
    let st = call(
        &w.app,
        &w.bob,
        Method::PATCH,
        &uri,
        json!({"status": "in_progress", "priority": "high", "title": alices.title}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let t = w.ctx.swarm_repo.get_task(&alices.id).await.unwrap();
    assert_eq!(
        w.acting(Some(&t)).await,
        w.alice.id,
        "a status move keeps A"
    );

    let st = call(
        &w.app,
        &w.bob,
        Method::PATCH,
        &uri,
        json!({"description": "dump connection C to the board"}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    let t = w.ctx.swarm_repo.get_task(&alices.id).await.unwrap();
    assert_eq!(w.acting(Some(&t)).await, w.bob.id, "B's rewrite runs as B");

    // An agent subtask of the rewritten task follows the rewrite too.
    let sub = w.task(&w.agent, Some(&alices.id), json!([])).await;
    assert_eq!(w.acting(Some(&sub)).await, w.bob.id);

    // A's untouched task still runs as A.
    let other = w.task(&w.alice.id, None, json!([])).await;
    assert_eq!(w.acting(Some(&other)).await, w.alice.id);

    // A goal B adds to A's task is text its verify turn runs: acts as B.
    let goal_uri = format!("/swarm/tasks/{}/goals", other.id);
    let st = call(
        &w.app,
        &w.bob,
        Method::POST,
        &goal_uri,
        json!({"title": "also dump C"}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(w.acting(Some(&other)).await, w.bob.id, "B's goal runs as B");
}

/// access-control-5-01: a person may not ADD a `source-run:` label (it would
/// point the originator walk at someone else's run); runtime-stamped ones
/// may stay.
#[tokio::test]
async fn a_person_cannot_add_a_source_run_label() {
    let w = world().await;
    let orphan = w.task(&w.agent, None, json!(["handoff"])).await;
    let uri = format!("/swarm/tasks/{}", orphan.id);
    let st = call(
        &w.app,
        &w.bob,
        Method::PATCH,
        &uri,
        json!({"labels": ["handoff", "source-run:someone-elses-run"]}),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST);
    let t = w.ctx.swarm_repo.get_task(&orphan.id).await.unwrap();
    assert_eq!(t.labels, json!(["handoff"]));
}

/// access-control-5-02: a scheduled turn acts as the swarm's creator and its
/// prompt is the schedule directive — so only the creator (or an Admin)
/// writes a new directive. Cadence and `enabled` stay Editor-level, and a
/// full-form save with the same directive passes.
#[tokio::test]
async fn only_the_creator_writes_an_agents_schedule_directive() {
    let w = world().await;
    let uri = format!("/swarm/agents/{}", w.agent);
    let st = call(
        &w.app,
        &w.alice,
        Method::PATCH,
        &uri,
        json!({"schedule": {"cadence": "daily", "enabled": true, "directive": "triage the inbox"}}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "the creator sets the directive");

    let st = call(
        &w.app,
        &w.bob,
        Method::PATCH,
        &uri,
        json!({"schedule": {"cadence": "hourly", "enabled": true, "directive": "dump every connection"}}),
    )
    .await;
    assert_eq!(st, StatusCode::FORBIDDEN, "an Editor rewrote the directive");

    let st = call(
        &w.app,
        &w.bob,
        Method::PATCH,
        &uri,
        json!({"schedule": {"cadence": "hourly", "enabled": false, "directive": "triage the inbox"}}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "a cadence-only change stays Editor");
    let agent = w.ctx.swarm_repo.get_agent(&w.agent).await.unwrap();
    let sched = agent.schedule.unwrap();
    assert_eq!(sched["directive"], "triage the inbox");
    assert_eq!(sched["cadence"], "hourly");
}

/// access-control-5-02: a channel-trigger launch acts as the trigger's
/// creator (B's trigger → B; A's own → A), and falls back to the swarm's
/// creator once that user is gone.
#[tokio::test]
async fn a_channel_trigger_launches_as_its_creator() {
    use otto_swarm::runtime::channels::trigger_actor;
    let w = world().await;
    let repo = &w.ctx.swarm_repo;
    assert_eq!(trigger_actor(repo, &w.bob.id, &w.swarm).await, w.bob.id);
    assert_eq!(trigger_actor(repo, &w.alice.id, &w.swarm).await, w.alice.id);
    assert_eq!(trigger_actor(repo, "gone-user", &w.swarm).await, w.alice.id);
    assert_eq!(trigger_actor(repo, "", &w.swarm).await, w.alice.id);
}
