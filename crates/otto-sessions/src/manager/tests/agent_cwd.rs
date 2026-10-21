//! Agent working-directory confinement (`check_agent_cwd`): registered
//! repos outside the root, worktrees, other workspaces, relative paths and
//! escaping symlinks.

use super::*;

/// Sweep 2026-10-11 M2: an agent may open a worker in a repo registered
/// in its workspace even outside the workspace root; an unregistered
/// folder is still refused.
#[tokio::test]
async fn agent_cwd_accepts_a_registered_repo_outside_the_root() {
    let (mgr, repo, mut ws, _user) = test_manager().await;
    let root = tempfile::tempdir().unwrap(); // the fixture's /tmp holds every Linux tempdir
    ws.root_path = root.path().to_string_lossy().into_owned();
    let checkout = tempfile::tempdir().unwrap();
    let path = std::fs::canonicalize(checkout.path()).unwrap();
    let path_s = path.to_string_lossy().into_owned();
    let caller: Id = new_id();
    assert!(matches!(
        mgr.check_agent_cwd(&ws, &caller, Some(&path_s)).await,
        Err(Error::Forbidden(_))
    ));
    sqlx::query(
        "INSERT INTO repos (id, workspace_id, name, path, created_at) VALUES (?, ?, 'r', ?, ?)",
    )
    .bind(new_id())
    .bind(&ws.id)
    .bind(&path_s)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&repo.pool())
    .await
    .unwrap();
    mgr.check_agent_cwd(&ws, &caller, Some(&path_s))
        .await
        .expect("registered repo");
    std::fs::create_dir_all(path.join("src")).unwrap();
    mgr.check_agent_cwd(&ws, &caller, Some(&format!("{path_s}/src")))
        .await
        .expect("a folder inside it");
    let other = tempfile::tempdir().unwrap();
    assert!(mgr
        .check_agent_cwd(&ws, &caller, Some(other.path().to_str().unwrap()))
        .await
        .is_err());
}

/// `git` in `dir` with a throwaway identity and no signing/hooks, so the
/// fixture never depends on the developer's global git config. Spawned via
/// `hardened_std_command` (the `git_spawn_guard` test forbids a bare git).
#[allow(clippy::disallowed_methods)] // test fixture: a blocking git call is fine here
fn git_fixture(dir: &std::path::Path, args: &[&str]) {
    let out = otto_git::hardened_std_command()
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=otto",
            "-c",
            "user.email=otto@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

/// A git repo with one commit (worktrees need a HEAD) and a worktree of it
/// in a separate folder; returns (repo, worktree), both canonical.
fn repo_with_worktree(repo_dir: &std::path::Path, wt_parent: &std::path::Path) -> (String, String) {
    git_fixture(repo_dir, &["init", "-q"]);
    git_fixture(repo_dir, &["commit", "-q", "--allow-empty", "-m", "init"]);
    let wt = wt_parent.join("wt");
    git_fixture(
        repo_dir,
        &["worktree", "add", "-q", "-b", "wt", wt.to_str().unwrap()],
    );
    let canon = |p: &std::path::Path| {
        std::fs::canonicalize(p)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    (canon(repo_dir), canon(&wt))
}

async fn register_repo(repo: &SessionsRepo, workspace_id: &str, path: &str) {
    sqlx::query(
        "INSERT INTO repos (id, workspace_id, name, path, created_at) VALUES (?, ?, 'r', ?, ?)",
    )
    .bind(new_id())
    .bind(workspace_id)
    .bind(path)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&repo.pool())
    .await
    .unwrap();
}

/// test-quality-1-04: the other bases and refusals of the agent-cwd
/// confinement — a repo registered in ANOTHER workspace, worktrees (allowed
/// only for a registered repo), relative paths, and a symlink inside a base
/// that escapes it (canonicalised before the containment check).
#[tokio::test]
async fn agent_cwd_confinement_edges() {
    let (mgr, repo, mut ws, _user) = test_manager().await;
    let root = tempfile::tempdir().unwrap();
    ws.root_path = root.path().to_string_lossy().into_owned();
    let caller: Id = new_id();
    let refused = |r: Result<()>| matches!(r, Err(Error::Forbidden(_)));

    // A repo a person registered in a DIFFERENT workspace is not a base here.
    let other_ws = new_id();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'o', '/tmp', ?)",
    )
    .bind(&other_ws)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(&repo.pool())
    .await
    .unwrap();
    let foreign = tempfile::tempdir().unwrap();
    let foreign_s = std::fs::canonicalize(foreign.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    register_repo(&repo, &other_ws, &foreign_s).await;
    assert!(
        refused(mgr.check_agent_cwd(&ws, &caller, Some(&foreign_s)).await),
        "another workspace's repo must be refused"
    );

    // A worktree of a repo registered HERE shares its git common dir → allowed.
    let registered = tempfile::tempdir().unwrap();
    let registered_wt = tempfile::tempdir().unwrap();
    let (reg_path, reg_wt) = repo_with_worktree(registered.path(), registered_wt.path());
    register_repo(&repo, &ws.id, &reg_path).await;
    mgr.check_agent_cwd(&ws, &caller, Some(&reg_wt))
        .await
        .expect("worktree of a registered repo");

    // A worktree of an UNregistered repo is refused (common dir matches no base).
    let loose = tempfile::tempdir().unwrap();
    let loose_wt = tempfile::tempdir().unwrap();
    let (loose_path, loose_wt_path) = repo_with_worktree(loose.path(), loose_wt.path());
    assert!(refused(
        mgr.check_agent_cwd(&ws, &caller, Some(&loose_path)).await
    ));
    assert!(
        refused(
            mgr.check_agent_cwd(&ws, &caller, Some(&loose_wt_path))
                .await
        ),
        "worktree of an unregistered repo must be refused"
    );

    // Relative paths never resolve against the daemon's cwd.
    std::fs::create_dir_all(root.path().join("src")).unwrap();
    assert!(refused(
        mgr.check_agent_cwd(&ws, &caller, Some("src")).await
    ));
    assert!(refused(
        mgr.check_agent_cwd(&ws, &caller, Some("./src")).await
    ));

    // A symlink inside the root that points outside it is judged by its target.
    let outside = tempfile::tempdir().unwrap();
    let link = root.path().join("escape");
    std::os::unix::fs::symlink(outside.path(), &link).unwrap();
    assert!(
        refused(
            mgr.check_agent_cwd(&ws, &caller, Some(link.to_str().unwrap()))
                .await
        ),
        "a symlink escaping the base must be refused"
    );
    // ...while the root itself (and a real folder in it) is still allowed.
    mgr.check_agent_cwd(&ws, &caller, Some(&ws.root_path))
        .await
        .expect("workspace root");
    mgr.check_agent_cwd(
        &ws,
        &caller,
        Some(root.path().join("src").to_str().unwrap()),
    )
    .await
    .expect("folder inside the root");
}
