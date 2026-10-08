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
/// A sustained stream must still make progress; neither queued events nor a
/// single event may retain an unbounded list of paths.
const MAX_BURST_MS: u64 = 1000;
const MAX_QUEUED_EVENTS: usize = 16;

/// One running watcher. Dropping it stops the FSEvents stream; the debounce
/// task then sees its channel close and exits.
pub(crate) struct VaultWatch {
    _watcher: notify::RecommendedWatcher,
    pub(crate) healthy: Arc<AtomicBool>,
}

#[derive(Debug)]
enum Touch {
    Paths(Vec<PathBuf>),
    /// Rescan needed regardless of paths (overflow / rescan flag / error).
    Rescan,
}

/// Record dropped paths after a full queue.
fn request_rescan(tx: &tokio::sync::mpsc::Sender<Touch>, overflow: &AtomicBool) {
    overflow.store(true, Ordering::Release);
    // The consumer may have drained the queue before the flag was published.
    // Sending again either wakes it or proves queued work exists AFTER the
    // publication; a closed receiver no longer needs a rescan.
    let _ = tx.try_send(Touch::Rescan);
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
        let (tx, rx) = tokio::sync::mpsc::channel::<Touch>(MAX_QUEUED_EVENTS);
        let overflow = Arc::new(AtomicBool::new(false));
        let cb_overflow = overflow.clone();
        let healthy = Arc::new(AtomicBool::new(true));
        let cb_healthy = healthy.clone();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let touch = match res {
                Ok(ev) if matches!(ev.kind, notify::EventKind::Access(_)) => return,
                Ok(ev) if ev.need_rescan() || ev.paths.len() > MAX_CHECKED_PATHS => Touch::Rescan,
                Ok(ev) => Touch::Paths(ev.paths),
                Err(_) => {
                    cb_healthy.store(false, Ordering::Relaxed);
                    Touch::Rescan
                }
            };
            if matches!(
                tx.try_send(touch),
                Err(tokio::sync::mpsc::error::TrySendError::Full(_))
            ) {
                // Preserve dropped paths and ensure the consumer observes the
                // overflow even if it drained the queue concurrently.
                request_rescan(&tx, &cb_overflow);
            }
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
        tokio::spawn(Self::watch_loop(
            Arc::downgrade(self),
            id,
            root,
            rx,
            overflow,
        ));
        true
    }

    async fn watch_loop(
        eng: Weak<Self>,
        id: i64,
        root: PathBuf,
        mut rx: tokio::sync::mpsc::Receiver<Touch>,
        overflow: Arc<AtomicBool>,
    ) {
        while let Some(first) = rx.recv().await {
            let mut rescan = false;
            let mut paths: HashSet<String> = HashSet::new();
            let mut absorb = |t: Touch, rescan: &mut bool| {
                if *rescan {
                    return;
                }
                match t {
                    Touch::Rescan => {
                        *rescan = true;
                        paths.clear();
                    }
                    Touch::Paths(ps) => {
                        for p in ps {
                            if let Some(rel) = indexable_rel(&root, &p) {
                                paths.insert(rel);
                                if paths.len() > MAX_CHECKED_PATHS {
                                    *rescan = true;
                                    paths.clear();
                                    break;
                                }
                            }
                        }
                    }
                }
            };
            absorb(first, &mut rescan);
            let deadline =
                tokio::time::Instant::now() + std::time::Duration::from_millis(MAX_BURST_MS);
            // Bound a burst even when recv is perpetually ready. A timeout
            // alone may keep accepting immediately-ready messages forever.
            loop {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    break;
                }
                let quiet = now + std::time::Duration::from_millis(DEBOUNCE_MS);
                match tokio::time::timeout_at(deadline.min(quiet), rx.recv()).await {
                    Ok(Some(t)) => absorb(t, &mut rescan),
                    Ok(None) | Err(_) => break,
                }
            }
            rescan |= overflow.swap(false, Ordering::AcqRel);
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

    #[tokio::test]
    async fn continuous_events_do_not_starve_index_refresh() {
        let engine = Arc::new(VaultEngine::new(otto_state::db::test_pool().await));
        let dir = tempfile::tempdir().unwrap();
        let id = engine
            .store()
            .create_vault("ws", "Burst", dir.path().to_str().unwrap(), false)
            .await
            .unwrap();
        std::fs::write(dir.path().join("a.md"), "# Before").unwrap();
        engine.scan(id).await.unwrap();
        std::fs::write(dir.path().join("a.md"), "# After the external edit").unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(MAX_QUEUED_EVENTS);
        let task = tokio::spawn(VaultEngine::watch_loop(
            Arc::downgrade(&engine),
            id,
            dir.path().to_path_buf(),
            rx,
            Arc::new(AtomicBool::new(false)),
        ));
        let path = dir.path().join("a.md");
        let producer = tokio::spawn(async move {
            for _ in 0..100 {
                if tx.send(Touch::Paths(vec![path.clone()])).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
        });
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        let kicks = engine.watch_kicks.load(Ordering::Relaxed);
        producer.abort();
        task.abort();
        assert!(
            kicks > 0,
            "continuous events must publish a bounded burst before waiting for silence"
        );
    }

    #[tokio::test]
    async fn overflow_published_after_queue_drain_still_wakes_consumer() {
        let (tx, mut rx) = tokio::sync::mpsc::channel(1);
        let overflow = AtomicBool::new(false);
        tx.try_send(Touch::Paths(vec![])).unwrap();
        assert!(matches!(
            tx.try_send(Touch::Rescan),
            Err(tokio::sync::mpsc::error::TrySendError::Full(_))
        ));
        // Callback is descheduled immediately after Full. The consumer drains
        // and completes its batch before the callback publishes overflow.
        rx.try_recv().unwrap();
        assert!(!overflow.swap(false, Ordering::AcqRel));
        request_rescan(&tx, &overflow);
        assert!(overflow.load(Ordering::Acquire));
        assert!(
            matches!(rx.try_recv(), Ok(Touch::Rescan)),
            "late overflow must schedule a new batch even when no more filesystem events arrive"
        );
    }

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
