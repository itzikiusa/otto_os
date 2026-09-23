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
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
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
        mach_services: None,
        trailing_rules: Vec::new(),
    };

    // 1. The profile must be accepted and a process must run + read at all.
    let (ok, err) = run_sandboxed(&pol, "echo alive");
    assert!(
        ok,
        "process failed to run under the profile (profile rejected?): {err}"
    );

    // 2. A write INSIDE a writable root succeeds.
    let inside_file = inside_real.join("ok.txt");
    let (ok, err) = run_sandboxed(&pol, &format!("echo hi > {}", shell_quote(&inside_file)));
    assert!(ok, "write inside the writable root was denied: {err}");
    assert!(inside_file.exists(), "file inside root not created");

    // 3. A write OUTSIDE every writable root is denied by the OS.
    let outside_file = outside_real.join("nope.txt");
    let (ok, _) = run_sandboxed(&pol, &format!("echo no > {}", shell_quote(&outside_file)));
    assert!(!ok, "write outside the writable roots was NOT denied");
    assert!(
        !outside_file.exists(),
        "file outside roots was created — sandbox leaked"
    );

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
    assert!(
        ok,
        "git commit inside the sandboxed workspace failed: {err}"
    );
    assert!(
        repo_real.join(".git").join("HEAD").exists(),
        "no .git created"
    );
}

/// The agent profile as the OS enforces it: Otto's data dir is write-denied
/// even under a writable root (here: the temp root), its agent work areas stay
/// writable, secrets / the state DB are unreadable, the daemon binary stays
/// readable (claude execs `ottod mcp-tools`), and LaunchServices is out of reach.
#[test]
fn seatbelt_agent_profile_confines_otto_data_dir() {
    if !otto_sandbox::is_supported() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let data = root.join("Otto Data");
    let cwd = root.join("project");
    for d in [data.join("bin"), data.join("workflow-context"), cwd.clone()] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(data.join("bin").join("ottod"), "bin").unwrap();
    std::fs::write(data.join("otto.db"), "db").unwrap();
    std::fs::write(data.join("otto.db.before-x.bak"), "bak").unwrap();
    std::fs::write(data.join("secrets.json"), "s").unwrap();
    let home = std::env::var("HOME").unwrap_or_default();
    let pol = SandboxPolicy::for_agent(&cwd, Path::new(&home), &data, &[], NetworkPolicy::Full);

    let q = |p: &Path| shell_quote(p);
    let (ok, _) = run_sandboxed(&pol, &format!("echo x > {}", q(&data.join("bin").join("ottod"))));
    assert!(!ok, "agent replaced the daemon binary");
    let (ok, _) = run_sandboxed(&pol, &format!("echo x >> {}", q(&data.join("otto.db"))));
    assert!(!ok, "agent wrote the state DB");
    let (ok, _) = run_sandboxed(&pol, &format!("cat {}", q(&data.join("secrets.json"))));
    assert!(!ok, "agent read secrets.json");
    let (ok, _) = run_sandboxed(&pol, &format!("cat {}", q(&data.join("otto.db.before-x.bak"))));
    assert!(!ok, "agent read a state DB backup");
    let (ok, err) = run_sandboxed(&pol, &format!("cat {} >/dev/null", q(&data.join("bin").join("ottod"))));
    assert!(ok, "the daemon binary must stay readable/executable: {err}");
    let step = data.join("workflow-context").join("step1.md");
    let (ok, err) = run_sandboxed(&pol, &format!("echo x > {}", q(&step)));
    assert!(ok, "workflow handoff dir must stay writable: {err}");
    let (ok, err) = run_sandboxed(&pol, &format!("echo x > {}", q(&cwd.join("f.txt"))));
    assert!(ok, "cwd must stay writable: {err}");
    // LaunchServices (how `open -a Terminal x.command` escapes the sandbox) is
    // not reachable: `lsappinfo front` answers an `ASN:` with the blanket
    // mach-lookup and `[ NULL ]` under the agent allow-list.
    let (prog, args) = pol.wrap("/usr/bin/lsappinfo", &["front".to_string()]);
    let out = Command::new(prog).args(args).output().expect("spawn sandbox-exec");
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("ASN:"),
        "LaunchServices must be unreachable under the agent profile"
    );
}

/// Minimal shell-quote for a path inside a `/bin/sh -c` script.
fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

fn which_git() -> Option<String> {
    for cand in [
        "/usr/bin/git",
        "/opt/homebrew/bin/git",
        "/usr/local/bin/git",
    ] {
        if Path::new(cand).exists() {
            return Some(cand.to_string());
        }
    }
    None
}
