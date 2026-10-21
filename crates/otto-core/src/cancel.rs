//! Per-run cancellation flags shared by the daemon's long-running engines
//! (skill evals, swarm runs, product analysis, reviews).
//!
//! A run registers an `Arc<AtomicBool>` under its id; the Stop handler flips
//! it; the run's loop polls it. The operations live here ONCE
//! ([`CancelRegistryExt`]) — they used to be re-implemented in six engines
//! with diverging semantics (some removed a newer retry's flag on exit, one
//! panicked on a poisoned lock), so a fix landed in one copy and not the rest.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// `run id → cancel flag`.
pub type CancelRegistry = Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>;

/// An empty registry.
pub fn new_cancel_registry() -> CancelRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

/// The registry operations every engine shares. Locking is poison-tolerant: a
/// panic elsewhere while holding the lock must not stop Cancel from working
/// (the map is always left consistent — every mutation is a single call).
pub trait CancelRegistryExt {
    /// The locked map, recovering from poisoning. For callers that must act
    /// atomically with a registry check (e.g. "sweep only while idle").
    fn lock_map(&self) -> MutexGuard<'_, HashMap<String, Arc<AtomicBool>>>;

    /// Register a FRESH, un-tripped flag under `id`, replacing any previous one
    /// (a retry must not inherit a stale tripped flag). The returned guard
    /// unregisters it on drop — only while the registry still holds THIS flag.
    fn register(&self, id: &str) -> CancelGuard;

    /// Like [`register`](Self::register), but joins the flag already registered
    /// under `id` (e.g. one the launch handler pre-registered) instead of
    /// replacing it.
    fn join(&self, id: &str) -> CancelGuard;

    /// Register `id` only if nothing is registered under it yet (one attempt
    /// per key); `None` when another attempt holds it.
    fn claim(&self, id: &str) -> Option<CancelGuard>;

    /// The flag registered under `id`, if any.
    fn flag(&self, id: &str) -> Option<Arc<AtomicBool>>;

    /// Trip the flag registered under `id`. Never inserts one: a tripped
    /// orphan would cancel a later rerun of the same id. Returns whether a
    /// flag was registered.
    fn signal(&self, id: &str) -> bool;

    /// Trip every flag whose key starts with `prefix`; returns how many.
    fn signal_prefix(&self, prefix: &str) -> usize;

    /// Whether a flag is registered under exactly `id`.
    fn is_live(&self, id: &str) -> bool;

    /// Whether any flag is registered under a key starting with `prefix`.
    fn is_live_prefix(&self, prefix: &str) -> bool;

    /// Whether `id` is registered AND its flag is tripped.
    fn is_signalled(&self, id: &str) -> bool;
}

impl CancelRegistryExt for CancelRegistry {
    fn lock_map(&self) -> MutexGuard<'_, HashMap<String, Arc<AtomicBool>>> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn register(&self, id: &str) -> CancelGuard {
        let flag = Arc::new(AtomicBool::new(false));
        self.lock_map().insert(id.to_string(), Arc::clone(&flag));
        CancelGuard::new(self, id, flag)
    }

    fn join(&self, id: &str) -> CancelGuard {
        let flag = Arc::clone(
            self.lock_map()
                .entry(id.to_string())
                .or_insert_with(|| Arc::new(AtomicBool::new(false))),
        );
        CancelGuard::new(self, id, flag)
    }

    fn claim(&self, id: &str) -> Option<CancelGuard> {
        let mut map = self.lock_map();
        if map.contains_key(id) {
            return None;
        }
        let flag = Arc::new(AtomicBool::new(false));
        map.insert(id.to_string(), Arc::clone(&flag));
        drop(map);
        Some(CancelGuard::new(self, id, flag))
    }

    fn flag(&self, id: &str) -> Option<Arc<AtomicBool>> {
        self.lock_map().get(id).cloned()
    }

    fn signal(&self, id: &str) -> bool {
        match self.lock_map().get(id) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    fn signal_prefix(&self, prefix: &str) -> usize {
        let map = self.lock_map();
        let mut tripped = 0;
        for (key, flag) in map.iter() {
            if key.starts_with(prefix) {
                flag.store(true, Ordering::SeqCst);
                tripped += 1;
            }
        }
        tripped
    }

    fn is_live(&self, id: &str) -> bool {
        self.lock_map().contains_key(id)
    }

    fn is_live_prefix(&self, prefix: &str) -> bool {
        self.lock_map().keys().any(|k| k.starts_with(prefix))
    }

    fn is_signalled(&self, id: &str) -> bool {
        self.lock_map()
            .get(id)
            .is_some_and(|f| f.load(Ordering::SeqCst))
    }
}

/// One attempt's registration (RAII). Dropping it — or [`release`]-ing it —
/// unregisters the flag only while the registry still maps the id to THIS
/// flag: a quick Stop + Retry registers a newer flag under the same id, and
/// the old attempt unwinding must not remove it (a later Stop would go
/// unheard) — S4-16.
///
/// [`release`]: CancelGuard::release
pub struct CancelGuard {
    reg: CancelRegistry,
    id: String,
    flag: Arc<AtomicBool>,
    released: bool,
}

impl CancelGuard {
    fn new(reg: &CancelRegistry, id: &str, flag: Arc<AtomicBool>) -> Self {
        Self {
            reg: Arc::clone(reg),
            id: id.to_string(),
            flag,
            released: false,
        }
    }

    /// The registry key.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// This attempt's flag (hand a clone to the loop that polls it).
    pub fn flag(&self) -> &Arc<AtomicBool> {
        &self.flag
    }

    /// Whether this attempt's flag is tripped.
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }

    /// Unregister now. Returns `false` when a newer attempt superseded this
    /// one (the registry maps the id to another flag, or to nothing).
    pub fn release(mut self) -> bool {
        self.released = true;
        self.unregister()
    }

    fn unregister(&self) -> bool {
        let mut map = self.reg.lock_map();
        if map
            .get(&self.id)
            .is_some_and(|cur| Arc::ptr_eq(cur, &self.flag))
        {
            map.remove(&self.id);
            true
        } else {
            false
        }
    }
}

impl Drop for CancelGuard {
    fn drop(&mut self) {
        if !self.released {
            self.unregister();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flag_registered_by_a_run_is_seen_by_the_canceller() {
        let reg = new_cancel_registry();
        let guard = reg.register("run");
        assert!(reg.signal("run"));
        assert!(guard.is_cancelled());
        assert!(reg.is_signalled("run"));
    }

    #[test]
    fn signal_never_inserts_an_orphan_flag() {
        let reg = new_cancel_registry();
        assert!(!reg.signal("gone"));
        assert!(!reg.is_live("gone"));
    }

    #[test]
    fn dropping_the_guard_unregisters_its_own_flag() {
        let reg = new_cancel_registry();
        drop(reg.register("run"));
        assert!(reg.lock_map().is_empty());
    }

    /// S4-16: an old attempt unwinding never removes a Retry's newer flag.
    #[test]
    fn a_superseded_guard_keeps_the_newer_flag() {
        let reg = new_cancel_registry();
        let old = reg.register("a1");
        let new = reg.register("a1"); // Retry re-registers
        assert!(!new.is_cancelled(), "the retry starts un-tripped");
        assert!(!old.release(), "old attempt is superseded");
        assert!(Arc::ptr_eq(&reg.flag("a1").unwrap(), new.flag()));
        // A Stop still reaches the retry.
        assert!(reg.signal("a1"));
        assert!(new.is_cancelled());
        assert!(new.release());
        assert!(!reg.is_live("a1"));
    }

    #[test]
    fn a_superseded_guard_dropped_after_its_successor_finished_is_a_noop() {
        let reg = new_cancel_registry();
        let old = reg.register("a1");
        let new = reg.register("a1");
        assert!(new.release());
        assert!(!old.release());
        assert!(!reg.is_live("a1"));
    }

    #[test]
    fn join_shares_the_preregistered_flag() {
        let reg = new_cancel_registry();
        let launch = reg.register("r");
        let run = reg.join("r");
        assert!(Arc::ptr_eq(launch.flag(), run.flag()));
        reg.signal("r");
        assert!(run.is_cancelled());
    }

    #[test]
    fn claim_admits_one_attempt_per_key() {
        let reg = new_cancel_registry();
        let first = reg.claim("e:retry:0").unwrap();
        assert!(reg.claim("e:retry:0").is_none());
        drop(first);
        assert!(reg.claim("e:retry:0").is_some());
    }

    #[test]
    fn prefix_ops_cover_only_matching_keys() {
        let reg = new_cancel_registry();
        let a = reg.register("e:retry:0");
        let b = reg.register("e:retry:1");
        let other = reg.register("f:retry:0");
        assert!(reg.is_live_prefix("e:retry:"));
        assert_eq!(reg.signal_prefix("e:retry:"), 2);
        assert!(a.is_cancelled() && b.is_cancelled());
        assert!(!other.is_cancelled());
        drop((a, b));
        assert!(!reg.is_live_prefix("e:retry:"));
    }

    #[test]
    fn a_poisoned_registry_still_signals() {
        let reg = new_cancel_registry();
        let guard = reg.register("run");
        let r2 = Arc::clone(&reg);
        let _ = std::thread::spawn(move || {
            let _held = r2.lock().unwrap();
            panic!("poison the registry");
        })
        .join();
        assert!(reg.lock().is_err(), "the lock is poisoned");
        assert!(reg.signal("run"));
        assert!(guard.is_cancelled());
        drop(guard);
        assert!(!reg.is_live("run"));
    }
}
