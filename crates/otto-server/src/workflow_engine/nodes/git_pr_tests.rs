//! test-quality-5-07: the `git_pr` node executed for real against a registered
//! temp repository (the `workflow_product_publish_tests` harness shape). The
//! draft agent turn is never reached: on the own-checkout case the branch has
//! no diff ("nothing to PR"), and the worktree case pins a provider no
//! registry knows, so the draft fails before any CLI could spawn.
use crate::routes::browser::tests::{mem_pool, root_user, seed_workspace, test_ctx};
use crate::workflow_engine::*;
use std::path::Path;

#[allow(clippy::disallowed_methods)] // test fixture setup: a short sync git call
fn git(dir: &Path, args: &[&str]) -> String {
    let out = otto_git::hardened_std_command()
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

struct Fixture {
    _dir: tempfile::TempDir,
    ctx: ServerCtx,
    ws: otto_core::domain::Workspace,
    user: otto_core::domain::User,
    run_id: Id,
    repo: otto_core::domain::Repo,
}

impl Fixture {
    async fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let checkout = dir.path().join("checkout");
        std::fs::create_dir(&checkout).unwrap();
        git(&checkout, &["init", "-q", "-b", "main"]);
        git(&checkout, &["config", "user.email", "t@example.invalid"]);
        git(&checkout, &["config", "user.name", "T"]);
        git(&checkout, &["config", "commit.gpgsign", "false"]);
        std::fs::write(checkout.join("README.md"), "base\n").unwrap();
        git(&checkout, &["add", "README.md"]);
        git(&checkout, &["commit", "-q", "-m", "base"]);

        let pool = mem_pool().await;
        seed_workspace(&pool, "gitpr-ws").await;
        sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
            .bind(chrono::Utc::now().to_rfc3339()).execute(&pool).await.unwrap();
        let ctx = test_ctx(&pool, dir.path().join("data")).await;
        let user = root_user();
        let ws = otto_state::WorkspacesRepo::new(pool.clone())
            .get(&"gitpr-ws".into())
            .await
            .unwrap();
        let repo = ctx
            .git_store
            .create_repo(otto_state::NewRepo {
                workspace_id: ws.id.clone(),
                name: "checkout".into(),
                path: checkout.to_string_lossy().into(),
                remote_url: None,
                provider: None,
                git_account_id: None,
            })
            .await
            .unwrap();
        let graph: WorkflowGraph = serde_json::from_value(
            json!({"nodes":[{"id":"pr","kind":"git_pr","params":{}}],"edges":[]}),
        )
        .unwrap();
        let wf_repo = WorkflowsRepo::new(pool);
        let wf = wf_repo
            .create(&ws.id, "Open PR", "", "", &graph, &user.id)
            .await
            .unwrap();
        let run = wf_repo
            .create_run(&wf.id, &ws.id, &Value::Null, Some(&user.id))
            .await
            .unwrap();
        Self {
            _dir: dir,
            ctx,
            ws,
            user,
            run_id: run.id,
            repo,
        }
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        let node: WorkflowNode =
            serde_json::from_value(json!({"id":"pr","kind":"git_pr","params":params})).unwrap();
        let env = RunEnv {
            budget: budget::ActiveBudget::new(RUN_WALL_CLOCK_TIMEOUT),
            run_id: self.run_id.clone(),
            wf_name: "Open PR".into(),
            run_cwd: self.repo.path.clone(),
            run_base: None,
            files: std::sync::Arc::new(crate::workflow_context::RunContextFiles::disabled(
                &self.run_id,
            )),
            default_provider: "no-such-cli".into(),
            run_input: Value::Null,
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
            Value::Null,
            &env,
            &StepScope {
                step_no: 1,
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
            .expect("git_pr settles without an agent")
            .map(|(out, _)| out)
    }
}

/// A user's own checkout is never swept: a dirty main checkout keeps its
/// uncommitted file and its HEAD, whatever the node returns.
#[tokio::test]
async fn git_pr_never_commits_in_the_repos_own_checkout() {
    let f = Fixture::new().await;
    let checkout = Path::new(&f.repo.path);
    let head = git(checkout, &["rev-parse", "HEAD"]);
    std::fs::write(checkout.join("wip.txt"), "uncommitted work\n").unwrap();
    let out = f
        .execute(json!({"open": false, "provider": "no-such-cli",
                        "repos": [{"repo_id": f.repo.id, "worktree": f.repo.path}]}))
        .await;
    let err = out.expect_err("nothing to PR on the base branch");
    assert!(err.to_string().contains("nothing to PR"), "{err}");
    assert_eq!(
        git(checkout, &["rev-parse", "HEAD"]),
        head,
        "HEAD unchanged"
    );
    assert_eq!(
        git(checkout, &["status", "--porcelain", "--", "wip.txt"]),
        "?? wip.txt",
        "the user's file is still uncommitted"
    );
}

/// A dedicated worktree's leftovers ARE swept — into exactly one commit —
/// and the main checkout is untouched.
#[tokio::test]
async fn git_pr_commits_a_dirty_worktrees_leftovers_once() {
    let f = Fixture::new().await;
    let checkout = Path::new(&f.repo.path);
    let main_head = git(checkout, &["rev-parse", "HEAD"]);
    let wt = f._dir.path().join("wt");
    git(
        checkout,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "wf/feature",
            &wt.to_string_lossy(),
        ],
    );
    let wt_head = git(&wt, &["rev-parse", "HEAD"]);
    std::fs::write(wt.join("feature.txt"), "agent work\n").unwrap();
    let out = f
        .execute(json!({"open": false, "provider": "no-such-cli",
                        "repos": [{"repo_id": f.repo.id, "worktree": wt.to_string_lossy()}]}))
        .await;
    // The draft agent cannot run (unknown provider), so no PR is drafted —
    // the sweep before it is what this test pins.
    assert!(out.is_err(), "{out:?}");
    assert_eq!(
        git(&wt, &["rev-list", "--count", &format!("{wt_head}..HEAD")]),
        "1",
        "exactly one leftover commit"
    );
    assert_eq!(
        git(&wt, &["status", "--porcelain"]),
        "",
        "nothing left behind"
    );
    assert!(git(&wt, &["log", "-1", "--format=%s"]).starts_with("chore: commit workflow changes"));
    assert_eq!(git(checkout, &["rev-parse", "HEAD"]), main_head);
}

/// R13: `open: true` with every repo's open refused (a repo with no git
/// account, a forge 422) is a failure naming the reasons — never a success
/// with `opened: false`. The node reaches this only after a real draft
/// agent turn, so the decision is pinned on the function it calls.
#[test]
fn git_pr_asked_to_open_but_none_opened_is_an_error() {
    use super::require_opened;
    let refused = vec![json!({
        "repo": "checkout", "opened": false,
        "error": "upstream: forge returned 422: no git account for this repo",
        "title": "t", "description": "d",
    })];
    let err = require_opened(true, 0, &refused).expect_err("R13 fails the step");
    let msg = err.to_string();
    assert!(msg.contains("asked to open a PR but none opened"), "{msg}");
    assert!(msg.contains("422"), "the forge's reason is surfaced: {msg}");
    // A dry run opens nothing legitimately.
    assert!(require_opened(false, 0, &refused).is_ok());
    // One opened PR among several is a success.
    assert!(require_opened(true, 1, &refused).is_ok());
    // Drafted-only entries with no error (nothing to report) don't fail.
    assert!(require_opened(true, 0, &[json!({"opened": false})]).is_ok());
}
