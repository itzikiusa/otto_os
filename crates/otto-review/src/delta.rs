//! Incremental delta re-review (E-08a): computes the commit/diff delta between
//! a previous completed review's recorded `head_sha` and the current reviewed
//! `HEAD`, and formats a focused prompt section (with a clean full-diff
//! fallback notice when the branch was rebased or force-pushed).

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use otto_core::finding::{Finding, FindingStatus};
use otto_core::{Id, Result};
use tokio::io::AsyncReadExt;

use crate::engine::ReviewBranches;

/// Default byte cap for inline incremental delta hunks (32 KiB).
pub const DEFAULT_MAX_DELTA_DIFF_BYTES: usize = 32 * 1024;

/// Maximum commits listed in the incremental delta summary.
const MAX_DELTA_COMMITS: &str = "50";

/// Hard ceiling per local git subprocess so a wedged repository never stalls a review.
const GIT_CMD_TIMEOUT: Duration = Duration::from_secs(15);

/// Computed git delta between a previous review's `prev_head_sha` and the
/// current review's `head_sha`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeltaReviewContext {
    pub prev_head_sha: String,
    pub head_sha: String,
    pub fast_forward: bool,
    pub commits: Vec<String>,
    pub delta_stat: String,
    pub delta_diff: String,
    pub truncated: bool,
}

/// Validate that `rev` looks like a safe hexadecimal git commit SHA (4..=64
/// ASCII hex characters, never empty, never flag-like such as `--exec=...`).
pub fn is_valid_git_sha(rev: &str) -> bool {
    let s = rev.trim();
    (4..=64).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Compute the incremental git delta between `prev_head_sha` and `head_sha` in
/// `repo_path`, capping the inline unified diff at `max_diff_bytes` on a clean
/// UTF-8 char boundary.
///
/// Returns `None` when either SHA is invalid/flag-like, `repo_path` is not a
/// readable git repository, or `head_sha` does not resolve to a commit.
///
/// When `prev_head_sha` is not an ancestor of `head_sha` (including when the
/// old commit was force-pushed away and is unreachable locally), returns
/// `Some(DeltaReviewContext { fast_forward: false, .. })` so the prompt tells
/// reviewers to fall back cleanly to the full PR diff.
pub async fn compute_delta(
    repo_path: &Path,
    prev_head_sha: &str,
    head_sha: &str,
    max_diff_bytes: usize,
) -> Option<DeltaReviewContext> {
    let prev = prev_head_sha.trim();
    let head = head_sha.trim();
    if !is_valid_git_sha(prev) || !is_valid_git_sha(head) {
        return None;
    }

    let resolved_head = rev_parse_commit(repo_path, head).await?;
    let Some(resolved_prev) = rev_parse_commit(repo_path, prev).await else {
        // The previous reviewed commit is no longer reachable in local objects
        // (common after a force-push + fetch): report rebase/force-push fallback.
        return Some(DeltaReviewContext {
            prev_head_sha: prev.to_string(),
            head_sha: resolved_head,
            fast_forward: false,
            commits: Vec::new(),
            delta_stat: String::new(),
            delta_diff: String::new(),
            truncated: false,
        });
    };

    if resolved_prev.eq_ignore_ascii_case(&resolved_head) {
        return Some(DeltaReviewContext {
            prev_head_sha: resolved_prev,
            head_sha: resolved_head,
            fast_forward: true,
            commits: Vec::new(),
            delta_stat: String::new(),
            delta_diff: String::new(),
            truncated: false,
        });
    }

    let fast_forward = is_ancestor(repo_path, &resolved_prev, &resolved_head).await?;
    if !fast_forward {
        return Some(DeltaReviewContext {
            prev_head_sha: resolved_prev,
            head_sha: resolved_head,
            fast_forward: false,
            commits: Vec::new(),
            delta_stat: String::new(),
            delta_diff: String::new(),
            truncated: false,
        });
    }

    let range = format!("{resolved_prev}..{resolved_head}");
    let log_text = run_git_text(
        repo_path,
        &["log", "--oneline", "-n", MAX_DELTA_COMMITS, &range, "--"],
    )
    .await?;
    let commits: Vec<String> = log_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();

    let delta_stat = run_git_text(repo_path, &["diff", "--stat", &range, "--"])
        .await?
        .trim_end()
        .to_string();

    let (delta_diff, truncated) = run_git_diff_capped(repo_path, &range, max_diff_bytes).await?;

    Some(DeltaReviewContext {
        prev_head_sha: resolved_prev,
        head_sha: resolved_head,
        fast_forward: true,
        commits,
        delta_stat,
        delta_diff,
        truncated,
    })
}

/// Format an incremental delta section for reviewer/scope/verifier prompts.
pub fn format_delta_section(
    delta: &DeltaReviewContext,
    open_prior_findings: &[(String, String)],
) -> String {
    let short_prev = short_sha(&delta.prev_head_sha);
    let short_head = short_sha(&delta.head_sha);
    let mut out =
        format!("\n## Incremental Delta Since Previous Review ({short_prev}..{short_head})\n");

    if !delta.fast_forward {
        out.push_str(&format!(
            "Branch history diverged (rebased or force-pushed) since the previous review: `{short_prev}` is not an ancestor of `{short_head}`. Incremental commit hunks cannot be isolated reliably — fall back to reviewing the full causal PR diff file while re-verifying every open prior finding against the current code.\n"
        ));
    } else if delta.prev_head_sha.eq_ignore_ascii_case(&delta.head_sha)
        || (delta.commits.is_empty() && delta.delta_diff.is_empty())
    {
        out.push_str(&format!(
            "No new commits landed since the previous review (HEAD remains `{short_head}`). Re-verify open prior findings against the current code and review the full causal diff as needed.\n"
        ));
    } else {
        out.push_str(
            "Fast-forward update since the previous review. Focus extra attention on the newly added commits and delta hunks below, and re-verify every open prior finding against the current code (the full causal PR diff file remains authoritative for overall PR scope).\n",
        );

        if !delta.commits.is_empty() {
            out.push_str(&format!("\n### New commits ({})\n", delta.commits.len()));
            for line in &delta.commits {
                out.push_str("- ");
                out.push_str(line);
                out.push('\n');
            }
        }

        if !delta.delta_stat.is_empty() {
            out.push_str("\n### Delta diffstat\n```text\n");
            out.push_str(&delta.delta_stat);
            out.push_str("\n```\n");
        }

        if !delta.delta_diff.is_empty() {
            out.push_str("\n### Delta hunks\n```diff\n");
            out.push_str(&delta.delta_diff);
            if !delta.delta_diff.ends_with('\n') {
                out.push('\n');
            }
            out.push_str("```\n");
        }

        if delta.truncated {
            out.push_str(
                "> Note: Incremental delta diff was truncated to stay within the prompt byte budget; read the full PR diff file for any remaining hunks.\n",
            );
        }
    }

    if !open_prior_findings.is_empty() {
        out.push_str("\n### Open prior findings to re-verify\n");
        for (id, summary) in open_prior_findings {
            let clean: String = summary.split_whitespace().collect::<Vec<_>>().join(" ");
            out.push_str(&format!("- `{id}`: {clean}\n"));
        }
    }

    out
}

/// Extract still-open prior findings as `(finding_id, concise_summary)` pairs
/// for [`format_delta_section`].
pub fn open_prior_finding_pairs(findings: &[Finding]) -> Vec<(String, String)> {
    findings
        .iter()
        .filter(|f| {
            !matches!(
                f.status,
                FindingStatus::Fixed
                    | FindingStatus::Verified
                    | FindingStatus::FalsePositive
                    | FindingStatus::Waived
            ) && !matches!(f.state.as_str(), "resolved" | "declined")
        })
        .map(|f| {
            let headline = if !f.title.trim().is_empty() {
                f.title.trim()
            } else {
                f.body
                    .lines()
                    .find(|l| !l.trim().is_empty())
                    .map(str::trim)
                    .unwrap_or("")
            };
            let headline = otto_core::text::clip_chars(headline, 160);
            let loc = match (&f.path, f.line) {
                (Some(p), Some(line)) if !p.is_empty() => format!(" {p}:{line}"),
                (Some(p), None) if !p.is_empty() => format!(" {p}"),
                _ => String::new(),
            };
            let summary = format!("[{}]{loc} — {headline}", f.severity.as_str());
            (f.id.clone(), summary)
        })
        .collect()
}

/// Load prior review context (identical to [`crate::history::prior_review_context`])
/// and append an incremental delta section whenever the prior review recorded a
/// `head_sha` and `repo_path` resolves a current `HEAD`.
#[allow(clippy::too_many_arguments)]
pub async fn prior_review_context_with_delta(
    reviews: &otto_state::ReviewsRepo,
    findings_store: &otto_state::ReviewFindingsRepo,
    repo_id: &Id,
    pr: u64,
    current: &Id,
    branches: Option<&ReviewBranches>,
    explicit: Option<&Id>,
    repo_path: &Path,
) -> Result<(String, Vec<String>)> {
    let Some(mut snap) = crate::history::load_prior_review(
        reviews,
        findings_store,
        repo_id,
        pr,
        current,
        branches,
        explicit,
    )
    .await?
    else {
        return Ok((String::new(), vec![]));
    };

    if let Some(prev_head) = snap.prev_head_sha.as_deref() {
        if let Some(cur_head) = rev_parse_head(repo_path).await {
            if let Some(delta) = compute_delta(
                repo_path,
                prev_head,
                &cur_head,
                DEFAULT_MAX_DELTA_DIFF_BYTES,
            )
            .await
            {
                let open_prior = open_prior_finding_pairs(&snap.findings);
                snap.context
                    .push_str(&format_delta_section(&delta, &open_prior));
            }
        }
    }

    Ok((snap.context, snap.finding_ids))
}

fn short_sha(sha: &str) -> String {
    sha.trim().chars().take(12).collect()
}

async fn rev_parse_head(repo_path: &Path) -> Option<String> {
    rev_parse_commit(repo_path, "HEAD").await
}

async fn rev_parse_commit(repo_path: &Path, rev: &str) -> Option<String> {
    let spec = format!("{rev}^{{commit}}");
    let out = run_git_text(repo_path, &["rev-parse", "--verify", "--quiet", &spec]).await?;
    let sha = out.trim();
    is_valid_git_sha(sha).then(|| sha.to_string())
}

async fn is_ancestor(repo_path: &Path, ancestor: &str, descendant: &str) -> Option<bool> {
    let mut cmd = otto_git::hardened_command();
    cmd.arg("-C")
        .arg(repo_path)
        .args(["merge-base", "--is-ancestor", ancestor, descendant])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let status = tokio::time::timeout(GIT_CMD_TIMEOUT, cmd.status())
        .await
        .ok()?
        .ok()?;
    match status.code() {
        Some(0) => Some(true),
        Some(1) => Some(false),
        _ => None,
    }
}

async fn run_git_text(repo_path: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = otto_git::hardened_command();
    cmd.arg("-C")
        .arg(repo_path)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let out = tokio::time::timeout(GIT_CMD_TIMEOUT, cmd.output())
        .await
        .ok()?
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

async fn run_git_diff_capped(
    repo_path: &Path,
    range: &str,
    max_diff_bytes: usize,
) -> Option<(String, bool)> {
    let mut cmd = otto_git::hardened_command();
    cmd.arg("-C")
        .arg(repo_path)
        .args(["diff", range, "--"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let mut child = cmd.spawn().ok()?;
    let stdout = child.stdout.take()?;

    // Read up to `max_diff_bytes + 4` bytes so we both detect truncation
    // without buffering unbounded output and avoid splitting a valid UTF-8
    // codepoint right at `max_diff_bytes`.
    let read_cap = (max_diff_bytes as u64).saturating_add(4);
    let mut buf = Vec::with_capacity(max_diff_bytes.min(64 * 1024).saturating_add(4));
    let read_res = tokio::time::timeout(GIT_CMD_TIMEOUT, async {
        stdout.take(read_cap).read_to_end(&mut buf).await
    })
    .await;

    match read_res {
        Ok(Ok(_)) => {}
        _ => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return None;
        }
    }

    let capped_by_reader = buf.len() > max_diff_bytes;
    if capped_by_reader {
        let _ = child.kill().await;
        let _ = child.wait().await;
    } else {
        let status = tokio::time::timeout(GIT_CMD_TIMEOUT, child.wait())
            .await
            .ok()?
            .ok()?;
        if !status.success() {
            return None;
        }
    }

    let clean_slice = if capped_by_reader {
        trim_incomplete_utf8_tail(&buf)
    } else {
        &buf[..]
    };
    let text = String::from_utf8_lossy(clean_slice);
    let (clipped, clipped_utf8) = truncate_utf8_bytes(&text, max_diff_bytes);
    Some((clipped.to_string(), capped_by_reader || clipped_utf8))
}

fn trim_incomplete_utf8_tail(bytes: &[u8]) -> &[u8] {
    let len = bytes.len();
    let mut i = len;
    while i > 0 && len - i < 4 && (bytes[i - 1] & 0b1100_0000) == 0b1000_0000 {
        i -= 1;
    }
    if i == 0 {
        return bytes;
    }
    match std::str::from_utf8(&bytes[i - 1..]) {
        Err(e) if e.error_len().is_none() => &bytes[..i - 1],
        _ => bytes,
    }
}

fn truncate_utf8_bytes(s: &str, max_bytes: usize) -> (&str, bool) {
    if s.len() <= max_bytes {
        return (s, false);
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(clippy::disallowed_methods)] // test helper: sync git in throwaway tempdir
    fn sh(dir: &Path, args: &[&str]) -> String {
        let mut cmd = otto_git::hardened_std_command();
        let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
        let out = cmd
            .args(args)
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", null)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "tester")
            .env("GIT_AUTHOR_EMAIL", "tester@example.com")
            .env("GIT_COMMITTER_NAME", "tester")
            .env("GIT_COMMITTER_EMAIL", "tester@example.com")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?} failed: {out:?}");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn init_repo(dir: &Path) -> String {
        sh(dir, &["init", "-q", "-b", "main"]);
        sh(dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(
            dir.join("app.rs"),
            "fn main() {\n    println!(\"v1\");\n}\n",
        )
        .unwrap();
        sh(dir, &["add", "."]);
        sh(dir, &["commit", "-qm", "initial commit"]);
        sh(dir, &["rev-parse", "HEAD"])
    }

    #[tokio::test]
    async fn fast_forward_delta_lists_commits_stat_hunks_and_prior_findings() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_sha = init_repo(tmp.path());

        // Commit 1 on top of prev_sha
        std::fs::write(
            tmp.path().join("app.rs"),
            "fn main() {\n    println!(\"v2\");\n}\n",
        )
        .unwrap();
        sh(tmp.path(), &["commit", "-qam", "fix: validate auth header"]);

        // Commit 2 on top of commit 1
        std::fs::write(
            tmp.path().join("helper.rs"),
            "pub fn check() -> bool { true }\n",
        )
        .unwrap();
        sh(tmp.path(), &["add", "helper.rs"]);
        sh(tmp.path(), &["commit", "-qm", "feat: add helper check"]);
        let head_sha = sh(tmp.path(), &["rev-parse", "HEAD"]);

        let delta = compute_delta(
            tmp.path(),
            &prev_sha,
            &head_sha,
            DEFAULT_MAX_DELTA_DIFF_BYTES,
        )
        .await
        .expect("fast-forward delta");

        assert!(delta.fast_forward);
        assert!(!delta.truncated);
        assert_eq!(delta.prev_head_sha, prev_sha);
        assert_eq!(delta.head_sha, head_sha);
        assert_eq!(delta.commits.len(), 2, "{:?}", delta.commits);
        assert!(delta.commits[0].contains("feat: add helper check"));
        assert!(delta.commits[1].contains("fix: validate auth header"));
        assert!(delta.delta_stat.contains("app.rs"));
        assert!(delta.delta_stat.contains("helper.rs"));
        assert!(delta.delta_diff.contains("println!(\"v2\")"));
        assert!(delta.delta_diff.contains("pub fn check()"));

        let prior = vec![(
            "F-001".to_string(),
            "[high] app.rs:2 — Missing auth check".to_string(),
        )];
        let section = format_delta_section(&delta, &prior);
        assert!(section.contains(&format!(
            "## Incremental Delta Since Previous Review ({}..{})",
            &prev_sha[..12],
            &head_sha[..12]
        )));
        assert!(section.contains("### New commits (2)"));
        assert!(section.contains("fix: validate auth header"));
        assert!(section.contains("### Delta diffstat"));
        assert!(section.contains("### Delta hunks"));
        assert!(section.contains("### Open prior findings to re-verify"));
        assert!(section.contains("`F-001`: [high] app.rs:2 — Missing auth check"));
        assert!(!section.contains("rebased or force-pushed"));
    }

    #[tokio::test]
    async fn rebased_or_diverged_history_sets_fast_forward_false_with_fallback_notice() {
        let tmp = tempfile::tempdir().unwrap();
        let base_sha = init_repo(tmp.path());

        // Original PR commit (previous review)
        sh(tmp.path(), &["checkout", "-q", "-b", "feature"]);
        std::fs::write(
            tmp.path().join("app.rs"),
            "fn main() { /* old attempt */ }\n",
        )
        .unwrap();
        sh(tmp.path(), &["commit", "-qam", "old feature commit"]);
        let prev_sha = sh(tmp.path(), &["rev-parse", "HEAD"]);

        // Simulate rebase / force-push: reset back to base and commit a diverged history
        sh(tmp.path(), &["reset", "--hard", "-q", &base_sha]);
        std::fs::write(
            tmp.path().join("app.rs"),
            "fn main() { /* rewritten history */ }\n",
        )
        .unwrap();
        sh(tmp.path(), &["commit", "-qam", "rewritten feature commit"]);
        let head_sha = sh(tmp.path(), &["rev-parse", "HEAD"]);

        let delta = compute_delta(
            tmp.path(),
            &prev_sha,
            &head_sha,
            DEFAULT_MAX_DELTA_DIFF_BYTES,
        )
        .await
        .expect("rebase delta context");

        assert!(!delta.fast_forward);
        assert!(!delta.truncated);
        assert!(delta.commits.is_empty());
        assert!(delta.delta_diff.is_empty());

        let prior = vec![(
            "F-002".to_string(),
            "[medium] app.rs:1 — Re-verify after rebase".to_string(),
        )];
        let section = format_delta_section(&delta, &prior);
        assert!(section.contains("rebased or force-pushed"));
        assert!(section.contains("full causal PR diff"));
        assert!(section.contains("`F-002`"));
    }

    #[tokio::test]
    async fn missing_prev_sha_after_force_push_falls_back_cleanly() {
        let tmp = tempfile::tempdir().unwrap();
        let head_sha = init_repo(tmp.path());
        let missing_prev = "deadbeefcafebabedeadbeefcafebabedeadbeef";

        let delta = compute_delta(
            tmp.path(),
            missing_prev,
            &head_sha,
            DEFAULT_MAX_DELTA_DIFF_BYTES,
        )
        .await
        .expect("unreachable prev SHA falls back to non-fast-forward");

        assert!(!delta.fast_forward);
        assert_eq!(delta.prev_head_sha, missing_prev);
        assert_eq!(delta.head_sha, head_sha);
        assert!(delta.delta_diff.is_empty());
    }

    #[tokio::test]
    async fn identical_prev_and_head_reports_no_new_commits() {
        let tmp = tempfile::tempdir().unwrap();
        let sha = init_repo(tmp.path());

        let delta = compute_delta(tmp.path(), &sha, &sha, DEFAULT_MAX_DELTA_DIFF_BYTES)
            .await
            .expect("same-head delta");

        assert!(delta.fast_forward);
        assert!(!delta.truncated);
        assert!(delta.commits.is_empty());
        assert!(delta.delta_diff.is_empty());

        let section = format_delta_section(&delta, &[]);
        assert!(section.contains("No new commits landed"));
    }

    #[tokio::test]
    async fn large_delta_diff_truncates_cleanly_on_utf8_boundary() {
        let tmp = tempfile::tempdir().unwrap();
        let prev_sha = init_repo(tmp.path());

        // Write multi-byte UTF-8 characters (3-byte `—` and 4-byte `🚀`) across many lines
        // so capping at a small byte limit cuts inside a multi-byte sequence.
        let mut content = String::new();
        for i in 0..80 {
            content.push_str(&format!("// line {i:02} — rocket 🚀 payload\n"));
        }
        std::fs::write(tmp.path().join("unicode.txt"), &content).unwrap();
        sh(tmp.path(), &["add", "unicode.txt"]);
        sh(tmp.path(), &["commit", "-qm", "add unicode file"]);
        let head_sha = sh(tmp.path(), &["rev-parse", "HEAD"]);

        // Test several tight byte caps around multi-byte UTF-8 boundaries.
        for cap in [120usize, 121, 122, 123, 256] {
            let delta = compute_delta(tmp.path(), &prev_sha, &head_sha, cap)
                .await
                .expect("truncated delta");
            assert!(delta.fast_forward);
            assert!(delta.truncated, "cap={cap} must mark truncated");
            assert!(
                delta.delta_diff.len() <= cap,
                "cap={cap}: got {} bytes",
                delta.delta_diff.len()
            );
            assert!(
                !delta.delta_diff.contains('\u{FFFD}'),
                "cap={cap}: must not corrupt UTF-8 with replacement char: {:?}",
                delta.delta_diff
            );
            let section = format_delta_section(&delta, &[]);
            assert!(section.contains("truncated"));
        }
    }

    #[tokio::test]
    async fn flag_like_and_invalid_sha_inputs_are_rejected_safely() {
        let tmp = tempfile::tempdir().unwrap();
        let valid_sha = init_repo(tmp.path());

        let bad_inputs = [
            "",
            "   ",
            "abc", // too short
            "--exec=touch /tmp/pwned",
            "-n",
            "--version",
            "HEAD",
            "main",
            &format!("{valid_sha}..{valid_sha}"),
            "zzzzzzzz",
            "01234567\n--exec=id",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0", // 65 chars
        ];

        for bad in bad_inputs {
            assert!(!is_valid_git_sha(bad), "should reject {bad:?}");
            assert_eq!(
                compute_delta(tmp.path(), bad, &valid_sha, DEFAULT_MAX_DELTA_DIFF_BYTES).await,
                None,
                "prev={bad:?} must return None"
            );
            assert_eq!(
                compute_delta(tmp.path(), &valid_sha, bad, DEFAULT_MAX_DELTA_DIFF_BYTES).await,
                None,
                "head={bad:?} must return None"
            );
        }
    }

    #[tokio::test]
    async fn prior_review_context_with_delta_combines_lineage_and_git_delta() {
        let dir = tempfile::tempdir().unwrap();
        let repo_dir = dir.path().join("repo");
        std::fs::create_dir(&repo_dir).unwrap();
        let first_sha = init_repo(&repo_dir);

        let pool = otto_state::db::open(&dir.path().join("state.sqlite"))
            .await
            .unwrap();
        let user = otto_state::UsersRepo::new(pool.clone())
            .create("reviewer", "hash", "Reviewer", true)
            .await
            .unwrap();
        let workspace = otto_state::WorkspacesRepo::new(pool.clone())
            .create("Review", repo_dir.to_str().unwrap(), &user.id)
            .await
            .unwrap();
        let repo = otto_state::GitStore::new(pool.clone())
            .create_repo(otto_state::NewRepo {
                workspace_id: workspace.id.clone(),
                name: "Repo".into(),
                path: repo_dir.to_string_lossy().into_owned(),
                remote_url: None,
                provider: None,
                git_account_id: None,
            })
            .await
            .unwrap();

        let reviews = otto_state::ReviewsRepo::new(pool.clone());
        let findings = otto_state::ReviewFindingsRepo::new(pool);

        // First review at `first_sha` with one open finding
        let first = reviews.create_review(&repo.id, 42).await.unwrap();
        reviews
            .set_run_context(
                &first.id,
                &otto_state::ReviewRunContext {
                    source_branch: Some("feat/delta".into()),
                    cwd: Some(repo_dir.to_string_lossy().into_owned()),
                    head_sha: Some(first_sha.clone()),
                },
            )
            .await
            .unwrap();
        let (finding, _) = findings
            .upsert(&otto_state::NewFinding {
                review_id: &first.id,
                workspace_id: &workspace.id,
                repo_id: &repo.id,
                pr_number: Some(42),
                path: Some("app.rs"),
                line: Some(2),
                line_end: None,
                severity: "high",
                category: Some("correctness"),
                title: "Unchecked unwrap",
                body: "Panics on empty input",
                evidence: "app.rs:2",
                agent_reasoning_summary: "Direct unwrap",
                suggested_fix: None,
                produced_by_agent: Some("review"),
                reviewer: "review",
                fingerprint: "fp-delta-1",
                run_id: &first.id,
            })
            .await
            .unwrap();
        reviews
            .set_status(&first.id, otto_core::domain::ReviewStatus::Done, None)
            .await
            .unwrap();

        // Advance HEAD with a fix commit
        std::fs::write(
            repo_dir.join("app.rs"),
            "fn main() {\n    println!(\"safe\");\n}\n",
        )
        .unwrap();
        sh(
            &repo_dir,
            &["commit", "-qam", "fix: handle empty input safely"],
        );
        let second_sha = sh(&repo_dir, &["rev-parse", "HEAD"]);

        // Second review computes combined prior + incremental delta context
        let second = reviews.create_review(&repo.id, 42).await.unwrap();
        let (ctx_md, ids) = prior_review_context_with_delta(
            &reviews, &findings, &repo.id, 42, &second.id, None, None, &repo_dir,
        )
        .await
        .unwrap();

        assert_eq!(ids, vec![finding.id.clone()]);
        assert!(ctx_md.contains(&format!("## Prior review {}", first.id)));
        assert!(ctx_md.contains(&format!(
            "## Incremental Delta Since Previous Review ({}..{})",
            &first_sha[..12],
            &second_sha[..12]
        )));
        assert!(ctx_md.contains("fix: handle empty input safely"));
        assert!(ctx_md.contains("println!(\"safe\")"));
        assert!(ctx_md.contains(&finding.id));
    }
}
