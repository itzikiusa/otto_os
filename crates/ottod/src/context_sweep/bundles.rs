//! Orphaned per-folder skill bundles: `~/.otto/context/<provider>/<enc(cwd)>`.
//!
//! Every agent launch materializes a bundle (context file + a full copy of
//! every active skill, ~2 MB / ~325 files) for its working directory, and
//! nothing ever removed one. Workflow runs launch in throwaway worktrees
//! (`<data>/workflow-runs/<run_id>/<repo>`), so one install held 1.4 GB of
//! bundles, most for worktrees reaped long ago.
//!
//! What a pass deletes — nothing else, ever:
//! - a bundle with a cwd marker (`.otto-cwd`, written by
//!   `otto_context::materialize` on every provision and resume) whose cwd is
//!   an absolute path that no longer exists (`lstat` says not-found; any
//!   other error keeps it) AND that nobody touched (newest mtime of the
//!   folder and its direct entries) for the grace window;
//! - a bundle WITHOUT a marker (made before markers existed) only when it is
//!   untouched for [`UNMARKED_MIN_DAYS`] (or the grace, if longer), its name
//!   is a workflow worktree's (`enc(<data>/workflow-runs)-<26-char run id>…`)
//!   and that run's `<data>/workflow-runs/<run_id>` folder is gone.
//!
//! Only real directories directly under the five provider folders (`claude`,
//! `codex`, `agy`, `grok`, `opencode`) are considered — never a symlink, never
//! `review-skills` / `vault-docs-skills`. `remove_dir_all` removes the codex
//! shadow home's links to `~/.codex` without following them. At most
//! [`MAX_REMOVALS_PER_PASS`] per daily pass, logged with counts. Governed by
//! `context_bundle_orphan_days` (grace in days, default [`DEFAULT_DAYS`],
//! `0` = off) and the `data_retention` master switch.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Settings key: grace in days (`0` turns the bundle sweep off).
pub const SETTING_KEY: &str = "context_bundle_orphan_days";
/// Default grace: a week unused after its folder disappeared.
pub const DEFAULT_DAYS: u64 = 7;
/// A bundle without a marker is never removed younger than this.
pub const UNMARKED_MIN_DAYS: u64 = 30;
/// Bundles removed per pass, at most.
pub const MAX_REMOVALS_PER_PASS: usize = 100;
/// Bundle folders looked at per pass, at most.
const MAX_SCANNED_PER_PASS: usize = 5_000;
/// Must equal `otto_context::materialize::BUNDLE_CWD_MARKER` (ottod does not
/// link otto-context; a test below checks the two stay in step).
pub const MARKER: &str = ".otto-cwd";
/// The provider folders bundles live in (`delivers_context` providers).
const PROVIDERS: [&str; 5] = ["claude", "codex", "agy", "grok", "opencode"];
const DAY: u64 = 24 * 3600;

/// The grace from the setting value: absent / malformed → the default,
/// `0` → off (`None`), anything else at least one day.
pub fn grace_days(v: Option<&serde_json::Value>) -> Option<u64> {
    match v.map(serde_json::Value::as_u64) {
        None | Some(None) => Some(DEFAULT_DAYS),
        Some(Some(0)) => None,
        Some(Some(d)) => Some(d),
    }
}

/// Why a bundle goes (for the log line).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orphan {
    /// Its marker's cwd is gone.
    Marked,
    /// No marker; a reaped workflow run's worktree.
    UnmarkedWorkflow,
}

/// Newest mtime of `dir` and its direct entries (never following a link):
/// a rewritten marker or context file counts as a use.
// Sync by design: runs on the blocking pool.
#[allow(clippy::disallowed_methods)]
fn last_touched(dir: &Path, own: SystemTime) -> SystemTime {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return own;
    };
    rd.flatten()
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .fold(own, SystemTime::max)
}

/// Only a positive "not found" counts as gone; a relative path, a permission
/// error or anything else keeps the bundle.
#[allow(clippy::disallowed_methods)]
fn path_gone(path: &Path) -> bool {
    path.is_absolute()
        && matches!(std::fs::symlink_metadata(path), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
}

/// The workflow run id in an unmarked bundle's folder name, when the name is
/// `<enc(runs_root)>-<26 alphanumerics>` optionally followed by `-<repo>`.
fn workflow_run_id<'a>(name: &'a str, runs_root: &Path) -> Option<&'a str> {
    let prefix = otto_transcript::roots::encode_cwd(&runs_root.to_string_lossy());
    let rest = name.strip_prefix(&prefix)?.strip_prefix('-')?;
    let id = rest.split('-').next()?;
    (id.len() == 26 && id.bytes().all(|b| b.is_ascii_alphanumeric())).then_some(id)
}

/// Whether the bundle folder `dir` (named `name`) is an orphan at `now`.
#[allow(clippy::disallowed_methods)]
pub fn judge(
    dir: &Path,
    name: &str,
    now: SystemTime,
    grace: Duration,
    runs_root: &Path,
) -> Option<Orphan> {
    let meta = std::fs::symlink_metadata(dir).ok()?;
    if !meta.is_dir() {
        return None;
    }
    let touched = last_touched(dir, meta.modified().ok()?);
    let idle = now.duration_since(touched).unwrap_or_default();
    match std::fs::read_to_string(dir.join(MARKER)) {
        Ok(cwd) => (idle >= grace && path_gone(Path::new(&cwd))).then_some(Orphan::Marked),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let min = grace.max(Duration::from_secs(UNMARKED_MIN_DAYS * DAY));
            let id = workflow_run_id(name, runs_root)?;
            (idle >= min && path_gone(&runs_root.join(id))).then_some(Orphan::UnmarkedWorkflow)
        }
        Err(_) => None,
    }
}

/// The orphans under `ctx_root`, at most `limit`, plus how many bundle
/// folders were looked at.
#[allow(clippy::disallowed_methods)]
pub fn orphans(
    ctx_root: &Path,
    runs_root: &Path,
    now: SystemTime,
    grace: Duration,
    limit: usize,
) -> (usize, Vec<(PathBuf, Orphan)>) {
    let mut scanned = 0;
    let mut found = Vec::new();
    for provider in PROVIDERS {
        let Ok(rd) = std::fs::read_dir(ctx_root.join(provider)) else {
            continue;
        };
        for entry in rd.flatten() {
            if found.len() >= limit || scanned >= MAX_SCANNED_PER_PASS {
                return (scanned, found);
            }
            scanned += 1;
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let path = entry.path();
            if let Some(why) = judge(&path, &name, now, grace, runs_root) {
                found.push((path, why));
            }
        }
    }
    (scanned, found)
}

/// What one pass did (for the log line).
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub scanned: usize,
    pub marked: usize,
    pub unmarked: usize,
    pub failed: usize,
}

/// One synchronous pass: find and remove up to [`MAX_REMOVALS_PER_PASS`].
#[allow(clippy::disallowed_methods)]
pub fn sweep(ctx_root: &Path, runs_root: &Path, now: SystemTime, grace: Duration) -> Report {
    let (scanned, found) = orphans(ctx_root, runs_root, now, grace, MAX_REMOVALS_PER_PASS);
    let mut report = Report {
        scanned,
        ..Report::default()
    };
    for (path, why) in found {
        // `remove_dir_all` unlinks symlinks inside (codex shadow home → the
        // real `~/.codex`) without following them.
        match std::fs::remove_dir_all(&path) {
            Ok(()) if why == Orphan::Marked => report.marked += 1,
            Ok(()) => report.unmarked += 1,
            Err(e) => {
                tracing::debug!(path = %path.display(), "context bundle sweep: {e}");
                report.failed += 1;
            }
        }
    }
    report
}

/// The daily pass (settings already read: `days` is the grace).
pub async fn pass(ctx_root: PathBuf, runs_root: PathBuf, days: u64) {
    let grace = Duration::from_secs(days * DAY);
    let report =
        tokio::task::spawn_blocking(move || sweep(&ctx_root, &runs_root, SystemTime::now(), grace))
            .await
            .unwrap_or_default();
    if report.marked + report.unmarked + report.failed > 0 {
        tracing::info!(
            "retention: removed {} orphaned context bundle(s) whose folder is gone and {} \
             unmarked workflow-worktree bundle(s) ({} scanned, {} could not be removed; \
             grace {days} d)",
            report.marked,
            report.unmarked,
            report.scanned,
            report.failed
        );
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // sync test scaffolding
mod tests {
    use super::*;

    const ULID: &str = "01KY4R7DY3DVQQEQ1AQAABX3Z4";

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ottod-bundle-sweep-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn bundle(root: &Path, provider: &str, name: &str, marker: Option<&str>) -> PathBuf {
        let dir = root.join(provider).join(name);
        std::fs::create_dir_all(dir.join(".claude/skills/triage")).unwrap();
        std::fs::write(dir.join("CONTEXT.md"), "ctx").unwrap();
        if let Some(cwd) = marker {
            std::fs::write(dir.join(MARKER), cwd).unwrap();
        }
        dir
    }

    fn days(n: u64) -> Duration {
        Duration::from_secs(n * DAY)
    }

    #[test]
    fn the_marker_name_matches_otto_context() {
        let src = include_str!("../../../otto-context/src/materialize.rs");
        assert!(src.contains(&format!(
            "pub const BUNDLE_CWD_MARKER: &str = \"{MARKER}\";"
        )));
    }

    #[test]
    fn the_setting_defaults_and_turns_off() {
        assert_eq!(grace_days(None), Some(DEFAULT_DAYS));
        assert_eq!(
            grace_days(Some(&serde_json::json!("x"))),
            Some(DEFAULT_DAYS)
        );
        assert_eq!(grace_days(Some(&serde_json::json!(0))), None, "0 = off");
        assert_eq!(grace_days(Some(&serde_json::json!(14))), Some(14));
    }

    #[test]
    fn a_marked_bundle_goes_only_when_its_cwd_is_gone_and_the_grace_passed() {
        let tmp = scratch("marked");
        let (ctx, runs) = (tmp.join("context"), tmp.join("data/workflow-runs"));
        let live_cwd = tmp.join("repo");
        std::fs::create_dir_all(&live_cwd).unwrap();
        let gone_cwd = tmp.join("deleted-worktree");
        let keep = bundle(&ctx, "claude", "a", Some(&live_cwd.to_string_lossy()));
        let gone = bundle(&ctx, "codex", "b", Some(&gone_cwd.to_string_lossy()));
        let relative = bundle(&ctx, "claude", "c", Some("not/absolute"));
        let later = SystemTime::now() + days(8);
        // Inside the grace nothing goes, even with the cwd gone.
        assert_eq!(judge(&gone, "b", SystemTime::now(), days(7), &runs), None);
        assert_eq!(judge(&keep, "a", later, days(7), &runs), None, "cwd exists");
        assert_eq!(judge(&relative, "c", later, days(7), &runs), None);
        assert_eq!(
            judge(&gone, "b", later, days(7), &runs),
            Some(Orphan::Marked)
        );
        let report = sweep(&ctx, &runs, later, days(7));
        assert_eq!((report.marked, report.unmarked, report.failed), (1, 0, 0));
        assert_eq!(report.scanned, 3);
        assert!(!gone.exists() && keep.is_dir() && relative.is_dir());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn an_unmarked_bundle_goes_only_for_a_reaped_workflow_run_after_30_days() {
        let tmp = scratch("unmarked");
        let ctx = tmp.join("context");
        let runs = tmp.join("data/workflow-runs");
        let prefix = otto_transcript::roots::encode_cwd(&runs.to_string_lossy());
        let reaped = bundle(&ctx, "claude", &format!("{prefix}-{ULID}-sinatra-go"), None);
        let live_id = "01KY4RYYN9CMWMJAPMZFDHB92S";
        std::fs::create_dir_all(runs.join(live_id)).unwrap();
        let running = bundle(
            &ctx,
            "claude",
            &format!("{prefix}-{live_id}-sinatra-go"),
            None,
        );
        let user = bundle(&ctx, "claude", "-Users-x-my-repo", None);
        let short_id = bundle(&ctx, "claude", &format!("{prefix}-01KY-x"), None);
        // 20 days: past the grace, not past the unmarked floor.
        let soon = SystemTime::now() + days(20);
        assert_eq!(
            sweep(&ctx, &runs, soon, days(7)),
            Report {
                scanned: 4,
                ..Report::default()
            }
        );
        let later = SystemTime::now() + days(31);
        let report = sweep(&ctx, &runs, later, days(7));
        assert_eq!((report.marked, report.unmarked), (0, 1));
        assert!(!reaped.exists());
        assert!(running.is_dir(), "its run folder still exists");
        assert!(
            user.is_dir(),
            "not a workflow worktree: never without a marker"
        );
        assert!(short_id.is_dir());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn only_real_bundle_folders_under_provider_dirs_are_considered() {
        let tmp = scratch("shape");
        let ctx = tmp.join("context");
        let runs = tmp.join("runs");
        let gone = tmp.join("gone").to_string_lossy().into_owned();
        // Other layouts under the context root are never looked at.
        let review = bundle(&ctx, "review-skills", "x", Some(&gone));
        // A symlinked bundle is never followed or removed.
        let target = bundle(&tmp, "elsewhere", "t", Some(&gone));
        std::fs::create_dir_all(ctx.join("claude")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, ctx.join("claude/link")).unwrap();
        std::fs::write(ctx.join("claude/stray-file"), "x").unwrap();
        let later = SystemTime::now() + days(60);
        let report = sweep(&ctx, &runs, later, days(7));
        assert_eq!((report.marked, report.unmarked, report.failed), (0, 0, 0));
        assert!(review.is_dir() && target.is_dir());
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn one_pass_removes_at_most_the_cap() {
        let tmp = scratch("cap");
        let ctx = tmp.join("context");
        let gone = tmp.join("gone").to_string_lossy().into_owned();
        for i in 0..MAX_REMOVALS_PER_PASS + 5 {
            bundle(&ctx, "claude", &format!("b{i}"), Some(&gone));
        }
        let later = SystemTime::now() + days(8);
        let first = sweep(&ctx, &tmp.join("runs"), later, days(7));
        assert_eq!(first.marked, MAX_REMOVALS_PER_PASS);
        let second = sweep(&ctx, &tmp.join("runs"), later, days(7));
        assert_eq!(second.marked, 5);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
