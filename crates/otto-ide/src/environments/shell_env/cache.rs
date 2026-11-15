//! Persistent disk cache (`ide-shell-env-cache.json`) for [`super::ShellEnvResolver`].
//!
//! Avoids paying the 1.0s–3.5s `$SHELL -lic` startup penalty on every daemon
//! restart or `Otto.app` deploy when none of the user's shell rc files have
//! changed since the previous probe.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub(super) const SHELL_ENV_CACHE_FILENAME: &str = "ide-shell-env-cache.json";
const CACHE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct CachedShellEnvDisk {
    pub version: u32,
    pub fingerprint: String,
    pub shell: String,
    pub loaded_at: String,
    pub duration_ms: u64,
    pub vars: BTreeMap<String, String>,
}

/// Resolve the active login shell and compute its RC metadata fingerprint
/// outside any `ShellEnvResolver` lock.
pub(super) fn current_shell_and_rc_fingerprint() -> (String, String) {
    let shell = super::detect_user_shell();
    let fp = compute_shell_rc_fingerprint(&shell);
    (shell, fp)
}

/// Compute a `<0.2ms` metadata fingerprint over the active login shell binary
/// and standard zsh/bash/POSIX startup files (`mtime` + `size`).
pub(super) fn compute_shell_rc_fingerprint(shell: &str) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let zdotdir = std::env::var_os("ZDOTDIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    compute_fingerprint_for_paths(shell, &user, home.as_deref(), zdotdir.as_deref())
}

fn compute_fingerprint_for_paths(
    shell: &str,
    user: &str,
    home: Option<&Path>,
    zdotdir: Option<&Path>,
) -> String {
    let mut h = Sha256::new();
    h.update(b"v1\0");
    h.update(shell.as_bytes());
    h.update(b"\0");
    h.update(user.as_bytes());
    h.update(b"\0");
    hash_file_stat(&mut h, Path::new(shell));

    let mut candidates: Vec<PathBuf> = Vec::with_capacity(16);
    if let Some(hm) = home {
        h.update(hm.to_string_lossy().as_bytes());
        h.update(b"\0");
        for rel in [
            ".zshenv",
            ".zprofile",
            ".zshrc",
            ".zlogin",
            ".bash_profile",
            ".bashrc",
            ".profile",
        ] {
            candidates.push(hm.join(rel));
        }
    }
    if let Some(zd) = zdotdir {
        for rel in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
            let p = zd.join(rel);
            if !candidates.contains(&p) {
                candidates.push(p);
            }
        }
    }
    for sys in [
        "/etc/zshenv",
        "/etc/zprofile",
        "/etc/zshrc",
        "/etc/profile",
        "/etc/bashrc",
    ] {
        candidates.push(PathBuf::from(sys));
    }
    for path in &candidates {
        hash_file_stat(&mut h, path);
    }
    hex::encode(h.finalize())
}

fn hash_file_stat(h: &mut Sha256, path: &Path) {
    h.update(b"\nP:");
    h.update(path.to_string_lossy().as_bytes());
    match fs::metadata(path) {
        Ok(meta) => {
            h.update([1u8]);
            h.update(meta.len().to_le_bytes());
            if let Ok(mt) = meta
                .modified()
                .and_then(|t| t.duration_since(UNIX_EPOCH).map_err(std::io::Error::other))
            {
                h.update(mt.as_secs().to_le_bytes());
                h.update(mt.subsec_nanos().to_le_bytes());
            }
        }
        Err(_) => {
            h.update([0u8]);
        }
    }
}

pub(super) fn load_cached_shell_env(
    cache_path: &Path,
    expected_fp: &str,
) -> Option<CachedShellEnvDisk> {
    let raw = fs::read(cache_path).ok()?;
    let mut disk: CachedShellEnvDisk = serde_json::from_slice(&raw).ok()?;
    if disk.version != CACHE_SCHEMA_VERSION || disk.fingerprint != expected_fp {
        return None;
    }
    disk.vars.retain(|k, v| super::should_keep_env_entry(k, v));
    Some(disk)
}

pub(super) fn save_cached_shell_env(cache_path: &Path, disk: &CachedShellEnvDisk) {
    let Some(parent) = cache_path.parent() else {
        return;
    };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(bytes) = serde_json::to_vec(disk) else {
        return;
    };
    let nonce = Uuid::new_v4().simple().to_string();
    let file_name = cache_path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(SHELL_ENV_CACHE_FILENAME);
    let tmp_path = parent.join(format!(".{file_name}.tmp.{nonce}"));
    let write_res = {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(&tmp_path)
            .and_then(|mut f| f.write_all(&bytes).and_then(|()| f.flush()))
    };
    if write_res.is_ok() {
        if fs::rename(&tmp_path, cache_path).is_err() {
            let _ = fs::remove_file(&tmp_path);
        }
    } else {
        let _ = fs::remove_file(&tmp_path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_cache_round_trips_and_invalidates_when_dotfile_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("home");
        fs::create_dir_all(&home).expect("mkdir home");
        let zshrc = home.join(".zshrc");
        fs::write(&zshrc, "export GOPRIVATE=example.com/*\n").expect("write zshrc");

        let fp1 = compute_fingerprint_for_paths("/bin/sh", "tester", Some(&home), None);
        let cache_path = dir.path().join(SHELL_ENV_CACHE_FILENAME);
        let mut vars = BTreeMap::new();
        vars.insert("GOPRIVATE".to_string(), "example.com/*".to_string());
        vars.insert("PWD".to_string(), "/ignored".to_string());

        let disk = CachedShellEnvDisk {
            version: CACHE_SCHEMA_VERSION,
            fingerprint: fp1.clone(),
            shell: "/bin/sh".to_string(),
            loaded_at: "2026-11-15T00:00:00Z".to_string(),
            duration_ms: 42,
            vars,
        };
        save_cached_shell_env(&cache_path, &disk);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&cache_path)
                .expect("cache metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600, "shell env cache must be owner-only (0600)");
        }

        let loaded = load_cached_shell_env(&cache_path, &fp1).expect("cache hit");
        assert_eq!(
            loaded.vars.get("GOPRIVATE").map(String::as_str),
            Some("example.com/*")
        );
        assert!(
            !loaded.vars.contains_key("PWD"),
            "ignored keys must be scrubbed on load"
        );

        // Appending to .zshrc changes size -> fingerprint changes -> cache invalidates.
        fs::write(
            &zshrc,
            "export GOPRIVATE=example.com/*\nexport EXTRA_FLAG=1\n",
        )
        .expect("update zshrc");
        let fp2 = compute_fingerprint_for_paths("/bin/sh", "tester", Some(&home), None);
        assert_ne!(fp1, fp2);
        assert!(load_cached_shell_env(&cache_path, &fp2).is_none());
    }

    #[test]
    fn resolver_with_cache_dir_warm_loads_and_invalidates_on_rc_change() {
        use super::super::ShellEnvResolver;
        use std::sync::Arc;

        if otto_core::e2e::isolated() {
            return;
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let data_dir = dir.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();

        // 1. Real `ShellEnvResolver::with_cache_dir` warm-loads immediately without spawning a shell.
        let (shell, real_fp) = current_shell_and_rc_fingerprint();
        let cache_file = data_dir.join(SHELL_ENV_CACHE_FILENAME);
        let mut cached_vars = BTreeMap::new();
        cached_vars.insert(
            "OTTO_WARM_SHELL_CACHE_SENTINEL".to_string(),
            "from_disk_cache".to_string(),
        );
        save_cached_shell_env(
            &cache_file,
            &CachedShellEnvDisk {
                version: CACHE_SCHEMA_VERSION,
                fingerprint: real_fp,
                shell: shell.clone(),
                loaded_at: "2026-11-15T01:02:03Z".to_string(),
                duration_ms: 17,
                vars: cached_vars.clone(),
            },
        );

        let warm = Arc::new(ShellEnvResolver::with_cache_dir(&data_dir));
        assert!(
            !warm.is_cold_or_warming(),
            "fresh resolver with valid disk cache must start warm"
        );
        let snap = warm.snapshot();
        assert_eq!(snap.status, "ready");
        assert_eq!(
            snap.vars
                .get("OTTO_WARM_SHELL_CACHE_SENTINEL")
                .map(String::as_str),
            Some("from_disk_cache")
        );

        // 2. When an RC file's fingerprint changes, a fresh resolver invalidates the cache (`cold`).
        let fake_home = dir.path().join("fake-home");
        fs::create_dir_all(&fake_home).unwrap();
        let rc_file = fake_home.join(".zshrc");
        fs::write(&rc_file, "export FOO=1\n").unwrap();
        let fp_v1 = compute_fingerprint_for_paths("/bin/sh", "tester", Some(&fake_home), None);
        let rc_cache_file = dir.path().join("rc-test-cache.json");
        save_cached_shell_env(
            &rc_cache_file,
            &CachedShellEnvDisk {
                version: CACHE_SCHEMA_VERSION,
                fingerprint: fp_v1.clone(),
                shell: "/bin/sh".to_string(),
                loaded_at: "2026-11-15T01:02:03Z".to_string(),
                duration_ms: 9,
                vars: cached_vars,
            },
        );
        let hit = ShellEnvResolver::new_with_shell_and_fp(
            Some(rc_cache_file.clone()),
            "/bin/sh".to_string(),
            &fp_v1,
        );
        assert!(!hit.is_cold_or_warming());

        fs::write(&rc_file, "export FOO=1\nexport BAR=2\n").unwrap();
        let fp_v2 = compute_fingerprint_for_paths("/bin/sh", "tester", Some(&fake_home), None);
        assert_ne!(fp_v1, fp_v2);
        let invalidated = ShellEnvResolver::new_with_shell_and_fp(
            Some(rc_cache_file),
            "/bin/sh".to_string(),
            &fp_v2,
        );
        assert!(
            invalidated.is_cold_or_warming(),
            "changing .zshrc fingerprint must invalidate disk cache to cold state"
        );
    }
}
