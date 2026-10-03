//! Spawn budgets per user action (perf G10): a `LocalGit` pointed at a shim
//! that appends each argv to a log before exec'ing the real git. A change that
//! quietly adds a git process to a hot path (status runs on every watcher
//! event, in every window) fails here instead of in a profile months later.
//!
//! Plus an `#[ignore]`d large-tree timing guard for `status()` — run it with
//! `cargo test -p otto-git --lib large_tree_status -- --ignored --nocapture`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::local::LocalGit;

fn sh(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
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
}

/// A repo with a few commits, a remote, and the counting shim.
struct Fixture {
    _tmp: tempfile::TempDir,
    repo: PathBuf,
    log: PathBuf,
    git: LocalGit,
}

impl Fixture {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let remote = root.join("remote.git");
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        sh(&root, &["init", "-q", "--bare", "-b", "main", "remote.git"]);
        sh(&repo, &["init", "-q", "-b", "main"]);
        sh(&repo, &["config", "user.email", "t@example.com"]);
        sh(&repo, &["config", "user.name", "T"]);
        sh(&repo, &["config", "commit.gpgsign", "false"]);
        for i in 0..5 {
            std::fs::write(repo.join("a.txt"), format!("v{i}\n")).unwrap();
            sh(&repo, &["add", "a.txt"]);
            sh(&repo, &["commit", "-q", "-m", &format!("c{i}")]);
        }
        sh(
            &repo,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        sh(&repo, &["push", "-q", "-u", "origin", "main"]);
        let log = root.join("spawns.log");
        let shim = root.join("git-shim.sh");
        std::fs::write(
            &shim,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec git \"$@\"\n",
                log.display()
            ),
        )
        .unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).unwrap();
        let git = LocalGit::new(&repo).with_git_bin(&shim);
        Fixture {
            _tmp: tmp,
            repo,
            log,
            git,
        }
    }

    /// Foreground spawns since the last call (and reset the log). The
    /// background commit-graph seed is excluded — it has its own test.
    fn take_fg(&self) -> Vec<String> {
        self.take()
            .into_iter()
            .filter(|l| !l.starts_with("commit-graph"))
            .collect()
    }

    /// Every spawn since the last call (and reset the log).
    fn take(&self) -> Vec<String> {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let _ = std::fs::remove_file(&self.log);
        text.lines().map(str::to_string).collect()
    }
}

#[tokio::test]
async fn spawn_budget_per_action() {
    let f = Fixture::new();

    std::fs::write(f.repo.join("b.txt"), "new\n").unwrap();
    f.git.status().await.unwrap();
    let s = f.take_fg();
    assert_eq!(s.len(), 1, "status = 1 spawn: {s:?}");

    f.git.stage(&["b.txt".to_string()]).await.unwrap();
    let s = f.take_fg();
    assert!(s.len() <= 2, "stage ≤ 2 spawns: {s:?}");

    f.git.log(50, 0, true).await.unwrap();
    let s = f.take_fg();
    assert_eq!(s.len(), 1, "graph page = 1 spawn: {s:?}");

    f.git.blame("a.txt", "HEAD").await.unwrap();
    let s = f.take_fg();
    assert_eq!(s.len(), 1, "blame = 1 spawn: {s:?}");

    // Cold: three ref lists + default-branch resolution + the two
    // `branch --merged` sets. Warm (nothing moved): the merged sets are
    // memoized, so only the lists and the resolution run.
    f.git.refs().await.unwrap();
    let s = f.take_fg();
    assert!(s.len() <= 8, "cold refs ≤ 8 spawns: {s:?}");
    f.git.refs().await.unwrap();
    let s = f.take_fg();
    assert!(s.len() <= 6, "warm refs ≤ 6 spawns: {s:?}");

    f.git.fetch(None).await.unwrap();
    let s = f.take_fg();
    assert!(s.len() <= 2, "fetch ≤ 2 spawns: {s:?}");
}

#[tokio::test]
async fn commit_graph_seed_runs_once_while_in_flight() {
    let f = Fixture::new();
    let info = f.repo.join(".git/objects/info");
    assert!(!info.join("commit-graph").exists() && !info.join("commit-graphs").exists());
    // Three triggers back to back (log --all, blame, the first status): one
    // write, never three concurrent ones.
    f.git.seed_commit_graph();
    f.git.seed_commit_graph();
    f.git.seed_commit_graph();
    let deadline = Instant::now() + Duration::from_secs(20);
    while LocalGit::seeding_now() > 0 && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let writes = f
        .take()
        .into_iter()
        .filter(|l| l.starts_with("commit-graph write"))
        .count();
    assert!(writes <= 1, "concurrent commit-graph writes: {writes}");
    if std::env::var("OTTO_GIT_COMMIT_GRAPH").map_or(true, |v| v.trim() != "0") {
        assert_eq!(writes, 1);
        assert!(
            info.join("commit-graph").exists() || info.join("commit-graphs").exists(),
            "the split graph was written"
        );
        // Present now: later triggers spawn nothing.
        f.git.seed_commit_graph();
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(f.take().is_empty());
    }
}

/// Large-tree guard: 100k tracked files + 10k untracked, `status()` under a
/// generous budget and the untracked rows capped. Slow to build (~20 s), so
/// ignored by default; the nightly/`ci` nextest profile runs it.
#[tokio::test]
#[ignore]
async fn large_tree_status_timing() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();
    sh(repo, &["init", "-q", "-b", "main"]);
    sh(repo, &["config", "user.email", "t@example.com"]);
    sh(repo, &["config", "user.name", "T"]);
    sh(repo, &["config", "commit.gpgsign", "false"]);
    for d in 0..100 {
        let dir = repo.join(format!("d{d}"));
        std::fs::create_dir_all(&dir).unwrap();
        for f in 0..1000 {
            std::fs::write(dir.join(format!("f{f}.txt")), "x\n").unwrap();
        }
    }
    sh(repo, &["add", "-A"]);
    sh(repo, &["commit", "-q", "-m", "big"]);
    let un = repo.join("untracked");
    std::fs::create_dir_all(&un).unwrap();
    for f in 0..10_000 {
        std::fs::write(un.join(format!("u{f}.txt")), "u\n").unwrap();
    }
    let git = LocalGit::new(repo);
    git.status().await.unwrap(); // warm the OS caches
    let t0 = Instant::now();
    let st = git.status().await.unwrap();
    let took = t0.elapsed();
    eprintln!("status on 100k tracked + 10k untracked: {took:?}");
    assert!(st.untracked_truncated);
    assert_eq!(st.untracked_total, Some(10_000));
    assert_eq!(st.changes.len(), crate::parse::UNTRACKED_ROW_CAP);
    let budget_ms: u64 = std::env::var("OTTO_STATUS_BUDGET_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3_000);
    assert!(
        took < Duration::from_millis(budget_ms),
        "status took {took:?} (budget {budget_ms} ms)"
    );
}
