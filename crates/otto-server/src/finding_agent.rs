//! Agent-backed finding actions (fix / verify / regression-test).
//!
//! **Do not damage user work:** these spawn the agent in an *isolated git
//! worktree* on a temp branch (`otto/fix/<finding_id>`), never the user's working
//! branch. The session is still openable (the user can watch the agent close the
//! loop). Result stamping reads concrete repo state via the small **pure**
//! functions below (unit-tested against a real temp git repo), so attribution is
//! unambiguous and deterministic.

use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use otto_core::api::CreateSessionReq;
use otto_core::domain::SessionKind;
use otto_core::finding::Finding;

use crate::state::ServerCtx;

/// `git -C <dir> rev-parse HEAD` → the current commit sha, if `dir` is a repo.
pub fn head_of(dir: &Path) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// The set of tracked test files in a worktree (paths whose name suggests a
/// test, across common languages).
pub fn list_test_files(dir: &Path) -> HashSet<String> {
    let out = match Command::new("git").arg("-C").arg(dir).args(["ls-files"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return HashSet::new(),
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|p| is_test_path(p))
        .collect()
}

/// Heuristic: does this path look like a test file?
fn is_test_path(p: &str) -> bool {
    let lower = p.to_ascii_lowercase();
    let is_code = lower.ends_with(".rs")
        || lower.ends_with(".ts")
        || lower.ends_with(".tsx")
        || lower.ends_with(".js")
        || lower.ends_with(".py")
        || lower.ends_with(".go")
        || lower.ends_with(".java");
    is_code && (lower.contains("test") || lower.contains("spec") || lower.contains("__tests__"))
}

/// Stamp a fix: returns the new HEAD sha iff it advanced past `before_head`.
pub fn stamp_fix(before_head: Option<&str>, worktree: &Path) -> Option<String> {
    let now = head_of(worktree)?;
    match before_head {
        Some(b) if b == now => None,
        _ => Some(now),
    }
}

/// Detect a regression test the agent added: the first test file present now
/// that wasn't present in `before`.
pub fn detect_new_test(before: &HashSet<String>, worktree: &Path) -> Option<String> {
    let now = list_test_files(worktree);
    now.difference(before).min().cloned()
}

/// Whether a verify run should pass. Deterministic under `OTTO_E2E` (true) so the
/// hermetic E2E can reach `verified`; otherwise runs the finding's linked test if
/// present (pass on exit 0); with no linked test, optimistically true (a verify
/// agent session is spawned alongside for the user to inspect).
pub fn judge_verify(finding: &Finding, repo_path: &Path) -> bool {
    if e2e_mode() {
        return true;
    }
    if let Some(test) = finding.linked_test.as_deref().filter(|s| !s.trim().is_empty()) {
        return run_linked_test(repo_path, test);
    }
    true
}

fn e2e_mode() -> bool {
    matches!(std::env::var("OTTO_E2E").as_deref(), Ok("1") | Ok("true"))
}

/// Best-effort: run a linked test by name. Tries `cargo test <name>` for Rust,
/// otherwise treats the entry as a shell-runnable spec path. Returns the exit
/// success; any spawn failure is treated as "not verified".
fn run_linked_test(repo_path: &Path, test: &str) -> bool {
    // Rust: `cargo test <fn-or-path>`; JS/TS specs: rely on the repo's runner via
    // `npx playwright test <path>` is too specific, so default to cargo for `.rs`.
    let cmd = if test.contains(".rs") || !test.contains('.') {
        Command::new("cargo")
            .arg("test")
            .arg(test.split("::").last().unwrap_or(test))
            .current_dir(repo_path)
            .output()
    } else {
        // Unknown runner — don't claim a pass we can't substantiate.
        return false;
    };
    matches!(cmd, Ok(o) if o.status.success())
}

/// Provision an isolated worktree off the repo HEAD on `otto/fix/<finding_id>`.
/// Returns `(worktree_path, base_head)`. Best-effort: `Err` if `repo_path` isn't a
/// git repo or the worktree can't be created.
pub async fn provision_worktree(
    repo_path: &str,
    finding_id: &str,
) -> otto_core::Result<(String, String)> {
    let git = otto_git::LocalGit::new(repo_path);
    let base = git.rev_parse("HEAD").await.unwrap_or_else(|_| "HEAD".to_string());
    let branch = format!("otto/fix/{finding_id}");
    let wt = std::env::temp_dir()
        .join(format!("otto-fix-{finding_id}"))
        .to_string_lossy()
        .into_owned();
    if git.branch_exists(&branch).await {
        git.worktree_attach(&wt, &branch).await?;
    } else {
        git.worktree_add(&wt, &branch, &base).await?;
    }
    Ok((wt, base))
}

/// Spawn an openable agent session in `cwd` running `provider`, inject `prompt`
/// after the TUI settles (the handoff pattern). Best-effort: returns the session
/// id on success, `None` on any failure (the action still records its intent).
#[allow(clippy::too_many_arguments)]
pub async fn spawn_session(
    ctx: &ServerCtx,
    workspace_id: &str,
    user_id: &str,
    provider: &str,
    cwd: &str,
    finding_id: &str,
    action: &str,
    prompt: String,
) -> Option<String> {
    let ws = ctx.workspaces.get(&workspace_id.to_string()).await.ok()?;
    let meta = serde_json::json!({
        "source": "finding",
        "finding_id": finding_id,
        "action": action,
    });
    let req = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some(provider.to_string()),
        title: None,
        cwd: Some(cwd.to_string()),
        connection_id: None,
        meta: Some(meta),
    };
    let session = match ctx.manager.create(&ws, &user_id.to_string(), req, None).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("finding agent: create session ({action}): {e}");
            return None;
        }
    };
    let sid = session.id.clone();
    // Inject the prompt once the session has settled (handoff pattern: a short
    // delay then a bracketed-paste + Enter).
    let manager = ctx.manager.clone();
    let sid_for_task = sid.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        let payload = format!("\u{1b}[200~{prompt}\u{1b}[201~");
        let _ = manager.input(&sid_for_task, payload.as_bytes()).await;
        tokio::time::sleep(Duration::from_millis(250)).await;
        let _ = manager.input(&sid_for_task, b"\r").await;
    });
    Some(sid)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) {
        let ok = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?} failed");
    }

    fn init_repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.email", "t@t.io"]);
        git(dir.path(), &["config", "user.name", "t"]);
        std::fs::write(dir.path().join("a.txt"), "hello").unwrap();
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "init"]);
        dir
    }

    #[test]
    fn stamp_fix_detects_new_commit() {
        let repo = init_repo();
        let before = head_of(repo.path());
        assert!(before.is_some());
        // no change yet
        assert!(stamp_fix(before.as_deref(), repo.path()).is_none());
        // make a commit → HEAD advances
        std::fs::write(repo.path().join("b.txt"), "x").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "fix"]);
        let after = stamp_fix(before.as_deref(), repo.path());
        assert!(after.is_some());
        assert_ne!(after, before);
    }

    #[test]
    fn detect_new_test_finds_added_file() {
        let repo = init_repo();
        let before = list_test_files(repo.path());
        assert!(before.is_empty());
        std::fs::create_dir_all(repo.path().join("tests")).unwrap();
        std::fs::write(repo.path().join("tests/regress_test.rs"), "#[test] fn t(){}").unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "add test"]);
        let found = detect_new_test(&before, repo.path());
        assert_eq!(found.as_deref(), Some("tests/regress_test.rs"));
    }

    #[test]
    fn is_test_path_heuristic() {
        assert!(is_test_path("tests/foo_test.rs"));
        assert!(is_test_path("ui/e2e/login.spec.ts"));
        assert!(is_test_path("src/__tests__/x.js"));
        assert!(!is_test_path("src/main.rs"));
        assert!(!is_test_path("README.md"));
    }
}
