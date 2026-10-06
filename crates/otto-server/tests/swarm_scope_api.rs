//! Route-level isolation tests for the swarm runtime + product agent routes
//! (review S4-01/04/05/06/17/26): every `/workspaces/{id}/…/{rowId}` handler
//! must refuse a row of ANOTHER workspace (404, nothing mutated), assignees
//! must stay on the task's own swarm, "Stop verification" must settle the
//! task, and PATCH `null` must clear nullable goal / task fields.
//!
//! Same harness as `workbench_api.rs`: a real `ServerCtx::for_tests` over an
//! in-memory, fully migrated sqlite; requests go through the production
//! handlers via `tower::ServiceExt::oneshot` with `AuthUser` injected the way
//! the auth middleware does.

use axum::body::Body;
use axum::extract::Request;
use axum::http::{Method, StatusCode};
use axum::routing::post;
use axum::Router;
use http_body_util::BodyExt;
use otto_core::auth::AuthUser;
use otto_core::domain::User;
use otto_server::ServerCtx;
use otto_state::swarm::NewTask;
use otto_state::{NewAgent, NewProject, NewSwarm, TaskPatch};
use serde_json::{json, Value};
use tower::ServiceExt;

struct World {
    app: Router,
    ctx: ServerCtx,
    alice: User,
    ws_a: String,
    swarm_a: String,
    project_a: String,
    agent_a: String,
    swarm_b: String,
    project_b: String,
    agent_b: String,
    _tmp: tempfile::TempDir,
}

async fn world() -> World {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = otto_server::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().to_path_buf()).await;
    let users = otto_state::UsersRepo::new(pool.clone());
    // Non-root users: alice administers A only, bob administers B only.
    let alice = users.create("alice", "x", "Alice", false).await.unwrap();
    let bob = users.create("bob", "x", "Bob", false).await.unwrap();
    let root = tmp.path().to_string_lossy().to_string();
    let ws_a = ctx.workspaces.create("A", &root, &alice.id).await.unwrap();
    let ws_b = ctx.workspaces.create("B", &root, &bob.id).await.unwrap();

    let mk_swarm = |ws: String, by: String| NewSwarm {
        workspace_id: ws,
        name: "s".into(),
        description: "secret mission".into(),
        preset_slug: None,
        config: json!({}),
        max_total_runs: None,
        max_cost_usd: None,
        max_runtime_secs: None,
        max_attempts: None,
        created_by: by,
    };
    let repo = &ctx.swarm_repo;
    let swarm_a = repo
        .create_swarm(mk_swarm(ws_a.id.clone(), alice.id.clone()))
        .await
        .unwrap();
    let swarm_b = repo
        .create_swarm(mk_swarm(ws_b.id.clone(), bob.id.clone()))
        .await
        .unwrap();
    let mk_agent = |s: &otto_state::Swarm, by: &str| NewAgent {
        swarm_id: s.id.clone(),
        workspace_id: s.workspace_id.clone(),
        name: "dev".into(),
        title: "Developer".into(),
        reports_to: None,
        provider: "claude".into(),
        model: None,
        soul_name: None,
        soul_md: None,
        specialization: String::new(),
        scope_md: String::new(),
        skills: json!([]),
        schedule: None,
        cwd_mode: None,
        avatar: String::new(),
        order_idx: 0,
        created_by: by.to_string(),
    };
    let agent_a = repo
        .create_agent(mk_agent(&swarm_a, &alice.id))
        .await
        .unwrap();
    let agent_b = repo
        .create_agent(mk_agent(&swarm_b, &bob.id))
        .await
        .unwrap();
    let mk_project = |s: &otto_state::Swarm, by: &str| NewProject {
        swarm_id: s.id.clone(),
        workspace_id: s.workspace_id.clone(),
        name: "p".into(),
        description: String::new(),
        repo_path: None,
        goal_md: Some("ship it".into()),
        story_id: None,
        order_idx: 0,
        created_by: by.to_string(),
    };
    let project_a = repo
        .create_project(mk_project(&swarm_a, &alice.id))
        .await
        .unwrap();
    let project_b = repo
        .create_project(mk_project(&swarm_b, &bob.id))
        .await
        .unwrap();

    let app = Router::new()
        .merge(otto_swarm::router::<ServerCtx>())
        .merge(otto_swarm::runtime::engine::routes::<ServerCtx>())
        .route(
            "/product/analyses/{aid}/agents/{agent_id}/stop",
            post(otto_product::analysis::stop_analysis_agent::<ServerCtx>),
        )
        .with_state(ctx.clone());
    World {
        app,
        ctx,
        alice,
        ws_a: ws_a.id,
        swarm_a: swarm_a.id,
        project_a: project_a.id,
        agent_a: agent_a.id,
        swarm_b: swarm_b.id,
        project_b: project_b.id,
        agent_b: agent_b.id,
        _tmp: tmp,
    }
}

async fn call(
    app: &Router,
    u: &User,
    method: Method,
    uri: &str,
    body: Value,
) -> (StatusCode, Value) {
    let mut req = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    req.extensions_mut().insert(AuthUser(u.clone()));
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// S4-01: the lifecycle routes 404 a swarm of another workspace and leave it
/// untouched; the same route works on the caller's own swarm.
#[tokio::test]
async fn lifecycle_routes_refuse_a_foreign_swarm() {
    let w = world().await;
    let before = w.ctx.swarm_repo.get_swarm(&w.swarm_b).await.unwrap().status;
    for action in ["abort", "pause", "resume", "start", "agent-stop"] {
        let uri = format!("/workspaces/{}/swarm/swarms/{}/{action}", w.ws_a, w.swarm_b);
        let (st, _) = call(&w.app, &w.alice, Method::POST, &uri, json!({})).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{action} on B's swarm via A");
    }
    let b = w.ctx.swarm_repo.get_swarm(&w.swarm_b).await.unwrap();
    // A new swarm starts `paused`; none of the refused calls changed it.
    assert_eq!(b.status, before, "B's swarm status untouched");
    assert_ne!(b.status, "aborted", "B's swarm was not aborted");

    let own = format!("/workspaces/{}/swarm/swarms/{}/pause", w.ws_a, w.swarm_a);
    let (st, body) = call(&w.app, &w.alice, Method::POST, &own, json!({})).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["status"], "paused");
}

/// S4-01: plan (B's project) and recruit (B's swarm) via workspace A → 404,
/// before any planner/recruiter agent is spawned or B's mission leaks.
#[tokio::test]
async fn plan_and_recruit_refuse_foreign_rows() {
    let w = world().await;
    let uri = format!("/workspaces/{}/swarm/projects/{}/plan", w.ws_a, w.project_b);
    let (st, _) = call(&w.app, &w.alice, Method::POST, &uri, json!({})).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let uri = format!("/workspaces/{}/swarm/recruit", w.ws_a);
    let (st, body) = call(
        &w.app,
        &w.alice,
        Method::POST,
        &uri,
        json!({"role": "QA", "swarm_id": w.swarm_b}),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert!(!body.to_string().contains("secret mission"));
}

/// S4-05 + S4-17: an assignee must be an agent of the task's own swarm (400
/// otherwise); `null` clears it.
#[tokio::test]
async fn task_assignee_stays_on_roster_and_null_clears() {
    let w = world().await;
    let uri = format!("/swarm/projects/{}/tasks", w.project_a);
    let (st, _) = call(
        &w.app,
        &w.alice,
        Method::POST,
        &uri,
        json!({"title": "t", "assignee_agent_id": w.agent_b}),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "foreign assignee on create");
    let (st, task) = call(
        &w.app,
        &w.alice,
        Method::POST,
        &uri,
        json!({"title": "t", "assignee_agent_id": w.agent_a}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{task}");
    let tid = task["id"].as_str().unwrap().to_string();
    let turi = format!("/swarm/tasks/{tid}");
    let (st, _) = call(
        &w.app,
        &w.alice,
        Method::PATCH,
        &turi,
        json!({"assignee_agent_id": w.agent_b}),
    )
    .await;
    assert_eq!(st, StatusCode::BAD_REQUEST, "foreign assignee on update");
    // An absent key leaves it unchanged …
    let (st, t) = call(
        &w.app,
        &w.alice,
        Method::PATCH,
        &turi,
        json!({"title": "t2"}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(t["assignee_agent_id"], json!(w.agent_a));
    // … an explicit null clears it.
    let (st, t) = call(
        &w.app,
        &w.alice,
        Method::PATCH,
        &turi,
        json!({"assignee_agent_id": null}),
    )
    .await;
    assert_eq!(st, StatusCode::OK);
    assert!(t["assignee_agent_id"].is_null(), "{t}");
}

/// S4-06: "Stop verification" settles a verifying task to blocked (it used
/// to stay `verifying` forever and be re-verified on the next start).
#[tokio::test]
async fn stop_verification_blocks_the_task() {
    let w = world().await;
    let task = w
        .ctx
        .swarm_repo
        .create_task(NewTask {
            project_id: w.project_a.clone(),
            swarm_id: w.swarm_a.clone(),
            workspace_id: w.ws_a.clone(),
            title: "v".into(),
            description: String::new(),
            assignee_agent_id: Some(w.agent_a.clone()),
            status: "todo".into(),
            priority: "medium".into(),
            parent_task_id: None,
            depends_on: json!([]),
            labels: json!([]),
            order_idx: 0,
            created_by: w.alice.id.clone(),
        })
        .await
        .unwrap();
    w.ctx
        .swarm_repo
        .update_task(
            &task.id,
            TaskPatch {
                status: Some("verifying".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let uri = format!("/swarm/tasks/{}/verify/stop", task.id);
    let (st, _) = call(&w.app, &w.alice, Method::POST, &uri, json!({})).await;
    assert_eq!(st, StatusCode::OK);
    let t = w.ctx.swarm_repo.get_task(&task.id).await.unwrap();
    assert_eq!(t.status, "blocked");
}

/// S4-17: PATCH `null` clears a goal's nullable fields.
#[tokio::test]
async fn goal_patch_null_clears_fields() {
    let w = world().await;
    let uri = format!("/swarm/projects/{}/goals", w.project_a);
    let (st, goal) = call(
        &w.app,
        &w.alice,
        Method::POST,
        &uri,
        json!({"title": "g", "metric": "coverage", "verify_cmd": "make test", "target_value": 80.0}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{goal}");
    let guri = format!("/swarm/goals/{}", goal["id"].as_str().unwrap());
    let (st, g) = call(
        &w.app,
        &w.alice,
        Method::PATCH,
        &guri,
        json!({"metric": null, "verify_cmd": null, "target_value": null}),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{g}");
    assert!(g["metric"].is_null(), "{g}");
    assert!(g["verify_cmd"].is_null(), "{g}");
    assert!(g["target_value"].is_null(), "{g}");
}

/// S4-04: stopping an agent through an analysis it does not belong to → 404,
/// and the foreign agent is untouched.
#[tokio::test]
async fn stop_analysis_agent_refuses_a_foreign_agent() {
    let w = world().await;
    let mk_story = |ws: &str, by: &str| otto_state::NewStory {
        workspace_id: ws.to_string(),
        source_kind: "draft".into(),
        account_id: String::new(),
        source_key: otto_core::new_id(),
        title: "s".into(),
        url: String::new(),
        issue_type: None,
        stage: "draft".into(),
        cwd: None,
        parent_id: None,
        tree_kind: "story".into(),
        folder: String::new(),
        created_by: by.to_string(),
    };
    let pr = &w.ctx.product_repo;
    let story_a = pr
        .create_story(mk_story(&w.ws_a, &w.alice.id))
        .await
        .unwrap();
    let ws_b = w
        .ctx
        .swarm_repo
        .get_swarm(&w.swarm_b)
        .await
        .unwrap()
        .workspace_id;
    let story_b = pr.create_story(mk_story(&ws_b, &w.alice.id)).await.unwrap();
    let mk_analysis = |sid: &str| otto_state::NewAnalysis {
        story_id: sid.to_string(),
        source_version_id: None,
        status: "running".into(),
        created_by: w.alice.id.clone(),
    };
    let an_a = pr.create_analysis(mk_analysis(&story_a.id)).await.unwrap();
    let an_b = pr.create_analysis(mk_analysis(&story_b.id)).await.unwrap();
    let agent_b = pr
        .add_analysis_agent(otto_state::NewAnalysisAgent {
            analysis_id: an_b.id.clone(),
            name: "Lens".into(),
            skill: "po-story-overview".into(),
            provider: "claude".into(),
            model: String::new(),
            status: "running".into(),
            session_id: None,
        })
        .await
        .unwrap();
    let uri = format!("/product/analyses/{}/agents/{}/stop", an_a.id, agent_b.id);
    let (st, _) = call(&w.app, &w.alice, Method::POST, &uri, json!({})).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    let still = pr.get_analysis_agent(&agent_b.id).await.unwrap();
    assert_eq!(still.status, "running", "foreign agent untouched");
}

/// S4-301: an agent that ran in worktree mode before S4-08 owns the legacy
/// branch `swarm/<s8>/<a8>`. The per-(agent, project) branch must not nest
/// under it (git refs are paths: "cannot lock ref"), or every pre-existing
/// worktree agent silently falls back to a scratch dir outside the repo.
#[tokio::test]
async fn worktree_survives_a_legacy_agent_branch() {
    let w = world().await;
    let repo_dir = tempfile::TempDir::new().unwrap();
    let repo = repo_dir.path();
    let git = |args: &[&str]| {
        std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .env("GIT_AUTHOR_NAME", "t")
            .env("GIT_AUTHOR_EMAIL", "t@t")
            .env("GIT_COMMITTER_NAME", "t")
            .env("GIT_COMMITTER_EMAIL", "t@t")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    };
    if !git(&["init", "-q", "-b", "main"]) {
        return; // no git on PATH — nothing to assert
    }
    std::fs::write(repo.join("README.md"), "hi\n").unwrap();
    assert!(git(&["add", "."]));
    assert!(git(&["commit", "-q", "-m", "init"]));
    let short = |id: &str| id[id.len() - id.len().min(8)..].to_string();
    let legacy = format!("swarm/{}/{}", short(&w.swarm_a), short(&w.agent_a));
    assert!(git(&["branch", &legacy]), "create the legacy agent branch");

    let repo_path = repo.to_string_lossy().to_string();
    w.ctx
        .swarm_repo
        .update_project(
            &w.project_a,
            otto_state::ProjectPatch {
                repo_path: Some(Some(repo_path.clone())),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let swarm = w.ctx.swarm_repo.get_swarm(&w.swarm_a).await.unwrap();
    let agent = w.ctx.swarm_repo.get_agent(&w.agent_a).await.unwrap();
    let project = w.ctx.swarm_repo.get_project(&w.project_a).await.unwrap();
    let info = otto_swarm::runtime::workspace::ensure_cwd_info(
        &w.ctx.swarm_rt(),
        &swarm,
        &agent,
        Some(&project),
    )
    .await
    .unwrap();
    assert_eq!(info.mode, "worktree", "fell back to scratch: {info:?}");
    let branch = info.branch.expect("agent branch");
    assert!(!branch.starts_with(&format!("{legacy}/")), "{branch}");
    assert!(std::path::Path::new(&info.path).join("README.md").exists());
    // The legacy branch (and any unmerged work on it) is left in place.
    assert!(git(&["rev-parse", "--verify", "-q", &legacy]));
}

/// S4-23: deleting a story stops its rewrite / test / plan runs too — their
/// story-keyed cancel flags are tripped (no recovery respawn) and every live
/// session attributed to the story is killed; another story's are untouched.
#[tokio::test]
async fn stop_story_agents_kills_rewrite_test_and_plan_sessions() {
    use otto_product::ProductCtx;
    let w = world().await;
    let mk_story = || otto_state::NewStory {
        workspace_id: w.ws_a.clone(),
        source_kind: "draft".into(),
        account_id: String::new(),
        source_key: otto_core::new_id(),
        title: "s".into(),
        url: String::new(),
        issue_type: None,
        stage: "draft".into(),
        cwd: None,
        parent_id: None,
        tree_kind: "story".into(),
        folder: String::new(),
        created_by: w.alice.id.clone(),
    };
    let pr = &w.ctx.product_repo;
    let doomed = pr.create_story(mk_story()).await.unwrap();
    let other = pr.create_story(mk_story()).await.unwrap();
    let sessions = otto_state::SessionsRepo::new(w.ctx.pool.clone());
    let mk_session = |sid: &str| otto_state::NewSession {
        workspace_id: w.ws_a.clone(),
        kind: otto_core::domain::SessionKind::Agent,
        provider: "claude".into(),
        title: "Product: rewrite".into(),
        cwd: "/tmp".into(),
        provider_session_id: None,
        connection_id: None,
        created_by: w.alice.id.clone(),
        meta: json!({ "work": { "story_id": sid, "origin": "product" } }),
    };
    let rewrite = sessions.create(mk_session(&doomed.id)).await.unwrap();
    let kept = sessions.create(mk_session(&other.id)).await.unwrap();
    // An in-flight rewrite run's flag (registered the way product_host does).
    let key = otto_product::run::story_run_cancel_key(&doomed.id);
    let other_key = otto_product::run::story_run_cancel_key(&other.id);
    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let other_flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut reg = w.ctx.product_agent_cancels.lock().unwrap();
        reg.insert(key, flag.clone());
        reg.insert(other_key, other_flag.clone());
    }

    w.ctx.stop_story_agents(&doomed.id).await;

    use std::sync::atomic::Ordering;
    assert!(flag.load(Ordering::SeqCst), "the story's run is cancelled");
    assert!(!other_flag.load(Ordering::SeqCst), "another story's run is not");
    let s = sessions.get(&rewrite.id).await.unwrap();
    assert_eq!(s.status, otto_core::domain::SessionStatus::Exited);
    let s = sessions.get(&kept.id).await.unwrap();
    assert_ne!(s.status, otto_core::domain::SessionStatus::Exited);
}

/// S4-20 / S4-26: the canvas Ask-AI stop route is workspace-scoped (a
/// non-member gets nothing), and with no turn running it reports
/// `stopping: false` without side effects.
#[tokio::test]
async fn canvas_assist_stop_is_scoped_and_idempotent() {
    let w = world().await;
    let app = Router::new()
        .route(
            "/canvas/scenes/{id}/assist/stop",
            post(otto_canvas::assist::stop_assist::<ServerCtx>),
        )
        .with_state(w.ctx.clone());
    let ws_b = w
        .ctx
        .swarm_repo
        .get_swarm(&w.swarm_b)
        .await
        .unwrap()
        .workspace_id;
    let mk = |ws: &str| otto_state::NewScene {
        workspace_id: ws.to_string(),
        story_id: None,
        title: "board".into(),
        doc_json: "{}".into(),
        provider: "claude".into(),
        section: None,
        created_by: w.alice.id.clone(),
    };
    let own = w.ctx.canvas_repo.create(mk(&w.ws_a)).await.unwrap();
    let foreign = w.ctx.canvas_repo.create(mk(&ws_b)).await.unwrap();

    let uri = format!("/canvas/scenes/{}/assist/stop", foreign.id);
    let (st, _) = call(&app, &w.alice, Method::POST, &uri, json!({})).await;
    assert!(
        st == StatusCode::FORBIDDEN || st == StatusCode::NOT_FOUND,
        "foreign scene: {st}"
    );
    let uri = format!("/canvas/scenes/{}/assist/stop", own.id);
    let (st, body) = call(&app, &w.alice, Method::POST, &uri, json!({})).await;
    assert_eq!(st, StatusCode::OK, "{body}");
    assert_eq!(body["stopping"], false, "no turn is running");
}
