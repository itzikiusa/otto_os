//! A per-key async lock registry: one `tokio::sync::Mutex<()>` per key (a repo,
//! a swarm, a branch, a story…), created on first use and dropped once nobody
//! holds or waits on it.
//!
//! This replaces the hand-rolled `HashMap<String, Arc<tokio::sync::Mutex<()>>>`
//! statics that had drifted into three pruning rules (weak entries, a
//! strong-count of 1, never — growing with every key ever seen) and two poison
//! policies (one of which panicked every later caller once a holder had
//! panicked). Here entries are `Weak` and dead ones are pruned whenever a new
//! key is inserted, and the inner map lock is poison-tolerant: it guards only
//! the map, never the protected state, so a panicked holder leaves nothing
//! half-written.
//!
//! `scripts/check.sh` refuses a new hand-rolled registry; reach for this.

use std::borrow::Borrow;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Per-key async locks. `const`-constructible, so a registry is a plain
/// `static LOCKS: KeyedLocks<String> = KeyedLocks::new();`.
pub struct KeyedLocks<K> {
    map: Mutex<BTreeMap<K, Weak<AsyncMutex<()>>>>,
}

impl<K: Ord + Clone> Default for KeyedLocks<K> {
    fn default() -> Self {
        Self::new()
    }
}

impl<K: Ord + Clone> KeyedLocks<K> {
    pub const fn new() -> Self {
        KeyedLocks {
            map: Mutex::new(BTreeMap::new()),
        }
    }

    /// The (shared) mutex of `key`. Every caller holding the returned `Arc`
    /// keeps the entry alive; once the last one drops it, the next insert
    /// prunes it. For call sites that lock a borrowed guard
    /// (`let m = locks.handle(k); let _g = m.lock().await;`).
    pub fn handle<Q>(&self, key: &Q) -> Arc<AsyncMutex<()>>
    where
        K: Borrow<Q>,
        Q: Ord + ToOwned<Owned = K> + ?Sized,
    {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(live) = map.get(key).and_then(Weak::upgrade) {
            return live;
        }
        map.retain(|_, w| w.strong_count() > 0);
        let fresh = Arc::new(AsyncMutex::new(()));
        map.insert(key.to_owned(), Arc::downgrade(&fresh));
        fresh
    }

    /// Wait for `key`'s lock. The owned guard can cross `.await`s and tasks.
    pub async fn lock<Q>(&self, key: &Q) -> OwnedMutexGuard<()>
    where
        K: Borrow<Q>,
        Q: Ord + ToOwned<Owned = K> + ?Sized,
    {
        self.handle(key).lock_owned().await
    }

    /// `key`'s lock if it is free right now, else `None`.
    pub fn try_lock<Q>(&self, key: &Q) -> Option<OwnedMutexGuard<()>>
    where
        K: Borrow<Q>,
        Q: Ord + ToOwned<Owned = K> + ?Sized,
    {
        self.handle(key).try_lock_owned().ok()
    }

    /// Live entries (tests: pruning).
    pub fn len(&self) -> usize {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.values().filter(|w| w.strong_count() > 0).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn same_key_serialises_and_other_keys_do_not() {
        static LOCKS: KeyedLocks<String> = KeyedLocks::new();
        let g = LOCKS.lock("a").await;
        assert!(
            LOCKS.try_lock("a").is_none(),
            "held key must not lock twice"
        );
        assert!(LOCKS.try_lock("b").is_some(), "another key is independent");
        drop(g);
        assert!(LOCKS.try_lock("a").is_some(), "released key locks again");
    }

    /// Entries never outlive their last holder: the registry does not grow
    /// with every key ever seen (the old never-pruned maps did).
    #[tokio::test]
    async fn released_keys_are_pruned_on_the_next_insert() {
        let locks: KeyedLocks<String> = KeyedLocks::new();
        for i in 0..100 {
            let _g = locks.lock(&format!("k{i}")).await;
        }
        let held = locks.lock("held").await;
        assert_eq!(locks.len(), 1);
        assert_eq!(locks.map.lock().unwrap().len(), 1, "dead entries pruned");
        drop(held);
        assert!(locks.is_empty());
    }

    /// A waiter keeps the entry alive, so a second handle for the same key is
    /// the SAME mutex even while the first holder is mid-section.
    #[tokio::test]
    async fn a_waiter_shares_the_holders_mutex() {
        let locks: Arc<KeyedLocks<String>> = Arc::new(KeyedLocks::new());
        let g = locks.lock("k").await;
        let l2 = locks.clone();
        let waiter = tokio::spawn(async move { l2.lock("k").await });
        tokio::task::yield_now().await;
        assert!(!waiter.is_finished());
        drop(g);
        let _g2 = waiter.await.unwrap();
        assert!(locks.try_lock("k").is_none());
    }

    /// A panic while the map lock is held does not poison every later caller.
    #[test]
    fn poisoned_map_lock_is_tolerated() {
        let locks: Arc<KeyedLocks<String>> = Arc::new(KeyedLocks::new());
        let l2 = locks.clone();
        let _ = std::thread::spawn(move || {
            let _m = l2.map.lock().unwrap();
            panic!("poison");
        })
        .join();
        assert!(locks.map.is_poisoned());
        assert!(locks.try_lock("k").is_some());
    }
}
