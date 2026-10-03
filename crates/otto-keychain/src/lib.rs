//! otto-keychain — `SecretStore` implementations.
//!
//! - [`EncryptedFileStore`]: `secrets.enc` under the data dir, AES-256-GCM
//!   sealed with ONE master key kept in the macOS Keychain (see
//!   [`encrypted`]). `OTTO_SECRETS=encrypted`, or automatically once a
//!   plaintext store was migrated.
//! - [`KeychainStore`]: one Keychain item per secret (service
//!   `"com.otto.daemon"`); the historic default when `OTTO_SECRETS` is unset.
//! - [`FileStore`]: a 0600-permission JSON file under the data dir, selected
//!   with `OTTO_SECRETS=file` (dev/CI/Linux fallback, secrets stored in
//!   PLAINTEXT; release macOS builds need `OTTO_SECRETS_ALLOW_PLAINTEXT=1` or a
//!   pre-existing legacy file). Settings ▸ Security offers the explicit
//!   "Secure secrets…" migration ([`SecretsControl::migrate_to_encrypted`]).
//! - [`CachingSecretStore`]: a TTL read-through cache in front of either, so
//!   hot request paths (MCP invoke, Jira/Confluence, webhooks) don't pay a
//!   Keychain IPC round trip — or a file decrypt/re-parse — per call.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use otto_core::secrets::SecretStore;
use otto_core::{Error, Result};

pub mod control;
pub mod encrypted;

pub use control::{SecretsControl, SecretsMode, SecretsStatus};
pub use encrypted::{EncryptedFileStore, KeychainMasterKey, MasterKeyCell};

/// Keychain service name under which all Otto secrets are stored.
pub const SERVICE_NAME: &str = "com.otto.daemon";

/// File name of the plaintext (legacy / dev) store under the data dir.
pub const PLAINTEXT_FILE: &str = "secrets.json";

/// Pick the secret store from the environment and the files on disk (see
/// [`control::choose_mode`]), wrap it in a switchable [`SecretsControl`]
/// (registered process-wide for the admin status/migration routes) and front
/// it with a [`CachingSecretStore`] with [`DEFAULT_CACHE_TTL`].
///
/// Never touches the Keychain itself: the master key is loaded lazily, with a
/// bounded wait, on the first encrypted read of an existing `secrets.enc`.
pub fn from_env(data_dir: &Path) -> Arc<dyn SecretStore> {
    let env = std::env::var("OTTO_SECRETS").ok();
    let allow_plaintext = std::env::var("OTTO_SECRETS_ALLOW_PLAINTEXT").as_deref() == Ok("1");
    let strict = cfg!(all(not(debug_assertions), target_os = "macos"));
    // Finish an interrupted plaintext wipe before deciding the mode (a zeroed
    // secrets.json next to secrets.enc must not select plaintext).
    control::sweep_plaintext_residue(data_dir);
    let plaintext_exists = data_dir.join(PLAINTEXT_FILE).exists();
    let mode = control::choose_mode(control::ModeInputs {
        env: env.as_deref(),
        allow_plaintext,
        strict,
        plaintext_exists,
        encrypted_exists: data_dir.join(encrypted::ENCRYPTED_FILE).exists(),
    });
    match mode {
        SecretsMode::Plaintext => {
            tracing::warn!(
                "secret store: PLAINTEXT file ({}/{PLAINTEXT_FILE}) — use Settings ▸ Security ▸ \
                 Secure secrets… to encrypt it",
                data_dir.display()
            );
            if strict && !allow_plaintext {
                tracing::warn!(
                    "secret store: plaintext kept only because a legacy {PLAINTEXT_FILE} exists \
                     (release builds otherwise require OTTO_SECRETS_ALLOW_PLAINTEXT=1)"
                );
            }
        }
        SecretsMode::Encrypted => {
            if env.as_deref() == Some("file") {
                tracing::info!(
                    "secret store: OTTO_SECRETS=file ignored — using the encrypted store \
                     ({}/{})",
                    data_dir.display(),
                    encrypted::ENCRYPTED_FILE
                );
            } else {
                tracing::info!(
                    "secret store: encrypted file ({}/{}, master key in the Keychain)",
                    data_dir.display(),
                    encrypted::ENCRYPTED_FILE
                );
            }
        }
        SecretsMode::Keychain => {
            tracing::info!("secret store: macOS Keychain (service {SERVICE_NAME})");
        }
    }
    let key = Arc::new(MasterKeyCell::new(
        Arc::new(KeychainMasterKey),
        encrypted::DEFAULT_KEY_TIMEOUT,
    ));
    let control = Arc::new(SecretsControl::new(data_dir, key, mode));
    control::register(control.clone());
    Arc::new(CachingSecretStore::new(control, DEFAULT_CACHE_TTL))
}

// ---------------------------------------------------------------------------
// Read-through cache
// ---------------------------------------------------------------------------

/// How long a cached read (value or "absent") is served before the backend is
/// asked again. Writes through this store evict immediately; the TTL only
/// bounds staleness for edits made OUTSIDE the daemon (Keychain Access, a
/// second process) — the same window the auth cache accepts.
pub const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(60);

/// Upper bound on cached keys — secrets are a few dozen items in practice; the
/// cap only keeps a pathological caller (unique key per request) bounded.
const CACHE_MAX_KEYS: usize = 1024;

/// TTL read-through cache over any [`SecretStore`]. `get` hits are served from
/// memory (including cached "absent"); `put`/`delete` write through and then
/// evict the key, so the next read always sees the new value. Backend errors
/// are never cached. Values stay in process memory only — the same place the
/// callers hold them anyway.
pub struct CachingSecretStore {
    inner: Arc<dyn SecretStore>,
    ttl: Duration,
    entries: Mutex<HashMap<String, (Instant, Option<String>)>>,
    /// Bumped by every put/delete. A read that raced a write (started before
    /// it, finished after) must not cache the value it fetched — it may be the
    /// pre-write one.
    generation: AtomicU64,
}

impl CachingSecretStore {
    pub fn new(inner: Arc<dyn SecretStore>, ttl: Duration) -> Self {
        Self {
            inner,
            ttl,
            entries: Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
        }
    }

    fn lookup(&self, key: &str) -> Option<Option<String>> {
        let map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        match map.get(key) {
            Some((at, v)) if at.elapsed() < self.ttl => Some(v.clone()),
            _ => None,
        }
    }

    fn remember(&self, key: &str, value: Option<String>, read_gen: u64) {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        // Checked under the map lock: `evict` bumps the generation under the
        // same lock, so a write can't slip in between this check and insert.
        if self.generation.load(Ordering::SeqCst) != read_gen {
            return;
        }
        if map.len() >= CACHE_MAX_KEYS && !map.contains_key(key) {
            let ttl = self.ttl;
            map.retain(|_, (at, _)| at.elapsed() < ttl);
            if map.len() >= CACHE_MAX_KEYS {
                map.clear();
            }
        }
        map.insert(key.to_string(), (Instant::now(), value));
    }

    fn evict(&self, key: &str) {
        let mut map = self.entries.lock().unwrap_or_else(|p| p.into_inner());
        self.generation.fetch_add(1, Ordering::SeqCst);
        map.remove(key);
    }
}

impl SecretStore for CachingSecretStore {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        // Evict (and bump the generation) on both sides of the write, so a
        // concurrent reader that raced the backend write can't leave the old
        // value cached afterwards.
        self.evict(key);
        let r = self.inner.put(key, value);
        self.evict(key);
        r
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        if let Some(hit) = self.lookup(key) {
            return Ok(hit);
        }
        let read_gen = self.generation.load(Ordering::SeqCst);
        let v = self.inner.get(key)?;
        self.remember(key, v.clone(), read_gen);
        Ok(v)
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.evict(key);
        let r = self.inner.delete(key);
        self.evict(key);
        r
    }

    fn get_cached(&self, key: &str) -> Option<Option<String>> {
        self.lookup(key)
    }
}

// ---------------------------------------------------------------------------
// Keychain
// ---------------------------------------------------------------------------

/// macOS Keychain-backed store. Each secret is one generic-password item with
/// service [`SERVICE_NAME`] and account = the secret key.
#[derive(Debug, Default, Clone)]
pub struct KeychainStore;

impl KeychainStore {
    pub fn new() -> Self {
        Self
    }

    fn entry(key: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE_NAME, key)
            .map_err(|e| Error::Internal(format!("keychain entry '{key}': {e}")))
    }
}

impl SecretStore for KeychainStore {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        Self::entry(key)?
            .set_password(value)
            .map_err(|e| Error::Internal(format!("keychain put '{key}': {e}")))
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        match Self::entry(key)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Internal(format!("keychain get '{key}': {e}"))),
        }
    }

    fn delete(&self, key: &str) -> Result<()> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(Error::Internal(format!("keychain delete '{key}': {e}"))),
        }
    }
}

// ---------------------------------------------------------------------------
// File fallback (dev / CI)
// ---------------------------------------------------------------------------

/// File-backed store: a single JSON object in `<dir>/secrets.json`, written
/// with 0600 permissions. Plaintext — only for headless dev/CI.
pub struct FileStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileStore {
    /// Store secrets under `dir/secrets.json`.
    pub fn new(dir: &Path) -> Self {
        Self {
            path: dir.join(PLAINTEXT_FILE),
            lock: Mutex::new(()),
        }
    }

    pub(crate) fn load(&self) -> Result<BTreeMap<String, String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => serde_json::from_str(&s)
                .map_err(|e| Error::Internal(format!("secrets file parse: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(e) => Err(Error::Internal(format!("secrets file read: {e}"))),
        }
    }

    fn save(&self, map: &BTreeMap<String, String>) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Internal(format!("secrets dir: {e}")))?;
        }
        let body = serde_json::to_string_pretty(map)
            .map_err(|e| Error::Internal(format!("secrets serialize: {e}")))?;

        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&self.path)
            .map_err(|e| Error::Internal(format!("secrets file open: {e}")))?;
        f.write_all(body.as_bytes())
            .map_err(|e| Error::Internal(format!("secrets file write: {e}")))?;
        // The file may pre-exist with looser permissions; enforce 0600 anyway.
        use std::os::unix::fs::PermissionsExt;
        let mut perms = f
            .metadata()
            .map_err(|e| Error::Internal(format!("secrets file meta: {e}")))?
            .permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&self.path, perms)
            .map_err(|e| Error::Internal(format!("secrets file chmod: {e}")))?;
        Ok(())
    }
}

impl SecretStore for FileStore {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        let _guard = self.lock.lock().expect("secrets lock poisoned");
        let mut map = self.load()?;
        map.insert(key.to_string(), value.to_string());
        self.save(&map)
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        let _guard = self.lock.lock().expect("secrets lock poisoned");
        Ok(self.load()?.get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<()> {
        let _guard = self.lock.lock().expect("secrets lock poisoned");
        let mut map = self.load()?;
        if map.remove(key).is_some() {
            self.save(&map)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// In-memory backend that counts reads so tests can see cache hits.
    #[derive(Default)]
    struct Counting {
        map: Mutex<BTreeMap<String, String>>,
        gets: AtomicUsize,
    }

    impl SecretStore for Counting {
        fn put(&self, key: &str, value: &str) -> Result<()> {
            self.map.lock().unwrap().insert(key.into(), value.into());
            Ok(())
        }
        fn get(&self, key: &str) -> Result<Option<String>> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            Ok(self.map.lock().unwrap().get(key).cloned())
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.map.lock().unwrap().remove(key);
            Ok(())
        }
    }

    fn store(ttl: Duration) -> (Arc<Counting>, CachingSecretStore) {
        let inner = Arc::new(Counting::default());
        let s = CachingSecretStore::new(inner.clone(), ttl);
        (inner, s)
    }

    #[test]
    fn cache_hit_skips_backend_including_absent() {
        let (inner, s) = store(Duration::from_secs(60));
        inner.put("a", "1").unwrap();
        assert_eq!(s.get("a").unwrap().as_deref(), Some("1"));
        assert_eq!(s.get("a").unwrap().as_deref(), Some("1"));
        assert_eq!(s.get("missing").unwrap(), None);
        assert_eq!(s.get("missing").unwrap(), None);
        assert_eq!(inner.gets.load(Ordering::SeqCst), 2);
        assert_eq!(s.get_cached("a"), Some(Some("1".into())));
        assert_eq!(s.get_cached("missing"), Some(None));
        assert_eq!(s.get_cached("never-read"), None);
    }

    #[test]
    fn put_and_delete_evict() {
        let (inner, s) = store(Duration::from_secs(60));
        assert_eq!(s.get("k").unwrap(), None); // cached "absent"
        s.put("k", "v1").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some("v1"));
        s.put("k", "v2").unwrap(); // rotation
        assert_eq!(s.get("k").unwrap().as_deref(), Some("v2"));
        s.delete("k").unwrap();
        assert_eq!(s.get("k").unwrap(), None);
        assert_eq!(inner.gets.load(Ordering::SeqCst), 4);
    }

    #[test]
    fn ttl_expiry_rereads_backend() {
        let (inner, s) = store(Duration::from_millis(0));
        inner.put("k", "v").unwrap();
        s.get("k").unwrap();
        s.get("k").unwrap();
        assert_eq!(inner.gets.load(Ordering::SeqCst), 2);
        assert_eq!(s.get_cached("k"), None);
    }

    #[test]
    fn get_async_uses_cache_then_blocking_pool() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let (inner, s) = store(Duration::from_secs(60));
        inner.put("k", "v").unwrap();
        let s: Arc<dyn SecretStore> = Arc::new(s);
        rt.block_on(async {
            let a = otto_core::secrets::get_async(&s, "k").await.unwrap();
            let b = otto_core::secrets::get_async(&s, "k").await.unwrap();
            assert_eq!(a.as_deref(), Some("v"));
            assert_eq!(b.as_deref(), Some("v"));
        });
        assert_eq!(inner.gets.load(Ordering::SeqCst), 1);
    }
}
