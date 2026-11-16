//! Fast source-stamp and Cargo `.d` dep-info cache for local Rust debug builds.
//!
//! Avoids spawning `cargo build --message-format=json` / `cargo test --no-run`
//! (~300ms–1500ms Cargo workspace graph + proc-macro startup overhead even on a
//! no-op rebuild) when no transitive Rust inputs, workspace manifests, lockfiles,
//! toolchains, or `RUSTFLAGS` have changed and the debug binary is intact on disk.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::rust_build_depinfo::{
    find_workspace_root, hash_dep_info_inputs, hash_rust_tree, hash_single_file,
};
use crate::go_build_cache::ide_build_cache_root;

const RUST_ENV_FINGERPRINT_KEYS: &[&str] = &[
    "RUSTFLAGS",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_TARGET_DIR",
    "RUSTUP_TOOLCHAIN",
    "RUSTC",
    "RUSTC_WRAPPER",
];

const ANCESTOR_CONFIG_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain",
    "rust-toolchain.toml",
    ".cargo/config.toml",
    ".cargo/config",
];

#[derive(Debug, Clone)]
pub(super) struct RustArtifactStamp {
    pub cache_key: String,
    pub stamp: String,
    cargo_bin: String,
    cargo_args: Vec<String>,
    cargo_cwd: PathBuf,
    target_root: PathBuf,
    raw_prog: String,
    want_test: bool,
    env_subset: Vec<(String, String)>,
    allowed_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CachedRustArtifactEntry {
    stamp: String,
    executable: String,
    exe_len: u64,
    exe_mtime_nanos: u128,
}

fn rust_build_cache_dir() -> PathBuf {
    ide_build_cache_root().join("rust")
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn lookup_cached_rust_artifact(
    cargo_bin: &str,
    cargo_args: &[String],
    cargo_cwd: &str,
    target_root: &str,
    raw_prog: &str,
    want_test: bool,
    env: &BTreeMap<String, String>,
    allowed_roots: &[PathBuf],
) -> Option<(RustArtifactStamp, Option<String>)> {
    let cargo_bin = cargo_bin.to_string();
    let cargo_args = cargo_args.to_vec();
    let cargo_cwd = PathBuf::from(cargo_cwd);
    let target_root = PathBuf::from(target_root);
    let raw_prog = raw_prog.to_string();
    let env_subset: Vec<(String, String)> = RUST_ENV_FINGERPRINT_KEYS
        .iter()
        .filter_map(|&k| env.get(k).map(|v| (k.to_string(), v.clone())))
        .collect();
    let allowed = allowed_roots.to_vec();

    tokio::task::spawn_blocking(move || {
        let cache_dir = rust_build_cache_dir();
        let stamp_info = compute_rust_stamp_with_cache_dir(
            &cache_dir,
            &cargo_bin,
            &cargo_args,
            &cargo_cwd,
            &target_root,
            &raw_prog,
            want_test,
            &env_subset,
            &allowed,
        )?;
        let entry_path = cache_dir.join(format!("{}.json", stamp_info.cache_key));
        let hit = read_valid_cached_executable(&entry_path, &stamp_info.stamp);
        Some((stamp_info, hit))
    })
    .await
    .ok()
    .flatten()
}

pub(super) fn store_cached_rust_artifact(stamp: RustArtifactStamp, executable: String) {
    tokio::task::spawn_blocking(move || {
        save_cached_executable_sync(&rust_build_cache_dir(), &stamp, &executable);
    });
}

fn build_cache_key(
    canon_cwd: &Path,
    raw_prog: &str,
    want_test: bool,
    cargo_args: &[String],
) -> String {
    let mut key_h = Sha256::new();
    key_h.update(canon_cwd.to_string_lossy().as_bytes());
    key_h.update(b"\0");
    key_h.update(raw_prog.as_bytes());
    key_h.update(b"\0");
    key_h.update([u8::from(want_test)]);
    for arg in cargo_args {
        key_h.update(b"\0");
        key_h.update(arg.as_bytes());
    }
    hex::encode(key_h.finalize())[..32].to_string()
}

#[allow(clippy::too_many_arguments, clippy::disallowed_methods)]
fn compute_rust_stamp_with_cache_dir(
    cache_dir: &Path,
    cargo_bin: &str,
    cargo_args: &[String],
    cargo_cwd: &Path,
    target_root: &Path,
    raw_prog: &str,
    want_test: bool,
    env_subset: &[(String, String)],
    allowed_roots: &[PathBuf],
) -> Option<RustArtifactStamp> {
    if !cargo_cwd.is_dir() {
        return None;
    }
    let canon_cwd = fs::canonicalize(cargo_cwd).unwrap_or_else(|_| cargo_cwd.to_path_buf());
    let canon_root = fs::canonicalize(target_root).unwrap_or_else(|_| target_root.to_path_buf());
    let ws_root = find_workspace_root(&canon_cwd, &canon_root, allowed_roots);
    let cache_key = build_cache_key(&canon_cwd, raw_prog, want_test, cargo_args);

    let mut h = Sha256::new();
    h.update(b"rust-debug-v2\0");
    h.update(cargo_bin.as_bytes());
    if cargo_bin.contains('/') {
        hash_single_file(&mut h, Path::new(cargo_bin));
    }
    h.update([u8::from(want_test)]);
    h.update(raw_prog.as_bytes());
    for arg in cargo_args {
        h.update(b"\0");
        h.update(arg.as_bytes());
    }
    for (k, v) in env_subset {
        h.update(b"\nE:");
        h.update(k.as_bytes());
        h.update(b"=");
        h.update(v.as_bytes());
    }

    let within_allowed = |cand: &Path| -> bool {
        allowed_roots.is_empty() || allowed_roots.iter().any(|r| cand.starts_with(r))
    };

    // Always fingerprint ancestor Cargo.toml, Cargo.lock, rust-toolchain*, and .cargo/config*
    // from `canon_cwd` up through `ws_root`.
    let mut anc = Some(canon_cwd.as_path());
    while let Some(dir) = anc {
        if !within_allowed(dir) {
            break;
        }
        for rel in ANCESTOR_CONFIG_FILES {
            let p = dir.join(rel);
            if p.is_file() {
                hash_single_file(&mut h, &p);
            } else {
                h.update(b"\nA_NONE:");
                h.update(p.to_string_lossy().as_bytes());
            }
        }
        if dir == ws_root {
            break;
        }
        anc = dir.parent();
    }

    let cached_dep_file = cache_dir.join(format!("{cache_key}.d"));
    let used_dep_info = cached_dep_file.is_file()
        && hash_dep_info_inputs(
            &cached_dep_file,
            &ws_root,
            &canon_cwd,
            &within_allowed,
            &mut h,
        );
    if !used_dep_info {
        h.update(b"\nWSWALK:");
        hash_rust_tree(&ws_root, &ws_root, &within_allowed, &mut h);
    }

    Some(RustArtifactStamp {
        cache_key,
        stamp: hex::encode(h.finalize()),
        cargo_bin: cargo_bin.to_string(),
        cargo_args: cargo_args.to_vec(),
        cargo_cwd: canon_cwd,
        target_root: canon_root,
        raw_prog: raw_prog.to_string(),
        want_test,
        env_subset: env_subset.to_vec(),
        allowed_roots: allowed_roots.to_vec(),
    })
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn compute_rust_stamp_sync(
    cargo_bin: &str,
    cargo_args: &[String],
    cargo_cwd: &Path,
    target_root: &Path,
    raw_prog: &str,
    want_test: bool,
    env_subset: &[(String, String)],
    allowed_roots: &[PathBuf],
) -> Option<RustArtifactStamp> {
    compute_rust_stamp_with_cache_dir(
        &rust_build_cache_dir(),
        cargo_bin,
        cargo_args,
        cargo_cwd,
        target_root,
        raw_prog,
        want_test,
        env_subset,
        allowed_roots,
    )
}

#[allow(clippy::disallowed_methods)]
fn read_valid_cached_executable(entry_path: &Path, expected_stamp: &str) -> Option<String> {
    let raw = fs::read(entry_path).ok()?;
    let entry: CachedRustArtifactEntry = serde_json::from_slice(&raw).ok()?;
    if entry.stamp != expected_stamp {
        return None;
    }
    let exe_meta = fs::metadata(&entry.executable).ok()?;
    if !exe_meta.is_file() || exe_meta.len() != entry.exe_len {
        return None;
    }
    let mtime_nanos = exe_meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    if mtime_nanos != entry.exe_mtime_nanos {
        return None;
    }
    Some(entry.executable)
}

#[allow(clippy::disallowed_methods)]
fn write_atomic_private(dir: &Path, filename: &str, bytes: &[u8]) -> bool {
    let nonce = Uuid::new_v4().simple().to_string();
    let final_path = dir.join(filename);
    let tmp_path = dir.join(format!(".{filename}.tmp.{nonce}"));
    let write_res = {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        opts.open(&tmp_path)
            .and_then(|mut f| f.write_all(bytes).and_then(|()| f.flush()))
    };
    if write_res.is_ok() && fs::rename(&tmp_path, &final_path).is_ok() {
        true
    } else {
        let _ = fs::remove_file(&tmp_path);
        false
    }
}

#[allow(clippy::disallowed_methods)]
fn save_cached_executable_sync(cache_dir: &Path, stamp: &RustArtifactStamp, executable: &str) {
    let Ok(exe_meta) = fs::metadata(executable) else {
        return;
    };
    if !exe_meta.is_file() {
        return;
    }
    let Ok(mtime_nanos) = exe_meta
        .modified()
        .and_then(|t| t.duration_since(UNIX_EPOCH).map_err(std::io::Error::other))
        .map(|d| d.as_nanos())
    else {
        return;
    };
    if fs::create_dir_all(cache_dir).is_err() {
        return;
    }

    // Preserve Cargo's `<built_exe>.d` dep-info next to the cache entry so subsequent
    // lookups stat only the exact transitive source inputs instead of walking the tree.
    let src_dep_path = PathBuf::from(format!("{executable}.d"));
    let dst_dep_name = format!("{}.d", stamp.cache_key);
    if let Ok(dep_bytes) = fs::read(&src_dep_path) {
        let _ = write_atomic_private(cache_dir, &dst_dep_name, &dep_bytes);
    } else {
        let _ = fs::remove_file(cache_dir.join(&dst_dep_name));
    }

    let final_stamp = compute_rust_stamp_with_cache_dir(
        cache_dir,
        &stamp.cargo_bin,
        &stamp.cargo_args,
        &stamp.cargo_cwd,
        &stamp.target_root,
        &stamp.raw_prog,
        stamp.want_test,
        &stamp.env_subset,
        &stamp.allowed_roots,
    )
    .map(|s| s.stamp)
    .unwrap_or_else(|| stamp.stamp.clone());

    let entry = CachedRustArtifactEntry {
        stamp: final_stamp,
        executable: executable.to_string(),
        exe_len: exe_meta.len(),
        exe_mtime_nanos: mtime_nanos,
    };
    let Ok(bytes) = serde_json::to_vec(&entry) else {
        return;
    };
    let _ = write_atomic_private(cache_dir, &format!("{}.json", stamp.cache_key), &bytes);
}

#[cfg(test)]
#[path = "rust_build_cache_tests.rs"]
mod tests;
