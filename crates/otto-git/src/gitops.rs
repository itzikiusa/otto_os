//! Worktree inspection + safe removal, stash browsing, and the bulk branch
//! delete — the "what's in there before I throw it away" side of the Git page.
//!
//! Worktree removal used to be `git worktree remove` under the 30 s local
//! spawn budget. git deletes the WHOLE directory, ignored files included, so a
//! worktree holding a Rust `target/` (tens of GB) or a `node_modules` was
//! killed mid-delete: the request failed, the tree was left half-deleted and
//! every retry failed too. Removal now checks safety itself (uncommitted and
//! untracked files, commits that exist nowhere else, lock, submodules), then
//! renames the directory aside — instant, same parent so same volume — drops
//! the worktree's admin dir, and deletes the renamed tree in the background.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use otto_core::api::{StashDetail, StashFile, StashInfo, WorktreeInfo};
use otto_core::{Error, Result};

use crate::local::{worktree_round_trips, LocalGit, SpawnClass};

/// Cap on a stash patch returned to the UI.
const STASH_PATCH_CAP: usize = 400 * 1024;
/// Stashes whose untracked parent is listed for the file count.
const STASH_COUNT_CAP: usize = 50;
/// Budget for the single-worktree status check before a removal. Generous:
/// it runs once, on a user action, and an unknown status forces a force.
const INSPECT_BUDGET: std::time::Duration = std::time::Duration::from_secs(20);

/// `(tracked changes, untracked files)` from `git status --porcelain` (v1).
pub(crate) fn porcelain_counts(stdout: &[u8]) -> (u32, u32) {
    let text = String::from_utf8_lossy(stdout);
    let (mut changes, mut untracked) = (0u32, 0u32);
    for line in text.lines().filter(|l| l.len() >= 3) {
        if line.starts_with("??") {
            untracked += 1;
        } else if !line.starts_with("!!") {
            changes += 1;
        }
    }
    (changes, untracked)
}

fn plural(n: u32, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Plain-language reasons removing `w` needs explicit force. Empty = a plain
/// remove is safe. The main worktree and stale entries carry none (the first
/// is never removable, the second has nothing left to lose).
pub fn removal_risks(w: &WorktreeInfo) -> Vec<String> {
    let mut out = Vec::new();
    if w.is_main || w.prunable {
        return out;
    }
    if !w.dirty_known {
        out.push(
            "its status is unknown (git status failed), so Otto can't tell what it holds".into(),
        );
    }
    if w.changes > 0 {
        out.push(plural(
            w.changes,
            "uncommitted change",
            "uncommitted changes",
        ));
    }
    if w.untracked > 0 {
        out.push(plural(w.untracked, "untracked file", "untracked files"));
    }
    if let Some(n) = w.unpushed.filter(|n| *n > 0) {
        let commits = plural(n, "commit", "commits");
        out.push(match (&w.branch, &w.base) {
            (None, _) => format!(
                "{commits} on a detached HEAD that no branch or remote holds — removal orphans them"
            ),
            (Some(_), Some(base)) => {
                format!("{commits} not pushed to any remote and not in {base}")
            }
            (Some(_), None) => format!("{commits} not pushed to any remote"),
        });
    }
    if w.locked {
        out.push(match &w.lock_reason {
            Some(r) => format!("it is locked ({r})"),
            None => "it is locked".into(),
        });
    }
    out
}

/// The worktree's admin dir (`<common>/worktrees/<id>`), read from its `.git`
/// file — `None` unless the pointer round-trips to THIS repo (an agent can
/// rewrite either side; never act on a dir it aimed elsewhere).
fn owned_admin_dir(path: &Path, common: &Path) -> Option<PathBuf> {
    if !worktree_round_trips(path, common) {
        return None;
    }
    let text = std::fs::read_to_string(path.join(".git")).ok()?;
    let target = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
    let admin = if target.is_absolute() {
        target
    } else {
        path.join(target)
    };
    std::fs::canonicalize(admin).ok()
}

/// Message of an otto error without its `invalid:` / `conflict:` prefix.
pub(crate) fn err_text(e: &Error) -> String {
    match e {
        Error::NotFound(m)
        | Error::Forbidden(m)
        | Error::Conflict(m)
        | Error::Invalid(m)
        | Error::PayloadTooLarge(m)
        | Error::UnsupportedMedia(m)
        | Error::Upstream(m)
        | Error::Internal(m) => m.clone(),
        Error::Unauthorized => "unauthorized".into(),
    }
}

/// What a removal did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeRemoval {
    /// The directory was gone; its stale registration was pruned.
    Pruned,
    /// Renamed aside and unregistered; `trash` is being deleted in the background.
    Detached { trash: PathBuf },
    /// Removed by `git worktree remove` (the rename was not possible).
    Git,
}

/// Where a new worktree for `branch` goes: a sibling of the repo folder,
/// `<repo>-<branch>` with every character outside `[A-Za-z0-9._-]` folded to
/// `-` (so `feat/x` → `<repo>-feat-x`). The daemon picks it, never the caller.
pub fn sibling_worktree_path(repo: &Path, branch: &str) -> PathBuf {
    let slug: String = branch
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-');
    let name = repo
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "repo".into());
    repo.with_file_name(format!("{name}-{slug}"))
}

impl LocalGit {
    async fn common_dir(&self) -> Option<PathBuf> {
        self.run_read(&["rev-parse", "--path-format=absolute", "--git-common-dir"])
            .await
            .ok()
            .map(|s| PathBuf::from(s.trim()))
    }

    /// Create a NEW branch `branch` from `base` (default `HEAD`) checked out in a
    /// new worktree at [`sibling_worktree_path`]. Never touches an existing
    /// branch or folder: either one is a 409, and `-b` refuses a branch created
    /// in between. Returns the new worktree's row.
    pub async fn worktree_create(&self, branch: &str, base: Option<&str>) -> Result<WorktreeInfo> {
        let branch = branch.trim();
        if branch.is_empty() {
            return Err(Error::Invalid("branch name must not be empty".into()));
        }
        Self::guard_ref(branch)?;
        let (valid, _, _, _) = self
            .run_raw(&["check-ref-format", "--branch", branch], &[])
            .await?;
        if !valid {
            return Err(Error::Invalid(format!(
                "“{branch}” is not a valid branch name"
            )));
        }
        let base = base
            .map(str::trim)
            .filter(|b| !b.is_empty())
            .unwrap_or("HEAD");
        Self::guard_ref(base)?;
        if self.branch_exists(branch).await {
            return Err(Error::Conflict(format!("branch {branch} already exists")));
        }
        let path = sibling_worktree_path(self.path(), branch);
        if tokio::fs::try_exists(&path).await.unwrap_or(true) {
            return Err(Error::Conflict(format!(
                "{} already exists",
                path.display()
            )));
        }
        let path_str = path.to_string_lossy().into_owned();
        self.run(&["worktree", "add", "-b", branch, &path_str, base])
            .await?;
        let want = std::fs::canonicalize(&path).unwrap_or(path);
        self.worktree_list_detailed()
            .await?
            .into_iter()
            .find(|w| std::fs::canonicalize(&w.path).ok().as_deref() == Some(want.as_path()))
            .ok_or_else(|| {
                Error::Internal("the new worktree is missing from git worktree list".into())
            })
    }

    /// `git worktree list` + status counts + branch/commit details + risks.
    pub async fn worktree_list_detailed(&self) -> Result<Vec<WorktreeInfo>> {
        let mut rows = self.worktree_list().await?;
        self.enrich_worktrees(&mut rows).await;
        Ok(rows)
    }

    /// Fill upstream / ahead / behind / on_remote / base / unmerged / unpushed
    /// and `risks` for every row. Best-effort: a failed count stays `None`
    /// (unknown), which never reads as "safe".
    pub(crate) async fn enrich_worktrees(&self, rows: &mut [WorktreeInfo]) {
        let heads = self
            .run_read(&[
                "for-each-ref",
                "--format=%(refname)%00%(upstream:short)%00%(upstream:track,nobracket)",
                "refs/heads",
            ])
            .await
            .unwrap_or_default();
        let mut tracking: HashMap<String, (String, String)> = HashMap::new();
        for line in heads.lines() {
            let mut it = line.split('\0');
            let (Some(r), Some(up), Some(track)) = (it.next(), it.next(), it.next()) else {
                continue;
            };
            if let Some(name) = r.strip_prefix("refs/heads/") {
                tracking.insert(name.to_string(), (up.to_string(), track.to_string()));
            }
        }
        let remote_refs: Vec<String> = self
            .run_read(&["for-each-ref", "--format=%(refname)", "refs/remotes"])
            .await
            .unwrap_or_default()
            .lines()
            .filter_map(|l| l.strip_prefix("refs/remotes/"))
            .filter(|l| !l.ends_with("/HEAD"))
            .map(str::to_string)
            .collect();
        let base_name = self.default_branch().await;
        let base_ref = base_name.as_ref().and_then(|b| {
            let remote = format!("origin/{b}");
            if remote_refs.contains(&remote) {
                Some(remote)
            } else if tracking.contains_key(b) {
                Some(b.clone())
            } else {
                None
            }
        });
        for w in rows.iter_mut() {
            w.base = base_ref.clone();
            if w.prunable || w.head.is_empty() {
                w.risks = removal_risks(w);
                continue;
            }
            if let Some(branch) = w.branch.clone() {
                if let Some((up, track)) = tracking.get(&branch) {
                    if !up.is_empty() {
                        w.upstream = Some(up.clone());
                        w.upstream_gone = track.trim() == "gone";
                        let (a, b) = parse_track(track);
                        (w.ahead, w.behind) = (a, b);
                    }
                }
                let suffix = format!("/{branch}");
                w.on_remote = (w.upstream.is_some() && !w.upstream_gone)
                    || remote_refs.iter().any(|r| r.ends_with(&suffix));
            }
            let is_base = match (&w.branch, &base_name) {
                (Some(b), Some(base)) => b == base,
                _ => false,
            };
            if let Some(base) = base_ref.as_deref().filter(|_| !is_base) {
                let range = format!("{base}..{}", w.head);
                w.unmerged = self.count(&["rev-list", "--count", &range]).await;
            } else if is_base {
                w.unmerged = Some(0);
            }
            // Commits on this HEAD nothing else holds: not on a remote, not
            // in the base; a detached HEAD must also be off every branch.
            let mut args = vec!["rev-list", "--count", w.head.as_str(), "--not", "--remotes"];
            if let Some(base) = base_ref.as_deref() {
                args.push(base);
            }
            if w.branch.is_none() {
                args.push("--branches");
            }
            w.unpushed = self.count(&args).await;
            w.risks = removal_risks(w);
        }
    }

    async fn count(&self, args: &[&str]) -> Option<u32> {
        self.run_read(args).await.ok()?.trim().parse().ok()
    }

    /// One worktree, freshly inspected for a removal decision: a status run
    /// with a generous budget (submodules included, as `git worktree remove`
    /// checks) plus the branch/commit details. The second value is the
    /// worktree's admin dir — `None` when its `.git` pointer doesn't round-trip
    /// to this repo (never touched then).
    pub async fn worktree_inspect(&self, path: &str) -> Result<(WorktreeInfo, Option<PathBuf>)> {
        let out = self.run_read(&["worktree", "list", "--porcelain"]).await?;
        let mut rows = crate::parse::parse_worktree_list(&out);
        let Some(i) = rows.iter().position(|w| w.path == path) else {
            return Err(Error::Invalid(format!(
                "'{path}' is not a worktree of this repo"
            )));
        };
        let mut w = rows.swap_remove(i);
        let mut admin = None;
        if !w.prunable {
            let common = self.common_dir().await;
            let p = PathBuf::from(&w.path);
            admin = match common {
                Some(c) => tokio::task::spawn_blocking(move || owned_admin_dir(&p, &c))
                    .await
                    .ok()
                    .flatten(),
                None => None,
            };
            if admin.is_some() {
                let child = LocalGit::new(&w.path).with_budget(INSPECT_BUDGET);
                let args = [
                    "status",
                    "--porcelain",
                    "--untracked-files=normal",
                    "--ignore-submodules=none",
                ];
                if let Ok((true, stdout, _, _)) =
                    child.run_raw_class(&args, &[], SpawnClass::LocalRead).await
                {
                    let (c, u) = porcelain_counts(stdout.as_bytes());
                    (w.changes, w.untracked, w.dirty, w.dirty_known) = (c, u, c + u > 0, true);
                }
            }
        }
        self.enrich_worktrees(std::slice::from_mut(&mut w)).await;
        if let Some(a) = admin.as_ref() {
            if a.join("modules").is_dir() {
                w.risks.push("it contains initialized submodules".into());
            }
        }
        Ok((w, admin))
    }

    /// Remove a linked worktree. Without `force` it refuses (409, naming each
    /// risk) when [`removal_risks`] finds anything to lose. Never deletes the
    /// branch. A worktree whose ownership can't be verified, or whose status
    /// is unknown, is refused even with `force` — Otto never deletes a
    /// directory it can't prove is this repo's worktree, or whose contents
    /// nobody has seen.
    pub async fn worktree_remove_safe(&self, path: &str, force: bool) -> Result<WorktreeRemoval> {
        Self::guard_path(path)?;
        let (w, admin) = self.worktree_inspect(path).await?;
        if w.is_main {
            return Err(Error::Invalid("cannot remove the main worktree".into()));
        }
        if w.prunable {
            self.worktree_prune().await?;
            return Ok(WorktreeRemoval::Pruned);
        }
        let Some(admin) = admin else {
            return Err(Error::Conflict(format!(
                "Otto couldn't verify that {path} is a worktree of this repo (its .git link \
                 doesn't point back here), so it won't delete it. Check it in a terminal \
                 with `git worktree list`."
            )));
        };
        // Unknown is never force-safe: nobody, the user included, has seen
        // what the tree holds. (A huge tree no longer lands here — the
        // single-tree check has a generous budget.)
        if !w.dirty_known {
            return Err(Error::Conflict(format!(
                "The status of the worktree at {path} is unknown (git status failed there), \
                 so Otto won't delete it. Inspect it in a terminal, then retry."
            )));
        }
        if !force && !w.risks.is_empty() {
            return Err(Error::Conflict(format!(
                "The worktree at {path} isn't safe to remove: {}. Confirm a force removal to \
                 discard it.",
                w.risks.join("; ")
            )));
        }
        let src = PathBuf::from(path);
        let trash = trash_path(&src);
        let moved = {
            let (src, trash) = (src.clone(), trash.clone());
            tokio::task::spawn_blocking(move || std::fs::rename(&src, &trash))
                .await
                .map_err(|e| Error::Internal(format!("rename task: {e}")))?
        };
        if let Err(e) = moved {
            tracing::info!(path, error = %e, "worktree rename failed; using git worktree remove");
            self.worktree_remove_checked(path, force).await?;
            return Ok(WorktreeRemoval::Git);
        }
        // Unregister: the admin dir is what `git worktree list` reads. Small
        // (HEAD, index, logs, a lock file), so inline.
        let admin_result =
            tokio::task::spawn_blocking(move || std::fs::remove_dir_all(&admin)).await;
        if !matches!(admin_result, Ok(Ok(()))) {
            // The dir is gone, so git now sees the entry as prunable.
            let _ = self.worktree_prune().await;
        }
        let bg = trash.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = std::fs::remove_dir_all(&bg) {
                tracing::warn!(path = %bg.display(), error = %e, "removed worktree: background delete failed");
            }
        });
        Ok(WorktreeRemoval::Detached { trash })
    }

    /// `git worktree remove` — the fallback of [`Self::worktree_remove_safe`] when
    /// the rename isn't possible. Surfaces git's error (unlike the reaper's
    /// best-effort [`Self::worktree_remove`]) and only forces when asked — git refuses
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

    /// `git stash list` with each entry's file count (tracked + untracked).
    pub async fn stash_list_counted(&self) -> Result<Vec<StashInfo>> {
        let mut list = self.stash_list().await?;
        if list.is_empty() {
            return Ok(list);
        }
        let out = self
            .run_read(&[
                "stash",
                "list",
                "--first-parent",
                "--no-renames",
                "--name-only",
                "--format=%x1e%H",
            ])
            .await
            .unwrap_or_default();
        let mut tracked: HashMap<String, HashSet<String>> = HashMap::new();
        for chunk in out.split('\u{1e}').filter(|c| !c.trim().is_empty()) {
            let mut lines = chunk.lines();
            let Some(sha) = lines.next().map(str::trim) else {
                continue;
            };
            let files = lines
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect();
            tracked.insert(sha.to_string(), files);
        }
        for s in list.iter_mut().take(STASH_COUNT_CAP) {
            let Some(mut files) = tracked.remove(&s.sha) else {
                continue;
            };
            if s.parents.len() >= 3 {
                let untracked = format!("{}^3", s.sha);
                if let Ok(out) = self
                    .run_read(&["ls-tree", "-r", "--name-only", &untracked])
                    .await
                {
                    files.extend(out.lines().map(str::to_string));
                }
            }
            s.files = Some(files.len() as u32);
        }
        Ok(list)
    }

    /// The files and patch of stash `sha` (`git stash show -p
    /// --include-untracked`, assembled from plumbing so binary/rename handling
    /// and the size cap stay ours). 400 when the sha isn't on the stash list.
    pub async fn stash_detail(&self, sha: &str) -> Result<StashDetail> {
        Self::guard_ref(sha)?;
        let Some(entry) = self.stash_list().await?.into_iter().find(|s| s.sha == sha) else {
            return Err(Error::Invalid(format!("stash {sha} no longer exists")));
        };
        let base = format!("{sha}^1");
        let diff = |mode: &'static str| {
            let mut a = vec![
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--no-renames",
            ];
            if !mode.is_empty() {
                a.push(mode);
            }
            a.push(base.as_str());
            a.push(sha);
            a
        };
        let mut files: Vec<StashFile> = Vec::new();
        let ns = self.run_read(&diff("--name-status")).await?;
        let num = self.run_read(&diff("--numstat")).await?;
        let counts: HashMap<String, LineCounts> = parse_numstat(&num).into_iter().collect();
        for line in ns.lines() {
            let Some((code, path)) = line.split_once('\t') else {
                continue;
            };
            let (additions, deletions) = counts.get(path).copied().unwrap_or((None, None));
            files.push(StashFile {
                path: path.to_string(),
                status: status_word(code).into(),
                additions,
                deletions,
            });
        }
        let mut patch = self.run_read(&diff("")).await?;
        if entry.parents.len() >= 3 {
            let untracked = format!("{sha}^3");
            let show = |extra: &'static str| {
                let u = untracked.clone();
                async move {
                    self.run_read(&[
                        "show",
                        "--format=",
                        extra,
                        "--no-ext-diff",
                        "--no-textconv",
                        "--no-color",
                        &u,
                    ])
                    .await
                }
            };
            let counts = parse_numstat(&show("--numstat").await.unwrap_or_default());
            for (path, (additions, deletions)) in counts {
                files.push(StashFile {
                    path,
                    status: "untracked".into(),
                    additions,
                    deletions,
                });
            }
            if let Ok(p) = show("--patch").await {
                if !patch.is_empty() && !patch.ends_with('\n') {
                    patch.push('\n');
                }
                patch.push_str(&p);
            }
        }
        let truncated = patch.len() > STASH_PATCH_CAP;
        if truncated {
            let mut cut = STASH_PATCH_CAP;
            while !patch.is_char_boundary(cut) {
                cut -= 1;
            }
            patch.truncate(cut);
        }
        Ok(StashDetail {
            sha: sha.to_string(),
            files,
            patch,
            truncated,
        })
    }

    /// Pop the stash with commit `sha` (not whatever sits at `stash@{0}`):
    /// apply it, then drop it unless the apply conflicted — git keeps a
    /// conflicting stash, and so do we.
    pub async fn stash_pop_sha(&self, sha: &str) -> Result<String> {
        let out = self.stash_apply(sha).await?;
        if !out.contains("CONFLICT") {
            self.stash_drop(sha).await?;
        }
        Ok(out)
    }
}

/// `"ahead 2, behind 1"` / `"ahead 3"` / `"gone"` / `""` → (ahead, behind).
fn parse_track(track: &str) -> (u32, u32) {
    let (mut a, mut b) = (0, 0);
    for part in track.split(',') {
        let mut it = part.split_whitespace();
        match (it.next(), it.next().and_then(|n| n.parse().ok())) {
            (Some("ahead"), Some(n)) => a = n,
            (Some("behind"), Some(n)) => b = n,
            _ => {}
        }
    }
    (a, b)
}

/// (additions, deletions) of one file; `None`s for a binary file.
type LineCounts = (Option<u32>, Option<u32>);

/// `git diff --numstat` → path → (additions, deletions); binary = `None`s.
fn parse_numstat(out: &str) -> Vec<(String, LineCounts)> {
    out.lines()
        .filter_map(|l| {
            let mut it = l.splitn(3, '\t');
            let (a, d, p) = (it.next()?, it.next()?, it.next()?);
            Some((p.to_string(), (a.parse().ok(), d.parse().ok())))
        })
        .collect()
}

fn status_word(code: &str) -> &'static str {
    match code.chars().next() {
        Some('A') => "added",
        Some('M') => "modified",
        Some('D') => "deleted",
        Some('R') => "renamed",
        Some('C') => "copied",
        Some('T') => "type-changed",
        _ => "changed",
    }
}

/// A hidden sibling of `src` (same parent ⇒ same volume ⇒ rename is atomic).
fn trash_path(src: &Path) -> PathBuf {
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "worktree".into());
    let parent = src.parent().unwrap_or(src);
    parent.join(format!(".{name}.otto-removed-{}", otto_core::new_id()))
}

#[cfg(test)]
#[path = "gitops_tests.rs"]
mod tests;
