//! Per-vault filesystem watcher (F2). Replaces "walk the whole vault every
//! 30 s while the page is open" with "walk only when something on disk
//! actually changed".
//!
//! An FSEvents watcher (via `notify`) feeds touched paths into a per-vault
//! task that debounces them ([`DEBOUNCE_MS`]) and then checks ONLY those
//! paths: hidden segments (`.otto-history`, `.trash`, `.git`, …) and
//! `node_modules` are ignored, and a path whose lstat `(size, mtime)` still
//! equals its indexed signature is dropped — which is exactly what the
//! engine's own guarded writes look like after they re-index. Anything else
//! (an external edit, a new or removed note, a folder moved in/out, overflow)
//! kicks the regular incremental scan, which keeps all of its epoch /
//! `changed_since` fencing. While a watcher is healthy the freshness window
//! stretches to [`STALE_AFTER_WATCHED_SECS`] (a safety-net full walk); any
//! watcher error marks it unhealthy and the 30 s window returns.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};

use notify::{RecursiveMode, Watcher};

use crate::VaultEngine;

/// Quiet period after the last event before the touched paths are checked.
pub(crate) const DEBOUNCE_MS: u64 = 300;
/// Freshness window while the vault's watcher is healthy (safety net only).
pub(crate) const STALE_AFTER_WATCHED_SECS: i64 = 600;
/// More touched paths than this in one burst → just scan (bulk change).
const MAX_CHECKED_PATHS: usize = 256;

/// One running watcher. Dropping it stops the FSEvents stream; the debounce
/// task then sees its channel close and exits.
pub(crate) struct VaultWatch {
    _watcher: notify::RecommendedWatcher,
    pub(crate) healthy: Arc<AtomicBool>,
}

enum Touch {
    Paths(Vec<PathBuf>),
    /// Rescan needed regardless of paths (overflow / rescan flag / error).
    Rescan,
}

/// The vault-relative form of `abs`, or `None` when the scanner never
/// indexes it (outside the root, hidden segment, `node_modules`).
pub(crate) fn indexable_rel(root: &Path, abs: &Path) -> Option<String> {
    let rel = abs.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for c in rel.components() {
        let s = c.as_os_str().to_str()?;
        if crate::scan::is_skipped_dir(s) {
            return None;
        }
        parts.push(s);
    }
    // Same protected-dir filter as the walk (S7-303).
    {
        use otto_core::secret_paths as sp;
        let set = sp::protected_set();
        if !set.in_protected_dir(root) && (set.in_protected_dir(abs) || sp::is_denied_file(abs)) {
            return None;
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

impl VaultEngine {
    /// Start the watcher for `id` once (idempotent, non-blocking). Called from
    /// the freshness probe, so only vaults someone is looking at get one.
    pub(crate) fn ensure_watch(self: &Arc<Self>, id: i64) {
        if !self.watch_enabled.load(Ordering::Relaxed) {
            return;
        }
        {
            let mut starting = self.watch_starting.lock().unwrap();
            if self.watches.lock().unwrap().contains_key(&id) || !starting.insert(id) {
                return;
            }
        }
        let eng = Arc::downgrade(self);
        tokio::spawn(async move {
            let Some(e) = eng.upgrade() else { return };
            let started = match e.store().get_vault(id).await {
                Ok(v) => e.start_watch(id, PathBuf::from(v.root_path)),
                Err(_) => false,
            };
            e.watch_starting.lock().unwrap().remove(&id);
            if !started {
                tracing::debug!(vault = id, "vault watcher unavailable; polling freshness");
            }
        });
    }

    fn start_watch(self: &Arc<Self>, id: i64, root: PathBuf) -> bool {
        // FSEvents reports canonical paths (/private/var/… for /var/…).
        let root = std::fs::canonicalize(&root).unwrap_or(root);
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Touch>();
        let healthy = Arc::new(AtomicBool::new(true));
        let cb_healthy = healthy.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let touch = match res {
                Ok(ev) if matches!(ev.kind, notify::EventKind::Access(_)) => return,
                Ok(ev) if ev.need_rescan() => Touch::Rescan,
                Ok(ev) => Touch::Paths(ev.paths),
                Err(_) => {
                    cb_healthy.store(false, Ordering::Relaxed);
                    Touch::Rescan
                }
            };
            let _ = tx.send(touch);
        });
        let Ok(mut watcher) = watcher else {
            return false;
        };
        if watcher.watch(&root, RecursiveMode::Recursive).is_err() {
            return false;
        }
        self.watches.lock().unwrap().insert(
            id,
            VaultWatch {
                _watcher: watcher,
                healthy: healthy.clone(),
            },
        );
        tokio::spawn(Self::watch_loop(Arc::downgrade(self), id, root, rx));
        true
    }

    async fn watch_loop(
        eng: Weak<Self>,
        id: i64,
        root: PathBuf,
        mut rx: tokio::sync::mpsc::UnboundedReceiver<Touch>,
    ) {
        while let Some(first) = rx.recv().await {
            let mut rescan = false;
            let mut paths: HashSet<String> = HashSet::new();
            let mut absorb = |t: Touch, rescan: &mut bool| match t {
                Touch::Rescan => *rescan = true,
                Touch::Paths(ps) => {
                    for p in ps {
                        if let Some(rel) = indexable_rel(&root, &p) {
                            paths.insert(rel);
                        }
                    }
                }
            };
            absorb(first, &mut rescan);
            // Debounce: keep absorbing until the stream is quiet.
            loop {
                match tokio::time::timeout(std::time::Duration::from_millis(DEBOUNCE_MS), rx.recv())
                    .await
                {
                    Ok(Some(t)) => absorb(t, &mut rescan),
                    Ok(None) => return,
                    Err(_) => break,
                }
            }
            let Some(e) = eng.upgrade() else { return };
            if !rescan && paths.is_empty() {
                continue;
            }
            if rescan
                || paths.len() > MAX_CHECKED_PATHS
                || e.touched_paths_differ(id, &root, paths).await
            {
                #[cfg(test)]
                e.watch_kicks.fetch_add(1, Ordering::Relaxed);
                e.kick_scan(id);
            }
        }
    }

    /// True when any touched path disagrees with the index: its lstat
    /// `(size, mtime)` differs from the stored signature, it vanished while
    /// indexed, or it appeared unindexed. A directory counts only when the
    /// index disagrees about its subtree: present but nothing indexed under
    /// it (moved/copied in), or gone while notes are indexed under it (moved
    /// away) — FSEvents reports a folder move once, not per descendant.
    async fn touched_paths_differ(&self, id: i64, root: &Path, paths: HashSet<String>) -> bool {
        let sigs = match self
            .store()
            .sigs_for(id, paths.iter().map(String::as_str))
            .await
        {
            Ok(sigs) => sigs,
            Err(_) => return true,
        };
        let root = root.to_path_buf();
        // (changed, [(dir rel, present)] needing a subtree check)
        let probe = tokio::task::spawn_blocking(move || {
            let mut dirs = Vec::new();
            for rel in &paths {
                let indexed = sigs.get(rel);
                let changed = match std::fs::symlink_metadata(root.join(rel)) {
                    Ok(meta) if meta.is_dir() => {
                        dirs.push((rel.clone(), true));
                        false
                    }
                    Ok(meta) if meta.is_file() => indexed.is_none_or(|&(size, mtime)| {
                        size != meta.len() as i64 || mtime != crate::prepare::mtime(&meta)
                    }),
                    // Symlinks / FIFOs are never indexed by the walk.
                    Ok(_) => indexed.is_some(),
                    Err(_) if indexed.is_some() => true,
                    Err(_) => {
                        dirs.push((rel.clone(), false));
                        false
                    }
                };
                if changed {
                    return (true, Vec::new());
                }
            }
            (false, dirs)
        })
        .await;
        let Ok((changed, dirs)) = probe else {
            return true;
        };
        if changed {
            return true;
        }
        let state = self.index_state(id);
        let cache = state.cache.read().unwrap();
        let Some(ix) = cache.as_ref() else {
            return true;
        };
        dirs.iter().any(|(rel, present)| {
            let prefix = format!("{rel}/");
            let has_children = ix
                .records
                .range(prefix.clone()..)
                .next()
                .is_some_and(|(k, _)| k.starts_with(&prefix));
            has_children != *present
        })
    }

    /// Freshness window for `id`: long while its watcher is healthy.
    pub(crate) fn stale_after(&self, id: i64) -> Option<i64> {
        self.watches
            .lock()
            .unwrap()
            .get(&id)
            .filter(|w| w.healthy.load(Ordering::Relaxed))
            .map(|_| STALE_AFTER_WATCHED_SECS)
    }

    pub(crate) fn stop_watch(&self, id: i64) {
        self.watches.lock().unwrap().remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_and_outside_paths_are_not_indexable() {
        let root = Path::new("/v");
        assert_eq!(
            indexable_rel(root, Path::new("/v/a.md")).as_deref(),
            Some("a.md")
        );
        assert_eq!(
            indexable_rel(root, Path::new("/v/d/b.png")).as_deref(),
            Some("d/b.png")
        );
        assert_eq!(
            indexable_rel(root, Path::new("/v/.otto-history/x/meta.json")),
            None
        );
        assert_eq!(
            indexable_rel(root, Path::new("/v/d/.a.md.otto-tmp-1")),
            None
        );
        assert_eq!(indexable_rel(root, Path::new("/v/node_modules/x.md")), None);
        assert_eq!(indexable_rel(root, Path::new("/elsewhere/a.md")), None);
        assert_eq!(indexable_rel(root, Path::new("/v")), None);
    }
}
