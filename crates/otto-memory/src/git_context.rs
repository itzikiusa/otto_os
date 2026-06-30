//! Git context for the Repo Brain: recent history, an author blame summary, and
//! recent merge ("PR") commits. Shells out to the `git` CLI, bounded by a
//! wall-clock timeout and output caps, and degrades to empty on any failure (no
//! git, not a repo, detached state) — it is purely additive context.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::process::Command;

const GIT_TIMEOUT: Duration = Duration::from_secs(5);
/// Cap on captured stdout (blame of a large file) — defends memory/time.
const MAX_OUT: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitInfo {
    pub hash: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlameAuthor {
    pub author: String,
    pub lines: usize,
}

/// Git context for a single file.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FileGitContext {
    pub commits: Vec<CommitInfo>,
    pub blame: Vec<BlameAuthor>,
    pub last_change: Option<String>,
}

/// Git context for a whole repo.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RepoGitContext {
    pub head: Option<String>,
    pub branch: Option<String>,
    pub recent: Vec<CommitInfo>,
    /// Recent merge commits — a proxy for "recently merged PRs".
    pub merges: Vec<CommitInfo>,
}

/// Run `git -C root <args>` with a timeout; returns stdout on success.
async fn run_git(root: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args(args);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::null());
    let child = cmd.spawn().ok()?;
    let out = tokio::time::timeout(GIT_TIMEOUT, child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut s = String::from_utf8_lossy(&out.stdout).to_string();
    if s.len() > MAX_OUT {
        s.truncate(MAX_OUT);
    }
    Some(s)
}

/// Parse `git log --format=%h\x1f%an\x1f%ad\x1f%s` output into commits.
fn parse_log(out: &str) -> Vec<CommitInfo> {
    out.lines()
        .filter_map(|l| {
            let mut parts = l.splitn(4, '\u{1f}');
            Some(CommitInfo {
                hash: parts.next()?.to_string(),
                author: parts.next()?.to_string(),
                date: parts.next()?.to_string(),
                subject: parts.next().unwrap_or("").to_string(),
            })
        })
        .collect()
}

const LOG_FMT: &str = "--pretty=format:%h%x1f%an%x1f%ad%x1f%s";

/// Recent history + blame summary for one file.
pub async fn file_context(root: &Path, file_rel: &str, commit_limit: usize) -> FileGitContext {
    let n = commit_limit.clamp(1, 50).to_string();
    let mut ctx = FileGitContext::default();

    if let Some(out) = run_git(root, &["log", "-n", &n, LOG_FMT, "--date=short", "--", file_rel]).await {
        ctx.commits = parse_log(&out);
        ctx.last_change = ctx.commits.first().map(|c| c.date.clone());
    }

    // Blame summary: count lines per author (porcelain) → top authors.
    if let Some(out) = run_git(root, &["blame", "--line-porcelain", "--", file_rel]).await {
        use std::collections::HashMap;
        let mut counts: HashMap<String, usize> = HashMap::new();
        for line in out.lines() {
            if let Some(name) = line.strip_prefix("author ") {
                *counts.entry(name.to_string()).or_default() += 1;
            }
        }
        let mut v: Vec<BlameAuthor> = counts
            .into_iter()
            .map(|(author, lines)| BlameAuthor { author, lines })
            .collect();
        v.sort_by_key(|b| std::cmp::Reverse(b.lines));
        v.truncate(5);
        ctx.blame = v;
    }
    ctx
}

/// Repo-level context: HEAD, branch, recent commits, recent merges.
pub async fn repo_context(root: &Path, limit: usize) -> RepoGitContext {
    let n = limit.clamp(1, 50).to_string();
    let mut ctx = RepoGitContext {
        head: run_git(root, &["rev-parse", "--short", "HEAD"])
            .await
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        branch: run_git(root, &["rev-parse", "--abbrev-ref", "HEAD"])
            .await
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        ..Default::default()
    };
    if let Some(out) = run_git(root, &["log", "-n", &n, LOG_FMT, "--date=short"]).await {
        ctx.recent = parse_log(&out);
    }
    if let Some(out) = run_git(root, &["log", "-n", &n, "--merges", LOG_FMT, "--date=short"]).await {
        ctx.merges = parse_log(&out);
    }
    ctx
}
