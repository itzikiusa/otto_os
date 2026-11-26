//! Goals through the real daemon router (goal.md §11, the route and source
//! rows): starting a goal, Pause / Resume / Extend / Cancel on the stored
//! hold, steering, the verify route's refusals, MCP's Goal Loops request, the
//! older loops' read-only history and Continue as a goal. Every test runs in
//! a child of this binary with the offline agent and a PATH without agent
//! CLIs, because a started goal's run is real.

use chrono::Utc;
use otto_core::domain::{Capability, Feature, GoalLoopConfig, GoalLoopDefinition, GoalLoopLimits};
use otto_rbac::AuthRepo;
use otto_state::{DbPool, GoalLoopsRepo, GrantsRepo, NewGoalLoop};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

const CHILD: &str = "OTTO_WF_GOAL_CHILD";

/// Run `test` again in an offline child (throwaway HOME, PATH without agent
/// CLIs, the offline e2e agent). True inside the child.
fn in_offline_child(test: &str) -> bool {
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let home = tempfile::tempdir().unwrap();
    #[allow(clippy::disallowed_methods)] // a sync re-exec of this binary, like wf_compat
    let out = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            &format!("wf_goal::{test}"),
            "--nocapture",
            "--test-threads=1",
        ])
        .env(CHILD, "1")
        .env("OTTO_E2E", "1")
        .env("HOME", home.path())
        .env("TMPDIR", home.path())
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.status.success() && text.contains("1 passed"),
        "{test} in the offline child:\n{text}"
    );
    false
}

struct Daemon {
    pool: DbPool,
    base: String,
    http: reqwest::Client,
    /// Admin of ws1.
    root: String,
    /// Viewer of ws1 with Workflows View.
    viewer: String,
    tmp: tempfile::TempDir,
}

async fn exec(pool: &DbPool, sql: &'static str, binds: &[&str]) {
    let mut q = sqlx::query(sql);
    for b in binds {
        q = q.bind(b.to_string());
    }
    q.execute(pool)
        .await
        .unwrap_or_else(|e| panic!("{sql}: {e}"));
}

async fn boot() -> Daemon {
    let tmp = tempfile::tempdir().unwrap();
    let opts = SqliteConnectOptions::new()
        .filename(tmp.path().join("otto.db"))
        .create_if_missing(true)
        .foreign_keys(true);
    let raw = SqlitePoolOptions::new()
        .max_connections(4)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&raw)
        .await
        .unwrap();
    let pool: DbPool = raw.into();
    let now = Utc::now().to_rfc3339();
    for (id, root) in [("root", "1"), ("vera", "0")] {
        exec(&pool, "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, 'x', ?, ?, ?)",
            &[id, id, id, root, &now]).await;
    }
    exec(&pool, "INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at) VALUES ('ws1', 'ws1', '/tmp', '{}', 0, ?)", &[&now]).await;
    for (user, role) in [("root", "admin"), ("vera", "viewer")] {
        exec(
            &pool,
            "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES ('ws1', ?, ?)",
            &[user, role],
        )
        .await;
    }
    GrantsRepo::new(pool.clone())
        .set_grants("vera", &[(Feature::Workflows, Capability::View)])
        .await
        .unwrap();
    let auth = AuthRepo::new(pool.clone());
    let root = auth
        .issue_api_token(&"root".to_string(), Some("ui"))
        .await
        .unwrap()
        .0;
    let viewer = auth
        .issue_api_token(&"vera".to_string(), Some("ui"))
        .await
        .unwrap()
        .0;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut ctx = otto_server::ServerCtx::for_tests(&pool, tmp.path()).await;
    ctx.base_url = base.clone();
    let (api, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx, api, root_extras);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Daemon {
        pool,
        base,
        http: reqwest::Client::new(),
        root,
        viewer,
        tmp,
    }
}

impl Daemon {
    async fn call(
        &self,
        token: &str,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> (u16, Value) {
        let url = format!("{}/api/v1{path}", self.base);
        let req = match method {
            "GET" => self.http.get(url),
            "DELETE" => self.http.delete(url),
            _ => self.http.post(url).json(&body.unwrap_or(json!({}))),
        };
        let resp = req.bearer_auth(token).send().await.unwrap();
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    async fn scalar(&self, sql: &'static str, id: &str) -> Option<String> {
        sqlx::query_scalar(sql)
            .bind(id)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    /// A workflow with one harmless step and a run of it in `status`.
    async fn seed_run(
        &self,
        run: &str,
        status: &str,
        hold: Option<Value>,
        overview: Option<Value>,
    ) {
        let now = Utc::now().to_rfc3339();
        let (from, to) = ("start", "say");
        let graph = json!({"nodes": [{"id": from, "kind": "manual_trigger", "name": "Start"},
                                     {"id": to, "kind": "log", "name": "Say", "params": {"message": "hi"}}],
                           "edges": [{"id": "e1", "source": from, "target": to}]});
        let wf = format!("wf-{run}");
        exec(&self.pool, "INSERT INTO workflows (id, workspace_id, name, graph_json, created_by, created_at, updated_at) VALUES (?, 'ws1', 'Ship', ?, 'root', ?, ?)",
            &[&wf, &graph.to_string(), &now, &now]).await;
        let nodes =
            json!([{"node_id": from, "status": "success"}, {"node_id": to, "status": "running"}]);
        exec(&self.pool, "INSERT INTO workflow_runs (id, workflow_id, workspace_id, status, started_at, nodes_json, rev) VALUES (?, ?, 'ws1', ?, ?, ?, 1)",
            &[run, &wf, status, &now, &nodes.to_string()]).await;
        if let Some(hold) = hold {
            exec(
                &self.pool,
                "UPDATE workflow_runs SET hold_json = ? WHERE id = ?",
                &[&hold.to_string(), run],
            )
            .await;
        }
        if let Some(ov) = overview {
            exec(
                &self.pool,
                "UPDATE workflow_runs SET overview_json = ? WHERE id = ?",
                &[&ov.to_string(), run],
            )
            .await;
        }
    }

    async fn seed_loop(&self, mode: &str) -> otto_core::domain::GoalLoop {
        let definition: GoalLoopDefinition = serde_json::from_value(json!({"title": "Old loop", "summary": "Finish it",
            "acceptance_criteria": [{"id": "c1", "text": "done", "verify": "look", "verify_kind": "agent"}]})).unwrap();
        let config = GoalLoopConfig {
            mode: mode.into(),
            ..Default::default()
        };
        let repo = GoalLoopsRepo::new(self.pool.clone());
        let l = repo
            .create(NewGoalLoop {
                workspace_id: "ws1".into(),
                name: "Old loop".into(),
                repo_path: "/nowhere".into(),
                definition,
                limits: GoalLoopLimits::default(),
                config,
                created_by: "root".into(),
            })
            .await
            .unwrap();
        exec(
            &self.pool,
            "UPDATE goal_loops SET status = 'paused', context_digest = 'half done' WHERE id = ?",
            &[&l.id],
        )
        .await;
        repo.get(&l.id).await.unwrap()
    }
}

fn research_spec(title: &str) -> Value {
    json!({"title": title, "goal": "Find out why\nand write it down",
        "criteria": [{"id": "why", "text": "the cause is named", "verify": "agent", "how": "read findings.md"}],
        "work": {"kind": "research"}, "budget": {"iterations": 3, "active_s": 1800, "step_timeout_s": 600}})
}

/// proves: G-17 — the Goal form's route stores a goal workflow and starts its run in the long lane.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn starting_a_goal_stores_a_goal_workflow_and_a_long_lane_run() {
    if !in_offline_child("starting_a_goal_stores_a_goal_workflow_and_a_long_lane_run") {
        return;
    }
    let d = boot().await;
    let (st, body) = d
        .call(
            &d.root,
            "POST",
            "/workspaces/ws1/goals",
            Some(research_spec("Why slow")),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(
        (
            body["workflow"]["name"].clone(),
            body["workflow"]["origin"].clone()
        ),
        (json!("Goal · Why slow"), json!("goal"))
    );
    assert!(body["workflow"].get("origin_ref").is_none(), "{body}");
    let kinds: Vec<_> = body["workflow"]["graph"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["kind"].clone())
        .collect();
    assert_eq!(kinds, [json!("manual_trigger"), json!("loop")]);
    let run = body["run"]["id"].as_str().unwrap();
    assert_eq!(
        d.scalar("SELECT lane FROM workflow_runs WHERE id = ?", run)
            .await
            .as_deref(),
        Some("long")
    );
    assert_eq!(body["run"]["input"]["research_dir"], true);

    // Insights' handoff is recorded as the origin; nothing else from the client is.
    let mut spec = research_spec("From insights");
    spec["from"] = json!("insights");
    let (st, body) = d
        .call(&d.root, "POST", "/workspaces/ws1/goals", Some(spec))
        .await;
    assert_eq!(
        (st, body["workflow"]["origin_ref"].clone()),
        (200, json!("insights")),
        "{body}"
    );

    // A bad spec names its field; a Viewer cannot start one; listing is a read.
    let mut bad = research_spec("x");
    bad["budget"]["iterations"] = json!(0);
    let (st, body) = d
        .call(&d.root, "POST", "/workspaces/ws1/goals", Some(bad))
        .await;
    assert!(
        st == 400 && body.to_string().contains("budget.iterations"),
        "{st} {body}"
    );
    let (st, _) = d
        .call(
            &d.viewer,
            "POST",
            "/workspaces/ws1/goals",
            Some(research_spec("v")),
        )
        .await;
    assert_eq!(st, 403);
    let (st, list) = d
        .call(&d.viewer, "GET", "/workspaces/ws1/goals", None)
        .await;
    assert_eq!(st, 200, "{list}");
    let names: Vec<_> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap().to_string())
        .collect();
    assert!(
        names.contains(&"Goal · Why slow".to_string())
            && names.contains(&"Goal · From insights".to_string()),
        "{names:?}"
    );
    assert!(
        list[0]["run_id"].is_string() && list[0]["workflow_id"].is_string(),
        "{list}"
    );
}

/// proves: G-7, G-11 — Pause writes the hold, Resume needs room when out of
/// budget and writes the extension, Cancel of a held run drops the hold.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pause_resume_extend_and_cancel_work_on_the_stored_hold() {
    if !in_offline_child("pause_resume_extend_and_cancel_work_on_the_stored_hold") {
        return;
    }
    let d = boot().await;
    d.seed_run("r1", "running", None, None).await;
    let (st, body) = d
        .call(&d.root, "POST", "/workflow-runs/r1/pause", None)
        .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(
        (body["status"].clone(), body["hold"]["kind"].clone()),
        (json!("canceled"), json!("paused"))
    );
    assert_eq!(body["hold"]["reason"], "Paused by root");
    let (st, _) = d
        .call(&d.root, "POST", "/workflow-runs/r1/pause", None)
        .await;
    assert_eq!(st, 409, "a paused run is not paused twice");
    let (st, body) = d
        .call(&d.viewer, "POST", "/workflow-runs/r1/resume", None)
        .await;
    assert_eq!(st, 403, "{body}");
    let (st, body) = d
        .call(&d.root, "POST", "/workflow-runs/r1/resume", Some(json!({})))
        .await;
    assert_eq!(st, 200, "{body}");
    assert!(body.get("hold").is_none_or(Value::is_null), "{body}");
    assert_eq!(
        d.scalar("SELECT hold_json FROM workflow_runs WHERE id = ?", "r1")
            .await,
        None
    );
    let (st, _) = d
        .call(&d.root, "POST", "/workflow-runs/r1/resume", Some(json!({})))
        .await;
    assert_eq!(st, 409, "nothing to resume once the hold is gone");

    // Out of iterations: 3 of 3 used.
    let hold = json!({"kind": "exhausted", "reason": "Out of iterations: 3 of 3 used", "since": Utc::now(), "node_ids": ["say"], "iteration": 3});
    let goal = json!({"iteration": 3, "max_iterations": 3, "criteria_met": 0, "criteria_total": 1, "active_ms": 1000});
    d.seed_run(
        "r2",
        "error",
        Some(hold),
        Some(json!({"rev": 1, "goal": goal})),
    )
    .await;
    let (st, body) = d
        .call(&d.root, "POST", "/workflow-runs/r2/resume", Some(json!({})))
        .await;
    assert_eq!(
        (
            st,
            body.to_string()
                .contains("Raise the iteration limit to resume: 3 of 3 iterations are used.")
        ),
        (409, true),
        "{body}"
    );
    let (st, body) = d
        .call(
            &d.root,
            "POST",
            "/workflow-runs/r2/resume",
            Some(json!({"extend": {"iterations": 5}})),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    let limits: Value = serde_json::from_str(
        &d.scalar("SELECT limits_json FROM workflow_runs WHERE id = ?", "r2")
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(limits, json!({"loops": {"say": {"max_iterations": 5}}}));

    // Cancel of a blocked run keeps its settled status and drops the hold.
    let hold = json!({"kind": "blocked", "reason": "stuck", "since": Utc::now(), "node_ids": []});
    d.seed_run("r3", "error", Some(hold), None).await;
    let (st, body) = d
        .call(&d.root, "POST", "/workflow-runs/r3/cancel", None)
        .await;
    assert_eq!(
        (st, body["status"].clone()),
        (200, json!("error")),
        "{body}"
    );
    assert_eq!(
        d.scalar("SELECT hold_json FROM workflow_runs WHERE id = ?", "r3")
            .await,
        None
    );
}

/// proves: G-15, G-2 — a note is queued in the run folder while the run can
/// take it; a person's record is refused unless the check waits on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn steering_queues_notes_and_verify_needs_a_waiting_check() {
    if !in_offline_child("steering_queues_notes_and_verify_needs_a_waiting_check") {
        return;
    }
    let d = boot().await;
    let hold =
        json!({"kind": "paused", "reason": "Paused by root", "since": Utc::now(), "node_ids": []});
    d.seed_run("r1", "canceled", Some(hold), None).await;
    let (st, body) = d
        .call(
            &d.root,
            "POST",
            "/workflow-runs/r1/steer",
            Some(json!({"note": " try the cache "})),
        )
        .await;
    assert_eq!(
        (st, body.clone()),
        (200, json!({"queued": ["try the cache"]})),
        "{body}"
    );
    let queued =
        std::fs::read_to_string(d.tmp.path().join("workflow-context/r1/steering.json")).unwrap();
    assert_eq!(queued, r#"["try the cache"]"#);
    let (st, _) = d
        .call(
            &d.root,
            "POST",
            "/workflow-runs/r1/steer",
            Some(json!({"note": "  "})),
        )
        .await;
    assert_eq!(st, 400);
    d.seed_run("r2", "success", None, None).await;
    let (st, _) = d
        .call(
            &d.root,
            "POST",
            "/workflow-runs/r2/steer",
            Some(json!({"note": "late"})),
        )
        .await;
    assert_eq!(st, 409, "a finished run takes no note");

    let verify = "/workflow-runs/r1/nodes/say/criteria/c1/verify";
    let (st, _) = d
        .call(
            &d.root,
            "POST",
            verify,
            Some(json!({"met": true, "evidence": " "})),
        )
        .await;
    assert_eq!(st, 400, "empty evidence");
    let (st, body) = d
        .call(
            &d.root,
            "POST",
            verify,
            Some(json!({"met": true, "evidence": "looked"})),
        )
        .await;
    assert_eq!(st, 409, "{body}");
}

/// proves: G-19 — MCP's Goal Loops request starts a goal and names the roles it dropped.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_goal_loop_request_starts_a_goal_and_refuses_engineering_graphs() {
    if !in_offline_child("a_goal_loop_request_starts_a_goal_and_refuses_engineering_graphs") {
        return;
    }
    let d = boot().await;
    let mut req = json!({"name": "n", "repo_path": "/nowhere",
        "definition": {"title": "Digest", "summary": "Summarise the logs",
            "acceptance_criteria": [{"id": "AC-1", "text": "a summary exists", "verify": "test -s findings.md", "verify_kind": "command"}]},
        "limits": {"max_iterations": 2, "max_runtime_secs": 1800, "per_phase_timeout_secs": 600},
        "config": serde_json::to_value(GoalLoopConfig { mode: "research".into(), ..Default::default() }).unwrap()});
    let (st, body) = d
        .call(
            &d.root,
            "POST",
            "/workspaces/ws1/goals/from-loop",
            Some(req.clone()),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(body["ignored"], json!(["planner", "digester", "definer"]));
    assert_eq!(body["workflow"]["origin_ref"], "mcp");
    assert!(body["run_id"].is_string() && body["workflow_id"].is_string());
    let check =
        &body["workflow"]["graph"]["nodes"][1]["params"]["steps"][1]["params"]["criteria"][0];
    assert_eq!(
        (check["id"].clone(), check["command"].clone()),
        (json!("ac-1"), json!("test -s findings.md"))
    );
    req["config"]["graph"] = json!({"nodes": []});
    let (st, body) = d
        .call(
            &d.root,
            "POST",
            "/workspaces/ws1/goals/from-loop",
            Some(req.clone()),
        )
        .await;
    assert!(
        st == 400 && body.to_string().contains("workflow canvas"),
        "{st} {body}"
    );
    req["config"]["mode"] = json!("build");
    req["config"]["graph"] = Value::Null;
    let (st, body) = d
        .call(
            &d.root,
            "POST",
            "/workspaces/ws1/goals/from-loop",
            Some(req),
        )
        .await;
    assert!(
        st == 400 && body.to_string().contains("registered repository"),
        "{st} {body}"
    );
}

/// proves: G-20 — older loops stay readable, the controller's routes are gone,
/// and a paused loop continues as a goal once, in its own folder.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn older_loops_are_read_only_history_and_continue_as_a_goal_once() {
    if !in_offline_child("older_loops_are_read_only_history_and_continue_as_a_goal_once") {
        return;
    }
    let d = boot().await;
    let l = d.seed_loop("research").await;
    let (st, list) = d
        .call(&d.viewer, "GET", "/workspaces/ws1/goal-loops", None)
        .await;
    assert_eq!((st, list[0]["id"].clone()), (200, json!(l.id)), "{list}");
    let (st, detail) = d
        .call(
            &d.viewer,
            "GET",
            &format!("/goal-loops/{}?summary=true", l.id),
            None,
        )
        .await;
    assert_eq!(st, 200, "{detail}");
    assert!(
        detail["spent_usd"].is_null() && detail["steering_notes"] == json!([]),
        "{detail}"
    );
    for gone in ["start", "pause", "resume", "stop", "steer"] {
        let (st, _) = d
            .call(
                &d.root,
                "POST",
                &format!("/goal-loops/{}/{gone}", l.id),
                None,
            )
            .await;
        assert!(st == 404 || st == 405, "{gone}: {st}");
    }
    let (_, goals) = d.call(&d.root, "GET", "/workspaces/ws1/goals", None).await;
    assert_eq!(
        (goals[0]["id"].clone(), goals[0]["legacy"].clone()),
        (json!(l.id), json!(true)),
        "{goals}"
    );

    let (st, body) = d
        .call(
            &d.root,
            "POST",
            &format!("/goal-loops/{}/continue", l.id),
            Some(research_spec("Old loop")),
        )
        .await;
    assert_eq!(st, 200, "{body}");
    assert_eq!(
        body["workflow"]["origin_ref"],
        format!("goal_loop:{}", l.id)
    );
    let kept = d.tmp.path().join("goal-loops").join(&l.id).join("work");
    assert_eq!(
        body["run"]["input"]["research_dir"],
        json!(kept.to_string_lossy())
    );
    let prompt = body["workflow"]["graph"]["nodes"][1]["params"]["steps"][0]["params"]["prompt"]
        .as_str()
        .unwrap();
    assert!(
        prompt.contains("Where the earlier loop got to: half done"),
        "{prompt}"
    );
    let after = GoalLoopsRepo::new(d.pool.clone()).get(&l.id).await.unwrap();
    assert_eq!(after.status.as_str(), "stopped");
    assert_eq!(
        after.summary.unwrap(),
        format!(
            "Continued as a goal — run {}",
            body["run"]["id"].as_str().unwrap()
        )
    );
    let (st, _) = d
        .call(
            &d.root,
            "POST",
            &format!("/goal-loops/{}/continue", l.id),
            Some(research_spec("again")),
        )
        .await;
    assert_eq!(st, 409, "a loop is continued once");

    let (st, _) = d
        .call(&d.root, "DELETE", &format!("/goal-loops/{}", l.id), None)
        .await;
    assert_eq!(st, 204);
    let (_, list) = d
        .call(&d.root, "GET", "/workspaces/ws1/goal-loops", None)
        .await;
    assert_eq!(list, json!([]));
}

/// proves: G-20, GL18.3 / GL24.7 — a loop still running when this build boots
/// is paused with its work kept, and one notice offers Continue as a goal.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_loop_running_at_upgrade_is_paused_with_one_notice() {
    if !in_offline_child("a_loop_running_at_upgrade_is_paused_with_one_notice") {
        return;
    }
    let d = boot().await;
    let l = d.seed_loop("build").await;
    exec(
        &d.pool,
        "UPDATE goal_loops SET status = 'running' WHERE id = ?",
        &[&l.id],
    )
    .await;
    let ctx = otto_server::ServerCtx::for_tests(&d.pool, d.tmp.path()).await;
    otto_server::boot::recover_goal_loops(&ctx).await;
    let after = GoalLoopsRepo::new(d.pool.clone()).get(&l.id).await.unwrap();
    assert_eq!(after.status.as_str(), "paused");
    assert_eq!(
        after.error.as_deref(),
        Some(otto_automation::goal_loop_history::RESTART_PAUSE_NOTE)
    );
    let key = format!("automation:goal_loop_restart:{}", l.id);
    let title = d
        .scalar("SELECT title FROM notifications WHERE source_key = ?", &key)
        .await;
    assert!(
        title
            .as_deref()
            .is_some_and(|t| t.contains("paused by the update")),
        "{title:?}"
    );
}
