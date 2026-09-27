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
