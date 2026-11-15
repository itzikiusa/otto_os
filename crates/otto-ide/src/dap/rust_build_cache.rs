//! Fast source-stamp metadata cache for local Rust debug artifact builds.
//!
//! Avoids spawning `cargo build --message-format=json` / `cargo test --no-run`
//! (~300ms–1500ms Cargo workspace graph + proc-macro startup overhead even on a
//! no-op rebuild) when no Rust sources, manifests, lockfiles, or Cargo config
//! files have changed and the compiled debug executable is untouched on disk.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

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

#[derive(Debug, Clone)]
pub(super) struct RustArtifactStamp {
    pub cache_key: String,
    pub stamp: String,
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
        let stamp_info = compute_rust_stamp_sync(
            &cargo_bin,
            &cargo_args,
            &cargo_cwd,
            &target_root,
            &raw_prog,
            want_test,
            &env_subset,
            &allowed,
        )?;
        let entry_path = rust_build_cache_dir().join(format!("{}.json", stamp_info.cache_key));
        let hit = read_valid_cached_executable(&entry_path, &stamp_info.stamp);
        Some((stamp_info, hit))
    })
    .await
    .ok()
    .flatten()
}

pub(super) fn store_cached_rust_artifact(stamp: RustArtifactStamp, executable: String) {
    tokio::task::spawn_blocking(move || {
        save_cached_executable_sync(&stamp, &executable);
    });
}

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
    if !cargo_cwd.is_dir() {
        return None;
    }
    let canon_cwd = fs::canonicalize(cargo_cwd).unwrap_or_else(|_| cargo_cwd.to_path_buf());
    let canon_root = fs::canonicalize(target_root).unwrap_or_else(|_| target_root.to_path_buf());

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
    let cache_key = hex::encode(key_h.finalize())[..32].to_string();

    let mut h = Sha256::new();
    h.update(b"rust-debug-v1\0");
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

    hash_rust_tree(&canon_cwd, &canon_cwd, &mut h);

    // Include ancestor workspace Cargo.toml / Cargo.lock / .cargo/config.toml up to target_root.
    let within_allowed = |cand: &Path| -> bool {
        allowed_roots.is_empty() || allowed_roots.iter().any(|r| cand.starts_with(r))
    };
    let mut anc = canon_cwd.parent();
    while let Some(dir) = anc {
        if !within_allowed(dir) {
            break;
        }
        for rel in [
            "Cargo.toml",
            "Cargo.lock",
            "rust-toolchain",
            "rust-toolchain.toml",
            ".cargo/config.toml",
            ".cargo/config",
        ] {
            let p = dir.join(rel);
            if p.is_file() {
                hash_single_file(&mut h, &p);
            }
        }
        if dir == canon_root {
            break;
        }
        anc = dir.parent();
    }

    Some(RustArtifactStamp {
        cache_key,
        stamp: hex::encode(h.finalize()),
    })
}

#[allow(clippy::disallowed_methods)] // runs inside tokio::task::spawn_blocking
fn hash_rust_tree(root: &Path, dir: &Path, h: &mut Sha256) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = rd.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name_os = entry.file_name();
        let Some(name) = name_os.to_str() else {
            continue;
        };
        if name.starts_with('.') && name != ".cargo" {
            continue;
        }
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        if ft.is_dir() {
            if matches!(
                name,
                "target" | "node_modules" | "vendor" | "dist" | "build"
            ) {
                continue;
            }
            hash_rust_tree(root, &path, h);
        } else if ft.is_file() && is_rust_stamp_input(name) {
            if let Ok(meta) = entry.metadata() {
                let rel = path.strip_prefix(root).unwrap_or(&path);
                h.update(b"\nF:");
                h.update(rel.to_string_lossy().as_bytes());
                h.update(meta.len().to_le_bytes());
                if let Ok(mt) = meta
                    .modified()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).map_err(std::io::Error::other))
                {
                    h.update(mt.as_secs().to_le_bytes());
                    h.update(mt.subsec_nanos().to_le_bytes());
                }
            }
        }
    }
}

fn is_rust_stamp_input(name: &str) -> bool {
    name.ends_with(".rs")
        || name.ends_with(".toml")
        || name.ends_with(".lock")
        || name.ends_with(".proto")
        || name.ends_with(".sql")
        || name.ends_with(".c")
        || name.ends_with(".cc")
        || name.ends_with(".cpp")
        || name.ends_with(".h")
        || matches!(name, "config" | "rust-toolchain")
}

fn hash_single_file(h: &mut Sha256, path: &Path) {
    h.update(b"\nS:");
    h.update(path.to_string_lossy().as_bytes());
    if let Ok(meta) = fs::metadata(path) {
        h.update(meta.len().to_le_bytes());
        if let Ok(mt) = meta
            .modified()
            .and_then(|t| t.duration_since(UNIX_EPOCH).map_err(std::io::Error::other))
        {
            h.update(mt.as_secs().to_le_bytes());
            h.update(mt.subsec_nanos().to_le_bytes());
        }
    }
}

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

fn save_cached_executable_sync(stamp: &RustArtifactStamp, executable: &str) {
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
    let dir = rust_build_cache_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let entry = CachedRustArtifactEntry {
        stamp: stamp.stamp.clone(),
        executable: executable.to_string(),
        exe_len: exe_meta.len(),
        exe_mtime_nanos: mtime_nanos,
    };
    let Ok(bytes) = serde_json::to_vec(&entry) else {
        return;
    };
    let nonce = Uuid::new_v4().simple().to_string();
    let final_path = dir.join(format!("{}.json", stamp.cache_key));
    let tmp_path = dir.join(format!(".{}.json.tmp.{nonce}", stamp.cache_key));
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
        if fs::rename(&tmp_path, &final_path).is_err() {
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
    fn rust_artifact_stamp_hits_and_invalidates_on_source_or_binary_edit() {
        let dir = tempfile::tempdir().expect("tempdir");
        let crate_dir = dir.path().join("demo");
        fs::create_dir_all(crate_dir.join("src")).expect("mkdir src");
        fs::write(
            crate_dir.join("Cargo.toml"),
            "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .expect("write Cargo.toml");
        fs::write(crate_dir.join("src/main.rs"), "fn main() {}\n").expect("write main.rs");

        let exe_path = dir.path().join("demo-bin");
        fs::write(&exe_path, b"fake-elf-macho-binary").expect("write binary");

        let args = vec!["build".to_string(), "--message-format=json".to_string()];
        let stamp1 =
            compute_rust_stamp_sync("cargo", &args, &crate_dir, dir.path(), "", false, &[], &[])
                .expect("stamp1");

        let entry_path = dir.path().join("entry.json");
        let exe_meta = fs::metadata(&exe_path).unwrap();
        let mtime = exe_meta
            .modified()
            .unwrap()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let entry = CachedRustArtifactEntry {
            stamp: stamp1.stamp.clone(),
            executable: exe_path.to_string_lossy().into_owned(),
            exe_len: exe_meta.len(),
            exe_mtime_nanos: mtime,
        };
        fs::write(&entry_path, serde_json::to_vec(&entry).unwrap()).unwrap();

        assert_eq!(
            read_valid_cached_executable(&entry_path, &stamp1.stamp).as_deref(),
            Some(exe_path.to_string_lossy().as_ref())
        );

        // Editing src/main.rs changes the stamp -> cache miss.
        fs::write(
            crate_dir.join("src/main.rs"),
            "fn main() { println!(\"hi\"); }\n",
        )
        .expect("modify main.rs");
        let stamp2 =
            compute_rust_stamp_sync("cargo", &args, &crate_dir, dir.path(), "", false, &[], &[])
                .expect("stamp2");
        assert_ne!(stamp1.stamp, stamp2.stamp);
        assert!(read_valid_cached_executable(&entry_path, &stamp2.stamp).is_none());

        // Removing the compiled binary (`cargo clean`) also invalidates even if stamp matched.
        fs::remove_file(&exe_path).unwrap();
        assert!(read_valid_cached_executable(&entry_path, &stamp1.stamp).is_none());
    }
}
