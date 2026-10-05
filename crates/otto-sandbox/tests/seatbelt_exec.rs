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

    let (ok, _) = run_sandboxed(
        &pol,
        &format!("echo x > {}", shell_quote(&data.join("bin").join("ottod"))),
    );
    assert!(!ok, "agent replaced the daemon binary");
    let (ok, _) = run_sandboxed(
        &pol,
        &format!("echo x >> {}", shell_quote(&data.join("otto.db"))),
    );
    assert!(!ok, "agent wrote the state DB");
    let (ok, _) = run_sandboxed(
        &pol,
        &format!("cat {}", shell_quote(&data.join("secrets.json"))),
    );
    assert!(!ok, "agent read secrets.json");
    let (ok, _) = run_sandboxed(
        &pol,
        &format!("cat {}", shell_quote(&data.join("otto.db.before-x.bak"))),
    );
    assert!(!ok, "agent read a state DB backup");
    let (ok, err) = run_sandboxed(
        &pol,
        &format!(
            "cat {} >/dev/null",
            shell_quote(&data.join("bin").join("ottod"))
        ),
    );
    assert!(ok, "the daemon binary must stay readable/executable: {err}");
    let step = data.join("workflow-context").join("step1.md");
    let (ok, err) = run_sandboxed(&pol, &format!("echo x > {}", shell_quote(&step)));
    assert!(ok, "workflow handoff dir must stay writable: {err}");
    let (ok, err) = run_sandboxed(
        &pol,
        &format!("echo x > {}", shell_quote(&cwd.join("f.txt"))),
    );
    assert!(ok, "cwd must stay writable: {err}");
    // LaunchServices (how `open -a Terminal x.command` escapes the sandbox) is
    // not reachable: `lsappinfo front` answers an `ASN:` with the blanket
    // mach-lookup and `[ NULL ]` under the agent allow-list.
    let (prog, args) = pol.wrap("/usr/bin/lsappinfo", &["front".to_string()]);
    let out = Command::new(prog)
        .args(args)
        .output()
        .expect("spawn sandbox-exec");
    assert!(
        !String::from_utf8_lossy(&out.stdout).contains("ASN:"),
        "LaunchServices must be unreachable under the agent profile"
    );
}

/// Design-assist agents edit `<data>/design/<artifact>/work/**` in place: the
/// OS lets them write there (the regex grant really matches, spaces and dots
/// in the data-dir path included) and nowhere else under `design/` — not the
/// blob store (even a `blobs/work/` look-alike), not a sibling of `work/`.
#[test]
fn seatbelt_agent_profile_opens_design_working_copies_only() {
    if !otto_sandbox::is_supported() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    let data = root.join("Otto Data.v2");
    let cwd = data.join("design").join("A1").join("work");
    let blobs = data.join("design").join("blobs");
    for d in [
        cwd.clone(),
        blobs.join("work"),
        data.join("design").join("A1").join("variants"),
    ] {
        std::fs::create_dir_all(d).unwrap();
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let pol = SandboxPolicy::for_agent(&cwd, Path::new(&home), &data, &[], NetworkPolicy::Full);

    let edit = cwd.join("index.html");
    let (ok, err) = run_sandboxed(&pol, &format!("echo '<h1>x</h1>' > {}", shell_quote(&edit)));
    assert!(ok, "the working copy must be writable: {err}");
    let nested = cwd.join("refs");
    let (ok, err) = run_sandboxed(
        &pol,
        &format!("mkdir -p {0} && echo x > {0}/R1.json", shell_quote(&nested)),
    );
    assert!(ok, "subdirs of the working copy must be writable: {err}");
    for denied in [
        blobs.join("0000"),
        blobs.join("work").join("x"),
        data.join("design").join("A1").join("variants").join("x"),
        data.join("design").join("A1").join("other.txt"),
        data.join("design").join("x.txt"),
    ] {
        let (ok, _) = run_sandboxed(&pol, &format!("echo x > {}", shell_quote(&denied)));
        assert!(!ok, "{} must stay write-denied", denied.display());
        assert!(!denied.exists(), "{} was created", denied.display());
    }
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

/// An EXISTING repo as the OS enforces it: the agent still commits and
/// branches (objects, refs, index), but cannot point the daemon's own git at
/// a program — `.git/config`, `hooks/` and `commondir` are write-denied, and
/// the git dir cannot be renamed away and replaced.
#[test]
fn seatbelt_agent_commits_but_cannot_edit_git_config_or_hooks() {
    if !otto_sandbox::is_supported() {
        return;
    }
    let Some(git) = which_git() else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(tmp.path()).unwrap().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let setup = Command::new(&git)
        .current_dir(&repo)
        .args(["init", "-q"])
        .status()
        .unwrap();
    assert!(setup.success());
    for kv in [
        ["user.email", "a@b.c"],
        ["user.name", "t"],
        ["commit.gpgsign", "false"],
    ] {
        Command::new(&git)
            .current_dir(&repo)
            .args(["config", kv[0], kv[1]])
            .status()
            .unwrap();
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let pol = SandboxPolicy::for_agent(
        &repo,
        Path::new(&home),
        &repo.join(".otto-data"),
        &[repo.join(".git")],
        NetworkPolicy::Full,
    );
    let q = |p: &Path| shell_quote(p);
    let g = q(Path::new(&git));
    let (ok, err) = run_sandboxed(
        &pol,
        &format!(
            "cd {} && echo hi > f.txt && {g} add f.txt && {g} commit -q -m first && {g} branch side",
            q(&repo)
        ),
    );
    assert!(ok, "commit in an existing repo must still work: {err}");

    let config_before = std::fs::read_to_string(repo.join(".git/config")).unwrap();
    let (ok, _) = run_sandboxed(
        &pol,
        &format!("cd {} && {g} config core.fsmonitor /tmp/x.sh", q(&repo)),
    );
    assert!(!ok, "editing .git/config must be refused");
    assert_eq!(
        std::fs::read_to_string(repo.join(".git/config")).unwrap(),
        config_before
    );
    for script in [
        format!(
            "mkdir -p {0}/.git/hooks && echo x > {0}/.git/hooks/pre-commit",
            q(&repo)
        ),
        format!("echo /tmp > {}/.git/commondir", q(&repo)),
        format!("mv {0}/.git {0}/.git-old", q(&repo)),
    ] {
        let (ok, _) = run_sandboxed(&pol, &script);
        assert!(!ok, "must be refused: {script}");
    }
    assert!(repo.join(".git/HEAD").exists());
    assert!(!repo.join(".git/hooks/pre-commit").exists());
}

/// A scratch root OUTSIDE every writable root of the agent profile (the
/// system temp dirs are granted wholesale, so a fake `$HOME` there would make
/// "not granted" checks vacuous).
fn scratch() -> (tempfile::TempDir, std::path::PathBuf) {
    let tmp = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    let root = std::fs::canonicalize(tmp.path()).unwrap();
    (tmp, root)
}

/// Whether the sandboxed `mkdir -p <dir> && echo x > <p>` succeeds (and `p` exists).
fn can_write(pol: &SandboxPolicy, p: &Path) -> bool {
    let dir = p.parent().unwrap();
    let (ok, _) = run_sandboxed(
        pol,
        &format!(
            "mkdir -p {} && echo x > {}",
            shell_quote(dir),
            shell_quote(p)
        ),
    );
    ok && p.exists()
}

fn can_read(pol: &SandboxPolicy, p: &Path) -> bool {
    run_sandboxed(pol, &format!("cat {} >/dev/null", shell_quote(p))).0
}

/// Code-loading configs under `$HOME` an unsandboxed program later runs:
/// `~/.claude.json` (user-scope `mcpServers`), gemini's `settings.json`, and
/// everything in `~/.config` that isn't a CLI's own state (direnv, fish…).
/// Claude Code's own state stays writable.
#[test]
fn seatbelt_agent_cannot_plant_home_code_loading_configs() {
    if !otto_sandbox::is_supported() {
        return;
    }
    let (_tmp, root) = scratch();
    let home = root.join("home");
    let cwd = root.join("project");
    // `~/.config` itself exists on a real host (it is not granted, so a
    // sandboxed `mkdir` of it would fail).
    for d in [
        home.join(".claude"),
        home.join(".gemini"),
        home.join(".config"),
        cwd.clone(),
    ] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(home.join(".claude.json"), "{}").unwrap();
    std::fs::write(home.join(".gemini/settings.json"), "{}").unwrap();
    let pol = SandboxPolicy::for_agent(&cwd, &home, &root.join("Otto"), &[], NetworkPolicy::Full);

    for denied in [
        home.join(".claude.json"),
        home.join(".gemini/settings.json"),
        home.join(".config/direnv/direnvrc"),
        home.join(".config/fish/config.fish"),
        home.join(".claude/settings.json"),
        home.join(".claude/hooks/x.sh"),
    ] {
        assert!(
            !can_write(&pol, &denied),
            "{} must be write-denied",
            denied.display()
        );
    }
    assert_eq!(
        std::fs::read_to_string(home.join(".claude.json")).unwrap(),
        "{}"
    );
    for allowed in [
        home.join(".claude/projects/p/s.jsonl"),
        home.join(".claude/todos/t.json"),
        home.join(".claude/statsig/s"),
        home.join(".gemini/antigravity-cli/state.json"),
        home.join(".config/configstore/update-notifier-x.json"),
        cwd.join("f.txt"),
    ] {
        assert!(
            can_write(&pol, &allowed),
            "{} must stay writable",
            allowed.display()
        );
    }
}

/// A dotfiles-style `~/.claude` symlink: the grant resolves to the real dir,
/// so the denies for not-yet-existing `hooks/` / `agents/` / `commands/` /
/// `settings.local.json` must resolve through it too.
#[test]
fn seatbelt_denies_follow_a_symlinked_claude_dir() {
    if !otto_sandbox::is_supported() {
        return;
    }
    let (_tmp, root) = scratch();
    let home = root.join("home");
    let real = root.join("dotfiles/claude");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&real).unwrap();
    std::os::unix::fs::symlink(&real, home.join(".claude")).unwrap();
    let cwd = root.join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let pol = SandboxPolicy::for_agent(&cwd, &home, &root.join("Otto"), &[], NetworkPolicy::Full);

    for rel in [
        "hooks/x.sh",
        "agents/a.md",
        "commands/c.md",
        "settings.local.json",
    ] {
        let p = home.join(".claude").join(rel);
        assert!(!can_write(&pol, &p), "{rel} via the symlink must be denied");
        assert!(!real.join(rel).exists(), "{rel} was created");
    }
    assert!(can_write(&pol, &home.join(".claude/projects/p.jsonl")));
}

/// A named-account session: its own account home (CLAUDE_CONFIG_DIR /
/// CODEX_HOME) is writable state, but not the configs in it that run code;
/// other accounts' homes and the daemon logs are not even readable.
#[test]
fn seatbelt_account_home_carve_outs_and_other_accounts_hidden() {
    if !otto_sandbox::is_supported() {
        return;
    }
    let (_tmp, root) = scratch();
    let data = root.join("Otto");
    let own = data.join("provider-accounts/a1");
    let other = data.join("provider-accounts/a2");
    let cwd = root.join("project");
    for d in [own.clone(), other.clone(), data.join("logs"), cwd.clone()] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(own.join(".credentials.json"), "mine").unwrap();
    std::fs::write(other.join(".credentials.json"), "theirs").unwrap();
    std::fs::write(data.join("logs/ottod.log"), "log").unwrap();
    let pol = SandboxPolicy::for_agent(
        &cwd,
        &root.join("home"),
        &data,
        std::slice::from_ref(&own),
        NetworkPolicy::Full,
    );

    for denied in [
        own.join("settings.json"),
        own.join("settings.local.json"),
        own.join(".claude.json"),
        own.join("config.toml"),
        own.join("hooks/h.sh"),
        own.join("plugins/p/plugin.json"),
        other.join("settings.json"),
    ] {
        assert!(
            !can_write(&pol, &denied),
            "{} must be write-denied",
            denied.display()
        );
    }
    assert!(can_write(&pol, &own.join("projects/p/s.jsonl")));
    assert!(can_write(&pol, &own.join("sessions/rollout.jsonl")));
    assert!(can_read(&pol, &own.join(".credentials.json")), "own creds");
    assert!(
        !can_read(&pol, &other.join(".credentials.json")),
        "other creds"
    );
    assert!(!can_read(&pol, &data.join("logs/ottod.log")), "daemon logs");
}

/// Read-only sessions (untrusted input) can't plant project-scope claude
/// config that the user's next unconfined `claude` in that folder loads.
#[test]
fn seatbelt_read_only_session_cannot_plant_project_claude_config() {
    if !otto_sandbox::is_supported() {
        return;
    }
    let (_tmp, root) = scratch();
    let cwd = root.join("project");
    std::fs::create_dir_all(cwd.join(".claude")).unwrap();
    let base = SandboxPolicy::for_agent(
        &cwd,
        &root.join("home"),
        &root.join("Otto"),
        &[],
        NetworkPolicy::Full,
    );
    // An ordinary session still edits its repo's checked-in `.claude/`.
    assert!(can_write(&base, &cwd.join(".claude/settings.json")));
    std::fs::remove_file(cwd.join(".claude/settings.json")).unwrap();

    let pol = base.deny_project_agent_config(&cwd);
    for rel in [
        ".claude/settings.json",
        ".claude/settings.local.json",
        ".claude/hooks/h.sh",
        ".claude/agents/a.md",
        ".claude/commands/c.md",
        ".mcp.json",
    ] {
        assert!(
            !can_write(&pol, &cwd.join(rel)),
            "{rel} must be write-denied"
        );
    }
    assert!(can_write(&pol, &cwd.join("notes.md")));
}

/// A linked worktree's `gitdir` pointer: rewriting it would point the
/// daemon's worktree probe (unsandboxed git) at an agent-built repo.
#[test]
fn seatbelt_agent_cannot_repoint_a_worktree_gitdir() {
    if !otto_sandbox::is_supported() {
        return;
    }
    let (_tmp, root) = scratch();
    let repo = root.join("repo");
    let admin = repo.join(".git/worktrees/w1");
    std::fs::create_dir_all(&admin).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(admin.join("gitdir"), "/real/w1/.git\n").unwrap();
    let pol = SandboxPolicy::for_agent(
        &repo,
        &root.join("home"),
        &root.join("Otto"),
        &[repo.join(".git")],
        NetworkPolicy::Full,
    );
    assert!(!can_write(&pol, &admin.join("gitdir")));
    assert_eq!(
        std::fs::read_to_string(admin.join("gitdir")).unwrap(),
        "/real/w1/.git\n"
    );
    // The worktree's own HEAD/index stay writable (checkouts, commits).
    assert!(can_write(&pol, &admin.join("HEAD")));
}
