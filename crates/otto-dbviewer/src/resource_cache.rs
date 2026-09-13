//! Per-key initialization. No registry lock is held while waiting on a remote.
use otto_core::{Error, Result};
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
};
use tokio::sync::{watch, Mutex as AsyncMutex};

/// Request lifecycle carried through resolution and driver acquisition. It is
/// deliberately absent from serialized configuration and cache fingerprints.
#[derive(Clone, Debug)]
pub struct Lifecycle(Arc<watch::Sender<bool>>);
impl Default for Lifecycle {
    fn default() -> Self {
        Self(Arc::new(watch::channel(false).0))
    }
}
impl Lifecycle {
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
    pub(crate) fn retire(&self) {
        self.0.send_replace(true);
    }
    pub(crate) fn check(&self) -> Result<()> {
        if *self.0.borrow() {
            Err(closed())
        } else {
            Ok(())
        }
    }
    pub(crate) async fn cancelled(&self) {
        let mut rx = self.0.subscribe();
        loop {
            if *rx.borrow_and_update() {
                return;
            }
            if rx.changed().await.is_err() {
                return;
            }
        }
    }
    pub(crate) async fn run<T>(&self, work: impl Future<Output = Result<T>>) -> Result<T> {
        self.check()?;
        tokio::select! { biased; _ = self.cancelled() => Err(closed()), result = work => { self.check()?; result } }
    }
}
fn closed() -> Error {
    Error::Conflict("Connection closed; retry to reconnect".into())
}
async fn cancelled(token: Option<&Lifecycle>) {
    match token {
        Some(token) => token.cancelled().await,
        None => std::future::pending().await,
    }
}

struct Slot<V> {
    initializer: AsyncMutex<()>,
    value: Mutex<Option<V>>,
    lifecycle: Lifecycle,
}
impl<V> Default for Slot<V> {
    fn default() -> Self {
        Self {
            initializer: AsyncMutex::new(()),
            value: Mutex::new(None),
            lifecycle: Lifecycle::default(),
        }
    }
}
pub(crate) struct ResourceCache<V> {
    slots: Mutex<HashMap<String, Arc<Slot<V>>>>,
}
impl<V> Default for ResourceCache<V> {
    fn default() -> Self {
        Self {
            slots: Mutex::new(HashMap::new()),
        }
    }
}
struct Lease<'a, V> {
    cache: &'a ResourceCache<V>,
    key: String,
    slot: Arc<Slot<V>>,
}
impl<V> Drop for Lease<'_, V> {
    fn drop(&mut self) {
        let mut slots = self.cache.slots.lock().unwrap_or_else(|e| e.into_inner());
        if Arc::strong_count(&self.slot) == 2
            && self
                .slot
                .value
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_none()
            && slots
                .get(&self.key)
                .is_some_and(|s| Arc::ptr_eq(s, &self.slot))
        {
            slots.remove(&self.key);
        }
    }
}
impl<V: Clone> ResourceCache<V> {
    pub(crate) async fn get_or_try_init(
        &self,
        key: String,
        token: Option<&Lifecycle>,
        usable: impl Fn(&V) -> bool,
        initializer: impl Future<Output = Result<V>>,
    ) -> Result<V> {
        if let Some(token) = token {
            token.check()?;
        }
        let lease = {
            let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
            Lease {
                cache: self,
                slot: slots.entry(key.clone()).or_default().clone(),
                key,
            }
        };
        let _guard = tokio::select! { biased;
            _ = cancelled(token) => return Err(closed()),
            _ = lease.slot.lifecycle.cancelled() => return Err(closed()),
            guard = lease.slot.initializer.lock() => guard,
        };
        lease.slot.lifecycle.check()?;
        if let Some(token) = token {
            token.check()?;
        }
        {
            let mut value = lease.slot.value.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(ready) = value.as_ref().filter(|v| usable(v)) {
                return Ok(ready.clone());
            }
            *value = None;
        }
        let value = tokio::select! { biased;
            _ = cancelled(token) => return Err(closed()),
            _ = lease.slot.lifecycle.cancelled() => return Err(closed()),
            value = initializer => value?,
        };
        let mut ready = lease.slot.value.lock().unwrap_or_else(|e| e.into_inner());
        lease.slot.lifecycle.check()?;
        if let Some(token) = token {
            token.check()?;
        }
        *ready = Some(value.clone());
        Ok(value)
    }
    pub(crate) fn insert_ready(&self, key: String, value: V) {
        let slot = Slot {
            value: Mutex::new(Some(value)),
            ..Slot::default()
        };
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(old) = slots.insert(key, Arc::new(slot)) {
            old.lifecycle.retire();
        }
    }
    /// Cleanup only: this cannot start initialization or recreate a slot.
    pub(crate) fn get_ready(&self, key: &str) -> Option<V> {
        let slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        slots
            .get(key)
            .and_then(|slot| slot.value.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }
    pub(crate) fn remove(&self, key: &str) -> Option<V> {
        self.remove_where(|candidate| candidate == key)
            .into_iter()
            .next()
    }
    pub(crate) fn remove_where(&self, predicate: impl Fn(&str) -> bool) -> Vec<V> {
        self.take_where(predicate)
            .into_iter()
            .map(|(_, value)| value)
            .collect()
    }
    pub(crate) fn take_where(&self, predicate: impl Fn(&str) -> bool) -> Vec<(String, V)> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let keys: Vec<_> = slots.keys().filter(|k| predicate(k)).cloned().collect();
        Self::detach_keys(&mut slots, keys)
    }
    pub(crate) fn remove_ready_where(&self, predicate: impl Fn(&str, &V) -> bool) -> Vec<V> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let keys = slots
            .iter()
            .filter_map(|(key, slot)| {
                let value = slot.value.lock().unwrap_or_else(|e| e.into_inner());
                value
                    .as_ref()
                    .filter(|v| predicate(key, v))
                    .map(|_| key.clone())
            })
            .collect();
        Self::detach_keys(&mut slots, keys)
            .into_iter()
            .map(|(_, value)| value)
            .collect()
    }
    fn detach_keys(
        slots: &mut HashMap<String, Arc<Slot<V>>>,
        keys: Vec<String>,
    ) -> Vec<(String, V)> {
        let mut values = Vec::new();
        for key in keys {
            if let Some(slot) = slots.remove(&key) {
                slot.lifecycle.retire();
                if let Some(value) = slot.value.lock().unwrap_or_else(|e| e.into_inner()).take() {
                    values.push((key, value));
                }
            }
        }
        values
    }
    #[cfg(test)]
    fn len(&self) -> usize {
        self.slots.lock().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::oneshot;

    #[tokio::test]
    async fn resource_cache_warm_key_is_independent_of_blocked_initialization() {
        let cache = Arc::new(ResourceCache::default());
        assert_eq!(
            cache
                .get_or_try_init("b".into(), None, |_| true, async { Ok(2) })
                .await
                .unwrap(),
            2
        );
        let (started, started_rx) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let c = cache.clone();
        let blocked = tokio::spawn(async move {
            c.get_or_try_init("a".into(), None, |_| true, async {
                let _ = started.send(());
                let _ = released.await;
                Ok(1)
            })
            .await
        });
        started_rx.await.unwrap();
        assert_eq!(
            cache
                .get_or_try_init("b".into(), None, |_| true, async {
                    panic!("warm key initialized twice")
                })
                .await
                .unwrap(),
            2
        );
        assert!(!blocked.is_finished());
        release.send(()).unwrap();
        assert_eq!(blocked.await.unwrap().unwrap(), 1);
    }

    #[tokio::test]
    async fn resource_cache_same_key_single_flight() {
        let cache = Arc::new(ResourceCache::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();
        for _ in 0..10 {
            let c = cache.clone();
            let n = calls.clone();
            tasks.push(tokio::spawn(async move {
                c.get_or_try_init("a".into(), None, |_| true, async {
                    n.fetch_add(1, Ordering::SeqCst);
                    tokio::task::yield_now().await;
                    Ok(7)
                })
                .await
                .unwrap()
            }));
        }
        for task in tasks {
            assert_eq!(task.await.unwrap(), 7);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn resource_cache_abort_initializer_allows_retry() {
        let cache = Arc::new(ResourceCache::<usize>::default());
        let (started, started_rx) = oneshot::channel();
        let c = cache.clone();
        let task = tokio::spawn(async move {
            c.get_or_try_init("a".into(), None, |_| true, async {
                let _ = started.send(());
                std::future::pending().await
            })
            .await
        });
        started_rx.await.unwrap();
        task.abort();
        let _ = task.await;
        assert_eq!(
            cache
                .get_or_try_init("a".into(), None, |_| true, async { Ok(9) })
                .await
                .unwrap(),
            9
        );
    }

    #[tokio::test]
    async fn resource_cache_retirement_stops_late_publication() {
        let cache = Arc::new(ResourceCache::<usize>::default());
        let (started, started_rx) = oneshot::channel();
        let c = cache.clone();
        let task = tokio::spawn(async move {
            c.get_or_try_init("a".into(), None, |_| true, async {
                let _ = started.send(());
                std::future::pending().await
            })
            .await
        });
        started_rx.await.unwrap();
        assert!(cache.remove("a").is_none());
        assert!(task.await.unwrap().is_err());
        assert!(cache.get_ready("a").is_none());
        assert_eq!(
            cache
                .get_or_try_init("a".into(), None, |_| true, async { Ok(3) })
                .await
                .unwrap(),
            3
        );
    }

    #[tokio::test]
    async fn resource_cache_lifecycle_blocks_stale_acquisition_without_initializing() {
        let cache = ResourceCache::<usize>::default();
        let lifecycle = Lifecycle::default();
        lifecycle.retire();
        assert!(cache
            .get_or_try_init("a".into(), Some(&lifecycle), |_| true, async {
                panic!("retired caller initialized")
            })
            .await
            .is_err());
        assert!(cache.get_ready("a").is_none());
    }

    #[tokio::test]
    async fn resource_cache_failed_keys_are_not_retained() {
        let cache = ResourceCache::<usize>::default();
        for n in 0..100 {
            assert!(cache
                .get_or_try_init(n.to_string(), None, |_| true, async {
                    Err(otto_core::Error::Internal("fixture".into()))
                })
                .await
                .is_err());
        }
        assert_eq!(cache.len(), 0);
    }

    #[tokio::test]
    async fn resource_cache_dead_resource_replaced_and_retired_lease_survives() {
        struct Resource {
            alive: std::sync::atomic::AtomicBool,
            drops: Arc<AtomicUsize>,
        }
        impl Drop for Resource {
            fn drop(&mut self) {
                self.drops.fetch_add(1, Ordering::SeqCst);
            }
        }
        let cache = ResourceCache::default();
        let drops = Arc::new(AtomicUsize::new(0));
        let first = cache
            .get_or_try_init(
                "conn\0fingerprint-old".into(),
                None,
                |v: &Arc<Resource>| v.alive.load(Ordering::SeqCst),
                async {
                    Ok(Arc::new(Resource {
                        alive: std::sync::atomic::AtomicBool::new(true),
                        drops: drops.clone(),
                    }))
                },
            )
            .await
            .unwrap();
        first.alive.store(false, Ordering::SeqCst);
        let replacement = cache
            .get_or_try_init(
                "conn\0fingerprint-old".into(),
                None,
                |v| v.alive.load(Ordering::SeqCst),
                async {
                    Ok(Arc::new(Resource {
                        alive: std::sync::atomic::AtomicBool::new(true),
                        drops: drops.clone(),
                    }))
                },
            )
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &replacement));
        // Removing cache ownership (TTL or changed fingerprint) must preserve
        // an operation's independently held lease until that operation ends.
        assert_eq!(
            cache
                .remove_ready_where(|key, _| key.starts_with("conn\0"))
                .len(),
            1
        );
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        assert!(replacement.alive.load(Ordering::SeqCst));
        drop(first);
        drop(replacement);
        assert_eq!(drops.load(Ordering::SeqCst), 2);
        assert_eq!(cache.len(), 0);
    }

    #[tokio::test]
    async fn resource_cache_ready_cleanup_does_not_initialize_and_predicate_close_is_scoped() {
        let cache = ResourceCache::default();
        for key in ["a|db=0", "a|db=1", "b|db=0"] {
            cache
                .get_or_try_init(key.into(), None, |_| true, async { Ok(1) })
                .await
                .unwrap();
        }
        assert!(cache.get_ready("missing").is_none());
        assert_eq!(cache.remove_where(|key| key.starts_with("a|")).len(), 2);
        assert_eq!(cache.get_ready("b|db=0"), Some(1));
    }
}
