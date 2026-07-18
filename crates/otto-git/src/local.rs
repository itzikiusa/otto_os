//! Local git operations: shells out to the system `git` binary via
//! `tokio::process`, never prompts (`GIT_TERMINAL_PROMPT=0`), and parses
//! plumbing output with `crate::parse`.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use otto_core::api::{
    BranchInfo, CommitInfo, ConflictFile, DiffResp, LocalMergeStrategy, MergeConflictStatus,
    MergePreview, MergeResult, RefBranch, RefTag, RefsResp, RepoStatusResp, StashInfo,
    SubmoduleInfo, WorktreeInfo,
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

/// A verified diff/PR base: `diff_ref` is a rev that exists in this checkout
/// (possibly `origin/x`); `branch` is the logical branch name a PR targets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedBase {
    pub diff_ref: String,
    pub branch: String,
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
    /// Returns stdout. Public for callers with plumbing needs the typed API
    /// doesn't cover (e.g. the workflow engine's worktree reaper resolving
    /// `--git-common-dir` / branch reachability).
    pub async fn run(&self, args: &[&str]) -> Result<String> {
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
            let err = upstream_err(&stderr, &stdout, out.status.code());
            // The HTTP layer collapses this to a bare 502 in the access log;
            // record WHAT failed here so the daemon log is diagnosable.
            tracing::warn!(
                repo = %self.repo_path.display(),
                args = ?args,
                code = out.status.code(),
                "git failed: {err}"
            );
            return Err(err);
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

    /// True iff `commit` is an ancestor of (already merged into) `branch`.
    ///
    /// Used to detect merge-to-`develop` completion. `git merge-base
    /// --is-ancestor` exits 0 (ancestor), 1 (not an ancestor), or another code
    /// (error, e.g. an unknown ref) — distinguished here so a bad ref surfaces as
    /// an error rather than a silent `false`.
    pub async fn is_ancestor_of(&self, commit: &str, branch: &str) -> Result<bool> {
        let (ok, _out, stderr, code) = self
            .run_raw(&["merge-base", "--is-ancestor", commit, branch], &[])
            .await?;
        match (ok, code) {
            (true, _) => Ok(true),
            (false, Some(1)) => Ok(false),
            (false, _) => Err(Error::Internal(format!(
                "merge-base --is-ancestor {commit} {branch}: {stderr}"
            ))),
        }
    }

    pub async fn status(&self) -> Result<RepoStatusResp> {
        // `--untracked-files=all` lists every untracked FILE individually instead
        // of collapsing an entirely-new directory (e.g. `.claude/skills/` with
        // 80+ files) into a single entry — so the Changes view can show/stage
        // them per-file. Gitignored paths are still excluded.
        let out = self
            .run(&["status", "--porcelain=v2", "--branch", "--untracked-files=all"])
            .await?;
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

    /// Resolve a ref (branch/sha/`HEAD`) to its full commit SHA. Used by Goal
    /// Loops to capture the launch HEAD as the diff base for the loop's branch.
    pub async fn rev_parse(&self, reference: &str) -> Result<String> {
        let out = self.run(&["rev-parse", reference]).await?;
        Ok(out.trim().to_string())
    }

    /// Files this worktree's HEAD changed relative to `base` (`git diff
    /// --name-only base...HEAD`). Used by the swarm to detect when two agents'
    /// branches touch the same shared files. Empty on no changes.
    pub async fn changed_files(&self, base: &str) -> Result<Vec<String>> {
        let range = format!("{base}...HEAD");
        let out = self.run(&["diff", "--name-only", &range]).await?;
        Ok(out
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect())
    }

    /// True when a local branch already exists. Lets Goal Loops re-attach an
    /// existing loop branch NON-destructively instead of `-B`-resetting it.
    pub async fn branch_exists(&self, branch: &str) -> bool {
        let refname = format!("refs/heads/{branch}");
        match self
            .run_raw(&["rev-parse", "--verify", "--quiet", &refname], &[])
            .await
        {
            Ok((ok, _, _, _)) => ok,
            Err(_) => false,
        }
    }

    /// True when `r` resolves to a commit in this checkout. `--end-of-options`
    /// stops an option-looking "ref" (e.g. `--output=…` arriving from untrusted
    /// run input) from being parsed as a flag — here and, because callers only
    /// diff refs this verified, downstream in `git diff` too.
    async fn verify_commit_ref(&self, r: &str) -> bool {
        let spec = format!("{r}^{{commit}}");
        match self
            .run_raw(
                &["rev-parse", "--verify", "--quiet", "--end-of-options", &spec],
                &[],
            )
            .await
        {
            Ok((ok, _, _, _)) => ok,
            Err(_) => false,
        }
    }

    /// The repository's default branch: `origin/HEAD` when set, else the first
    /// of `main`/`master`/`develop`/`trunk` that exists locally, else a remote
    /// `origin/main`/`origin/master`. `None` on a repo with no branches at all.
    /// Mirrors the fallback chain the bundled skill scripts already use.
    pub async fn default_branch(&self) -> Option<String> {
        if let Ok(out) = self.run(&["symbolic-ref", "refs/remotes/origin/HEAD"]).await {
            if let Some(b) = out.trim().strip_prefix("refs/remotes/origin/") {
                if !b.is_empty() {
                    return Some(b.to_string());
                }
            }
        }
        for cand in ["main", "master", "develop", "trunk"] {
            if self.branch_exists(cand).await {
                return Some(cand.to_string());
            }
        }
        for cand in ["origin/main", "origin/master"] {
            if self.verify_commit_ref(cand).await {
                return Some(cand.trim_start_matches("origin/").to_string());
            }
        }
        None
    }

    /// Resolve the base to diff/PR against: the wanted ref (as given, then
    /// `origin/<want>`), else the detected default branch (local, then remote).
    /// `diff_ref` is the verified rev to feed `git diff`; `branch` is the
    /// logical branch name a PR targets (no `origin/` prefix). Errors name
    /// every candidate tried — an actionable message instead of `git diff`
    /// exiting 128 on an unknown ref (the "fatal: ambiguous argument 'main'"
    /// failure this replaces).
    pub async fn resolve_base(&self, want: Option<&str>) -> Result<ResolvedBase> {
        let mut cands: Vec<(String, String)> = Vec::new(); // (diff_ref, branch)
        if let Some(w) = want.map(str::trim).filter(|s| !s.is_empty()) {
            let logical = w.trim_start_matches("origin/").to_string();
            cands.push((w.to_string(), logical.clone()));
            if !w.starts_with("origin/") {
                cands.push((format!("origin/{w}"), logical));
            }
        }
        if let Some(d) = self.default_branch().await {
            cands.push((d.clone(), d.clone()));
            cands.push((format!("origin/{d}"), d));
        }
        let mut tried: Vec<String> = Vec::new();
        for (diff_ref, branch) in cands {
            if tried.contains(&diff_ref) {
                continue;
            }
            if self.verify_commit_ref(&diff_ref).await {
                return Ok(ResolvedBase { diff_ref, branch });
            }
            tried.push(diff_ref);
        }
        Err(Error::Invalid(format!(
            "no base branch resolved (tried: {})",
            if tried.is_empty() {
                "nothing — repository has no branches".to_string()
            } else {
                tried.join(", ")
            }
        )))
    }

    /// Absolute path of the worktree that has `branch` checked out, if any —
    /// parsed from `git worktree list --porcelain` (the main checkout counts).
    pub async fn worktree_for_branch(&self, branch: &str) -> Option<String> {
        let out = self.run(&["worktree", "list", "--porcelain"]).await.ok()?;
        let want = format!("branch refs/heads/{branch}");
        let mut current: Option<&str> = None;
        for line in out.lines() {
            if let Some(p) = line.strip_prefix("worktree ") {
                current = Some(p.trim());
            } else if line.trim() == want {
                return current.map(str::to_string);
            }
        }
        None
    }

    /// Add a worktree at `path` checking out an EXISTING `branch` without
    /// resetting it (no `-B`, no base). Preserves the branch's commits — the
    /// safe path for resuming a loop whose worktree was removed but whose branch
    /// (and its work) must survive. `--force` tolerates a stale path registration.
    pub async fn worktree_attach(&self, path: &str, branch: &str) -> Result<()> {
        self.run(&["worktree", "add", "--force", path, branch]).await?;
        Ok(())
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
    /// Three cases, none of which ever discards committed work:
    /// 1. The worktree already exists → reuse it untouched (`Ok(false)`); the
    ///    agent resumes on top of its own prior commits, `base` ignored.
    /// 2. The worktree is absent but the `branch` already exists (e.g. its
    ///    worktree was pruned by idle cleanup or a restart, but `worktree_remove`
    ///    keeps the branch) → RE-ATTACH the surviving branch with
    ///    [`worktree_attach`] (no `-B`, `base` ignored), preserving every commit.
    /// 3. Neither exists → fresh [`worktree_add`], branching `branch` from `base`.
    ///
    /// Returns `true` when it (re)created the worktree directory, `false` when it
    /// reused an already-checked-out tree. Critically, this NEVER takes the
    /// destructive `-B` path against an existing branch — that reset-to-base is
    /// what used to throw away a swarm agent's work between turns.
    pub async fn worktree_add_if_absent(
        &self,
        path: &str,
        branch: &str,
        base: &str,
    ) -> Result<bool> {
        if self.worktree_exists(path).await {
            return Ok(false);
        }
        if self.branch_exists(branch).await {
            // The branch (and its commits) outlived its worktree. Re-attach it
            // instead of resetting it to `base`.
            self.worktree_attach(path, branch).await?;
        } else {
            self.worktree_add(path, branch, base).await?;
        }
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

    /// `git worktree list --porcelain` → parsed entries, each live worktree
    /// probed for uncommitted changes (best-effort; prunable entries are
    /// skipped — their directory is gone). The first entry is the main worktree.
    pub async fn worktree_list(&self) -> Result<Vec<WorktreeInfo>> {
        let out = self.run(&["worktree", "list", "--porcelain"]).await?;
        let mut wts = crate::parse::parse_worktree_list(&out);
        for wt in wts.iter_mut().filter(|w| !w.prunable) {
            wt.dirty = self.path_has_changes(&wt.path).await;
        }
        Ok(wts)
    }

    /// True when the git tree at `path` has uncommitted changes (staged,
    /// unstaged or untracked). Errors (missing dir, not a repo) read as clean —
    /// this feeds a UI hint, not a safety gate (`worktree remove` re-checks).
    async fn path_has_changes(&self, path: &str) -> bool {
        let out = Command::new("git")
            .arg("-C")
            .arg(path)
            .args(["status", "--porcelain", "--untracked-files=normal"])
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .output()
            .await;
        matches!(out, Ok(o) if o.status.success() && !o.stdout.is_empty())
    }

    /// User-facing worktree removal: surfaces git's error (unlike the reaper's
    /// best-effort [`worktree_remove`]) and only forces when asked — git refuses
    /// to remove a dirty/locked tree without `--force`, which is the safety net
    /// the UI relies on. Keeps the branch, like every other removal path.
    pub async fn worktree_remove_checked(&self, path: &str, force: bool) -> Result<()> {
        let mut args = vec!["worktree", "remove"];
        if force {
            // Twice: a locked worktree needs --force --force to be removed.
            args.push("--force");
            args.push("--force");
        }
        args.push("--");
        args.push(path);
        self.run(&args).await?;
        Ok(())
    }

    /// `git worktree prune` — drop stale registrations whose directory is gone.
    /// Returns git's verbose report ("" when there was nothing to prune).
    pub async fn worktree_prune(&self) -> Result<String> {
        let (out, err) = self.run_env(&["worktree", "prune", "--verbose"], &[]).await?;
        // --verbose reports on stderr in some git versions; prefer whichever spoke.
        let msg = if out.trim().is_empty() { err } else { out };
        Ok(msg.trim().to_string())
    }

    /// `git submodule status` → parsed entries enriched with `.gitmodules`
    /// url/branch. Empty list when the repo has no submodules.
    pub async fn submodule_list(&self) -> Result<Vec<SubmoduleInfo>> {
        let out = self.run(&["submodule", "status"]).await?;
        let mut subs = crate::parse::parse_submodule_status(&out);
        if subs.is_empty() {
            return Ok(subs);
        }
        // .gitmodules may be absent even with gitlinks recorded — best-effort.
        if let Ok((cfg, _)) = self
            .run_env(&["config", "-f", ".gitmodules", "--list"], &[])
            .await
        {
            crate::parse::enrich_submodules(&mut subs, &cfg);
        }
        Ok(subs)
    }

    /// `git submodule update --init --recursive [-- <path>]` — clone/checkout
    /// the recorded commit(s). Network-touching for uninitialized modules; uses
    /// the caller's ambient git auth (SSH agent / credential helper), like
    /// fetch/pull do for the origin remote.
    pub async fn submodule_update(&self, path: Option<&str>) -> Result<String> {
        let mut args = vec!["submodule", "update", "--init", "--recursive"];
        if let Some(p) = path {
            args.push("--");
            args.push(p);
        }
        let (out, err) = self.run_env(&args, &[]).await?;
        let msg = if out.trim().is_empty() { err } else { out };
        Ok(msg.trim().to_string())
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

    /// Compute a diff for `target`. When `pathspec` is `Some(path)`, every git
    /// invocation is scoped to that single file (`-- <path>`) — so selecting one
    /// file in the UI computes ONLY that file's diff instead of the entire
    /// working tree (which, for the `Working` target, also runs a `--no-index`
    /// diff per untracked file — seconds of work on a large changeset). `None`
    /// returns the full diff (the "All changes" view, commit/range views).
    pub async fn diff(&self, target: DiffTarget, pathspec: Option<&str>) -> Result<DiffResp> {
        // Trailing `-- <path>` appended to each command when a pathspec is given.
        let path_args: Vec<&str> = match pathspec {
            Some(p) if !p.is_empty() => vec!["--", p],
            _ => Vec::new(),
        };
        let with_path = |base: &[&str]| -> Vec<String> {
            // `core.quotePath=false` on every diff-family call: git's default
            // quotePath octal-escapes non-ASCII names (`"caf\303\251.txt"`),
            // which breaks feeding `ls-files` output back into `--no-index`
            // (file never found → silently missing from Changes) and litters
            // parsed headers with escapes. Raw UTF-8 round-trips cleanly.
            ["-c", "core.quotePath=false"]
                .iter()
                .chain(base.iter())
                .chain(path_args.iter())
                .map(|s| s.to_string())
                .collect()
        };
        let run_v = |args: Vec<String>| async move {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.run(&refs).await
        };
        let run_raw_v = |args: Vec<String>| async move {
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            self.run_raw(&refs, &[]).await
        };
        let out = match &target {
            DiffTarget::Worktree => run_v(with_path(&["diff", "--no-color", "-U3", "-M"])).await?,
            DiffTarget::Working => {
                // Staged + unstaged tracked changes vs HEAD (a staged-new file
                // shows as fully added). Falls back to cached+worktree when HEAD
                // is unborn (no commits yet).
                let (head_ok, head_out, _, _) =
                    run_raw_v(with_path(&["diff", "--no-color", "-U3", "-M", "HEAD"])).await?;
                let mut out = if head_ok {
                    head_out
                } else {
                    let mut s = run_v(with_path(&["diff", "--no-color", "-U3", "-M", "--cached"]))
                        .await
                        .unwrap_or_default();
                    s.push_str(
                        &run_v(with_path(&["diff", "--no-color", "-U3", "-M"]))
                            .await
                            .unwrap_or_default(),
                    );
                    s
                };
                // Untracked files: render each as a fully-added diff. Scope the
                // `ls-files` to the pathspec so a single-file request only checks
                // that one path (and runs at most one `--no-index` diff).
                let (_, untracked, _, _) =
                    run_raw_v(with_path(&["ls-files", "--others", "--exclude-standard"])).await?;
                for f in untracked.lines().filter(|l| !l.trim().is_empty()) {
                    let (_, stdout, _, _) = self
                        .run_raw(
                            &[
                                "-c", "core.quotePath=false", "diff", "--no-color", "-U3",
                                "--no-index", "--", "/dev/null", f,
                            ],
                            &[],
                        )
                        .await?;
                    out.push_str(&stdout);
                }
                out
            }
            DiffTarget::Staged => {
                run_v(with_path(&["diff", "--no-color", "-U3", "-M", "--cached"])).await?
            }
            DiffTarget::Commit(sha) => {
                // `-m --first-parent`: a merge commit's default `git show`
                // output is a combined (--cc) diff — files identical to any
                // parent are omitted (a normal integration merge shows "no
                // changes") and its `@@@` hunks don't parse. Diffing against
                // the first parent yields the reviewable "what this merge
                // brought in" diff; non-merge commits are unaffected.
                run_v(with_path(&[
                    "show", "-m", "--first-parent", "--no-color", "-U3", "-M", "--format=", sha,
                ]))
                .await?
            }
            DiffTarget::Range(a, b) => {
                let range = format!("{a}..{b}");
                run_v(with_path(&["diff", "--no-color", "-U3", "-M", &range])).await?
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
            // Creating a branch whose name already exists on origin is almost
            // never meant as "shadow it from my (possibly stale) HEAD" — a bare
            // `checkout -b` would do exactly that AND leave the branch without
            // an upstream, so the first `pull` dies with "no tracking
            // information". Start it at the remote tip and track it instead.
            let remote = format!("origin/{branch}");
            if self.verify_commit_ref(&format!("refs/remotes/{remote}")).await {
                self.run(&["checkout", "-b", branch, "--track", &remote])
                    .await?;
            } else {
                self.run(&["checkout", "-b", branch]).await?;
            }
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
                "renamed" => {
                    // Restore BOTH sides: the new name is absent at HEAD, so
                    // restoring it alone REMOVES the file (index + worktree)
                    // while the old name stays staged-deleted — i.e. "discard"
                    // would delete the user's file. Restoring old + new undoes
                    // the rename and brings the content back at the old path.
                    restore.push(c.path.clone());
                    if let Some(orig) = &c.orig_path {
                        restore.push(orig.clone());
                    }
                }
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
    ///
    /// Amend with an EMPTY message keeps the previous commit's message
    /// (`--amend --no-edit`) — the "fold staged changes into the last commit"
    /// flow; rejecting it forced users to retype the message.
    pub async fn commit(&self, message: &str, amend: bool) -> Result<String> {
        if message.trim().is_empty() {
            if !amend {
                return Err(Error::Invalid("empty commit message".into()));
            }
            self.run(&["commit", "--amend", "--no-edit"]).await?;
        } else {
            let mut args = vec!["commit", "-m", message];
            if amend {
                args.push("--amend");
            }
            self.run(&args).await?;
        }
        let sha = self.run(&["rev-parse", "HEAD"]).await?;
        Ok(sha.trim().to_string())
    }

    /// Safety net before opening a PR from a run/workflow worktree: stage and
    /// commit everything an agent left uncommitted (agents are TOLD to commit,
    /// but a stalled/stuck one leaves its work in the tree — the branch then
    /// has no commits ahead of base and the provider rejects the PR with
    /// "no changes to be pulled"). Otto's own runtime artifacts are excluded:
    /// `.mcp.json` is rendered into the cwd at session spawn and `.env*` /
    /// `.DS_Store` must never ride into a PR. Returns `Some(sha)` when a
    /// commit was made, `None` when there was nothing (real) to commit.
    /// Callers point this ONLY at dedicated run worktrees, never at a user's
    /// main checkout.
    pub async fn commit_all_if_dirty(&self, message: &str) -> Result<Option<String>> {
        self.run(&[
            "add",
            "-A",
            "--",
            ".",
            ":(exclude).mcp.json",
            ":(exclude).env",
            ":(exclude).env.*",
            ":(exclude,glob)**/.DS_Store",
        ])
        .await?;
        let staged = self.run(&["diff", "--cached", "--name-only"]).await?;
        if staged.trim().is_empty() {
            return Ok(None);
        }
        Ok(Some(self.commit(message, false).await?))
    }

    /// `git push`; for https remotes pass the account token so the askpass
    /// helper can answer credential prompts. Returns combined output.
    ///
    /// A branch that was never pushed has no upstream, so a plain `git push`
    /// fails ("has no upstream branch"). We detect that and retry with
    /// `--set-upstream origin <branch>`, so pushing (and creating a PR from) a
    /// fresh branch just works.
    ///
    /// `branch: Some(b)` pushes THAT branch explicitly (`git push origin b`)
    /// regardless of what's checked out — the Create-PR flow pushes the
    /// user-selected source branch, which previously silently pushed HEAD.
    pub async fn push_branch(&self, token: Option<String>, branch: Option<&str>) -> Result<String> {
        match branch {
            None => self.push(token).await,
            Some(b) => {
                let askpass = match &token {
                    Some(t) => Some(AskPass::new(t)?),
                    None => None,
                };
                let envs = askpass.as_ref().map(AskPass::envs).unwrap_or_default();
                let (ok, stdout, stderr, code) =
                    self.run_raw(&["push", "origin", b], &envs).await?;
                if ok {
                    return Ok(combine_push_output(&stdout, &stderr));
                }
                // First push of a fresh branch: set the upstream explicitly.
                if stderr.contains("has no upstream branch") || stderr.contains("--set-upstream") {
                    let (ok2, stdout2, stderr2, code2) = self
                        .run_raw(&["push", "--set-upstream", "origin", b], &envs)
                        .await?;
                    if ok2 {
                        return Ok(combine_push_output(&stdout2, &stderr2));
                    }
                    return Err(upstream_err(&stderr2, &stdout2, code2));
                }
                Err(upstream_err(&stderr, &stdout, code))
            }
        }
    }

    pub async fn push(&self, token: Option<String>) -> Result<String> {
        let askpass = match &token {
            Some(t) => Some(AskPass::new(t)?),
            None => None,
        };
        let envs = askpass.as_ref().map(AskPass::envs).unwrap_or_default();

        let (ok, stdout, stderr, code) = self.run_raw(&["push"], &envs).await?;
        if ok {
            return Ok(combine_push_output(&stdout, &stderr));
        }
        if stderr.contains("has no upstream branch") || stderr.contains("--set-upstream") {
            let branch = self.current_branch().await?;
            let (ok2, stdout2, stderr2, code2) = self
                .run_raw(&["push", "--set-upstream", "origin", &branch], &envs)
                .await?;
            if ok2 {
                return Ok(combine_push_output(&stdout2, &stderr2));
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
        // git writes progress/summary to stderr; surface both (minus benign
        // SSH noise like the post-quantum warning).
        let mut combined = strip_noise(&stdout);
        let err = strip_noise(&stderr);
        if !err.is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&err);
        }
        Ok(combined)
    }

    /// Like [`run_remote`] but DON'T error on a non-zero exit — return the raw
    /// outcome `(success, stdout, stderr, code)` so the caller can interpret an
    /// *expected* failure (e.g. deleting a remote ref that's already absent)
    /// instead of bubbling it up. Mirrors [`run_remote`]'s askpass setup.
    async fn run_remote_raw(
        &self,
        args: &[&str],
        token: Option<String>,
    ) -> Result<(bool, String, String, Option<i32>)> {
        let askpass = match token {
            Some(t) => Some(AskPass::new(&t)?),
            None => None,
        };
        let envs = askpass.as_ref().map(AskPass::envs).unwrap_or_default();
        self.run_raw(args, &envs).await
    }

    // -- graph context-menu ops (commit / branch / tag) ---------------------

    /// Cherry-pick a single commit onto the current branch. A conflicting pick
    /// exits non-zero → `Err` with git's stderr (the caller surfaces it; the
    /// graph UI does NOT auto-open the conflict resolver).
    pub async fn cherry_pick(&self, sha: &str) -> Result<()> {
        self.run(&["cherry-pick", sha]).await?;
        Ok(())
    }

    /// Revert a single commit, committing the inverse with `--no-edit`. A
    /// conflicting revert exits non-zero → `Err` with git's stderr.
    pub async fn revert(&self, sha: &str) -> Result<()> {
        self.run(&["revert", "--no-edit", sha]).await?;
        Ok(())
    }

    /// Create a branch `name`, optionally based at `start_point` (a commit/branch
    /// /tag; HEAD when None). `checkout=true` switches to it (`checkout -b`),
    /// otherwise it's created in place (`git branch`).
    pub async fn create_branch(
        &self,
        name: &str,
        start_point: Option<&str>,
        checkout: bool,
    ) -> Result<()> {
        let sp = start_point.filter(|s| !s.is_empty());
        // No explicit start point + the name exists on origin ⇒ the caller
        // means THAT branch: base it at the remote tip with tracking, not at a
        // possibly-stale HEAD with no upstream (see `checkout`).
        let remote = format!("origin/{name}");
        if sp.is_none()
            && self.verify_commit_ref(&format!("refs/remotes/{remote}")).await
        {
            let args: &[&str] = if checkout {
                &["checkout", "-b", name, "--track", &remote]
            } else {
                &["branch", "--track", name, &remote]
            };
            self.run(args).await?;
            return Ok(());
        }
        let mut args: Vec<&str> = if checkout {
            vec!["checkout", "-b", name]
        } else {
            vec!["branch", name]
        };
        if let Some(sp) = sp {
            args.push(sp);
        }
        self.run(&args).await?;
        Ok(())
    }

    /// Delete a local branch. `force=true` → `-D` (drops unmerged work), else
    /// `-d` (refuses to delete an unmerged branch).
    pub async fn delete_branch(&self, name: &str, force: bool) -> Result<()> {
        self.run(&["branch", if force { "-D" } else { "-d" }, name])
            .await?;
        Ok(())
    }

    /// Delete branch `name` on `origin` (`git push origin --delete <name>`).
    /// Returns the combined push output.
    ///
    /// Idempotent: a stale local remote-tracking ref (`refs/remotes/origin/
    /// <name>`) can outlive the real branch when it was deleted elsewhere
    /// without a local `fetch --prune`. The UI trusts that tracking ref and
    /// offers a remote delete, which git then rejects with "remote ref does not
    /// exist". That isn't a real failure — the desired end state (no such branch
    /// on origin) already holds — so we swallow it and fall through to prune the
    /// stale ref, which is what actually clears the phantom from the UI. Without
    /// this the request errored *before* the prune ran, so the bad menu entry
    /// persisted and every retry failed.
    pub async fn delete_remote_branch(&self, name: &str, token: Option<String>) -> Result<String> {
        let (ok, stdout, stderr, code) = self
            .run_remote_raw(&["push", "origin", "--delete", name], token)
            .await?;
        let already_gone = !ok && remote_ref_absent(&stderr);
        if !ok && !already_gone {
            return Err(upstream_err(&stderr, &stdout, code));
        }
        // `git push --delete` doesn't reliably prune the LOCAL remote-tracking ref
        // (`refs/remotes/origin/<name>`), so the branch lingers in the UI's REMOTE
        // list until the next `fetch --prune`. Remove it explicitly so the deletion
        // shows up immediately. Best-effort: if push already pruned it (or it never
        // existed), the delete is a no-op error we ignore.
        let _ = self
            .run(&["update-ref", "-d", &format!("refs/remotes/origin/{name}")])
            .await;
        if already_gone {
            return Ok(format!(
                "origin/{name} was already absent on origin; pruned the stale local tracking ref"
            ));
        }
        // Happy path: surface git's own summary (stdout + stderr, minus noise),
        // mirroring `run_remote`.
        let mut combined = strip_noise(&stdout);
        let err = strip_noise(&stderr);
        if !err.is_empty() {
            if !combined.is_empty() {
                combined.push('\n');
            }
            combined.push_str(&err);
        }
        Ok(combined)
    }

    /// Rename local branch `from` → `to` (`git branch -m`).
    pub async fn rename_branch(&self, from: &str, to: &str) -> Result<()> {
        self.run(&["branch", "-m", from, to]).await?;
        Ok(())
    }

    /// Create a tag at `sha`: annotated (`-a … -m <msg>`) when `message` is
    /// present, lightweight otherwise.
    pub async fn create_tag(&self, name: &str, sha: &str, message: Option<&str>) -> Result<()> {
        match message.filter(|m| !m.is_empty()) {
            Some(msg) => {
                self.run(&["tag", "-a", name, "-m", msg, sha]).await?;
            }
            None => {
                self.run(&["tag", name, sha]).await?;
            }
        }
        Ok(())
    }

    /// Push a single tag to `origin` (`git push origin refs/tags/<name>`).
    /// Returns the combined push output.
    pub async fn push_tag(&self, name: &str, token: Option<String>) -> Result<String> {
        let refspec = format!("refs/tags/{name}");
        self.run_remote(&["push", "origin", &refspec], token).await
    }

    /// Delete a local tag (`git tag -d <name>`).
    pub async fn delete_tag(&self, name: &str) -> Result<()> {
        self.run(&["tag", "-d", name]).await?;
        Ok(())
    }

    /// Delete a tag on `origin` (`git push origin --delete refs/tags/<name>`).
    /// Returns the combined push output.
    pub async fn delete_remote_tag(&self, name: &str, token: Option<String>) -> Result<String> {
        let refspec = format!("refs/tags/{name}");
        self.run_remote(&["push", "origin", "--delete", &refspec], token)
            .await
    }

    pub async fn stash_save(&self) -> Result<String> {
        let (out, _) = self.run_env(&["stash", "push"], &[]).await?;
        Ok(out.trim().to_string())
    }

    pub async fn stash_pop(&self) -> Result<String> {
        let (ok, out, err, code) = self.run_raw(&["stash", "pop"], &[]).await?;
        // A conflicting pop exits non-zero but HAS applied the stash (conflict
        // markers written, paths left unmerged) — a normal result the user
        // resolves, not a failure. Surface it as Ok so the caller refreshes into
        // the conflict flow rather than toasting a bogus error over stale state.
        if ok || out.contains("CONFLICT") {
            return Ok(out.trim().to_string());
        }
        Err(upstream_err(&err, &out, code))
    }

    /// `git stash list` → parsed entries (read-only). Empty list when there are
    /// no stashes (`git` exits 0 with empty output).
    pub async fn stash_list(&self) -> Result<Vec<StashInfo>> {
        let out = self
            .run(&[
                "stash",
                "list",
                "--pretty=format:%gd%x1f%H%x1f%P%x1f%aI%x1f%gs",
            ])
            .await?;
        Ok(crate::parse::parse_stash_list(&out))
    }

    /// Resolve the live `stash@{N}` selector for a stash commit SHA, reading the
    /// stash list at execution time. SHA-anchored (not the client's possibly
    /// stale positional index) so a concurrent drop/push that renumbers the
    /// stack can't make us apply/drop the WRONG stash — important since `drop`
    /// is irreversible. Errors if the stash is gone.
    async fn resolve_stash_selector(&self, sha: &str) -> Result<String> {
        self.stash_list()
            .await?
            .into_iter()
            .find(|s| s.sha == sha)
            .map(|s| format!("stash@{{{}}}", s.index))
            .ok_or_else(|| Error::Invalid(format!("stash {sha} no longer exists")))
    }

    /// Apply the stash with commit `sha` onto the working tree, keeping it in the
    /// list. A resulting merge conflict is a normal outcome (see `stash_pop`).
    pub async fn stash_apply(&self, sha: &str) -> Result<String> {
        let sel = self.resolve_stash_selector(sha).await?;
        let (ok, out, err, code) = self.run_raw(&["stash", "apply", &sel], &[]).await?;
        if ok || out.contains("CONFLICT") {
            return Ok(out.trim().to_string());
        }
        Err(upstream_err(&err, &out, code))
    }

    /// Drop (discard) the stash with commit `sha` without applying it.
    pub async fn stash_drop(&self, sha: &str) -> Result<String> {
        let sel = self.resolve_stash_selector(sha).await?;
        let (out, _) = self.run_env(&["stash", "drop", &sel], &[]).await?;
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

    /// True if the working tree has staged/unstaged TRACKED changes (untracked
    /// files don't block a merge and aren't stashed by a plain `git stash`).
    async fn working_dirty(&self) -> Result<bool> {
        let st = self.status().await?;
        Ok(st
            .changes
            .iter()
            .any(|c| (c.staged || c.unstaged) && c.kind != "untracked"))
    }

    /// Pop the stash after a clean merge. Returns a human note: a confirmation on
    /// a clean pop, or a warning if the pop conflicted (git KEEPS the stash in
    /// that case, so the user's work is never lost).
    async fn pop_after_merge(&self) -> Option<String> {
        match self.stash_pop().await {
            Ok(_) => Some("Your stashed changes were restored.".into()),
            Err(_) => Some(
                "Merge succeeded, but restoring your stashed changes hit a conflict — \
                 they're preserved in `git stash`; resolve the working tree and run \
                 `git stash pop` manually."
                    .into(),
            ),
        }
    }

    /// Dry-run a merge of `source` into `target` via `git merge-tree --write-tree`
    /// (writes only to the object DB — the index and working tree are NEVER
    /// touched). Lets callers warn about conflicts BEFORE starting a real merge.
    pub async fn merge_preview(&self, source: &str, target: &str) -> Result<MergePreview> {
        // No-op merge: source already contained in target.
        if self.is_ancestor_of(source, target).await.unwrap_or(false) {
            return Ok(MergePreview {
                conflicts: false,
                conflicted_files: Vec::new(),
                up_to_date: true,
            });
        }
        let (ok, stdout, _stderr, code) = self
            .run_raw(
                &["merge-tree", "--write-tree", "--name-only", target, source],
                &[],
            )
            .await?;
        if ok {
            return Ok(MergePreview {
                conflicts: false,
                conflicted_files: Vec::new(),
                up_to_date: false,
            });
        }
        // `merge-tree` exits exactly 1 for "conflicts". Any other non-zero code is
        // a usage/ref error (e.g. an older git) — don't block; let the real merge
        // surface it.
        if code != Some(1) {
            return Ok(MergePreview {
                conflicts: false,
                conflicted_files: Vec::new(),
                up_to_date: false,
            });
        }
        // Output: tree OID on line 1, then conflicted file names (--name-only).
        let conflicted_files: Vec<String> = stdout
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|l| l.to_string())
            .collect();
        Ok(MergePreview {
            conflicts: true,
            conflicted_files,
            up_to_date: false,
        })
    }

    /// Merge `source` into `target`. Never auto-resolves; conflicts are returned
    /// as `Ok(MergeResult{status:"conflicts", ..})`, not an error.
    ///
    /// When `auto_stash` is set and the working tree is dirty, the changes are
    /// stashed before the merge and popped afterwards (stash → merge → pop).
    pub async fn merge_branch(
        &self,
        source: &str,
        target: &str,
        strategy: LocalMergeStrategy,
        auto_stash: bool,
    ) -> Result<MergeResult> {
        let already_merging = self.is_merging().await;

        // Dirty-tree handling (continuing an in-progress merge is exempt — its
        // working-tree conflicts are expected). Either auto-stash, or refuse.
        let mut stashed = false;
        if !already_merging && self.working_dirty().await? {
            if auto_stash {
                self.stash_save().await?;
                stashed = true;
            } else {
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
            let up_to_date =
                combined.contains("Already up to date") || combined.contains("Already up-to-date");
            // The MergeCommit strategy promises "always create a merge commit".
            // git's `merge --no-ff` still refuses when <source> is already an
            // ancestor of <target> ("Already up to date") — but the user may want
            // to RECORD the integration anyway (e.g. closing a GitFlow release
            // into develop after develop already contains it). Build an explicit
            // 2-parent merge commit by hand and fast-forward onto it: its tree is
            // target's current tree, so the working tree is left untouched.
            if up_to_date && matches!(strategy, LocalMergeStrategy::MergeCommit) {
                let target_head = self.run(&["rev-parse", "HEAD"]).await?.trim().to_string();
                let source_head = self.run(&["rev-parse", source]).await?.trim().to_string();
                let tree = self.run(&["rev-parse", "HEAD^{tree}"]).await?.trim().to_string();
                let msg = format!("Merge branch '{source}' into {target}");
                let new_commit = self
                    .run(&[
                        "commit-tree", &tree, "-p", &target_head, "-p", &source_head, "-m", &msg,
                    ])
                    .await?
                    .trim()
                    .to_string();
                // Advance the checked-out target branch onto the new merge commit
                // (it descends from target_head, so this is a clean fast-forward).
                self.run(&["merge", "--ff-only", &new_commit]).await?;
                let note = if stashed { self.pop_after_merge().await } else { None };
                return Ok(MergeResult {
                    status: "merged".into(),
                    commit: Some(new_commit),
                    conflicted_files: Vec::new(),
                    repo_status: self.status().await?,
                    note,
                });
            }
            // `--squash` leaves changes staged but creates NO commit; the caller
            // must still run merge/commit, so report commit = None.
            let commit = if up_to_date || matches!(strategy, LocalMergeStrategy::Squash) {
                None
            } else {
                Some(self.run(&["rev-parse", "HEAD"]).await?.trim().to_string())
            };
            // Merge landed cleanly — restore any auto-stashed work.
            let note = if stashed {
                self.pop_after_merge().await
            } else {
                None
            };
            return Ok(MergeResult {
                status: if up_to_date { "up_to_date" } else { "merged" }.into(),
                commit,
                conflicted_files: Vec::new(),
                repo_status: self.status().await?,
                note,
            });
        }

        // Non-zero exit. Conflict markers / unmerged paths → a normal "conflicts"
        // result; anything else (ff-only impossible, bad ref, fatal) is an error.
        let conflicted = self.conflicted_paths().await?;
        let is_conflict = combined.contains("CONFLICT")
            || combined.contains("Automatic merge failed")
            || !conflicted.is_empty();
        if is_conflict {
            // We auto-stashed and the merge conflicted: do NOT pop onto a
            // conflicted tree. Leave the stash saved and tell the user.
            let note = if stashed {
                Some(
                    "Your uncommitted changes were stashed before the merge, which then \
                     conflicted. Resolve the conflicts and commit, then run `git stash pop` \
                     to restore your changes."
                        .into(),
                )
            } else {
                None
            };
            return Ok(MergeResult {
                status: "conflicts".into(),
                commit: None,
                conflicted_files: conflicted,
                repo_status: self.status().await?,
                note,
            });
        }
        // Hard error — if we stashed, restore the user's work before surfacing it
        // so nothing is stranded.
        if stashed {
            let _ = self.stash_pop().await;
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
            note: None,
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

/// stderr lines that SSH/git emit as benign chatter — never the reason a command
/// failed. We skip these when choosing the message to surface so the real git
/// error (rejected push, auth failure, …) isn't masked. Newer OpenSSH (9.x/10.x)
/// prints the post-quantum warning to stderr on every non-PQ connection and it
/// does NOT affect the exit status — yet it sorts first, so the old "first
/// non-empty line" logic reported it as the failure.
/// Remove any `user:password@` userinfo from a URL so a credentialed remote a
/// user may have pasted isn't echoed into notices/logs. Best-effort string op;
/// returns non-URL strings unchanged. The real (credentialed) URL is still used
/// for the actual git operation — this is only for display.
pub fn strip_url_userinfo(url: &str) -> String {
    let Some(scheme_end) = url.find("://") else {
        return url.to_string();
    };
    let after = scheme_end + 3;
    let rest = &url[after..];
    let path_start = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..path_start];
    match authority.rfind('@') {
        Some(at) => format!("{}{}{}", &url[..after], &authority[at + 1..], &rest[path_start..]),
        None => url.to_string(),
    }
}

fn is_noise_line(l: &str) -> bool {
    let t = l.trim();
    t.is_empty()
        || t.contains("post-quantum key exchange")
        || t.starts_with("Warning: Permanently added")
}

/// Drop benign SSH/git noise lines from combined command output (used for the
/// success path so a successful push/pull doesn't surface the post-quantum
/// warning).
fn strip_noise(s: &str) -> String {
    s.lines()
        .filter(|l| !is_noise_line(l))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}

/// Combine a push's stdout+stderr into one denoised block (git writes its
/// human summary to stderr). Shared by `push` and `push_branch`.
fn combine_push_output(stdout: &str, stderr: &str) -> String {
    let mut c = strip_noise(stdout);
    let err = strip_noise(stderr);
    if !err.is_empty() {
        if !c.is_empty() {
            c.push('\n');
        }
        c.push_str(&err);
    }
    c
}

/// True when `git push origin --delete <ref>` failed only because the ref is
/// already gone on origin. Git's wording is stable: "remote ref does not
/// exist". Lets [`LocalGit::delete_remote_branch`] treat that as a no-op
/// success (the branch is absent either way) rather than a hard error.
fn remote_ref_absent(stderr: &str) -> bool {
    stderr.contains("remote ref does not exist")
}

fn upstream_err(stderr: &str, stdout: &str, code: Option<i32>) -> Error {
    // Among the meaningful (non-noise) lines, prefer one that actually names the
    // failure — git scatters the real reason ("! [remote rejected] …", "error:
    // failed to push …") after benign chatter like "To <url>".
    let meaningful: Vec<&str> = stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .filter(|&l| !is_noise_line(l))
        .collect();
    let pick = meaningful
        .iter()
        .copied()
        .find(|l| {
            let lc = l.to_ascii_lowercase();
            lc.contains("rejected")
                || lc.starts_with("error:")
                || lc.starts_with("fatal:")
                || lc.starts_with("remote:")
        })
        .or_else(|| meaningful.first().copied())
        .unwrap_or("git failed with no output");
    Error::Upstream(format!(
        "git exited {}: {}",
        code.map_or_else(|| "?".to_string(), |c| c.to_string()),
        pick
    ))
}

// ---------------------------------------------------------------------------
// Askpass helper for https remotes
// ---------------------------------------------------------------------------

/// Temp executable script handed to git via GIT_ASKPASS. Echoes a placeholder
/// username for "Username" prompts and the token (provided via env var
/// OTTO_GIT_TOKEN, never written to disk) for everything else. Works for
/// GitHub (any username + PAT), Bitbucket (see [`AskPass::envs`] — API tokens
/// need the magic `x-bitbucket-api-token-auth` username; access tokens use
/// `x-token-auth`) and GitLab (any username + PAT).
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
        let mut envs = vec![
            (
                "GIT_ASKPASS".to_string(),
                self.path.to_string_lossy().into_owned(),
            ),
            ("OTTO_GIT_TOKEN".to_string(), self.token.clone()),
            // System credential helpers (osxkeychain) run BEFORE askpass, and
            // a stale credential they serve (e.g. a dying Bitbucket app
            // password) draws a 410 — a HARD error git never retries with the
            // next credential source, so the account token below is never
            // consulted. When Otto supplies the credential it must be
            // authoritative: reset the helper list via config-env (the
            // equivalent of `git -c credential.helper=`).
            ("GIT_CONFIG_COUNT".to_string(), "1".to_string()),
            ("GIT_CONFIG_KEY_0".to_string(), "credential.helper".to_string()),
            ("GIT_CONFIG_VALUE_0".to_string(), String::new()),
        ];
        // Atlassian API tokens (the app-password replacement, prefix ATATT)
        // authenticate git-over-HTTPS only under this exact magic username —
        // `x-token-auth` (the default) gets 410 Gone from bitbucket.org. The
        // prefix is Atlassian-specific, so this never misfires for GitHub
        // (ghp_/github_pat_) or GitLab (glpat-) tokens.
        if self.token.starts_with("ATATT") {
            envs.push((
                "OTTO_GIT_USERNAME".to_string(),
                "x-bitbucket-api-token-auth".to_string(),
            ));
        }
        envs
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
        let staged = git.diff(DiffTarget::Staged, None).await.unwrap();
        let dren = staged.files.iter().find(|f| f.path == "d.txt").unwrap();
        assert_eq!(dren.old_path.as_deref(), Some("c.txt"));
        assert!(staged.files.iter().any(|f| f.path == "f.txt"));

        // worktree diff: a.txt with one added line numbered 4 (untracked excluded)
        let wt = git.diff(DiffTarget::Worktree, None).await.unwrap();
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
        let cd = git.diff(DiffTarget::Commit(head.clone()), None).await.unwrap();
        assert_eq!(cd.files.len(), 1);
        assert_eq!(cd.files[0].path, "a.txt");

        // range diff
        let first = git.log(1, 1, false).await.unwrap()[0].sha.clone();
        let rd = git
            .diff(DiffTarget::Range(first.clone(), head.clone()), None)
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
    async fn commit_all_if_dirty_sweeps_leftovers_but_not_artifacts() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        // The fixture starts dirty (staged rename/new, unstaged, untracked) —
        // the sweep commits all of it.
        assert!(git.commit_all_if_dirty("sweep fixture").await.unwrap().is_some());
        // Now-clean tree → no commit.
        assert!(git.commit_all_if_dirty("noop").await.unwrap().is_none());
        // Leftover agent work + runtime artifacts that must stay out.
        write(&dir, "work.txt", "agent forgot to commit me\n");
        write(&dir, ".mcp.json", "{}\n");
        write(&dir, ".env", "SECRET=1\n");
        let sha = git.commit_all_if_dirty("sweep").await.unwrap();
        assert!(sha.is_some());
        let st = git.status().await.unwrap();
        assert!(!st.changes.iter().any(|c| c.path == "work.txt"));
        assert!(st.changes.iter().any(|c| c.path == ".mcp.json"));
        assert!(st.changes.iter().any(|c| c.path == ".env"));
        // Nothing real left → None again.
        assert!(git.commit_all_if_dirty("noop").await.unwrap().is_none());
    }

    #[test]
    fn askpass_username_follows_token_kind() {
        // Atlassian API tokens must authenticate under the magic username;
        // everything else keeps the x-token-auth script default (no env).
        let api = AskPass::new("ATATT3xFfGF0abc").unwrap();
        assert!(api.envs().iter().any(|(k, v)| {
            k == "OTTO_GIT_USERNAME" && v == "x-bitbucket-api-token-auth"
        }));
        let pat = AskPass::new("ghp_abc123").unwrap();
        assert!(!pat.envs().iter().any(|(k, _)| k == "OTTO_GIT_USERNAME"));
        // Otto's credential must be authoritative: helpers reset for every
        // token kind (a stale osxkeychain app password otherwise wins and
        // draws a hard 410 before askpass is ever consulted).
        for a in [&api, &pat] {
            assert!(a
                .envs()
                .iter()
                .any(|(k, v)| k == "GIT_CONFIG_KEY_0" && v == "credential.helper"));
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

    /// D2 regression: if the worktree was REMOVED between turns (the branch is
    /// kept by `worktree_remove`), a later `worktree_add_if_absent` must
    /// RE-ATTACH the surviving branch — never reset it to `base` with `-B`. The
    /// old destructive `-B` path discarded the agent's committed work whenever
    /// its worktree had been pruned (idle cleanup / restart), which is exactly
    /// the "destructive `-B --force` branch reuse" hazard.
    #[tokio::test]
    async fn worktree_add_if_absent_reattaches_branch_after_worktree_removed() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        let wt = dir.parent().unwrap().join("reattach-wt");
        let wt_str = wt.to_str().unwrap().to_string();
        let branch = "swarm/s2/a2";

        // base = the FIRST commit (≠ HEAD/second commit) so a reset-to-base is
        // observable as a different sha than the agent's own commit.
        let base = git.log(1, 1, false).await.unwrap()[0].sha.clone();

        // Turn 1: create on the branch from HEAD, then commit agent work.
        git.worktree_add_if_absent(&wt_str, branch, "HEAD").await.unwrap();
        let wt_git = LocalGit::new(&wt);
        write(&wt, "agent_work.txt", "turn 1 output\n");
        wt_git.stage(&["agent_work.txt".into()]).await.unwrap();
        let sha = wt_git.commit("agent turn 1", false).await.unwrap();

        // The worktree is pruned, but the branch + its commit must live on.
        git.worktree_remove(&wt_str).await.unwrap();
        assert!(!git.worktree_exists(&wt_str).await);
        assert!(git.branch_exists(branch).await, "branch survives worktree removal");

        // Turn 2: re-provision with a DIFFERENT base. It must RE-ATTACH the
        // existing branch (preserving the agent commit), not reset to base.
        let created = git.worktree_add_if_absent(&wt_str, branch, &base).await.unwrap();
        assert!(created, "re-provisioning a pruned worktree counts as created");
        let head = wt_git.log(1, 0, false).await.unwrap();
        assert_eq!(
            head[0].sha, sha,
            "branch must NOT be reset to base; agent commit preserved"
        );
        assert_eq!(head[0].subject, "agent turn 1");
        assert!(
            wt.join("agent_work.txt").exists(),
            "committed file restored on re-attach"
        );
    }

    /// `changed_files` lists what a worktree branch changed vs its base — the
    /// signal the swarm uses to detect two agents touching the same shared files.
    #[tokio::test]
    async fn changed_files_lists_branch_changes() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        let wt = dir.parent().unwrap().join("cf-wt");
        let wt_str = wt.to_str().unwrap().to_string();
        git.worktree_add_if_absent(&wt_str, "swarm/s1/a1", "main").await.unwrap();

        let wt_git = LocalGit::new(&wt);
        // No commits on the branch yet → no changes vs base.
        assert!(wt_git.changed_files("main").await.unwrap().is_empty());

        // Commit a new file + modify an existing one.
        write(&wt, "shared.txt", "agent A\n");
        write(&wt, "a.txt", "alpha line 1\nalpha line 2\nalpha A\n");
        wt_git.stage(&["shared.txt".into(), "a.txt".into()]).await.unwrap();
        wt_git.commit("agent A work", false).await.unwrap();

        let mut files = wt_git.changed_files("main").await.unwrap();
        files.sort();
        assert_eq!(files, vec!["a.txt".to_string(), "shared.txt".to_string()]);
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

    /// Local-only graph context-menu ops: branch create/rename/delete, tag
    /// create (lightweight + annotated) / delete, cherry-pick and revert. Remote
    /// ops (`delete_remote_*`, `push_tag`) need a remote and are exercised via
    /// the live verification, not here.
    #[tokio::test]
    async fn graph_context_ops_local() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);

        // Commit the pending fixture changes so HEAD is clean for picks/reverts.
        sh_git(&dir, &["add", "-A"]);
        sh_git(&dir, &["commit", "-m", "baseline"]);
        let head = git.log(1, 0, false).await.unwrap()[0].sha.clone();

        // create_branch in place (no checkout) from a start_point.
        git.create_branch("feat/a", Some(&head), false).await.unwrap();
        assert_eq!(git.current_branch().await.unwrap(), "main");
        assert!(git.refs().await.unwrap().local.iter().any(|b| b.name == "feat/a"));

        // create_branch + checkout from HEAD.
        git.create_branch("feat/b", None, true).await.unwrap();
        assert_eq!(git.current_branch().await.unwrap(), "feat/b");

        // rename it, then go back to main.
        git.rename_branch("feat/b", "feat/b2").await.unwrap();
        assert_eq!(git.current_branch().await.unwrap(), "feat/b2");
        git.checkout("main", false).await.unwrap();

        // delete branches (force for the unmerged renamed one).
        git.delete_branch("feat/a", false).await.unwrap();
        git.delete_branch("feat/b2", true).await.unwrap();
        let locals = git.refs().await.unwrap().local;
        assert!(!locals.iter().any(|b| b.name == "feat/a" || b.name == "feat/b2"));

        // lightweight + annotated tags, then list + delete.
        git.create_tag("v1", &head, None).await.unwrap();
        git.create_tag("v2", &head, Some("release two")).await.unwrap();
        let tags = git.refs().await.unwrap().tags;
        assert!(tags.iter().any(|t| t.name == "v1"));
        assert!(tags.iter().any(|t| t.name == "v2"));
        git.delete_tag("v1").await.unwrap();
        assert!(!git.refs().await.unwrap().tags.iter().any(|t| t.name == "v1"));

        // cherry-pick: make a commit on a side branch, pick it onto main.
        git.create_branch("side", Some(&head), true).await.unwrap();
        write(&dir, "picked.txt", "from side\n");
        git.stage(&["picked.txt".into()]).await.unwrap();
        let side_sha = git.commit("side change", false).await.unwrap();
        git.checkout("main", false).await.unwrap();
        git.cherry_pick(&side_sha).await.unwrap();
        assert!(dir.join("picked.txt").exists());
        assert_eq!(git.log(1, 0, false).await.unwrap()[0].subject, "side change");

        // revert the cherry-picked commit → file removed again.
        let picked = git.log(1, 0, false).await.unwrap()[0].sha.clone();
        git.revert(&picked).await.unwrap();
        assert!(!dir.join("picked.txt").exists());

        // a bad cherry-pick ref surfaces as an Upstream error.
        assert!(git.cherry_pick("deadbeefdeadbeef").await.is_err());
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

    /// MergeCommit must "always create a merge commit" — even when <source> is an
    /// ancestor of <target> (the "close the already-merged release into develop"
    /// case), where plain `merge --no-ff` would just say "Already up to date".
    #[tokio::test]
    async fn merge_commit_forces_a_commit_when_up_to_date() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        // Commit the fixture's dirty state so the working tree is clean to merge.
        sh_git(&dir, &["add", "-A"]);
        sh_git(&dir, &["commit", "-m", "tidy"]);
        let main_head = git.run(&["rev-parse", "HEAD"]).await.unwrap().trim().to_string();
        // `rel` points at an ANCESTOR of main's tip → already contained, so a
        // plain merge would be "Already up to date" with no commit.
        let ancestor = git.run(&["rev-parse", "HEAD~1"]).await.unwrap().trim().to_string();
        sh_git(&dir, &["branch", "rel", &ancestor]);

        let res = git
            .merge_branch("rel", "main", LocalMergeStrategy::MergeCommit, false)
            .await
            .unwrap();

        assert_eq!(res.status, "merged", "forced a merge even though up to date");
        let new_head = res.commit.expect("a merge commit sha");
        assert_ne!(new_head, main_head, "main advanced onto the new merge commit");
        // …a real 2-parent merge (target tip + the source).
        let parents = git
            .run(&["rev-list", "--parents", "-n", "1", "HEAD"])
            .await
            .unwrap();
        assert_eq!(
            parents.split_whitespace().count() - 1,
            2,
            "forced merge commit has two parents"
        );
        // No working-tree churn — the merge tree equals main's tree.
        assert!(
            git.status().await.unwrap().changes.is_empty(),
            "working tree stays clean"
        );
    }

    /// Deleting a branch on the remote must also drop the LOCAL remote-tracking
    /// ref, so the UI's REMOTE list reflects it immediately (no pull needed).
    #[tokio::test]
    async fn delete_remote_branch_prunes_local_tracking_ref() {
        let (_tmp, dir) = fixture();
        sh_git(&dir, &["add", "-A"]);
        sh_git(&dir, &["commit", "-m", "tidy"]);
        // A bare repo plays the role of `origin`.
        let parent = dir.parent().unwrap();
        sh_git(parent, &["init", "--bare", "origin.git"]);
        let bare = parent.join("origin.git");
        sh_git(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
        sh_git(&dir, &["push", "origin", "main"]);
        sh_git(&dir, &["branch", "tmp"]);
        sh_git(&dir, &["push", "origin", "tmp"]);
        sh_git(&dir, &["fetch", "origin"]);

        let git = LocalGit::new(&dir);
        let before = git.refs().await.unwrap().remote;
        assert!(
            before.iter().any(|b| b.name == "origin/tmp"),
            "origin/tmp present before delete"
        );

        git.delete_remote_branch("tmp", None).await.unwrap();

        // Pruned locally → gone from /refs WITHOUT a fetch.
        let after = git.refs().await.unwrap().remote;
        assert!(
            !after.iter().any(|b| b.name == "origin/tmp"),
            "origin/tmp pruned from local tracking refs after delete"
        );
    }

    /// Creating a branch whose name exists on origin must start AT the remote
    /// tip and track it — a bare `checkout -b` from a stale HEAD makes an
    /// upstream-less branch whose first `pull` dies with "no tracking
    /// information" (the koala-bigdaddy `develop` incident).
    #[tokio::test]
    async fn create_named_like_remote_branch_tracks_remote_tip() {
        let (_tmp, dir) = fixture();
        sh_git(&dir, &["add", "-A"]);
        sh_git(&dir, &["commit", "-m", "tidy"]);
        let parent = dir.parent().unwrap();
        sh_git(parent, &["init", "--bare", "origin.git"]);
        let bare = parent.join("origin.git");
        sh_git(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
        sh_git(&dir, &["push", "origin", "main"]);
        // `dev` exists on origin one commit AHEAD of local main (the "stale
        // HEAD" scenario), and only as a remote-tracking ref locally.
        sh_git(&dir, &["checkout", "-b", "dev"]);
        write(&dir, "remote-only.txt", "ahead\n");
        sh_git(&dir, &["add", "remote-only.txt"]);
        sh_git(&dir, &["commit", "-m", "remote ahead"]);
        sh_git(&dir, &["push", "origin", "dev"]);
        sh_git(&dir, &["checkout", "main"]);
        sh_git(&dir, &["branch", "-D", "dev"]);
        sh_git(&dir, &["fetch", "origin"]);

        let git = LocalGit::new(&dir);
        git.checkout("dev", true).await.unwrap();
        assert_eq!(git.current_branch().await.unwrap(), "dev");
        let head = git.run(&["rev-parse", "HEAD"]).await.unwrap();
        let remote_tip = git.run(&["rev-parse", "origin/dev"]).await.unwrap();
        assert_eq!(head, remote_tip, "branch starts at the remote tip, not stale HEAD");
        let upstream = git
            .run(&["rev-parse", "--abbrev-ref", "dev@{upstream}"])
            .await
            .unwrap();
        assert_eq!(upstream.trim(), "origin/dev", "upstream configured");

        // Same guarantee for the create-in-place path.
        sh_git(&dir, &["checkout", "main"]);
        sh_git(&dir, &["branch", "-D", "dev"]);
        git.create_branch("dev", None, false).await.unwrap();
        let upstream = git
            .run(&["rev-parse", "--abbrev-ref", "dev@{upstream}"])
            .await
            .unwrap();
        assert_eq!(upstream.trim(), "origin/dev", "create_branch tracks too");

        // A genuinely new name still branches from HEAD (no upstream to wire).
        git.checkout("feature/fresh", true).await.unwrap();
        assert_eq!(git.current_branch().await.unwrap(), "feature/fresh");
        // An explicit start_point is respected verbatim (no remote override).
        let base = git.run(&["rev-parse", "main~1"]).await.unwrap();
        git.create_branch("dev2", Some(base.trim()), false).await.unwrap();
        let dev2 = git.run(&["rev-parse", "dev2"]).await.unwrap();
        assert_eq!(dev2, base, "explicit start point wins");
    }

    /// A stale local remote-tracking ref — the branch was deleted on origin
    /// elsewhere, without a local `fetch --prune` — must not make a remote
    /// delete fail. The UI trusts that tracking ref and offers the delete, but
    /// `git push --delete` rejects an already-absent ref ("remote ref does not
    /// exist"). The desired end state (no such branch on origin) already holds,
    /// so the op must succeed AND prune the phantom ref so the REMOTE list
    /// clears it instead of looping on a doomed delete.
    #[tokio::test]
    async fn delete_remote_branch_tolerates_already_absent_ref() {
        let (_tmp, dir) = fixture();
        sh_git(&dir, &["add", "-A"]);
        sh_git(&dir, &["commit", "-m", "tidy"]);
        let parent = dir.parent().unwrap();
        sh_git(parent, &["init", "--bare", "origin.git"]);
        let bare = parent.join("origin.git");
        sh_git(&dir, &["remote", "add", "origin", bare.to_str().unwrap()]);
        sh_git(&dir, &["push", "origin", "main"]);
        sh_git(&dir, &["branch", "tmp"]);
        sh_git(&dir, &["push", "origin", "tmp"]);
        sh_git(&dir, &["fetch", "origin"]);

        // Someone else deletes `tmp` on origin WITHOUT pruning our tracking ref:
        // origin no longer has it, but `origin/tmp` lingers locally (stale).
        sh_git(&bare, &["update-ref", "-d", "refs/heads/tmp"]);

        let git = LocalGit::new(&dir);
        assert!(
            git.refs()
                .await
                .unwrap()
                .remote
                .iter()
                .any(|b| b.name == "origin/tmp"),
            "stale origin/tmp present before delete"
        );

        // Must NOT error even though origin has no such ref anymore.
        git.delete_remote_branch("tmp", None)
            .await
            .expect("deleting an already-absent remote branch is a no-op success");

        // …and the stale tracking ref is pruned, clearing the UI.
        assert!(
            !git
                .refs()
                .await
                .unwrap()
                .remote
                .iter()
                .any(|b| b.name == "origin/tmp"),
            "stale origin/tmp pruned after no-op remote delete"
        );
    }

    /// Minimal repo whose initial (and only) branch is `branch` — the
    /// master-only/develop-only shape that used to make `git diff main` exit 128.
    fn fixture_on_branch(branch: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        sh_git(&dir, &["init", "-b", branch]);
        sh_git(&dir, &["config", "user.email", "otto@test.local"]);
        sh_git(&dir, &["config", "user.name", "Otto Test"]);
        sh_git(&dir, &["config", "commit.gpgsign", "false"]);
        write(&dir, "a.txt", "hello\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        (tmp, dir)
    }

    #[tokio::test]
    async fn default_branch_probes_master_when_no_main() {
        let (_tmp, dir) = fixture_on_branch("master");
        let git = LocalGit::new(&dir);
        assert_eq!(git.default_branch().await.as_deref(), Some("master"));
    }

    #[tokio::test]
    async fn resolve_base_explicit_hit_fallback_and_none() {
        let (_tmp, dir) = fixture_on_branch("master");
        let git = LocalGit::new(&dir);
        // Explicit existing ref wins as-is.
        let r = git.resolve_base(Some("master")).await.unwrap();
        assert_eq!((r.diff_ref.as_str(), r.branch.as_str()), ("master", "master"));
        // A missing explicit ref falls back to the detected default — the exact
        // production failure ("main" on a master-only repo) becomes a success.
        let r = git.resolve_base(Some("main")).await.unwrap();
        assert_eq!(r.branch, "master");
        // want=None resolves the default directly.
        let r = git.resolve_base(None).await.unwrap();
        assert_eq!(r.branch, "master");
        // A SHA verifies too (run-engine passes base commits).
        let sha = git.rev_parse("HEAD").await.unwrap();
        let r = git.resolve_base(Some(&sha)).await.unwrap();
        assert_eq!(r.diff_ref, sha);
    }

    #[tokio::test]
    async fn resolve_base_error_lists_candidates() {
        // A repo with no commits has no branches at all — nothing can resolve.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        sh_git(&dir, &["init", "-b", "main"]);
        let git = LocalGit::new(&dir);
        let err = git.resolve_base(Some("develop")).await.unwrap_err().to_string();
        assert!(err.contains("develop"), "error names what was tried: {err}");
    }

    #[tokio::test]
    async fn resolve_base_rejects_option_injection() {
        let (_tmp, dir) = fixture_on_branch("master");
        let git = LocalGit::new(&dir);
        // An option-looking "ref" from untrusted input must not be treated as a
        // git flag; it fails verification and detection falls back to master.
        let r = git.resolve_base(Some("--output=/tmp/pwn")).await.unwrap();
        assert_eq!(r.branch, "master");
        assert!(!std::path::Path::new("/tmp/pwn").exists());
    }

    #[tokio::test]
    async fn worktree_for_branch_finds_checkout() {
        let (tmp, dir) = fixture_on_branch("master");
        let git = LocalGit::new(&dir);
        let wt = tmp.path().join("wt-feature");
        sh_git(&dir, &["worktree", "add", "-b", "feature/x", wt.to_str().unwrap()]);
        let found = git.worktree_for_branch("feature/x").await.unwrap();
        assert_eq!(
            std::fs::canonicalize(&found).unwrap(),
            std::fs::canonicalize(&wt).unwrap()
        );
        // The main checkout itself is a worktree entry too.
        let main_wt = git.worktree_for_branch("master").await.unwrap();
        assert_eq!(
            std::fs::canonicalize(&main_wt).unwrap(),
            std::fs::canonicalize(&dir).unwrap()
        );
        assert!(git.worktree_for_branch("nope").await.is_none());
    }

    /// Regression: discarding a STAGED RENAME must restore the file at its
    /// original path — restoring only the new name removed the file entirely
    /// (absent at HEAD) and left the old name staged-deleted.
    #[tokio::test]
    async fn discard_staged_rename_restores_original_file() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);

        git.discard(&["d.txt".to_string()]).await.unwrap();

        let original = "carrot content that is long enough to track renames\n";
        assert_eq!(
            std::fs::read_to_string(dir.join("c.txt")).unwrap(),
            original,
            "file restored at its ORIGINAL path with original content"
        );
        assert!(!dir.join("d.txt").exists(), "new name gone after discard");
        let st = git.status().await.unwrap();
        assert!(
            !st.changes.iter().any(|c| c.path == "c.txt" || c.path == "d.txt"),
            "rename fully undone — no residual staged entries: {:?}",
            st.changes
        );
    }

    /// Amend with an EMPTY message folds staged changes into HEAD and keeps
    /// the previous commit message (`--amend --no-edit`).
    #[tokio::test]
    async fn amend_empty_message_keeps_previous_message() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);

        // Fold the staged rename + add into "second commit".
        let before = git.log(1, 0, false).await.unwrap()[0].clone();
        let sha = git.commit("", true).await.unwrap();
        assert_ne!(sha, before.sha, "amend rewrote HEAD");
        let after = git.log(1, 0, false).await.unwrap()[0].clone();
        assert_eq!(after.subject, "second commit", "message preserved");
        // Empty message WITHOUT amend still rejects.
        assert!(git.commit("", false).await.is_err());
    }

    /// A merge commit's diff must show the changes it brought in (first-parent
    /// diff); the combined default rendered an empty/garbled diff.
    #[tokio::test]
    async fn merge_commit_diff_shows_first_parent_changes() {
        let (_tmp, dir) = fixture();
        // Clean the dirty fixture state so branching is simple.
        sh_git(&dir, &["checkout", "--", "."]);
        sh_git(&dir, &["stash", "--include-untracked"]);
        sh_git(&dir, &["checkout", "-b", "feature"]);
        write(&dir, "merged.txt", "from the feature branch\n");
        sh_git(&dir, &["add", "merged.txt"]);
        sh_git(&dir, &["commit", "-m", "feature work"]);
        sh_git(&dir, &["checkout", "main"]);
        sh_git(&dir, &["merge", "--no-ff", "--no-edit", "feature"]);

        let git = LocalGit::new(&dir);
        let head = git.log(1, 0, false).await.unwrap()[0].clone();
        assert_eq!(head.parents.len(), 2, "fixture produced a merge commit");
        let diff = git.diff(DiffTarget::Commit(head.sha.clone()), None).await.unwrap();
        assert!(
            diff.files.iter().any(|f| f.path == "merged.txt"),
            "merge diff lists the merged file: {:?}",
            diff.files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
    }

    /// Untracked files with non-ASCII names must appear in the Working diff —
    /// quotePath escaping made `--no-index` miss them silently.
    #[tokio::test]
    async fn untracked_non_ascii_filename_appears_in_working_diff() {
        let (_tmp, dir) = fixture();
        write(&dir, "café.txt", "accented\n");
        let git = LocalGit::new(&dir);
        let diff = git.diff(DiffTarget::Working, None).await.unwrap();
        assert!(
            diff.files.iter().any(|f| f.path.contains("café")),
            "non-ASCII untracked file present: {:?}",
            diff.files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
    }
}
