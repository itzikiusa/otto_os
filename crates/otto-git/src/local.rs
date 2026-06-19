//! Local git operations: shells out to the system `git` binary via
//! `tokio::process`, never prompts (`GIT_TERMINAL_PROMPT=0`), and parses
//! plumbing output with `crate::parse`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use otto_core::api::{
    BranchInfo, CommitInfo, ConflictFile, DiffResp, LocalMergeStrategy, MergeConflictStatus,
    MergeResult, RefBranch, RefTag, RefsResp, RepoStatusResp,
};
use otto_core::{Error, Result};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// What to diff.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffTarget {
    /// Unstaged tracked changes (`git diff`).
    Worktree,
    /// ALL working changes vs HEAD — staged + unstaged combined, plus untracked
    /// files shown as fully added. So a staged-but-uncommitted new file shows
    /// its whole content instead of an empty diff.
    Working,
    /// Staged changes (`git diff --cached`).
    Staged,
    /// A single commit (`git show <sha>`).
    Commit(String),
    /// A commit range (`git diff a..b`).
    Range(String, String),
}

impl DiffTarget {
    /// Parse the `?target=` query value: `worktree | staged | commit:<sha> |
    /// range:<a>..<b>`.
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "worktree" => Ok(Self::Worktree),
            "working" => Ok(Self::Working),
            "staged" => Ok(Self::Staged),
            _ => {
                if let Some(sha) = s.strip_prefix("commit:") {
                    if sha.is_empty() {
                        return Err(Error::Invalid("empty commit sha".into()));
                    }
                    return Ok(Self::Commit(sha.to_string()));
                }
                if let Some(range) = s.strip_prefix("range:") {
                    if let Some((a, b)) = range.split_once("..") {
                        if !a.is_empty() && !b.is_empty() {
                            return Ok(Self::Range(a.to_string(), b.to_string()));
                        }
                    }
                    return Err(Error::Invalid(format!("bad range: {range}")));
                }
                Err(Error::Invalid(format!("bad diff target: {s}")))
            }
        }
    }
}

/// A handle on one local repository; every method spawns `git -C <path> …`.
pub struct LocalGit {
    repo_path: PathBuf,
}

impl LocalGit {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    pub fn path(&self) -> &Path {
        &self.repo_path
    }

    // -- plumbing -----------------------------------------------------------

    fn base_cmd(&self) -> Command {
        let mut cmd = Command::new("git");
        cmd.current_dir(&self.repo_path)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    async fn check_repo(&self) -> Result<()> {
        match tokio::fs::metadata(&self.repo_path).await {
            Ok(m) if m.is_dir() => Ok(()),
            _ => Err(Error::NotFound(format!(
                "repo path missing: {}",
                self.repo_path.display()
            ))),
        }
    }

    /// Run git with args; non-zero exit → `Error::Upstream(first stderr line)`.
    /// Returns stdout.
    async fn run(&self, args: &[&str]) -> Result<String> {
        self.run_env(args, &[]).await.map(|(out, _)| out)
    }

    /// Run git with extra env vars; returns (stdout, stderr).
    async fn run_env(&self, args: &[&str], envs: &[(String, String)]) -> Result<(String, String)> {
        self.check_repo().await?;
        let mut cmd = self.base_cmd();
        cmd.args(args);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = cmd
            .output()
            .await
            .map_err(|e| Error::Internal(format!("spawn git: {e}")))?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        if !out.status.success() {
            return Err(upstream_err(&stderr, &stdout, out.status.code()));
        }
        Ok((stdout, stderr))
    }

    /// Run git but DON'T error on a non-zero exit — return the raw outcome so
    /// the caller can interpret it (used by merge, where conflicts exit non-zero
    /// yet are a normal result). Returns (success, stdout, stderr, exit code).
    async fn run_raw(
        &self,
        args: &[&str],
        envs: &[(String, String)],
    ) -> Result<(bool, String, String, Option<i32>)> {
        self.check_repo().await?;
        let mut cmd = self.base_cmd();
        cmd.args(args);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = cmd
            .output()
            .await
            .map_err(|e| Error::Internal(format!("spawn git: {e}")))?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Ok((out.status.success(), stdout, stderr, out.status.code()))
    }

    // -- queries ------------------------------------------------------------

    pub async fn status(&self) -> Result<RepoStatusResp> {
        let out = self.run(&["status", "--porcelain=v2", "--branch"]).await?;
        Ok(crate::parse::parse_status(&out))
    }

    pub async fn branches(&self) -> Result<Vec<BranchInfo>> {
        let out = self
            .run(&[
                "branch",
                "--format=%(refname:short)%09%(upstream:short)%09%(HEAD)",
            ])
            .await?;
        Ok(crate::parse::parse_branches(&out))
    }

    pub async fn current_branch(&self) -> Result<String> {
        let out = self.run(&["rev-parse", "--abbrev-ref", "HEAD"]).await?;
        Ok(out.trim().to_string())
    }

    /// Create (or reset) a linked worktree at `path` on `branch`, based on
    /// `base` (a branch/sha/HEAD). Used by the Agent Swarm to give each code
    /// agent an isolated, unique working directory it can edit in parallel.
    /// `-B` resets the branch to `base`; `--force` tolerates a path git still
    /// tracks from a stale prior run.
    ///
    /// DESTRUCTIVE: because `-B` resets `branch` to `base`, calling this on an
    /// existing worktree throws away any commits the branch had accumulated.
    /// For multi-turn swarm work use [`worktree_add_if_absent`] instead, which
    /// only creates on first use and otherwise reuses the existing tree.
    pub async fn worktree_add(&self, path: &str, branch: &str, base: &str) -> Result<()> {
        self.run(&["worktree", "add", "--force", "-B", branch, path, base])
            .await?;
        Ok(())
    }

    /// True when `path` is already registered as a linked worktree of this repo.
    /// Reads `git worktree list --porcelain` (each tree is a `worktree <abs>`
    /// line) and compares canonicalized paths so symlink/`..` differences don't
    /// cause a false negative. Returns `false` (rather than erroring) when the
    /// listing fails or the path can't be canonicalized.
    pub async fn worktree_exists(&self, path: &str) -> bool {
        let (ok, stdout, _, _) = match self
            .run_raw(&["worktree", "list", "--porcelain"], &[])
            .await
        {
            Ok(v) => v,
            Err(_) => return false,
        };
        if !ok {
            return false;
        }
        let want = std::fs::canonicalize(path).ok();
        stdout
            .lines()
            .filter_map(|l| l.strip_prefix("worktree "))
            .any(|registered| {
                let registered = registered.trim();
                if registered == path {
                    return true;
                }
                match (std::fs::canonicalize(registered).ok(), want.as_ref()) {
                    (Some(r), Some(w)) => &r == w,
                    _ => false,
                }
            })
    }

    /// Non-destructive worktree provisioning for multi-turn agents.
    ///
    /// On FIRST use the worktree doesn't exist yet, so this creates it exactly
    /// like [`worktree_add`] (`-B` + `--force`), branching `branch` from `base`.
    /// On every later turn the worktree already exists with the agent's prior
    /// commits, so this is a no-op and the agent resumes on top of its own work
    /// — `base` is ignored and the branch is NOT reset. Returns `true` when it
    /// created the worktree, `false` when it reused an existing one.
    pub async fn worktree_add_if_absent(
        &self,
        path: &str,
        branch: &str,
        base: &str,
    ) -> Result<bool> {
        if self.worktree_exists(path).await {
            return Ok(false);
        }
        self.worktree_add(path, branch, base).await?;
        Ok(true)
    }

    /// Remove a linked worktree at `path` (force-removes dirty/locked trees).
    /// Best-effort: a missing worktree is not an error.
    pub async fn worktree_remove(&self, path: &str) -> Result<()> {
        let _ = self
            .run(&["worktree", "remove", "--force", path])
            .await;
        Ok(())
    }

    pub async fn log(&self, limit: u32, skip: u32, all: bool) -> Result<Vec<CommitInfo>> {
        let limit_s = limit.to_string();
        let skip_s = skip.to_string();
        let mut args = vec![
            "log",
            "--pretty=format:%H%x1f%h%x1f%an%x1f%aI%x1f%s%x1f%P%x1f%D%x1e",
            "-n",
            &limit_s,
            "--skip",
            &skip_s,
        ];
        if all {
            args.insert(1, "--all");
        }
        let out = self.run(&args).await?;
        crate::parse::parse_log(&out)
    }

    pub async fn refs(&self) -> Result<RefsResp> {
        // Local branches: name TAB upstream TAB HEAD-marker
        let local_out = self
            .run(&[
                "for-each-ref",
                "--format=%(refname:short)\t%(upstream:short)\t%(HEAD)",
                "refs/heads",
            ])
            .await?;
        let local = local_out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let mut cols = line.splitn(3, '\t');
                let name = cols.next().unwrap_or("").to_string();
                let upstream_raw = cols.next().unwrap_or("").trim().to_string();
                let head = cols.next().unwrap_or("").trim();
                RefBranch {
                    name,
                    is_current: head == "*",
                    upstream: if upstream_raw.is_empty() {
                        None
                    } else {
                        Some(upstream_raw)
                    },
                    remote: false,
                }
            })
            .collect();

        // Remote branches: name only; skip entries ending in "/HEAD"
        let remote_out = self
            .run(&["for-each-ref", "--format=%(refname:short)", "refs/remotes"])
            .await?;
        let remote = remote_out
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim().ends_with("/HEAD"))
            .map(|line| RefBranch {
                name: line.trim().to_string(),
                is_current: false,
                upstream: None,
                remote: true,
            })
            .collect();

        // Tags: sorted newest-first, capped at 200
        let tags_out = self
            .run(&[
                "for-each-ref",
                "--sort=-creatordate",
                "--format=%(refname:short)",
                "refs/tags",
            ])
            .await?;
        let tags = tags_out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .take(200)
            .map(|line| RefTag {
                name: line.trim().to_string(),
            })
            .collect();

        Ok(RefsResp {
            local,
            remote,
            tags,
        })
    }

    pub async fn diff(&self, target: DiffTarget) -> Result<DiffResp> {
        let out = match &target {
            DiffTarget::Worktree => self.run(&["diff", "--no-color", "-U3", "-M"]).await?,
            DiffTarget::Working => {
                // Staged + unstaged tracked changes vs HEAD (a staged-new file
                // shows as fully added). Falls back to cached+worktree when HEAD
                // is unborn (no commits yet).
                let (head_ok, head_out, _, _) = self
                    .run_raw(&["diff", "--no-color", "-U3", "-M", "HEAD"], &[])
                    .await?;
                let mut out = if head_ok {
                    head_out
                } else {
                    let mut s = self
                        .run(&["diff", "--no-color", "-U3", "-M", "--cached"])
                        .await
                        .unwrap_or_default();
                    s.push_str(&self.run(&["diff", "--no-color", "-U3", "-M"]).await.unwrap_or_default());
                    s
                };
                // Untracked files: render each as a fully-added diff. `git diff
                // --no-index` exits non-zero when content differs — run_raw is
                // tolerant of that.
                let (_, untracked, _, _) = self
                    .run_raw(&["ls-files", "--others", "--exclude-standard"], &[])
                    .await?;
                for f in untracked.lines().filter(|l| !l.trim().is_empty()) {
                    let (_, stdout, _, _) = self
                        .run_raw(
                            &["diff", "--no-color", "-U3", "--no-index", "--", "/dev/null", f],
                            &[],
                        )
                        .await?;
                    out.push_str(&stdout);
                }
                out
            }
            DiffTarget::Staged => {
                self.run(&["diff", "--no-color", "-U3", "-M", "--cached"])
                    .await?
            }
            DiffTarget::Commit(sha) => {
                self.run(&["show", "--no-color", "-U3", "-M", "--format=", sha])
                    .await?
            }
            DiffTarget::Range(a, b) => {
                let range = format!("{a}..{b}");
                self.run(&["diff", "--no-color", "-U3", "-M", &range])
                    .await?
            }
        };
        Ok(crate::parse::parse_diff(&out))
    }

    /// Run `git diff <base>` — diffs the working tree (staged + unstaged)
    /// against `base` and returns the raw unified diff text.
    pub async fn diff_text_against(&self, base: &str) -> Result<String> {
        self.run(&["diff", base]).await
    }

    /// Raw unified diff of the staged changes (`git diff --cached`). Empty when
    /// nothing is staged.
    pub async fn staged_diff_text(&self) -> Result<String> {
        self.run(&["diff", "--no-color", "-M", "--cached"]).await
    }

    /// Raw unified diff of all unstaged tracked changes (`git diff`). Used as a
    /// fallback when nothing is staged.
    pub async fn working_diff_text(&self) -> Result<String> {
        self.run(&["diff", "--no-color", "-M"]).await
    }

    /// `git remote get-url origin`, best-effort.
    pub async fn remote_url(&self) -> Option<String> {
        self.run(&["remote", "get-url", "origin"])
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Absolute path of the work-tree root containing `repo_path` (walks up to
    /// the enclosing `.git`), or an error if the path is not inside a repo.
    pub async fn toplevel(&self) -> Result<String> {
        let out = self.run(&["rev-parse", "--show-toplevel"]).await?;
        let top = out.trim().to_string();
        if top.is_empty() {
            return Err(Error::Invalid("not a git repository".into()));
        }
        Ok(top)
    }

    // -- mutations ----------------------------------------------------------

    pub async fn checkout(&self, branch: &str, create: bool) -> Result<()> {
        if create {
            self.run(&["checkout", "-b", branch]).await?;
        } else {
            self.run(&["checkout", branch]).await?;
        }
        Ok(())
    }

    pub async fn stage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Err(Error::Invalid("no paths to stage".into()));
        }
        let mut args = vec!["add", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.run(&args).await?;
        Ok(())
    }

    pub async fn unstage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Err(Error::Invalid("no paths to unstage".into()));
        }
        let mut args = vec!["restore", "--staged", "--"];
        args.extend(paths.iter().map(String::as_str));
        self.run(&args).await?;
        Ok(())
    }

    /// Discard all working-tree + staged changes for `paths`, reverting them to
    /// their HEAD state. New files (untracked/added) are removed entirely;
    /// everything else (modified/deleted/renamed/conflicted) is restored from
    /// HEAD. Destructive and irreversible — the UI confirms first.
    pub async fn discard(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Err(Error::Invalid("no paths to discard".into()));
        }
        let want: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
        // Classify each requested path by its current change kind.
        let status = self.status().await?;
        let mut restore: Vec<String> = Vec::new(); // tracked → revert to HEAD
        let mut remove: Vec<String> = Vec::new(); // new → delete
        for c in &status.changes {
            if !want.contains(c.path.as_str()) {
                continue;
            }
            match c.kind.as_str() {
                "untracked" | "added" => remove.push(c.path.clone()),
                _ => restore.push(c.path.clone()),
            }
        }
        if !restore.is_empty() {
            let mut args = vec!["restore", "--staged", "--worktree", "--source=HEAD", "--"];
            args.extend(restore.iter().map(String::as_str));
            self.run(&args).await?;
        }
        if !remove.is_empty() {
            // Unstage first (a staged-new file → untracked), then `clean` removes
            // the untracked files/dirs. `reset` is a no-op for already-untracked.
            let mut reset = vec!["reset", "-q", "--"];
            reset.extend(remove.iter().map(String::as_str));
            let _ = self.run(&reset).await;
            let mut clean = vec!["clean", "-fdq", "--"];
            clean.extend(remove.iter().map(String::as_str));
            self.run(&clean).await?;
        }
        Ok(())
    }

    /// Commit staged changes; returns the new HEAD sha.
    pub async fn commit(&self, message: &str, amend: bool) -> Result<String> {
        if message.trim().is_empty() {
            return Err(Error::Invalid("empty commit message".into()));
        }
        let mut args = vec!["commit", "-m", message];
        if amend {
            args.push("--amend");
        }
        self.run(&args).await?;
        let sha = self.run(&["rev-parse", "HEAD"]).await?;
        Ok(sha.trim().to_string())
    }

    /// `git push`; for https remotes pass the account token so the askpass
    /// helper can answer credential prompts. Returns combined output.
    ///
    /// A branch that was never pushed has no upstream, so a plain `git push`
    /// fails ("has no upstream branch"). We detect that and retry with
    /// `--set-upstream origin <branch>`, so pushing (and creating a PR from) a
    /// fresh branch just works.
    pub async fn push(&self, token: Option<String>) -> Result<String> {
        let askpass = match &token {
            Some(t) => Some(AskPass::new(t)?),
            None => None,
        };
        let envs = askpass.as_ref().map(AskPass::envs).unwrap_or_default();
        let combine = |stdout: &str, stderr: &str| {
            let mut c = String::from(stdout.trim_end());
            if !stderr.trim().is_empty() {
                if !c.is_empty() {
                    c.push('\n');
                }
                c.push_str(stderr.trim_end());
            }
            c
        };

        let (ok, stdout, stderr, code) = self.run_raw(&["push"], &envs).await?;
        if ok {
            return Ok(combine(&stdout, &stderr));
        }
        if stderr.contains("has no upstream branch") || stderr.contains("--set-upstream") {
            let branch = self.current_branch().await?;
            let (ok2, stdout2, stderr2, code2) = self
                .run_raw(&["push", "--set-upstream", "origin", &branch], &envs)
                .await?;
            if ok2 {
                return Ok(combine(&stdout2, &stderr2));
            }
            return Err(upstream_err(&stderr2, &stdout2, code2));
        }
        Err(upstream_err(&stderr, &stdout, code))
    }

    pub async fn pull(&self, token: Option<String>) -> Result<String> {
        self.run_remote(&["pull", "--no-rebase"], token).await
    }

    pub async fn fetch(&self, token: Option<String>) -> Result<String> {
        self.run_remote(&["fetch", "--prune"], token).await
    }

    async fn run_remote(&self, args: &[&str], token: Option<String>) -> Result<String> {
        let askpass = match token {
            Some(t) => Some(AskPass::new(&t)?),
            None => None,
        };
        let envs = askpass.as_ref().map(AskPass::envs).unwrap_or_default();
        let (stdout, stderr) = self.run_env(args, &envs).await?;
        // git writes progress/summary to stderr; surface both.
        let mut combined = String::new();
        combined.push_str(stdout.trim_end());
        if !stderr.trim().is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(stderr.trim_end());
        }
        Ok(combined)
    }

    pub async fn stash_save(&self) -> Result<String> {
        let (out, _) = self.run_env(&["stash", "push"], &[]).await?;
        Ok(out.trim().to_string())
    }

    pub async fn stash_pop(&self) -> Result<String> {
        let (out, _) = self.run_env(&["stash", "pop"], &[]).await?;
        Ok(out.trim().to_string())
    }

    // -- merge + conflict resolution ----------------------------------------

    /// True when a merge is in progress (`MERGE_HEAD` exists).
    async fn is_merging(&self) -> bool {
        let (ok, _, _, _) = self
            .run_raw(&["rev-parse", "-q", "--verify", "MERGE_HEAD"], &[])
            .await
            .unwrap_or((false, String::new(), String::new(), None));
        ok
    }

    /// Conflicted paths from a fresh status (porcelain v2 `u` entries).
    async fn conflicted_paths(&self) -> Result<Vec<String>> {
        let st = self.status().await?;
        Ok(st
            .changes
            .iter()
            .filter(|c| c.kind == "conflicted")
            .map(|c| c.path.clone())
            .collect())
    }

    /// Merge `source` into `target`. Never auto-resolves; conflicts are returned
    /// as `Ok(MergeResult{status:"conflicts", ..})`, not an error.
    pub async fn merge_branch(
        &self,
        source: &str,
        target: &str,
        strategy: LocalMergeStrategy,
    ) -> Result<MergeResult> {
        let already_merging = self.is_merging().await;

        // Guard: refuse to start a merge on a dirty tree (but allow continuing an
        // in-progress merge whose working tree naturally shows conflicts).
        if !already_merging {
            let st = self.status().await?;
            let dirty = st
                .changes
                .iter()
                .any(|c| (c.staged || c.unstaged) && c.kind != "untracked");
            if dirty {
                return Err(Error::Conflict(
                    "working tree has uncommitted changes; commit or stash first".into(),
                ));
            }
        }

        // Ensure the target branch is checked out.
        if self.current_branch().await? != target {
            self.checkout(target, false).await?;
        }

        // Build the merge argv with EXPLICIT non-interactive flags. Crucially we
        // never pass `-X ours/-X theirs` or any auto-resolution strategy.
        //
        // `-c merge.conflictStyle=diff3` is a TOP-LEVEL git flag (before the
        // `merge` subcommand) so conflict markers include the merge base (the
        // `|||||||` section). It only changes how conflicts are *rendered*, never
        // whether they auto-resolve — the "no auto-merge" guarantee is intact.
        let args: Vec<&str> = match strategy {
            LocalMergeStrategy::MergeCommit => vec![
                "-c",
                "merge.conflictStyle=diff3",
                "merge",
                "--no-ff",
                "--no-edit",
                source,
            ],
            LocalMergeStrategy::Ff => {
                vec!["-c", "merge.conflictStyle=diff3", "merge", "--no-edit", source]
            }
            LocalMergeStrategy::FfOnly => {
                vec!["-c", "merge.conflictStyle=diff3", "merge", "--ff-only", source]
            }
            LocalMergeStrategy::Squash => {
                vec!["-c", "merge.conflictStyle=diff3", "merge", "--squash", source]
            }
        };
        let envs = vec![("GIT_TERMINAL_PROMPT".to_string(), "0".to_string())];
        let (success, stdout, stderr, code) = self.run_raw(&args, &envs).await?;
        let combined = format!("{stdout}\n{stderr}");

        if success {
            // Distinguish "nothing to do" from a real merge.
            if combined.contains("Already up to date") || combined.contains("Already up-to-date") {
                return Ok(MergeResult {
                    status: "up_to_date".into(),
                    commit: None,
                    conflicted_files: Vec::new(),
                    repo_status: self.status().await?,
                });
            }
            // `--squash` leaves changes staged but creates NO commit; the caller
            // must still run merge/commit, so report commit = None.
            let commit = if matches!(strategy, LocalMergeStrategy::Squash) {
                None
            } else {
                Some(self.run(&["rev-parse", "HEAD"]).await?.trim().to_string())
            };
            return Ok(MergeResult {
                status: "merged".into(),
                commit,
                conflicted_files: Vec::new(),
                repo_status: self.status().await?,
            });
        }

        // Non-zero exit. Conflict markers / unmerged paths → a normal "conflicts"
        // result; anything else (ff-only impossible, bad ref, fatal) is an error.
        let conflicted = self.conflicted_paths().await?;
        let is_conflict = combined.contains("CONFLICT")
            || combined.contains("Automatic merge failed")
            || !conflicted.is_empty();
        if is_conflict {
            return Ok(MergeResult {
                status: "conflicts".into(),
                commit: None,
                conflicted_files: conflicted,
                repo_status: self.status().await?,
            });
        }
        Err(upstream_err(&stderr, &stdout, code))
    }

    /// Current merge-in-progress status: whether a merge is underway, the
    /// best-effort source ref, and the conflicted file list.
    pub async fn merge_status(&self) -> Result<MergeConflictStatus> {
        let merging = self.is_merging().await;
        if !merging {
            return Ok(MergeConflictStatus {
                merging: false,
                source: None,
                conflicted_files: Vec::new(),
            });
        }
        let conflicted_files = self.conflicted_paths().await?;
        let source = self.merge_source().await;
        Ok(MergeConflictStatus {
            merging,
            source,
            conflicted_files,
        })
    }

    /// Best-effort source ref for an in-progress merge: first line of
    /// `.git/MERGE_MSG` (e.g. "Merge branch 'feature'"), else the MERGE_HEAD sha.
    async fn merge_source(&self) -> Option<String> {
        let git_dir = self.repo_path.join(".git");
        let msg_path = git_dir.join("MERGE_MSG");
        if let Ok(text) = tokio::fs::read_to_string(&msg_path).await {
            if let Some(line) = text.lines().find(|l| !l.trim().is_empty()) {
                return Some(line.trim().to_string());
            }
        }
        self.run(&["rev-parse", "MERGE_HEAD"])
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Read a conflicted working-tree file and split it into ordered segments.
    /// Binary files report `is_binary=true` with no segments.
    pub async fn conflict_file(&self, path: &str) -> Result<ConflictFile> {
        let abs = self.safe_join(path)?;
        let bytes = tokio::fs::read(&abs)
            .await
            .map_err(|e| Error::NotFound(format!("read {path}: {e}")))?;
        if bytes.contains(&0u8) {
            return Ok(ConflictFile {
                path: path.to_string(),
                is_binary: true,
                segments: Vec::new(),
            });
        }
        let text = String::from_utf8_lossy(&bytes);
        Ok(ConflictFile {
            path: path.to_string(),
            is_binary: false,
            segments: crate::parse::parse_conflict_segments(&text),
        })
    }

    /// Write the fully-resolved content of `path` and stage it.
    pub async fn write_resolution(&self, path: &str, content: &str) -> Result<()> {
        let abs = self.safe_join(path)?;
        if let Some(parent) = abs.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::Internal(format!("mkdir for {path}: {e}")))?;
        }
        tokio::fs::write(&abs, content)
            .await
            .map_err(|e| Error::Internal(format!("write {path}: {e}")))?;
        self.run(&["add", "--", path]).await?;
        Ok(())
    }

    /// Finish an in-progress merge (real merge OR staged squash). Fails when
    /// conflicts remain unresolved.
    pub async fn merge_commit(&self, message: Option<String>) -> Result<MergeResult> {
        if !self.conflicted_paths().await?.is_empty() {
            return Err(Error::Conflict("unresolved conflicts remain".into()));
        }
        match message {
            Some(m) if !m.trim().is_empty() => {
                self.run(&["commit", "-m", &m]).await?;
            }
            _ => {
                self.run(&["commit", "--no-edit"]).await?;
            }
        }
        let commit = self.run(&["rev-parse", "HEAD"]).await?.trim().to_string();
        Ok(MergeResult {
            status: "merged".into(),
            commit: Some(commit),
            conflicted_files: Vec::new(),
            repo_status: self.status().await?,
        })
    }

    /// Abort an in-progress merge (`git merge --abort`) or, for a staged squash
    /// with no MERGE_HEAD, discard the staged changes (`git reset --hard HEAD`).
    pub async fn merge_abort(&self) -> Result<RepoStatusResp> {
        if self.is_merging().await {
            self.run(&["merge", "--abort"]).await?;
        } else {
            self.run(&["reset", "--hard", "HEAD"]).await?;
        }
        self.status().await
    }

    /// Join `rel` under the repo root, rejecting absolute paths and any `..`
    /// component so a resolution can't escape the work-tree.
    fn safe_join(&self, rel: &str) -> Result<PathBuf> {
        let p = Path::new(rel);
        if p.is_absolute() {
            return Err(Error::Invalid(format!("path must be relative: {rel}")));
        }
        for comp in p.components() {
            match comp {
                std::path::Component::ParentDir => {
                    return Err(Error::Invalid(format!("path escapes repo: {rel}")));
                }
                std::path::Component::Prefix(_) | std::path::Component::RootDir => {
                    return Err(Error::Invalid(format!("path must be relative: {rel}")));
                }
                _ => {}
            }
        }
        Ok(self.repo_path.join(p))
    }
}

fn upstream_err(stderr: &str, stdout: &str, code: Option<i32>) -> Error {
    let first = stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .or_else(|| stdout.lines().find(|l| !l.trim().is_empty()))
        .unwrap_or("git failed with no output");
    Error::Upstream(format!(
        "git exited {}: {}",
        code.map_or_else(|| "?".to_string(), |c| c.to_string()),
        first.trim()
    ))
}

// ---------------------------------------------------------------------------
// Askpass helper for https remotes
// ---------------------------------------------------------------------------

/// Temp executable script handed to git via GIT_ASKPASS. Echoes a placeholder
/// username for "Username" prompts and the token (provided via env var
/// OTTO_GIT_TOKEN, never written to disk) for everything else. Works for
/// GitHub (any username + PAT), Bitbucket (x-token-auth or app-password user)
/// and GitLab (any username + PAT).
struct AskPass {
    // Held to keep the temp file alive for the duration of the command.
    _file: tempfile::TempPath,
    path: PathBuf,
    token: String,
}

impl AskPass {
    fn new(token: &str) -> Result<Self> {
        use std::io::Write;
        let mut f = tempfile::Builder::new()
            .prefix("otto-askpass-")
            .suffix(".sh")
            .tempfile()
            .map_err(|e| Error::Internal(format!("askpass tmp: {e}")))?;
        f.write_all(
            b"#!/bin/sh\ncase \"$1\" in\n  *sername*) echo \"${OTTO_GIT_USERNAME:-x-token-auth}\" ;;\n  *) echo \"$OTTO_GIT_TOKEN\" ;;\nesac\n",
        )
        .map_err(|e| Error::Internal(format!("askpass write: {e}")))?;
        f.flush().ok();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o700))
                .map_err(|e| Error::Internal(format!("askpass chmod: {e}")))?;
        }
        let path = f.path().to_path_buf();
        Ok(Self {
            _file: f.into_temp_path(),
            path,
            token: token.to_string(),
        })
    }

    fn envs(&self) -> Vec<(String, String)> {
        vec![
            (
                "GIT_ASKPASS".to_string(),
                self.path.to_string_lossy().into_owned(),
            ),
            ("OTTO_GIT_TOKEN".to_string(), self.token.clone()),
        ]
    }
}

// ---------------------------------------------------------------------------
// Clone
// ---------------------------------------------------------------------------

/// Clone `url` into `dest`, streaming progress lines (from git's stderr) into
/// `progress`. Token is used via askpass for https remotes.
pub async fn clone_repo(
    url: &str,
    dest: &Path,
    token: Option<&str>,
    mut progress: impl FnMut(String) + Send,
) -> Result<()> {
    let askpass = match token {
        Some(t) => Some(AskPass::new(t)?),
        None => None,
    };
    let mut cmd = Command::new("git");
    cmd.arg("clone")
        .arg("--progress")
        .arg(url)
        .arg(dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if let Some(a) = &askpass {
        for (k, v) in a.envs() {
            cmd.env(k, v);
        }
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Internal(format!("spawn git clone: {e}")))?;

    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Internal("clone stderr unavailable".into()))?;

    // git progress lines are \r-terminated; split on both \r and \n.
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let mut last_line = String::new();
    loop {
        let n = stderr
            .read(&mut chunk)
            .await
            .map_err(|e| Error::Internal(format!("clone read: {e}")))?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        while let Some(pos) = buf.iter().position(|&b| b == b'\n' || b == b'\r') {
            let line: Vec<u8> = buf.drain(..=pos).collect();
            let text = String::from_utf8_lossy(&line[..line.len() - 1])
                .trim()
                .to_string();
            if !text.is_empty() {
                last_line = text.clone();
                progress(text);
            }
        }
    }
    if !buf.is_empty() {
        let text = String::from_utf8_lossy(&buf).trim().to_string();
        if !text.is_empty() {
            last_line = text.clone();
            progress(text);
        }
    }

    let status = child
        .wait()
        .await
        .map_err(|e| Error::Internal(format!("clone wait: {e}")))?;
    if !status.success() {
        return Err(Error::Upstream(format!(
            "git clone exited {}: {}",
            status.code().map_or_else(|| "?".into(), |c| c.to_string()),
            last_line
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests — real throwaway repos under the system temp dir
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::api::LineOrigin;

    /// Run `git` synchronously for fixture setup.
    fn sh_git(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .env("GIT_TERMINAL_PROMPT", "0")
            .args(args)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn write(dir: &Path, rel: &str, content: &str) {
        let p = dir.join(rel);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(p, content).unwrap();
    }

    /// Repo with two commits, a staged rename, a staged add, an unstaged
    /// modification and an untracked file.
    fn fixture() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        sh_git(&dir, &["init", "-b", "main"]);
        sh_git(&dir, &["config", "user.email", "otto@test.local"]);
        sh_git(&dir, &["config", "user.name", "Otto Test"]);
        sh_git(&dir, &["config", "commit.gpgsign", "false"]);

        write(&dir, "a.txt", "alpha line 1\nalpha line 2\nalpha line 3\n");
        write(
            &dir,
            "c.txt",
            "carrot content that is long enough to track renames\n",
        );
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "first commit"]);

        write(
            &dir,
            "a.txt",
            "alpha line 1\nalpha CHANGED 2\nalpha line 3\n",
        );
        sh_git(&dir, &["add", "a.txt"]);
        sh_git(&dir, &["commit", "-m", "second commit"]);

        // staged rename
        sh_git(&dir, &["mv", "c.txt", "d.txt"]);
        // staged new file
        write(&dir, "f.txt", "fresh\n");
        sh_git(&dir, &["add", "f.txt"]);
        // unstaged modification
        write(
            &dir,
            "a.txt",
            "alpha line 1\nalpha CHANGED 2\nalpha line 3\nappended\n",
        );
        // untracked
        write(&dir, "e.txt", "loose\n");

        (tmp, dir)
    }

    #[tokio::test]
    async fn end_to_end_status_log_diff_commit() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);

        // status
        let st = git.status().await.unwrap();
        assert_eq!(st.branch, "main");
        let by_path = |p: &str| st.changes.iter().find(|c| c.path == p).cloned();
        let ren = by_path("d.txt").expect("rename present");
        assert_eq!(ren.kind, "renamed");
        assert_eq!(ren.orig_path.as_deref(), Some("c.txt"));
        assert!(ren.staged && !ren.unstaged);
        let add = by_path("f.txt").expect("added present");
        assert_eq!(add.kind, "added");
        assert!(add.staged);
        let m = by_path("a.txt").expect("modified present");
        assert_eq!(m.kind, "modified");
        assert!(!m.staged && m.unstaged);
        let unt = by_path("e.txt").expect("untracked present");
        assert_eq!(unt.kind, "untracked");

        // branches / current
        let branches = git.branches().await.unwrap();
        let main = branches.iter().find(|b| b.name == "main").unwrap();
        assert!(main.is_current);
        assert_eq!(git.current_branch().await.unwrap(), "main");

        // log
        let log = git.log(10, 0, false).await.unwrap();
        assert_eq!(log.len(), 2);
        assert_eq!(log[0].subject, "second commit");
        assert_eq!(log[1].subject, "first commit");
        assert_eq!(log[0].author, "Otto Test");
        let one = git.log(1, 1, false).await.unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].subject, "first commit");

        // staged diff: rename detected, new file present
        let staged = git.diff(DiffTarget::Staged).await.unwrap();
        let dren = staged.files.iter().find(|f| f.path == "d.txt").unwrap();
        assert_eq!(dren.old_path.as_deref(), Some("c.txt"));
        assert!(staged.files.iter().any(|f| f.path == "f.txt"));

        // worktree diff: a.txt with one added line numbered 4 (untracked excluded)
        let wt = git.diff(DiffTarget::Worktree).await.unwrap();
        assert!(!wt.files.iter().any(|f| f.path == "e.txt"));
        let fa = wt.files.iter().find(|f| f.path == "a.txt").unwrap();
        let adds: Vec<_> = fa.hunks[0]
            .lines
            .iter()
            .filter(|l| l.origin == LineOrigin::Add)
            .collect();
        assert_eq!(adds.len(), 1);
        assert_eq!(adds[0].content, "appended");
        assert_eq!(adds[0].new_line, Some(4));

        // commit diff of HEAD (the a.txt change)
        let head = git.log(1, 0, false).await.unwrap()[0].sha.clone();
        let cd = git.diff(DiffTarget::Commit(head.clone())).await.unwrap();
        assert_eq!(cd.files.len(), 1);
        assert_eq!(cd.files[0].path, "a.txt");

        // range diff
        let first = git.log(1, 1, false).await.unwrap()[0].sha.clone();
        let rd = git
            .diff(DiffTarget::Range(first.clone(), head.clone()))
            .await
            .unwrap();
        assert_eq!(rd.files.len(), 1);

        // stage the modification, commit, verify log grows and sha returned
        git.stage(&["a.txt".into()]).await.unwrap();
        let sha = git.commit("third commit", false).await.unwrap();
        assert_eq!(sha.len(), 40);
        let log = git.log(10, 0, false).await.unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(log[0].sha, sha);

        // unstage works
        git.stage(&["e.txt".into()]).await.unwrap();
        git.unstage(&["e.txt".into()]).await.unwrap();
        let st = git.status().await.unwrap();
        assert_eq!(
            st.changes.iter().find(|c| c.path == "e.txt").unwrap().kind,
            "untracked"
        );

        // checkout -b
        git.checkout("feature/x", true).await.unwrap();
        assert_eq!(git.current_branch().await.unwrap(), "feature/x");
        git.checkout("main", false).await.unwrap();

        // stash save/pop round-trip
        write(&dir, "a.txt", "stash me\n");
        git.stash_save().await.unwrap();
        let st = git.status().await.unwrap();
        assert!(!st.changes.iter().any(|c| c.path == "a.txt"));
        git.stash_pop().await.unwrap();
        let st = git.status().await.unwrap();
        assert!(st.changes.iter().any(|c| c.path == "a.txt"));
    }

    #[tokio::test]
    async fn missing_repo_dir_is_not_found() {
        let git = LocalGit::new("/tmp/otto-definitely-not-a-repo-xyz");
        match git.status().await {
            Err(Error::NotFound(_)) => {}
            other => panic!("expected NotFound, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn git_failure_maps_to_upstream() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        match git.checkout("no-such-branch", false).await {
            Err(Error::Upstream(msg)) => assert!(msg.contains("git exited")),
            other => panic!("expected Upstream, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn clone_local_repo_with_progress() {
        let (_tmp, dir) = fixture();
        let dest_tmp = tempfile::tempdir().unwrap();
        let dest = dest_tmp.path().join("cloned");
        let mut lines = Vec::new();
        clone_repo(dir.to_str().unwrap(), &dest, None, |l| lines.push(l))
            .await
            .unwrap();
        assert!(dest.join(".git").exists());
        assert!(!lines.is_empty(), "expected progress output");
        let cloned = LocalGit::new(&dest);
        assert_eq!(cloned.log(10, 0, false).await.unwrap().len(), 2);
    }

    /// D1 regression: a worktree provisioned with `worktree_add_if_absent` must
    /// be REUSED on the second call (not reset), so an agent's committed work
    /// from a prior turn survives. The old unconditional `worktree_add`
    /// (`-B`/`--force`) would discard it by resetting the branch to base.
    #[tokio::test]
    async fn worktree_add_if_absent_reuses_and_preserves_commits() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        let wt = dir.parent().unwrap().join("agent-wt");
        let wt_str = wt.to_str().unwrap().to_string();
        let branch = "swarm/s1/a1";

        // First turn: created from absent → true.
        assert!(!git.worktree_exists(&wt_str).await);
        let created = git
            .worktree_add_if_absent(&wt_str, branch, "HEAD")
            .await
            .unwrap();
        assert!(created, "first call should create the worktree");
        assert!(git.worktree_exists(&wt_str).await);

        // Agent does work IN the worktree and commits it (multi-turn progress).
        let wt_git = LocalGit::new(&wt);
        write(&wt, "agent_work.txt", "turn 1 output\n");
        wt_git.stage(&["agent_work.txt".into()]).await.unwrap();
        let sha = wt_git.commit("agent turn 1", false).await.unwrap();

        // Second turn: already exists → reuse (false), NO reset. The commit and
        // the file must still be there.
        let created2 = git
            .worktree_add_if_absent(&wt_str, branch, "HEAD")
            .await
            .unwrap();
        assert!(!created2, "second call should reuse, not recreate");
        assert_eq!(
            wt_git.current_branch().await.unwrap(),
            branch,
            "still on the agent's branch"
        );
        let head = wt_git.log(1, 0, false).await.unwrap();
        assert_eq!(head[0].sha, sha, "prior commit preserved");
        assert_eq!(head[0].subject, "agent turn 1");
        assert!(wt.join("agent_work.txt").exists(), "committed file preserved");
    }

    /// `worktree_exists` is path-aware: false for an unrelated path, true once
    /// registered (even via a non-canonical path with a trailing component).
    #[tokio::test]
    async fn worktree_exists_tracks_registration() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        let wt = dir.parent().unwrap().join("wt2");
        let wt_str = wt.to_str().unwrap().to_string();

        assert!(!git.worktree_exists(&wt_str).await);
        git.worktree_add(&wt_str, "swarm/s/b", "HEAD").await.unwrap();
        assert!(git.worktree_exists(&wt_str).await);
        // An unrelated path is not a worktree.
        assert!(!git.worktree_exists("/tmp/definitely-not-a-worktree-xyz").await);
    }

    #[test]
    fn diff_target_parse() {
        assert_eq!(DiffTarget::parse("worktree").unwrap(), DiffTarget::Worktree);
        assert_eq!(DiffTarget::parse("staged").unwrap(), DiffTarget::Staged);
        assert_eq!(
            DiffTarget::parse("commit:abc").unwrap(),
            DiffTarget::Commit("abc".into())
        );
        assert_eq!(
            DiffTarget::parse("range:a1..b2").unwrap(),
            DiffTarget::Range("a1".into(), "b2".into())
        );
        assert!(DiffTarget::parse("bogus").is_err());
        assert!(DiffTarget::parse("range:onlyone").is_err());
        assert!(DiffTarget::parse("commit:").is_err());
    }
}
