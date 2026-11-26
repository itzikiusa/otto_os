//! `git_checkout` on a fixture origin (a local bare repository) with a
//! wiremock GitLab as the forge — test-plan.md §5.13, DR-008, DR-029.
use super::super::script::tests::NodeFixture;
use crate::workflow_context::RepoEntry;
use crate::workflow_engine::*;
use std::path::Path;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MRS: &str = "/api/v4/projects/team%2Fapp/merge_requests";

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

fn commit(dir: &Path, file: &str) {
    std::fs::write(dir.join(file), file).unwrap();
    git(dir, &["add", file]);
    git(dir, &["commit", "-q", "-m", file]);
}

struct Checkout {
    f: NodeFixture,
    wt: std::path::PathBuf,
    main_sha: String,
    feature_sha: String,
    forge: MockServer,
    repo_id: Id,
}

/// origin (bare): main = develop = one commit, `feature/GS-1` one ahead,
/// `feature/GS-12-other` one ahead; HEAD → main. The run's worktree is a
/// clone of it; the registered repo carries a GitLab remote for the forge.
async fn checkout() -> Checkout {
    let f = NodeFixture::new().await;
    let root = f.dir.path().to_path_buf();
    let (seed, origin, wt, main) = (
        root.join("seed"),
        root.join("origin.git"),
        root.join("wt"),
        root.join("main"),
    );
    for d in [&seed, &main] {
        std::fs::create_dir_all(d).unwrap();
    }
    git(&seed, &["init", "-q", "-b", "main"]);
    for (k, v) in [
        ("user.email", "t@example.invalid"),
        ("user.name", "T"),
        ("commit.gpgsign", "false"),
    ] {
        git(&seed, &["config", k, v]);
    }
    commit(&seed, "base.txt");
    git(&seed, &["branch", "develop"]);
    git(&seed, &["checkout", "-q", "-b", "feature/GS-1"]);
    commit(&seed, "gs1.txt");
    git(
        &seed,
        &["checkout", "-q", "-b", "feature/GS-12-other", "main"],
    );
    commit(&seed, "gs12.txt");
    git(&seed, &["checkout", "-q", "main"]);
    git(
        &root,
        &[
            "clone",
            "-q",
            "--bare",
            seed.to_str().unwrap(),
            origin.to_str().unwrap(),
        ],
    );
    git(
        &root,
        &[
            "clone",
            "-q",
            origin.to_str().unwrap(),
            wt.to_str().unwrap(),
        ],
    );
    let main_sha = git(&seed, &["rev-parse", "main"]);
    let feature_sha = git(&seed, &["rev-parse", "feature/GS-1"]);

    git(&main, &["init", "-q"]);
    git(
        &main,
        &[
            "remote",
            "add",
            "origin",
            "ssh://git@gitlab.ssh.example/team/app.git",
        ],
    );
    let forge = MockServer::start().await;
    f.ctx.secrets.put("gl-checkout", "glpat-test").unwrap();
    let account = f
        .ctx
        .git_store
        .create_account(otto_state::NewGitAccount {
            user_id: f.user.id.clone(),
            provider: otto_core::domain::GitProviderKind::Gitlab,
            label: "gitlab".into(),
            username: "owner".into(),
            token_ref: "gl-checkout".into(),
            api_base_url: Some(forge.uri()),
            namespace: None,
            token_expires_at: None,
        })
        .await
        .unwrap();
    let repo = f
        .ctx
        .git_store
        .create_repo(otto_state::NewRepo {
            workspace_id: f.ws.id.clone(),
            name: "app".into(),
            path: main.to_string_lossy().into(),
            remote_url: Some("ssh://git@gitlab.ssh.example/team/app.git".into()),
            provider: Some(otto_core::domain::GitProviderKind::Gitlab),
            git_account_id: Some(account.id.clone()),
        })
        .await
        .unwrap();
    f.files.set_repos(vec![RepoEntry {
        repo: "app".into(),
        repo_id: Some(repo.id.clone()),
        repo_name: Some("app".into()),
        kind: "worktree".into(),
        name: wt.to_string_lossy().into(),
        worktree: Some(wt.to_string_lossy().into()),
        base: Some("develop".into()),
        ..Default::default()
    }]);
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1})))
        .mount(&forge)
        .await;
    Checkout {
        f,
        wt,
        main_sha,
        feature_sha,
        forge,
        repo_id: repo.id,
    }
}

fn mr(iid: u64, state: &str, source: &str, title: &str) -> Value {
    json!({"iid": iid, "title": title, "state": state, "source_branch": source,
           "target_branch": "develop", "updated_at": "2026-11-01T00:00:00Z",
           "web_url": format!("https://gitlab.example/team/app/-/merge_requests/{iid}"),
           "author": {"name": "a"}})
}

impl Checkout {
    async fn run(&self, mode: &str, input: Value) -> Result<(Value, Vec<String>)> {
        self.f
            .run(json!({"id": "n3", "kind": "git_checkout", "name": "Checkout", "params": {"mode": mode}}), input)
            .await
    }
    async fn open_mrs(&self, mrs: Vec<Value>) {
        Mock::given(method("GET"))
            .and(path(MRS))
            .respond_with(ResponseTemplate::new(200).set_body_json(mrs))
            .mount(&self.forge)
            .await;
    }
    fn repos(&self) -> Vec<RepoEntry> {
        self.f.files.repos()
    }
}

#[tokio::test]
async fn branch_mode_checks_out_detached_and_rewrites_the_registry() {
    let c = checkout().await;
    let (out, logs) = c
        .run(
            "branch",
            json!({"pr_branch": "feature/GS-1", "base": "develop"}),
        )
        .await
        .unwrap();
    assert!(logs.is_empty(), "{logs:?}");
    let e = &c.repos()[0];
    // Exactly the fields the stored prompts read; the others untouched.
    assert_eq!(e.source.as_deref(), Some("feature/GS-1"));
    assert_eq!(
        e.base.as_deref(),
        Some(c.main_sha.as_str()),
        "base = the merge-base SHA"
    );
    assert_eq!(e.base_branch.as_deref(), Some("develop"));
    assert_eq!(e.name, c.wt.to_string_lossy());
    assert_eq!(git(&c.wt, &["rev-parse", "HEAD"]), c.feature_sha);
    assert_eq!(
        git(&c.wt, &["rev-parse", "--abbrev-ref", "HEAD"]),
        "HEAD",
        "detached"
    );
    let row = &out["rows"][0];
    assert_eq!(
        (
            row["via"].as_str(),
            row["ahead"].as_u64(),
            row["files_changed"].as_u64()
        ),
        (Some("pr_branch"), Some(1), Some(1))
    );
    let md = std::fs::read_to_string(c.f.ctx_dir().join("step2-checkout.md")).unwrap();
    assert!(md.contains("| app |") && md.contains("repos.json"), "{md}");

    // No branch named: the ticket's convention branch; no base: origin/HEAD.
    c.f.files.update_repos(|r| r[0].source = None);
    let (out, _) = c
        .run(
            "branch",
            json!({"jira_ticket": "https://jira.example/browse/GS-1"}),
        )
        .await
        .unwrap();
    assert_eq!(out["rows"][0]["via"], "convention");
    assert_eq!(out["rows"][0]["dest"], "main");
}

#[tokio::test]
async fn branch_mode_refuses_bad_refs_and_empty_diffs() {
    let c = checkout().await;
    let err = c
        .run("branch", json!({"pr_branch": "--upload-pack=touch x"}))
        .await
        .expect_err("invalid");
    assert!(err.to_string().contains("invalid branch name"), "{err}");
    assert!(
        c.repos()[0]
            .error
            .as_deref()
            .unwrap()
            .contains("invalid branch name"),
        "the entry stays, with its reason"
    );
    c.f.files.update_repos(|r| r[0].error = None);
    let err = c
        .run("branch", json!({"pr_branch": "develop", "base": "main"}))
        .await
        .expect_err("empty");
    assert!(err.to_string().contains("empty diff"), "{err}");
}

#[tokio::test]
async fn pr_mode_explicit_pr_is_authoritative() {
    let c = checkout().await;
    for (p, body) in [
        (
            format!("{MRS}/7"),
            mr(7, "merged", "feature/GS-1", "GS-1 fix"),
        ),
        (format!("{MRS}/7/discussions"), json!([])),
    ] {
        Mock::given(method("GET"))
            .and(path(p))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&c.forge)
            .await;
    }
    c.open_mrs(vec![mr(8, "opened", "feature/GS-1", "GS-1 again")])
        .await;
    let err = c
        .run(
            "pr",
            json!({"pr": 7, "pr_branch": "feature/GS-1", "jira_ticket": "GS-1"}),
        )
        .await
        .expect_err("a merged PR is never corrected from the branch or the ticket");
    assert!(err.to_string().contains("PR #7 is merged"), "{err}");
}

#[tokio::test]
async fn pr_mode_finds_the_pr_by_branch_then_ticket_and_writes_prs_json() {
    let c = checkout().await;
    c.open_mrs(vec![
        mr(7, "opened", "feature/GS-1", "Fix it"),
        mr(8, "opened", "feature/GS-12-other", "GS-12 other"),
    ])
    .await;
    let (out, _) = c
        .run("pr", json!({"pr_branch": "feature/GS-1"}))
        .await
        .unwrap();
    assert_eq!(
        (
            out["rows"][0]["pr"].as_u64(),
            out["rows"][0]["via"].as_str()
        ),
        (Some(7), Some("pr_branch"))
    );
    let e = &c.repos()[0];
    assert_eq!(
        (e.source.as_deref(), e.base_branch.as_deref()),
        (Some("feature/GS-1"), Some("develop"))
    );
    let prs: Vec<otto_workflows::run_refs::RunPr> =
        serde_json::from_str(&c.f.files.read_named("prs.json").unwrap()).unwrap();
    assert_eq!(prs.len(), 1);
    assert_eq!(
        (prs[0].number, prs[0].repo_id.as_str()),
        (7, c.repo_id.as_str())
    );
    assert_eq!(prs[0].state, Some(otto_core::api::PrState::Open));
    assert!(prs[0].url.as_deref().unwrap().ends_with("/7"));
    // GS-1 names #7's branch only (GS-12 is another ticket).
    let (out, _) = c.run("pr", json!({"jira_ticket": "GS-1"})).await.unwrap();
    assert_eq!(out["rows"][0]["via"], "jira");
}

#[tokio::test]
async fn pr_mode_several_candidates_are_unresolved_with_the_list() {
    let c = checkout().await;
    c.open_mrs(vec![
        mr(7, "opened", "feature/GS-1", "Fix it"),
        mr(9, "opened", "hotfix/x", "GS-1 follow-up"),
    ])
    .await;
    let err = c
        .run("pr", json!({"jira_ticket": "GS-1"}))
        .await
        .expect_err("two candidates");
    let msg = err.to_string();
    assert!(
        msg.contains("2 open PRs for GS-1")
            && msg.contains("#7 feature/GS-1")
            && msg.contains("#9 hotfix/x"),
        "{msg}"
    );
}
