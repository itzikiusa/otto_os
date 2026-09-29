//! Live working-tree change detection for the repositories the Git page has
//! open.
//!
//! Without this, a file saved in an editor only reached the Changes list when
//! the next auto-fetch round re-read status (every 30 s for the active repo —
//! a network `git fetch` just to notice a local edit). Now the first `status`
//! or `fetch` request for a repo arms an OS file watcher (FSEvents on macOS,
//! inotify on Linux) on its working tree; a relevant change emits
//! [`Event::RepoStatusChanged`] within ~150–400 ms and the client re-reads the
//! (local, lock-free) status.
//!
//! Noise is filtered before anything is sent: `.git` internals other than the
//! index / HEAD / refs / in-progress-operation markers, `node_modules`, and
//! paths the repo's top-level `.gitignore` files (plus `info/exclude`) ignore —
//! a `cargo build` in `target/` must not turn into a status storm. A path the
//! filter can't classify counts as relevant: the cost of a false positive is
//! one cheap status read.
//!
//! Bounded: at most [`MAX_WATCHED`] repos (least recently used evicted), a
//! watcher idles out after [`IDLE_TTL`] without a status/fetch request, and a
//! repo emits at most one event per [`MIN_GAP`] (trailing edge, so the last
//! change of a burst is never lost).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::time::{Duration, Instant};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use otto_core::event::Event;
use otto_core::Id;
use tokio::sync::{broadcast, mpsc};

/// Trailing debounce after the first change of a burst.
const DEBOUNCE: Duration = Duration::from_millis(150);
/// Minimum spacing between two events for the same repo.
const MIN_GAP: Duration = Duration::from_millis(400);
/// Drop a watcher nobody has asked about for this long.
const IDLE_TTL: Duration = Duration::from_secs(15 * 60);
/// Hard cap on simultaneously watched repositories.
const MAX_WATCHED: usize = 32;

/// `.git` entries whose change can alter `git status` output.
const GIT_DIR_RELEVANT: &[&str] = &[
    "index",
    "HEAD",
    "packed-refs",
    "MERGE_HEAD",
    "CHERRY_PICK_HEAD",
    "REVERT_HEAD",
    "REBASE_HEAD",
    "refs",
    "rebase-merge",
    "rebase-apply",
];

/// What the notify callback thread needs; shared with the dispatcher task.
struct RepoState {
    repo_id: Id,
    workspace_id: Id,
    root: PathBuf,
    git_dir: PathBuf,
    ignore: RwLock<Gitignore>,
    /// Set by the first change of a burst, cleared when its event is sent.
    pending: AtomicBool,
    last_emit: Mutex<Option<Instant>>,
}

struct Entry {
    /// Owns the OS stream; its callback holds the `RepoState`.
    _watcher: RecommendedWatcher,
    last_touch: Instant,
}

/// Registry of armed watchers. One per daemon (see [`global`]); tests build
/// their own with [`RepoWatchers::new`].
pub struct RepoWatchers {
    entries: Mutex<HashMap<Id, Entry>>,
    tx: mpsc::UnboundedSender<Arc<RepoState>>,
}

static GLOBAL: OnceLock<RepoWatchers> = OnceLock::new();

/// The daemon-wide registry, created on first use (must be inside a tokio
/// runtime: it spawns the dispatcher task).
pub fn global(events: &broadcast::Sender<Event>) -> &'static RepoWatchers {
    GLOBAL.get_or_init(|| RepoWatchers::new(events.clone()))
}

impl RepoWatchers {
    pub fn new(events: broadcast::Sender<Event>) -> Self {
        let (tx, mut rx) = mpsc::unbounded_channel::<Arc<RepoState>>();
        tokio::spawn(async move {
            while let Some(state) = rx.recv().await {
                let events = events.clone();
                tokio::spawn(async move {
                    let wait = {
                        let last = state.last_emit.lock().unwrap_or_else(|e| e.into_inner());
                        let gap = last
                            .map(|t| MIN_GAP.saturating_sub(t.elapsed()))
                            .unwrap_or_default();
                        gap.max(DEBOUNCE)
                    };
                    tokio::time::sleep(wait).await;
                    // Clear BEFORE sending: a change landing after this point
                    // schedules the next event instead of being swallowed.
                    state.pending.store(false, Ordering::SeqCst);
                    *state.last_emit.lock().unwrap_or_else(|e| e.into_inner()) =
                        Some(Instant::now());
                    let _ = events.send(Event::RepoStatusChanged {
                        workspace_id: state.workspace_id.clone(),
                        repo_id: state.repo_id.clone(),
                    });
                });
            }
        });
        Self {
            entries: Mutex::new(HashMap::new()),
            tx,
        }
    }

    /// Keep `repo_id` watched (arming a watcher on first call). Cheap when it
    /// is already armed. Never fails the caller: a watcher that can't be
    /// armed just means changes surface on the next poll instead.
    pub fn touch(&self, repo_id: &Id, workspace_id: &Id, root: &Path, git_dir: Option<&Path>) {
        let now = Instant::now();
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.retain(|_, e| now.duration_since(e.last_touch) < IDLE_TTL);
        if let Some(e) = entries.get_mut(repo_id) {
            e.last_touch = now;
            return;
        }
        let Some(entry) = self.arm(repo_id, workspace_id, root, git_dir, now) else {
            return;
        };
        if entries.len() >= MAX_WATCHED {
            if let Some(oldest) = entries
                .iter()
                .min_by_key(|(_, e)| e.last_touch)
                .map(|(k, _)| k.clone())
            {
                entries.remove(&oldest);
            }
        }
        entries.insert(repo_id.clone(), entry);
    }

    /// Stop watching (repo removed).
    pub fn forget(&self, repo_id: &Id) {
        self.entries
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(repo_id);
    }

    #[cfg(test)]
    fn watched(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    fn arm(
        &self,
        repo_id: &Id,
        workspace_id: &Id,
        root: &Path,
        git_dir: Option<&Path>,
        now: Instant,
    ) -> Option<Entry> {
        // FSEvents reports real paths (/private/var/…, resolved symlinks).
        let root = std::fs::canonicalize(root).ok()?;
        let git_dir = git_dir
            .and_then(|g| std::fs::canonicalize(g).ok())
            .unwrap_or_else(|| root.join(".git"));
        let state = Arc::new(RepoState {
            repo_id: repo_id.clone(),
            workspace_id: workspace_id.clone(),
            ignore: RwLock::new(build_ignore(&root, &git_dir)),
            root: root.clone(),
            git_dir: git_dir.clone(),
            pending: AtomicBool::new(false),
            last_emit: Mutex::new(None),
        });
        let cb_state = state;
        let tx = self.tx.clone();
        let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(ev) = res else { return };
            let mut hit = false;
            for p in &ev.paths {
                if p.file_name().is_some_and(|n| n == ".gitignore") {
                    let fresh = build_ignore(&cb_state.root, &cb_state.git_dir);
                    *cb_state.ignore.write().unwrap_or_else(|e| e.into_inner()) = fresh;
                }
                let ignore = cb_state.ignore.read().unwrap_or_else(|e| e.into_inner());
                if relevant(&cb_state.root, &cb_state.git_dir, p, &ignore) {
                    hit = true;
                    break;
                }
            }
            // A rescan request (dropped kernel events) may hide anything.
            if (hit || ev.need_rescan()) && !cb_state.pending.swap(true, Ordering::SeqCst) {
                let _ = tx.send(cb_state.clone());
            }
        })
        .ok()?;
        watcher.watch(&root, RecursiveMode::Recursive).ok()?;
        // A linked worktree's git dir lives outside the tree (under the main
        // repo's .git/worktrees/<name>): its index/HEAD need their own watch.
        if !git_dir.starts_with(&root) {
            let _ = watcher.watch(&git_dir, RecursiveMode::NonRecursive);
        }
        Some(Entry {
            _watcher: watcher,
            last_touch: now,
        })
    }
}

/// Top-level `.gitignore`, one level of sub-directory `.gitignore` files (the
/// usual home of `node_modules`/`dist`/`target` rules), and `info/exclude`.
fn build_ignore(root: &Path, git_dir: &Path) -> Gitignore {
    let mut b = GitignoreBuilder::new(root);
    let _ = b.add(root.join(".gitignore"));
    let _ = b.add(git_dir.join("info").join("exclude"));
    if let Ok(rd) = std::fs::read_dir(root) {
        for e in rd.flatten().take(512) {
            let p = e.path();
            if p.is_dir() && p.file_name().is_some_and(|n| n != ".git") {
                let gi = p.join(".gitignore");
                if gi.is_file() {
                    let _ = b.add(gi);
                }
            }
        }
    }
    b.build().unwrap_or_else(|_| Gitignore::empty())
}

/// Could a change at `path` alter this repo's `git status`?
fn relevant(root: &Path, git_dir: &Path, path: &Path, ignore: &Gitignore) -> bool {
    if let Ok(rel) = path.strip_prefix(git_dir) {
        return match rel.components().next() {
            // The git dir itself (e.g. a rescan of `.git`).
            None => true,
            Some(first) => GIT_DIR_RELEVANT.iter().any(|n| first.as_os_str() == *n),
        };
    }
    let Ok(rel) = path.strip_prefix(root) else {
        return false;
    };
    if rel
        .components()
        .any(|c| c.as_os_str() == ".git" || c.as_os_str() == "node_modules")
    {
        return false;
    }
    if rel.as_os_str().is_empty() {
        return true;
    }
    !ignore.matched_path_or_any_parents(path, false).is_ignore()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_with_ignore(ignore: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join(".git/objects/ab")).unwrap();
        std::fs::create_dir_all(root.join(".git/refs/heads")).unwrap();
        std::fs::write(root.join(".gitignore"), ignore).unwrap();
        (dir, root)
    }

    #[test]
    fn filters_git_internals_ignored_paths_and_node_modules() {
        let (_d, root) = repo_with_ignore("target/\n*.log\n");
        std::fs::create_dir_all(root.join("ui")).unwrap();
        std::fs::write(root.join("ui/.gitignore"), "dist\n").unwrap();
        let git_dir = root.join(".git");
        let ig = build_ignore(&root, &git_dir);
        let rel = |p: &str| relevant(&root, &git_dir, &root.join(p), &ig);

        assert!(rel("src/main.rs"));
        assert!(rel("README.md"));
        assert!(rel(".gitignore"));
        assert!(rel(".git/index"));
        assert!(rel(".git/HEAD"));
        assert!(rel(".git/refs/heads/main"));
        assert!(rel(".git/MERGE_HEAD"));

        assert!(!rel(".git/objects/ab/cdef"));
        assert!(!rel(".git/index.lock"));
        assert!(!rel(".git/FETCH_HEAD"));
        assert!(!rel(".git/logs/HEAD"));
        assert!(!rel("target/debug/app"));
        assert!(!rel("build.log"));
        assert!(!rel("ui/dist/index.html"));
        assert!(!rel("ui/node_modules/x/index.js"));
        assert!(!relevant(
            &root,
            &git_dir,
            Path::new("/elsewhere/file"),
            &ig
        ));
    }

    async fn next_change(rx: &mut broadcast::Receiver<Event>, within: Duration) -> Option<Id> {
        let deadline = tokio::time::Instant::now() + within;
        loop {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Ok(Event::RepoStatusChanged { repo_id, .. })) => return Some(repo_id),
                Ok(Ok(_)) => continue,
                _ => return None,
            }
        }
    }

    #[tokio::test]
    async fn an_edit_emits_one_event_and_ignored_churn_emits_none() {
        let (_d, root) = repo_with_ignore("target/\n");
        std::fs::create_dir_all(root.join("target")).unwrap();
        let (tx, mut rx) = broadcast::channel(64);
        let w = RepoWatchers::new(tx);
        w.touch(&"r1".to_string(), &"w1".to_string(), &root, None);
        assert_eq!(w.watched(), 1);
        // Let the OS stream start before producing changes.
        tokio::time::sleep(Duration::from_millis(500)).await;

        std::fs::write(root.join("target/out.o"), b"x").unwrap();
        assert_eq!(
            next_change(&mut rx, Duration::from_millis(1500)).await,
            None
        );

        // A burst of edits coalesces into (at most a couple of) events, the
        // first arriving quickly.
        let t0 = Instant::now();
        for i in 0..20 {
            std::fs::write(root.join("lib.rs"), format!("fn f() {{ {i} }}")).unwrap();
        }
        assert_eq!(
            next_change(&mut rx, Duration::from_secs(5))
                .await
                .as_deref(),
            Some("r1")
        );
        assert!(t0.elapsed() < Duration::from_secs(3), "{:?}", t0.elapsed());
        let mut extra = 0;
        while next_change(&mut rx, Duration::from_millis(800))
            .await
            .is_some()
        {
            extra += 1;
        }
        assert!(extra <= 2, "burst produced {extra} extra events");

        // Touching again keeps one watcher; forget drops it.
        w.touch(&"r1".to_string(), &"w1".to_string(), &root, None);
        assert_eq!(w.watched(), 1);
        w.forget(&"r1".to_string());
        assert_eq!(w.watched(), 0);
    }
}
