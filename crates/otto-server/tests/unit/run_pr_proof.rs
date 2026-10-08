use super::*;
use otto_core::proof::{ProofStatus, WorkItemKind};
use otto_core::run::RunMode;

async fn fixture() -> (
    tempfile::TempDir,
    ServerCtx,
    OttoRun,
    otto_core::proof::ProofPack,
) {
    let dir = tempfile::tempdir().unwrap();
    let pool = crate::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, dir.path()).await;
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('proof-ws', 'Proof', '/tmp', '2026-10-08')").execute(&pool).await.unwrap();
    let run = ctx
        .runs
        .create(NewRun {
            workspace_id: "proof-ws".into(),
            title: "Proof gate fixture".into(),
            source_kind: SourceKind::Finding,
            source_ref: "fixture".into(),
            source_url: None,
            goal: "No external publication".into(),
            mode: RunMode::SingleAgent,
            provider: "claude".into(),
            model: String::new(),
            repo_id: None,
            origin_kind: RunOrigin::Slack,
            origin_chat: None,
            origin_thread: None,
            origin_user: None,
            callback_url: None,
            auto_open_pr: false,
            context_summary: None,
            created_by: "root".into(),
        })
        .await
        .unwrap();
    let pack = ctx
        .proof_repo
        .create_pack(
            "proof-ws",
            WorkItemKind::Task,
            &run.id,
            "Proof",
            "root",
            None,
        )
        .await
        .unwrap();
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                approval_decision: Some("approved".into()),
                proof_status: Some("passed".into()),
                proof_pack_id: Some(pack.id.clone()),
                repo_id: Some("missing-repo".into()),
                // Deliberately invalid: the old code fails here instead of enforcing
                // current proof. No path in this fixture can invoke a git provider.
                pr_draft_json: Some("not-json".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    (dir, ctx, run, pack)
}

#[tokio::test]
async fn open_pr_rechecks_live_proof_instead_of_cached_run_status() {
    let (_dir, ctx, run, pack) = fixture().await;
    for status in [
        ProofStatus::Failed,
        ProofStatus::Missing,
        ProofStatus::Partial,
    ] {
        ctx.proof_repo
            .set_status_risk(&pack.id, status, 0)
            .await
            .unwrap();
        let error = open_pr(&ctx, &run.id).await.unwrap_err();
        assert!(
            matches!(error, Error::Conflict(_)),
            "live {status:?} proof bypassed: {error}"
        );
    }
    ctx.proof_repo
        .set_status_risk(&pack.id, ProofStatus::Passed, 0)
        .await
        .unwrap();
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                proof_pack_id: Some("deleted-pack".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        open_pr(&ctx, &run.id).await.unwrap_err(),
        Error::Conflict(_)
    ));
}

// Synchronous fixture setup helper; never called by production handlers.
#[allow(clippy::disallowed_methods)]
fn git(dir: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().into()
}

#[tokio::test]
async fn open_pr_rejects_changed_worktree_and_changed_commit_without_mutating_either() {
    let (dir, ctx, run, pack) = fixture().await;
    let repo = dir.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    git(&repo, &["init", "-q"]);
    git(&repo, &["config", "user.name", "Fixture"]);
    git(&repo, &["config", "user.email", "fixture@example.invalid"]);
    std::fs::write(repo.join("app.txt"), "proven\n").unwrap();
    git(&repo, &["add", "app.txt"]);
    git(&repo, &["commit", "-qm", "fixture"]);
    let base = git(&repo, &["rev-parse", "HEAD"]);
    std::fs::write(repo.join("app.txt"), "reviewed change\n").unwrap();
    git(&repo, &["add", "app.txt"]);
    git(&repo, &["commit", "-qm", "reviewed"]);
    let head = git(&repo, &["rev-parse", "HEAD"]);
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                worktree_path: Some(repo.to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    ctx.proof_repo
        .waive(&pack.id, "root", "Explicit test exception")
        .await
        .unwrap();
    crate::proof::assemble_diff(&ctx, &pack, repo.to_str().unwrap(), Some(&base))
        .await
        .unwrap();
    let artifacts = ctx.proof_repo.list_artifacts_meta(&pack.id).await.unwrap();
    assert!(artifacts
        .iter()
        .any(|artifact| artifact.metadata["head_commit"] == head));
    std::fs::write(repo.join(".mcp.json"), "{}").unwrap();
    // A matching clean revision clears the evidence gate, then fails at the
    // deliberately invalid draft. This is the positive control, with no provider.
    assert!(matches!(
        open_pr(&ctx, &run.id).await.unwrap_err(),
        Error::Internal(_)
    ));
    std::fs::write(repo.join("app.txt"), "unreviewed\n").unwrap();
    let dirty = open_pr(&ctx, &run.id).await.unwrap_err();
    assert!(matches!(dirty, Error::Conflict(_)), "{dirty}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), head);
    assert!(
        git(&repo, &["diff", "--cached"]).is_empty(),
        "gate must not stage changes"
    );
    git(&repo, &["add", "app.txt"]);
    git(&repo, &["commit", "-qm", "later edit"]);
    let later = git(&repo, &["rev-parse", "HEAD"]);
    let stale = open_pr(&ctx, &run.id).await.unwrap_err();
    assert!(matches!(stale, Error::Conflict(_)), "{stale}");
    assert_eq!(git(&repo, &["rev-parse", "HEAD"]), later);
    // Refresh proof for the later commit, then use a valid draft. This fixture
    // has no remote: publication must return the push failure before asking a
    // provider to create a PR against a potentially older branch.
    crate::proof::assemble_diff(&ctx, &pack, repo.to_str().unwrap(), Some(&base))
        .await
        .unwrap();
    let registered = ctx
        .git_store
        .create_repo(otto_state::NewRepo {
            workspace_id: "proof-ws".into(),
            name: "Local fixture".into(),
            path: repo.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    ctx.runs.set_fields(&run.id, &RunPatch {
        repo_id: Some(registered.id),
        pr_draft_json: Some(serde_json::json!({"title":"Fixture", "description":"Local only", "source_branch":git(&repo, &["branch", "--show-current"]), "target_branch":"main"}).to_string()),
        ..Default::default()
    }).await.unwrap();
    let fresh = ctx.runs.get(&run.id).await.unwrap();
    let (proved, branch) = check_proof_revision(&ctx, &fresh).await.unwrap();
    assert_eq!(proved, later);
    assert_eq!(branch, git(&repo, &["branch", "--show-current"]));
    let valid_draft = fresh.pr_draft_json.unwrap();
    let mut wrong_draft: serde_json::Value = serde_json::from_str(&valid_draft).unwrap();
    wrong_draft["source_branch"] = serde_json::json!("unproved-other-branch");
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                pr_draft_json: Some(wrong_draft.to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        open_pr(&ctx, &run.id).await.unwrap_err(),
        Error::Conflict(_)
    ));
    ctx.runs
        .set_fields(
            &run.id,
            &RunPatch {
                pr_draft_json: Some(valid_draft),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let push_error = open_pr(&ctx, &run.id).await.unwrap_err();
    assert!(
        matches!(push_error, Error::Upstream(_)),
        "push failure was hidden: {push_error}"
    );
    assert!(ctx.runs.get(&run.id).await.unwrap().pr_url.is_none());
}
