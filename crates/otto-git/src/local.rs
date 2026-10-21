//! Local git operations: shells out to the system `git` binary via
//! `tokio::process`, never prompts (`GIT_TERMINAL_PROMPT=0`), and parses
//! plumbing output with `crate::parse`.

use crate::transport::TransportCredential;
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
    /// Two commits' trees (`git diff a..b` — for `git diff` two dots mean
    /// "a vs b", NOT the `log` meaning; kept as-is for existing callers).
    Range(String, String),
    /// Branch-vs-base (`git diff a...b`): `b` against the merge-base of `a`
    /// and `b` — what the branch itself changed, ignoring the base's own
    /// newer commits.
    MergeBase(String, String),
}

impl DiffTarget {
    /// Parse the `?target=` query value: `worktree | staged | commit:<sha> |
    /// range:<a>..<b> | range:<a>...<b>`.
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
                    // Three dots first: `a...b` also contains `..`.
                    let (split, three) = match range.split_once("...") {
                        Some(ab) => (Some(ab), true),
                        None => (range.split_once(".."), false),
                    };
                    if let Some((a, b)) = split {
                        if !a.is_empty() && !b.is_empty() {
                            LocalGit::guard_ref(a)?;
                            LocalGit::guard_ref(b)?;
                            // A space would split one argv slot into two once the
                            // range is re-assembled as `<a>..<b>`.
                            if a.chars().chain(b.chars()).any(char::is_whitespace) {
                                return Err(Error::Invalid(format!("bad range: {range}")));
                            }
                            let (a, b) = (a.to_string(), b.to_string());
                            return Ok(if three {
                                Self::MergeBase(a, b)
                            } else {
                                Self::Range(a, b)
                            });
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

/// Top-level `-c` config Otto adds per subcommand (prepended before `args`):
///
/// * `merge.conflictStyle=diff3` for every subcommand that can leave conflict
///   markers (for `stash`, only `pop` / `apply`). With git's default `merge`
///   style the markers carry only ours/theirs, so the resolver had no BASE for
///   rebase, cherry-pick, revert, pull or stash conflicts (only `merge_branch`
///   set it). `diff3`,
///   not `zdiff3`, so older gits still work; `parse_conflict_segments` reads
///   it.
/// * `fetch.writeCommitGraph=true` on fetch/pull: git keeps an incremental
///   commit-graph, which makes `log --all` (the graph), `merge-base`,
///   `branch --merged` and blame scale on big histories. Opt out with
///   `OTTO_GIT_COMMIT_GRAPH=0`.
///
/// Callers that already pass their own `-c …` (args[0] == "-c") get nothing.
pub(crate) fn implicit_config_args(args: &[&str]) -> &'static [&'static str] {
    const DIFF3: &[&str] = &["-c", "merge.conflictStyle=diff3"];
    const CG: &[&str] = &["-c", "fetch.writeCommitGraph=true"];
    const DIFF3_CG: &[&str] = &[
        "-c",
        "merge.conflictStyle=diff3",
        "-c",
        "fetch.writeCommitGraph=true",
    ];
    let cg = commit_graph_enabled();
    match args.first() {
        Some(&"fetch") if cg => CG,
        Some(&"pull") if cg => DIFF3_CG,
        Some(&("cherry-pick" | "revert" | "rebase" | "pull" | "merge" | "am")) => DIFF3,
        // Only the stash verbs that can conflict: `stash push` / `list` stay
        // bare (and keep their argv shape for wrappers keyed on `$1`).
        Some(&"stash") if matches!(args.get(1), Some(&("pop" | "apply"))) => DIFF3,
        _ => &[],
    }
}

/// `OTTO_GIT_COMMIT_GRAPH=0` turns off Otto's commit-graph upkeep.
fn commit_graph_enabled() -> bool {
    std::env::var("OTTO_GIT_COMMIT_GRAPH").map_or(true, |v| v.trim() != "0")
}

/// Config every daemon git runs with, as `-c` would set it but through the
/// environment (`GIT_CONFIG_PARAMETERS`), so argv keeps its shape for
/// [`verb_of`] and the test shims, and a per-spawn `GIT_CONFIG_COUNT` (the
/// askpass credential reset) does not clobber it. A confined agent can write
/// the repo's `.git`, and the daemon polls status / fetches it unconfined:
///
/// - `core.fsmonitor=false` — an fsmonitor hook is a program `git status`
///   runs on every refresh;
/// - `core.hooksPath=/dev/null` — no hook (`reference-transaction` fires on a
///   background fetch) runs, except on the verbs a person runs on purpose
///   ([`HOOK_VERBS`], which keep the repo's hooks).
///
/// `diff.external`/textconv are refused per command (`--no-ext-diff
/// --no-textconv`, [`DIFF_FORMAT`]) and the pager by `GIT_PAGER=cat`; other
/// program-valued keys (`core.sshCommand`, `credential.helper`, filters) are
/// legitimately user-set, so the Seatbelt profile keeps agents from writing
/// `.git/config` instead.
pub(crate) const HARDENED_GIT_CONFIG: &str = "'core.fsmonitor=false' 'core.hooksPath=/dev/null'";
/// [`HARDENED_GIT_CONFIG`] for a [`HOOK_VERBS`] spawn: the repo's hooks run.
pub(crate) const HOOKED_GIT_CONFIG: &str = "'core.fsmonitor=false'";

/// Verbs a person runs deliberately (commit / merge / push / branch switch…)
/// whose hooks (pre-commit, commit-msg, pre-push, LFS post-checkout) they
/// expect to run. Every other daemon git — status polling, fetch, log, diff,
/// gc, commit-graph — runs with hooks disabled.
pub(crate) const HOOK_VERBS: &[&str] = &[
    "commit",
    "merge",
    "pull",
    "push",
    "rebase",
    "cherry-pick",
    "revert",
    "am",
    "checkout",
    "switch",
];

/// The `GIT_CONFIG_PARAMETERS` value a spawn of `verb` runs with.
pub(crate) fn hardened_config_for(verb: &str) -> &'static str {
    if HOOK_VERBS.contains(&verb) {
        HOOKED_GIT_CONFIG
    } else {
        HARDENED_GIT_CONFIG
    }
}

/// A `git` command for code OUTSIDE [`LocalGit`] (rev-parse probes, goal-loop
/// evidence, skill-eval scratch repos, plugin clones…) with the same
/// hardening every daemon git gets: repo config can't pick a program for it
/// to run ([`HARDENED_GIT_CONFIG`]: fsmonitor off, hooks off), no pager, no
/// credential prompt, stdin closed, killed when dropped. Prefer `LocalGit`
/// (output cap + timeouts) where it fits; spawning a bare
/// `Command::new("git")` outside otto-git is refused by a guard test
/// (`otto-server` `no_bare_git_spawns`).
pub fn hardened_command() -> Command {
    let mut cmd = Command::new("git");
    harden(cmd.as_std_mut());
    cmd.kill_on_drop(true);
    cmd
}

/// [`hardened_command`] for synchronous callers.
pub fn hardened_std_command() -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    harden(&mut cmd);
    cmd
}

fn harden(cmd: &mut std::process::Command) {
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_PARAMETERS", HARDENED_GIT_CONFIG)
        .env("GIT_PAGER", "cat")
        .stdin(Stdio::null());
    test_hermetic(cmd);
}

/// This crate's unit tests run git against throwaway fixtures: the developer's
/// own `~/.gitconfig` / system config (a `url.*.insteadOf` rewrite, a signing
/// key, a default branch) must not leak in — CI has none, so a laptop with,
/// say, a Bitbucket `insteadOf` failed tests CI passes. Production spawns keep
/// the user's config.
#[cfg(test)]
fn test_hermetic(cmd: &mut std::process::Command) {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    cmd.env("GIT_CONFIG_GLOBAL", null)
        .env("GIT_CONFIG_NOSYSTEM", "1");
}

#[cfg(not(test))]
fn test_hermetic(_cmd: &mut std::process::Command) {}

/// Does the work tree at `path` round-trip to the repo whose common git dir
/// is `common`? The main tree's `.git` must BE `common`; a linked tree's
/// `.git` file must name `<common>/worktrees/<id>`, whose `gitdir` file must
/// name that same `.git` back. Anything else (a forged pointer, a foreign
/// repo) is not probed.
pub(crate) fn worktree_round_trips(path: &Path, common: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    let Some(common) = canon(common) else {
        return false;
    };
    let dot_git = path.join(".git");
    let Ok(meta) = std::fs::symlink_metadata(&dot_git) else {
        return false;
    };
    if meta.is_dir() {
        return canon(&dot_git).is_some_and(|g| g == common);
    }
    if !meta.is_file() {
        return false;
    }
    let Ok(text) = std::fs::read_to_string(&dot_git) else {
        return false;
    };
    let Some(target) = text.trim().strip_prefix("gitdir:").map(str::trim) else {
        return false;
    };
    let target = Path::new(target);
    let admin = if target.is_absolute() {
        target.to_path_buf()
    } else {
        path.join(target)
    };
    let Some(admin) = canon(&admin) else {
        return false;
    };
    if admin.parent() != Some(common.join("worktrees").as_path()) {
        return false;
    }
    let Ok(back) = std::fs::read_to_string(admin.join("gitdir")) else {
        return false;
    };
    canon(Path::new(back.trim())).is_some_and(|b| Some(b) == canon(&dot_git))
}

pub(crate) use crate::hooks::hooks_path_inside;

/// The git subcommand in an argv: the first word after git's GLOBAL options.
/// Used for error text AND for [`HOOK_VERBS`] gating, so every leading global
/// option must be skipped — `-c <k=v>` (the diff family), `-C <dir>`, the
/// separate-argument long forms (`--git-dir <p>`, `--work-tree <p>`,
/// `--namespace <n>`, …) and flag-only ones (`--literal-pathspecs`,
/// `--no-pager`). Stopping at a flag would turn `--no-pager commit` into a
/// non-hook verb and silently skip the user's hooks.
pub(crate) fn verb_of<'a>(args: &'a [&'a str]) -> &'a str {
    const TAKES_VALUE: &[&str] = &[
        "-c",
        "-C",
        "--git-dir",
        "--work-tree",
        "--namespace",
        "--super-prefix",
        "--config-env",
        "--exec-path",
        "--attr-source",
        "--list-cmds",
    ];
    let mut i = 0;
    while let Some(&a) = args.get(i) {
        if TAKES_VALUE.contains(&a) {
            i += 2;
        } else if a.starts_with('-') {
            i += 1;
        } else {
            return a;
        }
    }
    "command"
}

/// git's `-l<N>` overflow warning ("exhaustive/inexact rename detection was
/// skipped due to too many files"): past [`RENAMES`]' limit some renames come
/// back as a delete + an add. Recorded so the diff can say so
/// (`DiffResp.renames_incomplete`).
fn note_renames(stderr: &str, ri: &std::sync::atomic::AtomicBool) {
    if stderr.contains("rename detection was skipped") {
        ri.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Per-file cap on one untracked file's patch in a review diff (it is
/// clipped past this, with an omitted marker).
const REVIEW_UNTRACKED_FILE_CAP: usize = 1024 * 1024;

/// The `(diff omitted: ` marker line [`LocalGit::review_diff_text`] writes for
/// an untracked file the reviewers did not get in full. MUST equal
/// `otto_review::engine::DIFF_OMITTED_MARKER` (otto-git cannot depend on
/// otto-review); `diff_is_partial` keys on it to block auto-resolution.
pub const UNTRACKED_OMITTED_MARKER: &str = "(diff omitted: ";

/// Ceiling on the stdout ANY single git spawn may buffer. Nothing Otto asks
/// for legitimately comes close (a full 100k-line commit diff is ~10 MB, a
/// 100k-commit `log` page ~25 MB); it exists so `log limit=0` on a monorepo, a
/// blame of a generated file or a runaway `show` fails fast instead of growing
/// the daemon without bound. A read is killed the moment it crosses; a
/// write/remote spawn is never killed mid-flight (index.lock) — its surplus is
/// drained and dropped.
pub(crate) const GIT_STDOUT_CAP: usize = 128 * 1024 * 1024;

/// stderr retained for error text; anything past it is drained and dropped.
const GIT_STDERR_CAP: usize = 1024 * 1024;

/// What a spawn does when stdout crosses its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Overflow {
    /// `Error::Upstream` (a read is killed first).
    Error,
    /// Keep the first `limit` bytes and report success with `cut`.
    Truncate,
}

/// Early-stop hook for a streamed read: called after every chunk with the
/// buffer so far and where the new bytes start; `Some(n)` keeps `buf[..n]`
/// and kills git (reads only).
pub(crate) type StopFn = Box<dyn FnMut(&[u8], usize) -> Option<usize> + Send>;

/// How a spawn's stdout is bounded ([`LocalGit::spawn_capture`]).
pub(crate) struct StdoutLimit {
    pub(crate) max: usize,
    pub(crate) overflow: Overflow,
    pub(crate) stop: Option<StopFn>,
}

impl StdoutLimit {
    /// The crate-wide ceiling, failing on overflow.
    pub(crate) fn ceiling() -> Self {
        Self {
            max: GIT_STDOUT_CAP,
            overflow: Overflow::Error,
            stop: None,
        }
    }

    fn of(c: &GitCmd) -> Self {
        match c.limit {
            Some((max, overflow)) => Self {
                max: max.min(GIT_STDOUT_CAP),
                overflow,
                stop: None,
            },
            None => Self::ceiling(),
        }
    }
}

/// Why a capped read ended before EOF.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cut {
    None,
    /// The [`StopFn`] asked for it.
    Stopped,
    /// stdout crossed [`StdoutLimit::max`].
    Overflow,
}

/// Read `stdout` in chunks up to `limit`. On a stop/overflow of a READ the
/// whole process group is SIGKILLed right here — a read writes nothing, and
/// killing before returning is what lets the concurrent stderr drain see EOF.
/// Any other class keeps draining (and dropping) so git finishes normally.
async fn read_stdout_capped(
    mut stdout: tokio::process::ChildStdout,
    limit: &mut StdoutLimit,
    kill_pid: Option<libc::pid_t>,
) -> std::io::Result<(Vec<u8>, Cut)> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut cut = Cut::None;
    loop {
        let n = stdout.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        if cut != Cut::None {
            continue; // non-read class: drain to EOF, keep nothing more
        }
        let start = buf.len();
        buf.extend_from_slice(&chunk[..n]);
        if let Some(stop) = limit.stop.as_mut() {
            if let Some(keep) = stop(&buf, start) {
                buf.truncate(keep);
                cut = Cut::Stopped;
            }
        }
        if cut == Cut::None && buf.len() > limit.max {
            buf.truncate(limit.max);
            cut = Cut::Overflow;
        }
        if cut != Cut::None {
            if let Some(pid) = kill_pid {
                // SAFETY: signalling the process group we created with
                // `process_group(0)`; ESRCH (already gone) is ignored.
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
                break;
            }
        }
    }
    Ok((buf, cut))
}

/// Drain stderr, keeping the first [`GIT_STDERR_CAP`] bytes.
async fn read_stderr_capped(mut stderr: tokio::process::ChildStderr) -> std::io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        let n = stderr.read(&mut chunk).await?;
        if n == 0 {
            return Ok(buf);
        }
        let room = GIT_STDERR_CAP.saturating_sub(buf.len());
        buf.extend_from_slice(&chunk[..n.min(room)]);
    }
}

/// SIGTERM the whole process group (git plus the `ssh` / `git-remote-https`
/// children it forked), give it 2 s to unwind — git removes `index.lock` on
/// SIGTERM — then SIGKILL whatever is left.
/// Why [`LocalGit::spawn_capture`]'s task failed: the spawn itself (mapped to
/// `Internal("spawn git: …")`) or I/O while collecting (`io_err`).
enum SpawnErr {
    Spawn(String),
    Io(std::io::Error),
}

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
///
/// `--submodule=short`: a `diff.submodule=log|diff` config replaced a gitlink's
/// `diff --git` block with `Submodule sub a..b:` summary lines (hashed into the
/// PREVIOUS file's fingerprint) or inlined the submodule's own files as if
/// they were this repo's.
///
/// `--ignore-submodules=dirty`: deciding whether a gitlink's WORK TREE is
/// dirty means running `git status` INSIDE it, with that repo's own config —
/// and an agent can plant an embedded repo (`sub/.git/config` with a
/// `filter.<x>.clean` program + `sub/.gitattributes`, then `git add sub`)
/// whose filters the daemon's unconfined git would run. A gitlink's moved
/// HEAD still shows; only its work-tree dirt does not. The same flag is on
/// every daemon `status` ([`STATUS_IGNORE_SUBMODULES`]).
pub(crate) const DIFF_FORMAT: [&str; 7] = [
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--src-prefix=a/",
    "--dst-prefix=b/",
    "--submodule=short",
    "--ignore-submodules=dirty",
];

/// Every daemon `git status` skips submodule work-tree dirt (see
/// [`DIFF_FORMAT`]): never recurse into a repo whose config the agent wrote.
pub(crate) const STATUS_IGNORE_SUBMODULES: &str = "--ignore-submodules=dirty";

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
    /// Bytes piped to git's stdin ([`GitCmd::paths_stdin`]).
    stdin: Option<Vec<u8>>,
    /// A tighter stdout ceiling than [`GIT_STDOUT_CAP`] and what happens when
    /// git crosses it ([`GitCmd::max_stdout`] / [`GitCmd::truncate_stdout`]).
    limit: Option<(usize, Overflow)>,
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
            stdin: None,
            limit: None,
        }
    }

    /// Fail the command (killing a read) once its stdout passes `bytes`,
    /// instead of buffering up to the crate-wide [`GIT_STDOUT_CAP`].
    pub(crate) fn max_stdout(mut self, bytes: usize) -> Self {
        self.limit = Some((bytes, Overflow::Error));
        self
    }

    /// Keep only the first `bytes` of stdout: a READ is killed there and the
    /// call still succeeds, reporting the cut ([`LocalGit::exec_truncated`]).
    pub(crate) fn truncate_stdout(mut self, bytes: usize) -> Self {
        self.limit = Some((bytes, Overflow::Truncate));
        self
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

    /// [`Self::paths`] for an UNBOUNDED path list ("Stage all", a folder's
    /// checkbox): the names go NUL-separated over stdin
    /// (`--pathspec-from-file=- --pathspec-file-nul`) instead of argv, which
    /// dies with E2BIG around 15k paths (ARG_MAX is 1 MB on macOS). Same
    /// literal-name semantics. Only for subcommands that take the flag
    /// (`add`, `restore`, `reset`, `rm`, `checkout`…) — NOT `clean`.
    pub(crate) fn paths_stdin<I, S>(mut self, paths: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut buf = Vec::new();
        for p in paths {
            buf.extend_from_slice(p.as_ref().as_bytes());
            buf.push(0);
        }
        self.args.push("--pathspec-from-file=-".to_string());
        self.args.push("--pathspec-file-nul".to_string());
        if !self.envs.iter().any(|(k, _)| k == "GIT_LITERAL_PATHSPECS") {
            self.envs
                .push(("GIT_LITERAL_PATHSPECS".to_string(), "1".to_string()));
        }
        self.stdin = Some(buf);
        self
    }

    /// [`Self::paths`] when `path` is `Some`, unchanged otherwise.
    pub(crate) fn maybe_path(self, path: Option<&str>) -> Self {
        match path {
            Some(p) => self.paths([p]),
            None => self,
        }
    }

    /// [`Self::paths`] when `paths` is non-empty, unchanged otherwise.
    pub(crate) fn maybe_paths(self, paths: &[&str]) -> Self {
        if paths.is_empty() {
            self
        } else {
            self.paths(paths.iter().copied())
        }
    }

    fn argv(&self) -> Vec<&str> {
        self.args.iter().map(String::as_str).collect()
    }
}

/// Per-spawn argv budget for a path list that can't go over stdin (`git
/// clean`): well under macOS's 1 MB ARG_MAX, which also holds the env.
pub(crate) const CLEAN_ARGV_BYTES: usize = 256 * 1024;

/// `paths` split into consecutive batches of at most `max_bytes` of argv
/// (each name + its NUL); a single longer name still gets its own batch.
pub(crate) fn argv_batches(paths: &[String], max_bytes: usize) -> Vec<&[String]> {
    let mut out = Vec::new();
    let (mut start, mut bytes) = (0, 0);
    for (i, p) in paths.iter().enumerate() {
        let n = p.len() + 1;
        if i > start && bytes + n > max_bytes {
            out.push(&paths[start..i]);
            start = i;
            bytes = 0;
        }
        bytes += n;
    }
    if start < paths.len() {
        out.push(&paths[start..]);
    }
    out
}

/// Split `-z` output into its non-empty NUL-terminated records.
pub(crate) fn nul_records(out: &str) -> impl Iterator<Item = &str> {
    out.split('\0').filter(|r| !r.is_empty())
}

/// Rename detection on every parsed diff: `-M` with git's default limit
/// PINNED (`-l1000`). A user's `diff.renameLimit=0` (unlimited) otherwise
/// turned one 1,600-move commit into ~2.8 s of O(adds×deletes) similarity
/// scoring; above the limit git skips inexact detection (moves show as
/// delete + add) at ~0.2 s. Worst case at the limit ≈ 1 s, paid once per
/// immutable diff (the route caches it).
pub(crate) const RENAMES: [&str; 2] = ["-M", "-l1000"];

/// Knobs of one `/diff` request beyond its target.
#[derive(Debug, Clone, Default)]
pub struct DiffOpts {
    /// Limit to one file (`-- <path>`).
    pub path: Option<String>,
    /// A rename's origin, passed alongside `path` so `-M` still pairs the two
    /// sides (a lone new path would come back as an add).
    pub old_path: Option<String>,
    /// File list + counts only (`--raw --numstat`); no patch is produced.
    pub summary: bool,
    /// Per-file cap + response budget; `None` = uncapped (internal callers).
    pub caps: Option<crate::parse::DiffCaps>,
}

impl DiffOpts {
    /// The literal pathspecs this request is scoped to (validated).
    fn pathspecs(&self) -> Result<Vec<&str>> {
        let mut v = Vec::new();
        for p in [self.path.as_deref(), self.old_path.as_deref()]
            .into_iter()
            .flatten()
            .filter(|p| !p.is_empty())
        {
            LocalGit::guard_path(p)?;
            if !v.contains(&p) {
                v.push(p);
            }
        }
        Ok(v)
    }
}

impl DiffTarget {
    /// `a..b` / `a...b` for the range targets.
    pub(crate) fn range_arg(&self) -> Option<String> {
        match self {
            Self::Range(a, b) => Some(format!("{a}..{b}")),
            Self::MergeBase(a, b) => Some(format!("{a}...{b}")),
            _ => None,
        }
    }
}

/// The `(label, rev)` integration branches a branch can have "landed in": the
/// cleanup base first, then whichever of `develop` / `main` / `master` exist —
/// the local branch when there is one (a merge not pushed yet still counts),
/// else `origin/<name>`. The base resolves the same way: a default branch
/// that only exists as `origin/<base>` (a `-b develop` clone, a deleted local
/// `main`) is walked as `origin/<base>`, never as a bare name git can't
/// resolve. A label whose `origin/`-stripped name is already present is not
/// added twice (`origin/develop` as the base hides the `develop` candidate).
/// Read off the `for-each-ref` listings already in hand (`refs_with_base`),
/// so it costs no extra spawn.
fn integration_bases(
    base: Option<&str>,
    local_out: &str,
    remote_out: &str,
) -> Vec<(String, String)> {
    let local_has = |name: &str| listed_oid_local(local_out, name).is_some();
    let remote_has =
        |name: &str| listed_oid_remote(remote_out, &format!("origin/{name}")).is_some();
    let mut out: Vec<(String, String)> = base
        .map(|b| {
            let rev = if !local_has(b) && remote_has(b) {
                format!("origin/{b}")
            } else {
                b.to_string()
            };
            (b.to_string(), rev)
        })
        .into_iter()
        .collect();
    for cand in ["develop", "main", "master"] {
        if out.iter().any(|(label, _)| integration_name(label) == cand) {
            continue;
        }
        if local_has(cand) {
            out.push((cand.to_string(), cand.to_string()));
        } else if remote_has(cand) {
            out.push((cand.to_string(), format!("origin/{cand}")));
        }
    }
    out
}

/// An integration label with its `origin/` prefix dropped — `origin/develop`
/// and `develop` name the same integration branch.
fn integration_name(label: &str) -> &str {
    label.strip_prefix("origin/").unwrap_or(label)
}

/// The tip of local branch `name` from the `refs_with_base` local listing
/// (`name TAB upstream TAB HEAD TAB sha TAB track`).
fn listed_oid_local<'a>(local_out: &'a str, name: &str) -> Option<&'a str> {
    local_out.lines().find_map(|l| {
        let mut cols = l.split('\t');
        (cols.next() == Some(name))
            .then(|| cols.nth(2).map(str::trim))
            .flatten()
    })
}

/// The tip of remote-tracking branch `short` (`origin/x`) from the
/// `refs_with_base` remote listing (`refs/remotes/<short> TAB sha`).
fn listed_oid_remote<'a>(remote_out: &'a str, short: &str) -> Option<&'a str> {
    let full = format!("refs/remotes/{short}");
    remote_out.lines().find_map(|l| {
        let (name, sha) = l.split_once('\t')?;
        (name.trim() == full).then(|| sha.trim())
    })
}

/// The commit `rev` names, read from the listings when it is a branch there
/// (local first, as git resolves a bare name) — `None` for anything else.
fn listed_oid<'a>(rev: &str, local_out: &'a str, remote_out: &'a str) -> Option<&'a str> {
    listed_oid_local(local_out, rev).or_else(|| listed_oid_remote(remote_out, rev))
}

/// A full-length (SHA-1 or SHA-256) hex object id.
pub(crate) fn is_full_oid(s: &str) -> bool {
    (s.len() == 40 || s.len() == 64) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Run CPU-bound work (a big parse / serialize) on the blocking pool when the
/// input is large enough to hold a runtime worker for milliseconds; inline
/// otherwise.
pub(crate) async fn off_runtime<T, F>(size: usize, f: F) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    if size < 64 * 1024 {
        return Ok(f());
    }
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| Error::Internal(format!("diff worker: {e}")))
}

/// Summary rows for untracked files (what `Working` renders as fully-added
/// `--no-index` diffs), counted from disk instead of spawning git per file:
/// git's own binary test (a NUL in the first 8,000 bytes) and line count (a
/// final line without `\n` still counts).
fn untracked_summary(root: &Path, files: &[String]) -> Vec<otto_core::api::FileDiff> {
    use std::io::Read;
    files
        .iter()
        .map(|f| {
            let full = root.join(f);
            let (is_binary, lines) = match std::fs::symlink_metadata(&full) {
                // A symlink's "content" is its target: one line.
                Ok(m) if m.file_type().is_symlink() => (false, Some(1)),
                Ok(m) if m.is_file() => match std::fs::File::open(&full) {
                    Ok(mut fh) => {
                        let mut buf = vec![0u8; 64 * 1024];
                        let (mut n, mut last, mut first) = (0u32, b'\n', true);
                        let mut binary = false;
                        loop {
                            let k = match fh.read(&mut buf) {
                                Ok(0) | Err(_) => break,
                                Ok(k) => k,
                            };
                            if first {
                                binary = buf[..k.min(8000)].contains(&0);
                                first = false;
                                if binary {
                                    break;
                                }
                            }
                            n += buf[..k].iter().filter(|b| **b == b'\n').count() as u32;
                            last = buf[k - 1];
                        }
                        if binary {
                            (true, None)
                        } else {
                            (false, Some(n + u32::from(last != b'\n')))
                        }
                    }
                    Err(_) => (false, Some(0)),
                },
                _ => (false, Some(0)),
            };
            otto_core::api::FileDiff {
                fingerprint: String::new(),
                language: crate::parse::lang_from_ext(f),
                path: f.clone(),
                old_path: None,
                is_binary,
                hunks: Vec::new(),
                too_large: None,
                hunks_omitted: Some(true),
                added: lines,
                deleted: lines.map(|_| 0),
                status: Some(otto_core::api::FileChangeStatus::Added),
            }
        })
        .collect()
}

/// Repos with a `commit-graph write` in flight (see [`LocalGit::seed_commit_graph`]).
fn seeding() -> &'static std::sync::Mutex<std::collections::HashSet<PathBuf>> {
    static S: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    S.get_or_init(Default::default)
}

/// A repo's seat in [`seeding`]: taken before the spawn, released on drop
/// (when the write task ends, however it ends).
struct SeedSeat(PathBuf);

impl SeedSeat {
    fn take(repo: &Path) -> Option<Self> {
        let mut s = seeding().lock().unwrap_or_else(|p| p.into_inner());
        s.insert(repo.to_path_buf())
            .then(|| SeedSeat(repo.to_path_buf()))
    }
}

impl Drop for SeedSeat {
    fn drop(&mut self) {
        seeding()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&self.0);
    }
}

/// A handle on one local repository; every method spawns `git -C <path> …`.
#[derive(Clone)]
pub struct LocalGit {
    repo_path: PathBuf,
    /// The git binary to spawn. Overridable so tests can point at a shim
    /// without touching the process `PATH`.
    git_bin: PathBuf,
    /// Test-only override of BOTH spawn budgets.
    budget_override: Option<std::time::Duration>,
    /// A person asked for this git (the git routes' [`crate::http`] handle).
    /// Only then do [`HOOK_VERBS`] run hooks from a `core.hooksPath` INSIDE
    /// the work tree (husky / lefthook: `.husky/_`), which any agent working
    /// there can edit; daemon-initiated commits / merges / pushes (workflows,
    /// Run with Otto, swarm, scheduled tasks) run with hooks off in that case.
    pub(crate) person_initiated: bool,
}

impl LocalGit {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
            git_bin: PathBuf::from("git"),
            budget_override: None,
            person_initiated: false,
        }
    }

    /// Mark this handle as acting on a person's request (see
    /// `person_initiated`): hooks from an in-work-tree `core.hooksPath` run.
    pub fn person_initiated(mut self) -> Self {
        self.person_initiated = true;
        self
    }

    /// Is `core.hooksPath` set to a directory inside this work tree (where an
    /// agent session can rewrite the hooks)? A relative value is resolved
    /// against the work tree root, so it counts as inside. Errors (no repo,
    /// unset key) are `false`: the repo's own `.git/hooks` is Seatbelt-denied.
    ///
    /// Returns an OWNED future (both commands are built up front) so a
    /// detached write can run the probe inside its own task — see
    /// [`Self::spawn_capture`].
    pub(crate) fn hooks_path_in_worktree(
        &self,
    ) -> impl std::future::Future<Output = bool> + Send + 'static {
        let cmd = |args: &'static [&'static str]| {
            let mut cmd = self.base_cmd();
            // Read the REPO's value: the hardened `core.hooksPath=/dev/null`
            // would otherwise answer for it. Neither `config --get` nor
            // `rev-parse` runs a hook; fsmonitor stays off.
            cmd.env("GIT_CONFIG_PARAMETERS", HOOKED_GIT_CONFIG)
                .args(args)
                .kill_on_drop(true);
            cmd
        };
        async fn run(mut cmd: Command) -> Option<String> {
            let out = tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output())
                .await
                .ok()?
                .ok()?;
            out.status
                .success()
                .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
                .filter(|s| !s.is_empty())
        }
        let (get, top) = (
            cmd(&["config", "--get", "core.hooksPath"]),
            cmd(&["rev-parse", "--show-toplevel"]),
        );
        async move {
            let Some(hooks) = run(get).await else {
                return false;
            };
            hooks_path_inside(&hooks, run(top).await.as_deref())
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
            // The repo's `.git` is writable by the (sandboxed) agent working in
            // it, but THIS git runs unconfined as the user: never let repo
            // config pick a program for it to run (see `HARDENED_GIT_CONFIG`).
            .env("GIT_CONFIG_PARAMETERS", HARDENED_GIT_CONFIG)
            .env("GIT_PAGER", "cat")
            // Force English output: every error classification here
            // (`local_refusal`, "CONFLICT", "has no upstream branch", …) matches
            // git's English wording, which a non-English LANG silently breaks.
            .env("LC_ALL", "C")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        test_hermetic(cmd.as_std_mut());
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
        cmd.args(implicit_config_args(args)).args(args);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = otto_telemetry::context::measure_result(
            "git.spawn",
            self.spawn_output(cmd, class, verb_of(args), None),
        )
        .await?;
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
        cmd.args(implicit_config_args(args)).args(args);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = otto_telemetry::context::measure_result(
            "git.spawn",
            self.spawn_output(cmd, class, verb_of(args), None),
        )
        .await?;
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
        cmd.args(implicit_config_args(args)).args(args);
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
        let stdin = stdin.or(c.stdin.as_deref());
        let (out, _) = self
            .spawn_limited(cmd, c.class, verb_of(&argv), stdin, StdoutLimit::of(c))
            .await?;
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Ok((out.status.success(), out.stdout, stderr, out.status.code()))
    }

    /// [`Self::exec_bytes`] for a command built with
    /// [`GitCmd::truncate_stdout`] (or a [`StopFn`] via `stop`): stdout up to
    /// the limit, and whether git was cut off there. A non-zero exit that we
    /// did not cause is still an error.
    pub(crate) async fn exec_truncated(
        &self,
        c: &GitCmd,
        stop: Option<StopFn>,
    ) -> Result<(Vec<u8>, bool)> {
        self.check_repo().await?;
        let argv = c.argv();
        let mut cmd = self.base_cmd();
        cmd.args(&argv);
        for (k, v) in &c.envs {
            cmd.env(k, v);
        }
        let mut limit = StdoutLimit::of(c);
        limit.stop = stop;
        let (out, cut) = self
            .spawn_limited(cmd, c.class, verb_of(&argv), c.stdin.as_deref(), limit)
            .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            let err = upstream_err(
                &stderr,
                &String::from_utf8_lossy(&out.stdout),
                out.status.code(),
            );
            tracing::warn!(
                repo = %self.repo_path.display(),
                args = ?c.args,
                code = out.status.code(),
                "git failed: {err}"
            );
            return Err(err);
        }
        Ok((out.stdout, cut))
    }

    /// [`Self::exec`] where a non-zero exit is an error, classified exactly
    /// like [`Self::run`]. Returns stdout as raw bytes.
    pub(crate) async fn exec_bytes(&self, c: &GitCmd) -> Result<Vec<u8>> {
        let (ok, stdout, stderr, code) =
            otto_telemetry::context::measure_result("git.execute", self.exec(c, None)).await?;
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

    /// [`Self::exec_bytes`] for a diff/show that carries [`RENAMES`]: also
    /// notes git's "rename detection was skipped due to too many files"
    /// warning into `ri`, so the response can say its pairing is partial.
    async fn exec_diff(&self, c: &GitCmd, ri: &std::sync::atomic::AtomicBool) -> Result<Vec<u8>> {
        let (ok, stdout, stderr, code) =
            otto_telemetry::context::measure_result("git.execute", self.exec(c, None)).await?;
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
        note_renames(&stderr, ri);
        Ok(stdout)
    }

    /// [`Self::exec_bytes`] for an index-writing command, with the same
    /// bounded `index.lock` retry as [`Self::run_locked`].
    pub(crate) async fn exec_locked(&self, c: &GitCmd) -> Result<()> {
        for attempt in 1u64..=3 {
            let (ok, stdout, stderr, code) =
                otto_telemetry::context::measure_result("git.execute", self.exec(c, None)).await?;
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
        cmd: Command,
        class: SpawnClass,
        verb: &str,
        stdin: Option<&[u8]>,
    ) -> Result<std::process::Output> {
        self.spawn_limited(cmd, class, verb, stdin, StdoutLimit::ceiling())
            .await
            .map(|(out, _)| out)
    }

    /// [`Self::spawn_capture`] with an [`Overflow::Error`] overflow turned
    /// into `Error::Upstream`; `bool` = a stop/truncate cut the output short.
    pub(crate) async fn spawn_limited(
        &self,
        cmd: Command,
        class: SpawnClass,
        verb: &str,
        stdin: Option<&[u8]>,
        limit: StdoutLimit,
    ) -> Result<(std::process::Output, bool)> {
        let (max, overflow) = (limit.max, limit.overflow);
        let (out, cut) = self.spawn_capture(cmd, class, verb, stdin, limit).await?;
        match cut {
            Cut::None => Ok((out, false)),
            Cut::Stopped => Ok((out, true)),
            Cut::Overflow if overflow == Overflow::Truncate => Ok((out, true)),
            Cut::Overflow if class != SpawnClass::LocalRead => {
                // Never killed (see `read_stdout_capped`): the write happened,
                // only its chatter was dropped.
                tracing::warn!(
                    repo = %self.repo_path.display(),
                    verb,
                    "git {verb} stdout passed {max} bytes; the surplus was dropped"
                );
                Ok((out, true))
            }
            Cut::Overflow => Err(Error::Upstream(format!(
                "git {verb} produced more than {} KB of output and was stopped — \
                 narrow the request (a path, a smaller range or page)",
                max / 1024
            ))),
        }
    }

    /// The one place a git process is created. Every spawn gets its own process
    /// GROUP (`setpgid(0,0)`) so a timeout can signal git AND the `ssh` /
    /// `git-remote-https` helpers it forked with a single `kill(-pid)`.
    ///
    /// `LocalWrite`/`Remote` run inside a detached task: dropping the request
    /// future (client disconnect — the UI passes an `AbortSignal` on every
    /// call) then stops the WAIT, not the git. `kill_on_drop` stays armed as a
    /// last-resort guard for runtime shutdown.
    ///
    /// stdout is read incrementally against `limit` (never more than
    /// [`GIT_STDOUT_CAP`]) instead of `wait_with_output`'s unbounded buffer;
    /// a cut READ is killed on the spot. A cut output reports a zero exit
    /// status — the caller asked for the prefix, and git died by our signal.
    pub(crate) async fn spawn_capture(
        &self,
        mut cmd: Command,
        class: SpawnClass,
        verb: &str,
        stdin: Option<&[u8]>,
        mut limit: StdoutLimit,
    ) -> Result<(std::process::Output, Cut)> {
        cmd.process_group(0).kill_on_drop(true);
        // The hooks-path probe for a hook verb runs INSIDE the (for writes,
        // detached) task below, before the spawn: awaited here, a request
        // dropped during the probe would cancel a commit before it started.
        let probe = (HOOK_VERBS.contains(&verb) && !self.person_initiated)
            .then(|| self.hooks_path_in_worktree());
        let base_config = hardened_config_for(verb);
        let repo = self.repo_path.clone();
        let verb_owned = verb.to_string();
        let stdin = stdin.map(<[u8]>::to_vec);
        limit.max = limit.max.min(GIT_STDOUT_CAP);
        // The spawned git's pid, for the timeout's group kill (0 = not yet).
        let pid_cell = std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0));
        let pid_seen = pid_cell.clone();
        let collect = async move {
            let in_worktree = match probe {
                Some(p) => p.await,
                None => false,
            };
            let config = if in_worktree {
                tracing::info!(
                    repo = %repo.display(),
                    verb = %verb_owned,
                    "core.hooksPath lies inside the work tree; daemon-initiated git runs without hooks"
                );
                HARDENED_GIT_CONFIG
            } else {
                base_config
            };
            cmd.env("GIT_CONFIG_PARAMETERS", config);
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
            let mut child = cmd.spawn().map_err(|e| SpawnErr::Spawn(e.to_string()))?;
            // stdin is written INSIDE `collect`, concurrently with the readers and
            // under the time budget: written up front, a large patch could block
            // forever on a full pipe (git waiting on its own full stdout), and an
            // early git exit surfaced as "Broken pipe" instead of git's stderr.
            let stdin_pipe = stdin.map(|bytes| (child.stdin.take().expect("piped stdin"), bytes));
            let pid = child.id().expect("spawned") as libc::pid_t;
            pid_cell.store(pid, std::sync::atomic::Ordering::SeqCst);
            let kill_pid = (class == SpawnClass::LocalRead).then_some(pid);
            let stdout = child.stdout.take().expect("piped stdout");
            let stderr = child.stderr.take().expect("piped stderr");
            let feed = async move {
                use tokio::io::AsyncWriteExt;
                let Some((mut si, bytes)) = stdin_pipe else {
                    return Ok(());
                };
                match si.write_all(&bytes).await {
                    // git exited (or closed stdin) before reading it all: its
                    // exit status + stderr below are the real error.
                    Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
                    other => other,
                }
                // `si` drops here → EOF; git blocks reading otherwise.
            };
            let (fed, so, se) = tokio::join!(
                feed,
                read_stdout_capped(stdout, &mut limit, kill_pid),
                read_stderr_capped(stderr)
            );
            let (stdout, cut) = so.map_err(SpawnErr::Io)?;
            let stderr = se.map_err(SpawnErr::Io)?;
            fed.map_err(SpawnErr::Io)?;
            let mut status = child.wait().await.map_err(SpawnErr::Io)?;
            if cut != Cut::None && kill_pid.is_some() {
                use std::os::unix::process::ExitStatusExt;
                status = std::process::ExitStatus::from_raw(0);
            }
            Ok::<_, SpawnErr>((
                std::process::Output {
                    status,
                    stdout,
                    stderr,
                },
                cut,
            ))
        };
        let lift = |r: std::result::Result<_, SpawnErr>| {
            r.map_err(|e| match e {
                SpawnErr::Spawn(m) => Error::Internal(format!("spawn git: {m}")),
                SpawnErr::Io(e) => io_err(e),
            })
        };
        let budget = self.budget_override.unwrap_or_else(|| budget_for(class));
        let secs = budget.as_secs();
        let waited = match class {
            SpawnClass::LocalRead => tokio::time::timeout(budget, collect).await.map(lift),
            SpawnClass::LocalWrite | SpawnClass::Remote => {
                // Detached: dropping the request future stops the WAIT, not
                // the probe + git (a commit already started must finish).
                let jh = tokio::spawn(collect);
                tokio::time::timeout(budget, async move {
                    match jh.await {
                        Ok(r) => lift(r),
                        Err(e) => Err(Error::Internal(format!("git task: {e}"))),
                    }
                })
                .await
            }
        };
        match waited {
            Ok(r) => r,
            Err(_) => {
                let pid = pid_seen.load(std::sync::atomic::Ordering::SeqCst);
                if pid > 0 {
                    kill_group(pid).await;
                }
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

    /// Status for a RESPONSE: untracked rows capped at
    /// [`crate::parse::UNTRACKED_ROW_CAP`] (with `untracked_total` /
    /// `untracked_truncated` set past it). Internal callers that act on
    /// untracked paths use [`Self::status_full`].
    pub async fn status(&self) -> Result<RepoStatusResp> {
        self.status_with_cap(crate::parse::UNTRACKED_ROW_CAP).await
    }

    /// Every untracked row, uncapped — for callers that classify specific
    /// paths (discard must still find the 6,000th untracked file).
    pub async fn status_full(&self) -> Result<RepoStatusResp> {
        self.status_with_cap(usize::MAX).await
    }

    async fn status_with_cap(&self, cap: usize) -> Result<RepoStatusResp> {
        // `--untracked-files=all` lists every untracked FILE individually instead
        // of collapsing an entirely-new directory (e.g. `.claude/skills/` with
        // 80+ files) into a single entry — so the Changes view can show/stage
        // them per-file. Gitignored paths are still excluded.
        //
        // `-z`: without it git C-quotes any name with a non-ASCII byte, `"` or
        // `\` (`"caf\303\251.txt"`), and that quoted form is what came back
        // to stage/discard — `add` then failed "did not match", and `clean`
        // exited 0 having deleted NOTHING while the UI said "Discarded".
        let mut args: Vec<&str> = Vec::new();
        if self.large_index_accel().await {
            args.extend([
                "-c",
                "core.untrackedCache=true",
                "-c",
                "core.fsmonitor=true",
            ]);
        }
        args.extend([
            "status",
            "--porcelain=v2",
            "--branch",
            "-z",
            "--untracked-files=all",
            STATUS_IGNORE_SUBMODULES,
        ]);
        let out = self.exec_text(&GitCmd::read(&args)).await?;
        // 100k rows is tens of ms of parsing — off the async workers.
        let mut st = off_runtime(out.len(), move || {
            crate::parse::parse_status_capped(&out, cap)
        })
        .await?;
        st.op_in_progress = self.op_in_progress().await.map(str::to_string);
        Ok(st)
    }

    /// Opt-in (`OTTO_GIT_FSMONITOR=1`) status acceleration for big repos:
    /// `-c core.untrackedCache=true -c core.fsmonitor=true` on the status
    /// argv when the index is over ~6 MB (≈50k entries). Never written to the
    /// repo config (see the `core.fsmonitor` guard on `.git/config` writes).
    ///
    /// Not on by default: both caches live in the INDEX, and Otto's status
    /// runs with `GIT_OPTIONAL_LOCKS=0` (it must never take `index.lock` from
    /// under an agent's commit), so it can't persist the fsmonitor token or
    /// the untracked cache — the gain only appears when the user's own git
    /// writes the index with them. Skipped when the repo config already
    /// names an fsmonitor (a watchman hook must not be overridden) and on a
    /// git without the built-in daemon (< 2.37 would run `true` as a hook).
    async fn large_index_accel(&self) -> bool {
        if std::env::var("OTTO_GIT_FSMONITOR").ok().as_deref() != Some("1") {
            return false;
        }
        let Some(gd) = self.git_dir().await else {
            return false;
        };
        let big = tokio::fs::metadata(gd.join("index"))
            .await
            .is_ok_and(|m| m.len() > 6 << 20);
        if !big {
            return false;
        }
        let cfg = tokio::fs::read_to_string(gd.join("config"))
            .await
            .unwrap_or_default();
        if cfg.to_ascii_lowercase().contains("fsmonitor") {
            return false;
        }
        static BUILTIN: tokio::sync::OnceCell<bool> = tokio::sync::OnceCell::const_new();
        let bin = self.git_bin.clone();
        *BUILTIN
            .get_or_init(|| async move {
                let out = Command::new(bin).arg("--version").output().await;
                out.ok()
                    .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
                    .and_then(|v| {
                        let ver = v.split_whitespace().nth(2)?.to_string();
                        let mut it = ver.split('.').map(|p| p.parse::<u32>().unwrap_or(0));
                        Some((it.next()?, it.next()?))
                    })
                    .is_some_and(|(maj, min)| (maj, min) >= (2, 37))
            })
            .await
    }

    /// Tracked changes only (`--untracked-files=no`) — for internal callers
    /// that never look at untracked files (rename origins, conflicted paths,
    /// "is the tracked tree dirty"). Enumerating every untracked file costs a
    /// full directory walk, and these ran on every unstage / merge commit /
    /// pull, two or three times each.
    async fn tracked_status(&self) -> Result<RepoStatusResp> {
        let out = self
            .exec_text(&GitCmd::read(&[
                "status",
                "--porcelain=v2",
                "-z",
                "--untracked-files=no",
                STATUS_IGNORE_SUBMODULES,
            ]))
            .await?;
        Ok(crate::parse::parse_status(&out))
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
    /// `merge_abort` may discard with `reset --merge`). SQUASH_MSG is the only
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
                // `lstrip=2`, not `:short`: a branch named like a tag
                // abbreviates to `heads/<name>` (see `refs_with_base`).
                "--format=%(refname:lstrip=2)%09%(upstream:short)%09%(HEAD)",
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

    /// True when the remote-tracking branch `origin/<branch>` exists. A
    /// probe, not a failure: `--quiet` + the raw runner keep a missing ref (the
    /// workflow reaper asking about an already-deleted `otto-wf/*` branch) out
    /// of the "git failed … code 128" warn log.
    pub async fn origin_branch_exists(&self, branch: &str) -> bool {
        if Self::guard_ref(branch).is_err() {
            return false;
        }
        let refname = format!("refs/remotes/origin/{branch}");
        matches!(
            self.run_raw(&["rev-parse", "--verify", "--quiet", &refname], &[])
                .await,
            Ok((true, _, _, _))
        )
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
    /// 3. Neither exists → create a new branch from `base` with `-b`.
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
            Self::guard_ref(path)?;
            Self::guard_ref(branch)?;
            Self::guard_ref(base)?;
            // The existence probe can fail or race another creator. `-b`
            // refuses an existing branch atomically; `-B` would reset its work.
            self.run(&["worktree", "add", "-b", branch, path, base])
                .await?;
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
        // Only probe trees that really belong to THIS repo: an agent can
        // rewrite a `worktrees/<id>/gitdir` pointer (or a tree's `.git` file)
        // to aim the probe's unconfined `git status` at a repo it built.
        let common = self
            .run_read(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .await
            .ok()
            .map(|s| PathBuf::from(s.trim()));
        crate::worktree_probe::probe(&mut rows, admission, phase_budget, move |path| {
            let owned = common
                .as_deref()
                .is_some_and(|c| worktree_round_trips(Path::new(&path), c));
            let git = LocalGit::new(path)
                .with_git_bin(bin.clone())
                .with_budget(probe_budget);
            async move {
                if !owned {
                    return None;
                }
                // The subprocess itself checks the worktree. Avoid a separate
                // filesystem metadata await before this optional bounded probe.
                let mut cmd = git.base_cmd();
                cmd.args([
                    "status",
                    "--porcelain",
                    "--untracked-files=normal",
                    STATUS_IGNORE_SUBMODULES,
                ]);
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
        // A NETWORK op (`--init` clones each submodule): the remote budget,
        // not the 30 s local-write one that killed any clone running longer
        // and left a half-written `.git/modules/<x>` behind. No askpass
        // token — a submodule's host need not be the repo's, and the bound
        // account's credential must not be offered to it.
        let (out, err) = self.run_env_class(&args, &[], SpawnClass::Remote).await?;
        let msg = if out.trim().is_empty() { err } else { out };
        Ok(msg.trim().to_string())
    }

    /// Read history. `limit == 0` means NO `-n` at all — the caller wants the
    /// whole reachable history, not a page of it.
    pub async fn log(&self, limit: u32, skip: u32, all: bool) -> Result<Vec<CommitInfo>> {
        let limit_s = limit.to_string();
        let skip_s = skip.to_string();
        // `--no-show-signature`: a user's `log.showSignature=true` prints gpg
        // output to stdout AHEAD of each signed record, corrupting the parsed
        // SHA/parents (and running gpg once per commit).
        let mut args = vec![
            "log",
            "--no-show-signature",
            "--pretty=format:%H%x1f%h%x1f%an%x1f%aI%x1f%s%x1f%P%x1f%D%x1e",
            "--skip",
            &skip_s,
        ];
        if limit > 0 {
            args.splice(3..3, ["-n", limit_s.as_str()]);
        }
        if all {
            // `--date-order`: never a parent before ALL of its children, even
            // with clock skew / rewritten committer dates — the graph's lane
            // layout assumes children come first.
            args.splice(1..1, ["--all", "--date-order"]);
        }
        let out = match self.run_read(&args).await {
            Ok(out) => out,
            // A fresh repo before its first commit has no history — an answer,
            // not git's "does not have any commits yet" as an error. Checked
            // only on failure, so a normal page costs no extra spawn.
            Err(_) if !all && !self.head_exists().await => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        crate::parse::parse_log(&out)
    }

    pub async fn refs(&self) -> Result<RefsResp> {
        self.refs_with_base(None).await
    }

    /// List refs, flagging every local/remote branch already contained in the
    /// cleanup base branch (`base_override` if valid, else the detected default).
    /// Containment is computed with one bulk `git for-each-ref --merged=<base>`
    /// per integration branch — never a per-branch merge-base spawn. The base
    /// branch itself is never flagged (it always "contains" itself). `base_branch`
    /// in the response echoes the resolved base so the UI can exclude/label it.
    pub async fn refs_with_base(&self, base_override: Option<&str>) -> Result<RefsResp> {
        // The three listings are independent reads — one round of spawns, not
        // three in a row (this runs on every 30 s auto-fetch).
        let (local_out, remote_out, tags_out, base) = tokio::join!(
            // Local branches: name TAB upstream TAB HEAD-marker TAB sha TAB tracking
            self.run_read(&[
                "for-each-ref",
                "--format=%(refname:lstrip=2)\t%(upstream:short)\t%(HEAD)\t%(objectname)\t%(upstream:track,nobracket)",
                "refs/heads",
            ]),
            // Remote branches: full refname TAB sha.
            self.run_read(&[
                "for-each-ref",
                "--format=%(refname)\t%(objectname)",
                "refs/remotes",
            ]),
            // Tags: sorted newest-first, ALL of them (an old tag is precisely
            // the one a user reaches for). `%(*objectname)` is the dereferenced
            // commit and is non-empty only for ANNOTATED tags.
            self.run_read(&[
                "for-each-ref",
                "--sort=-creatordate",
                "--format=%(refname:short)\t%(objectname)\t%(*objectname)",
                "refs/tags",
            ]),
            // Resolve the base up front so each branch row is a cheap set
            // lookup. A missing base (empty repo) leaves the sets empty.
            self.resolve_cleanup_base(base_override),
        );
        let (local_out, remote_out, tags_out) = (local_out?, remote_out?, tags_out?);
        let integrations = integration_bases(base.as_deref(), &local_out, &remote_out);
        let landed_sets = futures_util::future::join_all(
            integrations
                .iter()
                .map(|(_, rev)| self.merged_sets_cached(Some(rev), &local_out, &remote_out)),
        )
        .await;
        // The base is always first in `integrations` when there is one.
        let empty = Default::default();
        let (merged_local, merged_remote) = match base {
            Some(_) => landed_sets.first().unwrap_or(&empty),
            None => &empty,
        };
        let is_integration = |name: &str| {
            integrations
                .iter()
                .any(|(label, _)| integration_name(label) == name)
        };
        // Labels of every integration branch holding `name` (none for an
        // integration branch itself — main ⊂ develop is not a cleanup hint).
        let landed_in = |name: &str, remote: bool| -> Vec<String> {
            let own = if remote {
                name.split_once('/').map_or(name, |(_, b)| b)
            } else {
                name
            };
            if is_integration(own) {
                return Vec::new();
            }
            integrations
                .iter()
                .zip(&landed_sets)
                .filter(|(_, (l, r))| if remote { r } else { l }.contains(name))
                .map(|((label, _), _)| label.clone())
                .collect()
        };

        let local = local_out
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|line| {
                let mut cols = line.splitn(5, '\t');
                let name = cols.next().unwrap_or("").to_string();
                let upstream_raw = cols.next().unwrap_or("").trim().to_string();
                let head = cols.next().unwrap_or("").trim();
                let sha = cols.next().unwrap_or("").trim().to_string();
                let (ahead, behind) =
                    crate::parse::parse_upstream_tracking(cols.next().unwrap_or(""));
                let merged =
                    base.as_deref() != Some(name.as_str()) && merged_local.contains(name.as_str());
                let merged_into = landed_in(&name, false);
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
                    merged_into,
                    sha,
                }
            })
            .collect();

        // Remote branches: skip the symbolic `refs/remotes/<remote>/HEAD`.
        // Filter on the FULL name — `:short` abbreviates `origin/HEAD` to just
        // "origin", which slipped past a "/HEAD" suffix check and showed up as
        // a stray "origin" branch.
        let remote = remote_out
            .lines()
            .filter(|l| {
                let full = l.split('\t').next().unwrap_or("").trim();
                !l.trim().is_empty() && !full.ends_with("/HEAD")
            })
            .map(|line| {
                let mut cols = line.splitn(2, '\t');
                let full = cols.next().unwrap_or("").trim();
                let name = full
                    .strip_prefix("refs/remotes/")
                    .unwrap_or(full)
                    .to_string();
                let sha = cols.next().unwrap_or("").trim().to_string();
                // Don't flag the base's own remote twin (origin/<base>) as safe.
                let is_base_remote = base
                    .as_deref()
                    .is_some_and(|b| name.strip_prefix("origin/") == Some(b));
                let merged = !is_base_remote && merged_remote.contains(name.as_str());
                let merged_into = landed_in(&name, true);
                RefBranch {
                    name,
                    is_current: false,
                    upstream: None,
                    remote: true,
                    ahead: 0,
                    behind: 0,
                    merged_into_base: merged,
                    merged_into,
                    sha,
                }
            })
            .collect();

        // Fall back to `%(objectname)` for lightweight tags — either way `sha`
        // names a commit, never a tag object.
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

    /// [`Self::merged_sets`] memoized per repo on the ref state: the base's
    /// commit id plus every branch tip (the `for-each-ref` listings already in
    /// hand). `--merged` walks history once per branch tip — O(branches ×
    /// merge-base) — and the Git page asks every 30 s; while nothing moved
    /// the answer is the same, so only a fetch/commit/checkout that changes a
    /// tip (or the base) pays for it again. A base that is a listed branch
    /// (every integration branch) is already hashed with the listings, so the
    /// memo check costs no spawn; only an override naming a tag or a sha is
    /// resolved first.
    async fn merged_sets_cached(
        &self,
        base: Option<&str>,
        local_out: &str,
        remote_out: &str,
    ) -> (
        std::collections::HashSet<String>,
        std::collections::HashSet<String>,
    ) {
        type Sets = (
            std::collections::HashSet<String>,
            std::collections::HashSet<String>,
        );
        // Keyed per (repo, base): `refs_with_base` asks for every integration
        // branch, and a single per-repo slot would evict between them.
        type Memo = std::collections::HashMap<(PathBuf, String), (u64, Sets)>;
        static MEMO: std::sync::OnceLock<std::sync::Mutex<Memo>> = std::sync::OnceLock::new();
        let Some(b) = base else {
            return (Default::default(), Default::default());
        };
        let base_oid = match listed_oid(b, local_out, remote_out) {
            Some(oid) => oid.to_string(),
            // An unresolvable base can't be walked either: skip the doomed
            // `--merged` spawn instead of repeating it on every poll.
            None => match self.resolve_commit(b).await {
                Ok(oid) => oid,
                Err(_) => return (Default::default(), Default::default()),
            },
        };
        let state = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            (b, &base_oid, local_out, remote_out).hash(&mut h);
            h.finish()
        };
        let memo = MEMO.get_or_init(Default::default);
        if let Some((k, sets)) = memo
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&(self.repo_path.clone(), b.to_string()))
        {
            if *k == state {
                return sets.clone();
            }
        }
        let (sets, complete) = self.merged_sets(base).await;
        // A failed `--merged` walk (timeout, lock) is never memoized: its
        // empty set would stick until a ref moved.
        if complete {
            memo.lock().unwrap_or_else(|p| p.into_inner()).insert(
                (self.repo_path.clone(), b.to_string()),
                (state, sets.clone()),
            );
        }
        sets
    }

    /// The `(local, remote)` sets of branch short-names whose tip is contained in
    /// `base` — ONE bulk `git for-each-ref --merged` over both namespaces.
    /// Empty when `base` is `None` or the git call fails (best-effort: cleanup
    /// hints must never break `refs`).
    async fn merged_sets(
        &self,
        base: Option<&str>,
    ) -> (
        (
            std::collections::HashSet<String>,
            std::collections::HashSet<String>,
        ),
        bool,
    ) {
        let Some(base) = base else {
            return (Default::default(), true);
        };
        let merged = format!("--merged={base}");
        let out = self
            .run_read(&[
                "for-each-ref",
                &merged,
                "--format=%(refname)",
                "refs/heads",
                "refs/remotes",
            ])
            .await;
        let complete = out.is_ok();
        let mut local = std::collections::HashSet::new();
        let mut remote = std::collections::HashSet::new();
        for full in out.unwrap_or_default().lines().map(str::trim) {
            if let Some(name) = full.strip_prefix("refs/heads/") {
                local.insert(name.to_string());
            } else if let Some(name) = full.strip_prefix("refs/remotes/") {
                // Skip the symbolic `<remote>/HEAD`.
                if !name.ends_with("/HEAD") {
                    remote.insert(name.to_string());
                }
            }
        }
        ((local, remote), complete)
    }

    /// Compute a diff for `target`. When `pathspec` is `Some(path)`, every git
    /// invocation is scoped to that single file (`-- <path>`) — so selecting one
    /// file in the UI computes ONLY that file's diff instead of the entire
    /// working tree (which, for the `Working` target, also runs a `--no-index`
    /// diff per untracked file — seconds of work on a large changeset). `None`
    /// returns the full diff (the "All changes" view, commit/range views).
    ///
    /// Uncapped: internal consumers (proof packs, reviews, goal loops) read
    /// every line. The HTTP route goes through [`Self::diff_with`] with caps.
    pub async fn diff(&self, target: DiffTarget, pathspec: Option<&str>) -> Result<DiffResp> {
        let opts = DiffOpts {
            path: pathspec.map(str::to_string),
            ..DiffOpts::default()
        };
        self.diff_with(&target, &opts).await
    }

    /// [`Self::diff`] with the `/diff` route's knobs: `old_path` (a rename's
    /// origin, so a per-file request still pairs the two sides), `summary`
    /// (file list + counts from `--raw --numstat`, never the patch) and `caps`
    /// (per-file cap + response budget, applied WHILE parsing).
    pub async fn diff_with(&self, target: &DiffTarget, opts: &DiffOpts) -> Result<DiffResp> {
        let paths = opts.pathspecs()?;
        if opts.summary {
            return self.diff_summary(target, &paths).await;
        }
        let ri = std::sync::atomic::AtomicBool::new(false);
        let out = self.diff_patch_bytes(target, &paths, &ri).await?;
        let captured_bytes = out.len();
        let caps = opts.caps;
        let mut resp = off_runtime(out.len(), move || {
            crate::parse::parse_diff_bytes_capped(&out, caps.as_ref())
        })
        .await?;
        if matches!(target, DiffTarget::Working) {
            self.append_untracked(&paths, &mut resp, caps, captured_bytes)
                .await?;
        }
        if ri.load(std::sync::atomic::Ordering::Relaxed) {
            resp.renames_incomplete = Some(true);
        }
        Ok(resp)
    }

    /// Parse each untracked patch as it arrives, then release its raw bytes.
    /// Keep bounded read-ahead, but stop launching patches once the response
    /// budget is spent. Remaining paths still receive explicit summary rows.
    async fn append_untracked(
        &self,
        paths: &[&str],
        resp: &mut DiffResp,
        caps: Option<crate::parse::DiffCaps>,
        mut captured_bytes: usize,
    ) -> Result<()> {
        use futures_util::StreamExt;
        let files = self.untracked(paths).await?;
        let mut remaining = caps;
        let charge = |remaining: &mut Option<crate::parse::DiffCaps>, parsed: &DiffResp| {
            if let Some(caps) = remaining {
                for line in parsed
                    .files
                    .iter()
                    .flat_map(|f| &f.hunks)
                    .flat_map(|h| &h.lines)
                {
                    caps.total_lines = caps.total_lines.saturating_sub(1);
                    caps.total_bytes = caps.total_bytes.saturating_sub(line.content.len() + 1);
                }
            }
        };
        charge(&mut remaining, resp);
        // A patch adds a prefix byte per rendered line plus file headers.
        // The parser still applies exact rendered line/byte budgets. A child
        // exceeding this safety ceiling becomes an explicit too-large row.
        // Discarded per-file bodies still cost I/O and memory. Bound collection
        // separately from retained rendering, with room for one cut file plus
        // a following small file. Read-ahead adds at most eight bounded children.
        let raw_budget = caps.map(|c| {
            c.total_bytes
                .saturating_mul(2)
                .saturating_add(64 * 1024)
                .min(GIT_STDOUT_CAP)
        });
        let file_capture_limit = caps.map(|c| {
            c.file_bytes
                .saturating_mul(2)
                .saturating_add(64 * 1024)
                .min(GIT_STDOUT_CAP)
        });
        let child_limit = file_capture_limit
            .zip(raw_budget)
            .map(|(file, total)| file.min(total));
        let aggregate_cut = child_limit
            .zip(file_capture_limit)
            .is_some_and(|(child, file)| child < file);
        let mut consumed = 0;
        {
            let mut patches = futures_util::stream::iter(files.clone())
                .map(|f: String| async move {
                    let cmd = GitCmd::diff("diff")
                        .args(["-U3", "--no-index"])
                        .paths(["/dev/null", f.as_str()]);
                    let cmd = match child_limit {
                        Some(limit) => cmd.truncate_stdout(limit),
                        None => cmd,
                    };
                    let mut process = self.base_cmd();
                    process.args(cmd.argv());
                    let (output, cut) = self
                        .spawn_limited(
                            process,
                            SpawnClass::LocalRead,
                            "diff",
                            None,
                            StdoutLimit::of(&cmd),
                        )
                        .await?;
                    // --no-index returns 1 for a normal difference. A killed
                    // capped read is also expected, but all other failures matter.
                    if !cut && !matches!(output.status.code(), Some(0 | 1)) {
                        return Err(upstream_err(
                            &String::from_utf8_lossy(&output.stderr),
                            &String::from_utf8_lossy(&output.stdout),
                            output.status.code(),
                        ));
                    }
                    Ok((output.stdout, cut))
                })
                .buffered(8);
            while resp.truncated != Some(true) {
                if raw_budget.is_some_and(|budget| captured_bytes >= budget) {
                    resp.truncated = Some(true);
                    break;
                }
                let Some(result) = patches.next().await else {
                    break;
                };
                let (bytes, cut) = result?;
                captured_bytes = captured_bytes.saturating_add(bytes.len());
                if caps.is_none() && captured_bytes > GIT_STDOUT_CAP {
                    return Err(Error::Upstream(format!(
                        "working diff exceeds the {GIT_STDOUT_CAP}-byte aggregate patch limit; request individual files"
                    )));
                }
                if cut {
                    let root = self.repo_path.clone();
                    let file = vec![files[consumed].clone()];
                    let mut summary =
                        off_runtime(usize::MAX, move || untracked_summary(&root, &file)).await?;
                    if aggregate_cut {
                        resp.truncated = Some(true);
                    } else {
                        for row in &mut summary {
                            row.too_large = Some(true);
                        }
                    }
                    resp.files.extend(summary);
                } else {
                    let mut parsed = off_runtime(bytes.len(), move || {
                        crate::parse::parse_diff_bytes_capped(&bytes, remaining.as_ref())
                    })
                    .await?;
                    charge(&mut remaining, &parsed);
                    if parsed.truncated == Some(true) {
                        resp.truncated = Some(true);
                    }
                    resp.files.append(&mut parsed.files);
                }
                consumed += 1;
                if raw_budget.is_some_and(|budget| captured_bytes > budget) {
                    resp.truncated = Some(true);
                }
            }
            // Dropping this bounded stream cancels outstanding LocalRead jobs;
            // kill_on_drop terminates their direct child. Explicit capture
            // overflow and timeout paths also terminate the process group.
        }
        if consumed < files.len() {
            let root = self.repo_path.clone();
            let omitted = files[consumed..].to_vec();
            let summary =
                off_runtime(usize::MAX, move || untracked_summary(&root, &omitted)).await?;
            resp.files.extend(summary);
            resp.truncated = Some(true);
        }
        crate::parse::fill_totals(resp);
        Ok(())
    }

    /// git's raw unified diff for `target`, scoped to `paths` (empty = all).
    async fn diff_patch_bytes(
        &self,
        target: &DiffTarget,
        paths: &[&str],
        ri: &std::sync::atomic::AtomicBool,
    ) -> Result<Vec<u8>> {
        let run = |c: GitCmd| async move { self.exec_diff(&c, ri).await };
        // Every call is a [`GitCmd::diff`] — fixed output format, raw UTF-8
        // names (`core.quotePath=false`: the default octal-escaping broke
        // feeding `ls-files` names back into `--no-index`) — scoped with a
        // LITERAL `-- <path>` when one is given (`app/[id]/x.tsx` must not
        // glob-match `app/d/x.tsx` into the same response). Bytes throughout:
        // `parse_diff_bytes` fingerprints exactly what git printed.
        let diff = |sub: &str, extra: &[&str]| -> GitCmd {
            GitCmd::diff(sub)
                .args(extra.iter().copied())
                .maybe_paths(paths)
        };
        let [m, l] = RENAMES;
        let out: Vec<u8> = match target {
            DiffTarget::Worktree => run(diff("diff", &["-U3", m, l])).await?,
            DiffTarget::Working => {
                // Staged + unstaged tracked changes vs HEAD (a staged-new file
                // shows as fully added). Falls back to cached+worktree when HEAD
                // is unborn (no commits yet).
                let (head_ok, head_out, head_err, _) = self
                    .exec(&diff("diff", &["-U3", m, l, "HEAD"]), None)
                    .await?;
                note_renames(&head_err, ri);
                // Untracked patches are streamed/parsed separately by diff_with.
                if head_ok {
                    head_out
                } else {
                    let mut s = run(diff("diff", &["-U3", m, l, "--cached"]))
                        .await
                        .unwrap_or_default();
                    s.extend(run(diff("diff", &["-U3", m, l])).await.unwrap_or_default());
                    s
                }
            }
            DiffTarget::Staged => run(diff("diff", &["-U3", m, l, "--cached"])).await?,
            DiffTarget::Commit(sha) => {
                // `-m --first-parent`: a merge commit's default `git show`
                // output is a combined (--cc) diff — files identical to any
                // parent are omitted (a normal integration merge shows "no
                // changes") and its `@@@` hunks don't parse. Diffing against
                // the first parent yields the reviewable "what this merge
                // brought in" diff; non-merge commits are unaffected.
                run(diff(
                    "show",
                    &[
                        "-m",
                        "--first-parent",
                        "-U3",
                        m,
                        l,
                        "--format=",
                        "--end-of-options",
                        sha,
                    ],
                ))
                .await?
            }
            DiffTarget::Range(..) | DiffTarget::MergeBase(..) => {
                let range = target.range_arg().unwrap_or_default();
                run(diff("diff", &["-U3", m, l, "--end-of-options", &range])).await?
            }
        };
        Ok(out)
    }

    /// The file list of a diff — status, rename origin, binary flag and line
    /// counts from ONE `--raw --numstat -z` pass (plus `ls-files` for the
    /// untracked side of `Working`). git still computes the changes but never
    /// renders or ships a patch: ~40–100 KB for a 100k-line branch instead of
    /// 15–30 MB. Every file comes back `hunks: []`, `hunks_omitted: true`.
    async fn diff_summary(&self, target: &DiffTarget, paths: &[&str]) -> Result<DiffResp> {
        let ri = std::sync::atomic::AtomicBool::new(false);
        let run = |c: GitCmd| {
            let ri = &ri;
            async move { self.exec_diff(&c, ri).await }
        };
        let [m, l] = RENAMES;
        let stat = |sub: &str, extra: &[&str]| -> GitCmd {
            GitCmd::diff(sub)
                .args(["--raw", "--numstat", "-z", m, l])
                .args(extra.iter().copied())
                .maybe_paths(paths)
        };
        let mut untracked = Vec::new();
        let out: Vec<u8> = match target {
            DiffTarget::Worktree => run(stat("diff", &[])).await?,
            DiffTarget::Staged => run(stat("diff", &["--cached"])).await?,
            DiffTarget::Working => {
                let (head_ok, head_out, head_err, _) =
                    self.exec(&stat("diff", &["HEAD"]), None).await?;
                note_renames(&head_err, &ri);
                untracked = self.untracked(paths).await?;
                if head_ok {
                    head_out
                } else {
                    let mut s = run(stat("diff", &["--cached"])).await.unwrap_or_default();
                    s.extend(run(stat("diff", &[])).await.unwrap_or_default());
                    s
                }
            }
            DiffTarget::Commit(sha) => {
                run(stat(
                    "show",
                    &["-m", "--first-parent", "--format=", "--end-of-options", sha],
                ))
                .await?
            }
            DiffTarget::Range(..) | DiffTarget::MergeBase(..) => {
                let range = target.range_arg().unwrap_or_default();
                run(stat("diff", &["--end-of-options", &range])).await?
            }
        };
        let mut resp = crate::parse::parse_raw_numstat(&out);
        if ri.load(std::sync::atomic::Ordering::Relaxed) {
            resp.renames_incomplete = Some(true);
        }
        if !untracked.is_empty() {
            let root = self.repo_path.clone();
            let extra =
                off_runtime(usize::MAX, move || untracked_summary(&root, &untracked)).await?;
            resp.files.extend(extra);
            crate::parse::fill_totals(&mut resp);
        }
        Ok(resp)
    }

    /// Untracked (not ignored) files, optionally limited to `paths`. `-z`:
    /// names arrive raw, never C-quoted.
    async fn untracked(&self, paths: &[&str]) -> Result<Vec<String>> {
        let ls =
            GitCmd::read(&["ls-files", "-z", "--others", "--exclude-standard"]).maybe_paths(paths);
        let (_, out, _, _) = self.exec(&ls, None).await?;
        let out = String::from_utf8_lossy(&out);
        Ok(nul_records(&out).map(str::to_string).collect())
    }

    /// Resolve a caller's rev to its full commit id — the cache key of an
    /// immutable diff. A full-length hex id is taken as-is (git reads it as an
    /// object name, never a ref); anything else costs one `rev-parse`.
    pub async fn resolve_commit(&self, rev: &str) -> Result<String> {
        Self::guard_ref(rev)?;
        if is_full_oid(rev) {
            return Ok(rev.to_ascii_lowercase());
        }
        let spec = format!("{rev}^{{commit}}");
        let out = self
            .run_read(&["rev-parse", "--verify", "--end-of-options", &spec])
            .await?;
        let oid = out.trim();
        if !is_full_oid(oid) {
            return Err(Error::Invalid(format!("not a commit: {rev}")));
        }
        Ok(oid.to_string())
    }

    /// Run `git diff -M <merge-base(base, HEAD)>` — diffs the working tree
    /// (staged + unstaged) against the point this branch FORKED from `base`
    /// and returns the raw unified diff text.
    ///
    /// The merge-base, not `base`'s tip: once `base` moves on after the branch
    /// point, a tip diff shows every commit landed on `base` since as a
    /// reverse change of this branch — reviewers flagged code the branch never
    /// touched and PR drafts described it. Falls back to `base` itself when
    /// there is no common ancestor (unrelated histories, unborn HEAD).
    ///
    /// These texts feed reviews and commit-message drafts, so they carry the
    /// same fixed [`DIFF_FORMAT`] as the parsed diffs: a `diff.external` in the
    /// user's config otherwise handed the reviewers an empty diff. The trailing
    /// `--` pins `base` as a REVISION even when a file shares its name.
    pub async fn diff_text_against(&self, base: &str) -> Result<String> {
        Self::guard_ref(base)?;
        let from = self.fork_point(base).await;
        self.exec_text(&GitCmd::diff("diff").args(["-M", "--end-of-options", from.as_str(), "--"]))
            .await
    }

    /// `git merge-base <base> HEAD`, or `base` itself when git finds none.
    async fn fork_point(&self, base: &str) -> String {
        match self
            .run_read(&["merge-base", "--end-of-options", base, "HEAD"])
            .await
        {
            Ok(out) if is_full_oid(out.trim()) => out.trim().to_string(),
            _ => base.to_string(),
        }
    }

    /// One untracked file as a new-file patch, stdout capped just past the
    /// per-file review cap (`--no-index` exits 1 for a normal difference).
    async fn untracked_patch(&self, f: String) -> Result<(bool, Vec<u8>, String, Option<i32>)> {
        let cmd = GitCmd::diff("diff")
            .args(["-U3", "--no-index"])
            .paths(["/dev/null", f.as_str()])
            .truncate_stdout(REVIEW_UNTRACKED_FILE_CAP + 1);
        self.exec(&cmd, None).await
    }

    /// [`Self::diff_text_against`] plus every untracked (not ignored) file as
    /// a new-file patch — what a LOCAL review must see: a brand-new module the
    /// user hasn't `git add`ed yet is part of the change under review, but a
    /// plain `git diff` never lists it.
    pub async fn review_diff_text(&self, base: &str) -> Result<String> {
        // Bound the untracked tail: an unignored build/vendor dir must not
        // turn a review into a multi-hundred-MB prompt file. The budget is
        // enforced PER FILE (a single 60 MB dump is clipped, not appended
        // whole), the file count is capped, and every file that was clipped
        // or skipped gets a `(diff omitted: …)` line — the marker
        // `otto_review::engine::diff_is_partial` keys on, so a review that
        // never saw a file cannot resolve findings in it.
        const UNTRACKED_BUDGET: usize = 8 * 1024 * 1024;
        const UNTRACKED_FILE_CAP: usize = REVIEW_UNTRACKED_FILE_CAP;
        const UNTRACKED_MAX_FILES: usize = 2_000;
        // Per-file omission blocks past this collapse into one summary line.
        const MAX_OMITTED_BLOCKS: usize = 200;
        let mut out = self.diff_text_against(base).await?;
        let files = self.untracked(&[]).await?;
        let (take, over_count) = files.split_at(files.len().min(UNTRACKED_MAX_FILES));
        let mut omitted: Vec<(&str, &'static str)> = Vec::new();

        // A few `--no-index` spawns in flight at a time (order preserved),
        // each killed past its per-file cap. Owned paths + plain future vecs:
        // a stream closure borrowing `&String` made the whole future
        // non-`Send` (higher-ranked lifetime) for every axum handler above.
        let mut consumed = 0usize;
        'chunks: for chunk in take.chunks(4) {
            let mut futs = Vec::with_capacity(chunk.len());
            for f in chunk {
                futs.push(self.untracked_patch(f.clone()));
            }
            let results = futures_util::future::join_all(futs).await;
            for (f, r) in chunk.iter().zip(results) {
                consumed += 1;
                let remaining = UNTRACKED_BUDGET.saturating_sub(out.len());
                if remaining == 0 {
                    omitted.push((f.as_str(), "review size cap reached"));
                    break 'chunks;
                }
                // --no-index exits 1 for a normal difference.
                let (_, stdout, _, _) = r?;
                let mut patch = String::from_utf8_lossy(&stdout).into_owned();
                let cap = remaining.min(UNTRACKED_FILE_CAP);
                let clipped = patch.len() > cap;
                if clipped {
                    let mut end = cap;
                    while !patch.is_char_boundary(end) {
                        end -= 1;
                    }
                    let end = patch[..end].rfind('\n').map_or(0, |i| i + 1);
                    patch.truncate(end);
                }
                if !patch.is_empty() {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str(&patch);
                }
                if clipped {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str(&format!(
                        "{UNTRACKED_OMITTED_MARKER}untracked file clipped at the review size cap — \
                         read {f} on disk)\n"
                    ));
                }
            }
        }
        for f in &take[consumed..] {
            omitted.push((f.as_str(), "review size cap reached"));
        }
        for f in over_count {
            omitted.push((f.as_str(), "too many untracked files"));
        }
        if !omitted.is_empty() && !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        for (path, why) in omitted.iter().take(MAX_OMITTED_BLOCKS) {
            out.push_str(&format!(
                "--- a/{path}\n+++ b/{path}\n{UNTRACKED_OMITTED_MARKER}{why} — read the file on disk)\n"
            ));
        }
        if omitted.len() > MAX_OMITTED_BLOCKS {
            out.push_str(&format!(
                "{UNTRACKED_OMITTED_MARKER}{} more untracked files not shown — review size cap reached)\n",
                omitted.len() - MAX_OMITTED_BLOCKS
            ));
        }
        Ok(out)
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

    /// [`Self::diff_text_against`] (`base` given) or [`Self::working_diff_text`]
    /// held to its first `max` bytes: git is killed there instead of the whole
    /// patch of a huge worktree being buffered only to be clipped by the
    /// caller (proof packs store at most `STORE_CAP`). `bool` = cut short.
    pub async fn diff_text_capped(&self, base: Option<&str>, max: usize) -> Result<(String, bool)> {
        let cmd = match base {
            Some(b) => {
                Self::guard_ref(b)?;
                let from = self.fork_point(b).await;
                GitCmd::diff("diff").args(["-M", "--end-of-options", from.as_str(), "--"])
            }
            None => GitCmd::diff("diff").args(["-M"]),
        };
        let (out, cut) = self.exec_truncated(&cmd.truncate_stdout(max), None).await?;
        Ok((String::from_utf8_lossy(&out).into_owned(), cut))
    }

    /// [`Self::staged_diff_text`] held to its first `max` bytes (git is killed
    /// there): a commit-message draft keeps 40 KB of the patch, so buffering
    /// a 100 MB staged vendor drop just to clip it is waste. `bool` = cut.
    pub async fn staged_diff_text_capped(&self, max: usize) -> Result<(String, bool)> {
        let cmd = GitCmd::diff("diff").args(["-M", "--cached"]);
        let (out, cut) = self.exec_truncated(&cmd.truncate_stdout(max), None).await?;
        Ok((String::from_utf8_lossy(&out).into_owned(), cut))
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
    /// → apply the captured SHA (including its index), retaining a recovery backup. NEVER pulls, fetches or merges — a branch switch is a switch (the
    /// removed stash·pull·pop gesture pulled here, which turned "go look at
    /// develop" into an unasked-for merge commit).
    ///
    /// Failure contract: a failed switch applies the stash back (tree exactly as
    /// it was); a CONFLICTING restore is a normal outcome (git keeps the entry and
    /// `status()` reports `kind:"conflicted"` rows); a pop that fails for any
    /// other reason keeps the stash and is a 409 telling the user to
    /// `git stash pop`.
    pub async fn checkout_autostash(&self, branch: &str, create: bool) -> Result<CheckoutOutcome> {
        Self::guard_ref(branch)?;
        let dirty = !self
            .run(&["status", "--porcelain", STATUS_IGNORE_SUBMODULES])
            .await?
            .trim()
            .is_empty();
        // A unique message identifies OUR object even if another worktree
        // pushes a stash before we read the shared reflog.
        let ours = if dirty {
            self.stash_save_unique("otto: branch-switch recovery backup")
                .await?
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
        self.resolve_stash_selector(&sha).await?;
        let (ok, out, err, code) = self
            .run_raw_retry_lock(&["stash", "apply", "--index", &sha])
            .await?;
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
            "switched to {branch}, but restoring your stashed changes failed: {line} — your stash is retained; run `git stash pop --index`"
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
        self.exec_locked(&GitCmd::write(&["add"]).paths_stdin(paths))
            .await
    }

    pub async fn unstage(&self, paths: &[String]) -> Result<()> {
        if paths.is_empty() {
            return Err(Error::Invalid("no paths to unstage".into()));
        }
        Self::guard_paths(paths)?;
        let mut expanded = paths.to_vec();
        // A set, and the cheap kind test first: `paths.contains` per change
        // was O(paths × changes) — 400M compares for "Unstage all" at 20k.
        let want: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
        for change in self.tracked_status().await?.changes {
            if change.kind == "renamed" && want.contains(change.path.as_str()) {
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
        self.exec_locked(&cmd.paths_stdin(&expanded)).await
    }

    /// True once HEAD resolves to a commit (false on an unborn branch).
    pub(crate) async fn head_exists(&self) -> bool {
        matches!(
            self.exec(
                &GitCmd::read(&["rev-parse", "-q", "--verify", "HEAD^{commit}"]),
                None
            )
            .await,
            Ok((true, ..))
        )
    }

    /// Discard all working-tree + staged changes for `paths`, reverting them to
    /// their HEAD state. New files (untracked/added) are removed entirely;
    /// everything else (modified/deleted/renamed/conflicted) is restored from
    /// HEAD. Destructive and irreversible — the UI confirms first.
    pub async fn discard(&self, paths: &[String]) -> Result<()> {
        self.discard_with(paths, false).await
    }

    /// [`Self::discard`], or with `keep_staged` only the UNSTAGED side: a
    /// tracked path's worktree goes back to its INDEX version (`restore
    /// --worktree`, staged hunks kept), an untracked file is removed, and a
    /// path with nothing unstaged is left alone. This is what "Discard" means
    /// in the Unstaged list — reverting a partially staged file to HEAD there
    /// threw away the hunks the user had deliberately staged.
    pub async fn discard_with(&self, paths: &[String], keep_staged: bool) -> Result<()> {
        if paths.is_empty() {
            return Err(Error::Invalid("no paths to discard".into()));
        }
        Self::guard_paths(paths)?;
        let want: std::collections::HashSet<&str> = paths.iter().map(String::as_str).collect();
        // Classify each requested path by its current change kind — from the
        // UNCAPPED status: a response's untracked rows stop at 5,000.
        let status = self.status_full().await?;
        let mut restore: Vec<String> = Vec::new(); // tracked → revert to HEAD
        let mut remove: Vec<String> = Vec::new(); // new → delete
        let mut worktree: Vec<String> = Vec::new(); // keep_staged → revert to index
        for c in &status.changes {
            if !want.contains(c.path.as_str()) {
                continue;
            }
            if keep_staged {
                // Conflicted paths are the resolver's business, never a
                // worktree-only discard.
                match c.kind.as_str() {
                    "untracked" => remove.push(c.path.clone()),
                    "conflicted" => {}
                    _ if c.unstaged => worktree.push(c.path.clone()),
                    _ => {}
                }
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
        if restore.is_empty() && remove.is_empty() && worktree.is_empty() {
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
                    .paths_stdin(&restore),
            )
            .await?;
        }
        if !worktree.is_empty() {
            self.exec_locked(&GitCmd::write(&["restore", "--worktree"]).paths_stdin(&worktree))
                .await?;
        }
        if !remove.is_empty() {
            // Unstage first (a staged-new file → untracked), then `clean` removes
            // the untracked files/dirs. `reset` is a no-op for already-untracked.
            let _ = self
                .exec_locked(&GitCmd::write(&["reset", "-q"]).paths_stdin(&remove))
                .await;
            // `clean` has no `--pathspec-from-file`: bounded argv batches.
            for batch in argv_batches(&remove, CLEAN_ARGV_BYTES) {
                self.exec_locked(&GitCmd::write(&["clean", "-fdq"]).paths(batch))
                    .await?;
            }
        }
        Ok(())
    }

    /// Commit staged changes; returns the new HEAD sha.
    ///
    /// Amend with an EMPTY message keeps the previous commit's message
    /// (`--amend --no-edit`) — the "fold staged changes into the last commit"
    /// flow; rejecting it forced users to retype the message.
    /// Commit ONLY `paths` (`git commit --only -- <paths>`), whatever else is
    /// staged: the other staged changes stay staged and out of this commit.
    /// For automated commits (the API client's collection push) that must
    /// never sweep the user's own half-finished staged work along with them.
    /// The paths must already be known to git (stage new files first).
    pub async fn commit_only(&self, message: &str, paths: &[String]) -> Result<String> {
        if message.trim().is_empty() {
            return Err(Error::Invalid("empty commit message".into()));
        }
        if paths.is_empty() {
            return Err(Error::Invalid("no paths to commit".into()));
        }
        Self::guard_paths(paths)?;
        self.exec_locked(&GitCmd::write(&["commit", "-m", message, "--only"]).paths_stdin(paths))
            .await?;
        let sha = self.run_read(&["rev-parse", "HEAD"]).await?;
        Ok(sha.trim().to_string())
    }

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
        // Otto's own launcher files (`.mcp.json`, agy's `.agents/mcp_config.json`,
        // grok's `.grok/config.toml` — secret env included) are never swept in.
        let launcher: Vec<String> = otto_core::paths::LAUNCHER_ARTIFACTS
            .iter()
            .map(|p| format!(":(exclude){p}"))
            .collect();
        let mut args = vec!["add", "-A", "--", "."];
        args.extend(launcher.iter().map(String::as_str));
        args.extend([
            ":(exclude).env",
            ":(exclude).env.*",
            ":(exclude,glob)**/.DS_Store",
        ]);
        self.run(&args).await?;
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
    pub async fn push_branch(
        &self,
        token: Option<TransportCredential>,
        branch: Option<&str>,
    ) -> Result<String> {
        self.push_with(token, branch, false).await
    }

    pub async fn push(&self, token: Option<TransportCredential>) -> Result<String> {
        self.push_with(token, None, false).await
    }

    /// The one push implementation behind [`Self::push`] / [`Self::push_branch`].
    ///
    /// `force_with_lease` is the ONLY way this crate ever overwrites remote
    /// history, and only when the caller asked for it explicitly (the UI asks
    /// the user first). It is `--force-with-lease --force-if-includes`, never a
    /// bare `--force`: the lease refuses when the remote branch moved since our
    /// remote-tracking ref was updated, and `--force-if-includes` additionally
    /// refuses when that tracking ref was refreshed by a fetch whose commits
    /// were never integrated locally — so a background fetch can't turn the
    /// lease into a blind overwrite of someone else's push.
    ///
    /// A non-fast-forward rejection is a 409 the caller can act on ("pull
    /// first" / force with lease), not a 502 provider outage.
    pub async fn push_with(
        &self,
        token: Option<TransportCredential>,
        branch: Option<&str>,
        force_with_lease: bool,
    ) -> Result<String> {
        if let Some(b) = branch {
            Self::guard_ref(b)?;
        }
        if force_with_lease {
            // Forcing a detached HEAD has no branch to lease against.
            let b = match branch {
                Some(b) => b.to_string(),
                None => self.current_branch().await?,
            };
            if b == "HEAD" {
                return Err(Error::Conflict(
                    "HEAD is detached — check out a branch before force pushing".into(),
                ));
            }
        }
        let askpass = match &token {
            Some(t) => Some(AskPass::new(t)?),
            None => None,
        };
        let envs = askpass.as_ref().map(AskPass::envs).unwrap_or_default();
        let force: &[&str] = if force_with_lease {
            &["--force-with-lease", "--force-if-includes"]
        } else {
            &[]
        };
        let mut args = vec!["push"];
        args.extend_from_slice(force);
        // `branch: Some(b)` pushes THAT branch explicitly regardless of what's
        // checked out; `None` is a plain `git push` of the current branch to
        // its configured upstream. The explicit form names `refs/heads/b` (a
        // same-named tag can't make the refspec ambiguous) and sets the
        // upstream when the branch has none: `push origin b` SUCCEEDS without
        // one, so the "no upstream" retry below never ran and a branch the
        // Create-PR flow published was left untracked (no ahead/behind, and
        // Pull died "no tracking information").
        let full_ref = branch.map(|b| format!("refs/heads/{b}"));
        if let (Some(b), Some(full)) = (branch, full_ref.as_deref()) {
            let has_upstream = self
                .run_raw_class(
                    &[
                        "rev-parse",
                        "-q",
                        "--verify",
                        &format!("{full}@{{upstream}}"),
                    ],
                    &[],
                    SpawnClass::LocalRead,
                )
                .await
                .is_ok_and(|(ok, ..)| ok);
            if !has_upstream && self.branch_exists(b).await {
                args.push("--set-upstream");
            }
            args.extend_from_slice(&["origin", full]);
        }
        let (ok, stdout, stderr, code) =
            self.run_raw_class(&args, &envs, SpawnClass::Remote).await?;
        if ok {
            return Ok(combine_push_output(&stdout, &stderr));
        }
        // First push of a fresh branch: set the upstream explicitly.
        if stderr.contains("has no upstream branch") || stderr.contains("--set-upstream") {
            let b = match branch {
                Some(b) => b.to_string(),
                None => self.current_branch().await?,
            };
            let mut args2 = vec!["push"];
            args2.extend_from_slice(force);
            args2.extend_from_slice(&["--set-upstream", "origin", &b]);
            let (ok2, stdout2, stderr2, code2) = self
                .run_raw_class(&args2, &envs, SpawnClass::Remote)
                .await?;
            if ok2 {
                return Ok(combine_push_output(&stdout2, &stderr2));
            }
            return Err(push_err(&stderr2, &stdout2, code2));
        }
        Err(push_err(&stderr, &stdout, code))
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
    pub async fn pull_outcome(&self, token: Option<TransportCredential>) -> Result<PullOutcome> {
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
    pub async fn pull(&self, token: Option<TransportCredential>) -> Result<String> {
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
        token: Option<TransportCredential>,
    ) -> Result<(PullOutcome, Option<String>)> {
        let dirty = !self
            .run(&["status", "--porcelain", STATUS_IGNORE_SUBMODULES])
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

    pub async fn fetch(&self, token: Option<TransportCredential>) -> Result<String> {
        let out = self.run_remote(&["fetch", "--prune"], token).await?;
        self.seed_commit_graph();
        Ok(out)
    }

    /// Fetch a PR/MR's head commit into a private `refs/otto/pr-review/…` ref
    /// and return that ref. A fork PR's
    /// source branch does not exist on `origin`, so `origin/<branch>` cannot
    /// be checked out — but the host publishes the head under a
    /// provider-specific ref: GitHub `refs/pull/N/head`, GitLab
    /// `refs/merge-requests/N/head`. Both are tried (a miss is a cheap
    /// "couldn't find remote ref"). Falls back to `head_sha` when the commit
    /// is already present locally (e.g. Bitbucket, which exposes neither).
    ///
    /// The ref is keyed by PR AND head (`<n>-<sha12>`, see [`pr_review_ref`]):
    /// two reviews of the same PR at different heads no longer overwrite each
    /// other's ref mid-checkout. A fetch that failed for a reason other than
    /// a missing ref (expired token, network) is reported as that upstream
    /// error instead of the misleading "no pull/merge-request ref".
    /// Delete a `refs/otto/pr-review/*` ref [`Self::fetch_pr_head`] created
    /// (S2-309: one per reviewed head, each pinning a fork head's objects
    /// against gc forever). Refuses any other ref — this never touches a
    /// user's branch or tag.
    pub async fn delete_pr_review_ref(&self, name: &str) -> Result<()> {
        if !name.starts_with("refs/otto/pr-review/") || name.contains("..") {
            return Err(Error::Invalid(format!("not a PR review ref: {name}")));
        }
        Self::guard_ref(name)?;
        self.run(&["update-ref", "-d", name]).await.map(|_| ())
    }

    pub async fn fetch_pr_head(
        &self,
        pr_number: u64,
        head_sha: Option<&str>,
        token: Option<TransportCredential>,
    ) -> Result<String> {
        let head_sha = head_sha.map(str::trim).filter(|s| !s.is_empty());
        if let Some(sha) = head_sha {
            Self::guard_ref(sha)?;
        }
        let local = pr_review_ref(pr_number, head_sha);
        let mut last_failure: Option<(String, String, Option<i32>)> = None;
        for remote_ref in [
            format!("refs/pull/{pr_number}/head"),
            format!("refs/merge-requests/{pr_number}/head"),
        ] {
            let spec = format!("+{remote_ref}:{local}");
            match self
                .run_remote_raw(&["fetch", "--no-tags", "origin", &spec], token.clone())
                .await
            {
                Ok((true, ..)) => return Ok(local),
                Ok((false, stdout, stderr, code)) => {
                    if !is_missing_remote_ref(&stderr) {
                        last_failure = Some((stderr, stdout, code));
                    }
                }
                Err(e) => last_failure = Some((e.to_string(), String::new(), None)),
            }
        }
        if let Some(sha) = head_sha {
            let commit = format!("{sha}^{{commit}}");
            if let Ok(full) = self
                .run(&["rev-parse", "--verify", "--quiet", &commit])
                .await
            {
                return Ok(full.trim().to_string());
            }
        }
        if let Some((stderr, stdout, code)) = last_failure {
            return Err(upstream_err(&stderr, &stdout, code));
        }
        Err(Error::Invalid(format!(
            "could not fetch the head of PR #{pr_number} (no pull/merge-request ref and the head commit is not local)"
        )))
    }

    /// One-shot background `commit-graph write --reachable --changed-paths`
    /// for a repo that has no commit-graph yet. After that the per-fetch
    /// `fetch.writeCommitGraph` keeps it current; the changed-path Bloom
    /// filters make `log -- <path>` / `--follow` (file history) and blame
    /// several times faster on big repos. Purely a local cache git itself
    /// maintains — never blocks or fails the caller.
    pub(crate) fn seed_commit_graph(&self) {
        if !commit_graph_enabled() {
            return;
        }
        let git_dir = self.repo_path.join(".git");
        // Linked worktrees have a `.git` FILE; their objects live in the
        // main repo, which seeds its own graph — skip them.
        if !git_dir.is_dir() {
            return;
        }
        let info = git_dir.join("objects").join("info");
        if info.join("commit-graph").exists() || info.join("commit-graphs").exists() {
            return;
        }
        // One write per repo at a time: a first write on a huge history can
        // outlast the 30 s auto-fetch, and log/blame/status seed too now.
        let Some(seat) = SeedSeat::take(&self.repo_path) else {
            return;
        };
        let mut cmd = self.base_cmd();
        // `--split`: later writes (fetch.writeCommitGraph, the next seed)
        // append a layer instead of rewriting the whole graph.
        cmd.args([
            "commit-graph",
            "write",
            "--reachable",
            "--changed-paths",
            "--split",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(false);
        let repo = self.repo_path.clone();
        tokio::spawn(async move {
            let _seat = seat;
            match cmd.status().await {
                Ok(st) if st.success() => {
                    tracing::info!(repo = %repo.display(), "wrote commit-graph")
                }
                Ok(st) => tracing::debug!(repo = %repo.display(), "commit-graph write: {st}"),
                Err(e) => tracing::debug!(repo = %repo.display(), "commit-graph write: {e}"),
            }
        });
    }

    /// Number of commit-graph seeds currently running (tests).
    #[cfg(test)]
    pub(crate) fn seeding_now() -> usize {
        seeding().lock().map(|s| s.len()).unwrap_or(0)
    }

    async fn run_remote(
        &self,
        args: &[&str],
        token: Option<TransportCredential>,
    ) -> Result<String> {
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
        token: Option<TransportCredential>,
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
        Self::guard_ref(name)?;
        let sp = start_point.filter(|s| !s.is_empty());
        if let Some(sp) = sp {
            Self::guard_ref(sp)?;
        }
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
                &["branch", "--track", "--end-of-options", name, &remote]
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
    pub async fn delete_remote_branch(
        &self,
        name: &str,
        token: Option<TransportCredential>,
    ) -> Result<String> {
        self.delete_remote_branch_on("origin", name, token).await
    }

    /// [`Self::delete_remote_branch`] against the NAMED remote
    /// (`git push <remote> --delete <name>`). The remote-ref row for
    /// `upstream/feature-x` must delete `feature-x` on `upstream` — routing it
    /// to origin destroyed the wrong branch. `remote` must be a configured
    /// remote (an unknown name is a 400, never a guess); the caller decides
    /// whether the account token may be offered to that remote's host.
    pub async fn delete_remote_branch_on(
        &self,
        remote: &str,
        name: &str,
        token: Option<TransportCredential>,
    ) -> Result<String> {
        Self::guard_ref(remote)?;
        Self::guard_ref(name)?;
        if !self.remotes().await?.iter().any(|r| r.name == remote) {
            return Err(Error::Invalid(format!("no remote named '{remote}'")));
        }
        let (ok, stdout, stderr, code) = self
            .run_remote_raw(&["push", remote, "--delete", name], token)
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
            .run(&["update-ref", "-d", &format!("refs/remotes/{remote}/{name}")])
            .await;
        if already_gone {
            return Ok(format!(
                "{remote}/{name} was already absent on {remote}; pruned the stale local tracking ref"
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
    pub async fn push_tag(&self, name: &str, token: Option<TransportCredential>) -> Result<String> {
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
    pub async fn delete_remote_tag(
        &self,
        name: &str,
        token: Option<TransportCredential>,
    ) -> Result<String> {
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

    /// Remote-tracking refs that already contain HEAD (`git branch -r
    /// --contains HEAD`), at most [`HEAD_REMOTES_CAP`]. Non-empty ⇒ amending
    /// HEAD rewrites a pushed commit — whatever the branch's upstream says: a
    /// HEAD pushed under another name, or a branch with no upstream at all,
    /// is still published (S15-28). An unborn HEAD has none.
    pub async fn head_remote_refs(&self) -> Result<Vec<String>> {
        let out = match self
            .run_read(&[
                "branch",
                "-r",
                "--contains",
                "HEAD",
                "--format=%(refname:short)%00%(symref)",
            ])
            .await
        {
            Ok(out) => out,
            // No commit yet: nothing can contain it.
            Err(_)
                if self
                    .run_read(&["rev-parse", "--verify", "-q", "HEAD"])
                    .await
                    .is_err() =>
            {
                return Ok(Vec::new())
            }
            Err(e) => return Err(e),
        };
        Ok(out
            .lines()
            .filter_map(|l| l.split_once('\0'))
            // `origin/HEAD` is a symbolic alias of a real ref listed anyway.
            .filter(|(_, symref)| symref.is_empty())
            .map(|(name, _)| name.trim().to_string())
            .filter(|n| !n.is_empty())
            .take(HEAD_REMOTES_CAP)
            .collect())
    }

    /// `git stash list` → parsed entries (read-only). Empty list when there are
    /// no stashes (`git` exits 0 with empty output).
    pub async fn stash_list(&self) -> Result<Vec<StashInfo>> {
        crate::stash_identity::recover_pending(self, &self.repo_path).await?;
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
    /// stash list at execution time. This rejects a stale client selection,
    /// but the returned index can still change before a later Git command.
    /// Apply uses the immutable SHA after this membership check; Drop selects
    /// that SHA again under Git's own reference lock.
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
        self.resolve_stash_selector(sha).await?;
        // Validate membership above, but apply the immutable object ID: another
        // process can renumber stash selectors before this command executes.
        let (ok, out, err, code) = self.run_raw_retry_lock(&["stash", "apply", sha]).await?;
        if ok || out.contains("CONFLICT") {
            return Ok(out.trim().to_string());
        }
        Err(upstream_err(&err, &out, code))
    }

    /// Drop (discard) the stash with commit `sha` without applying it.
    pub async fn stash_drop(&self, sha: &str) -> Result<String> {
        self.resolve_stash_selector(sha).await?;
        crate::stash_identity::drop_selected(self, &self.repo_path, sha).await
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
        let st = self.tracked_status().await?;
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
        let st = self.tracked_status().await?;
        Ok(st
            .changes
            .iter()
            .any(|c| (c.staged || c.unstaged) && c.kind != "untracked"))
    }

    /// Restore the stash after a clean merge. Returns a human note explaining
    /// the restoration outcome and retained recovery backup.
    ///
    /// `sha` is the auto-stash's own commit ([`Self::stash_save_sha`]): the pop
    /// is addressed by it, never by `stash@{0}` — an agent in the same repo
    /// may have pushed its own stash on top in the meantime.
    pub(crate) async fn pop_after_merge(&self, sha: &str) -> Option<String> {
        match self.pop_stash_sha(sha).await {
            Ok(out) if !out.contains("CONFLICT") => Some(
                "Your stashed changes were restored; a recovery backup remains in Stashes.".into(),
            ),
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
    /// Captured through a unique reflog marker, never a later `refs/stash` read.
    pub(crate) async fn stash_save_sha(&self) -> Result<String> {
        self.stash_save_unique("otto: automatic recovery backup")
            .await?
            .ok_or_else(|| {
                Error::Invalid("nothing to stash — the working tree has no local changes".into())
            })
    }

    /// Save tracked and untracked changes with a unique marker. `refs/stash`
    /// alone is not identity: another worktree can push before the next read.
    async fn stash_save_unique(&self, label: &str) -> Result<Option<String>> {
        let marker = format!("{label} {}", otto_core::new_id());
        let (ok, out, err, code) = self
            .run_raw_retry_lock(&["stash", "push", "--include-untracked", "-m", &marker])
            .await?;
        if !ok {
            return Err(upstream_err(&err, &out, code));
        }
        if out.contains("No local changes to save") || err.contains("No local changes to save") {
            return Ok(None);
        }
        self.stash_list()
            .await?
            .into_iter()
            .find(|entry| entry.message.ends_with(&marker))
            .map(|entry| Some(entry.sha))
            .ok_or_else(|| Error::Conflict(
                "automatic stash changed before it could be identified — inspect git stash list before continuing".into(),
            ))
    }

    /// Restore precisely this auto-stash, including its index. Keep the backup:
    /// Git's positional `stash pop` can remove somebody else's concurrently
    /// pushed entry between our lookup and its own reflog lock acquisition.
    pub(crate) async fn pop_stash_sha(&self, sha: &str) -> Result<String> {
        self.resolve_stash_selector(sha).await?;
        let (ok, out, err, code) = self
            .run_raw_retry_lock(&["stash", "apply", "--index", sha])
            .await?;
        if ok || out.contains("CONFLICT") || err.contains("CONFLICT") {
            return Ok(format!("{out}\n{err}").trim().to_string());
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
        Self::guard_ref(source)?;
        Self::guard_ref(target)?;
        // Merging a branch into itself is a no-op git reports as "Already up
        // to date" — and the MergeCommit path would then record an empty
        // "Merge branch 'x' into x" commit. Refuse it up front.
        if source.trim() == target.trim() {
            return Err(Error::Invalid(format!(
                "cannot merge '{source}' into itself — pick a different target branch"
            )));
        }
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
            return self
                .merge_branch_inner(source, target, strategy, None)
                .await;
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
            // When source's tip IS target's tip (a branch just cut from target
            // with no commits) there is nothing to record: `-p X -p X` collapses
            // to one parent and would land an empty "Merge branch" commit, so
            // that case falls through to the plain `up_to_date` result.
            let heads = if up_to_date && matches!(strategy, LocalMergeStrategy::MergeCommit) {
                let target_head = self.run(&["rev-parse", "HEAD"]).await?.trim().to_string();
                let source_head = self.resolve_commit(source).await?;
                (source_head != target_head).then_some((target_head, source_head))
            } else {
                None
            };
            if let Some((target_head, source_head)) = heads {
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
        // The real git dir: in a linked worktree `.git` is a FILE, so
        // `<tree>/.git/MERGE_MSG` never existed and the label fell back to a sha.
        if let Some(git_dir) = self.git_dir().await {
            if let Ok(text) = tokio::fs::read_to_string(git_dir.join("MERGE_MSG")).await {
                if let Some(line) = text.lines().find(|l| !l.trim().is_empty()) {
                    return Some(line.trim().to_string());
                }
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
    ///
    /// Data-loss boundary: a recomposed text body never replaces a BINARY
    /// working file (its segments are empty, so the body would be `""` — one
    /// click truncated `logo.png` to 0 bytes), and an empty body never
    /// recreates an ABSENT working file (a modify/delete conflict would come
    /// back as an empty file instead of a deletion). Both are resolved by
    /// [`Self::resolve_take_side`], which preserves bytes / removes the path.
    pub async fn write_resolution(&self, path: &str, content: &str) -> Result<()> {
        self.conflict_sides(path).await?;
        let abs = self.confined_worktree_file(path, true).await?;
        match tokio::fs::read(&abs).await {
            Ok(bytes) if bytes.contains(&0) || std::str::from_utf8(&bytes).is_err() => {
                return Err(Error::Invalid(format!(
                    "{path} is a binary conflict — take a whole side or keep the working file"
                )));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && content.is_empty() => {
                return Err(Error::Invalid(format!(
                    "{path} is absent from the working tree — take a side or delete it"
                )));
            }
            _ => {}
        }
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
    /// / cherry-pick / revert), or discard a staged squash (`reset --merge`,
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
                    // `--merge`, not `--hard`: it drops the squash's staged
                    // result (and any conflict markers) but KEEPS unrelated
                    // uncommitted edits, which `--hard` silently wiped.
                    self.run(&["reset", "--merge"]).await?;
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
pub use crate::remote_url::{strip_url_userinfo, validate_remote_url};

fn is_git_dir_name(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .is_some_and(|s| s.eq_ignore_ascii_case(".git"))
}

/// stderr lines that SSH/git emit as benign chatter — never the reason a command
/// failed. We skip these when choosing the message to surface so the real git
/// error (rejected push, auth failure, …) isn't masked. Newer OpenSSH (9.x/10.x)
/// prints the post-quantum warning to stderr on every non-PQ connection and it
/// does NOT affect the exit status — yet it sorts first, so the old "first
/// non-empty line" logic reported it as the failure.
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
    const MARKERS: [&str; 29] = [
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
        // Push/pull on a detached HEAD, or pushing an unborn branch (no
        // commit yet) — nothing was sent anywhere.
        "you are not currently on a branch",
        "src refspec",
        // Unborn branch (no first commit) — stash/log have nothing to act on.
        "you do not have the initial commit yet",
        "does not have any commits yet",
        // A conflicted path handed to restore/unstage — resolve it first.
        "is unmerged",
    ];
    let lc = msg.to_ascii_lowercase();
    MARKERS.iter().any(|m| lc.contains(m))
}

/// Stable prefix of the 409 a non-fast-forward push rejection maps to. The UI
/// matches on it to offer "Pull" / "Force push with lease…" instead of a dead
/// end — keep it in sync with `ui/src/modules/git/push-errors.ts`.
pub const PUSH_REJECTED: &str = "push rejected: the remote branch has commits yours doesn't";
/// Stable prefix of the 409 a refused force-with-lease maps to.
pub const PUSH_LEASE_REFUSED: &str = "force push refused: the remote branch moved";

/// Map a failed `git push` to an error the caller can act on. A rejection
/// because the remote has commits we don't (`fetch first` / `non-fast-forward`)
/// or a refused lease (`stale info` / `remote ref updated since checkout`) is
/// a LOCAL-state 409 — nothing is wrong with the provider, and calling it a 502
/// told the user the remote was down when the fix is "pull first". Everything
/// else (auth, network, hook/protected-branch refusals) goes through
/// [`upstream_err`].
pub(crate) fn push_err(stderr: &str, stdout: &str, code: Option<i32>) -> Error {
    let rejected = stderr
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("! [rejected]"));
    if let Some(line) = rejected {
        if line.contains("(stale info)") || line.contains("(remote ref updated since checkout)") {
            return Error::Conflict(format!(
                "{PUSH_LEASE_REFUSED} since you last fetched and integrated it — fetch, review \
                 the new commits, then try again ({line})"
            ));
        }
        if line.contains("(fetch first)") || line.contains("(non-fast-forward)") {
            return Error::Conflict(format!(
                "{PUSH_REJECTED} — pull to bring them in, then push again ({line})"
            ));
        }
    }
    upstream_err(stderr, stdout, code)
}

/// A one-line next step for a remote op that failed on CREDENTIALS, appended
/// to the error so "fatal: Authentication failed for …" says where to fix it.
/// `None` for anything that isn't recognisably an auth failure.
fn auth_hint(full_lc: &str) -> Option<&'static str> {
    const HTTPS: [&str; 5] = [
        "authentication failed",
        "could not read username",
        "could not read password",
        "the requested url returned error: 401",
        "the requested url returned error: 403",
    ];
    if full_lc.contains("permission denied (publickey")
        || full_lc.contains("host key verification failed")
    {
        return Some(
            "SSH authentication failed — check that your SSH agent has a key this remote accepts \
             (`ssh-add -l`) and that the host is in known_hosts",
        );
    }
    if HTTPS.iter().any(|m| full_lc.contains(m)) {
        return Some(
            "authentication failed — check the repository's git account token (Settings → Git \
             Accounts: Test, expiry and write scope)",
        );
    }
    None
}

/// The private ref [`LocalGit::fetch_pr_head`] fetches a PR head into:
/// `refs/otto/pr-review/<n>-<first 12 of head_sha>`, or `<n>` when the head
/// is unknown. A sibling name (not `<n>/<sha>`) so it never D/F-conflicts with
/// a `refs/otto/pr-review/<n>` an older build left behind.
pub(crate) fn pr_review_ref(pr_number: u64, head_sha: Option<&str>) -> String {
    match head_sha {
        Some(sha) => {
            let short: String = sha
                .chars()
                .filter(char::is_ascii_hexdigit)
                .take(12)
                .collect::<String>()
                .to_ascii_lowercase();
            if short.is_empty() {
                format!("refs/otto/pr-review/{pr_number}")
            } else {
                format!("refs/otto/pr-review/{pr_number}-{short}")
            }
        }
        None => format!("refs/otto/pr-review/{pr_number}"),
    }
}

/// A `git fetch` failure that only means "the host doesn't publish this
/// ref" (try the next one) — as opposed to auth / network / repo errors.
pub(crate) fn is_missing_remote_ref(stderr: &str) -> bool {
    let lc = stderr.to_ascii_lowercase();
    lc.contains("couldn't find remote ref") || lc.contains("could not find remote ref")
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
    let hint = auth_hint(&full.to_ascii_lowercase())
        .map(|h| format!(" — {h}"))
        .unwrap_or_default();
    Error::Upstream(format!(
        "git exited {}: {}{hint}",
        code.map_or_else(|| "?".to_string(), |c| c.to_string()),
        pick
    ))
}

// ---------------------------------------------------------------------------
// Askpass helper for https remotes
// ---------------------------------------------------------------------------

/// Temp executable script handed to git via GIT_ASKPASS. Echoes a placeholder
/// username or token only for an HTTP(S) prompt matching the credential's
/// approved authority. The token lives in OTTO_GIT_TOKEN, never on disk.
/// Generic/SSH prompts and rewritten unrelated destinations are refused. Works for
/// GitHub (any username + PAT), Bitbucket (see [`AskPass::envs`] — API tokens
/// need the magic `x-bitbucket-api-token-auth` username; access tokens use
/// `x-token-auth`) and GitLab (any username + PAT).
pub(crate) struct AskPass {
    // Held to keep the temp file alive for the duration of the command.
    _file: tempfile::TempPath,
    path: PathBuf,
    credential: TransportCredential,
}

impl AskPass {
    pub(crate) fn new(credential: &TransportCredential) -> Result<Self> {
        use std::io::Write;
        let mut f = tempfile::Builder::new()
            .prefix("otto-askpass-")
            .suffix(".sh")
            .tempfile()
            .map_err(|e| Error::Internal(format!("askpass tmp: {e}")))?;
        f.write_all(include_bytes!("askpass.sh"))
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
            credential: credential.clone(),
        })
    }

    pub(crate) fn envs(&self) -> Vec<(String, String)> {
        let mut envs = vec![
            (
                "GIT_ASKPASS".to_string(),
                self.path.to_string_lossy().into_owned(),
            ),
            ("OTTO_GIT_TOKEN".to_string(), self.credential.token.clone()),
            (
                "OTTO_GIT_ORIGINS".to_string(),
                self.credential.origins.join("\n"),
            ),
            ("OTTO_GIT_USERNAME".to_string(), "x-token-auth".to_string()),
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
        if self.credential.token.starts_with("ATATT") {
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
    token: Option<&TransportCredential>,
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
        .stderr(Stdio::piped())
        // Own process group (git + its `git-remote-https`/`ssh` helpers) so a
        // stall can be killed as a whole; never outlive the caller either.
        .process_group(0)
        .kill_on_drop(true);
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
    // A clone may legitimately take long, but not SILENTLY: `--progress`
    // reports every second or so. No output for this long (a dead VPN, a
    // server that stopped sending) ends the clone instead of hanging the
    // clone task forever — the one git spawn here that had no bound.
    let idle = std::time::Duration::from_secs(env_secs("OTTO_GIT_CLONE_IDLE_SECS", 300));
    loop {
        let n = match tokio::time::timeout(idle, stderr.read(&mut chunk)).await {
            Ok(read) => read.map_err(|e| Error::Internal(format!("clone read: {e}")))?,
            Err(_) => {
                if let Some(pid) = child.id() {
                    kill_group(pid as libc::pid_t).await;
                }
                let _ = child.wait().await;
                return Err(Error::Upstream(format!(
                    "git clone stalled: no progress for {}s (last: {last_line})",
                    idle.as_secs()
                )));
            }
        };
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

/// Cap on [`LocalGit::head_remote_refs`] — the UI only names a few.
pub const HEAD_REMOTES_CAP: usize = 20;

#[cfg(test)]
#[path = "local/tests.rs"]
mod tests;

/// Merged marks, the up-to-date merge guard and the launcher-file sweep.
/// Kept beside the code (not in `local/tests.rs`) so each fix carries the
/// test that fails without it.
#[cfg(test)]
mod merge_and_marks_tests {
    use super::*;

    fn sh(dir: &Path, args: &[&str]) -> String {
        let mut cmd = std::process::Command::new("git");
        test_hermetic(&mut cmd);
        let out = cmd
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
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn repo(branch: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("repo");
        std::fs::create_dir(&dir).unwrap();
        sh(&dir, &["init", "-q", "-b", branch]);
        sh(&dir, &["config", "user.email", "otto@test.local"]);
        sh(&dir, &["config", "user.name", "Otto Test"]);
        sh(&dir, &["config", "commit.gpgsign", "false"]);
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        sh(&dir, &["add", "."]);
        sh(&dir, &["commit", "-qm", "init"]);
        (tmp, dir)
    }

    fn commit(dir: &Path, file: &str) {
        std::fs::write(dir.join(file), file).unwrap();
        sh(dir, &["add", file]);
        sh(dir, &["commit", "-qm", file]);
    }

    /// A branch cut from `main` with no commits, merged back with
    /// MergeCommit, is up to date — no empty "Merge branch" commit lands.
    #[tokio::test]
    async fn merge_commit_of_an_identical_tip_reports_up_to_date_without_a_commit() {
        let (_tmp, dir) = repo("main");
        sh(&dir, &["branch", "feature"]);
        let before = sh(&dir, &["rev-parse", "HEAD"]);
        let git = LocalGit::new(&dir);
        let res = git
            .merge_branch("feature", "main", LocalMergeStrategy::MergeCommit, false)
            .await
            .unwrap();
        assert_eq!(res.status, "up_to_date");
        assert!(res.commit.is_none());
        assert_eq!(
            sh(&dir, &["rev-parse", "HEAD"]),
            before,
            "main did not move"
        );
        // Merging a branch into itself is refused outright.
        let err = git
            .merge_branch("main", "main", LocalMergeStrategy::MergeCommit, false)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{err}");
        assert_eq!(sh(&dir, &["rev-parse", "HEAD"]), before);
    }

    /// No local `main`, only `origin/main` (a `-b develop` clone): the base
    /// is walked as `origin/main`, so a branch it holds is marked "main".
    #[tokio::test]
    async fn merged_into_reports_a_remote_only_default_branch() {
        let (_tmp, dir) = repo("develop");
        sh(&dir, &["branch", "old"]); // at the shared tip
        sh(&dir, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        commit(&dir, "dev.txt"); // develop moves ahead of origin/main
        let git = LocalGit::new(&dir);
        let local = sh(
            &dir,
            &[
                "for-each-ref",
                "--format=%(refname:lstrip=2)\t%(upstream:short)\t%(HEAD)\t%(objectname)\t%(upstream:track,nobracket)",
                "refs/heads",
            ],
        );
        let remote = sh(
            &dir,
            &[
                "for-each-ref",
                "--format=%(refname)\t%(objectname)",
                "refs/remotes",
            ],
        );
        let bases = integration_bases(Some("main"), &local, &remote);
        assert_eq!(
            bases,
            [
                ("main".to_string(), "origin/main".to_string()),
                ("develop".to_string(), "develop".to_string()),
            ]
        );
        // End to end through `origin/HEAD` → main (the detected default).
        sh(
            &dir,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );
        let refs = git.refs_with_base(None).await.unwrap();
        assert_eq!(refs.base_branch.as_deref(), Some("main"));
        let old = refs.local.iter().find(|b| b.name == "old").unwrap();
        let mut landed = old.merged_into.clone();
        landed.sort();
        assert_eq!(landed, ["develop", "main"]);
        assert!(old.merged_into_base, "origin/main holds old");
        let develop = refs.local.iter().find(|b| b.name == "develop").unwrap();
        assert!(develop.merged_into.is_empty(), "integration branch");
    }

    /// An `origin/develop` base override does not also add a `develop` label.
    #[test]
    fn integration_labels_dedupe_after_stripping_origin() {
        let local = "develop\t\t*\tabc\t\nfeature\t\t\tdef\t\n";
        let remote = "refs/remotes/origin/develop\tabc\nrefs/remotes/origin/main\t123\n";
        let bases = integration_bases(Some("origin/develop"), local, remote);
        assert_eq!(
            bases,
            [
                ("origin/develop".to_string(), "origin/develop".to_string()),
                ("main".to_string(), "origin/main".to_string()),
            ]
        );
        assert_eq!(listed_oid("develop", local, remote), Some("abc"));
        assert_eq!(listed_oid("origin/main", local, remote), Some("123"));
        assert_eq!(listed_oid("v1.0", local, remote), None);
    }

    /// The merged sets come from ONE `for-each-ref --merged` covering both
    /// namespaces, and skip the symbolic `origin/HEAD`.
    #[tokio::test]
    async fn merged_sets_split_local_and_remote_from_one_listing() {
        let (_tmp, dir) = repo("main");
        sh(&dir, &["branch", "done"]);
        sh(&dir, &["update-ref", "refs/remotes/origin/done", "HEAD"]);
        sh(&dir, &["update-ref", "refs/remotes/origin/main", "HEAD"]);
        sh(
            &dir,
            &[
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ],
        );
        sh(&dir, &["checkout", "-qb", "ahead"]);
        commit(&dir, "x.txt");
        sh(&dir, &["checkout", "-q", "main"]);
        let git = LocalGit::new(&dir);
        let ((local, remote), complete) = git.merged_sets(Some("main")).await;
        assert!(complete);
        let mut l: Vec<_> = local.into_iter().collect();
        l.sort();
        let mut r: Vec<_> = remote.into_iter().collect();
        r.sort();
        assert_eq!(l, ["done", "main"]);
        assert_eq!(r, ["origin/done", "origin/main"]);
        // A base no listing or rev-parse resolves yields empty sets.
        let (none_l, none_r) = git.merged_sets_cached(Some("no-such-branch"), "", "").await;
        assert!(none_l.is_empty() && none_r.is_empty());
    }

    /// agy's `.agents/mcp_config.json` and grok's `.grok/config.toml` carry
    /// resolved secret env — the workflow sweep must never commit them.
    #[tokio::test]
    async fn commit_all_if_dirty_skips_every_launcher_artifact() {
        let (_tmp, dir) = repo("main");
        std::fs::create_dir_all(dir.join(".agents")).unwrap();
        std::fs::create_dir_all(dir.join(".grok")).unwrap();
        std::fs::write(dir.join(".agents/mcp_config.json"), "{\"secret\":1}").unwrap();
        std::fs::write(dir.join(".grok/config.toml"), "secret = 1").unwrap();
        std::fs::write(dir.join(".mcp.json"), "{}").unwrap();
        let git = LocalGit::new(&dir);
        assert!(git.commit_all_if_dirty("noop").await.unwrap().is_none());
        std::fs::write(dir.join("work.txt"), "w").unwrap();
        assert!(git.commit_all_if_dirty("sweep").await.unwrap().is_some());
        let committed = sh(&dir, &["show", "--name-only", "--format=", "HEAD"]);
        assert_eq!(committed, "work.txt");
    }
}
