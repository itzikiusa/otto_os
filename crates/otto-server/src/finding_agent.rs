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
    let out = match Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["ls-files"])
        .output()
    {
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

/// Upper bound on a verify run's linked-test execution (compile included).
const VERIFY_TEST_TIMEOUT: Duration = Duration::from_secs(20 * 60);

/// Whether a verify run passes: `Ok(evidence)` or `Err(why not)`.
///
/// Verification needs EVIDENCE. It used to pass with no linked test at all
/// (Fix → Verify went green before the fix agent had done anything), and with
/// a linked test *file* it ran `cargo test <file path>` in the user's checkout
/// — a filter no test name matches, so zero tests ran, cargo exited 0, and the
/// finding was "verified". Now: no linked test → not verified; the test runs
/// in `worktree` (the fix branch — where the fix actually is); and a run that
/// executed zero tests is not a pass. Deterministic under `OTTO_E2E` (pass) so
/// the hermetic E2E can reach `verified`.
pub async fn judge_verify(finding: &Finding, worktree: Option<&Path>) -> Result<String, String> {
    if e2e_mode() {
        return Ok("e2e".to_string());
    }
    let Some(worktree) = worktree else {
        return Err("fix worktree unavailable — cannot run the linked test".to_string());
    };
    let Some(test) = finding
        .linked_test
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return Err(
            "no linked test — nothing substantiates the fix; add a regression test first"
                .to_string(),
        );
    };
    run_linked_test(worktree, test).await
}

fn e2e_mode() -> bool {
    matches!(std::env::var("OTTO_E2E").as_deref(), Ok("1") | Ok("true"))
}

/// How to run a linked test with cargo: `(manifest_path, args)` relative to
/// the worktree. A linked test is either a test NAME (`mod::name` / `name`)
/// or a test FILE (what the regression watcher records):
/// - `<pkg>/tests/<stem>.rs` → `--manifest-path <pkg>/Cargo.toml --test <stem>`
///   (that integration-test target, every test in it);
/// - any other `.rs` file → the nearest package's tests filtered by the file
///   stem (unit-test names carry their module path, e.g. `foo::tests::…`);
/// - `<file>.rs::name` → the file's plan, filtered to `name`;
/// - a name → `cargo test <last :: segment>` at the root.
///
/// `None` for a non-Rust test — there is no runner Otto can vouch for.
pub(crate) fn cargo_test_plan(
    worktree: &Path,
    test: &str,
) -> Option<(Option<String>, Vec<String>)> {
    // `path/to/file.rs::test_name` — a file plus a name filter.
    let (file, name_filter) = match test.split_once(".rs::") {
        Some((f, n)) if !n.trim().is_empty() => (
            format!("{f}.rs"),
            Some(n.rsplit("::").next().unwrap_or(n).to_string()),
        ),
        _ => (test.to_string(), None),
    };
    if !file.ends_with(".rs") {
        if file.contains('/') || file.contains('.') {
            return None;
        }
        let name = file.rsplit("::").next().unwrap_or(&file).to_string();
        return Some((None, vec![name]));
    }
    let rel = Path::new(&file);
    let stem = rel.file_stem()?.to_string_lossy().into_owned();
    // Nearest ancestor directory holding a Cargo.toml (within the worktree).
    let mut pkg: Option<&Path> = None;
    let mut cur = rel.parent();
    while let Some(dir) = cur {
        if worktree.join(dir).join("Cargo.toml").is_file() {
            pkg = Some(dir);
            break;
        }
        cur = dir.parent();
    }
    let pkg_dir = pkg.unwrap_or(Path::new(""));
    let manifest = if pkg_dir.as_os_str().is_empty() {
        None
    } else {
        Some(pkg_dir.join("Cargo.toml").to_string_lossy().into_owned())
    };
    let tests_dir = pkg_dir.join("tests");
    let in_tests_dir = rel.parent() == Some(tests_dir.as_path());
    let mut args = if in_tests_dir {
        vec!["--test".to_string(), stem]
    } else {
        vec![name_filter.clone().unwrap_or(stem)]
    };
    if in_tests_dir {
        if let Some(n) = name_filter {
            args.push(n);
        }
    }
    Some((manifest, args))
}

/// Total tests cargo reports as passed, summed over every `test result:` line
/// (one per test binary; a filter that matches nothing yields `0 passed`).
pub(crate) fn cargo_tests_passed(output: &str) -> u64 {
    output
        .lines()
        .filter_map(|l| l.trim().strip_prefix("test result: "))
        .filter_map(|rest| {
            let idx = rest.find(" passed")?;
            rest[..idx]
                .rsplit(|c: char| !c.is_ascii_digit())
                .next()?
                .parse::<u64>()
                .ok()
        })
        .sum()
}

/// Run a linked test in `worktree` (async — never blocks the runtime) and
/// require it to have actually executed at least one passing test.
async fn run_linked_test(worktree: &Path, test: &str) -> Result<String, String> {
    let Some((manifest, args)) = cargo_test_plan(worktree, test) else {
        return Err(format!(
            "no runner for linked test `{test}` — only Rust (cargo) tests can be verified automatically"
        ));
    };
    let mut cmd = tokio::process::Command::new("cargo");
    cmd.arg("test");
    if let Some(m) = &manifest {
        cmd.arg("--manifest-path").arg(m);
    }
    cmd.args(&args).current_dir(worktree).kill_on_drop(true);
    let out = match tokio::time::timeout(VERIFY_TEST_TIMEOUT, cmd.output()).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("could not run cargo test: {e}")),
        Err(_) => {
            return Err(format!(
                "linked test timed out after {}m",
                VERIFY_TEST_TIMEOUT.as_secs() / 60
            ))
        }
    };
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    if !out.status.success() {
        return Err(format!("linked test `{test}` failed"));
    }
    match cargo_tests_passed(&text) {
        0 => Err(format!(
            "linked test `{test}` matched no tests (0 run) — not evidence of a fix"
        )),
        n => Ok(format!("{n} test{} passed", if n == 1 { "" } else { "s" })),
    }
}

/// Provision an isolated worktree off the repo HEAD on `otto/fix/<finding_id>`.
/// Returns `(worktree_path, base_head)`. Best-effort: `Err` if `repo_path` isn't a
/// git repo or the worktree can't be created.
pub async fn provision_worktree(
    repo_path: &str,
    finding_id: &str,
) -> otto_core::Result<(String, String)> {
    let git = otto_git::LocalGit::new(repo_path);
    let base = git
        .rev_parse("HEAD")
        .await
        .unwrap_or_else(|_| "HEAD".to_string());
    let branch = format!("otto/fix/{finding_id}");
    let wt = std::env::temp_dir()
        .join(format!("otto-fix-{finding_id}"))
        .to_string_lossy()
        .into_owned();
    // Reuse the finding's worktree when a previous action (Fix → Verify /
    // Regression) already created it: re-running `worktree add --force` onto
    // the existing, non-empty path failed, so every follow-up action silently
    // got no session. Never resets the fix branch (no `-B` on an existing one).
    git.worktree_add_if_absent(&wt, &branch, &base).await?;
    // The watchers stamp a fix as "HEAD advanced past base", so base must be
    // the WORKTREE's head — a re-attached fix branch is ahead of the repo's.
    let base = otto_git::LocalGit::new(&wt)
        .rev_parse("HEAD")
        .await
        .unwrap_or(base);
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
        model: None,
        meta: Some(meta),
    };
    let session = match ctx
        .manager
        .create(&ws, &user_id.to_string(), req, None)
        .await
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("finding agent: create session ({action}): {e}");
            return None;
        }
    };
    let sid = session.id.clone();
    // Inject the prompt through the shared submit path: wait for the TUI to
    // draw, paste, confirm the paste echoed, then Enter. A blind paste 1.5 s
    // after spawn landed before a cold CLI had drawn its input box, so the
    // fix/verify/regression agent sat idle with no prompt.
    let manager = ctx.manager.clone();
    let sid_for_task = sid.clone();
    tokio::spawn(async move {
        if !crate::review_session::submit_prompt(&manager, &sid_for_task, &prompt).await {
            tracing::warn!(
                "finding agent: session {sid_for_task} never drew its TUI; prompt not sent"
            );
        }
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
        std::fs::write(
            repo.path().join("tests/regress_test.rs"),
            "#[test] fn t(){}",
        )
        .unwrap();
        git(repo.path(), &["add", "."]);
        git(repo.path(), &["commit", "-qm", "add test"]);
        let found = detect_new_test(&before, repo.path());
        assert_eq!(found.as_deref(), Some("tests/regress_test.rs"));
    }

    #[test]
    fn cargo_plan_maps_files_and_names_to_real_filters() {
        let wt = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(wt.path().join("crates/foo/tests")).unwrap();
        std::fs::create_dir_all(wt.path().join("crates/foo/src")).unwrap();
        std::fs::write(wt.path().join("crates/foo/Cargo.toml"), "").unwrap();
        std::fs::write(wt.path().join("Cargo.toml"), "").unwrap();
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();

        // Integration-test file → that --test target (the old code passed the
        // whole path as a name filter, which matched nothing and "passed").
        assert_eq!(
            cargo_test_plan(wt.path(), "crates/foo/tests/regress_test.rs"),
            Some((
                Some("crates/foo/Cargo.toml".to_string()),
                s(&["--test", "regress_test"])
            ))
        );
        // …plus a name filter.
        assert_eq!(
            cargo_test_plan(wt.path(), "crates/foo/tests/db_test.rs::no_injection"),
            Some((
                Some("crates/foo/Cargo.toml".to_string()),
                s(&["--test", "db_test", "no_injection"])
            ))
        );
        // A src file → filtered by its module stem.
        assert_eq!(
            cargo_test_plan(wt.path(), "crates/foo/src/parser.rs"),
            Some((Some("crates/foo/Cargo.toml".to_string()), s(&["parser"])))
        );
        // Root-package tests dir → no manifest override.
        assert_eq!(
            cargo_test_plan(wt.path(), "tests/top.rs"),
            Some((None, s(&["--test", "top"])))
        );
        // A bare test name.
        assert_eq!(
            cargo_test_plan(wt.path(), "db::tests::no_injection"),
            Some((None, s(&["no_injection"])))
        );
        // Non-Rust tests have no runner Otto can vouch for.
        assert_eq!(cargo_test_plan(wt.path(), "ui/e2e/login.spec.ts"), None);
    }

    #[test]
    fn zero_tests_run_is_not_a_pass() {
        let none = "running 0 tests\n\ntest result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out; finished in 0.00s\n";
        assert_eq!(cargo_tests_passed(none), 0);
        let some = "test result: ok. 2 passed; 0 failed; 0 ignored\n   Doc-tests x\ntest result: ok. 1 passed; 0 failed\n";
        assert_eq!(cargo_tests_passed(some), 3);
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
