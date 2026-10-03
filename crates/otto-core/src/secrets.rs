//! Secret storage abstraction. Implemented by otto-keychain (macOS Keychain,
//! file fallback); consumed by connections, git, and the server.

use std::sync::Arc;

use crate::Result;

/// Simple synchronous secret store keyed by item name.
///
/// Keychain reads are a Security.framework IPC round trip (1–10 ms, seconds
/// when the keychain is locked or prompting), so the daemon wraps its store in
/// `otto_keychain::CachingSecretStore`. Hot async callers should prefer
/// [`get_async`], which answers cache hits inline and runs misses on a blocking
/// thread instead of stalling a tokio worker.
pub trait SecretStore: Send + Sync {
    fn put(&self, key: &str, value: &str) -> Result<()>;
    fn get(&self, key: &str) -> Result<Option<String>>;
    fn delete(&self, key: &str) -> Result<()>;

    /// Answer `key` from an in-memory cache without blocking, if this store has
    /// one: `Some(value)` is a fresh cached answer (including a cached
    /// "absent"), `None` means "not cached — call [`SecretStore::get`]".
    /// Plain stores have no cache.
    fn get_cached(&self, _key: &str) -> Option<Option<String>> {
        None
    }
}

/// Async-friendly [`SecretStore::get`]: a cache hit returns inline; a miss runs
/// the (possibly blocking) backend read on tokio's blocking pool so a slow or
/// locked keychain never parks a runtime worker.
pub async fn get_async(store: &Arc<dyn SecretStore>, key: &str) -> Result<Option<String>> {
    if let Some(hit) = store.get_cached(key) {
        return Ok(hit);
    }
    let store = store.clone();
    let key = key.to_string();
    match tokio::task::spawn_blocking(move || store.get(&key)).await {
        Ok(r) => r,
        Err(e) => Err(crate::Error::Internal(format!("secret read task: {e}"))),
    }
}

/// Async-friendly [`SecretStore::put`]: the backend write runs on tokio's
/// blocking pool (a Keychain write can prompt, just like a read).
pub async fn put_async(store: &Arc<dyn SecretStore>, key: &str, value: &str) -> Result<()> {
    let (store, key, value) = (store.clone(), key.to_string(), value.to_string());
    match tokio::task::spawn_blocking(move || store.put(&key, &value)).await {
        Ok(r) => r,
        Err(e) => Err(crate::Error::Internal(format!("secret write task: {e}"))),
    }
}

/// Async-friendly [`SecretStore::delete`] (blocking pool, like [`put_async`]).
pub async fn delete_async(store: &Arc<dyn SecretStore>, key: &str) -> Result<()> {
    let (store, key) = (store.clone(), key.to_string());
    match tokio::task::spawn_blocking(move || store.delete(&key)).await {
        Ok(r) => r,
        Err(e) => Err(crate::Error::Internal(format!("secret delete task: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    /// Backend whose every call blocks the calling thread — stands in for a
    /// Keychain ACL dialog waiting on a person.
    #[derive(Default)]
    struct Slow(Mutex<HashMap<String, String>>);

    const STALL: Duration = Duration::from_millis(300);

    impl SecretStore for Slow {
        fn put(&self, key: &str, value: &str) -> Result<()> {
            std::thread::sleep(STALL);
            self.0.lock().unwrap().insert(key.into(), value.into());
            Ok(())
        }
        fn get(&self, key: &str) -> Result<Option<String>> {
            std::thread::sleep(STALL);
            Ok(self.0.lock().unwrap().get(key).cloned())
        }
        fn delete(&self, key: &str) -> Result<()> {
            std::thread::sleep(STALL);
            self.0.lock().unwrap().remove(key);
            Ok(())
        }
    }

    /// On a single-worker runtime a slow backend read/write must not stall
    /// other tasks: a 10 ms ticker keeps ticking while each call is in flight.
    #[test]
    fn slow_backend_never_parks_the_runtime_worker() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let store: Arc<dyn SecretStore> = Arc::new(Slow::default());
        rt.block_on(async {
            let ticks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let t = ticks.clone();
            let ticker = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    t.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
            });
            let started = Instant::now();
            put_async(&store, "k", "v").await.unwrap();
            assert_eq!(get_async(&store, "k").await.unwrap().as_deref(), Some("v"));
            delete_async(&store, "k").await.unwrap();
            assert_eq!(get_async(&store, "k").await.unwrap(), None);
            let elapsed = started.elapsed();
            ticker.abort();
            let n = ticks.load(std::sync::atomic::Ordering::SeqCst);
            // 4 calls × 300 ms ≈ 1.2 s; a blocked worker would tick ~0 times.
            assert!(elapsed >= STALL * 4, "calls really were slow: {elapsed:?}");
            assert!(
                n >= 20,
                "runtime worker was parked: only {n} ticks in {elapsed:?}"
            );
        });
    }
}
