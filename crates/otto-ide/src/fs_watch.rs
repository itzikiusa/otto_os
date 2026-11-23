//! Live project-tree refresh: one OS file watcher (FSEvents on macOS, inotify
//! on Linux) per open local IDE project root.
//!
//! The tree lists a folder once per expand, so a file created by a terminal,
//! an agent or a `git checkout` stayed invisible until the folder was closed
//! and re-opened. Now each IDE window holds a lease (`POST /ide/fs/watch`): its
//! project roots plus the folders it has expanded. A change that creates,
//! removes or renames an entry directly inside one of those folders emits one
//! [`Event::IdeFsChanged`] naming the folders to re-list.
//!
//! Cheap by construction: only expanded folders count, so a build writing into
//! a collapsed `target/` (or `node_modules`, or `.git` internals) sends
//! nothing, and content-only writes (a save) never re-list. A burst — a branch
//! switch touching 10k files — is debounced per root into one event carrying
//! at most the window's expanded folders.
//!
//! Bounded: a root is watched while at least one lease names it. A window
//! re-posts its lease every minute and releases it (empty `roots`) when it
//! leaves the project; a lease silent for [`LEASE_TTL`] (a closed tab, a
//! crashed window) is dropped. Remote SSH targets are not watched.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};

use notify::event::ModifyKind;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use otto_core::event::Event;
use otto_core::Id;
use tokio::sync::broadcast;

/// Trailing debounce after the first change of a burst.
const DEBOUNCE: Duration = Duration::from_millis(150);
/// Minimum spacing between two flushes of the same root.
const MIN_GAP: Duration = Duration::from_millis(400);
/// A lease not re-posted for this long is dropped (the UI re-posts every 60 s).
pub const LEASE_TTL: Duration = Duration::from_secs(180);
/// How often expired leases are swept while any exist.
const REAP_EVERY: Duration = Duration::from_secs(60);

type LeaseKey = (Id, String);

/// One window's view of a project: the roots it shows and the folders it has
/// expanded (canonical path → the client's spelling, echoed back in events).
struct Lease {
    roots: Vec<PathBuf>,
    dirs: HashMap<PathBuf, String>,
    touched: Instant,
}

/// What the notify callback thread shares with the flush task.
struct RootShared {
    root: PathBuf,
    /// Union of every lease's expanded folders under `root`.
    interest: RwLock<HashSet<PathBuf>>,
    pending: Mutex<Pending>,
}

#[derive(Default)]
struct Pending {
    dirs: HashSet<PathBuf>,
    /// A dropped-events rescan: every watched folder may be stale.
    all: bool,
    scheduled: bool,
    last_flush: Option<Instant>,
}

struct RootWatch {
    /// Owns the OS stream; its callback holds the `RootShared`.
    _watcher: RecommendedWatcher,
    shared: Arc<RootShared>,
}

#[derive(Default)]
struct Inner {
    leases: HashMap<LeaseKey, Lease>,
    roots: HashMap<PathBuf, RootWatch>,
    events: Option<broadcast::Sender<Event>>,
    reaping: bool,
}

/// Registry of project-root watchers and the leases that keep them alive.
#[derive(Default)]
pub struct FsWatchRegistry {
    inner: Arc<Mutex<Inner>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Can this change add, drop or rename a directory entry? Content and
/// metadata writes cannot; an unknown kind might.
fn changes_listing(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(ModifyKind::Name(_))
            | EventKind::Any
            | EventKind::Other
    )
}

/// `dir` (as the client spells it) mapped under the canonical root whose
/// spelling is its longest prefix; `None` when it is under none of them.
fn canonical_dir(dir: &str, roots: &[(String, PathBuf)]) -> Option<PathBuf> {
    let dir = dir.trim_end_matches('/');
    roots
        .iter()
        .filter_map(|(spelled, canon)| {
            let spelled = spelled.trim_end_matches('/');
            let rest = dir.strip_prefix(spelled)?;
            if rest.is_empty() {
                return Some((spelled.len(), canon.clone()));
            }
            let rest = rest.strip_prefix('/')?;
            Some((spelled.len(), canon.join(rest)))
        })
        .max_by_key(|(len, _)| *len)
        .map(|(_, p)| p)
}

/// The events one flush of `root` sends: per lease watching it, the lease's
/// changed folders (all of them under `root` after a rescan) in its spelling.
fn events_for(
    leases: &HashMap<LeaseKey, Lease>,
    root: &Path,
    dirs: &HashSet<PathBuf>,
    all: bool,
) -> Vec<Event> {
    let mut out = Vec::new();
    for ((user_id, client_id), lease) in leases {
        if !lease.roots.iter().any(|r| r == root) {
            continue;
        }
        let mut hit: Vec<String> = lease
            .dirs
            .iter()
            .filter(|(c, _)| c.starts_with(root) && (all || dirs.contains(*c)))
            .map(|(_, s)| s.clone())
            .collect();
        if hit.is_empty() {
            continue;
        }
        hit.sort();
        out.push(Event::IdeFsChanged {
            user_id: user_id.clone(),
            client_id: client_id.clone(),
            dirs: hit,
        });
    }
    out
}

impl FsWatchRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the lease of `client_id` (one IDE window of `user_id`): the
    /// `(spelling, resolved path)` of each project root it shows and the
    /// folders it has expanded. Empty `roots` releases it. Arms watchers for
    /// newly named roots, drops those no lease names any more, and returns the
    /// roots (client spelling) that are watched. Blocking (canonicalizes and
    /// arms OS watchers): call it from `spawn_blocking` inside a tokio runtime.
    pub fn set_lease(
        &self,
        user_id: &Id,
        client_id: &str,
        roots: Vec<(String, PathBuf)>,
        dirs: &[String],
        events: broadcast::Sender<Event>,
    ) -> Vec<String> {
        let roots: Vec<(String, PathBuf)> = roots
            .into_iter()
            .filter_map(|(s, p)| Some((s, std::fs::canonicalize(p).ok()?)))
            .collect();
        let key = (user_id.clone(), client_id.to_string());
        let mut g = lock(&self.inner);
        g.events = Some(events);
        if roots.is_empty() {
            g.leases.remove(&key);
        } else {
            let lease = Lease {
                roots: roots.iter().map(|(_, c)| c.clone()).collect(),
                dirs: dirs
                    .iter()
                    .filter_map(|d| Some((canonical_dir(d, &roots)?, d.clone())))
                    .collect(),
                touched: Instant::now(),
            };
            g.leases.insert(key, lease);
        }
        self.reconcile(&mut g, Instant::now());
        if !g.leases.is_empty() && !g.reaping {
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                g.reaping = true;
                rt.spawn(reap_loop(Arc::downgrade(&self.inner)));
            }
        }
        roots
            .into_iter()
            .filter(|(_, c)| g.roots.contains_key(c))
            .map(|(s, _)| s)
            .collect()
    }

    /// Number of project roots with a live OS watcher.
    pub fn watched_roots(&self) -> usize {
        lock(&self.inner).roots.len()
    }

    /// Drop leases silent since before `now - LEASE_TTL` and their watchers.
    #[cfg(test)]
    pub(crate) fn reap_at(&self, now: Instant) {
        let mut g = lock(&self.inner);
        self.reconcile(&mut g, now);
    }

    /// Expire stale leases, arm/disarm roots to match the live ones, and
    /// refresh each root's interest set.
    fn reconcile(&self, g: &mut Inner, now: Instant) {
        g.leases
            .retain(|_, l| now.saturating_duration_since(l.touched) < LEASE_TTL);
        let wanted: HashSet<PathBuf> = g
            .leases
            .values()
            .flat_map(|l| l.roots.iter().cloned())
            .collect();
        g.roots.retain(|r, _| wanted.contains(r));
        for root in wanted {
            if let Entry::Vacant(slot) = g.roots.entry(root) {
                if let Some(w) = arm(slot.key(), Arc::downgrade(&self.inner)) {
                    slot.insert(w);
                }
            }
        }
        for (root, w) in &g.roots {
            let interest = g
                .leases
                .values()
                .flat_map(|l| l.dirs.keys())
                .filter(|d| d.starts_with(root))
                .cloned()
                .collect();
            *w.shared.interest.write().unwrap_or_else(|e| e.into_inner()) = interest;
        }
    }
}

async fn reap_loop(inner: Weak<Mutex<Inner>>) {
    loop {
        tokio::time::sleep(REAP_EVERY).await;
        let Some(inner) = inner.upgrade() else { return };
        let reg = FsWatchRegistry { inner };
        let mut g = lock(&reg.inner);
        reg.reconcile(&mut g, Instant::now());
        if g.leases.is_empty() {
            g.reaping = false;
            return;
        }
    }
}

/// Start watching `root` (canonical). `None` for a broad non-project root
/// (`/`, `/Users`, …) or when the OS refuses the watch: that tree still
/// refreshes on expand and on window focus.
fn arm(root: &Path, inner: Weak<Mutex<Inner>>) -> Option<RootWatch> {
    if otto_core::paths::is_broad_non_project_root(root) {
        return None;
    }
    let rt = tokio::runtime::Handle::try_current().ok()?;
    let shared = Arc::new(RootShared {
        root: root.to_path_buf(),
        interest: RwLock::new(HashSet::new()),
        pending: Mutex::new(Pending::default()),
    });
    let cb = shared.clone();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(ev) = res else { return };
        let rescan = ev.need_rescan();
        if !rescan && !changes_listing(&ev.kind) {
            return;
        }
        let hits: Vec<PathBuf> = {
            let interest = cb.interest.read().unwrap_or_else(|e| e.into_inner());
            ev.paths
                .iter()
                .filter_map(|p| p.parent())
                .filter(|d| interest.contains(*d))
                .map(Path::to_path_buf)
                .collect()
        };
        if !rescan && hits.is_empty() {
            return;
        }
        let mut p = lock(&cb.pending);
        p.all |= rescan;
        p.dirs.extend(hits);
        if !p.scheduled {
            p.scheduled = true;
            rt.spawn(flush_later(inner.clone(), cb.clone()));
        }
    })
    .ok()?;
    watcher.watch(root, RecursiveMode::Recursive).ok()?;
    Some(RootWatch {
        _watcher: watcher,
        shared,
    })
}

/// Wait out the debounce (and the per-root gap), then send what piled up.
async fn flush_later(inner: Weak<Mutex<Inner>>, shared: Arc<RootShared>) {
    let wait = {
        let p = lock(&shared.pending);
        p.last_flush
            .map(|t| MIN_GAP.saturating_sub(t.elapsed()))
            .unwrap_or_default()
            .max(DEBOUNCE)
    };
    tokio::time::sleep(wait).await;
    // Taken and unscheduled together: a change landing after this schedules
    // the next flush instead of being swallowed.
    let (dirs, all) = {
        let mut p = lock(&shared.pending);
        p.scheduled = false;
        p.last_flush = Some(Instant::now());
        (std::mem::take(&mut p.dirs), std::mem::take(&mut p.all))
    };
    let Some(inner) = inner.upgrade() else { return };
    let g = lock(&inner);
    let Some(tx) = g.events.clone() else { return };
    for ev in events_for(&g.leases, &shared.root, &dirs, all) {
        let _ = tx.send(ev);
    }
}

#[cfg(test)]
#[path = "fs_watch_tests.rs"]
mod tests;
