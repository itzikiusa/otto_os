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

/// Result of a pull that may have merged with conflicts. `conflicted_files` is
/// empty on a clean pull; when it isn't, the fetch+merge ran and left a merge in
/// progress that the caller must surface for resolution.
#[derive(Debug, Clone, Default)]
pub struct PullOutcome {
    /// git's combined stdout/stderr (progress + summary), noise stripped.
    pub output: String,
    /// Paths left unmerged by the pull's merge. Empty on a clean pull.
    pub conflicted_files: Vec<String>,
}

/// Outcome of an auto-stashed switch: whether a stash was needed, and whether
/// restoring it left conflicts (git keeps the entry; the tree shows them).
#[derive(Debug, Clone, Copy, Default)]
pub struct CheckoutOutcome {
    pub stashed: bool,
    pub pop_conflicted: bool,
}

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
                    // The rev reaches `git show` as a positional argument: an
                    // option-looking one (`--output=/tmp/x`) would be a file-write
                    // primitive handed to every Viewer.
                    LocalGit::guard_ref(sha)?;
                    return Ok(Self::Commit(sha.to_string()));
                }
                if let Some(range) = s.strip_prefix("range:") {
                    if let Some((a, b)) = range.split_once("..") {
                        if !a.is_empty() && !b.is_empty() {
                            LocalGit::guard_ref(a)?;
                            LocalGit::guard_ref(b)?;
                            // A space would split one argv slot into two once the
                            // range is re-assembled as `<a>..<b>`.
                            if a.chars().chain(b.chars()).any(char::is_whitespace) {
                                return Err(Error::Invalid(format!("bad range: {range}")));
                            }
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

/// How long a spawn may run and whether the REQUEST may cancel it.
///
/// `LocalRead` (log/diff/status/refs/blame…) writes nothing, so it stays tied
/// to the handler future: a client disconnect kills it and frees the work.
/// `LocalWrite` (commit/checkout/stash/merge…) and `Remote` (fetch/push/pull/
/// ls-remote) are DETACHED from the request — hyper drops the handler future on
/// a client abort, and a SIGKILL there would leave `.git/index.lock` (git only
/// cleans it up on SIGTERM/INT/HUP/QUIT) or a half-written `FETCH_HEAD.lock`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpawnClass {
    LocalRead,
    LocalWrite,
    Remote,
}

/// Seconds a local spawn may run before it is signalled. `OTTO_GIT_TIMEOUT_SECS`
/// overrides it; `0`/unparsable → the default (a budget is never disabled).
fn local_budget_secs() -> u64 {
    static V: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *V.get_or_init(|| env_secs("OTTO_GIT_TIMEOUT_SECS", 30))
}

/// Same for remote spawns (`OTTO_GIT_REMOTE_TIMEOUT_SECS`, default 180 s) —
/// a fetch over a slow VPN legitimately takes minutes.
fn remote_budget_secs() -> u64 {
    static V: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *V.get_or_init(|| env_secs("OTTO_GIT_REMOTE_TIMEOUT_SECS", 180))
}

fn env_secs(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(default)
}

fn budget_for(class: SpawnClass) -> std::time::Duration {
    let secs = match class {
        SpawnClass::Remote => remote_budget_secs(),
        SpawnClass::LocalRead | SpawnClass::LocalWrite => local_budget_secs(),
    };
    std::time::Duration::from_secs(secs)
}

fn io_err(e: std::io::Error) -> Error {
    Error::Internal(format!("git io: {e}"))
}

/// The git subcommand in an argv, for error text: `args[0]`, or `args[2]` when
/// the call is prefixed with `-c <key=value>` (the diff family does that).
fn verb_of<'a>(args: &'a [&'a str]) -> &'a str {
    match args.first() {
        Some(&"-c") => args.get(2).copied().unwrap_or("command"),
        Some(v) => v,
        None => "command",
    }
}

/// SIGTERM the whole process group (git plus the `ssh` / `git-remote-https`
/// children it forked), give it 2 s to unwind — git removes `index.lock` on
/// SIGTERM — then SIGKILL whatever is left.
async fn kill_group(pid: libc::pid_t) {
    // SAFETY: signalling a process group we created ourselves with
    // `process_group(0)`; ESRCH (already gone and reaped) is ignored.
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
}

// ---------------------------------------------------------------------------
// The one git command builder for FILE paths and PARSED output
// ---------------------------------------------------------------------------

/// Output-format flags every `diff` / `show` whose text Otto PARSES (or hands
/// to `git apply`) carries. Without them the user's own config leaks in:
/// `diff.external` (difftastic) prints no `diff --git` headers at all (the
/// Changes view goes empty), `diff.noprefix` / `diff.mnemonicPrefix` change
/// the `a/`/`b/` prefixes the path matchers key on (every hunk op 404s/409s),
/// and a textconv filter renders something that is not the blob.
pub(crate) const DIFF_FORMAT: [&str; 5] = [
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--src-prefix=a/",
    "--dst-prefix=b/",
];

/// One git invocation — argv, per-spawn env and spawn class. Every call that
/// names FILES, or whose output is parsed as file names/diffs, is built here:
///
/// * [`GitCmd::paths`] appends `-- <paths…>` and sets `GIT_LITERAL_PATHSPECS=1`
///   for THAT spawn. After `--` git still reads `[ ] * ?` and a leading `:` as
///   pathspec magic, so discarding `app/[id]/page.tsx` also reverted
///   `app/d/page.tsx` and `clean`ed an untracked `app/i/page.tsx` — every
///   Next.js/SvelteKit route folder is shaped like that. The env var (not the
///   global `--literal-pathspecs` flag) keeps `argv[0]` the subcommand for
///   [`verb_of`] and the test shims keyed on `$1`. Never process-wide:
///   [`LocalGit::commit_all_if_dirty`] relies on `:(exclude)` magic.
/// * [`GitCmd::diff`] fixes the output format ([`DIFF_FORMAT`], raw UTF-8
///   names via `core.quotePath=false`).
/// * [`GitCmd::read`] spawns are `LocalRead`, which [`LocalGit::spawn_output`]
///   runs with `GIT_OPTIONAL_LOCKS=0`: a background status refresh must never
///   take `index.lock` out from under an agent's `git commit`.
///
/// Name lists are requested with `-z` at the call site and split on NUL.
#[derive(Debug, Clone)]
pub(crate) struct GitCmd {
    args: Vec<String>,
    envs: Vec<(String, String)>,
    class: SpawnClass,
}

impl GitCmd {
    /// A read-only command: cancelled with the request, no optional locks.
    pub(crate) fn read(args: &[&str]) -> Self {
        Self::with_class(args, SpawnClass::LocalRead)
    }

    /// An index/worktree-writing command: detached from the request.
    pub(crate) fn write(args: &[&str]) -> Self {
        Self::with_class(args, SpawnClass::LocalWrite)
    }

    /// `git -c core.quotePath=false <sub> <DIFF_FORMAT…>` — a read whose
    /// output is parsed. `sub` is `diff` or `show`.
    pub(crate) fn diff(sub: &str) -> Self {
        Self::read(&["-c", "core.quotePath=false", sub]).args(DIFF_FORMAT)
    }

    fn with_class(args: &[&str], class: SpawnClass) -> Self {
        Self {
            args: args.iter().map(|s| s.to_string()).collect(),
            envs: Vec::new(),
            class,
        }
    }

    pub(crate) fn args<I, S>(mut self, extra: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.extend(extra.into_iter().map(Into::into));
        self
    }

    /// `-- <paths…>`, each a LITERAL file name (see the type docs).
    pub(crate) fn paths<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.args.push("--".to_string());
        self.args.extend(paths.into_iter().map(Into::into));
        if !self.envs.iter().any(|(k, _)| k == "GIT_LITERAL_PATHSPECS") {
            self.envs
                .push(("GIT_LITERAL_PATHSPECS".to_string(), "1".to_string()));
        }
        self
    }

    /// [`Self::paths`] when `path` is `Some`, unchanged otherwise.
    pub(crate) fn maybe_path(self, path: Option<&str>) -> Self {
        match path {
            Some(p) => self.paths([p]),
            None => self,
        }
    }

    fn argv(&self) -> Vec<&str> {
        self.args.iter().map(String::as_str).collect()
    }
}

/// Split `-z` output into its non-empty NUL-terminated records.
pub(crate) fn nul_records(out: &str) -> impl Iterator<Item = &str> {
    out.split('\0').filter(|r| !r.is_empty())
}

/// A handle on one local repository; every method spawns `git -C <path> …`.
pub struct LocalGit {
    repo_path: PathBuf,
    /// The git binary to spawn. Overridable so tests can point at a shim
    /// without touching the process `PATH`.
    git_bin: PathBuf,
    /// Test-only override of BOTH spawn budgets.
    budget_override: Option<std::time::Duration>,
}

impl LocalGit {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
            git_bin: PathBuf::from("git"),
            budget_override: None,
        }
    }

    /// Spawn `bin` instead of `git` (tests: a shim that sleeps/traps signals).
    pub fn with_git_bin(mut self, bin: impl Into<PathBuf>) -> Self {
        self.git_bin = bin.into();
        self
    }

    /// Override both spawn budgets (tests: a sub-second timeout).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn with_budget(mut self, d: std::time::Duration) -> Self {
        self.budget_override = Some(d);
        self
    }

    pub fn path(&self) -> &Path {
        &self.repo_path
    }

    // -- plumbing -----------------------------------------------------------

    pub(crate) fn base_cmd(&self) -> Command {
        let mut cmd = Command::new(&self.git_bin);
        cmd.current_dir(&self.repo_path)
            .env("GIT_TERMINAL_PROMPT", "0")
            // Force English output: every error classification here
            // (`local_refusal`, "CONFLICT", "has no upstream branch", …) matches
            // git's English wording, which a non-English LANG silently breaks.
            .env("LC_ALL", "C")
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

    /// Refuse a caller-supplied ref / path / revision that git would read as an
    /// OPTION. Branch names, commit-ish, worktree paths and pathspecs arrive
    /// from HTTP handlers; a value such as `--upload-pack=…` or `-c…` spliced
    /// into an argv changes what git does instead of what it operates on.
    /// Refs can never legitimately start with `-` (`git check-ref-format`), and
    /// the few commands here that take a free-form path already pass `--`.
    pub(crate) fn guard_ref(value: &str) -> Result<()> {
        let v = value.trim_start();
        if v.starts_with('-') {
            return Err(Error::Invalid(format!(
                "refusing option-like git argument '{value}'"
            )));
        }
        if value.chars().any(|c| c == '\0' || c == '\n' || c == '\r') {
            return Err(Error::Invalid(
                "git argument contains a control character".into(),
            ));
        }
        Ok(())
    }

    /// Refuse a caller-supplied PATH that could smuggle a second argument or
    /// break the argv (empty, or a control character). Unlike [`guard_ref`]
    /// a leading `-` is allowed — `-notes.md` is a legal filename — because
    /// every path argument in this crate is emitted after `--`.
    pub(crate) fn guard_path(value: &str) -> Result<()> {
        if value.is_empty() {
            return Err(Error::Invalid("path must not be empty".into()));
        }
        if value.chars().any(|c| c == '\0' || c == '\n' || c == '\r') {
            return Err(Error::Invalid("path contains a control character".into()));
        }
        Ok(())
    }

    /// Run git with args; non-zero exit → `Error::Upstream(first stderr line)`.
    /// Returns stdout. Public for callers with plumbing needs the typed API
    /// doesn't cover (e.g. the workflow engine's worktree reaper resolving
    /// `--git-common-dir` / branch reachability).
    pub async fn run(&self, args: &[&str]) -> Result<String> {
        self.run_env(args, &[]).await.map(|(out, _)| out)
    }

    /// Run server automation plumbing without editor or SSH password prompts.
    /// The explicit remote flag chooses the existing network budget; both paths
    /// retain process-group cleanup and detached completion on caller disconnect.
    pub async fn run_automation(&self, args: &[&str], remote: bool) -> Result<String> {
        let envs = [
            ("GIT_EDITOR".to_owned(), "true".to_owned()),
            (
                "GIT_SSH_COMMAND".to_owned(),
                "ssh -o BatchMode=yes".to_owned(),
            ),
        ];
        let class = if remote {
            SpawnClass::Remote
        } else {
            SpawnClass::LocalWrite
        };
        self.run_env_class(args, &envs, class)
            .await
            .map(|(out, _)| out)
    }

    /// Run a READ-ONLY git command (log/diff/status/refs/…): bounded by the
    /// local budget and cancelled with the request, since nothing is written.
    pub(crate) async fn run_read(&self, args: &[&str]) -> Result<String> {
        self.run_env_class(args, &[], SpawnClass::LocalRead)
            .await
            .map(|(out, _)| out)
    }

    /// Run git with extra env vars; returns (stdout, stderr).
    pub(crate) async fn run_env(
        &self,
        args: &[&str],
        envs: &[(String, String)],
    ) -> Result<(String, String)> {
        self.run_env_class(args, envs, SpawnClass::LocalWrite).await
    }

    async fn run_env_class(
        &self,
        args: &[&str],
        envs: &[(String, String)],
        class: SpawnClass,
    ) -> Result<(String, String)> {
        self.check_repo().await?;
        let mut cmd = self.base_cmd();
        cmd.args(args);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = self.spawn_output(cmd, class, verb_of(args), None).await?;
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
    pub(crate) async fn run_raw(
        &self,
        args: &[&str],
        envs: &[(String, String)],
    ) -> Result<(bool, String, String, Option<i32>)> {
        self.run_raw_class(args, envs, SpawnClass::LocalWrite).await
    }

    pub(crate) async fn run_raw_class(
        &self,
        args: &[&str],
        envs: &[(String, String)],
        class: SpawnClass,
    ) -> Result<(bool, String, String, Option<i32>)> {
        self.check_repo().await?;
        let mut cmd = self.base_cmd();
        cmd.args(args);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = self.spawn_output(cmd, class, verb_of(args), None).await?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Ok((out.status.success(), stdout, stderr, out.status.code()))
    }

    /// Like [`Self::run_raw`] but feeds `stdin` to git (a patch for `git
    /// apply`). Index-writing, so it takes the `LocalWrite` class.
    pub(crate) async fn run_raw_stdin(
        &self,
        args: &[&str],
        stdin: &[u8],
    ) -> Result<(bool, String, String, Option<i32>)> {
        self.check_repo().await?;
        let mut cmd = self.base_cmd();
        cmd.args(args);
        let out = self
            .spawn_output(cmd, SpawnClass::LocalWrite, verb_of(args), Some(stdin))
            .await?;
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Ok((out.status.success(), stdout, stderr, out.status.code()))
    }

    /// Spawn a [`GitCmd`] without judging its exit: `(success, stdout BYTES,
    /// stderr, code)`. Bytes, because a diff that `git apply` will read back
    /// must not pass through a lossy UTF-8 decode (a Latin-1 `\xE9` came back
    /// as `\xEF\xBF\xBD` and was staged that way).
    pub(crate) async fn exec(
        &self,
        c: &GitCmd,
        stdin: Option<&[u8]>,
    ) -> Result<(bool, Vec<u8>, String, Option<i32>)> {
        self.check_repo().await?;
        let argv = c.argv();
        let mut cmd = self.base_cmd();
        cmd.args(&argv);
        for (k, v) in &c.envs {
            cmd.env(k, v);
        }
        let out = self.spawn_output(cmd, c.class, verb_of(&argv), stdin).await?;
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Ok((out.status.success(), out.stdout, stderr, out.status.code()))
    }

    /// [`Self::exec`] where a non-zero exit is an error, classified exactly
    /// like [`Self::run`]. Returns stdout as raw bytes.
    pub(crate) async fn exec_bytes(&self, c: &GitCmd) -> Result<Vec<u8>> {
        let (ok, stdout, stderr, code) = self.exec(c, None).await?;
        if !ok {
            let err = upstream_err(&stderr, &String::from_utf8_lossy(&stdout), code);
            tracing::warn!(
                repo = %self.repo_path.display(),
                args = ?c.args,
                code = code,
                "git failed: {err}"
            );
            return Err(err);
        }
        Ok(stdout)
    }

    /// [`Self::exec_bytes`], stdout decoded (lossily) as text.
    pub(crate) async fn exec_text(&self, c: &GitCmd) -> Result<String> {
        let out = self.exec_bytes(c).await?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    }

    /// [`Self::exec_bytes`] for an index-writing command, with the same
    /// bounded `index.lock` retry as [`Self::run_locked`].
    pub(crate) async fn exec_locked(&self, c: &GitCmd) -> Result<()> {
        for attempt in 1u64..=3 {
            let (ok, stdout, stderr, code) = self.exec(c, None).await?;
            if ok {
                return Ok(());
            }
            if attempt < 3 && stderr.contains("index.lock") {
                tokio::time::sleep(std::time::Duration::from_millis(300 * attempt)).await;
                continue;
            }
            let err = upstream_err(&stderr, &String::from_utf8_lossy(&stdout), code);
            tracing::warn!(
                repo = %self.repo_path.display(),
                args = ?c.args,
                code = code,
                "git failed: {err}"
            );
            return Err(err);
        }
        unreachable!("loop returns on its final attempt")
    }

    /// The one place a git process is created. Every spawn gets its own process
    /// GROUP (`setpgid(0,0)`) so a timeout can signal git AND the `ssh` /
    /// `git-remote-https` helpers it forked with a single `kill(-pid)`.
    ///
    /// `LocalWrite`/`Remote` run inside a detached task: dropping the request
    /// future (client disconnect — the UI passes an `AbortSignal` on every
    /// call) then stops the WAIT, not the git. `kill_on_drop` stays armed as a
    /// last-resort guard for runtime shutdown.
    async fn spawn_output(
        &self,
        mut cmd: Command,
        class: SpawnClass,
        verb: &str,
        stdin: Option<&[u8]>,
    ) -> Result<std::process::Output> {
        cmd.process_group(0).kill_on_drop(true);
        if class == SpawnClass::LocalRead {
            // A read must never take `index.lock`: `git status` otherwise
            // refreshes the index opportunistically, and an agent's concurrent
            // `git commit`/`git add` (which do NOT retry) dies with "index.lock
            // exists". Covers every status refresh and the worktree probe.
            cmd.env("GIT_OPTIONAL_LOCKS", "0");
        }
        if stdin.is_some() {
            cmd.stdin(Stdio::piped());
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Internal(format!("spawn git: {e}")))?;
        if let Some(bytes) = stdin {
            use tokio::io::AsyncWriteExt;
            let mut si = child.stdin.take().expect("piped stdin");
            si.write_all(bytes)
                .await
                .map_err(|e| Error::Internal(format!("git stdin: {e}")))?;
            drop(si); // EOF — git blocks reading otherwise
        }
        let pid = child.id().expect("spawned") as libc::pid_t;
        let budget = self.budget_override.unwrap_or_else(|| budget_for(class));
        let secs = budget.as_secs();
        let waited = match class {
            SpawnClass::LocalRead => tokio::time::timeout(budget, child.wait_with_output())
                .await
                .map(|r| r.map_err(io_err)),
            SpawnClass::LocalWrite | SpawnClass::Remote => {
                let jh = tokio::spawn(async move { child.wait_with_output().await });
                tokio::time::timeout(budget, async move {
                    jh.await
                        .map_err(|e| Error::Internal(format!("git task: {e}")))?
                        .map_err(io_err)
                })
                .await
            }
        };
        match waited {
            Ok(r) => r,
            Err(_) => {
                kill_group(pid).await;
                Err(Error::Upstream(format!(
                    "git {verb} timed out after {secs}s — if it was writing, \
                     `.git/index.lock` may be left behind; remove it once no git \
                     process is running"
                )))
            }
        }
    }

    // -- queries ------------------------------------------------------------

    /// True iff `commit` is an ancestor of (already merged into) `branch`.
    ///
    /// Used to detect merge-to-`develop` completion. `git merge-base
    /// --is-ancestor` exits 0 (ancestor), 1 (not an ancestor), or another code
    /// (error, e.g. an unknown ref) — distinguished here so a bad ref surfaces as
    /// an error rather than a silent `false`.
    pub async fn is_ancestor_of(&self, commit: &str, branch: &str) -> Result<bool> {
        Self::guard_ref(commit)?;
        Self::guard_ref(branch)?;
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
        //
        // `-z`: without it git C-quotes any name with a non-ASCII byte, `"` or
        // `\` (`"caf\303\251.txt"`), and that quoted form is what came back
        // to stage/discard — `add` then failed "did not match", and `clean`
        // exited 0 having deleted NOTHING while the UI said "Discarded".
        let out = self
            .exec_text(&GitCmd::read(&[
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=all",
            ]))
            .await?;
        let mut st = crate::parse::parse_status(&out);
        st.op_in_progress = self.op_in_progress().await.map(str::to_string);
        Ok(st)
    }

    /// Absolute path of this worktree's git dir: `.git` when it's a directory,
    /// or the `gitdir:` target when `.git` is a file (linked worktree /
    /// submodule). Pure filesystem — called on every `status()`, so it must not
    /// cost a git process.
    pub(crate) async fn git_dir(&self) -> Option<PathBuf> {
        let dot = self.repo_path.join(".git");
        match tokio::fs::metadata(&dot).await {
            Ok(m) if m.is_dir() => Some(dot),
            Ok(_) => {
                let text = tokio::fs::read_to_string(&dot).await.ok()?;
                let rel = text.strip_prefix("gitdir:")?.trim();
                let p = Path::new(rel);
                Some(if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    self.repo_path.join(p)
                })
            }
            Err(_) => None,
        }
    }

    /// Which multi-step git operation is underway in this worktree, if any:
    /// "rebase" | "merge" | "cherry_pick" | "revert". Presence-of-state-file
    /// checks mirroring git's own wt-status logic. Rebase is checked FIRST —
    /// `rebase -r` can hold a MERGE_HEAD mid-rebase, and "rebase" is the label
    /// whose abort/continue actually applies then.
    pub async fn op_in_progress(&self) -> Option<&'static str> {
        let gd = self.git_dir().await?;
        for (file, op) in [
            ("rebase-merge", "rebase"),
            ("rebase-apply", "rebase"),
            ("MERGE_HEAD", "merge"),
            ("CHERRY_PICK_HEAD", "cherry_pick"),
            ("REVERT_HEAD", "revert"),
        ] {
            if tokio::fs::metadata(gd.join(file)).await.is_ok() {
                return Some(op);
            }
        }
        None
    }

    /// True when a `merge --squash` left its staged result pending (the state
    /// `merge_abort` may discard with `reset --hard`). SQUASH_MSG is the only
    /// marker git leaves for it.
    async fn squash_pending(&self) -> bool {
        match self.git_dir().await {
            Some(gd) => tokio::fs::metadata(gd.join("SQUASH_MSG")).await.is_ok(),
            None => false,
        }
    }

    pub async fn branches(&self) -> Result<Vec<BranchInfo>> {
        let out = self
            .run_read(&[
                "branch",
                "--format=%(refname:short)%09%(upstream:short)%09%(HEAD)",
            ])
            .await?;
        Ok(crate::parse::parse_branches(&out))
    }

    /// The checked-out branch name; `"HEAD"` when detached. `symbolic-ref`
    /// first: `rev-parse --abbrev-ref HEAD` dies on an UNBORN branch (a fresh
    /// repo before its first commit), which broke branch delete and the push
    /// `--set-upstream` fallback there.
    pub async fn current_branch(&self) -> Result<String> {
        if let Some(name) = self.symbolic_head().await {
            return Ok(name);
        }
        let out = self
            .run_read(&["rev-parse", "--abbrev-ref", "HEAD"])
            .await?;
        Ok(out.trim().to_string())
    }

    /// The branch HEAD points at (born or unborn); `None` when detached.
    /// Raw spawn: a detached HEAD exits 1, which is an answer, not a failure
    /// worth a warning in the daemon log.
    async fn symbolic_head(&self) -> Option<String> {
        match self
            .run_raw_class(
                &["symbolic-ref", "-q", "--short", "HEAD"],
                &[],
                SpawnClass::LocalRead,
            )
            .await
        {
            Ok((true, out, _, _)) => Some(out.trim().to_string()).filter(|s| !s.is_empty()),
            _ => None,
        }
    }

    /// Resolve a ref (branch/sha/`HEAD`) to its full commit SHA. Used by Goal
    /// Loops to capture the launch HEAD as the diff base for the loop's branch.
    pub async fn rev_parse(&self, reference: &str) -> Result<String> {
        Self::guard_ref(reference)?;
        let out = self.run_read(&["rev-parse", reference]).await?;
        Ok(out.trim().to_string())
    }

    /// Files this worktree's HEAD changed relative to `base` (`git diff
    /// --name-only base...HEAD`). Used by the swarm to detect when two agents'
    /// branches touch the same shared files. Empty on no changes.
    pub async fn changed_files(&self, base: &str) -> Result<Vec<String>> {
        Self::guard_ref(base)?;
        let range = format!("{base}...HEAD");
        // `-z`: raw names — a C-quoted `"caf\303\251.rs"` never equals the
        // path another agent's branch reports for the same file.
        let out = self
            .exec_text(&GitCmd::read(&[
                "diff",
                "--no-ext-diff",
                "--name-only",
                "-z",
                "--end-of-options",
                &range,
                "--",
            ]))
            .await?;
        Ok(nul_records(&out).map(str::to_string).collect())
    }

    /// True when a local branch already exists. Lets Goal Loops re-attach an
    /// existing loop branch NON-destructively instead of `-B`-resetting it.
    pub async fn branch_exists(&self, branch: &str) -> bool {
        if Self::guard_ref(branch).is_err() {
            return false;
        }
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
    pub(crate) async fn verify_commit_ref(&self, r: &str) -> bool {
        let spec = format!("{r}^{{commit}}");
        match self
            .run_raw(
                &[
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    "--end-of-options",
                    &spec,
                ],
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
        if let Ok(out) = self
            .run(&["symbolic-ref", "refs/remotes/origin/HEAD"])
            .await
        {
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
    ///
    /// A candidate that resolves to the SAME commit as HEAD is only used as a
    /// last resort: diffing a branch against itself yields an empty diff, which
    /// every caller reads as "no changes" — a review then completes with zero
    /// reviewers and scores a false 100/PASS. That happens whenever a run
    /// declares the PR's own SOURCE branch as the base (`base:
    /// feature/X-1` with `feature/X-1` checked out, detached or not).
    /// Prefer the first verified candidate that is NOT HEAD; keep a HEAD-equal
    /// one when nothing else resolves, so a branch genuinely identical to its
    /// base still resolves exactly as before.
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
        let head = self
            .rev_parse("HEAD")
            .await
            .ok()
            .filter(|s| !s.trim().is_empty());
        let mut tried: Vec<String> = Vec::new();
        // First verified candidate that equals HEAD — the fallback used only
        // when no other candidate resolves.
        let mut head_equal: Option<ResolvedBase> = None;
        for (diff_ref, branch) in cands {
            if tried.contains(&diff_ref) {
                continue;
            }
            if self.verify_commit_ref(&diff_ref).await {
                let same_as_head = match (&head, self.rev_parse(&diff_ref).await.ok()) {
                    (Some(h), Some(c)) => !c.trim().is_empty() && c.trim() == h.trim(),
                    _ => false,
                };
                if !same_as_head {
                    return Ok(ResolvedBase { diff_ref, branch });
                }
                head_equal.get_or_insert_with(|| ResolvedBase {
                    diff_ref: diff_ref.clone(),
                    branch,
                });
            }
            tried.push(diff_ref);
        }
        if let Some(rb) = head_equal {
            return Ok(rb);
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
        Self::guard_ref(path)?;
        Self::guard_ref(branch)?;
        self.run(&["worktree", "add", "--force", path, branch])
            .await?;
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
        Self::guard_ref(path)?;
        Self::guard_ref(branch)?;
        Self::guard_ref(base)?;
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
        Self::guard_ref(path)?;
        let _ = self.run(&["worktree", "remove", "--force", path]).await;
        Ok(())
    }

    /// `git worktree list --porcelain` → parsed entries, each live worktree
    /// probed for uncommitted changes (best-effort; prunable entries are
    /// skipped — their directory is gone). The first entry is the main worktree.
    pub async fn worktree_list(&self) -> Result<Vec<WorktreeInfo>> {
        self.worktree_list_with_budgets(
            std::time::Duration::from_secs(3),
            std::time::Duration::from_secs(10),
        )
        .await
    }

    async fn worktree_list_with_budgets(
        &self,
        probe_budget: std::time::Duration,
        phase_budget: std::time::Duration,
    ) -> Result<Vec<WorktreeInfo>> {
        static ADMISSION: std::sync::OnceLock<std::sync::Arc<tokio::sync::Semaphore>> =
            std::sync::OnceLock::new();
        let admission = ADMISSION
            .get_or_init(|| std::sync::Arc::new(tokio::sync::Semaphore::new(8)))
            .clone();
        let out = self.run_read(&["worktree", "list", "--porcelain"]).await?;
        let mut rows = crate::parse::parse_worktree_list(&out);
        let bin = self.git_bin.clone();
        crate::worktree_probe::probe(&mut rows, admission, phase_budget, move |path| {
            let git = LocalGit::new(path)
                .with_git_bin(bin.clone())
                .with_budget(probe_budget);
            async move {
                // The subprocess itself checks the worktree. Avoid a separate
                // filesystem metadata await before this optional bounded probe.
                let mut cmd = git.base_cmd();
                cmd.args(["status", "--porcelain", "--untracked-files=normal"]);
                match git
                    .spawn_output(cmd, SpawnClass::LocalRead, "status", None)
                    .await
                {
                    Ok(out) if out.status.success() => Some(!out.stdout.is_empty()),
                    _ => None,
                }
            }
        })
        .await;
        Ok(rows)
    }

    /// User-facing worktree removal: surfaces git's error (unlike the reaper's
    /// best-effort [`worktree_remove`]) and only forces when asked — git refuses
    /// to remove a dirty/locked tree without `--force`, which is the safety net
    /// the UI relies on. Keeps the branch, like every other removal path.
    pub async fn worktree_remove_checked(&self, path: &str, force: bool) -> Result<()> {
        Self::guard_ref(path)?;
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
        let (out, err) = self
            .run_env(&["worktree", "prune", "--verbose"], &[])
            .await?;
        // --verbose reports on stderr in some git versions; prefer whichever spoke.
        let msg = if out.trim().is_empty() { err } else { out };
        Ok(msg.trim().to_string())
    }

    /// `git submodule status` → parsed entries enriched with `.gitmodules`
    /// url/branch. Empty list when the repo has no submodules.
    pub async fn submodule_list(&self) -> Result<Vec<SubmoduleInfo>> {
        let out = self.run_read(&["submodule", "status"]).await?;
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

    /// Read history. `limit == 0` means NO `-n` at all — the caller wants the
    /// whole reachable history, not a page of it.
    pub async fn log(&self, limit: u32, skip: u32, all: bool) -> Result<Vec<CommitInfo>> {
        let limit_s = limit.to_string();
        let skip_s = skip.to_string();
        let mut args = vec![
            "log",
            "--pretty=format:%H%x1f%h%x1f%an%x1f%aI%x1f%s%x1f%P%x1f%D%x1e",
            "--skip",
            &skip_s,
        ];
        if limit > 0 {
            args.splice(2..2, ["-n", limit_s.as_str()]);
        }
        if all {
            args.insert(1, "--all");
        }
        let out = self.run_read(&args).await?;
        crate::parse::parse_log(&out)
    }

    pub async fn refs(&self) -> Result<RefsResp> {
        self.refs_with_base(None).await
    }

    /// List refs, flagging every local/remote branch already contained in the
    /// cleanup base branch (`base_override` if valid, else the detected default).
    /// Containment is computed with two bulk `git branch --merged <base>` calls
    /// (one local, one remote) — never a per-branch merge-base spawn. The base
    /// branch itself is never flagged (it always "contains" itself). `base_branch`
    /// in the response echoes the resolved base so the UI can exclude/label it.
    pub async fn refs_with_base(&self, base_override: Option<&str>) -> Result<RefsResp> {
        // Local branches: name TAB upstream TAB HEAD-marker TAB sha TAB tracking
        let local_out = self
            .run_read(&[
                "for-each-ref",
                "--format=%(refname:short)\t%(upstream:short)\t%(HEAD)\t%(objectname)\t%(upstream:track,nobracket)",
                "refs/heads",
            ])
            .await?;

        // Resolve the base and gather the "merged into base" sets up front so each
        // branch row is a cheap set lookup. A missing base (empty repo) leaves the
        // sets empty → nothing flagged.
        let base = self.resolve_cleanup_base(base_override).await;
        let (merged_local, merged_remote) = self.merged_sets(base.as_deref()).await;

        let local = local_out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let mut cols = line.splitn(5, '\t');
                let name = cols.next().unwrap_or("").to_string();
                let upstream_raw = cols.next().unwrap_or("").trim().to_string();
                let head = cols.next().unwrap_or("").trim();
                let sha = cols.next().unwrap_or("").trim().to_string();
                let (ahead, behind) = crate::parse::parse_upstream_tracking(cols.next().unwrap_or(""));
                let merged =
                    base.as_deref() != Some(name.as_str()) && merged_local.contains(name.as_str());
                RefBranch {
                    name,
                    is_current: head == "*",
                    upstream: if upstream_raw.is_empty() {
                        None
                    } else {
                        Some(upstream_raw)
                    },
                    remote: false,
                    ahead,
                    behind,
                    merged_into_base: merged,
                    sha,
                }
            })
            .collect();

        // Remote branches: name TAB sha; skip entries ending in "/HEAD"
        let remote_out = self
            .run_read(&[
                "for-each-ref",
                "--format=%(refname:short)\t%(objectname)",
                "refs/remotes",
            ])
            .await?;
        let remote = remote_out
            .lines()
            .filter(|l| {
                let name = l.split('\t').next().unwrap_or("").trim();
                !l.trim().is_empty() && !name.ends_with("/HEAD")
            })
            .map(|line| {
                let mut cols = line.splitn(2, '\t');
                let name = cols.next().unwrap_or("").trim().to_string();
                let sha = cols.next().unwrap_or("").trim().to_string();
                // Don't flag the base's own remote twin (origin/<base>) as safe.
                let is_base_remote = base
                    .as_deref()
                    .is_some_and(|b| name.strip_prefix("origin/") == Some(b));
                let merged = !is_base_remote && merged_remote.contains(name.as_str());
                RefBranch {
                    name,
                    is_current: false,
                    upstream: None,
                    remote: true,
                    ahead: 0,
                    behind: 0,
                    merged_into_base: merged,
                    sha,
                }
            })
            .collect();

        // Tags: sorted newest-first, ALL of them (an old tag is precisely the one
        // a user reaches for). `%(*objectname)` is the dereferenced commit and is
        // non-empty only for ANNOTATED tags, so fall back to `%(objectname)` for
        // lightweight ones — either way `sha` names a commit, never a tag object.
        let tags_out = self
            .run_read(&[
                "for-each-ref",
                "--sort=-creatordate",
                "--format=%(refname:short)\t%(objectname)\t%(*objectname)",
                "refs/tags",
            ])
            .await?;
        let tags = tags_out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let mut cols = line.splitn(3, '\t');
                let name = cols.next().unwrap_or("").trim().to_string();
                let obj = cols.next().unwrap_or("").trim();
                let deref = cols.next().unwrap_or("").trim();
                RefTag {
                    name,
                    sha: if deref.is_empty() { obj } else { deref }.to_string(),
                }
            })
            .collect();

        Ok(RefsResp {
            local,
            remote,
            tags,
            base_branch: base,
        })
    }

    /// Resolve the base branch for cleanup indicators: the override (verified to
    /// exist) if given, else the detected [`default_branch`](Self::default_branch).
    pub async fn resolve_cleanup_base(&self, base_override: Option<&str>) -> Option<String> {
        if let Some(b) = base_override.map(str::trim).filter(|s| !s.is_empty()) {
            if self.branch_exists(b).await || self.verify_commit_ref(b).await {
                return Some(b.to_string());
            }
        }
        self.default_branch().await
    }

    /// The `(local, remote)` sets of branch short-names whose tip is contained in
    /// `base` — one bulk `git branch --merged` each. Empty when `base` is `None`
    /// or the git call fails (best-effort: cleanup hints must never break `refs`).
    async fn merged_sets(
        &self,
        base: Option<&str>,
    ) -> (
        std::collections::HashSet<String>,
        std::collections::HashSet<String>,
    ) {
        let Some(base) = base else {
            return (Default::default(), Default::default());
        };
        let parse = |out: String| {
            out.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.ends_with("/HEAD"))
                .map(str::to_string)
                .collect::<std::collections::HashSet<String>>()
        };
        let local = self
            .run_read(&["branch", "--merged", base, "--format=%(refname:short)"])
            .await
            .map(parse)
            .unwrap_or_default();
        let remote = self
            .run_read(&[
                "branch",
                "-r",
                "--merged",
                base,
                "--format=%(refname:short)",
            ])
            .await
            .map(parse)
            .unwrap_or_default();
        (local, remote)
    }

    /// Compute a diff for `target`. When `pathspec` is `Some(path)`, every git
    /// invocation is scoped to that single file (`-- <path>`) — so selecting one
    /// file in the UI computes ONLY that file's diff instead of the entire
    /// working tree (which, for the `Working` target, also runs a `--no-index`
    /// diff per untracked file — seconds of work on a large changeset). `None`
    /// returns the full diff (the "All changes" view, commit/range views).
    pub async fn diff(&self, target: DiffTarget, pathspec: Option<&str>) -> Result<DiffResp> {
        let path = pathspec.filter(|p| !p.is_empty());
        if let Some(p) = path {
            Self::guard_path(p)?;
        }
        // Every call is a [`GitCmd::diff`] — fixed output format, raw UTF-8
        // names (`core.quotePath=false`: the default octal-escaping broke
        // feeding `ls-files` names back into `--no-index`) — scoped with a
        // LITERAL `-- <path>` when one is given (`app/[id]/x.tsx` must not
        // glob-match `app/d/x.tsx` into the same response). Bytes throughout:
        // `parse_diff_bytes` fingerprints exactly what git printed.
        let diff = |sub: &str, extra: &[&str]| -> GitCmd {
            GitCmd::diff(sub)
                .args(extra.iter().copied())
                .maybe_path(path)
        };
        let out: Vec<u8> = match &target {
            DiffTarget::Worktree => self.exec_bytes(&diff("diff", &["-U3", "-M"])).await?,
            DiffTarget::Working => {
                // Staged + unstaged tracked changes vs HEAD (a staged-new file
                // shows as fully added). Falls back to cached+worktree when HEAD
                // is unborn (no commits yet).
                let (head_ok, head_out, _, _) = self
                    .exec(&diff("diff", &["-U3", "-M", "HEAD"]), None)
                    .await?;
                let mut out = if head_ok {
                    head_out
                } else {
                    let mut s = self
                        .exec_bytes(&diff("diff", &["-U3", "-M", "--cached"]))
                        .await
                        .unwrap_or_default();
                    s.extend(
                        self.exec_bytes(&diff("diff", &["-U3", "-M"]))
                            .await
                            .unwrap_or_default(),
                    );
                    s
                };
                // Untracked files: render each as a fully-added diff. Scope the
                // `ls-files` to the pathspec so a single-file request only checks
                // that one path (and runs at most one `--no-index` diff). `-z`:
                // names arrive raw, never C-quoted.
                let ls = GitCmd::read(&["ls-files", "-z", "--others", "--exclude-standard"])
                    .maybe_path(path);
                let (_, untracked, _, _) = self.exec(&ls, None).await?;
                let untracked = String::from_utf8_lossy(&untracked).into_owned();
                for f in nul_records(&untracked) {
                    let (_, stdout, _, _) = self
                        .exec(
                            &GitCmd::diff("diff")
                                .args(["-U3", "--no-index"])
                                .paths(["/dev/null", f]),
                            None,
                        )
                        .await?;
                    out.extend_from_slice(&stdout);
                }
                out
            }
            DiffTarget::Staged => {
                self.exec_bytes(&diff("diff", &["-U3", "-M", "--cached"]))
                    .await?
            }
            DiffTarget::Commit(sha) => {
                // `-m --first-parent`: a merge commit's default `git show`
                // output is a combined (--cc) diff — files identical to any
                // parent are omitted (a normal integration merge shows "no
                // changes") and its `@@@` hunks don't parse. Diffing against
                // the first parent yields the reviewable "what this merge
                // brought in" diff; non-merge commits are unaffected.
                self.exec_bytes(&diff(
                    "show",
                    &[
                        "-m",
                        "--first-parent",
                        "-U3",
                        "-M",
                        "--format=",
                        "--end-of-options",
                        sha,
                    ],
                ))
                .await?
            }
            DiffTarget::Range(a, b) => {
                let range = format!("{a}..{b}");
                self.exec_bytes(&diff("diff", &["-U3", "-M", "--end-of-options", &range]))
                    .await?
            }
        };
        Ok(crate::parse::parse_diff_bytes(&out))
    }

    /// Run `git diff <base>` — diffs the working tree (staged + unstaged)
    /// against `base` and returns the raw unified diff text.
    ///
    /// These texts feed reviews and commit-message drafts, so they carry the
    /// same fixed [`DIFF_FORMAT`] as the parsed diffs: a `diff.external` in the
    /// user's config otherwise handed the reviewers an empty diff. The trailing
    /// `--` pins `base` as a REVISION even when a file shares its name.
    pub async fn diff_text_against(&self, base: &str) -> Result<String> {
        Self::guard_ref(base)?;
        self.exec_text(&GitCmd::diff("diff").args(["--end-of-options", base, "--"]))
            .await
    }

    /// Raw unified diff of the staged changes (`git diff --cached`). Empty when
    /// nothing is staged.
    pub async fn staged_diff_text(&self) -> Result<String> {
        self.exec_text(&GitCmd::diff("diff").args(["-M", "--cached"]))
            .await
    }

    /// Raw unified diff of all unstaged tracked changes (`git diff`). Used as a
    /// fallback when nothing is staged.
    pub async fn working_diff_text(&self) -> Result<String> {
        self.exec_text(&GitCmd::diff("diff").args(["-M"])).await
    }

    /// `git remote get-url origin`, best-effort.
    pub async fn remote_url(&self) -> Option<String> {
        self.run_read(&["remote", "get-url", "origin"])
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Absolute path of the work-tree root containing `repo_path` (walks up to
    /// the enclosing `.git`), or an error if the path is not inside a repo.
    pub async fn toplevel(&self) -> Result<String> {
        let out = self.run_read(&["rev-parse", "--show-toplevel"]).await?;
        let top = out.trim().to_string();
        if top.is_empty() {
            return Err(Error::Invalid("not a git repository".into()));
        }
        Ok(top)
    }

    // -- mutations ----------------------------------------------------------

    pub async fn checkout(&self, branch: &str, create: bool) -> Result<()> {
        Self::guard_ref(branch)?;
        if create {
            // Creating a branch whose name already exists on origin is almost
            // never meant as "shadow it from my (possibly stale) HEAD" — a bare
            // `checkout -b` would do exactly that AND leave the branch without
            // an upstream, so the first `pull` dies with "no tracking
            // information". Start it at the remote tip and track it instead.
            let remote = format!("origin/{branch}");
            if self
                .verify_commit_ref(&format!("refs/remotes/{remote}"))
                .await
            {
                self.run(&["checkout", "-b", branch, "--track", &remote])
                    .await?;
            } else {
                self.run(&["checkout", "-b", branch]).await?;
            }
        } else {
            // The trailing `--` makes `branch` a REVISION, never a path: a bare
            // `checkout docs` with no branch `docs` but a directory `docs/`
            // restored that directory from the index — exit 0, "Switched to
            // docs" in the UI, the user's uncommitted edits gone. With `--` git
            // refuses ("invalid reference: docs"). Tags/SHAs still detach and
            // an origin-only name still DWIM-creates its tracking branch.
            self.run(&["checkout", "--end-of-options", branch, "--"])
                .await?;
        }
        Ok(())
    }

    /// Switch with an automatic stash around a dirty tree: stash -u → checkout
    /// → pop. NEVER pulls, fetches or merges — a branch switch is a switch (the
    /// removed stash·pull·pop gesture pulled here, which turned "go look at
    /// develop" into an unasked-for merge commit).
    ///
    /// Failure contract: a failed switch pops the stash back (tree exactly as
    /// it was); a CONFLICTING pop is a normal outcome (git keeps the entry and
    /// `status()` reports `kind:"conflicted"` rows); a pop that fails for any
    /// other reason keeps the stash and is a 409 telling the user to
    /// `git stash pop`.
    pub async fn checkout_autostash(&self, branch: &str, create: bool) -> Result<CheckoutOutcome> {
        Self::guard_ref(branch)?;
        let mut dirty = !self
            .run(&["status", "--porcelain"])
            .await?
            .trim()
            .is_empty();
        if dirty {
            // `--include-untracked`: without it a new file survives the stash and
            // the checkout still dies with "untracked working tree files would be
            // overwritten" — the exact refusal this stash exists to clear.
            let (ok, out, err, code) = self
                .run_raw_retry_lock(&[
                    "stash",
                    "push",
                    "--include-untracked",
                    "-m",
                    "otto: auto-stash for branch switch",
                ])
                .await?;
            if !ok {
                return Err(upstream_err(&err, &out, code));
            }
            // Exit 0 with nothing saved (a dirty submodule, say): there is no
            // entry of OURS to pop, and popping would take the user's own
            // `stash@{0}` instead.
            if out.contains("No local changes to save") || err.contains("No local changes to save")
            {
                dirty = false;
            }
        }
        // Our entry's commit: every pop below addresses THIS stash by SHA,
        // never `stash@{0}` — an agent in the same repo may push its own
        // stash in between, and popping that one onto the user's tree is
        // both a loss (theirs) and a corruption (ours).
        let ours: Option<String> = if dirty {
            Some(
                self.run_read(&["rev-parse", "-q", "--verify", "refs/stash"])
                    .await?
                    .trim()
                    .to_string(),
            )
        } else {
            None
        };
        if let Err(e) = self.checkout(branch, create).await {
            if let Some(sha) = &ours {
                // The pop rides the same `index.lock` retry as everything else
                // here: a concurrent agent's lock must not be the reason the
                // user's changes are left behind in the stash. A CONFLICTING
                // pop is Ok (applied with markers, entry kept).
                if self.pop_stash_sha(sha).await.is_err() {
                    return Err(match e {
                        Error::Conflict(m) => Error::Conflict(format!(
                            "{m} — your changes are kept in `git stash list`"
                        )),
                        other => other,
                    });
                }
            }
            return Err(e);
        }
        let Some(sha) = ours else {
            return Ok(CheckoutOutcome::default());
        };
        let sel = self.resolve_stash_selector(&sha).await?;
        let (ok, out, err, code) = self.run_raw_retry_lock(&["stash", "pop", &sel]).await?;
        if ok {
            return Ok(CheckoutOutcome {
                stashed: true,
                pop_conflicted: false,
            });
        }
        if out.contains("CONFLICT") || err.contains("CONFLICT") {
            return Ok(CheckoutOutcome {
                stashed: true,
                pop_conflicted: true,
            });
        }
        // Not a conflict: git restores untracked files BEFORE applying tracked
        // changes, so the stash entry is still intact and the tree is clean on
        // `branch`. Say exactly that instead of a bare 502.
        let line = match upstream_err(&err, &out, code) {
            Error::Conflict(m) | Error::Upstream(m) => m,
            e => e.to_string(),
        };
        Err(Error::Conflict(format!(
            "switched to {branch}, but restoring your stashed changes failed: {line} — run `git stash pop`"
        )))
    }

    /// Refuse an empty or control-character path in a caller's path list.
    fn guard_paths(paths: &[String]) -> Result<()> {
        paths.iter().try_for_each(|p| Self::guard_path(p))
    }

    /// Every path is a LITERAL file name ([`GitCmd::paths`]): staging
    /// `app/[id]/page.tsx` must not also stage `app/d/page.tsx`.
    pub async fn stage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Err(Error::Invalid("no paths to stage".into()));
        }
        Self::guard_paths(paths)?;
        self.exec_locked(&GitCmd::write(&["add"]).paths(paths))
            .await
    }

    pub async fn unstage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Err(Error::Invalid("no paths to unstage".into()));
        }
        Self::guard_paths(paths)?;
        let mut expanded = paths.to_vec();
        for change in self.status().await?.changes {
            if paths.contains(&change.path) && change.kind == "renamed" {
                if let Some(original) = change.orig_path {
                    expanded.push(original);
                }
            }
        }
        expanded.sort();
        expanded.dedup();
        // Before the first commit there is no HEAD to restore the index FROM
        // (`restore --staged` dies "could not resolve 'HEAD'"); `reset` treats
        // an unborn HEAD as the empty tree, i.e. un-adds the paths.
        let cmd = if self.head_exists().await {
            GitCmd::write(&["restore", "--staged"])
        } else {
            GitCmd::write(&["reset", "-q"])
        };
        self.exec_locked(&cmd.paths(&expanded)).await
    }

    /// True once HEAD resolves to a commit (false on an unborn branch).
    async fn head_exists(&self) -> bool {
        matches!(
            self.exec(&GitCmd::read(&["rev-parse", "-q", "--verify", "HEAD^{commit}"]), None)
                .await,
            Ok((true, ..))
        )
    }

    /// Discard all working-tree + staged changes for `paths`, reverting them to
    /// their HEAD state. New files (untracked/added) are removed entirely;
    /// everything else (modified/deleted/renamed/conflicted) is restored from
    /// HEAD. Destructive and irreversible — the UI confirms first.
    pub async fn discard(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Err(Error::Invalid("no paths to discard".into()));
        }
        Self::guard_paths(paths)?;
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
        if restore.is_empty() && remove.is_empty() {
            // Nothing the caller named has a change any more (a stale list, or
            // a name status never reported). Saying "Discarded" for a no-op is
            // how a silently-failed discard used to look like success.
            return Err(Error::Conflict(
                "nothing to discard — none of these files has changes any more; refresh".into(),
            ));
        }
        // Every path below is a LITERAL name ([`GitCmd::paths`]): as a
        // pathspec, `app/[id]/page.tsx` also matched `app/d/page.tsx` (whose
        // edits `restore` reverted) and `app/i/page.tsx` (which `clean` deleted).
        if !restore.is_empty() {
            self.exec_locked(
                &GitCmd::write(&["restore", "--staged", "--worktree", "--source=HEAD"])
                    .paths(&restore),
            )
            .await?;
        }
        if !remove.is_empty() {
            // Unstage first (a staged-new file → untracked), then `clean` removes
            // the untracked files/dirs. `reset` is a no-op for already-untracked.
            let _ = self
                .exec_locked(&GitCmd::write(&["reset", "-q"]).paths(&remove))
                .await;
            self.exec_locked(&GitCmd::write(&["clean", "-fdq"]).paths(&remove))
                .await?;
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
            self.run_locked(&["commit", "--amend", "--no-edit"]).await?;
        } else {
            let mut args = vec!["commit", "-m", message];
            if amend {
                args.push("--amend");
            }
            self.run_locked(&args).await?;
        }
        let sha = self.run_read(&["rev-parse", "HEAD"]).await?;
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
                Self::guard_ref(b)?;
                let askpass = match &token {
                    Some(t) => Some(AskPass::new(t)?),
                    None => None,
                };
                let envs = askpass.as_ref().map(AskPass::envs).unwrap_or_default();
                let (ok, stdout, stderr, code) = self
                    .run_raw_class(&["push", "origin", b], &envs, SpawnClass::Remote)
                    .await?;
                if ok {
                    return Ok(combine_push_output(&stdout, &stderr));
                }
                // First push of a fresh branch: set the upstream explicitly.
                if stderr.contains("has no upstream branch") || stderr.contains("--set-upstream") {
                    let (ok2, stdout2, stderr2, code2) = self
                        .run_raw_class(
                            &["push", "--set-upstream", "origin", b],
                            &envs,
                            SpawnClass::Remote,
                        )
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

        let (ok, stdout, stderr, code) = self
            .run_raw_class(&["push"], &envs, SpawnClass::Remote)
            .await?;
        if ok {
            return Ok(combine_push_output(&stdout, &stderr));
        }
        if stderr.contains("has no upstream branch") || stderr.contains("--set-upstream") {
            let branch = self.current_branch().await?;
            let (ok2, stdout2, stderr2, code2) = self
                .run_raw_class(
                    &["push", "--set-upstream", "origin", &branch],
                    &envs,
                    SpawnClass::Remote,
                )
                .await?;
            if ok2 {
                return Ok(combine_push_output(&stdout2, &stderr2));
            }
            return Err(upstream_err(&stderr2, &stdout2, code2));
        }
        Err(upstream_err(&stderr, &stdout, code))
    }

    /// `git pull --no-rebase`, treating a CONFLICTING merge as a normal outcome
    /// rather than a failure: git exits non-zero, but the pull itself succeeded
    /// — the fetch landed and a merge is now in progress with unmerged paths.
    /// Reporting that as an error (what `run_remote` does) left the repo mid-
    /// merge while the UI only said "Pull failed", so the conflicted files
    /// surfaced as unexplained WIP changes with no way to resolve them.
    ///
    /// Genuine failures (auth, network, no upstream, dirty-tree refusal) still
    /// come back as `Err`.
    pub async fn pull_outcome(&self, token: Option<String>) -> Result<PullOutcome> {
        let (ok, stdout, stderr, code) =
            self.run_remote_raw(&["pull", "--no-rebase"], token).await?;
        let mut output = strip_noise(&stdout);
        let err = strip_noise(&stderr);
        if !err.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&err);
        }
        if ok {
            return Ok(PullOutcome {
                output,
                conflicted_files: Vec::new(),
            });
        }
        // Non-zero: a conflicted merge (unmerged paths present) is an expected
        // outcome; anything else is a real failure.
        let conflicted = self.conflicted_paths().await.unwrap_or_default();
        let combined = format!("{stdout}\n{stderr}");
        let is_conflict = !conflicted.is_empty()
            && (combined.contains("CONFLICT")
                || combined.contains("Automatic merge failed")
                || self.is_merging().await);
        if is_conflict {
            return Ok(PullOutcome {
                output,
                conflicted_files: conflicted,
            });
        }
        Err(upstream_err(&stderr, &stdout, code))
    }

    /// `git pull --no-rebase` where a conflict IS a failure — callers that
    /// continue mutating the tree afterwards (popping a stash, say) must not
    /// proceed onto a half-merged working tree.
    pub async fn pull(&self, token: Option<String>) -> Result<String> {
        let out = self.pull_outcome(token).await?;
        if !out.conflicted_files.is_empty() {
            return Err(Error::Conflict(format!(
                "pull merged with conflicts in {} file(s): {}",
                out.conflicted_files.len(),
                out.conflicted_files.join(", ")
            )));
        }
        Ok(out.output)
    }

    /// Pull with an automatic stash → pull → pop around a dirty tree (the
    /// opt-in `auto_stash` pull the UI offers when a plain pull is refused with
    /// "commit or stash first"). Returns the pull outcome plus a human note
    /// about what happened to the stashed changes.
    ///
    /// Contract mirrors [`Self::merge_branch`]'s auto-stash: a pull that merges
    /// with CONFLICTS leaves the changes STASHED (popping onto a half-merged
    /// tree would bury them) and says so; a failed pull pops the stash back so
    /// the tree is exactly as it was.
    pub async fn pull_autostash(
        &self,
        token: Option<String>,
    ) -> Result<(PullOutcome, Option<String>)> {
        let dirty = !self
            .run(&["status", "--porcelain"])
            .await?
            .trim()
            .is_empty();
        if !dirty {
            return Ok((self.pull_outcome(token).await?, None));
        }
        let sha = self.stash_save_sha().await?;
        match self.pull_outcome(token).await {
            Ok(out) if out.conflicted_files.is_empty() => {
                let note = self.pop_after_merge(&sha).await;
                Ok((out, note))
            }
            Ok(out) => Ok((
                out,
                Some(
                    "The pull merged with conflicts. Your uncommitted changes stay stashed — \
                     resolve the conflicts and commit, then restore them from `git stash list`."
                        .into(),
                ),
            )),
            Err(e) => {
                // The refused pull never touched the tree — restore it (by
                // SHA), or say where the changes are if that fails.
                let origin = self.head_label().await.unwrap_or_default();
                Err(self.restore_autostash(e, &sha, &origin).await)
            }
        }
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
        let (stdout, stderr) = self.run_env_class(args, &envs, SpawnClass::Remote).await?;
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
    pub(crate) async fn run_remote_raw(
        &self,
        args: &[&str],
        token: Option<String>,
    ) -> Result<(bool, String, String, Option<i32>)> {
        let askpass = match token {
            Some(t) => Some(AskPass::new(&t)?),
            None => None,
        };
        let envs = askpass.as_ref().map(AskPass::envs).unwrap_or_default();
        self.run_raw_class(args, &envs, SpawnClass::Remote).await
    }

    // -- graph context-menu ops (commit / branch / tag) ---------------------

    /// Cherry-pick a single commit onto the current branch. A CONFLICTING pick
    /// is a normal outcome, not an error: git leaves CHERRY_PICK_HEAD + the
    /// unmerged paths, `status()` reports `op_in_progress:"cherry_pick"` with
    /// the conflicted files, and the UI routes into the resolver. (Previously a
    /// conflicting pick surfaced as a bare 502 AND stranded the sequencer state
    /// with no way to continue or abort it from the app.)
    pub async fn cherry_pick(&self, sha: &str) -> Result<()> {
        self.op_conflict_as_result(&["cherry-pick", "--end-of-options", sha])
            .await
    }

    /// Revert a single commit, committing the inverse with `--no-edit`. A
    /// conflicting revert is a normal outcome (see [`Self::cherry_pick`]).
    pub async fn revert(&self, sha: &str) -> Result<()> {
        self.op_conflict_as_result(&["revert", "--no-edit", "--end-of-options", sha])
            .await
    }

    /// Run a sequencer op (cherry-pick/revert) treating a conflict as success —
    /// the caller returns the fresh status, which now carries the op + the
    /// conflicted paths. Anything else non-zero is a classified error.
    pub(crate) async fn op_conflict_as_result(&self, args: &[&str]) -> Result<()> {
        let (ok, stdout, stderr, code) = self.run_raw(args, &[]).await?;
        if ok {
            return Ok(());
        }
        let combined = format!("{stdout}\n{stderr}");
        let conflicted = combined.contains("CONFLICT")
            || combined.contains("could not apply")
            || combined.contains("could not revert")
            || !self.conflicted_paths().await.unwrap_or_default().is_empty();
        if conflicted {
            return Ok(());
        }
        Err(upstream_err(&stderr, &stdout, code))
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
            && self
                .verify_commit_ref(&format!("refs/remotes/{remote}"))
                .await
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
            vec!["checkout", "-b", name, "--end-of-options"]
        } else {
            vec!["branch", "--end-of-options", name]
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
        self.run(&[
            "branch",
            if force { "-D" } else { "-d" },
            "--end-of-options",
            name,
        ])
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
        Self::guard_ref(name)?;
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
        self.run(&["branch", "-m", "--end-of-options", from, to])
            .await?;
        Ok(())
    }

    /// Create a tag at `sha`: annotated (`-a … -m <msg>`) when `message` is
    /// present, lightweight otherwise.
    pub async fn create_tag(&self, name: &str, sha: &str, message: Option<&str>) -> Result<()> {
        match message.filter(|m| !m.is_empty()) {
            Some(msg) => {
                self.run(&["tag", "-a", "-m", msg, "--end-of-options", name, sha])
                    .await?;
            }
            None => {
                self.run(&["tag", "--end-of-options", name, sha]).await?;
            }
        }
        Ok(())
    }

    /// Push a single tag to `origin` (`git push origin refs/tags/<name>`).
    /// Returns the combined push output.
    pub async fn push_tag(&self, name: &str, token: Option<String>) -> Result<String> {
        Self::guard_ref(name)?;
        let refspec = format!("refs/tags/{name}");
        self.run_remote(&["push", "origin", &refspec], token).await
    }

    /// Delete a local tag (`git tag -d <name>`).
    pub async fn delete_tag(&self, name: &str) -> Result<()> {
        self.run(&["tag", "-d", "--end-of-options", name]).await?;
        Ok(())
    }

    /// Delete a tag on `origin` (`git push origin --delete refs/tags/<name>`).
    /// Returns the combined push output.
    pub async fn delete_remote_tag(&self, name: &str, token: Option<String>) -> Result<String> {
        Self::guard_ref(name)?;
        let refspec = format!("refs/tags/{name}");
        self.run_remote(&["push", "origin", "--delete", &refspec], token)
            .await
    }

    /// Run a mutating git command with a short bounded retry when the index is
    /// locked by a CONCURRENT git process. Agent sessions run git in the same
    /// repos the user clicks around in, so "Unable to create '….git/index.lock':
    /// File exists" is a transient collision, not a real failure — it surfaced
    /// as stash/stage buttons "sometimes erroring" for no visible reason.
    pub(crate) async fn run_raw_retry_lock(
        &self,
        args: &[&str],
    ) -> Result<(bool, String, String, Option<i32>)> {
        for attempt in 1u64..=3 {
            let res = self.run_raw(args, &[]).await?;
            if res.0 || attempt == 3 || !res.2.contains("index.lock") {
                return Ok(res);
            }
            tokio::time::sleep(std::time::Duration::from_millis(300 * attempt)).await;
        }
        unreachable!("loop returns on its final attempt")
    }

    /// [`Self::run_raw_retry_lock`] for a command whose only interesting result
    /// is success: a non-zero exit is classified through `upstream_err` exactly
    /// as [`Self::run`] would, but a concurrent agent's `index.lock` costs a
    /// 300/600 ms retry instead of a spurious 409.
    pub(crate) async fn run_locked(&self, args: &[&str]) -> Result<()> {
        let (ok, out, err, code) = self.run_raw_retry_lock(args).await?;
        if !ok {
            let e = upstream_err(&err, &out, code);
            tracing::warn!(
                repo = %self.repo_path.display(),
                args = ?args,
                code = code,
                "git failed: {e}"
            );
            return Err(e);
        }
        Ok(())
    }

    /// `git stash push --include-untracked`: stash tracked changes AND
    /// untracked files. Without `-u` a tree whose only changes were NEW files
    /// "stashed" successfully while stashing nothing — the button looked dead.
    /// A genuinely clean tree errors explicitly for the same reason: git exits
    /// 0 with "No local changes to save", and swallowing that toasted a
    /// success that did nothing.
    pub async fn stash_save(&self) -> Result<String> {
        let (ok, out, err, code) = self
            .run_raw_retry_lock(&["stash", "push", "--include-untracked"])
            .await?;
        if !ok {
            return Err(upstream_err(&err, &out, code));
        }
        if out.contains("No local changes to save") || err.contains("No local changes to save") {
            return Err(Error::Invalid(
                "nothing to stash — the working tree has no local changes".into(),
            ));
        }
        Ok(out.trim().to_string())
    }

    pub async fn stash_pop(&self) -> Result<String> {
        let (ok, out, err, code) = self.run_raw_retry_lock(&["stash", "pop"]).await?;
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
            .run_read(&[
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
        let (ok, out, err, code) = self.run_raw_retry_lock(&["stash", "apply", &sel]).await?;
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
    pub(crate) async fn is_merging(&self) -> bool {
        let (ok, _, _, _) = self
            .run_raw(&["rev-parse", "-q", "--verify", "MERGE_HEAD"], &[])
            .await
            .unwrap_or((false, String::new(), String::new(), None));
        ok
    }

    /// Conflicted paths from a fresh status (porcelain v2 `u` entries).
    pub(crate) async fn conflicted_paths(&self) -> Result<Vec<String>> {
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
    pub(crate) async fn working_dirty(&self) -> Result<bool> {
        let st = self.status().await?;
        Ok(st
            .changes
            .iter()
            .any(|c| (c.staged || c.unstaged) && c.kind != "untracked"))
    }

    /// Pop the stash after a clean merge. Returns a human note: a confirmation on
    /// a clean pop, or a warning if the pop conflicted (git KEEPS the stash in
    /// that case, so the user's work is never lost).
    ///
    /// `sha` is the auto-stash's own commit ([`Self::stash_save_sha`]): the pop
    /// is addressed by it, never by `stash@{0}` — an agent in the same repo
    /// may have pushed its own stash on top in the meantime.
    pub(crate) async fn pop_after_merge(&self, sha: &str) -> Option<String> {
        match self.pop_stash_sha(sha).await {
            Ok(out) if !out.contains("CONFLICT") => {
                Some("Your stashed changes were restored.".into())
            }
            _ => Some(
                "Merge succeeded, but restoring your stashed changes hit a conflict — \
                 they're preserved in `git stash`; resolve the working tree and run \
                 `git stash pop` manually."
                    .into(),
            ),
        }
    }

    /// [`Self::stash_save`], returning the new entry's commit SHA so the
    /// matching restore can address THIS stash rather than whatever is on top.
    pub(crate) async fn stash_save_sha(&self) -> Result<String> {
        self.stash_save().await?;
        let sha = self
            .run_read(&["rev-parse", "-q", "--verify", "refs/stash"])
            .await?
            .trim()
            .to_string();
        if sha.is_empty() {
            return Err(Error::Internal("stash saved but refs/stash is unreadable".into()));
        }
        Ok(sha)
    }

    /// `git stash pop` of the entry whose commit is `sha`. A CONFLICTING pop
    /// is Ok (the stash is applied with markers and git KEEPS the entry), as
    /// in [`Self::stash_pop`].
    pub(crate) async fn pop_stash_sha(&self, sha: &str) -> Result<String> {
        let sel = self.resolve_stash_selector(sha).await?;
        let (ok, out, err, code) = self.run_raw_retry_lock(&["stash", "pop", &sel]).await?;
        if ok || out.contains("CONFLICT") {
            return Ok(out.trim().to_string());
        }
        Err(upstream_err(&err, &out, code))
    }

    /// Put the user back where an auto-stashing operation found them after it
    /// FAILED: switch back to `origin` (the branch or detached SHA they were
    /// on, when the failure left HEAD elsewhere) and pop the auto-stash `sha`
    /// there. Returns `err`, annotated when the changes could not be
    /// restored so the message says where they are instead of implying they
    /// were lost.
    pub(crate) async fn restore_autostash(&self, err: Error, sha: &str, origin: &str) -> Error {
        let short = &sha[..sha.len().min(12)];
        let here = self.head_label().await;
        let mut back = true;
        if here.as_deref() != Some(origin) {
            back = self.checkout(origin, false).await.is_ok();
        }
        let popped = if back {
            Some(self.pop_stash_sha(sha).await)
        } else {
            None
        };
        let kept = match popped {
            Some(Ok(out)) if !out.contains("CONFLICT") => return err,
            Some(Ok(_)) => format!(
                "restoring your uncommitted changes conflicted: resolve the marked files; \
                 the stash is kept in `git stash list` (stash {short})"
            ),
            _ => format!(
                "your uncommitted changes are kept in `git stash list` (stash {short}) — \
                 run `git stash apply {short}` on {origin} to restore them"
            ),
        };
        match err {
            Error::Conflict(m) => Error::Conflict(format!("{m} — {kept}")),
            Error::Upstream(m) => Error::Upstream(format!("{m} — {kept}")),
            Error::Invalid(m) => Error::Invalid(format!("{m} — {kept}")),
            Error::NotFound(m) => Error::NotFound(format!("{m} — {kept}")),
            Error::Internal(m) => Error::Internal(format!("{m} — {kept}")),
            other => Error::Conflict(format!("{other} — {kept}")),
        }
    }

    /// Where HEAD is, in a form `checkout` can return to: the branch name, or
    /// the commit SHA when detached. `None` when HEAD can't be read.
    pub(crate) async fn head_label(&self) -> Option<String> {
        if let Some(b) = self.symbolic_head().await {
            return Some(b);
        }
        self.run_read(&["rev-parse", "-q", "--verify", "HEAD"])
            .await
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Dry-run a merge of `source` into `target` via `git merge-tree --write-tree`
    /// (writes only to the object DB — the index and working tree are NEVER
    /// touched). Lets callers warn about conflicts BEFORE starting a real merge.
    pub async fn merge_preview(&self, source: &str, target: &str) -> Result<MergePreview> {
        Self::guard_ref(source)?;
        Self::guard_ref(target)?;
        // No-op merge: source already contained in target.
        if self.is_ancestor_of(source, target).await.unwrap_or(false) {
            return Ok(MergePreview {
                conflicts: false,
                conflicted_files: Vec::new(),
                up_to_date: true,
            });
        }
        // `--no-messages -z`: without them the "Auto-merging x" / "CONFLICT
        // (content): …" message block that follows the name list was read as
        // more conflicted FILE names, and a quoted name never matched.
        let (ok, stdout, _stderr, code) = self
            .run_raw_class(
                &[
                    "merge-tree",
                    "--write-tree",
                    "--name-only",
                    "--no-messages",
                    "-z",
                    target,
                    source,
                ],
                &[],
                SpawnClass::LocalRead,
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
        // Output: the tree OID record, then one record per conflicted file.
        let mut conflicted_files: Vec<String> = Vec::new();
        for name in nul_records(&stdout).skip(1) {
            if !conflicted_files.iter().any(|n| n == name) {
                conflicted_files.push(name.to_string());
            }
        }
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
        // Refuse to START a merge over an unfinished one (or a rebase /
        // cherry-pick / revert). The old "already merging" exemption then
        // checked out `target`, which git refuses with "you need to resolve
        // your current index first" — surfaced as a raw 502. Continuing an
        // in-progress merge goes through merge/commit, never through here.
        if let Some(op) = self.op_in_progress().await {
            return Err(Error::Conflict(format!(
                "a {} is already in progress — resolve its conflicts or abort it first",
                op.replace('_', "-"),
            )));
        }

        // Dirty-tree handling: either auto-stash, or refuse.
        if !self.working_dirty().await? {
            return self.merge_branch_inner(source, target, strategy, None).await;
        }
        if !auto_stash {
            return Err(Error::Conflict(
                "working tree has uncommitted changes; commit or stash first".into(),
            ));
        }
        // Where the user is BEFORE the stash: any failure from here on puts
        // them back there with their changes. The old code returned early when
        // the switch to `target` failed (routine: `target` checked out in an
        // agent's worktree) and every tracked + untracked change silently
        // stayed in the stash, the error not even mentioning it.
        let origin = self
            .head_label()
            .await
            .ok_or_else(|| Error::Conflict("cannot read HEAD — nothing was stashed".into()))?;
        let sha = self.stash_save_sha().await?;
        match self
            .merge_branch_inner(source, target, strategy, Some(&sha))
            .await
        {
            Ok(r) => Ok(r),
            Err(e) => Err(self.restore_autostash(e, &sha, &origin).await),
        }
    }

    /// [`Self::merge_branch`] after the dirty-tree decision. `stash` is the
    /// auto-stash taken for this merge: popped (by SHA) after a clean merge,
    /// kept with a note on conflicts. An `Err` pops nothing — the caller
    /// restores the stash on the original branch.
    async fn merge_branch_inner(
        &self,
        source: &str,
        target: &str,
        strategy: LocalMergeStrategy,
        stash: Option<&str>,
    ) -> Result<MergeResult> {
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
                "--end-of-options",
                source,
            ],
            LocalMergeStrategy::Ff => {
                vec![
                    "-c",
                    "merge.conflictStyle=diff3",
                    "merge",
                    "--no-edit",
                    "--end-of-options",
                    source,
                ]
            }
            LocalMergeStrategy::FfOnly => {
                vec![
                    "-c",
                    "merge.conflictStyle=diff3",
                    "merge",
                    "--ff-only",
                    "--end-of-options",
                    source,
                ]
            }
            LocalMergeStrategy::Squash => {
                vec![
                    "-c",
                    "merge.conflictStyle=diff3",
                    "merge",
                    "--squash",
                    "--end-of-options",
                    source,
                ]
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
                let tree = self
                    .run(&["rev-parse", "HEAD^{tree}"])
                    .await?
                    .trim()
                    .to_string();
                let msg = format!("Merge branch '{source}' into {target}");
                let new_commit = self
                    .run(&[
                        "commit-tree",
                        &tree,
                        "-p",
                        &target_head,
                        "-p",
                        &source_head,
                        "-m",
                        &msg,
                    ])
                    .await?
                    .trim()
                    .to_string();
                // Advance the checked-out target branch onto the new merge commit
                // (it descends from target_head, so this is a clean fast-forward).
                self.run(&["merge", "--ff-only", &new_commit]).await?;
                let note = match stash {
                    Some(sha) => self.pop_after_merge(sha).await,
                    None => None,
                };
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
            let note = match stash {
                Some(sha) => self.pop_after_merge(sha).await,
                None => None,
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
            let note = stash.map(|sha| {
                let short = &sha[..sha.len().min(12)];
                format!(
                    "Your uncommitted changes were stashed before the merge, which then \
                     conflicted. Resolve the conflicts and commit, then restore your changes \
                     from `git stash list` (stash {short})."
                )
            });
            return Ok(MergeResult {
                status: "conflicts".into(),
                commit: None,
                conflicted_files: conflicted,
                repo_status: self.status().await?,
                note,
            });
        }
        // Hard error — the caller (`merge_branch`) switches back and restores
        // any auto-stash before surfacing it, so nothing is stranded.
        Err(upstream_err(&stderr, &stdout, code))
    }

    /// Current resolvable-operation status: which op is underway (merge /
    /// rebase / cherry-pick / revert), the best-effort source ref, and the
    /// conflicted file list. Conflicted files with NO op (a conflicting
    /// `stash pop`, a conflicted `merge --squash`) still report `merging:true`
    /// so the resolver opens for them too — they used to be invisible.
    pub async fn merge_status(&self) -> Result<MergeConflictStatus> {
        let op = self.op_in_progress().await.map(str::to_string);
        let conflicted_files = self.conflicted_paths().await?;
        if op.is_none() && conflicted_files.is_empty() {
            return Ok(MergeConflictStatus {
                merging: false,
                op: None,
                source: None,
                conflicted_files: Vec::new(),
            });
        }
        let source = match op.as_deref() {
            Some("merge") => self.merge_source().await,
            Some("cherry_pick") => self
                .run(&["rev-parse", "--short", "CHERRY_PICK_HEAD"])
                .await
                .ok()
                .map(|s| format!("cherry-pick {}", s.trim())),
            Some("revert") => self
                .run(&["rev-parse", "--short", "REVERT_HEAD"])
                .await
                .ok()
                .map(|s| format!("revert {}", s.trim())),
            Some("rebase") => Some("rebase".to_string()),
            _ => None,
        };
        Ok(MergeConflictStatus {
            merging: true,
            op,
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
        let (ours_present, theirs_present) = self.conflict_sides(path).await?;
        // Read only a regular file confined to the tree (see
        // `confined_worktree_file`): a conflicted SYMLINK would otherwise be
        // read through to wherever it points and served to a Viewer. Such an
        // entry is shown as binary — resolved by taking a side.
        let readable = match self.confined_worktree_file(path, false).await {
            Ok(_) => true,
            Err(Error::Invalid(_)) => false,
            Err(e) => return Err(e),
        };
        let (bytes, worktree_present) = if !readable {
            (Vec::new(), tokio::fs::symlink_metadata(&abs).await.is_ok())
        } else {
            match tokio::fs::read(&abs).await {
                Ok(bytes) => (bytes, true),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Vec::new(), false),
                Err(e) => return Err(Error::Internal(format!("read {path}: {e}"))),
            }
        };
        let is_binary = !readable || bytes.contains(&0) || std::str::from_utf8(&bytes).is_err();
        Ok(ConflictFile {
            path: path.to_string(),
            is_binary,
            ours_present,
            theirs_present,
            worktree_present,
            trailing_newline: bytes.ends_with(b"\n"),
            segments: if is_binary {
                Vec::new()
            } else {
                crate::parse::parse_conflict_segments(&String::from_utf8_lossy(&bytes))
            },
        })
    }

    /// Index stages distinguish a deleted side from an empty file. Read them
    /// before taking a side so modify/delete and binary conflicts share one flow.
    ///
    /// Doubles as THE "is this exact path conflicted right now" gate for every
    /// resolver write: only entries whose name equals `path` byte for byte
    /// count (a literal pathspec, so `*` or a directory prefix can't borrow
    /// another file's conflict).
    async fn conflict_sides(&self, path: &str) -> Result<(bool, bool)> {
        Self::guard_path(path)?;
        let entries = self
            .exec_text(&GitCmd::read(&["ls-files", "--unmerged", "-z"]).paths([path]))
            .await?;
        let stages: Vec<&str> = nul_records(&entries)
            .filter_map(|entry| entry.split_once('\t'))
            .filter(|(_, name)| *name == path)
            .filter_map(|(meta, _)| meta.split_whitespace().nth(2))
            .collect();
        if stages.is_empty() {
            return Err(Error::Conflict(
                "this file is no longer conflicted — refresh first".into(),
            ));
        }
        Ok((stages.contains(&"2"), stages.contains(&"3")))
    }

    /// Explicit whole-file actions preserve binary bytes and represent deletion
    /// as removal, never as an empty replacement file.
    pub async fn resolve_take_side(&self, path: &str, side: &str) -> Result<()> {
        let (ours, theirs) = self.conflict_sides(path).await?;
        let delete = match side {
            "ours" => !ours,
            "theirs" => !theirs,
            "delete" => true,
            "keep" => false,
            other => {
                return Err(Error::Invalid(format!(
                    "bad side: {other} (ours|theirs|keep|delete)"
                )))
            }
        };
        if delete {
            self.exec_locked(&GitCmd::write(&["rm", "--force"]).paths([path]))
                .await?;
        } else {
            if side != "keep" {
                let flag = if side == "ours" { "--ours" } else { "--theirs" };
                self.exec_locked(&GitCmd::write(&["checkout", flag]).paths([path]))
                    .await?;
            } else if !self.safe_join(path)?.exists() {
                return Err(Error::Conflict(
                    "the working file is absent; choose Delete instead".into(),
                ));
            }
            self.exec_locked(&GitCmd::write(&["add"]).paths([path]))
                .await?;
        }
        Ok(())
    }

    /// Write the fully-resolved content of `path` and stage it.
    ///
    /// Security boundary (the resolver is reachable by any Editor and by
    /// agents through the local API): this writes ONLY a file that is
    /// conflicted right now, and only INSIDE the working tree. Unchecked, a
    /// `{path:".git/config"}` body planted `core.fsmonitor` — a command the
    /// daemon's own next `git status` runs as the user. So: the exact path
    /// must have unmerged index entries ([`Self::conflict_sides`]), and
    /// [`Self::confined_worktree_file`] rejects `.git` components, `..`,
    /// absolute paths, a symlinked final component and any parent that
    /// resolves (symlinks followed) outside the tree or into the git dir.
    pub async fn write_resolution(&self, path: &str, content: &str) -> Result<()> {
        self.conflict_sides(path).await?;
        let abs = self.confined_worktree_file(path, true).await?;
        tokio::fs::write(&abs, content)
            .await
            .map_err(|e| Error::Internal(format!("write {path}: {e}")))?;
        self.exec_locked(&GitCmd::write(&["add"]).paths([path]))
            .await?;
        Ok(())
    }

    /// The absolute location of repo-relative `rel`, proven to be a regular
    /// file slot INSIDE this working tree and outside every git dir.
    ///
    /// Checked on the real filesystem, symlinks followed: the nearest existing
    /// ancestor of `rel` is canonicalized and must stay under the canonical
    /// work-tree root and out of the canonical `$GIT_DIR`; with `create_dirs`
    /// the missing directories below it (plain names — they did not exist)
    /// are then created. A final component that is a symlink is refused, as
    /// a write or read would follow it wherever it points.
    async fn confined_worktree_file(&self, rel: &str, create_dirs: bool) -> Result<PathBuf> {
        let abs = self.safe_join(rel)?;
        let refuse = |why: &str| Error::Invalid(format!("refusing path {rel}: {why}"));
        if Path::new(rel)
            .components()
            .any(|c| matches!(c, std::path::Component::Normal(n) if is_git_dir_name(n)))
        {
            return Err(refuse("it is inside the git directory"));
        }
        let io = |e: std::io::Error| Error::Internal(format!("resolve {rel}: {e}"));
        let root = tokio::fs::canonicalize(&self.repo_path).await.map_err(io)?;
        let git_dir = match self.git_dir().await {
            Some(gd) => tokio::fs::canonicalize(&gd).await.ok(),
            None => None,
        };
        // Walk up to the nearest ancestor that exists and resolve it.
        let parent = abs.parent().ok_or_else(|| refuse("no parent directory"))?;
        let mut existing = parent.to_path_buf();
        loop {
            match tokio::fs::symlink_metadata(&existing).await {
                Ok(_) => break,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if !existing.pop() {
                        return Err(refuse("no existing ancestor"));
                    }
                }
                Err(e) => return Err(io(e)),
            }
        }
        let real = tokio::fs::canonicalize(&existing).await.map_err(io)?;
        if !real.starts_with(&root) {
            return Err(refuse("it resolves outside the working tree"));
        }
        if git_dir.as_ref().is_some_and(|gd| real.starts_with(gd)) {
            return Err(refuse("it resolves into the git directory"));
        }
        if let Ok(inner) = real.strip_prefix(&root) {
            if inner
                .components()
                .any(|c| matches!(c, std::path::Component::Normal(n) if is_git_dir_name(n)))
            {
                return Err(refuse("it resolves into the git directory"));
            }
        }
        if create_dirs && existing.as_path() != parent {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::Internal(format!("mkdir for {rel}: {e}")))?;
        }
        match tokio::fs::symlink_metadata(&abs).await {
            Ok(m) if m.file_type().is_symlink() => Err(refuse(
                "it is a symlink — take a side instead of writing through it",
            )),
            Ok(m) if m.is_dir() => Err(refuse("it is a directory")),
            _ => Ok(abs),
        }
    }

    /// Conclude the in-progress operation: commit a merge, `--continue` a
    /// rebase / cherry-pick / revert, or commit a staged squash. Fails with a
    /// 409 when conflicts remain, and when there is nothing to conclude —
    /// `git commit --no-edit` on a clean tree used to fall through here and
    /// surface "nothing to commit" as a 502.
    pub async fn merge_commit(&self, message: Option<String>) -> Result<MergeResult> {
        if !self.conflicted_paths().await?.is_empty() {
            return Err(Error::Conflict("unresolved conflicts remain".into()));
        }
        // GIT_EDITOR=true: `--continue` opens an editor for the commit message;
        // headless here, so accept the prepared message as-is.
        let noedit = [("GIT_EDITOR".to_string(), "true".to_string())];
        match self.op_in_progress().await {
            Some("merge") | None => {
                // None = a staged squash (or nothing): require actual staged
                // changes so this can't produce a confusing git error.
                if self.op_in_progress().await.is_none() {
                    let (staged_empty, ..) =
                        self.run_raw(&["diff", "--cached", "--quiet"], &[]).await?;
                    if staged_empty {
                        return Err(Error::Conflict(
                            "no merge in progress and nothing staged — there is nothing to conclude"
                                .into(),
                        ));
                    }
                }
                match message {
                    Some(m) if !m.trim().is_empty() => {
                        self.run(&["commit", "-m", &m]).await?;
                    }
                    _ => {
                        self.run(&["commit", "--no-edit"]).await?;
                    }
                }
            }
            Some("rebase") => {
                self.continue_operation("rebase", &noedit).await?;
            }
            Some("cherry_pick") => {
                self.continue_operation("cherry-pick", &noedit).await?;
            }
            Some("revert") => {
                self.continue_operation("revert", &noedit).await?;
            }
            Some(other) => {
                return Err(Error::Conflict(format!(
                    "cannot conclude an in-progress {other} here"
                )));
            }
        }
        let conflicted_files = self.conflicted_paths().await?;
        if !conflicted_files.is_empty() {
            return Ok(MergeResult {
                status: "conflicts".into(),
                commit: None,
                conflicted_files,
                repo_status: self.status().await?,
                note: None,
            });
        }
        if self.op_in_progress().await.is_some() {
            return Ok(MergeResult {
                status: "paused".into(),
                commit: None,
                conflicted_files: Vec::new(),
                repo_status: self.status().await?,
                note: Some(
                    "Rebase paused for editing. Amend the current commit, then continue.".into(),
                ),
            });
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

    async fn continue_operation(&self, op: &str, env: &[(String, String)]) -> Result<()> {
        match self.run_env(&[op, "--continue"], env).await {
            Ok(_) => Ok(()),
            Err(_) if !self.conflicted_paths().await?.is_empty() => Ok(()),
            Err(e) => Err(e),
        }
    }

    /// Abort the in-progress operation with its own abort verb (merge / rebase
    /// / cherry-pick / revert), or discard a staged squash (`reset --hard`,
    /// only when SQUASH_MSG proves a squash is actually pending). With NOTHING
    /// in progress this is a 409 — the old fallback hard-reset the working
    /// tree, so a stale "Abort" click could destroy all uncommitted work.
    pub async fn merge_abort(&self) -> Result<RepoStatusResp> {
        match self.op_in_progress().await {
            Some("merge") => {
                self.run(&["merge", "--abort"]).await?;
            }
            Some("rebase") => {
                self.run(&["rebase", "--abort"]).await?;
            }
            Some("cherry_pick") => {
                self.run(&["cherry-pick", "--abort"]).await?;
            }
            Some("revert") => {
                self.run(&["revert", "--abort"]).await?;
            }
            _ => {
                if self.squash_pending().await {
                    self.run(&["reset", "--hard", "HEAD"]).await?;
                    // git leaves SQUASH_MSG behind; clear it so a SECOND abort
                    // can't take this destructive branch again.
                    if let Some(gd) = self.git_dir().await {
                        let _ = tokio::fs::remove_file(gd.join("SQUASH_MSG")).await;
                        let _ = tokio::fs::remove_file(gd.join("MERGE_MSG")).await;
                    }
                } else {
                    return Err(Error::Conflict(
                        "no merge in progress — nothing to abort".into(),
                    ));
                }
            }
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

/// `.git` as a path component, compared the way the default case-insensitive
/// APFS/HFS+ volume resolves it: `.GIT/config` IS `.git/config` there.
fn is_git_dir_name(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|s| s.eq_ignore_ascii_case(".git"))
}

/// stderr lines that SSH/git emit as benign chatter — never the reason a command
/// failed. We skip these when choosing the message to surface so the real git
/// error (rejected push, auth failure, …) isn't masked. Newer OpenSSH (9.x/10.x)
/// prints the post-quantum warning to stderr on every non-PQ connection and it
/// does NOT affect the exit status — yet it sorts first, so the old "first
/// non-empty line" logic reported it as the failure.
/// Accept only URLs `git clone` / `git remote add` should ever receive:
/// `https?://`, `ssh://`, `git://` with a host and a path, or scp-like
/// `user@host:path`. Refuses empty, whitespace, control characters, a leading
/// `-` (git would read it as an option), and local paths / `file://` — cloning
/// a local path is a REGISTRATION, and the UI registers those via
/// `POST /repos {path}` instead.
pub fn validate_remote_url(url: &str) -> Result<()> {
    let bad = |why: &str| Err(Error::Invalid(format!("invalid remote url: {why}")));
    let u = url.trim();
    if u.is_empty() {
        return bad("must not be empty");
    }
    if u.starts_with('-') {
        return bad("must not start with '-'");
    }
    if u.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return bad("contains whitespace or a control character");
    }
    if let Some((scheme, rest)) = u.split_once("://") {
        if !matches!(scheme, "http" | "https" | "ssh" | "git") {
            return bad("scheme must be http, https, ssh or git");
        }
        let (host, path) = match rest.split_once('/') {
            Some(p) => p,
            None => return bad("missing path"),
        };
        // `user[:pass]@host[:port]` — only the host half has to be non-empty.
        let host = host.rsplit_once('@').map(|(_, h)| h).unwrap_or(host);
        if host.is_empty() {
            return bad("missing host");
        }
        if path.is_empty() {
            return bad("missing path");
        }
        return Ok(());
    }
    // scp-like `user@host:path`.
    let ok_ident = |v: &str| {
        !v.is_empty()
            && v.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
    };
    let Some((user, rest)) = u.split_once('@') else {
        return bad("expected a URL with a scheme or a user@host:path remote");
    };
    let Some((host, path)) = rest.split_once(':') else {
        return bad("expected a URL with a scheme or a user@host:path remote");
    };
    if !ok_ident(user) || !ok_ident(host) {
        return bad("bad user or host");
    }
    if path.is_empty() || path.starts_with('-') {
        return bad("missing path");
    }
    Ok(())
}

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
        Some(at) => format!(
            "{}{}{}",
            &url[..after],
            &authority[at + 1..],
            &rest[path_start..]
        ),
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
pub(crate) fn strip_noise(s: &str) -> String {
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

/// True when git's own message says the LOCAL repository refused the operation
/// — a dirty tree, an unconfigured upstream, divergent branches, an unfinished
/// merge. Nothing is wrong with the remote (git usually never even dialled it),
/// so these must NOT surface as `Error::Upstream`/502: the UI reports a 502 as
/// "the git provider is unavailable" and raises a global outage banner, which
/// for "please commit your changes before you merge" is simply untrue.
///
/// Matched on git's stable porcelain wording; anything unrecognised keeps the
/// conservative 502 (a genuine network/auth failure looks like nothing here).
pub(crate) fn local_refusal(msg: &str) -> bool {
    const MARKERS: [&str; 24] = [
        "local changes to the following files would be overwritten",
        "would be overwritten by",
        "please commit your changes or stash them",
        "you have unstaged changes",
        "cannot pull with rebase",
        "need to specify how to reconcile divergent branches",
        "there is no tracking information for the current branch",
        "you have not concluded your merge",
        "fix conflicts and run",
        "refusing to merge unrelated histories",
        "not something we can merge",
        "no such ref was fetched",
        // Unresolved index / mid-operation refusals (merge onto a conflicted
        // index, a second cherry-pick, …).
        "you need to resolve your current index first",
        "you have unmerged files",
        "unmerged files",
        "cherry-pick or revert is already in progress",
        "a rebase is in progress",
        "no rebase in progress",
        // A bad/unknown ref is the caller's input, not a provider outage.
        "did not match any file",
        // `checkout <name> --` of a name that is no revision.
        "invalid reference",
        // Nothing to do — e.g. `commit` with an empty index.
        "nothing to commit",
        // A concurrent git process holds the index lock — transient, retryable.
        "index.lock",
        "another git process seems to be running",
        // `pull --ff-only` on a diverged branch — the caller's choice of mode,
        // not an outage.
        "not possible to fast-forward",
    ];
    let lc = msg.to_ascii_lowercase();
    MARKERS.iter().any(|m| lc.contains(m))
}

pub(crate) fn upstream_err(stderr: &str, stdout: &str, code: Option<i32>) -> Error {
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
    // Classify BEFORE wrapping: a local refusal is a 409 the user can act on
    // ("stash your changes"), not a 502 that accuses the remote of being down.
    // Scan every meaningful line, not just `pick` — git prints the reason
    // ("Please commit your changes…") on a different line from the "error:" it
    // leads with.
    let full = meaningful.join("\n");
    if local_refusal(&full) {
        return Error::Conflict(pick.to_string());
    }
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
            (
                "GIT_CONFIG_KEY_0".to_string(),
                "credential.helper".to_string(),
            ),
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
    validate_remote_url(url)?;
    let askpass = match token {
        Some(t) => Some(AskPass::new(t)?),
        None => None,
    };
    let mut cmd = Command::new("git");
    cmd.arg("clone")
        .arg("--progress")
        // `--` so a URL can never be read as an option, and `LC_ALL=C` so the
        // progress/error lines parsed below stay English.
        .arg("--")
        .arg(url)
        .arg(dest)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
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
    #[test]
    fn guard_ref_refuses_option_like_and_control_values() {
        assert!(super::LocalGit::guard_ref("feature/x").is_ok());
        assert!(super::LocalGit::guard_ref("v1.2.3").is_ok());
        assert!(super::LocalGit::guard_ref("HEAD~2").is_ok());
        assert!(super::LocalGit::guard_ref("--upload-pack=touch /tmp/pwn").is_err());
        assert!(super::LocalGit::guard_ref("-c").is_err());
        assert!(super::LocalGit::guard_ref("  --force").is_err());
        assert!(super::LocalGit::guard_ref("main\nrm").is_err());
    }

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
    async fn binary_and_marker_free_conflicts_preserve_exact_bytes() {
        for side in ["ours", "theirs", "keep", "delete"] {
            let (_tmp, dir) = fixture_on_branch("main");
            let git = LocalGit::new(&dir);
            std::fs::write(dir.join("blob"), b"base\0bytes").unwrap();
            sh_git(&dir, &["add", "."]);
            sh_git(&dir, &["commit", "-m", "test: base binary"]);
            sh_git(&dir, &["checkout", "-b", "other"]);
            std::fs::write(dir.join("blob"), b"other\0bytes").unwrap();
            sh_git(&dir, &["commit", "-am", "test: other binary"]);
            sh_git(&dir, &["checkout", "main"]);
            std::fs::write(dir.join("blob"), b"ours\0bytes").unwrap();
            sh_git(&dir, &["commit", "-am", "test: our binary"]);
            let _ = git.run(&["merge", "other"]).await;
            let file = git.conflict_file("blob").await.unwrap();
            assert!(file.is_binary && file.ours_present && file.theirs_present);
            if side == "keep" {
                std::fs::write(dir.join("blob"), b"manual resolution").unwrap();
            }
            git.resolve_take_side("blob", side).await.unwrap();
            if side == "delete" {
                assert!(!dir.join("blob").exists());
            } else {
                let expected: &[u8] = match side {
                    "theirs" => b"other\0bytes",
                    "keep" => b"manual resolution",
                    _ => b"ours\0bytes",
                };
                assert_eq!(std::fs::read(dir.join("blob")).unwrap(), expected);
            }
            assert!(git.conflicted_paths().await.unwrap().is_empty());
            assert!(
                git.resolve_take_side("blob", "delete").await.is_err(),
                "stale action cannot delete a resolved file"
            );
        }
    }

    #[tokio::test]
    async fn resolve_deleted_side_removes_file_and_index_conflict() {
        let (_tmp, dir) = fixture_on_branch("main");
        let git = LocalGit::new(&dir);
        write(&dir, "deleted.txt", "base\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "test: base"]);
        sh_git(&dir, &["checkout", "-b", "remove"]);
        sh_git(&dir, &["rm", "deleted.txt"]);
        sh_git(&dir, &["commit", "-m", "test: remove"]);
        sh_git(&dir, &["checkout", "main"]);
        write(&dir, "deleted.txt", "changed\n");
        sh_git(&dir, &["commit", "-am", "test: modify"]);
        let _ = git.run(&["merge", "remove"]).await;
        git.resolve_take_side("deleted.txt", "theirs")
            .await
            .unwrap();
        assert!(!dir.join("deleted.txt").exists());
        assert!(git.conflicted_paths().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn unstage_rename_restores_both_index_paths() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        git.unstage(&["d.txt".into()]).await.unwrap();
        let staged = git.run(&["diff", "--cached", "--name-only"]).await.unwrap();
        assert_eq!(
            staged.trim(),
            "f.txt",
            "the unrelated addition stays staged; neither rename side does"
        );
        assert!(dir.join("d.txt").exists());
        assert!(!dir.join("c.txt").exists());
    }

    #[tokio::test]
    async fn continuing_rebase_returns_successive_conflicts() {
        let (_tmp, dir) = fixture_on_branch("main");
        let git = LocalGit::new(&dir);
        write(&dir, "sequence.txt", "base\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "test: add base"]);
        sh_git(&dir, &["checkout", "-b", "topic"]);
        for content in ["first\n", "second\n"] {
            write(&dir, "sequence.txt", content);
            sh_git(&dir, &["commit", "-am", "test: change topic"]);
        }
        sh_git(&dir, &["checkout", "main"]);
        write(&dir, "sequence.txt", "upstream\n");
        sh_git(&dir, &["commit", "-am", "test: change upstream"]);
        sh_git(&dir, &["checkout", "topic"]);
        git.rebase("main", false).await.unwrap();
        git.write_resolution("sequence.txt", "resolved differently\n")
            .await
            .unwrap();
        let next = git.merge_commit(None).await.unwrap();
        assert_eq!(next.status, "conflicts");
        assert_eq!(next.conflicted_files, vec!["sequence.txt"]);
        git.merge_abort().await.unwrap();
    }

    /// Stash must take UNTRACKED files too — without `-u`, a tree whose only
    /// changes were new files "stashed" successfully while stashing nothing
    /// (the Stash button looked dead) — and a genuinely clean tree must error
    /// loudly instead of toasting a success that did nothing.
    #[tokio::test]
    async fn stash_includes_untracked_and_clean_tree_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        sh_git(&dir, &["init", "-b", "main"]);
        sh_git(&dir, &["config", "user.email", "otto@test.local"]);
        sh_git(&dir, &["config", "user.name", "Otto Test"]);
        sh_git(&dir, &["config", "commit.gpgsign", "false"]);
        write(&dir, "base.txt", "base\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "init"]);
        let git = LocalGit::new(&dir);

        // Untracked-only tree → the stash really takes it.
        write(&dir, "new.txt", "loose\n");
        git.stash_save().await.expect("untracked-only stash");
        assert!(
            git.status().await.unwrap().changes.is_empty(),
            "tree must be clean after stashing the untracked file"
        );
        assert_eq!(git.stash_list().await.unwrap().len(), 1);

        // Pop restores it.
        git.stash_pop().await.expect("pop");
        assert!(
            dir.join("new.txt").exists(),
            "pop must restore the untracked file"
        );

        // Clean tree → explicit error, not a silent no-op "success".
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "absorb"]);
        let err = git
            .stash_save()
            .await
            .expect_err("clean tree must not 'stash'");
        assert!(
            err.to_string().contains("nothing to stash"),
            "unexpected error: {err}"
        );
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
        let cd = git
            .diff(DiffTarget::Commit(head.clone()), None)
            .await
            .unwrap();
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
    async fn checkout_of_unknown_branch_is_a_conflict_not_an_outage() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        // A bad ref is the caller's input — a 409 with git's own message, never
        // the 502 that raises the provider-outage banner.
        match git.checkout("no-such-branch", false).await {
            Err(Error::Conflict(msg)) => assert!(msg.contains("invalid reference"), "{msg}"),
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    /// Abort with NOTHING in progress must refuse — the old fallback ran
    /// `reset --hard HEAD`, so a stale/double "Abort" click destroyed every
    /// uncommitted change in the tree.
    #[tokio::test]
    async fn merge_abort_with_nothing_in_progress_refuses_and_keeps_work() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        match git.merge_abort().await {
            Err(Error::Conflict(_)) => {}
            other => panic!("expected Conflict, got {other:?}"),
        }
        // The fixture's dirty working tree survived untouched.
        let st = git.status().await.unwrap();
        assert!(st.changes.iter().any(|c| c.path == "a.txt" && c.unstaged));
        assert!(st.changes.iter().any(|c| c.path == "e.txt"));
    }

    /// A conflicting cherry-pick is a normal outcome: status reports the op +
    /// conflicted files, merge/status opens the resolver, and abort uses
    /// `cherry-pick --abort` (not a hard reset).
    #[tokio::test]
    async fn conflicting_cherry_pick_is_visible_and_abortable() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        sh_git(&dir, &["add", "-A"]);
        sh_git(&dir, &["commit", "-m", "clean slate"]);
        sh_git(&dir, &["checkout", "-b", "side"]);
        write(&dir, "a.txt", "side version\n");
        sh_git(&dir, &["commit", "-am", "side change"]);
        sh_git(&dir, &["checkout", "main"]);
        write(&dir, "a.txt", "main version\n");
        sh_git(&dir, &["commit", "-am", "main change"]);
        let side_sha = git.rev_parse("side").await.unwrap();

        git.cherry_pick(&side_sha).await.unwrap();
        let st = git.status().await.unwrap();
        assert_eq!(st.op_in_progress.as_deref(), Some("cherry_pick"));
        assert!(st.changes.iter().any(|c| c.kind == "conflicted"));
        let ms = git.merge_status().await.unwrap();
        assert!(ms.merging);
        assert_eq!(ms.op.as_deref(), Some("cherry_pick"));
        assert!(!ms.conflicted_files.is_empty());

        let st = git.merge_abort().await.unwrap();
        assert_eq!(st.op_in_progress, None);
        assert!(st.changes.is_empty());
    }

    /// Starting a merge over an unfinished merge is refused with a 409 — the
    /// old path checked out `target` and died with git's "resolve your current
    /// index first" as a 502.
    #[tokio::test]
    async fn merge_over_an_unfinished_merge_is_refused() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        sh_git(&dir, &["add", "-A"]);
        sh_git(&dir, &["commit", "-m", "clean slate"]);
        sh_git(&dir, &["checkout", "-b", "side"]);
        write(&dir, "a.txt", "side version\n");
        sh_git(&dir, &["commit", "-am", "side change"]);
        sh_git(&dir, &["checkout", "main"]);
        write(&dir, "a.txt", "main version\n");
        sh_git(&dir, &["commit", "-am", "main change"]);

        let r = git
            .merge_branch("side", "main", LocalMergeStrategy::MergeCommit, false)
            .await
            .unwrap();
        assert_eq!(r.status, "conflicts");
        match git
            .merge_branch("side", "main", LocalMergeStrategy::MergeCommit, false)
            .await
        {
            Err(Error::Conflict(msg)) => assert!(msg.contains("already in progress")),
            other => panic!("expected Conflict, got {other:?}"),
        }
        git.merge_abort().await.unwrap();
    }

    /// A LOCAL refusal (dirty tree, no upstream, divergent branches) is a 409,
    /// not the 502 that makes the UI announce a git-provider outage. A remote
    /// failure — and anything unrecognised — still maps to Upstream.
    #[test]
    fn local_git_refusals_are_conflicts_not_upstream() {
        let local = [
            "error: Your local changes to the following files would be overwritten by merge:\n\tsrc/a.rs\nPlease commit your changes or stash them before you merge.",
            "fatal: Need to specify how to reconcile divergent branches",
            "fatal: There is no tracking information for the current branch.",
            "error: You have not concluded your merge (MERGE_HEAD exists).",
            "fatal: refusing to merge unrelated histories",
            "error: you need to resolve your current index first",
            "error: Merging is not possible because you have unmerged files.",
            "error: pathspec 'no-such-branch' did not match any file(s) known to git",
            "fatal: Unable to create '/r/.git/index.lock': File exists.\nAnother git process seems to be running",
        ];
        for msg in local {
            match upstream_err(msg, "", Some(1)) {
                Error::Conflict(_) => {}
                other => panic!("expected Conflict for {msg:?}, got {other:?}"),
            }
        }
        let remote = [
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled",
            "ssh: connect to host github.com port 22: Connection timed out",
        ];
        for msg in remote {
            match upstream_err(msg, "", Some(1)) {
                Error::Upstream(_) => {}
                other => panic!("expected Upstream for {msg:?}, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn commit_all_if_dirty_sweeps_leftovers_but_not_artifacts() {
        let (_tmp, dir) = fixture();
        let git = LocalGit::new(&dir);
        // The fixture starts dirty (staged rename/new, unstaged, untracked) —
        // the sweep commits all of it.
        assert!(git
            .commit_all_if_dirty("sweep fixture")
            .await
            .unwrap()
            .is_some());
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
        assert!(api
            .envs()
            .iter()
            .any(|(k, v)| { k == "OTTO_GIT_USERNAME" && v == "x-bitbucket-api-token-auth" }));
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

    /// `clone_repo` only ever receives a REMOTE url: a local path is a
    /// registration (`POST /repos {path}`), and letting one through would make
    /// the clone route an arbitrary-path reader. Nothing is spawned — the guard
    /// fires before git.
    #[tokio::test]
    async fn clone_refuses_local_paths_and_option_like_urls() {
        let (_tmp, dir) = fixture();
        let dest_tmp = tempfile::tempdir().unwrap();
        let dest = dest_tmp.path().join("cloned");
        for url in [
            dir.to_str().unwrap(),
            "file:///tmp/x",
            "--upload-pack=touch /tmp/pwn",
        ] {
            let err = clone_repo(url, &dest, None, |_| {}).await.unwrap_err();
            assert!(
                matches!(&err, Error::Invalid(m) if m.contains("invalid remote url")),
                "{url:?} → {err:?}"
            );
        }
        assert!(!dest.exists(), "a refused clone creates nothing");
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
        assert!(
            wt.join("agent_work.txt").exists(),
            "committed file preserved"
        );
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
        git.worktree_add_if_absent(&wt_str, branch, "HEAD")
            .await
            .unwrap();
        let wt_git = LocalGit::new(&wt);
        write(&wt, "agent_work.txt", "turn 1 output\n");
        wt_git.stage(&["agent_work.txt".into()]).await.unwrap();
        let sha = wt_git.commit("agent turn 1", false).await.unwrap();

        // The worktree is pruned, but the branch + its commit must live on.
        git.worktree_remove(&wt_str).await.unwrap();
        assert!(!git.worktree_exists(&wt_str).await);
        assert!(
            git.branch_exists(branch).await,
            "branch survives worktree removal"
        );

        // Turn 2: re-provision with a DIFFERENT base. It must RE-ATTACH the
        // existing branch (preserving the agent commit), not reset to base.
        let created = git
            .worktree_add_if_absent(&wt_str, branch, &base)
            .await
            .unwrap();
        assert!(
            created,
            "re-provisioning a pruned worktree counts as created"
        );
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
        git.worktree_add_if_absent(&wt_str, "swarm/s1/a1", "main")
            .await
            .unwrap();

        let wt_git = LocalGit::new(&wt);
        // No commits on the branch yet → no changes vs base.
        assert!(wt_git.changed_files("main").await.unwrap().is_empty());

        // Commit a new file + modify an existing one.
        write(&wt, "shared.txt", "agent A\n");
        write(&wt, "a.txt", "alpha line 1\nalpha line 2\nalpha A\n");
        wt_git
            .stage(&["shared.txt".into(), "a.txt".into()])
            .await
            .unwrap();
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
        git.worktree_add(&wt_str, "swarm/s/b", "HEAD")
            .await
            .unwrap();
        assert!(git.worktree_exists(&wt_str).await);
        // An unrelated path is not a worktree.
        assert!(
            !git.worktree_exists("/tmp/definitely-not-a-worktree-xyz")
                .await
        );
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
        git.create_branch("feat/a", Some(&head), false)
            .await
            .unwrap();
        assert_eq!(git.current_branch().await.unwrap(), "main");
        assert!(git
            .refs()
            .await
            .unwrap()
            .local
            .iter()
            .any(|b| b.name == "feat/a"));

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
        assert!(!locals
            .iter()
            .any(|b| b.name == "feat/a" || b.name == "feat/b2"));

        // lightweight + annotated tags, then list + delete.
        git.create_tag("v1", &head, None).await.unwrap();
        git.create_tag("v2", &head, Some("release two"))
            .await
            .unwrap();
        let tags = git.refs().await.unwrap().tags;
        assert!(tags.iter().any(|t| t.name == "v1"));
        assert!(tags.iter().any(|t| t.name == "v2"));
        git.delete_tag("v1").await.unwrap();
        assert!(!git
            .refs()
            .await
            .unwrap()
            .tags
            .iter()
            .any(|t| t.name == "v1"));

        // cherry-pick: make a commit on a side branch, pick it onto main.
        git.create_branch("side", Some(&head), true).await.unwrap();
        write(&dir, "picked.txt", "from side\n");
        git.stage(&["picked.txt".into()]).await.unwrap();
        let side_sha = git.commit("side change", false).await.unwrap();
        git.checkout("main", false).await.unwrap();
        git.cherry_pick(&side_sha).await.unwrap();
        assert!(dir.join("picked.txt").exists());
        assert_eq!(
            git.log(1, 0, false).await.unwrap()[0].subject,
            "side change"
        );

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
        let main_head = git
            .run(&["rev-parse", "HEAD"])
            .await
            .unwrap()
            .trim()
            .to_string();
        // `rel` points at an ANCESTOR of main's tip → already contained, so a
        // plain merge would be "Already up to date" with no commit.
        let ancestor = git
            .run(&["rev-parse", "HEAD~1"])
            .await
            .unwrap()
            .trim()
            .to_string();
        sh_git(&dir, &["branch", "rel", &ancestor]);

        let res = git
            .merge_branch("rel", "main", LocalMergeStrategy::MergeCommit, false)
            .await
            .unwrap();

        assert_eq!(
            res.status, "merged",
            "forced a merge even though up to date"
        );
        let new_head = res.commit.expect("a merge commit sha");
        assert_ne!(
            new_head, main_head,
            "main advanced onto the new merge commit"
        );
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
        assert_eq!(
            head, remote_tip,
            "branch starts at the remote tip, not stale HEAD"
        );
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
        git.create_branch("dev2", Some(base.trim()), false)
            .await
            .unwrap();
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
            !git.refs()
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

    /// A pull whose merge conflicts is an OUTCOME, not a failure: `pull_outcome`
    /// returns Ok with the unmerged paths (so the UI can open the resolver),
    /// while the strict `pull` still errors — callers that keep mutating the
    /// tree afterwards (an auto-stash pull's pop) must not run on a
    /// half-merged tree.
    #[tokio::test]
    async fn conflicting_pull_reports_conflicts_instead_of_failing() {
        let tmp = tempfile::tempdir().unwrap();
        // Bare "origin" + a clone that publishes the first commit.
        let origin = tmp.path().join("origin.git");
        std::fs::create_dir(&origin).unwrap();
        sh_git(&origin, &["init", "--bare", "-b", "main"]);

        let seed = tmp.path().join("seed");
        std::fs::create_dir(&seed).unwrap();
        sh_git(&seed, &["init", "-b", "main"]);
        sh_git(&seed, &["config", "user.email", "otto@test.local"]);
        sh_git(&seed, &["config", "user.name", "Otto Test"]);
        sh_git(&seed, &["config", "commit.gpgsign", "false"]);
        write(&seed, "shared.txt", "line1\nline2\nline3\n");
        sh_git(&seed, &["add", "."]);
        sh_git(&seed, &["commit", "-m", "init"]);
        sh_git(
            &seed,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        sh_git(&seed, &["push", "-u", "origin", "main"]);

        // The user's clone.
        let dir = tmp.path().join("work");
        sh_git(
            tmp.path(),
            &["clone", origin.to_str().unwrap(), dir.to_str().unwrap()],
        );
        sh_git(&dir, &["config", "user.email", "otto@test.local"]);
        sh_git(&dir, &["config", "user.name", "Otto Test"]);
        sh_git(&dir, &["config", "commit.gpgsign", "false"]);

        // Upstream moves (adds a file + edits the shared line)…
        write(&seed, "shared.txt", "line1\nUPSTREAM\nline3\n");
        write(&seed, "pulled_only.txt", "from upstream\n");
        sh_git(&seed, &["add", "-A"]);
        sh_git(&seed, &["commit", "-m", "upstream work"]);
        sh_git(&seed, &["push", "origin", "main"]);

        // …and the user edits the SAME line locally → the pull's merge conflicts.
        write(&dir, "shared.txt", "line1\nLOCAL\nline3\n");
        sh_git(&dir, &["commit", "-am", "local work"]);

        let git = LocalGit::new(&dir);
        let outcome = git
            .pull_outcome(None)
            .await
            .expect("a conflicting pull is an outcome, not an error");
        assert_eq!(outcome.conflicted_files, vec!["shared.txt".to_string()]);
        assert!(git.is_merging().await, "the merge is left in progress");

        // The status the endpoint returns carries the conflict, so the UI can
        // route to the resolver: the unmerged path is kind="conflicted" and the
        // cleanly-merged incoming file shows up staged (this is what looked like
        // mystery WIP).
        let st = git.status().await.unwrap();
        assert!(st
            .changes
            .iter()
            .any(|c| c.path == "shared.txt" && c.kind == "conflicted"));
        assert!(st.changes.iter().any(|c| c.path == "pulled_only.txt"));

        // The strict wrapper still fails so stash-popping callers stay safe.
        assert!(git.pull(None).await.is_err());
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
        assert_eq!(
            (r.diff_ref.as_str(), r.branch.as_str()),
            ("master", "master")
        );
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
    async fn resolve_base_skips_a_base_that_is_head() {
        // The PR-Reviewer failure: the run declares the PR's OWN branch as the
        // base and that branch is what's checked out, so `git diff base HEAD`
        // is empty and the review passes 100 without running a reviewer.
        let (_tmp, dir) = fixture_on_branch("develop");
        let git = LocalGit::new(&dir);
        sh_git(&dir, &["checkout", "-b", "feature/X-1"]);
        write(&dir, "b.txt", "change\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "work"]);

        // Attached: base == the current branch → fall through to `develop`.
        let r = git.resolve_base(Some("feature/X-1")).await.unwrap();
        assert_eq!(r.branch, "develop", "base equal to HEAD must not win");
        assert!(!git
            .diff_text_against(&r.diff_ref)
            .await
            .unwrap()
            .trim()
            .is_empty());

        // Detached at the same commit — how the PR-Reviewer checks a PR out.
        let sha = git.rev_parse("HEAD").await.unwrap();
        sh_git(&dir, &["checkout", "--detach", &sha]);
        let r = git.resolve_base(Some("feature/X-1")).await.unwrap();
        assert_eq!(r.branch, "develop");

        // A branch genuinely identical to its base still resolves (nothing
        // narrows): every candidate is HEAD, so the HEAD-equal one is kept.
        sh_git(&dir, &["checkout", "develop"]);
        let r = git.resolve_base(Some("develop")).await.unwrap();
        assert_eq!(r.branch, "develop");
    }

    #[tokio::test]
    async fn resolve_base_error_lists_candidates() {
        // A repo with no commits has no branches at all — nothing can resolve.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        sh_git(&dir, &["init", "-b", "main"]);
        let git = LocalGit::new(&dir);
        let err = git
            .resolve_base(Some("develop"))
            .await
            .unwrap_err()
            .to_string();
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
        sh_git(
            &dir,
            &["worktree", "add", "-b", "feature/x", wt.to_str().unwrap()],
        );
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
            !st.changes
                .iter()
                .any(|c| c.path == "c.txt" || c.path == "d.txt"),
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
        let diff = git
            .diff(DiffTarget::Commit(head.sha.clone()), None)
            .await
            .unwrap();
        assert!(
            diff.files.iter().any(|f| f.path == "merged.txt"),
            "merge diff lists the merged file: {:?}",
            diff.files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
    }

    /// `refs_with_base` flags a branch merged into the base and leaves an
    /// unmerged one alone, while never flagging the base branch or the current
    /// branch itself.
    #[tokio::test]
    async fn refs_flag_branches_merged_into_base() {
        let (_tmp, dir) = fixture();
        // `main` has two commits from the fixture. A `merged` branch forked and
        // folded straight back (fast-forward → contained in main); a `feature`
        // branch adds its own commit and stays ahead.
        sh_git(&dir, &["checkout", "-b", "merged"]);
        // No new commit: `merged` tip == main tip → contained in main.
        sh_git(&dir, &["checkout", "main"]);
        sh_git(&dir, &["checkout", "-b", "feature"]);
        write(&dir, "feat.txt", "feature work\n");
        sh_git(&dir, &["add", "feat.txt"]);
        sh_git(&dir, &["commit", "-m", "feature commit"]);
        sh_git(&dir, &["checkout", "main"]);

        let git = LocalGit::new(&dir);
        let refs = git.refs_with_base(Some("main")).await.unwrap();
        assert_eq!(refs.base_branch.as_deref(), Some("main"));
        let by = |name: &str| {
            refs.local
                .iter()
                .find(|b| b.name == name)
                .unwrap_or_else(|| panic!("branch {name} present"))
                .merged_into_base
        };
        assert!(by("merged"), "a branch contained in base is flagged");
        assert!(!by("feature"), "a branch ahead of base is not flagged");
        assert!(!by("main"), "the base branch is never flagged");
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

    /// A repo with `n` linear commits ("c1".."cn") on `main`.
    fn fixture_n_commits(n: usize) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        sh_git(&dir, &["init", "-b", "main"]);
        sh_git(&dir, &["config", "user.email", "otto@test.local"]);
        sh_git(&dir, &["config", "user.name", "Otto Test"]);
        sh_git(&dir, &["config", "commit.gpgsign", "false"]);
        for i in 1..=n {
            write(&dir, "f.txt", &format!("line {i}\n"));
            sh_git(&dir, &["add", "."]);
            sh_git(&dir, &["commit", "-m", &format!("c{i}")]);
        }
        (tmp, dir)
    }

    /// `limit = 0` means the WHOLE history — the graph must be able to walk back
    /// to the root commit, which a hard `-n` cap silently prevented.
    #[tokio::test]
    async fn log_limit_zero_returns_full_history() {
        let (_tmp, dir) = fixture_n_commits(30);
        let git = LocalGit::new(&dir);

        let all = git.log(0, 0, false).await.unwrap();
        assert_eq!(all.len(), 30, "limit=0 returns every commit");
        assert_eq!(all[0].subject, "c30", "newest first");
        assert_eq!(all[29].subject, "c1", "reaches the root commit");
    }

    /// Paging with skip/limit must tile history exactly once — no gap, no
    /// overlap — since the graph appends each page onto the previous one.
    #[tokio::test]
    async fn log_paging_tiles_history_without_gaps() {
        let (_tmp, dir) = fixture_n_commits(25);
        let git = LocalGit::new(&dir);

        let mut paged = Vec::new();
        let mut skip = 0u32;
        loop {
            let page = git.log(10, skip, true).await.unwrap();
            if page.is_empty() {
                break;
            }
            skip += page.len() as u32;
            paged.extend(page);
        }

        let full = git.log(0, 0, true).await.unwrap();
        assert_eq!(paged.len(), 25, "paging reaches every commit");
        assert_eq!(
            paged.iter().map(|c| &c.sha).collect::<Vec<_>>(),
            full.iter().map(|c| &c.sha).collect::<Vec<_>>(),
            "paged order matches an unpaged read"
        );
    }

    /// Branch/tag rows carry the sha they point at so the UI can jump to a ref
    /// whose commit is NOT in the currently loaded page. Annotated tags must
    /// report the dereferenced COMMIT, never the tag object's own sha.
    #[tokio::test]
    async fn refs_carry_commit_shas_for_branches_and_tags() {
        let (_tmp, dir) = fixture_n_commits(3);
        let git = LocalGit::new(&dir);

        // A lightweight tag and an annotated tag on the FIRST commit.
        let head = git.log(0, 0, false).await.unwrap();
        let root = head.last().unwrap().sha.clone();
        sh_git(&dir, &["tag", "light", &root]);
        sh_git(&dir, &["tag", "-a", "annotated", "-m", "ann", &root]);

        let refs = git.refs().await.unwrap();

        let main = refs.local.iter().find(|b| b.name == "main").unwrap();
        assert_eq!(main.sha, head[0].sha, "branch sha is its tip commit");

        for name in ["light", "annotated"] {
            let t = refs
                .tags
                .iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("tag {name} present"));
            assert_eq!(t.sha, root, "tag {name} resolves to the tagged COMMIT");
        }
    }

    // ── R0: a branch switch never pulls ─────────────────────────────────────

    /// Bare origin + a clone whose `develop` is BOTH ahead of and behind
    /// `origin/develop`, with a dirty overlapping file. The user's bug: this is
    /// exactly the shape where `pull --no-rebase` created an unasked-for merge
    /// commit during a plain switch.
    fn diverged_fixture() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let origin = tmp.path().join("origin.git");
        std::fs::create_dir(&origin).unwrap();
        sh_git(&origin, &["init", "--bare", "-b", "main"]);

        let seed = tmp.path().join("seed");
        std::fs::create_dir(&seed).unwrap();
        sh_git(&seed, &["init", "-b", "main"]);
        sh_git(&seed, &["config", "user.email", "otto@test.local"]);
        sh_git(&seed, &["config", "user.name", "Otto Test"]);
        sh_git(&seed, &["config", "commit.gpgsign", "false"]);
        write(&seed, "shared.txt", "line1\nline2\nline3\n");
        sh_git(&seed, &["add", "."]);
        sh_git(&seed, &["commit", "-m", "init"]);
        sh_git(&seed, &["branch", "develop"]);
        sh_git(
            &seed,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        sh_git(&seed, &["push", "-u", "origin", "main", "develop"]);

        let dir = tmp.path().join("work");
        sh_git(
            tmp.path(),
            &["clone", origin.to_str().unwrap(), dir.to_str().unwrap()],
        );
        sh_git(&dir, &["config", "user.email", "otto@test.local"]);
        sh_git(&dir, &["config", "user.name", "Otto Test"]);
        sh_git(&dir, &["config", "commit.gpgsign", "false"]);

        // Local `develop` gets a commit of its own, then we go back to main.
        sh_git(&dir, &["checkout", "-q", "develop"]);
        write(&dir, "local_only.txt", "mine\n");
        sh_git(&dir, &["add", "-A"]);
        sh_git(&dir, &["commit", "-m", "local develop work"]);
        sh_git(&dir, &["checkout", "-q", "main"]);

        // …and origin/develop moves on, so the branch is behind too.
        sh_git(&seed, &["checkout", "-q", "develop"]);
        write(&seed, "upstream_only.txt", "theirs\n");
        sh_git(&seed, &["add", "-A"]);
        sh_git(&seed, &["commit", "-m", "upstream develop work"]);
        sh_git(&seed, &["push", "origin", "develop"]);
        sh_git(&dir, &["fetch", "-q", "origin"]);

        (tmp, dir)
    }

    fn rev(dir: &Path, spec: &str) -> String {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .args(["rev-parse", spec])
            .output()
            .expect("spawn git rev-parse");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn count(dir: &Path, range: &str) -> u32 {
        let out = std::process::Command::new("git")
            .current_dir(dir)
            .args(["rev-list", "--count", range])
            .output()
            .expect("spawn git rev-list");
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    }

    #[tokio::test]
    async fn checkout_autostash_never_pulls() {
        let (_tmp, dir) = diverged_fixture();
        let git = LocalGit::new(&dir);
        let develop_before = rev(&dir, "refs/heads/develop");
        let behind_before = count(&dir, "develop..origin/develop");
        assert!(behind_before > 0, "fixture: develop must start behind");

        // Dirty the file that DIFFERS across the switch so git itself would
        // refuse the plain checkout.
        write(&dir, "shared.txt", "line1\nDIRTY\nline3\n");

        let outcome = git.checkout_autostash("develop", false).await.unwrap();
        assert!(outcome.stashed, "a dirty tree must be stashed");
        assert!(!outcome.pop_conflicted);

        assert_eq!(git.current_branch().await.unwrap(), "develop");
        assert_eq!(
            rev(&dir, "HEAD"),
            develop_before,
            "the switch must not move develop — nothing was pulled or merged"
        );
        assert_eq!(
            count(&dir, "develop..origin/develop"),
            behind_before,
            "still behind origin/develop: a switch never fetches"
        );
        assert!(!dir.join(".git/MERGE_HEAD").exists(), "no merge started");
        assert!(git.op_in_progress().await.is_none());
        assert_eq!(
            std::fs::read_to_string(dir.join("shared.txt")).unwrap(),
            "line1\nDIRTY\nline3\n",
            "the stashed change is restored on the new branch"
        );
        assert!(
            git.stash_list().await.unwrap().is_empty(),
            "a clean pop leaves no stash entry"
        );
    }

    #[tokio::test]
    async fn checkout_autostash_creates_tracking_branch_when_remote_only() {
        let (_tmp, dir) = diverged_fixture();
        // A branch that exists ONLY on origin.
        sh_git(
            &dir,
            &[
                "update-ref",
                "refs/remotes/origin/feature",
                "origin/develop",
            ],
        );
        write(&dir, "shared.txt", "line1\nDIRTY\nline3\n");

        let git = LocalGit::new(&dir);
        let outcome = git.checkout_autostash("feature", true).await.unwrap();
        assert!(outcome.stashed);
        assert_eq!(git.current_branch().await.unwrap(), "feature");

        let upstream = git
            .run(&["rev-parse", "--abbrev-ref", "feature@{u}"])
            .await
            .unwrap();
        assert_eq!(
            upstream.trim(),
            "origin/feature",
            "created branch tracks origin"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("shared.txt")).unwrap(),
            "line1\nDIRTY\nline3\n"
        );
    }

    #[tokio::test]
    async fn checkout_autostash_restores_stash_when_switch_fails() {
        let (_tmp, dir) = diverged_fixture();
        write(&dir, "shared.txt", "line1\nDIRTY\nline3\n");
        let git = LocalGit::new(&dir);

        let err = git
            .checkout_autostash("no-such-branch", false)
            .await
            .unwrap_err();
        assert!(
            !matches!(err, Error::Invalid(_)),
            "git's own refusal: {err:?}"
        );

        assert_eq!(
            git.current_branch().await.unwrap(),
            "main",
            "tree untouched"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("shared.txt")).unwrap(),
            "line1\nDIRTY\nline3\n",
            "a failed switch pops the stash back"
        );
        assert!(git.stash_list().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn checkout_autostash_pop_conflict_is_ok() {
        let (_tmp, dir) = diverged_fixture();
        // develop changes the same line the user is editing → the pop conflicts.
        sh_git(&dir, &["checkout", "-q", "develop"]);
        write(&dir, "shared.txt", "line1\nDEVELOP\nline3\n");
        sh_git(&dir, &["commit", "-am", "develop edits the shared line"]);
        sh_git(&dir, &["checkout", "-q", "main"]);
        write(&dir, "shared.txt", "line1\nDIRTY\nline3\n");

        let git = LocalGit::new(&dir);
        let outcome = git.checkout_autostash("develop", false).await.unwrap();
        assert!(outcome.stashed && outcome.pop_conflicted, "{outcome:?}");

        let st = git.status().await.unwrap();
        assert!(
            st.changes
                .iter()
                .any(|c| c.path == "shared.txt" && c.kind == "conflicted"),
            "the conflicted pop is visible in the status: {:?}",
            st.changes
        );
        assert!(
            st.op_in_progress.is_none(),
            "a stash pop is not a merge/rebase — no op to abort"
        );
    }

    #[tokio::test]
    async fn checkout_autostash_untracked_collision_is_409_and_keeps_stash() {
        let (_tmp, dir) = diverged_fixture();
        // `develop` TRACKS a file the user has as an untracked local file: the
        // switch works, the pop refuses to clobber it.
        sh_git(&dir, &["checkout", "-q", "develop"]);
        write(&dir, "collide.txt", "tracked on develop\n");
        sh_git(&dir, &["add", "-A"]);
        sh_git(&dir, &["commit", "-m", "develop adds collide.txt"]);
        sh_git(&dir, &["checkout", "-q", "main"]);
        write(&dir, "collide.txt", "untracked locally\n");

        let git = LocalGit::new(&dir);
        let err = git.checkout_autostash("develop", false).await.unwrap_err();
        match &err {
            Error::Conflict(m) => {
                assert!(m.contains("switched to develop"), "{m}");
                assert!(m.contains("git stash pop"), "{m}");
            }
            other => panic!("expected a 409 Conflict, got {other:?}"),
        }
        assert_eq!(git.current_branch().await.unwrap(), "develop");
        assert_eq!(
            git.stash_list().await.unwrap().len(),
            1,
            "the stash entry is kept so nothing is lost"
        );
    }

    /// `git stash push` exits 0 with "No local changes to save" when porcelain
    /// is non-empty but nothing is stashable (a submodule with a dirty work
    /// tree is the real-world shape). Nothing of OURS is on the stack, so the
    /// switch must not pop — popping would take the user's own `stash@{0}`.
    #[tokio::test]
    async fn checkout_autostash_does_not_pop_when_nothing_was_stashed() {
        let (tmp, dir) = diverged_fixture();
        // The user's own stash, the entry a bogus pop would steal.
        write(&dir, "shared.txt", "line1\nUSER STASH\nline3\n");
        sh_git(&dir, &["stash", "push", "-m", "user"]);
        // A dirty-but-(pretend)-unstashable tree: porcelain is non-empty, so
        // `checkout_autostash` takes the stash branch.
        write(&dir, "scratch.txt", "mine\n");

        // A real submodule fixture reproduces this without a shim, but needs
        // `protocol.file.allow=always` and a nested clone; forcing the ONE
        // outcome that matters is deterministic and git-version independent.
        let sh = shim(
            tmp.path(),
            "nostash-git",
            "if [ \"$1\" = stash ] && [ \"$2\" = push ]; then\n\
             echo 'No local changes to save'\n\
             exit 0\n\
             fi\n\
             exec git \"$@\"",
        );
        let git = LocalGit::new(&dir).with_git_bin(&sh);

        let outcome = git.checkout_autostash("develop", false).await.unwrap();
        assert_eq!(git.current_branch().await.unwrap(), "develop");

        let stashes = git.stash_list().await.unwrap();
        assert_eq!(
            stashes.len(),
            1,
            "the user's own stash must survive the switch untouched: {stashes:?}"
        );
        assert!(stashes[0].message.contains("user"), "{:?}", stashes[0]);
        assert_eq!(
            std::fs::read_to_string(dir.join("shared.txt")).unwrap(),
            "line1\nline2\nline3\n",
            "the user's stash was not applied onto the new branch"
        );
        assert!(
            !outcome.stashed,
            "nothing was stashed, so nothing may be reported as stashed: {outcome:?}"
        );
        assert!(!outcome.pop_conflicted);
        let st = git.status().await.unwrap();
        assert!(
            !st.changes.iter().any(|c| c.kind == "conflicted"),
            "a stash we never pushed cannot conflict: {:?}",
            st.changes
        );
    }

    // ── R1: hardening ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn diff_target_refuses_option_like_revs() {
        assert!(matches!(
            DiffTarget::parse("commit:--output=/tmp/pwn"),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            DiffTarget::parse("range:--upload-pack=x..HEAD"),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            DiffTarget::parse("range:HEAD..--x"),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            DiffTarget::parse("range:a b..HEAD"),
            Err(Error::Invalid(_))
        ));

        // A real sha still parses AND still diffs.
        let (_tmp, dir) = fixture_n_commits(2);
        let git = LocalGit::new(&dir);
        let sha = git.log(1, 0, false).await.unwrap()[0].sha.clone();
        let target = DiffTarget::parse(&format!("commit:{sha}")).unwrap();
        assert_eq!(target, DiffTarget::Commit(sha));
        assert!(!git.diff(target, None).await.unwrap().files.is_empty());
    }

    #[test]
    fn clone_url_validation() {
        for ok in [
            "https://h/o/r.git",
            "http://h/o/r",
            "ssh://git@h/o/r",
            "git://h/o/r.git",
            "git@h:o/r.git",
            "git@github.com:otto/otto_os.git",
        ] {
            assert!(validate_remote_url(ok).is_ok(), "should accept {ok}");
        }
        for bad in [
            "",
            "   ",
            "-c",
            "--upload-pack=x",
            "file:///tmp/x",
            "/tmp/x",
            "https://h/o r",
            "a\nb",
            "https://h",
            "https://h/",
            "ssh://",
            "ftp://h/o/r",
            "git@h:",
            "git@h:-x",
        ] {
            assert!(validate_remote_url(bad).is_err(), "should refuse {bad:?}");
        }
    }

    #[tokio::test]
    async fn worktree_probes_distinguish_known_clean_and_dirty() {
        let (tmp, dir) = fixture_on_branch("main");
        let git = LocalGit::new(&dir);
        let wt = tmp.path().join("linked");
        git.worktree_add(wt.to_str().unwrap(), "other", "HEAD")
            .await
            .unwrap();
        write(&wt, "untracked.txt", "changes");
        let rows = git.worktree_list().await.unwrap();
        assert_eq!(rows.len(), 2);
        assert!(!rows[0].dirty);
        assert!(rows[1].dirty);
        assert_eq!(serde_json::to_value(&rows[0]).unwrap()["dirty_known"], true);
        assert_eq!(serde_json::to_value(&rows[1]).unwrap()["dirty_known"], true);
    }

    #[tokio::test]
    async fn worktree_probe_failure_is_unknown_and_uses_configured_git() {
        let (tmp, dir) = fixture_on_branch("main");
        let bin = shim(
            tmp.path(),
            "probe-git",
            "if [ \"$1\" = status ]; then exit 1; fi\nexec git \"$@\"",
        );
        let rows = LocalGit::new(&dir)
            .with_git_bin(bin)
            .worktree_list()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].dirty);
        assert_eq!(
            serde_json::to_value(&rows[0]).unwrap()["dirty_known"],
            false
        );
    }

    /// Write an executable `sh` shim and return its path. `body` runs with the
    /// repo as cwd (`base_cmd` sets `current_dir`).
    fn shim(tmp: &Path, name: &str, body: &str) -> PathBuf {
        let p = tmp.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    /// Budget for the two shim tests. Generous on purpose: the whole suite
    /// spawns git in parallel, and a tight budget occasionally SIGTERMed the
    /// shim's process group before `sh` had even recorded its pids — a flake
    /// that looks exactly like a broken group kill.
    const SHIM_BUDGET: std::time::Duration = std::time::Duration::from_secs(5);

    /// Read the pids a shim recorded, with a message that names the real cause
    /// if it never got that far.
    fn shim_pids(path: &Path) -> Vec<libc::pid_t> {
        let raw = std::fs::read_to_string(path).unwrap_or_else(|e| {
            panic!(
                "shim never recorded its pids ({e}) — the {SHIM_BUDGET:?} budget \
                 beat its own startup; raise SHIM_BUDGET"
            )
        });
        raw.lines().filter_map(|l| l.trim().parse().ok()).collect()
    }

    fn alive(pid: libc::pid_t) -> bool {
        // SAFETY: signal 0 only probes for the process's existence.
        unsafe { libc::kill(pid, 0) == 0 }
    }

    async fn wait_dead(pids: &[libc::pid_t], within: std::time::Duration) {
        let deadline = std::time::Instant::now() + within;
        loop {
            if pids.iter().all(|p| !alive(*p)) {
                return;
            }
            if std::time::Instant::now() >= deadline {
                let left: Vec<_> = pids.iter().filter(|p| alive(**p)).collect();
                panic!("still alive after {within:?}: {left:?}");
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn worktree_probe_timeout_keeps_cleanup_after_response() {
        let (tmp, dir) = fixture_on_branch("main");
        let bin = shim(tmp.path(), "slow-status", "if [ \"$1\" != status ]; then exec git \"$@\"; fi\ntrap '' TERM\nsleep 30 &\necho $! > shim-pids\necho $$ >> shim-pids\nwait");
        let rows = LocalGit::new(&dir)
            .with_git_bin(bin)
            .worktree_list_with_budgets(
                SHIM_BUDGET,
                SHIM_BUDGET + std::time::Duration::from_secs(1),
            )
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].dirty_known && !rows[0].dirty);
        let pids = shim_pids(&dir.join("shim-pids"));
        assert_eq!(pids.len(), 2);
        wait_dead(&pids, std::time::Duration::from_secs(5)).await;
    }

    /// A spawn that overruns its budget is SIGTERMed as a GROUP, so the helper
    /// processes git forked (`ssh`, `git-remote-https`) die with it — a plain
    /// `kill(pid)` would leave them holding the network connection.
    #[tokio::test]
    async fn spawn_timeout_terminates_group_then_kills() {
        let (tmp, dir) = fixture_n_commits(1);
        let sh = shim(
            tmp.path(),
            "slow-git",
            "sleep 30 &\necho $! > shim-pids\necho $$ >> shim-pids\nsleep 30",
        );
        let git = LocalGit::new(&dir)
            .with_git_bin(&sh)
            .with_budget(SHIM_BUDGET);

        let started = std::time::Instant::now();
        let err = git.run(&["status"]).await.unwrap_err();
        assert!(
            started.elapsed() < SHIM_BUDGET + std::time::Duration::from_secs(6),
            "the timeout must not wait for the process: {:?}",
            started.elapsed()
        );
        match &err {
            Error::Upstream(m) => {
                assert!(m.contains("timed out"), "{m}");
                assert!(m.contains("index.lock"), "the text names the leftover: {m}");
            }
            other => panic!("expected Upstream, got {other:?}"),
        }

        let pids = shim_pids(&dir.join("shim-pids"));
        assert_eq!(pids.len(), 2, "shim recorded its own pid and its child's");
        wait_dead(&pids, std::time::Duration::from_secs(5)).await;
    }

    /// A process that IGNORES SIGTERM (git never does, but a hung helper might)
    /// is escalated to SIGKILL 2 s later — the budget is a real bound.
    #[tokio::test]
    async fn spawn_timeout_escalates_to_sigkill() {
        let (tmp, dir) = fixture_n_commits(1);
        // `trap '' TERM` BEFORE forking `sleep`: an ignored disposition is
        // inherited, so neither process dies until the SIGKILL.
        let sh = shim(
            tmp.path(),
            "stubborn-git",
            "trap '' TERM\necho $$ > shim-pid\nsleep 30",
        );
        let git = LocalGit::new(&dir)
            .with_git_bin(&sh)
            .with_budget(SHIM_BUDGET);

        let started = std::time::Instant::now();
        let err = git.run(&["status"]).await.unwrap_err();
        assert!(
            matches!(&err, Error::Upstream(m) if m.contains("timed out")),
            "{err:?}"
        );
        assert!(
            started.elapsed() < SHIM_BUDGET + std::time::Duration::from_secs(6),
            "elapsed {:?}",
            started.elapsed()
        );
        let pids = shim_pids(&dir.join("shim-pid"));
        assert_eq!(pids.len(), 1, "shim recorded its pid");
        wait_dead(&pids, std::time::Duration::from_secs(5)).await;
    }

    /// The reason `LocalWrite` is detached: the UI aborts in-flight requests, and
    /// hyper drops the handler future with them. A commit that is already writing
    /// the index must survive that, or the repo is left with `index.lock`.
    #[tokio::test]
    async fn mutating_spawn_survives_request_drop() {
        let (_tmp, dir) = fixture_n_commits(1);
        let hooks = dir.join(".git/hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        let hook = hooks.join("pre-commit");
        std::fs::write(&hook, "#!/bin/sh\nsleep 0.4\nexit 0\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        write(&dir, "late.txt", "committed under a dropped request\n");
        sh_git(&dir, &["add", "-A"]);

        let git = LocalGit::new(&dir);
        let dropped = tokio::time::timeout(
            std::time::Duration::from_millis(10),
            git.commit("survives the drop", false),
        )
        .await;
        assert!(
            dropped.is_err(),
            "the caller future must be dropped mid-commit"
        );

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let subject = LocalGit::new(&dir)
            .run(&["log", "-1", "--format=%s"])
            .await
            .unwrap();
        assert_eq!(subject.trim(), "survives the drop");
        assert!(
            !dir.join(".git/index.lock").exists(),
            "a SIGKILLed git would have left index.lock behind"
        );
    }

    /// Same contract for the `Remote` class: a dropped request must not abort a
    /// fetch mid-write (`FETCH_HEAD.lock` / `refs/remotes/*.lock`).
    #[tokio::test]
    async fn remote_class_survives_request_drop() {
        let (tmp, dir) = diverged_fixture();
        // Advance origin/main so the fetch has something to do.
        let seed = tmp.path().join("seed");
        sh_git(&seed, &["checkout", "-q", "main"]);
        write(&seed, "fetched.txt", "after\n");
        sh_git(&seed, &["add", "-A"]);
        sh_git(&seed, &["commit", "-m", "advance main"]);
        sh_git(&seed, &["push", "origin", "main"]);
        let origin_tip = rev(&tmp.path().join("origin.git"), "refs/heads/main");

        let sh = shim(tmp.path(), "slow-fetch-git", "sleep 0.4\nexec git \"$@\"");
        let git = LocalGit::new(&dir).with_git_bin(&sh);
        let dropped =
            tokio::time::timeout(std::time::Duration::from_millis(10), git.fetch(None)).await;
        assert!(
            dropped.is_err(),
            "the caller future must be dropped mid-fetch"
        );

        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        assert_eq!(
            rev(&dir, "refs/remotes/origin/main"),
            origin_tip,
            "the detached fetch finished after the request was dropped"
        );
    }

    /// An agent's concurrent git holding `index.lock` is transient — staging
    /// retries instead of surfacing a bogus failure.
    #[tokio::test]
    async fn stage_retries_transient_index_lock() {
        let (_tmp, dir) = fixture_n_commits(1);
        write(&dir, "staged.txt", "one\n");
        let lock = dir.join(".git/index.lock");
        std::fs::write(&lock, "").unwrap();

        let unlock = lock.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(200));
            let _ = std::fs::remove_file(&unlock);
        });

        let git = LocalGit::new(&dir);
        git.stage(&["staged.txt".to_string()]).await.unwrap();
        let st = git.status().await.unwrap();
        assert!(st
            .changes
            .iter()
            .any(|c| c.path == "staged.txt" && c.staged));
    }

    /// G-1: every path is a LITERAL name. As a pathspec `app/[id]/page.tsx`
    /// also matched `app/d/page.tsx` (restore reverted its edits) and
    /// `app/i/page.tsx` (clean deleted it) — silently, with a success reply.
    #[tokio::test]
    async fn discard_and_stage_treat_bracketed_paths_literally() {
        let (_tmp, dir) = fixture_on_branch("main");
        write(&dir, "app/[id]/page.tsx", "id\n");
        write(&dir, "app/d/page.tsx", "d\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "routes"]);
        write(&dir, "app/[id]/page.tsx", "id EDIT\n");
        write(&dir, "app/d/page.tsx", "d EDIT\n");
        write(&dir, "app/i/page.tsx", "untracked\n");
        let git = LocalGit::new(&dir);

        git.discard(&["app/[id]/page.tsx".into()]).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("app/[id]/page.tsx")).unwrap(),
            "id\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("app/d/page.tsx")).unwrap(),
            "d EDIT\n",
            "a glob match must not revert a sibling"
        );
        assert!(dir.join("app/i/page.tsx").exists(), "…nor clean one");

        write(&dir, "app/[id]/page.tsx", "id EDIT\n");
        git.stage(&["app/[id]/page.tsx".into()]).await.unwrap();
        let staged = git.run(&["diff", "--cached", "--name-only"]).await.unwrap();
        assert_eq!(staged.trim(), "app/[id]/page.tsx");
    }

    /// G-3: status is read with `-z`, so a non-ASCII or `"`-carrying name is
    /// the REAL name — stage/unstage/discard find the file, and a discard
    /// with nothing left to do is a 409 instead of a silent "Discarded".
    #[tokio::test]
    async fn quoted_and_non_ascii_names_stage_and_discard() {
        let (_tmp, dir) = fixture_on_branch("main");
        write(&dir, "caf\u{e9}.txt", "accent\n");
        write(&dir, "q\"uote.txt", "quote\n");
        let git = LocalGit::new(&dir);
        let st = git.status().await.unwrap();
        let names: Vec<&str> = st.changes.iter().map(|c| c.path.as_str()).collect();
        assert!(names.contains(&"caf\u{e9}.txt"), "{names:?}");
        assert!(names.contains(&"q\"uote.txt"), "{names:?}");

        let both: Vec<String> = vec!["caf\u{e9}.txt".into(), "q\"uote.txt".into()];
        git.stage(&both).await.unwrap();
        let st = git.status().await.unwrap();
        assert!(st.changes.iter().all(|c| c.staged), "{:?}", st.changes);
        git.unstage(&both).await.unwrap();

        git.discard(&["caf\u{e9}.txt".into()]).await.unwrap();
        assert!(!dir.join("caf\u{e9}.txt").exists(), "the untracked file is gone");
        assert!(dir.join("q\"uote.txt").exists());
        assert!(matches!(
            git.discard(&["caf\u{e9}.txt".into()]).await,
            Err(Error::Conflict(_))
        ));
    }

    /// G-9: before the first commit there is no HEAD to restore the index
    /// from; unstage still works and the branch name still reads.
    #[tokio::test]
    async fn unstage_and_branch_name_before_the_first_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        sh_git(&dir, &["init", "-b", "main"]);
        write(&dir, "a.txt", "new\n");
        sh_git(&dir, &["add", "a.txt"]);
        let git = LocalGit::new(&dir);
        assert_eq!(git.current_branch().await.unwrap(), "main");
        git.unstage(&["a.txt".into()]).await.unwrap();
        let st = git.status().await.unwrap();
        assert_eq!(st.changes.len(), 1);
        assert_eq!(st.changes[0].kind, "untracked");
    }

    /// The preview lists conflicted FILES only — not merge-tree's
    /// "Auto-merging …" / "CONFLICT (content): …" message lines.
    #[tokio::test]
    async fn merge_preview_lists_only_conflicted_file_names() {
        let (_tmp, dir) = conflicted_fixture("c.txt");
        let git = LocalGit::new(&dir);
        let p = git.merge_preview("side", "main").await.unwrap();
        assert!(p.conflicts);
        assert_eq!(p.conflicted_files, vec!["c.txt".to_string()]);
    }

    /// Two branches that change the same line of `rel` → `merge side`
    /// conflicts on it. Returns (tmp, repo).
    fn conflicted_fixture(rel: &str) -> (tempfile::TempDir, PathBuf) {
        let (tmp, dir) = fixture_on_branch("main");
        write(&dir, rel, "base\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "base"]);
        sh_git(&dir, &["checkout", "-q", "-b", "side"]);
        write(&dir, rel, "side\n");
        sh_git(&dir, &["commit", "-qam", "side"]);
        sh_git(&dir, &["checkout", "-q", "main"]);
        write(&dir, rel, "main\n");
        sh_git(&dir, &["commit", "-qam", "main"]);
        (tmp, dir)
    }

    /// G-4: the resolver writes ONLY a currently-conflicted file, inside the
    /// working tree — never `.git/config` (fsmonitor = code execution), never
    /// through a symlink or a symlinked parent.
    #[tokio::test]
    async fn write_resolution_is_confined_to_conflicted_worktree_files() {
        let (tmp, dir) = conflicted_fixture("sub/c.txt");
        let git = LocalGit::new(&dir);
        let _ = git.run(&["merge", "side"]).await;
        assert_eq!(git.conflicted_paths().await.unwrap(), vec!["sub/c.txt"]);

        let config_before = std::fs::read(dir.join(".git/config")).unwrap();
        let payload = "[core]\n\tfsmonitor = /tmp/otto-pwn.sh\n";
        for bad in [
            ".git/config",
            ".GIT/config",
            "a.txt",
            "sub",
            "sub/*",
            "../outside.txt",
            "/etc/hosts",
        ] {
            assert!(git.write_resolution(bad, payload).await.is_err(), "{bad}");
        }
        assert_eq!(std::fs::read(dir.join(".git/config")).unwrap(), config_before);
        assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "hello\n");

        // The conflicted file itself swapped for a symlink to a file outside.
        let outside = tmp.path().join("outside.txt");
        std::fs::write(&outside, "keep\n").unwrap();
        let conflicted = dir.join("sub/c.txt");
        let marked = std::fs::read(&conflicted).unwrap();
        std::fs::remove_file(&conflicted).unwrap();
        std::os::unix::fs::symlink(&outside, &conflicted).unwrap();
        assert!(matches!(
            git.write_resolution("sub/c.txt", "pwned\n").await,
            Err(Error::Invalid(_))
        ));
        assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep\n");
        std::fs::remove_file(&conflicted).unwrap();

        // …and a symlinked PARENT directory that leads out of the tree.
        let outside_dir = tmp.path().join("outside_dir");
        std::fs::create_dir(&outside_dir).unwrap();
        std::fs::write(outside_dir.join("c.txt"), "keep\n").unwrap();
        std::fs::rename(dir.join("sub"), tmp.path().join("sub_parked")).unwrap();
        std::os::unix::fs::symlink(&outside_dir, dir.join("sub")).unwrap();
        assert!(git.write_resolution("sub/c.txt", "pwned\n").await.is_err());
        assert_eq!(
            std::fs::read_to_string(outside_dir.join("c.txt")).unwrap(),
            "keep\n"
        );
        std::fs::remove_file(dir.join("sub")).unwrap();
        std::fs::rename(tmp.path().join("sub_parked"), dir.join("sub")).unwrap();
        std::fs::write(&conflicted, marked).unwrap();

        // The legitimate write still resolves and stages the file.
        git.write_resolution("sub/c.txt", "resolved\n").await.unwrap();
        assert!(git.conflicted_paths().await.unwrap().is_empty());
        assert_eq!(std::fs::read_to_string(&conflicted).unwrap(), "resolved\n");
    }

    /// G-5: a name that is a DIRECTORY, not a branch, must never be read as a
    /// path — `checkout docs` restored `docs/` from the index (exit 0) and the
    /// user's uncommitted edits were gone.
    #[tokio::test]
    async fn checkout_of_a_directory_name_keeps_uncommitted_work() {
        let (_tmp, dir) = fixture_on_branch("main");
        write(&dir, "docs/a.md", "committed\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "docs"]);
        write(&dir, "docs/a.md", "committed\nUNCOMMITTED EDIT\n");
        let git = LocalGit::new(&dir);
        match git.checkout("docs", false).await {
            Err(Error::Conflict(msg)) => assert!(msg.contains("invalid reference"), "{msg}"),
            other => panic!("expected Conflict, got {other:?}"),
        }
        assert_eq!(
            std::fs::read_to_string(dir.join("docs/a.md")).unwrap(),
            "committed\nUNCOMMITTED EDIT\n",
            "the edit survives"
        );
        assert_eq!(git.current_branch().await.unwrap(), "main");
        // A tag / SHA still checks out (detached) through the same path.
        sh_git(&dir, &["stash", "push", "-m", "park"]);
        sh_git(&dir, &["tag", "v1"]);
        git.checkout("v1", false).await.unwrap();
        git.checkout("main", false).await.unwrap();
    }

    /// G-6: when the switch to `target` fails after the auto-stash (target is
    /// checked out in an agent's worktree), the user's tracked AND untracked
    /// changes come back on the branch they were on — nothing is stranded.
    #[tokio::test]
    async fn merge_autostash_restores_changes_when_the_switch_fails() {
        let (tmp, dir) = fixture_on_branch("main");
        sh_git(&dir, &["branch", "develop"]);
        sh_git(&dir, &["checkout", "-q", "-b", "feature"]);
        write(&dir, "f.txt", "feature\n");
        sh_git(&dir, &["add", "."]);
        sh_git(&dir, &["commit", "-m", "feature"]);
        let wt = tmp.path().join("wt");
        sh_git(&dir, &["worktree", "add", wt.to_str().unwrap(), "develop"]);
        write(&dir, "a.txt", "hello\nDIRTY\n");
        write(&dir, "new.txt", "untracked\n");

        let git = LocalGit::new(&dir);
        let err = git
            .merge_branch("feature", "develop", LocalMergeStrategy::MergeCommit, true)
            .await
            .unwrap_err();
        assert_eq!(git.current_branch().await.unwrap(), "feature", "{err:?}");
        assert_eq!(
            std::fs::read_to_string(dir.join("a.txt")).unwrap(),
            "hello\nDIRTY\n"
        );
        assert!(dir.join("new.txt").exists(), "untracked file restored");
        assert!(git.stash_list().await.unwrap().is_empty(), "{err:?}");
    }

}
