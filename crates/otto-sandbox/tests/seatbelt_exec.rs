//! Real Apple Seatbelt enforcement test — runs the generated profile through
//! `/usr/bin/sandbox-exec` and asserts the OS actually confines writes while
//! leaving reads, process execution, and in-workspace git commits working.
//!
//! macOS-only (Seatbelt is a macOS facility); a no-op elsewhere.
#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::Command;

use otto_sandbox::{NetworkPolicy, SandboxPolicy};

/// Run `/bin/sh -c <script>` under the sandbox, returning (success, stderr).
fn run_sandboxed(pol: &SandboxPolicy, script: &str) -> (bool, String) {
    let (prog, args) = pol.wrap("/bin/sh", &["-c".to_string(), script.to_string()]);
    let out = Command::new(prog)
        .args(args)
        .output()
        .expect("spawn sandbox-exec");
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn seatbelt_confines_writes_but_allows_reads_and_exec() {
    if !otto_sandbox::is_supported() {
        eprintln!("sandbox-exec unavailable; skipping");
        return;
    }
    let inside = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let inside_real = std::fs::canonicalize(inside.path()).unwrap();
    let outside_real = std::fs::canonicalize(outside.path()).unwrap();

    let pol = SandboxPolicy {
        writable_roots: vec![inside_real.clone()],
        deny_read: Vec::new(),
        network: NetworkPolicy::Full,
    };

    // 1. The profile must be accepted and a process must run + read at all.
    let (ok, err) = run_sandboxed(&pol, "echo alive");
    assert!(ok, "process failed to run under the profile (profile rejected?): {err}");

    // 2. A write INSIDE a writable root succeeds.
    let inside_file = inside_real.join("ok.txt");
    let (ok, err) = run_sandboxed(&pol, &format!("echo hi > {}", shell_quote(&inside_file)));
    assert!(ok, "write inside the writable root was denied: {err}");
    assert!(inside_file.exists(), "file inside root not created");

    // 3. A write OUTSIDE every writable root is denied by the OS.
    let outside_file = outside_real.join("nope.txt");
    let (ok, _) = run_sandboxed(&pol, &format!("echo no > {}", shell_quote(&outside_file)));
    assert!(!ok, "write outside the writable roots was NOT denied");
    assert!(!outside_file.exists(), "file outside roots was created — sandbox leaked");

    // 4. Reading an arbitrary file outside the roots still works (read is global).
    let readable = outside_real.join("readme.txt");
    std::fs::write(&readable, "secret-but-readable").unwrap();
    let (ok, err) = run_sandboxed(&pol, &format!("cat {}", shell_quote(&readable)));
    assert!(ok, "reading a file outside the roots was denied: {err}");
}

#[test]
fn seatbelt_allows_git_commit_in_the_workspace() {
    if !otto_sandbox::is_supported() {
        return;
    }
    let Some(git) = which_git() else {
        eprintln!("git not found; skipping commit test");
        return;
    };
    let repo = tempfile::tempdir().unwrap();
    let repo_real = std::fs::canonicalize(repo.path()).unwrap();

    // The agent's git dir lives under cwd here; for a non-worktree repo it is
    // exactly `<cwd>/.git`, which is inside the workspace writable root.
    let pol = SandboxPolicy::for_agent(
        &repo_real,
        Path::new(&std::env::var("HOME").unwrap_or_default()),
        &repo_real.join(".otto-data"),
        &[repo_real.join(".git")],
        NetworkPolicy::Full,
    );

    // init + identity + commit, all under the sandbox.
    let script = format!(
        "cd {dir} && {git} init -q && {git} config user.email a@b.c && \
         {git} config user.name t && {git} config commit.gpgsign false && \
         echo hello > f.txt && {git} add f.txt && {git} commit -q -m first",
        dir = shell_quote(&repo_real),
        git = shell_quote(Path::new(&git)),
    );
    let (ok, err) = run_sandboxed(&pol, &script);
    assert!(ok, "git commit inside the sandboxed workspace failed: {err}");
    assert!(repo_real.join(".git").join("HEAD").exists(), "no .git created");
}

/// Minimal shell-quote for a path inside a `/bin/sh -c` script.
fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

fn which_git() -> Option<String> {
    for cand in ["/usr/bin/git", "/opt/homebrew/bin/git", "/usr/local/bin/git"] {
        if Path::new(cand).exists() {
            return Some(cand.to_string());
        }
    }
    None
}
