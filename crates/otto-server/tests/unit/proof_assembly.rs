use super::*;

#[allow(clippy::disallowed_methods)]
fn fixture_git(path: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().into()
}

#[tokio::test]
async fn command_proof_tracks_commits_and_dirty_binary_content() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    fixture_git(dir.path(), &["init", "-q"]);
    fixture_git(
        dir.path(),
        &["config", "user.email", "proof@example.invalid"],
    );
    fixture_git(dir.path(), &["config", "user.name", "Proof"]);
    tokio::fs::write(dir.path().join("app.bin"), [0, 1, 2])
        .await
        .unwrap();
    fixture_git(dir.path(), &["add", "."]);
    fixture_git(dir.path(), &["commit", "-qm", "base"]);
    let base = fixture_git(dir.path(), &["rev-parse", "HEAD"]);
    tokio::fs::write(dir.path().join("app.bin"), [0, 3, 4])
        .await
        .unwrap();
    fixture_git(dir.path(), &["commit", "-qam", "A"]);
    let pool = crate::test_support::mem_pool().await;
    let state_dir = tempfile::tempdir().unwrap();
    let ctx = ServerCtx::for_tests(&pool, state_dir.path()).await;
    sqlx::query("INSERT INTO workspaces (id,name,root_path,created_at) VALUES ('ws','Proof','/tmp','2026-10-18')")
        .execute(&pool).await.unwrap();
    let pack = gate(&ctx, WorkItemKind::Task, "task", "ws", "proof", "root")
        .await
        .unwrap();
    assemble_diff(&ctx, &pack, cwd, Some(&base)).await.unwrap();
    // Recognized runner name, deterministic local shell command: no Cargo subprocess.
    run_command_artifact(&ctx, &pack, cwd, "echo cargo test", None)
        .await
        .unwrap();
    assert_eq!(
        recompute_and_emit(&ctx, &pack.id).await.unwrap().status,
        ProofStatus::Passed
    );
    tokio::fs::write(dir.path().join("app.bin"), [0, 5, 6])
        .await
        .unwrap();
    fixture_git(dir.path(), &["commit", "-qam", "B"]);
    assemble_diff(&ctx, &pack, cwd, Some(&base)).await.unwrap();
    assert_eq!(
        recompute_and_emit(&ctx, &pack.id).await.unwrap().status,
        ProofStatus::Partial
    );
    run_command_artifact(&ctx, &pack, cwd, "echo cargo test", None)
        .await
        .unwrap();
    assert_eq!(
        recompute_and_emit(&ctx, &pack.id).await.unwrap().status,
        ProofStatus::Passed
    );
    let clean = proof_identity(cwd).await.unwrap();
    tokio::fs::write(dir.path().join("app.bin"), [0, 7, 8])
        .await
        .unwrap();
    let dirty = proof_identity(cwd).await.unwrap();
    assert_ne!(clean, dirty);
    tokio::fs::write(dir.path().join("app.bin"), [0, 9, 8])
        .await
        .unwrap();
    assert_ne!(dirty, proof_identity(cwd).await.unwrap());
    assemble_diff(&ctx, &pack, cwd, Some(&base)).await.unwrap();
    assert_eq!(
        recompute_and_emit(&ctx, &pack.id).await.unwrap().status,
        ProofStatus::Partial
    );
    run_command_artifact(
        &ctx,
        &pack,
        cwd,
        "printf changed > app.bin # cargo test",
        None,
    )
    .await
    .unwrap();
    let commands = ctx.proof_repo.list_artifacts_meta(&pack.id).await.unwrap();
    assert!(commands
        .iter()
        .any(|a| a.title.starts_with("printf changed")
            && a.metadata.get("proof_identity") == Some(&Value::Null)));
    ctx.proof_repo
        .waive(&pack.id, "root", "explicit exception")
        .await
        .unwrap();
    assert_eq!(
        recompute_and_emit(&ctx, &pack.id).await.unwrap().status,
        ProofStatus::Waived
    );
}

#[test]
fn proof_locks_prune_unheld_entries() {
    let locks = new_locks();
    let held = locks.handle("p1");
    drop(locks.handle("p2"));
    // p2 is unheld → pruned; p1 (held) survives and is handed out again as
    // the SAME mutex.
    let again = locks.handle("p1");
    assert!(Arc::ptr_eq(&held, &again));
    assert_eq!(locks.len(), 1);
}

/// error-handling-6-02: a folder that isn't a repo is reported, not a silent
/// success; a repo with no changes says so; a changed one assembles.
#[tokio::test]
async fn assemble_diff_reports_what_it_did() {
    let pool = crate::test_support::mem_pool().await;
    let state_dir = tempfile::tempdir().unwrap();
    let ctx = ServerCtx::for_tests(&pool, state_dir.path()).await;
    sqlx::query("INSERT INTO workspaces (id,name,root_path,created_at) VALUES ('ws','Proof','/tmp','2026-10-18')")
        .execute(&pool).await.unwrap();
    let pack = gate(&ctx, WorkItemKind::Task, "task", "ws", "proof", "root")
        .await
        .unwrap();
    let plain = tempfile::tempdir().unwrap();
    assert_eq!(
        assemble_diff(&ctx, &pack, plain.path().to_str().unwrap(), None)
            .await
            .unwrap(),
        DiffAssembly::NotARepo
    );
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap();
    fixture_git(dir.path(), &["init", "-q"]);
    fixture_git(dir.path(), &["config", "user.email", "p@example.invalid"]);
    fixture_git(dir.path(), &["config", "user.name", "P"]);
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    fixture_git(dir.path(), &["add", "."]);
    fixture_git(dir.path(), &["commit", "-qm", "base"]);
    assert_eq!(
        assemble_diff(&ctx, &pack, cwd, None).await.unwrap(),
        DiffAssembly::NoChanges
    );
    std::fs::write(dir.path().join("a.txt"), "two\n").unwrap();
    assert_eq!(
        assemble_diff(&ctx, &pack, cwd, None).await.unwrap(),
        DiffAssembly::Assembled { files_changed: 1 }
    );
    let failed = assemble_diff(&ctx, &pack, cwd, Some("no-such-base")).await;
    assert!(
        matches!(failed, Ok(DiffAssembly::DiffFailed { ref commands }) if !commands.is_empty()),
        "{failed:?}"
    );
}

#[test]
fn risky_file_segments_not_substrings() {
    assert!(is_risky_file("crates/otto-state/migrations/0077.sql"));
    assert!(is_risky_file("crates/otto-server/src/policy.rs"));
    assert!(is_risky_file("crates/otto-rbac/src/auth.rs"));
    assert!(is_risky_file(".github/workflows/ci.yml"));
    assert!(is_risky_file("ui/package-lock.json"));
    assert!(is_risky_file("Cargo.lock"));
    // false positives the naive substring approach would hit:
    assert!(!is_risky_file("crates/otto-core/src/author.rs"));
    assert!(!is_risky_file("ui/src/lib/tokenizer.ts"));
    assert!(!is_risky_file("crates/otto-server/src/modules.rs"));
}

#[test]
fn classify_kinds() {
    assert_eq!(classify_test_kind("cargo clippy --workspace"), "lint");
    assert_eq!(classify_test_kind("go vet ./..."), "lint");
    assert_eq!(classify_test_kind("npm run build"), "build");
    assert_eq!(classify_test_kind("cargo test --workspace"), "test");
}

#[tokio::test]
async fn run_command_captures_success_and_failure() {
    let here = ".";
    let ok = run_command(here, "true", 30).await;
    assert!(ok.success && ok.exit_code == 0);
    let bad = run_command(here, "exit 3", 30).await;
    assert!(!bad.success && bad.exit_code == 3);
    let echo = run_command(here, "echo hello", 30).await;
    assert!(echo.output.contains("hello"));
}

#[tokio::test]
async fn run_command_timeout_kills_descendants() {
    let dir = tempfile::tempdir().unwrap();
    let result = run_command(
        dir.path().to_str().unwrap(),
        "(sleep 2; touch survived-timeout) & wait",
        1,
    )
    .await;
    assert!(!result.success);
    assert!(result.output.contains("timed out"));
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert!(!dir.path().join("survived-timeout").exists());
}

#[test]
fn prepare_content_caps_and_reports() {
    let (small, meta) = prepare_content("hello");
    assert_eq!(small, "hello");
    assert_eq!(meta["truncated"], json!(false));
    let big = "x".repeat(STORE_CAP + 1000);
    let (capped, meta) = prepare_content(&big);
    assert!(capped.len() <= STORE_CAP + 32);
    assert_eq!(meta["truncated"], json!(true));
}

#[test]
fn pr_gate_decision() {
    // Unproven pack with no override → block.
    assert!(pr_should_block(ProofStatus::Failed, false));
    assert!(pr_should_block(ProofStatus::Missing, false));
    assert!(pr_should_block(ProofStatus::Partial, false));
    // Proven (or waived) → never block.
    assert!(!pr_should_block(ProofStatus::Passed, false));
    assert!(!pr_should_block(ProofStatus::Waived, false));
    // Override lets an unproven pack through.
    assert!(!pr_should_block(ProofStatus::Failed, true));
}

#[test]
fn prepare_content_redacts_secrets() {
    // A trust layer must not itself leak secrets (D6). A Bearer token in
    // captured output must be redacted before storage.
    let raw = "running with Authorization: Bearer abcdef0123456789ABCDEF0123456789 in the log";
    let (stored, meta) = prepare_content(raw);
    assert!(
        !stored.contains("abcdef0123456789ABCDEF0123456789"),
        "secret must be redacted from stored content"
    );
    assert!(
        meta["redactions"].as_u64().unwrap_or(0) >= 1,
        "redaction count should be reported"
    );
}
