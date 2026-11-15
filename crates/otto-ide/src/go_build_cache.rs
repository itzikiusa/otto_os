//! Persistent external build cache for local Go Run & Debug targets.
//!
//! Compiles Go executables and test binaries outside the user's repository
//! (`~/Library/Caches/Otto/ide-builds/go/`, `$TMPDIR/otto-ide-builds-<user>/go/`,
//! or `$OTTO_IDE_BUILD_CACHE_DIR/go/`), keeping `git status` and the project tree
//! free of `__debug_bin*` artifacts. A fast metadata stamp over `.go`/`go.mod`/
//! `go.sum`/`go.work` files skips `go build` entirely on warm launches, and cache
//! misses atomically `rename` a fresh inode into place so macOS Apple Silicon's
//! `cs_blobs` code-signing cache never `SIGKILL`s an updated binary.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, OnceLock};
use std::time::UNIX_EPOCH;

use dashmap::DashMap;
use otto_core::{Error, Result};
use sha2::{Digest, Sha256};
use tokio::process::Command;
use tokio::sync::Mutex;
use uuid::Uuid;

const MAX_CACHED_GO_BINARIES: usize = 64;
const GO_ENV_FINGERPRINT_KEYS: &[&str] = &[
    "GOOS",
    "GOARCH",
    "CGO_ENABLED",
    "GOFLAGS",
    "GOEXPERIMENT",
    "GOROOT",
    "GOPATH",
    "GOMODCACHE",
    "GOPRIVATE",
    "GOWORK",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoBuildMode {
    Run,
    Debug,
    DebugTest,
}

impl GoBuildMode {
    fn as_tag(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Debug => "debug",
            Self::DebugTest => "debug_test",
        }
    }
}

#[derive(Debug, Clone)]
pub struct GoBuildRequest<'a> {
    pub go_bin: &'a str,
    pub cwd: &'a Path,
    pub pkg_spec: &'a str,
    pub mode: GoBuildMode,
    pub extra_build_flags: &'a [String],
    pub env_map: &'a BTreeMap<String, String>,
    pub allowed_roots: &'a [PathBuf],
}

#[derive(Debug, Clone)]
pub struct GoCachedArtifact {
    pub binary_path: PathBuf,
    pub module_root: PathBuf,
    pub cache_hit: bool,
}

fn build_locks() -> &'static DashMap<String, Arc<Mutex<()>>> {
    static LOCKS: OnceLock<DashMap<String, Arc<Mutex<()>>>> = OnceLock::new();
    LOCKS.get_or_init(DashMap::new)
}

/// Root directory for external IDE build caches (shared by Go and Java builders).
pub fn ide_build_cache_root() -> PathBuf {
    if let Ok(raw) = std::env::var("OTTO_IDE_BUILD_CACHE_DIR") {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Some(cache) = dirs::cache_dir() {
        return cache.join("Otto").join("ide-builds");
    }
    let user_tag: String = std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "otto".into())
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let suffix = if user_tag.is_empty() {
        "otto"
    } else {
        user_tag.as_str()
    };
    std::env::temp_dir().join(format!("otto-ide-builds-{suffix}"))
}

pub fn go_build_cache_dir() -> PathBuf {
    ide_build_cache_root().join("go")
}

/// Discover the owning `go.work` or `go.mod` directory for `start`, confined to
/// `allowed_roots` when provided so non-root builds never walk outside authorized roots.
pub fn find_go_module_root(start: &Path, allowed_roots: &[PathBuf]) -> PathBuf {
    let base = if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    };
    let canon_base = fs::canonicalize(base).unwrap_or_else(|_| base.to_path_buf());
    let within_allowed = |cand: &Path| -> bool {
        allowed_roots.is_empty() || allowed_roots.iter().any(|r| cand.starts_with(r))
    };
    let mut cur = Some(canon_base.as_path());
    let mut found_mod: Option<PathBuf> = None;
    while let Some(dir) = cur {
        if !within_allowed(dir) {
            break;
        }
        if dir.join("go.work").is_file() {
            return dir.to_path_buf();
        }
        if found_mod.is_none() && dir.join("go.mod").is_file() {
            found_mod = Some(dir.to_path_buf());
        }
        cur = dir.parent();
    }
    found_mod.unwrap_or(canon_base)
}

/// Ensure a compiled Go binary for `req` is up to date in the external build cache.
pub async fn ensure_go_binary(req: GoBuildRequest<'_>) -> Result<GoCachedArtifact> {
    let cache_dir = go_build_cache_dir();
    fs::create_dir_all(&cache_dir).map_err(|e| {
        Error::Internal(format!(
            "failed to create Go build cache directory {}: {e}",
            cache_dir.display()
        ))
    })?;

    let canon_cwd = fs::canonicalize(req.cwd).unwrap_or_else(|_| req.cwd.to_path_buf());
    let module_root = find_go_module_root(&canon_cwd, req.allowed_roots);
    let env_fp = compute_env_fingerprint(req.go_bin, req.env_map);
    let cache_key = compute_cache_key(
        &module_root,
        &canon_cwd,
        req.pkg_spec,
        req.mode,
        req.extra_build_flags,
        &env_fp,
    );

    let key_lock = build_locks()
        .entry(cache_key.clone())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .value()
        .clone();
    let _guard = key_lock.lock().await;

    let mut scan_roots = vec![module_root.clone()];
    for extra in req.allowed_roots {
        let canon = fs::canonicalize(extra).unwrap_or_else(|_| extra.clone());
        if !scan_roots.iter().any(|r| canon.starts_with(r)) {
            scan_roots.push(canon);
        }
    }
    let stamp_roots = scan_roots.clone();
    let env_fp_clone = env_fp.clone();
    let extra_flags_vec = req.extra_build_flags.to_vec();
    let mode = req.mode;
    let pkg_spec_owned = req.pkg_spec.to_string();
    let stamp = tokio::task::spawn_blocking(move || {
        compute_go_source_stamp(
            &stamp_roots,
            &pkg_spec_owned,
            mode,
            &extra_flags_vec,
            &env_fp_clone,
        )
    })
    .await
    .map_err(|e| Error::Internal(format!("Go source stamp task failed: {e}")))??;

    let bin_path = cache_dir.join(&cache_key);
    let stamp_path = cache_dir.join(format!("{cache_key}.stamp"));
    if bin_path.is_file() {
        if let Ok(existing_stamp) = fs::read_to_string(&stamp_path) {
            if existing_stamp.trim() == stamp {
                return Ok(GoCachedArtifact {
                    binary_path: bin_path,
                    module_root,
                    cache_hit: true,
                });
            }
        }
    }

    let nonce = Uuid::new_v4().simple().to_string();
    let tmp_bin = cache_dir.join(format!("{cache_key}.tmp.{nonce}"));
    let mut cmd = Command::new(req.go_bin);
    match req.mode {
        GoBuildMode::Run => {
            cmd.arg("build");
            for flag in req.extra_build_flags {
                if !flag.trim().is_empty() {
                    cmd.arg(flag);
                }
            }
            cmd.arg("-o").arg(&tmp_bin).arg(req.pkg_spec);
        }
        GoBuildMode::Debug => {
            cmd.arg("build").arg("-gcflags=all=-N -l");
            for flag in req.extra_build_flags {
                if !flag.trim().is_empty() {
                    cmd.arg(flag);
                }
            }
            cmd.arg("-o").arg(&tmp_bin).arg(req.pkg_spec);
        }
        GoBuildMode::DebugTest => {
            cmd.arg("test").arg("-c").arg("-gcflags=all=-N -l");
            for flag in req.extra_build_flags {
                if !flag.trim().is_empty() {
                    cmd.arg(flag);
                }
            }
            cmd.arg("-o").arg(&tmp_bin).arg(req.pkg_spec);
        }
    }
    cmd.current_dir(&canon_cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .envs(req.env_map);

    let out = cmd.output().await.map_err(|e| {
        let _ = fs::remove_file(&tmp_bin);
        Error::Invalid(format!(
            "failed to invoke `{}` for Go build: {e}",
            req.go_bin
        ))
    })?;
    if !out.status.success() {
        let _ = fs::remove_file(&tmp_bin);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        return Err(Error::Invalid(format!("go build failed:\n{detail}")));
    }

    fs::rename(&tmp_bin, &bin_path).map_err(|e| {
        let _ = fs::remove_file(&tmp_bin);
        Error::Internal(format!(
            "failed to finalize cached Go binary {}: {e}",
            bin_path.display()
        ))
    })?;
    let tmp_stamp = cache_dir.join(format!("{cache_key}.stamp.tmp.{nonce}"));
    if fs::write(&tmp_stamp, format!("{stamp}\n")).is_ok() {
        let _ = fs::rename(&tmp_stamp, &stamp_path);
    }
    let prune_dir = cache_dir.clone();
    let prune_key = cache_key.clone();
    let _ = tokio::task::spawn_blocking(move || prune_go_cache_dir(&prune_dir, &prune_key)).await;

    Ok(GoCachedArtifact {
        binary_path: bin_path,
        module_root,
        cache_hit: false,
    })
}

fn compute_env_fingerprint(go_bin: &str, env_map: &BTreeMap<String, String>) -> String {
    let mut h = Sha256::new();
    h.update(go_bin.as_bytes());
    if let Ok(meta) = fs::metadata(go_bin) {
        h.update(meta.len().to_le_bytes());
        if let Ok(mt) = meta
            .modified()
            .and_then(|t| t.duration_since(UNIX_EPOCH).map_err(std::io::Error::other))
        {
            h.update(mt.as_nanos().to_le_bytes());
        }
    }
    for &key in GO_ENV_FINGERPRINT_KEYS {
        if let Some(val) = env_map.get(key) {
            h.update(key.as_bytes());
            h.update(b"=");
            h.update(val.as_bytes());
            h.update(b"\0");
        }
    }
    hex::encode(h.finalize())[..24].to_string()
}

fn compute_cache_key(
    module_root: &Path,
    cwd: &Path,
    pkg_spec: &str,
    mode: GoBuildMode,
    extra_flags: &[String],
    env_fp: &str,
) -> String {
    let mut h = Sha256::new();
    h.update(module_root.to_string_lossy().as_bytes());
    h.update(b"\0");
    h.update(cwd.to_string_lossy().as_bytes());
    h.update(b"\0");
    h.update(pkg_spec.as_bytes());
    h.update(b"\0");
    h.update(mode.as_tag().as_bytes());
    h.update(b"\0");
    for f in extra_flags {
        h.update(f.as_bytes());
        h.update(b"\x1f");
    }
    h.update(b"\0");
    h.update(env_fp.as_bytes());
    hex::encode(h.finalize())[..32].to_string()
}

fn compute_go_source_stamp(
    roots: &[PathBuf],
    pkg_spec: &str,
    mode: GoBuildMode,
    extra_flags: &[String],
    env_fp: &str,
) -> Result<String> {
    let mut h = Sha256::new();
    h.update(mode.as_tag().as_bytes());
    h.update(b"\0");
    h.update(pkg_spec.as_bytes());
    h.update(b"\0");
    h.update(env_fp.as_bytes());
    for f in extra_flags {
        h.update(b"\0");
        h.update(f.as_bytes());
    }
    for root in roots {
        h.update(b"\nROOT:");
        h.update(root.to_string_lossy().as_bytes());
        hash_go_tree(root, root, &mut h);
    }
    Ok(hex::encode(h.finalize()))
}

#[allow(clippy::disallowed_methods)] // invoked inside tokio::task::spawn_blocking
fn hash_go_tree(root: &Path, dir: &Path, h: &mut Sha256) {
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
        if name.starts_with('.') || name.starts_with("__debug_bin") {
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
                "vendor" | "node_modules" | "target" | "dist" | "build" | "testdata"
            ) {
                continue;
            }
            hash_go_tree(root, &path, h);
        } else if ft.is_file() && is_go_stamp_input(name) {
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

fn is_go_stamp_input(name: &str) -> bool {
    name.ends_with(".go")
        || name.ends_with(".s")
        || matches!(name, "go.mod" | "go.sum" | "go.work" | "go.work.sum")
}

#[allow(clippy::disallowed_methods)] // invoked inside tokio::task::spawn_blocking
fn prune_go_cache_dir(cache_dir: &Path, keep_key: &str) {
    let Ok(rd) = fs::read_dir(cache_dir) else {
        return;
    };
    let mut bins: Vec<(PathBuf, u128)> = Vec::new();
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Some(s) = name.to_str() else {
            continue;
        };
        if s.contains('.') || s == keep_key {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        bins.push((entry.path(), mtime));
    }
    if bins.len() <= MAX_CACHED_GO_BINARIES {
        return;
    }
    bins.sort_by_key(|(_, mt)| *mt);
    let drop_count = bins.len().saturating_sub(MAX_CACHED_GO_BINARIES);
    for (bin, _) in bins.into_iter().take(drop_count) {
        let stamp = bin.with_extension("stamp");
        let _ = fs::remove_file(&bin);
        let _ = fs::remove_file(&stamp);
    }
}
