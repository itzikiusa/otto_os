//! `commit_all_if_dirty` in a repo that IGNORES the files it excludes.
//!
//! `git add -A -- . ':(exclude).env'` exits 1 with "The following paths are
//! ignored by one of your .gitignore files" when `.env` is ignored: git
//! matches ignored paths against every pathspec item, excludes included. The
//! workflow reaper then kept each run worktree forever (410 warnings in three
//! days, 46 folders never reaped).

use super::*;

fn sh(dir: &Path, args: &[&str]) -> String {
    let mut cmd = std::process::Command::new("git");
    test_hermetic(&mut cmd);
    let out = cmd
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn put(dir: &Path, rel: &str, body: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, body).unwrap();
}

fn repo() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).unwrap();
    sh(&dir, &["init", "-b", "main"]);
    sh(&dir, &["config", "user.email", "otto@test.local"]);
    sh(&dir, &["config", "user.name", "Otto Test"]);
    sh(&dir, &["config", "commit.gpgsign", "false"]);
    put(&dir, "README.md", "r\n");
    sh(&dir, &["add", "-A"]);
    sh(&dir, &["commit", "-m", "init"]);
    (tmp, dir)
}

/// Every excluded name is gitignored and present: the sweep still commits
/// the real leftovers and none of the excluded files.
#[tokio::test]
async fn sweep_succeeds_when_the_excluded_files_are_gitignored() {
    let (_tmp, dir) = repo();
    put(
        &dir,
        ".gitignore",
        ".env\n.env.*\n.mcp.json\n.agents/\n.grok/\n.DS_Store\n",
    );
    put(&dir, ".env", "SECRET=1\n");
    put(&dir, ".env.local", "SECRET=2\n");
    put(&dir, ".mcp.json", "{}\n");
    put(&dir, ".agents/mcp_config.json", "{\"secret\":1}\n");
    put(&dir, ".grok/config.toml", "secret = 1\n");
    put(&dir, ".DS_Store", "x");
    put(&dir, "src/work.rs", "fn main() {}\n");
    let git = LocalGit::new(&dir);
    let sha = git
        .commit_all_if_dirty("sweep")
        .await
        .expect("ignored excludes must not fail the sweep");
    assert!(sha.is_some());
    let committed = sh(&dir, &["show", "--name-only", "--format=", "HEAD"]);
    let mut names: Vec<&str> = committed.lines().collect();
    names.sort_unstable();
    assert_eq!(names, [".gitignore", "src/work.rs"]);
    assert!(git.commit_all_if_dirty("noop").await.unwrap().is_none());
}

/// Not ignored: the excludes still keep the top-level secrets out — and a
/// TRACKED `.env` edited by the agent is not staged either.
#[tokio::test]
async fn sweep_still_excludes_unignored_and_tracked_secrets() {
    let (_tmp, dir) = repo();
    put(&dir, ".env", "SECRET=0\n");
    sh(&dir, &["add", ".env"]);
    sh(&dir, &["commit", "-m", "tracked env"]);
    put(&dir, ".env", "SECRET=1\n");
    put(&dir, ".env.prod", "SECRET=2\n");
    put(&dir, ".mcp.json", "{}\n");
    put(&dir, "nested/.DS_Store", "x");
    put(&dir, "work.txt", "w\n");
    let git = LocalGit::new(&dir);
    assert!(git.commit_all_if_dirty("sweep").await.unwrap().is_some());
    let committed = sh(&dir, &["show", "--name-only", "--format=", "HEAD"]);
    assert_eq!(committed, "work.txt");
    assert_eq!(sh(&dir, &["diff", "--name-only"]), ".env", "left unstaged");
    assert_eq!(sh(&dir, &["diff", "--cached", "--name-only"]), "");
}
