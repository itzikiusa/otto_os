//! Encrypted secrets file: ONE random 256-bit master key in the macOS Keychain
//! (a single generic-password item), every secret in `<data_dir>/secrets.enc`
//! sealed with it (AES-256-GCM via `ring`, authenticated — a flipped byte or a
//! wrong key fails to open instead of yielding garbage).
//!
//! Why not one Keychain item per secret: an ad-hoc re-signed `ottod` fails the
//! item ACL match on every rebuild, so per-item storage re-prompts once PER
//! SECRET. With a master key it is at most one prompt per rebuild, and the
//! values are never on disk in plaintext.
//!
//! File format: `MAGIC (8) ‖ nonce (12) ‖ ciphertext ‖ tag (16)`; the AAD is
//! `MAGIC`, the plaintext a JSON object `{key: value}`. Writes are atomic
//! (temp sibling, `fsync`, `rename`, directory `fsync`) and 0600.
//!
//! Keychain access never blocks a caller indefinitely: [`MasterKeyCell`] runs
//! the Keychain call on its own thread and waits at most a bounded time; a
//! dialog still waiting for the user surfaces as a "locked" error (and as
//! `key_state: "locked"` in the status), not a parked thread.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use otto_core::secrets::SecretStore;
use otto_core::{Error, Result};
use ring::aead::{Aad, LessSafeKey, Nonce, UnboundKey, AES_256_GCM, NONCE_LEN};
use ring::rand::{SecureRandom, SystemRandom};

/// File name of the encrypted store under the data dir.
pub const ENCRYPTED_FILE: &str = "secrets.enc";
/// Keychain account (under [`crate::SERVICE_NAME`]) holding the master key.
pub const MASTER_KEY_ACCOUNT: &str = "otto-secrets-master-key-v1";
/// Leading bytes of every encrypted file (also the AEAD associated data).
const MAGIC: &[u8; 8] = b"OTTOSEC1";
const TAG_LEN: usize = 16;
/// How long a caller waits for the Keychain before getting "locked".
pub const DEFAULT_KEY_TIMEOUT: Duration = Duration::from_secs(8);
/// After a failed Keychain read (denied, error) don't re-ask more often than this.
const KEY_RETRY_AFTER: Duration = Duration::from_secs(30);

/// 256-bit master key. Zeroed on drop (best effort — copies made by the
/// allocator or the Keychain framework are out of our reach).
pub struct MasterKey([u8; 32]);

impl MasterKey {
    pub fn from_bytes(b: [u8; 32]) -> Self {
        Self(b)
    }

    /// A fresh key from the OS CSPRNG.
    pub fn generate() -> Result<Self> {
        let mut b = [0u8; 32];
        SystemRandom::new()
            .fill(&mut b)
            .map_err(|_| Error::Internal("secrets: no system randomness".into()))?;
        Ok(Self(b))
    }

    fn aead(&self) -> Result<LessSafeKey> {
        let k = UnboundKey::new(&AES_256_GCM, &self.0)
            .map_err(|_| Error::Internal("secrets: bad master key".into()))?;
        Ok(LessSafeKey::new(k))
    }

    pub(crate) fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    fn from_hex(s: &str) -> Result<Self> {
        let v = hex::decode(s.trim())
            .map_err(|_| Error::Internal("secrets: master key item is malformed".into()))?;
        let b: [u8; 32] = v
            .try_into()
            .map_err(|_| Error::Internal("secrets: master key item has the wrong length".into()))?;
        Ok(Self(b))
    }
}

/// Redacted: a key must never reach a log line.
impl std::fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MasterKey(<redacted>)")
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        for b in self.0.iter_mut() {
            // SAFETY: `b` is a valid, aligned &mut u8; volatile so the wipe
            // isn't optimised away as a dead store.
            unsafe { std::ptr::write_volatile(b, 0) };
        }
    }
}

/// Where the master key lives. Production: [`KeychainMasterKey`]; tests use an
/// in-memory source.
pub trait MasterKeySource: Send + Sync {
    /// The stored key, or `None` when none was ever created.
    fn load(&self) -> Result<Option<MasterKey>>;
    /// Store `key` as the master key (first use only).
    fn store(&self, key: &MasterKey) -> Result<()>;
}

/// The single Keychain item holding the master key.
#[derive(Debug, Default, Clone)]
pub struct KeychainMasterKey;

impl KeychainMasterKey {
    fn entry() -> Result<keyring::Entry> {
        keyring::Entry::new(crate::SERVICE_NAME, MASTER_KEY_ACCOUNT)
            .map_err(|e| Error::Internal(format!("keychain master-key entry: {e}")))
    }
}

impl MasterKeySource for KeychainMasterKey {
    fn load(&self) -> Result<Option<MasterKey>> {
        match Self::entry()?.get_password() {
            Ok(v) => MasterKey::from_hex(&v).map(Some),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(Error::Internal(format!("keychain master-key read: {e}"))),
        }
    }

    fn store(&self, key: &MasterKey) -> Result<()> {
        Self::entry()?
            .set_password(&key.to_hex())
            .map_err(|e| Error::Internal(format!("keychain master-key write: {e}")))
    }
}

/// Why the master key isn't available right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyError {
    /// The Keychain didn't answer in time (locked, or a dialog is waiting for
    /// the user). Retrying later may succeed; nothing was changed.
    Locked,
    /// No master key exists yet (and the caller didn't ask to create one).
    Missing,
    /// The Keychain refused or failed (message has no secret material).
    Failed(String),
}

impl From<KeyError> for Error {
    fn from(e: KeyError) -> Self {
        match e {
            KeyError::Locked => Error::Upstream(
                "secret store locked: the macOS Keychain did not answer (locked, or an \
                 access prompt is waiting) — unlock it / approve the prompt and retry"
                    .into(),
            ),
            KeyError::Missing => Error::Internal(
                "secret store: secrets.enc exists but its master key is missing from the Keychain"
                    .into(),
            ),
            KeyError::Failed(m) => Error::Internal(format!("secret store: {m}")),
        }
    }
}

enum KeyState {
    Idle,
    Loading,
    Ready(Arc<MasterKey>),
    Missing,
    Failed(String, Instant),
}

struct CellInner {
    state: Mutex<KeyState>,
    cv: Condvar,
}

/// Lazily-loaded master key with a bounded wait. The Keychain call runs on a
/// dedicated thread; callers wait at most `timeout` for it and otherwise get
/// [`KeyError::Locked`] while the call keeps going in the background (one in
/// flight at a time — a later caller joins it instead of stacking prompts).
pub struct MasterKeyCell {
    source: Arc<dyn MasterKeySource>,
    timeout: Duration,
    inner: Arc<CellInner>,
}

impl MasterKeyCell {
    pub fn new(source: Arc<dyn MasterKeySource>, timeout: Duration) -> Self {
        Self {
            source,
            timeout,
            inner: Arc::new(CellInner {
                state: Mutex::new(KeyState::Idle),
                cv: Condvar::new(),
            }),
        }
    }

    /// `"unlocked"` (key in memory), `"locked"` (a Keychain call is waiting),
    /// `"not_loaded"` (never asked / no key yet) or `"error"`.
    pub fn state_label(&self) -> &'static str {
        match &*self.inner.state.lock().unwrap_or_else(|p| p.into_inner()) {
            KeyState::Ready(_) => "unlocked",
            KeyState::Loading => "locked",
            KeyState::Idle | KeyState::Missing => "not_loaded",
            KeyState::Failed(..) => "error",
        }
    }

    /// The master key. With `create`, a missing key is generated and stored
    /// (first secret ever written); without it, a missing key is
    /// [`KeyError::Missing`].
    pub fn fetch(&self, create: bool) -> std::result::Result<Arc<MasterKey>, KeyError> {
        let deadline = Instant::now() + self.timeout;
        let mut st = self.inner.state.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            match &*st {
                KeyState::Ready(k) => return Ok(k.clone()),
                KeyState::Missing if !create => return Err(KeyError::Missing),
                KeyState::Failed(m, at) if at.elapsed() < KEY_RETRY_AFTER => {
                    return Err(KeyError::Failed(m.clone()))
                }
                KeyState::Loading => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(KeyError::Locked);
                    }
                    let (g, _) = self
                        .inner
                        .cv
                        .wait_timeout(st, deadline - now)
                        .unwrap_or_else(|p| p.into_inner());
                    st = g;
                }
                // Idle, Missing+create, or a Failed past its back-off.
                _ => {
                    *st = KeyState::Loading;
                    let source = self.source.clone();
                    let inner = self.inner.clone();
                    let spawned = std::thread::Builder::new()
                        .name("otto-keychain-master".into())
                        .spawn(move || {
                            let next = match load_or_create(&*source, create) {
                                Ok(Some(k)) => KeyState::Ready(Arc::new(k)),
                                Ok(None) => KeyState::Missing,
                                Err(e) => KeyState::Failed(e.to_string(), Instant::now()),
                            };
                            *inner.state.lock().unwrap_or_else(|p| p.into_inner()) = next;
                            inner.cv.notify_all();
                        });
                    if let Err(e) = spawned {
                        *st = KeyState::Idle;
                        return Err(KeyError::Failed(format!("keychain thread: {e}")));
                    }
                }
            }
        }
    }
}

fn load_or_create(src: &dyn MasterKeySource, create: bool) -> Result<Option<MasterKey>> {
    if let Some(k) = src.load()? {
        return Ok(Some(k));
    }
    if !create {
        return Ok(None);
    }
    let k = MasterKey::generate()?;
    src.store(&k)?;
    // Read it back: a store that "succeeded" but can't be read would make
    // every later secret unrecoverable.
    match src.load()? {
        Some(back) if back.0 == k.0 => Ok(Some(k)),
        _ => Err(Error::Internal(
            "keychain master key did not read back after creation".into(),
        )),
    }
}

// ---------------------------------------------------------------------------
// Sealing
// ---------------------------------------------------------------------------

/// Encrypt a secrets map into the on-disk format.
pub fn seal(key: &MasterKey, map: &BTreeMap<String, String>) -> Result<Vec<u8>> {
    let mut plain =
        serde_json::to_vec(map).map_err(|e| Error::Internal(format!("secrets serialize: {e}")))?;
    let mut nonce = [0u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| Error::Internal("secrets: no system randomness".into()))?;
    let r = key.aead()?.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(MAGIC),
        &mut plain,
    );
    if r.is_err() {
        wipe(&mut plain);
        return Err(Error::Internal("secrets: encryption failed".into()));
    }
    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + plain.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&plain);
    Ok(out)
}

/// Decrypt + authenticate the on-disk format. Any tampering (or a different
/// key) is an error — never a partially-decoded map.
pub fn open(key: &MasterKey, bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    if bytes.len() < MAGIC.len() + NONCE_LEN + TAG_LEN || &bytes[..MAGIC.len()] != MAGIC {
        return Err(Error::Internal(
            "secrets.enc is not an Otto encrypted secrets file".into(),
        ));
    }
    let mut nonce = [0u8; NONCE_LEN];
    nonce.copy_from_slice(&bytes[MAGIC.len()..MAGIC.len() + NONCE_LEN]);
    let mut buf = bytes[MAGIC.len() + NONCE_LEN..].to_vec();
    let parsed = match key.aead()?.open_in_place(
        Nonce::assume_unique_for_key(nonce),
        Aad::from(MAGIC),
        &mut buf,
    ) {
        Ok(plain) => serde_json::from_slice::<BTreeMap<String, String>>(plain)
            .map_err(|e| Error::Internal(format!("secrets.enc payload: {e}"))),
        Err(_) => Err(Error::Internal(
            "secrets.enc failed authentication (tampered, truncated or wrong key)".into(),
        )),
    };
    wipe(&mut buf);
    parsed
}

fn wipe(buf: &mut [u8]) {
    for b in buf.iter_mut() {
        // SAFETY: valid &mut u8; volatile keeps the wipe from being elided.
        unsafe { std::ptr::write_volatile(b, 0) };
    }
}

/// Atomically replace `path` with `bytes` (0600): temp sibling, fsync,
/// rename, fsync the directory. A crash leaves either the old or the new file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let dir = path
        .parent()
        .ok_or_else(|| Error::Internal("secrets path has no parent".into()))?;
    std::fs::create_dir_all(dir).map_err(|e| Error::Internal(format!("secrets dir: {e}")))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "secrets".into());
    let tmp = dir.join(format!(".{name}.tmp.{}", std::process::id()));
    let res = (|| -> std::io::Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)?;
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
        Ok(())
    })();
    if let Err(e) = res {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::Internal(format!("secrets write {name}: {e}")));
    }
    Ok(())
}

/// Read + decrypt an encrypted file; `Ok(None)` when it doesn't exist.
pub(crate) fn read_sealed(
    path: &Path,
    key: &MasterKey,
) -> Result<Option<BTreeMap<String, String>>> {
    match std::fs::read(path) {
        Ok(b) => open(key, &b).map(Some),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Error::Internal(format!("secrets.enc read: {e}"))),
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// [`SecretStore`] over `secrets.enc`. Reads when the file doesn't exist yet
/// answer "absent" without touching the Keychain, so a fresh install with no
/// secrets never prompts; the master key is created on the first `put`.
pub struct EncryptedFileStore {
    path: PathBuf,
    key: Arc<MasterKeyCell>,
    lock: Mutex<()>,
}

impl EncryptedFileStore {
    pub fn new(dir: &Path, key: Arc<MasterKeyCell>) -> Self {
        Self {
            path: dir.join(ENCRYPTED_FILE),
            key,
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self, key: &MasterKey) -> Result<BTreeMap<String, String>> {
        Ok(read_sealed(&self.path, key)?.unwrap_or_default())
    }
}

impl SecretStore for EncryptedFileStore {
    // The master key is fetched BEFORE the file lock (perf2/03 N2): the fetch
    // can wait up to the Keychain timeout, and under the lock N concurrent
    // callers would each wait their own full timeout in turn. Outside it they
    // all share the one in-flight `MasterKeyCell` load and its deadline.
    fn put(&self, key: &str, value: &str) -> Result<()> {
        let mk = self.key.fetch(true)?;
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut map = self.load(&mk)?;
        map.insert(key.to_string(), value.to_string());
        write_atomic(&self.path, &seal(&mk, &map)?)
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let mk = self.key.fetch(false)?;
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        Ok(self.load(&mk)?.remove(key))
    }

    fn delete(&self, key: &str) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let mk = self.key.fetch(false)?;
        let _g = self.lock.lock().unwrap_or_else(|p| p.into_inner());
        let mut map = self.load(&mk)?;
        if map.remove(key).is_some() {
            write_atomic(&self.path, &seal(&mk, &map)?)?;
        }
        Ok(())
    }
}
