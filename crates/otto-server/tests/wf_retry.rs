//! Retry step and Retry from here through the real daemon router and the
//! parallel scheduler (`POST /workflow-runs/{id}/retry-node`): the retried
//! step runs again — alone, with its downstream, in the middle of a chain,
//! on one branch of a fork with a join after it — and a succeeded step's
//! Retry from here re-runs it and what follows. Every run starts with no
//! input, as the UI's Run does: the Start step's output is then JSON null,
//! which the run row stores as no output at all. And a plan usage limit
//! whose words name the reset parks its step until then (runtime.md §1.7)
//! instead of being retried by backoff.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::Utc;
use otto_core::workflows::{NodeRunState, NodeStatus, RunStatus, WorkflowRun};
use otto_rbac::AuthRepo;
use otto_state::{DbPool, WorkflowsRepo};
use serde_json::{json, Value};

const WS: &str = "ws-retry";

struct Daemon {
    pool: DbPool,
    base: String,
    http: reqwest::Client,
    token: String,
    root: PathBuf,
    ctx: otto_server::ServerCtx,
    _tmp: tempfile::TempDir,
}

async fn boot() -> Daemon {
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    std::fs::create_dir_all(root.join("data")).unwrap();
    let pool = otto_state::open(&root.join("data/otto.db")).await.unwrap();
    let now = Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES ('root', 'root', 'x', 'Root', 1, ?)")
        .bind(&now).execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, settings_json, archived, created_at) VALUES (?, 'Retry', ?, '{}', 0, ?)")
        .bind(WS).bind(root.to_string_lossy()).bind(&now).execute(&pool).await.unwrap();
    sqlx::query(
        "INSERT INTO workspace_members (workspace_id, user_id, role) VALUES (?, 'root', 'admin')",
    )
    .bind(WS)
    .execute(&pool)
    .await
    .unwrap();
    let token = AuthRepo::new(pool.clone())
        .issue_api_token(&"root".to_string(), Some("ui"))
        .await
        .unwrap()
        .0;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut ctx = otto_server::ServerCtx::for_tests(&pool, root.join("data")).await;
    ctx.base_url = base.clone();
    // Agent steps run on the in-memory fakes.
    for d in ["scripts", "fake-state"] {
        std::fs::create_dir_all(root.join(d)).unwrap();
    }
    ctx.agent_runtime = crate::wf_restart::runtime(&pool, &ctx, &root);
    otto_state::SettingsRepo::new(pool.clone())
        .put("agent_admission_enabled", &json!(false))
        .await
        .unwrap();
    let (api, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api, root_extras);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Daemon {
        pool,
        base,
        http: reqwest::Client::new(),
        token,
        root,
        ctx,
        _tmp: tmp,
    }
}

/// A step that fails until `marker` exists (no automatic retry).
fn flaky(id: &str, marker: &Path) -> Value {
    json!({"id": id, "kind": "script", "name": id, "retry": {"max_attempts": 0},
           "params": {"command": format!("test -f '{}'", marker.display()), "timeout_s": 30}})
}

fn log(id: &str) -> Value {
    json!({"id": id, "kind": "log", "name": id})
}

fn edge(from: &str, to: &str) -> Value {
    json!({"id": format!("{from}-{to}"), "source": from, "target": to})
}

/// Start → `steps` one after another.
fn chain(steps: Vec<Value>) -> Value {
    let mut nodes = vec![json!({"id": "start", "kind": "manual_trigger", "name": "Start"})];
    nodes.extend(steps);
    let edges: Vec<Value> = nodes
        .windows(2)
        .map(|w| edge(w[0]["id"].as_str().unwrap(), w[1]["id"].as_str().unwrap()))
        .collect();
    json!({"nodes": nodes, "edges": edges})
}

impl Daemon {
    async fn post(&self, path: &str, body: Value) -> (u16, Value) {
        let resp = self
            .http
            .post(format!("{}/api/v1{path}", self.base))
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        (
            status,
            serde_json::from_str(&text).unwrap_or(Value::String(text)),
        )
    }

    async fn save(&self, graph: Value) -> String {
        let (st, wf) = self
            .post(
                &format!("/workspaces/{WS}/workflows"),
                json!({"name": "Retry", "graph": graph}),
            )
            .await;
        assert_eq!(st, 200, "{wf}");
        wf["id"].as_str().unwrap().to_string()
    }

    /// Save `graph` and run it with no input, to its end.
    async fn run(&self, graph: Value) -> WorkflowRun {
        let wf = self.save(graph).await;
        let (st, run) = self.post(&format!("/workflows/{wf}/run"), json!({})).await;
        assert_eq!(st, 200, "{run}");
        self.settled(run["id"].as_str().unwrap()).await
    }

    async fn retry(&self, run: &WorkflowRun, node: &str, downstream: bool) -> WorkflowRun {
        let body = json!({"node_id": node, "include_downstream": downstream});
        let (st, out) = self
            .post(&format!("/workflow-runs/{}/retry-node", run.id), body)
            .await;
        assert_eq!(st, 200, "{out}");
        self.settled(&run.id).await
    }

    async fn settled(&self, id: &str) -> WorkflowRun {
        let repo = WorkflowsRepo::new(self.pool.clone());
        let id: otto_core::Id = id.into();
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            let run = repo.get_run(&id).await.unwrap();
            if matches!(
                run.status,
                RunStatus::Success | RunStatus::Error | RunStatus::Canceled
            ) && !otto_server::workflow_engine::driver_alive(&id)
            {
                return run;
            }
            assert!(
                Instant::now() < deadline,
                "{id} did not settle: {:?}",
                run.nodes
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

fn node<'a>(run: &'a WorkflowRun, id: &str) -> &'a NodeRunState {
    run.nodes
        .iter()
        .find(|n| n.node_id == id)
        .unwrap_or_else(|| panic!("no {id}: {:?}", run.nodes))
}

/// `id` ran in the retry and succeeded (not skipped past).
fn ran_again(before: &WorkflowRun, after: &WorkflowRun, id: &str) {
    let (b, a) = (node(before, id), node(after, id));
    assert_eq!(
        a.status,
        NodeStatus::Success,
        "{id}: {:?} {:?}",
        a.logs,
        a.error
    );
    assert!(
        a.started_at.is_some() && a.started_at != b.started_at,
        "{id} did not run again: {a:?}"
    );
}

/// `id` kept its earlier state: not re-run, same step number.
fn adopted(before: &WorkflowRun, after: &WorkflowRun, id: &str) {
    let (b, a) = (node(before, id), node(after, id));
    assert_eq!(
        (a.status, a.started_at, a.step_no),
        (b.status, b.started_at, b.step_no),
        "{id}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_step_runs_again_alone_and_with_its_downstream() {
    let d = boot().await;
    for downstream in [false, true] {
        let marker = d.root.join(format!("ok-{downstream}"));
        let failed = d.run(chain(vec![flaky("flaky", &marker)])).await;
        assert_eq!(failed.status, RunStatus::Error, "{:?}", failed.nodes);
        assert_eq!(node(&failed, "flaky").status, NodeStatus::Error);
        std::fs::write(&marker, "").unwrap();
        let after = d.retry(&failed, "flaky", downstream).await;
        assert_eq!(
            after.status,
            RunStatus::Success,
            "{downstream}: {:?}",
            after.nodes
        );
        ran_again(&failed, &after, "flaky");
        adopted(&failed, &after, "start");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_middle_step_reruns_with_the_steps_it_blocked() {
    let d = boot().await;
    let marker = d.root.join("ok");
    let failed = d
        .run(chain(vec![log("a"), flaky("flaky", &marker), log("c")]))
        .await;
    assert_eq!(failed.status, RunStatus::Error);
    assert_eq!(node(&failed, "c").status, NodeStatus::Skipped);
    std::fs::write(&marker, "").unwrap();
    let after = d.retry(&failed, "flaky", false).await;
    assert_eq!(after.status, RunStatus::Success, "{:?}", after.nodes);
    adopted(&failed, &after, "a");
    ran_again(&failed, &after, "flaky");
    ran_again(&failed, &after, "c");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_failed_branch_of_a_fork_reruns_and_its_join_follows() {
    let d = boot().await;
    let marker = d.root.join("ok");
    let graph = json!({
        "nodes": [{"id": "start", "kind": "manual_trigger", "name": "Start"},
                  flaky("p", &marker), log("q"), log("join")],
        "edges": [edge("start", "p"), edge("start", "q"), edge("p", "join"), edge("q", "join")],
    });
    let failed = d.run(graph).await;
    assert_eq!(failed.status, RunStatus::Error);
    assert_eq!(node(&failed, "q").status, NodeStatus::Success);
    assert_eq!(node(&failed, "join").status, NodeStatus::Skipped);
    std::fs::write(&marker, "").unwrap();
    let after = d.retry(&failed, "p", false).await;
    assert_eq!(after.status, RunStatus::Success, "{:?}", after.nodes);
    adopted(&failed, &after, "q");
    ran_again(&failed, &after, "p");
    ran_again(&failed, &after, "join");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn retry_from_here_on_a_succeeded_step_reruns_it_and_what_follows() {
    let d = boot().await;
    let done = d.run(chain(vec![log("a"), log("b")])).await;
    assert_eq!(done.status, RunStatus::Success, "{:?}", done.nodes);
    let after = d.retry(&done, "a", true).await;
    assert_eq!(after.status, RunStatus::Success, "{:?}", after.nodes);
    adopted(&done, &after, "start");
    ran_again(&done, &after, "a");
    ran_again(&done, &after, "b");
}

/// The restart re-entry adopts the same way: a run with no input cut by a
/// restart while its second step ran continues that step and the rest.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_run_cut_by_a_restart_continues_after_a_start_with_no_output() {
    let d = boot().await;
    let graph = chain(vec![
        json!({"id": "wait", "kind": "delay", "name": "wait", "params": {"ms": 50}}),
        log("c"),
    ]);
    let wf = d.save(graph).await;
    let repo = WorkflowsRepo::new(d.pool.clone());
    let run = repo
        .create_run(&wf, &WS.into(), &Value::Null, Some(&"root".into()))
        .await
        .unwrap();
    // The row a dead daemon leaves: Start done (its null output stored as
    // none), the delay running since a minute ago.
    let ago = Utc::now() - chrono::Duration::seconds(60);
    let nodes = json!([
        {"node_id": "start", "status": "success", "started_at": ago, "duration_ms": 1, "step_no": 1},
        {"node_id": "wait", "status": "running", "started_at": ago, "step_no": 2},
        {"node_id": "c", "status": "pending"},
    ]);
    sqlx::query(
        "UPDATE workflow_runs SET status = 'running', started_at = ?, nodes_json = ? WHERE id = ?",
    )
    .bind(ago)
    .bind(nodes.to_string())
    .bind(&run.id)
    .execute(&d.pool)
    .await
    .unwrap();
    otto_server::boot::recover_before_serve(&d.ctx, &mut otto_server::boot::BootPhases::start())
        .await
        .unwrap();
    let after = d.settled(&run.id).await;
    assert_eq!(after.status, RunStatus::Success, "{:?}", after.nodes);
    for id in ["wait", "c"] {
        let n = node(&after, id);
        assert_eq!(n.status, NodeStatus::Success, "{id}: {:?}", n.logs);
    }
}

/// The fake Claude's limit says "resets 3pm", as the CLI does: the step
/// parks in needs you with the reset, never retried by backoff.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_usage_limit_that_names_its_reset_parks_the_step() {
    let d = boot().await;
    let agent = json!({"id": "a", "kind": "agent_prompt", "name": "Hit the limit",
        "params": {"provider": "claude", "prompt": "Do it.\nOTTO_FAKE: fail usage_limit"}});
    let wf = d.save(chain(vec![agent])).await;
    let (st, run) = d.post(&format!("/workflows/{wf}/run"), json!({})).await;
    assert_eq!(st, 200, "{run}");
    let id: otto_core::Id = run["id"].as_str().unwrap().into();
    let repo = WorkflowsRepo::new(d.pool.clone());
    let deadline = Instant::now() + Duration::from_secs(30);
    let parked = loop {
        let run = repo.get_run(&id).await.unwrap();
        let waits = run.overview.as_ref().map(|o| o.state)
            == Some(otto_core::workflows::RunState::NeedsYou);
        if waits {
            break run;
        }
        assert!(
            Instant::now() < deadline
                && matches!(run.status, RunStatus::Pending | RunStatus::Running),
            "the step did not park: {:?} {:?}",
            run.status,
            run.nodes
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let attention = &parked.overview.as_ref().unwrap().attention[0].attention;
    assert_eq!(
        attention.kind,
        otto_core::agent_runtime::AttentionKind::UsageLimit
    );
    assert!(
        attention.resets_at.is_some_and(|t| t > Utc::now()),
        "{attention:?}"
    );
    let a = node(&parked, "a");
    assert_eq!(a.attempts, Some(1), "{:?}", a.logs);
    assert!(!a.logs.iter().any(|l| l.contains("retry")), "{:?}", a.logs);
    let (st, _) = d
        .post(&format!("/workflow-runs/{id}/cancel"), json!({}))
        .await;
    assert_eq!(st, 200);
    assert_eq!(d.settled(&id).await.status, RunStatus::Canceled);
}
