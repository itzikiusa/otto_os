//! Which secret backend is active, its status for Settings, and the explicit
//! plaintext → encrypted migration ("Secure secrets…").
//!
//! The migration is NEVER run automatically: the first master-key access can
//! pop a Keychain prompt, and an unattended deploy that blocks on a prompt
//! would leave every integration without its secrets. It runs only from the
//! confirmed, root-only, audited admin action (`POST /admin/secrets/secure`).
//!
//! [`SecretsControl`] is itself a [`SecretStore`] that delegates to the active
//! backend, so a successful migration switches the running daemon over
//! in-process — every `Arc<dyn SecretStore>` handed out earlier keeps working.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, RwLock};

use otto_core::secrets::SecretStore;
use otto_core::{Error, Result};

use crate::encrypted::{
    read_sealed, seal, write_atomic, EncryptedFileStore, MasterKeyCell, ENCRYPTED_FILE,
};
use crate::{FileStore, KeychainStore, PLAINTEXT_FILE};

/// Encrypted copy of the plaintext entries kept while a migration verifies.
pub const MIGRATION_BACKUP_FILE: &str = "secrets.migrate-backup.enc";

/// The active backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretsMode {
    /// `secrets.json`, plaintext (legacy installs / dev / CI / Linux).
    Plaintext,
    /// `secrets.enc` sealed with the Keychain master key.
    Encrypted,
    /// One Keychain item per secret (no `OTTO_SECRETS` set).
    Keychain,
}

/// `GET /admin/secrets/status` body. Counts and states only — never values.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SecretsStatus {
    pub mode: SecretsMode,
    /// `secrets.json` (plaintext) exists on disk.
    pub plaintext_file: bool,
    /// Entries in `secrets.json` (0 when absent or unreadable).
    pub plaintext_entries: usize,
    /// Master-key state: `unlocked` / `locked` / `not_loaded` / `error`.
    pub key_state: &'static str,
    /// "Secure secrets…" is offered (plaintext mode with a plaintext file).
    pub migration_available: bool,
    /// A migration is running right now.
    pub migrating: bool,
    /// An encrypted migration backup is still on disk (a migration stopped
    /// before its final verification).
    pub backup_present: bool,
}

/// Result of a successful migration. Key NAMES are not included either —
/// just counts.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MigrationReport {
    /// Entries moved out of `secrets.json`.
    pub migrated: usize,
    /// Entries in `secrets.enc` afterwards.
    pub total: usize,
    pub duration_ms: u64,
}

/// The active backend + the migration. See the module docs.
pub struct SecretsControl {
    data_dir: PathBuf,
    key: Arc<MasterKeyCell>,
    active: RwLock<(SecretsMode, Arc<dyn SecretStore>)>,
    migrating: AtomicBool,
}

impl SecretsControl {
    pub fn new(data_dir: &Path, key: Arc<MasterKeyCell>, mode: SecretsMode) -> Self {
        let store: Arc<dyn SecretStore> = match mode {
            SecretsMode::Plaintext => Arc::new(FileStore::new(data_dir)),
            SecretsMode::Encrypted => Arc::new(EncryptedFileStore::new(data_dir, key.clone())),
            SecretsMode::Keychain => Arc::new(KeychainStore::new()),
        };
        Self {
            data_dir: data_dir.to_path_buf(),
            key,
            active: RwLock::new((mode, store)),
            migrating: AtomicBool::new(false),
        }
    }

    pub fn mode(&self) -> SecretsMode {
        self.active.read().unwrap_or_else(|p| p.into_inner()).0
    }

    fn store(&self) -> Arc<dyn SecretStore> {
        self.active
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .1
            .clone()
    }

    /// Status for Settings. Reads `secrets.json` only to COUNT entries.
    pub fn status(&self) -> SecretsStatus {
        let mode = self.mode();
        let plain = self.data_dir.join(PLAINTEXT_FILE);
        let plaintext_file = plain.exists();
        let plaintext_entries = if plaintext_file {
            FileStore::new(&self.data_dir)
                .load()
                .map(|m| m.len())
                .unwrap_or(0)
        } else {
            0
        };
        SecretsStatus {
            mode,
            plaintext_file,
            plaintext_entries,
            key_state: self.key.state_label(),
            migration_available: mode == SecretsMode::Plaintext && plaintext_file,
            migrating: self.migrating.load(Ordering::SeqCst),
            backup_present: self.data_dir.join(MIGRATION_BACKUP_FILE).exists(),
        }
    }

    /// Move every `secrets.json` entry into `secrets.enc`, verify, then wipe
    /// and delete the plaintext file and switch this process to the encrypted
    /// store. Blocking (Keychain + file I/O) — call it on a blocking thread.
    pub fn migrate_to_encrypted(&self) -> Result<MigrationReport> {
        if self.mode() != SecretsMode::Plaintext {
            return Err(Error::Conflict(
                "secrets are not in the plaintext store — nothing to migrate".into(),
            ));
        }
        if self
            .migrating
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(Error::Conflict(
                "a secrets migration is already running".into(),
            ));
        }
        struct Reset<'a>(&'a AtomicBool);
        impl Drop for Reset<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _reset = Reset(&self.migrating);
        let started = std::time::Instant::now();

        // Master key FIRST, outside the store lock: it may wait on a Keychain
        // prompt (bounded). Locked → abort with nothing changed.
        let mk = self.key.fetch(true)?;

        // Writers wait for the switch (the file work below is milliseconds).
        let mut active = self.active.write().unwrap_or_else(|p| p.into_inner());
        let (migrated, total) = migrate_files(&self.data_dir, &mk)?;
        let enc: Arc<dyn SecretStore> =
            Arc::new(EncryptedFileStore::new(&self.data_dir, self.key.clone()));
        *active = (SecretsMode::Encrypted, enc.clone());
        drop(active);

        // Final verification through the LIVE store, then drop the backup.
        let backup = self.data_dir.join(MIGRATION_BACKUP_FILE);
        if let Some(saved) = read_sealed(&backup, &mk)? {
            for (k, v) in &saved {
                if enc.get(k)?.as_deref() != Some(v.as_str()) {
                    return Err(Error::Internal(format!(
                        "secrets migration: live store verification failed; the encrypted \
                         backup {MIGRATION_BACKUP_FILE} was kept"
                    )));
                }
            }
        }
        let _ = std::fs::remove_file(&backup);
        tracing::info!(
            "secrets: migrated {migrated} entr{} from plaintext to the encrypted store",
            if migrated == 1 { "y" } else { "ies" }
        );
        Ok(MigrationReport {
            migrated,
            total,
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }
}

impl SecretStore for SecretsControl {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.store().put(key, value)
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        self.store().get(key)
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.store().delete(key)
    }
}

/// The file half of the migration (testable without a live control):
///
/// 1. read `secrets.json`;
/// 2. merge with an existing `secrets.enc` (a crash after step 4 of an
///    earlier run) — a key with a DIFFERENT value on each side aborts;
/// 3. write an encrypted backup of the plaintext entries and read it back;
/// 4. write `secrets.enc` and verify every plaintext entry reads back equal
///    from disk (failure → restore the previous `secrets.enc` / remove the new
///    one; the plaintext file is untouched);
/// 5. overwrite `secrets.json` with zeros, fsync, unlink, fsync the directory.
///
/// Returns `(migrated, total)`. The backup is left for the caller's final
/// live verification.
pub fn migrate_files(dir: &Path, mk: &crate::encrypted::MasterKey) -> Result<(usize, usize)> {
    migrate_files_with(dir, mk, |p| read_sealed(p, mk))
}

/// [`migrate_files`] with the step-4 read-back injectable (tests simulate a
/// store that doesn't read back to prove the plaintext survives).
fn migrate_files_with(
    dir: &Path,
    mk: &crate::encrypted::MasterKey,
    read_back: impl Fn(&Path) -> Result<Option<BTreeMap<String, String>>>,
) -> Result<(usize, usize)> {
    let plain_path = dir.join(PLAINTEXT_FILE);
    if !plain_path.exists() {
        return Err(Error::Invalid(
            "no plaintext secrets.json to migrate".into(),
        ));
    }
    let plain = FileStore::new(dir).load()?;
    let enc_path = dir.join(ENCRYPTED_FILE);
    let previous = read_sealed(&enc_path, mk)?;
    let mut merged: BTreeMap<String, String> = previous.clone().unwrap_or_default();
    for (k, v) in &plain {
        match merged.get(k) {
            Some(existing) if existing != v => {
                return Err(Error::Conflict(format!(
                    "secret '{k}' differs between secrets.json and secrets.enc — resolve it \
                     before migrating (nothing was changed)"
                )))
            }
            _ => {
                merged.insert(k.clone(), v.clone());
            }
        }
    }

    // 3. Encrypted backup, verified.
    let backup = dir.join(MIGRATION_BACKUP_FILE);
    write_atomic(&backup, &seal(mk, &plain)?)?;
    if read_sealed(&backup, mk)?.as_ref() != Some(&plain) {
        let _ = std::fs::remove_file(&backup);
        return Err(Error::Internal(
            "secrets migration: backup did not verify (nothing was changed)".into(),
        ));
    }

    // 4. The real store, verified from disk.
    let written = seal(mk, &merged).and_then(|b| write_atomic(&enc_path, &b));
    let verified = written.is_ok()
        && match read_back(&enc_path) {
            Ok(Some(back)) => plain.iter().all(|(k, v)| back.get(k) == Some(v)),
            _ => false,
        };
    if !verified {
        match &previous {
            Some(prev) => {
                let _ = seal(mk, prev).and_then(|b| write_atomic(&enc_path, &b));
            }
            None => {
                let _ = std::fs::remove_file(&enc_path);
            }
        }
        let _ = std::fs::remove_file(&backup);
        return Err(Error::Internal(
            "secrets migration: encrypted store did not read back — plaintext kept, nothing \
             was changed"
                .into(),
        ));
    }

    // 5. Wipe + delete the plaintext.
    wipe_and_remove(&plain_path)?;
    Ok((plain.len(), merged.len()))
}

/// Overwrite a file's bytes with zeros (fsync'd) before unlinking it. On APFS
/// copy-on-write this is best effort; the unlink is what matters, and the
/// overwrite stops the old blocks being trivially readable meanwhile.
fn wipe_and_remove(path: &Path) -> Result<()> {
    let len = std::fs::metadata(path)
        .map(|m| m.len() as usize)
        .unwrap_or(0);
    if len > 0 {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .map_err(|e| Error::Internal(format!("secrets.json wipe open: {e}")))?;
        f.write_all(&vec![0u8; len])
            .and_then(|_| f.sync_all())
            .map_err(|e| Error::Internal(format!("secrets.json wipe: {e}")))?;
    }
    std::fs::remove_file(path).map_err(|e| Error::Internal(format!("secrets.json unlink: {e}")))?;
    if let Some(dir) = path.parent() {
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Process-wide handle
// ---------------------------------------------------------------------------

static GLOBAL: OnceLock<Arc<SecretsControl>> = OnceLock::new();

/// Register the daemon's control (first registration wins).
pub(crate) fn register(c: Arc<SecretsControl>) {
    let _ = GLOBAL.set(c);
}

/// The daemon's secrets control, if `from_env` built one in this process
/// (routes use it for status + the migration; tests without one get `None`).
pub fn global() -> Option<Arc<SecretsControl>> {
    GLOBAL.get().cloned()
}

// ---------------------------------------------------------------------------
// Mode selection
// ---------------------------------------------------------------------------

/// Inputs of [`choose_mode`] (separate so the decision table is unit-tested).
#[derive(Debug, Clone, Copy)]
pub struct ModeInputs<'a> {
    /// `OTTO_SECRETS` value.
    pub env: Option<&'a str>,
    /// `OTTO_SECRETS_ALLOW_PLAINTEXT=1`.
    pub allow_plaintext: bool,
    /// Release build on macOS (plaintext needs an explicit opt-in).
    pub strict: bool,
    pub plaintext_exists: bool,
    pub encrypted_exists: bool,
}

/// Decide the backend:
/// - `encrypted` → encrypted; `keychain` → per-item Keychain.
/// - `file` → plaintext, EXCEPT: once `secrets.enc` exists and `secrets.json`
///   is gone the migration happened (the plist may still say `file` until the
///   app rewrites it) → encrypted; and a strict build without the opt-in
///   refuses plaintext unless a legacy `secrets.json` already holds the
///   user's secrets (refusing then would strand every integration) →
///   encrypted for a fresh install.
/// - unset → `secrets.enc` present ? encrypted : per-item Keychain (the
///   historic default).
pub fn choose_mode(i: ModeInputs<'_>) -> SecretsMode {
    match i.env {
        Some("encrypted") => SecretsMode::Encrypted,
        Some("keychain") => SecretsMode::Keychain,
        Some("file") => {
            if i.encrypted_exists && !i.plaintext_exists {
                SecretsMode::Encrypted
            } else if i.strict && !i.allow_plaintext && !i.plaintext_exists {
                SecretsMode::Encrypted
            } else {
                SecretsMode::Plaintext
            }
        }
        _ if i.encrypted_exists => SecretsMode::Encrypted,
        _ => SecretsMode::Keychain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::encrypted::{open, seal, KeyError, MasterKey, MasterKeySource};
    use std::sync::atomic::AtomicUsize;
    use std::sync::Mutex;
    use std::time::Duration;

    /// In-memory master-key source; counts loads and can be made slow.
    #[derive(Default)]
    struct MemKey {
        key: Mutex<Option<[u8; 32]>>,
        loads: AtomicUsize,
        delay_ms: u64,
    }
    impl MasterKeySource for MemKey {
        fn load(&self) -> Result<Option<MasterKey>> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            if self.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(self.delay_ms));
            }
            Ok(self.key.lock().unwrap().map(MasterKey::from_bytes))
        }
        fn store(&self, key: &MasterKey) -> Result<()> {
            // Round-trip through the hex form the Keychain item uses.
            let mut b = [0u8; 32];
            b.copy_from_slice(&hex::decode(key.to_hex()).unwrap());
            *self.key.lock().unwrap() = Some(b);
            Ok(())
        }
    }
    fn cell(src: Arc<MemKey>, timeout_ms: u64) -> Arc<MasterKeyCell> {
        Arc::new(MasterKeyCell::new(src, Duration::from_millis(timeout_ms)))
    }

    fn map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Every byte of every file in `dir`, for "no plaintext left" checks.
    fn all_bytes(dir: &Path) -> Vec<u8> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            if e.path().is_file() {
                out.extend(std::fs::read(e.path()).unwrap());
            }
        }
        out
    }
    fn contains(hay: &[u8], needle: &str) -> bool {
        hay.windows(needle.len()).any(|w| w == needle.as_bytes())
    }

    #[test]
    fn seal_open_round_trip() {
        let k = MasterKey::generate().unwrap();
        let m = map(&[("conn-1", "hunter2"), ("chan-bot-x-slack", "xoxb-123")]);
        let sealed = seal(&k, &m).unwrap();
        assert!(!contains(&sealed, "hunter2"));
        assert_eq!(open(&k, &sealed).unwrap(), m);
        // Fresh nonce per seal: same input, different ciphertext.
        assert_ne!(seal(&k, &m).unwrap(), sealed);
    }

    #[test]
    fn tamper_truncation_and_wrong_key_are_rejected() {
        let k = MasterKey::generate().unwrap();
        let sealed = seal(&k, &map(&[("a", "secret-value")])).unwrap();
        for i in [0, 9, sealed.len() / 2, sealed.len() - 1] {
            let mut t = sealed.clone();
            t[i] ^= 0x01;
            assert!(open(&k, &t).is_err(), "flip at byte {i} must fail");
        }
        assert!(open(&k, &sealed[..sealed.len() - 1]).is_err());
        assert!(open(&k, &sealed[..10]).is_err());
        let other = MasterKey::generate().unwrap();
        assert!(open(&other, &sealed).is_err());
    }

    #[test]
    fn encrypted_store_round_trip_0600_no_plaintext() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let src = Arc::new(MemKey::default());
        let s = EncryptedFileStore::new(dir.path(), cell(src.clone(), 2000));
        // Reads before any secret exists never touch the Keychain.
        assert_eq!(s.get("k").unwrap(), None);
        s.delete("k").unwrap();
        assert_eq!(src.loads.load(Ordering::SeqCst), 0);
        s.put("k", "very-secret-token").unwrap();
        s.put("k2", "v2").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some("very-secret-token"));
        s.delete("k2").unwrap();
        assert_eq!(s.get("k2").unwrap(), None);
        let meta = std::fs::metadata(s.path()).unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        assert!(!contains(&all_bytes(dir.path()), "very-secret-token"));
        // A second store over the same file + key source reads it (persisted).
        let s2 = EncryptedFileStore::new(dir.path(), cell(src, 2000));
        assert_eq!(s2.get("k").unwrap().as_deref(), Some("very-secret-token"));
    }

    #[test]
    fn locked_keychain_times_out_instead_of_blocking() {
        let dir = tempfile::tempdir().unwrap();
        let src = Arc::new(MemKey {
            delay_ms: 400,
            ..Default::default()
        });
        *src.key.lock().unwrap() = Some([7u8; 32]);
        let c = cell(src.clone(), 50);
        // Seed a file sealed with the same key so `get` needs the key.
        let k = MasterKey::from_bytes([7u8; 32]);
        write_atomic(
            &dir.path().join(ENCRYPTED_FILE),
            &seal(&k, &map(&[("a", "1")])).unwrap(),
        )
        .unwrap();
        let s = EncryptedFileStore::new(dir.path(), c.clone());
        let t = std::time::Instant::now();
        let err = s.get("a").unwrap_err();
        assert!(
            t.elapsed() < Duration::from_millis(300),
            "must not wait for the Keychain"
        );
        assert!(matches!(err, Error::Upstream(ref m) if m.contains("locked")));
        assert_eq!(c.state_label(), "locked");
        assert_eq!(c.fetch(false).unwrap_err(), KeyError::Locked);
        // The single in-flight load finishes in the background; no second
        // Keychain call was stacked by the retries.
        std::thread::sleep(Duration::from_millis(600));
        assert_eq!(c.state_label(), "unlocked");
        assert_eq!(s.get("a").unwrap().as_deref(), Some("1"));
        assert_eq!(src.loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn migration_moves_everything_and_leaves_no_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let plain = FileStore::new(dir.path());
        plain.put("conn-a", "pw-alpha-123").unwrap();
        plain.put("chan-bot-b-slack", "xoxb-bravo-456").unwrap();
        let src = Arc::new(MemKey::default());
        let c = SecretsControl::new(dir.path(), cell(src, 2000), SecretsMode::Plaintext);
        let st = c.status();
        assert!(st.migration_available);
        assert_eq!(st.plaintext_entries, 2);
        let r = c.migrate_to_encrypted().unwrap();
        assert_eq!((r.migrated, r.total), (2, 2));
        assert_eq!(c.mode(), SecretsMode::Encrypted);
        assert!(!dir.path().join(PLAINTEXT_FILE).exists());
        assert!(!dir.path().join(MIGRATION_BACKUP_FILE).exists());
        assert_eq!(c.get("conn-a").unwrap().as_deref(), Some("pw-alpha-123"));
        assert_eq!(
            c.get("chan-bot-b-slack").unwrap().as_deref(),
            Some("xoxb-bravo-456")
        );
        let bytes = all_bytes(dir.path());
        assert!(!contains(&bytes, "pw-alpha-123") && !contains(&bytes, "xoxb-bravo-456"));
        let st = c.status();
        assert!(!st.migration_available && !st.plaintext_file && !st.backup_present);
        assert_eq!(st.key_state, "unlocked");
        // A restart with the stale plist (`OTTO_SECRETS=file`) picks encrypted.
        assert_eq!(
            choose_mode(ModeInputs {
                env: Some("file"),
                allow_plaintext: true,
                strict: true,
                plaintext_exists: false,
                encrypted_exists: true,
            }),
            SecretsMode::Encrypted
        );
        // Second run: nothing to migrate.
        assert!(matches!(c.migrate_to_encrypted(), Err(Error::Conflict(_))));
    }

    #[test]
    fn migration_verifies_before_deleting_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        FileStore::new(dir.path()).put("conn-a", "pw").unwrap();
        FileStore::new(dir.path()).put("conn-b", "pw2").unwrap();
        let before = std::fs::read(dir.path().join(PLAINTEXT_FILE)).unwrap();
        let k = MasterKey::generate().unwrap();
        // The store "reads back" missing an entry → abort.
        let err = migrate_files_with(dir.path(), &k, |p| {
            let mut m = read_sealed(p, &k)?.unwrap_or_default();
            m.remove("conn-b");
            Ok(Some(m))
        })
        .unwrap_err();
        assert!(err.to_string().contains("did not read back"));
        assert_eq!(
            std::fs::read(dir.path().join(PLAINTEXT_FILE)).unwrap(),
            before
        );
        assert!(!dir.path().join(ENCRYPTED_FILE).exists());
        assert!(!dir.path().join(MIGRATION_BACKUP_FILE).exists());
    }

    #[test]
    fn migration_aborts_untouched_when_keychain_locked_or_conflicting() {
        let dir = tempfile::tempdir().unwrap();
        FileStore::new(dir.path()).put("conn-a", "pw").unwrap();
        let before = std::fs::read(dir.path().join(PLAINTEXT_FILE)).unwrap();
        let slow = Arc::new(MemKey {
            delay_ms: 300,
            ..Default::default()
        });
        let c = SecretsControl::new(dir.path(), cell(slow, 30), SecretsMode::Plaintext);
        assert!(matches!(c.migrate_to_encrypted(), Err(Error::Upstream(_))));
        assert_eq!(c.mode(), SecretsMode::Plaintext);
        assert_eq!(
            std::fs::read(dir.path().join(PLAINTEXT_FILE)).unwrap(),
            before
        );
        assert!(!dir.path().join(ENCRYPTED_FILE).exists());

        // Conflicting value already in secrets.enc → Conflict, nothing changed.
        let k = MasterKey::generate().unwrap();
        write_atomic(
            &dir.path().join(ENCRYPTED_FILE),
            &seal(&k, &map(&[("conn-a", "other")])).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            migrate_files(dir.path(), &k),
            Err(Error::Conflict(_))
        ));
        assert_eq!(
            std::fs::read(dir.path().join(PLAINTEXT_FILE)).unwrap(),
            before
        );
        // Same value (a crash after writing secrets.enc) → completes.
        write_atomic(
            &dir.path().join(ENCRYPTED_FILE),
            &seal(&k, &map(&[("conn-a", "pw"), ("x", "y")])).unwrap(),
        )
        .unwrap();
        assert_eq!(migrate_files(dir.path(), &k).unwrap(), (1, 2));
        assert!(!dir.path().join(PLAINTEXT_FILE).exists());
    }

    #[test]
    fn mode_decision_table() {
        let m = |env, allow, strict, plain, enc| {
            choose_mode(ModeInputs {
                env,
                allow_plaintext: allow,
                strict,
                plaintext_exists: plain,
                encrypted_exists: enc,
            })
        };
        use SecretsMode::*;
        // Dev/CI/Linux: plaintext as asked.
        assert_eq!(m(Some("file"), false, false, false, false), Plaintext);
        // Release macOS, no opt-in, fresh install → refuse plaintext.
        assert_eq!(m(Some("file"), false, true, false, false), Encrypted);
        // Release macOS with the opt-in → plaintext allowed.
        assert_eq!(m(Some("file"), true, true, false, false), Plaintext);
        // Legacy install with secrets.json: never strand the secrets.
        assert_eq!(m(Some("file"), false, true, true, false), Plaintext);
        // Already migrated (stale plist) → encrypted.
        assert_eq!(m(Some("file"), true, false, false, true), Encrypted);
        assert_eq!(m(Some("encrypted"), false, true, true, false), Encrypted);
        assert_eq!(m(Some("keychain"), false, true, false, true), Keychain);
        assert_eq!(m(None, false, true, false, false), Keychain);
        assert_eq!(m(None, false, true, false, true), Encrypted);
    }

    #[test]
    fn async_read_with_locked_keychain_returns_promptly() {
        let dir = tempfile::tempdir().unwrap();
        let k = MasterKey::from_bytes([9u8; 32]);
        write_atomic(
            &dir.path().join(ENCRYPTED_FILE),
            &seal(&k, &map(&[("a", "1")])).unwrap(),
        )
        .unwrap();
        let src = Arc::new(MemKey {
            delay_ms: 2_000,
            ..Default::default()
        });
        let control = Arc::new(SecretsControl::new(
            dir.path(),
            cell(src, 100),
            SecretsMode::Encrypted,
        ));
        let store: Arc<dyn SecretStore> = Arc::new(crate::CachingSecretStore::new(
            control,
            Duration::from_secs(60),
        ));
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            // A ticker on the SAME (single) worker keeps running while the
            // read waits on the blocking pool.
            let ticks = Arc::new(AtomicUsize::new(0));
            let t2 = ticks.clone();
            let ticker = tokio::spawn(async move {
                loop {
                    t2.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            });
            let started = std::time::Instant::now();
            let r = otto_core::secrets::get_async(&store, "a").await;
            assert!(matches!(r, Err(Error::Upstream(_))));
            assert!(started.elapsed() < Duration::from_millis(1_000));
            assert!(ticks.load(Ordering::SeqCst) >= 5, "worker was blocked");
            ticker.abort();
        });
    }
}
