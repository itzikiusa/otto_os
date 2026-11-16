//! Persistent external build cache for local Go Run & Debug targets.
//!
//! Compiles Go executables and test binaries outside the user's repository
//! (`~/Library/Caches/Otto/ide-builds/go/`, `$TMPDIR/otto-ide-builds-<user>/go/`,
//! or `$OTTO_IDE_BUILD_CACHE_DIR/go/`), keeping `git status` and the project tree
//! free of `__debug_bin*` artifacts. Cache misses atomically `rename` a fresh
//! inode into place so macOS Apple Silicon's `cs_blobs` code-signing cache never
//! `SIGKILL`s an updated binary.
//!
//! Staleness: the stamp is derived from `go list -deps -json` — Go's own answer
//! to "which files make up this binary" — so `//go:embed` assets, cgo and
//! assembly sources, `.syso` objects, packages in directories named `build`/
//! `dist`/`testdata`, and locally `replace`d modules outside the project all
//! count. Re-deriving Go's file-selection rules (build tags, `GOOS`, embed
//! patterns) here is how the old directory walk went stale. When `go list`
//! fails or its output cannot be read there is no stamp and the binary is
//! rebuilt: a stale binary is worse than a slow build. A full `go build` on
//! every launch would also be correct, but relinks (seconds for a large
//! service); `go list` plus a stat per file is far cheaper on a warm launch.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use otto_core::{Error, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::process::Command;
use uuid::Uuid;

use crate::proc_group::{wait_bounded, BUILD_STEP_TIMEOUT, HELPER_OUTPUT_CAP};

const MAX_CACHED_GO_BINARIES: usize = 64;
/// Total bytes of cached binaries kept (debug builds are 40–150 MB each).
const MAX_GO_CACHE_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Cached binaries unused for this long are pruned.
const MAX_GO_CACHE_AGE: Duration = Duration::from_secs(14 * 24 * 60 * 60);
/// `*.tmp.*` files older than this were left by an interrupted build.
const STALE_TEMP_AGE: Duration = Duration::from_secs(60 * 60);
/// Bound on `go list` (it may download modules on a cold cache).
const GO_LIST_TIMEOUT: Duration = Duration::from_secs(5 * 60);
/// Fields `go list` reports per package; every file list that feeds the build.
const GO_LIST_FIELDS: &str =
    "ImportPath,Dir,Standard,GoFiles,CgoFiles,CFiles,CXXFiles,MFiles,HFiles,\
FFiles,SFiles,SwigFiles,SwigCXXFiles,SysoFiles,EmbedFiles,TestGoFiles,XTestGoFiles,\
TestEmbedFiles,XTestEmbedFiles,Module";
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

static BUILD_LOCKS: otto_core::keyed_lock::KeyedLocks<String> =
    otto_core::keyed_lock::KeyedLocks::new();

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

    let _guard = BUILD_LOCKS.lock(&cache_key).await;

    let stamp = compute_go_list_stamp(&req, &canon_cwd, &module_root, &env_fp).await;
    let bin_path = cache_dir.join(&cache_key);
    let stamp_path = cache_dir.join(format!("{cache_key}.stamp"));
    if let Some(stamp) = stamp.as_deref() {
        if bin_path.is_file() {
            if let Ok(existing_stamp) = fs::read_to_string(&stamp_path) {
                if existing_stamp.trim() == stamp {
                    touch_mtime(&bin_path);
                    return Ok(GoCachedArtifact {
                        binary_path: bin_path,
                        module_root,
                        cache_hit: true,
                    });
                }
            }
        }
    }

    let nonce = Uuid::new_v4().simple().to_string();
    let tmp_bin = cache_dir.join(format!("{cache_key}.tmp.{nonce}"));
    let mut tmp_guard = TempFileGuard(Some(tmp_bin.clone()));
    let mut cmd = Command::new(req.go_bin);
    match req.mode {
        GoBuildMode::Run => {
            cmd.arg("build");
        }
        GoBuildMode::Debug => {
            cmd.arg("build").arg("-gcflags=all=-N -l");
        }
        GoBuildMode::DebugTest => {
            cmd.arg("test").arg("-c").arg("-gcflags=all=-N -l");
        }
    }
    for flag in req.extra_build_flags {
        if !flag.trim().is_empty() {
            cmd.arg(flag);
        }
    }
    cmd.arg("-o").arg(&tmp_bin).arg(req.pkg_spec);
    // Own process group + kill-on-drop: Stop (a dropped future) or the timeout
    // takes `compile`/`link` children down with `go` itself.
    cmd.current_dir(&canon_cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_clear()
        .envs(req.env_map);
    #[cfg(unix)]
    cmd.process_group(0);

    let child = cmd.spawn().map_err(|e| {
        Error::Invalid(format!(
            "failed to invoke `{}` for Go build: {e}",
            req.go_bin
        ))
    })?;
    let out = wait_bounded(child, BUILD_STEP_TIMEOUT, HELPER_OUTPUT_CAP)
        .await
        .map_err(|e| Error::Invalid(format!("go build: {e}")))?;
    if !out.status.success() {
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
        Error::Internal(format!(
            "failed to finalize cached Go binary {}: {e}",
            bin_path.display()
        ))
    })?;
    tmp_guard.0 = None;
    match stamp {
        Some(stamp) => {
            let tmp_stamp = cache_dir.join(format!("{cache_key}.stamp.tmp.{nonce}"));
            if fs::write(&tmp_stamp, format!("{stamp}\n")).is_ok() {
                let _ = fs::rename(&tmp_stamp, &stamp_path);
            }
        }
        // No stamp: the next launch must rebuild rather than trust an old one.
        None => {
            let _ = fs::remove_file(&stamp_path);
        }
    }
    let prune_dir = cache_dir.clone();
    let prune_key = cache_key.clone();
    tokio::task::spawn_blocking(move || {
        prune_go_cache_dir(&prune_dir, &prune_key, SystemTime::now())
    });

    Ok(GoCachedArtifact {
        binary_path: bin_path,
        module_root,
        cache_hit: false,
    })
}

/// Removes a half-written build output when the build is cancelled or fails.
struct TempFileGuard(Option<PathBuf>);

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = fs::remove_file(p);
        }
    }
}

/// Mark a cache hit as recently used so age/size pruning keeps it.
fn touch_mtime(path: &Path) {
    if let Ok(f) = fs::File::options().append(true).open(path) {
        let _ = f.set_modified(SystemTime::now());
    }
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

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
struct GoListPackage {
    import_path: String,
    dir: String,
    standard: bool,
    go_files: Vec<String>,
    cgo_files: Vec<String>,
    c_files: Vec<String>,
    #[serde(rename = "CXXFiles")]
    cxx_files: Vec<String>,
    m_files: Vec<String>,
    h_files: Vec<String>,
    f_files: Vec<String>,
    s_files: Vec<String>,
    swig_files: Vec<String>,
    #[serde(rename = "SwigCXXFiles")]
    swig_cxx_files: Vec<String>,
    syso_files: Vec<String>,
    embed_files: Vec<String>,
    test_go_files: Vec<String>,
    x_test_go_files: Vec<String>,
    test_embed_files: Vec<String>,
    x_test_embed_files: Vec<String>,
    module: Option<GoListModule>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "PascalCase", default)]
struct GoListModule {
    go_mod: String,
}

impl GoListPackage {
    fn files(&self) -> impl Iterator<Item = &String> {
        self.go_files
            .iter()
            .chain(&self.cgo_files)
            .chain(&self.c_files)
            .chain(&self.cxx_files)
            .chain(&self.m_files)
            .chain(&self.h_files)
            .chain(&self.f_files)
            .chain(&self.s_files)
            .chain(&self.swig_files)
            .chain(&self.swig_cxx_files)
            .chain(&self.syso_files)
            .chain(&self.embed_files)
            .chain(&self.test_go_files)
            .chain(&self.x_test_go_files)
            .chain(&self.test_embed_files)
            .chain(&self.x_test_embed_files)
    }
}

/// Ask `go list` for every non-standard package the build reads, then stamp the
/// size and mtime of each listed file plus each module's `go.mod`. `None` (no
/// stamp, so rebuild) whenever `go list` cannot give a trustworthy answer.
async fn compute_go_list_stamp(
    req: &GoBuildRequest<'_>,
    canon_cwd: &Path,
    module_root: &Path,
    env_fp: &str,
) -> Option<String> {
    let mut cmd = Command::new(req.go_bin);
    cmd.arg("list").arg("-deps");
    if req.mode == GoBuildMode::DebugTest {
        cmd.arg("-test");
    }
    cmd.arg(format!("-json={GO_LIST_FIELDS}"));
    for flag in req.extra_build_flags {
        if !flag.trim().is_empty() {
            cmd.arg(flag);
        }
    }
    cmd.arg(req.pkg_spec)
        .current_dir(canon_cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env_clear()
        .envs(req.env_map);
    #[cfg(unix)]
    cmd.process_group(0);
    let child = cmd.spawn().ok()?;
    let out = wait_bounded(child, GO_LIST_TIMEOUT, HELPER_OUTPUT_CAP)
        .await
        .ok()?;
    if !out.status.success() {
        tracing::debug!(
            stderr = %String::from_utf8_lossy(&out.stderr).trim(),
            "go list failed; Go binary will be rebuilt"
        );
        return None;
    }
    let mut header = Sha256::new();
    header.update(req.mode.as_tag().as_bytes());
    header.update(b"\0");
    header.update(req.pkg_spec.as_bytes());
    header.update(b"\0");
    header.update(env_fp.as_bytes());
    for f in req.extra_build_flags {
        header.update(b"\0");
        header.update(f.as_bytes());
    }
    let module_root = module_root.to_path_buf();
    tokio::task::spawn_blocking(move || stamp_go_list_output(header, &out.stdout, &module_root))
        .await
        .ok()?
}

/// Hash `go list -json` output (a stream of package objects) into a stamp.
fn stamp_go_list_output(mut h: Sha256, stdout: &[u8], module_root: &Path) -> Option<String> {
    if stdout.len() >= HELPER_OUTPUT_CAP {
        return None; // the tail was kept: the package list is incomplete
    }
    let mut packages = 0usize;
    let mut go_mods: Vec<PathBuf> = ["go.mod", "go.sum", "go.work", "go.work.sum"]
        .iter()
        .map(|n| module_root.join(n))
        .collect();
    for pkg in serde_json::Deserializer::from_slice(stdout).into_iter::<GoListPackage>() {
        let pkg = pkg.ok()?;
        // GOROOT is covered by the toolchain fingerprint; a `-test` main
        // (`p.test`) is generated from the test files hashed with `p`.
        if pkg.standard || pkg.import_path.ends_with(".test") {
            continue;
        }
        packages += 1;
        let dir = Path::new(&pkg.dir);
        h.update(b"\nP:");
        h.update(pkg.dir.as_bytes());
        for f in pkg.files() {
            hash_file_meta(&mut h, &dir.join(f));
        }
        if let Some(m) = pkg.module.as_ref().filter(|m| !m.go_mod.is_empty()) {
            let gm = PathBuf::from(&m.go_mod);
            if !go_mods.contains(&gm) {
                go_mods.push(gm);
            }
        }
    }
    if packages == 0 {
        return None;
    }
    for gm in &go_mods {
        hash_file_meta(&mut h, gm);
    }
    Some(hex::encode(h.finalize()))
}

fn hash_file_meta(h: &mut Sha256, path: &Path) {
    h.update(b"\nF:");
    h.update(path.to_string_lossy().as_bytes());
    match fs::metadata(path) {
        Ok(meta) => {
            h.update(meta.len().to_le_bytes());
            if let Ok(mt) = meta
                .modified()
                .and_then(|t| t.duration_since(UNIX_EPOCH).map_err(std::io::Error::other))
            {
                h.update(mt.as_secs().to_le_bytes());
                h.update(mt.subsec_nanos().to_le_bytes());
            }
        }
        Err(_) => h.update(b"missing"),
    }
}

/// Keep the cache bounded by count, total bytes and age (newest first, the
/// just-built `keep_key` always kept), and remove `*.tmp.*` files that an
/// interrupted build left behind.
#[allow(clippy::disallowed_methods)] // invoked inside tokio::task::spawn_blocking
fn prune_go_cache_dir(cache_dir: &Path, keep_key: &str, now: SystemTime) {
    let Ok(rd) = fs::read_dir(cache_dir) else {
        return;
    };
    let age_of = |m: &fs::Metadata| {
        m.modified()
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .unwrap_or_default()
    };
    let mut bins: Vec<(PathBuf, Duration, u64)> = Vec::new();
    let mut keep_size = 0u64;
    for entry in rd.flatten() {
        let name = entry.file_name();
        let Some(s) = name.to_str() else {
            continue;
        };
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if s.contains(".tmp.") {
            if age_of(&meta) > STALE_TEMP_AGE {
                let _ = fs::remove_file(entry.path());
            }
            continue;
        }
        if s.contains('.') {
            continue;
        }
        if s == keep_key {
            keep_size = meta.len();
            continue;
        }
        bins.push((entry.path(), age_of(&meta), meta.len()));
    }
    bins.sort_by_key(|(_, age, _)| *age);
    let mut kept = 1usize;
    let mut total = keep_size;
    for (bin, age, size) in bins {
        if kept < MAX_CACHED_GO_BINARIES
            && total.saturating_add(size) <= MAX_GO_CACHE_BYTES
            && age <= MAX_GO_CACHE_AGE
        {
            kept += 1;
            total = total.saturating_add(size);
            continue;
        }
        let stamp = bin.with_extension("stamp");
        let _ = fs::remove_file(&bin);
        let _ = fs::remove_file(&stamp);
    }
}

#[cfg(test)]
#[path = "go_build_cache_tests.rs"]
mod tests;
