//! otto-keychain — `SecretStore` implementations.
//!
//! - [`KeychainStore`]: macOS Keychain via the `keyring` crate (service
//!   `"com.otto.daemon"`). The default for normal operation.
//! - [`FileStore`]: a 0600-permission JSON file under the data dir, selected
//!   with `OTTO_SECRETS=file` (dev/CI fallback, secrets stored in plaintext).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use otto_core::secrets::SecretStore;
use otto_core::{Error, Result};

/// Keychain service name under which all Otto secrets are stored.
pub const SERVICE_NAME: &str = "com.otto.daemon";

/// Pick the secret store implementation from the environment:
/// `OTTO_SECRETS=file` → [`FileStore`] under `data_dir`, anything else →
/// [`KeychainStore`].
pub fn from_env(data_dir: &Path) -> Arc<dyn SecretStore> {
    if std::env::var("OTTO_SECRETS").as_deref() == Ok("file") {
        tracing::info!("secret store: file ({}/secrets.json)", data_dir.display());
        Arc::new(FileStore::new(data_dir))
    } else {
        tracing::info!("secret store: macOS Keychain (service {SERVICE_NAME})");
        Arc::new(KeychainStore::new())
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
            path: dir.join("secrets.json"),
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> Result<BTreeMap<String, String>> {
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
