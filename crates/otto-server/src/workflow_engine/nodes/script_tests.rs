//! The `script` node executed for real (`/bin/sh -c` through
//! `verify_command`) on a run with a real context directory — test-plan.md
//! §5.13 and DR-029/DR-202. The fixture is shared with the other
//! deterministic nodes' tests.
use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};
use crate::workflow_context::{RepoEntry, RunContextFiles};
use crate::workflow_engine::*;
use std::sync::Arc;

pub(in crate::workflow_engine) struct NodeFixture {
    pub dir: tempfile::TempDir,
    pub ctx: ServerCtx,
    pub ws: otto_core::domain::Workspace,
    pub user: otto_core::domain::User,
    pub run_id: Id,
    pub files: Arc<RunContextFiles>,
}

impl NodeFixture {
    pub async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let pool = mem_pool().await;
        seed_workspace(&pool, "nodes-ws").await;
        sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
            .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
        let ctx = test_ctx(&pool, dir.path().join("data")).await;
        let ws = otto_state::WorkspacesRepo::new(pool.clone())
            .get(&"nodes-ws".into())
            .await
            .unwrap();
        let user = root_user();
        let graph: WorkflowGraph = serde_json::from_value(
            json!({"nodes":[{"id":"s","kind":"log","params":{}}],"edges":[]}),
        )
        .unwrap();
        let repo = WorkflowsRepo::new(pool);
        let wf = repo
            .create(&ws.id, "Nodes", "", "", &graph, &user.id)
            .await
            .unwrap();
        let run = repo
            .create_run(&wf.id, &ws.id, &Value::Null, Some(&user.id))
            .await
            .unwrap();
        let files = Arc::new(RunContextFiles::create(&ctx.data_dir, &run.id));
        Self {
            dir,
            ctx,
            ws,
            user,
            run_id: run.id,
            files,
        }
    }

    pub fn ctx_dir(&self) -> std::path::PathBuf {
        self.files.dir_str().unwrap().into()
    }

    /// Execute one node as step 2 of this run.
    pub async fn run(&self, node: Value, run_input: Value) -> Result<(Value, Vec<String>)> {
        let node: WorkflowNode = serde_json::from_value(node).unwrap();
        let env = RunEnv {
            budget: budget::ActiveBudget::new(RUN_WALL_CLOCK_TIMEOUT),
            run_id: self.run_id.clone(),
            wf_name: "Nodes".into(),
            run_cwd: self.dir.path().to_string_lossy().into(),
            run_base: None,
            files: self.files.clone(),
            default_provider: "no-such-cli".into(),
            run_input: run_input.clone(),
        };
        let (sessions, _sr) = tokio::sync::mpsc::unbounded_channel();
        let (logs, _lr) = tokio::sync::mpsc::unbounded_channel();
        let (activity, _ar) = tokio::sync::mpsc::unbounded_channel();
        let progress = ProgressSink::disabled();
        let step = execute_node(
            &self.ctx,
            &self.ws,
            &self.user,
            &node,
            run_input,
            &env,
            &StepScope {
                step_no: 2,
                iter: None,
                inner_idx: None,
            },
            &sessions,
            &logs,
            &activity,
            &progress,
        );
        tokio::time::timeout(Duration::from_secs(60), step)
            .await
            .expect("the node settles")
    }
}

fn script(params: Value) -> Value {
    json!({"id": "gates", "kind": "script", "name": "Gates", "params": params})
}

fn entry(name: &str, wt: &std::path::Path, base: &str) -> RepoEntry {
    RepoEntry {
        repo: name.into(),
        repo_name: Some(name.into()),
        kind: "worktree".into(),
        name: name.into(),
        worktree: Some(wt.to_string_lossy().into()),
        base: Some(base.into()),
        base_branch: Some("develop".into()),
        ..Default::default()
    }
}

#[tokio::test]
async fn exit_codes_decide_the_status() {
    let f = NodeFixture::new().await;
    let err = f
        .run(
            script(json!({"command": "echo one; echo two; exit 3"})),
            json!({}),
        )
        .await
        .expect_err("exit 3 is not in [0]");
    let msg = err.to_string();
    assert!(msg.contains("exit code 3") && msg.contains("two"), "{msg}");
    let (out, _) = f
        .run(
            script(json!({"command": "exit 3", "ok_exit_codes": [0, 3]})),
            json!({}),
        )
        .await
        .unwrap();
    assert_eq!(out["exit_code"], 3);
    let (out, _) = f
        .run(
            script(json!({"command": "echo table; exit 1", "allow_failure": true})),
            json!({}),
        )
        .await
        .expect("allow_failure keeps a failing gate table a successful step");
    assert_eq!(out["exit_code"], 1);
    assert!(out["reply"].as_str().unwrap().contains("table"));
    let (out, _) = f
        .run(script(json!({"command": "echo $0"})), json!({}))
        .await
        .unwrap();
    assert!(
        out["reply"].as_str().unwrap().starts_with("/bin/sh"),
        "{out}"
    );
}

/// The 15-minute linger class (runs.md §6.3): a backgrounded child neither
/// keeps the step open nor outlives a timeout.
#[tokio::test]
async fn timeout_kills_the_group_and_a_background_child_cannot_hold_the_step() {
    let f = NodeFixture::new().await;
    let t0 = std::time::Instant::now();
    let (out, _) = f
        .run(
            script(json!({"command": "sleep 30 & echo started", "timeout_s": 60})),
            json!({}),
        )
        .await
        .unwrap();
    assert!(
        t0.elapsed() < Duration::from_secs(10),
        "lingered {:?}",
        t0.elapsed()
    );
    assert!(out["reply"].as_str().unwrap().contains("started"));

    let late = f.ctx_dir().join("late");
    let t0 = std::time::Instant::now();
    let err = f
        .run(
            script(json!({"command": format!("(sleep 11; touch '{}') & sleep 60", late.display()), "timeout_s": 10})),
            json!({}),
        )
        .await
        .expect_err("times out");
    assert!(err.to_string().contains("timed out after 10 s"), "{err}");
    assert!(t0.elapsed() < Duration::from_secs(14), "{:?}", t0.elapsed());
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(!late.exists(), "a child of the timed-out command survived");
}

#[tokio::test]
async fn foreach_repos_passes_values_as_env_never_as_shell_text() {
    let f = NodeFixture::new().await;
    let (a, b) = (f.dir.path().join("a"), f.dir.path().join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let pwned = f.dir.path().join("pwned");
    let evil = format!("$(touch {})", pwned.display());
    f.files.set_repos(vec![
        entry("alpha", &a, &evil),
        entry("beta", &b, "abc123"),
        RepoEntry {
            error: Some("unresolved".into()),
            ..entry("broken", &b, "x")
        },
    ]);
    let node = script(json!({
        "command": "printf '%s|%s|%s|%s\\n' \"$OTTO_REPO_NAME\" \"$OTTO_BASE\" \"$(pwd)\" \"$OTTO_BASE_BRANCH\" >> \"$OTTO_CONTEXT_DIR/$OTTO_OUT\"; echo ran-$OTTO_REPO_NAME",
        "foreach": "repos", "output_file": "gates.md", "idempotent": true,
    }));
    // Twice: a re-run (retry / resumed idempotent step) starts the file afresh.
    f.run(node.clone(), json!({})).await.unwrap();
    let (out, logs) = f.run(node, json!({})).await.unwrap();
    let reply = out["reply"].as_str().unwrap();
    let lines: Vec<&str> = reply.lines().collect();
    assert_eq!(lines.len(), 2, "{reply}");
    assert!(lines[0].starts_with(&format!("alpha|{evil}|")), "{reply}");
    assert!(lines[0].ends_with("|develop"), "{reply}");
    assert!(
        lines[1].contains(&b.file_name().unwrap().to_string_lossy().to_string()),
        "cwd = the entry's worktree: {reply}"
    );
    assert!(!pwned.exists(), "a branch value was run as shell");
    assert_eq!(
        out["runs"].as_array().unwrap().len(),
        2,
        "the errored entry is skipped"
    );
    assert_eq!(out["output_file"], "gates.md");
    let log = std::fs::read_to_string(f.ctx_dir().join("step2-gates.log")).unwrap();
    assert!(
        log.contains("ran-alpha") && log.contains("ran-beta"),
        "{log}"
    );
    assert!(logs.iter().any(|l| l.contains("ran-beta")), "{logs:?}");
}

#[tokio::test]
async fn foreach_zero_repos_succeeds_and_bad_output_names_fail() {
    let f = NodeFixture::new().await;
    let (out, _) = f
        .run(
            script(json!({"command": "exit 9", "foreach": "repos"})),
            json!({}),
        )
        .await
        .unwrap();
    assert_eq!(out["reply"], "no repositories to run on");
    let err = f
        .run(
            script(json!({"command": "true", "output_file": "../escape.md"})),
            json!({}),
        )
        .await
        .expect_err("an output_file outside the run dir is refused");
    assert!(err.to_string().contains("bare file name"), "{err}");
}
