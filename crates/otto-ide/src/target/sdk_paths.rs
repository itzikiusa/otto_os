//! Allowlisted read-only external SDK / module-cache path resolution for
//! "Dive In" (`Cmd+Click` / `F12` / Step-Into).
//!
//! Security invariants:
//! - External paths are strictly read-only (`read_only: true`); every write or
//!   mutation path continues to reject anything outside writable project roots.
//! - Credential directories (`~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.kube`, …) and
//!   secret filenames (`.env`, `id_rsa`, `*.pem`, `*.key`, …) are ALWAYS denied
//!   on external paths, even when `is_root_user == true`.
//! - Non-root callers never inherit unvalidated parent `go.work`/`go.mod` roots
//!   or custom SDK directories outside standard/registered toolchains or RBAC.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use otto_core::{Error, Result};

pub(crate) use super::sdk_archives::{discover_jdk_src_zips, read_virtual_java_archive};

use super::fs_ops::enforce_non_root_path;
use super::map_io_err;

/// Decode a `file://` URI into a filesystem path string, handling percent-encoded
/// bytes such as `%20` (spaces) and `%40` (`@` in Go `$GOMODCACHE` module paths).
pub(crate) fn strip_file_uri(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("file://") {
        if let Some(p) = otto_core::paths::file_uri_to_path(trimmed) {
            return p.to_string_lossy().into_owned();
        }
        let rest = trimmed
            .strip_prefix("file://localhost/")
            .map(|r| format!("/{r}"))
            .or_else(|| trimmed.strip_prefix("file:///").map(|r| format!("/{r}")))
            .or_else(|| trimmed.strip_prefix("file://").map(str::to_string))
            .unwrap_or_else(|| trimmed.to_string());
        return percent_decode(&rest);
    }
    trimmed.to_string()
}

pub(crate) fn percent_decode(input: &str) -> String {
    otto_core::paths::percent_decode_utf8(input).unwrap_or_else(|| input.to_string())
}

/// True when `path` is a virtual Java source/class reference (`jdt://...`,
/// `jar:file://...!/...`, or `/path/to/archive.(jar|zip)!/...`).
pub(crate) fn is_virtual_archive_uri(raw: &str) -> bool {
    let t = raw.trim();
    t.starts_with("jdt://") || t.starts_with("jar:") || t.contains("!/")
}

pub(crate) fn collect_readonly_sdk_roots(extra_dirs: &[PathBuf]) -> Vec<PathBuf> {
    collect_readonly_sdk_roots_with_env(extra_dirs, None)
}

/// Collect canonicalized read-only external SDK & package-cache roots on the
/// local machine without spawning any subprocesses.
pub(crate) fn collect_readonly_sdk_roots_with_env(
    extra_dirs: &[PathBuf],
    shell_env: Option<&BTreeMap<String, String>>,
) -> Vec<PathBuf> {
    let base = collect_base_readonly_sdk_roots(shell_env);
    if extra_dirs.is_empty() {
        return base;
    }
    let home_canon = dirs::home_dir().and_then(|h| h.canonicalize().ok());
    let mut out = Vec::with_capacity(extra_dirs.len() + base.len());
    for raw in extra_dirs {
        let Ok(canon) = raw.canonicalize() else {
            continue;
        };
        let sdk_dir = if canon.is_file() {
            match canon.parent() {
                Some(p) if p.file_name().and_then(|n| n.to_str()) == Some("bin") => {
                    p.parent().unwrap_or(p).to_path_buf()
                }
                Some(p) => p.to_path_buf(),
                None => continue,
            }
        } else {
            canon
        };
        if sdk_dir == Path::new("/")
            || home_canon.as_ref().is_some_and(|h| *h == sdk_dir)
            || otto_core::secret_paths::is_denied_dir(&sdk_dir)
        {
            continue;
        }
        if !out.contains(&sdk_dir) {
            out.push(sdk_dir);
        }
    }
    for b in base {
        if !out.contains(&b) {
            out.push(b);
        }
    }
    out
}

pub(crate) fn collect_base_readonly_sdk_roots(
    shell_env: Option<&BTreeMap<String, String>>,
) -> Vec<PathBuf> {
    use std::sync::{Mutex, OnceLock};
    use std::time::{Duration, Instant};

    type BaseCacheEntry = (Instant, Option<String>, Vec<PathBuf>);
    static NO_SHELL_CACHE: OnceLock<Mutex<Option<BaseCacheEntry>>> = OnceLock::new();
    let extra_env = std::env::var("OTTO_EXTRA_READONLY_ROOTS").ok();
    if shell_env.is_none() {
        let cache = NO_SHELL_CACHE.get_or_init(|| Mutex::new(None));
        if let Ok(guard) = cache.lock() {
            if let Some((at, prev_env, paths)) = guard.as_ref() {
                if at.elapsed() < Duration::from_secs(10) && *prev_env == extra_env {
                    return paths.clone();
                }
            }
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::with_capacity(48);
    let env_val = |key: &str| -> Option<String> {
        std::env::var(key)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                shell_env
                    .and_then(|m| m.get(key))
                    .cloned()
                    .filter(|v| !v.trim().is_empty())
            })
    };

    for key in [
        "GOROOT",
        "GOMODCACHE",
        "JAVA_HOME",
        "VIRTUAL_ENV",
        "MAVEN_REPO_LOCAL",
        "GRADLE_USER_HOME",
    ] {
        if let Some(v) = env_val(key) {
            candidates.push(PathBuf::from(v));
        }
    }
    if let Some(gopath) = env_val("GOPATH") {
        for part in gopath.split(':').filter(|s| !s.trim().is_empty()) {
            let gp = PathBuf::from(part);
            candidates.push(gp.join("pkg/mod"));
            candidates.push(gp.join("src"));
        }
    }
    if let Some(cargo_home) = env_val("CARGO_HOME") {
        let ch = PathBuf::from(cargo_home);
        candidates.push(ch.join("registry"));
        candidates.push(ch.join("git/checkouts"));
    }
    if let Some(rustup_home) = env_val("RUSTUP_HOME") {
        candidates.push(PathBuf::from(rustup_home).join("toolchains"));
    }
    if let Some(extra) = env_val("OTTO_EXTRA_READONLY_ROOTS") {
        for part in extra.split(':').filter(|s| !s.trim().is_empty()) {
            candidates.push(PathBuf::from(part));
        }
    }
    if let Some(path_var) = env_val("PATH") {
        for seg in path_var.split(':').filter(|s| !s.trim().is_empty()) {
            let go_bin = Path::new(seg).join("go");
            if go_bin.is_file() {
                if let Ok(canon_go) = go_bin.canonicalize() {
                    if let Some(go_home) = canon_go.parent().and_then(Path::parent) {
                        if go_home.join("src/runtime").is_dir()
                            || go_home.join("libexec/src/runtime").is_dir()
                        {
                            candidates.push(go_home.to_path_buf());
                        }
                    }
                }
            }
        }
    }

    if let Some(home) = dirs::home_dir() {
        for rel in [
            "go/pkg/mod",
            "go/src",
            "sdk",
            ".otto/toolchains",
            ".cargo/registry",
            ".cargo/git/checkouts",
            ".rustup/toolchains",
            ".m2/repository",
            ".gradle/caches",
            ".sdkman/candidates/java",
            ".pyenv/versions",
            ".local/share/uv/python",
            ".nvm/versions/node",
            ".fnm/node-versions",
            ".volta/tools",
            ".asdf/installs",
            ".local/share/mise/installs",
        ] {
            candidates.push(home.join(rel));
        }
    }

    for sys in [
        "/usr/local/go",
        "/opt/homebrew/Cellar/go",
        "/opt/homebrew/opt/go",
        "/opt/homebrew/Cellar",
        "/opt/homebrew/opt",
        "/opt/homebrew/lib/node_modules",
        "/usr/local/Cellar",
        "/usr/local/lib/node_modules",
        "/usr/lib/go",
        "/usr/share/go",
        "/Library/Java/JavaVirtualMachines",
        "/usr/lib/jvm",
        "/Library/Frameworks/Python.framework",
        "/usr/lib/python3",
        "/Library/Developer/CommandLineTools/SDKs",
    ] {
        candidates.push(PathBuf::from(sys));
    }

    let home_canon = dirs::home_dir().and_then(|h| h.canonicalize().ok());
    let mut out = Vec::with_capacity(candidates.len());
    for cand in candidates {
        let Ok(canon) = cand.canonicalize() else {
            continue;
        };
        if canon == Path::new("/") {
            continue;
        }
        if home_canon.as_ref().is_some_and(|h| *h == canon) {
            continue;
        }
        if otto_core::secret_paths::is_denied_dir(&canon) {
            continue;
        }
        if !out.contains(&canon) {
            out.push(canon);
        }
    }
    if shell_env.is_none() {
        if let Some(cache) = NO_SHELL_CACHE.get() {
            if let Ok(mut guard) = cache.lock() {
                *guard = Some((Instant::now(), extra_env, out.clone()));
            }
        }
    }
    out
}

fn is_toolchains_dir(canonical: &Path, readonly_roots: &[PathBuf]) -> bool {
    readonly_roots.iter().any(|r| {
        let is_tc = r
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == "toolchains")
            || r.components()
                .any(|c| c.as_os_str().to_str() == Some("toolchains"));
        is_tc && canonical.starts_with(r)
    })
}

fn is_dependency_vendor_subpath(canonical: &Path) -> bool {
    canonical.components().any(|c| {
        matches!(
            c.as_os_str().to_str(),
            Some("node_modules" | "site-packages" | ".venv")
        )
    })
}

/// Resolve an existing local file for reading across:
/// 1. Primary `root` + attached `additional_roots` (writable unless inside
///    `node_modules`/`site-packages`/`.venv` or filesystem `readonly`),
/// 2. Allowlisted external `readonly_roots` (`force_readonly = true`).
pub(crate) fn resolve_local_readable(
    root: &Path,
    additional_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    is_root_user: bool,
    raw: &str,
) -> Result<(PathBuf, bool)> {
    let decoded = strip_file_uri(raw);
    let trimmed = decoded.trim();
    if trimmed.is_empty() || trimmed == "." {
        return Err(Error::Invalid("file path must not be empty".into()));
    }
    let candidate = if trimmed.starts_with('~') {
        otto_core::paths::expand_home_strict(trimmed).map_err(Error::Invalid)?
    } else {
        let p = Path::new(trimmed);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            root.join(p)
        }
    };

    let canonical = candidate
        .canonicalize()
        .map_err(|e| map_io_err("resolve path", &candidate, e))?;

    let in_project = std::iter::once(root)
        .chain(additional_roots.iter().map(PathBuf::as_path))
        .any(|r| canonical.starts_with(r));
    if in_project {
        if !is_root_user {
            enforce_non_root_path(&canonical)?;
        }
        let vendor_ro = is_dependency_vendor_subpath(&canonical);
        return Ok((canonical, vendor_ro));
    }

    if otto_core::secret_paths::is_denied_dir(&canonical)
        || otto_core::secret_paths::is_denied_file(&canonical)
    {
        return Err(Error::Forbidden(format!(
            "{} holds protected credentials or secrets; access denied",
            canonical.display()
        )));
    }
    if !is_toolchains_dir(&canonical, readonly_roots)
        && otto_core::secret_paths::in_protected_dir(&canonical)
    {
        return Err(Error::Forbidden(format!(
            "{} holds protected Otto state; access denied",
            canonical.display()
        )));
    }

    let in_allowlist = readonly_roots.iter().any(|r| canonical.starts_with(r))
        || collect_readonly_sdk_roots_with_env(&[], None)
            .iter()
            .any(|r| canonical.starts_with(r));
    if !in_allowlist {
        return Err(Error::Forbidden(format!(
            "path '{}' is outside workspace roots and allowlisted SDK directories",
            canonical.display()
        )));
    }

    Ok((canonical, true))
}

/// Discover local Go `replace` (`go.mod` / `go.work`) and `use` (`go.work`) target
/// directories referenced by `primary_root` or `additional_roots`.
///
/// Callers MUST authorize any returned directory via workspace RBAC before admitting
/// it for non-root users, and NEVER place discovered directories into writable roots.
#[allow(clippy::disallowed_methods)] // tiny manifest reads (`go.mod` / `go.work` <64KB)
pub(crate) fn discover_local_go_replace_roots(
    primary_root: &Path,
    additional_roots: &[PathBuf],
    is_root_user: bool,
) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut inspect_dirs: Vec<PathBuf> = Vec::with_capacity(1 + additional_roots.len() + 1);
    inspect_dirs.push(primary_root.to_path_buf());
    inspect_dirs.extend(additional_roots.iter().cloned());
    if is_root_user {
        if let Some(parent) = primary_root.parent() {
            if parent.join("go.work").is_file() && !inspect_dirs.iter().any(|d| d == parent) {
                inspect_dirs.push(parent.to_path_buf());
            }
        }
    }

    let mut push_candidate = |base: &Path, raw_target: &str| {
        let trimmed = raw_target.trim().trim_matches('"').trim_matches('\'');
        if trimmed.is_empty() {
            return;
        }
        let candidate = if let Some(rest) = trimmed.strip_prefix("~/") {
            let Some(home) = dirs::home_dir() else {
                return;
            };
            home.join(rest)
        } else if Path::new(trimmed).is_absolute() {
            PathBuf::from(trimmed)
        } else if trimmed.starts_with('.') || base.join(trimmed).join("go.mod").is_file() {
            base.join(trimmed)
        } else {
            return;
        };
        let Ok(canon) = super::ssh_exec::canonicalize_local_root_dir(
            &candidate.to_string_lossy(),
            is_root_user,
        ) else {
            return;
        };
        if canon != primary_root && !additional_roots.contains(&canon) && !found.contains(&canon) {
            found.push(canon);
        }
    };

    for dir in &inspect_dirs {
        for manifest_name in ["go.mod", "go.work"] {
            let mf = dir.join(manifest_name);
            let Ok(text) = std::fs::read_to_string(&mf) else {
                continue;
            };
            let mut in_use_block = false;
            for raw_line in text.lines() {
                let line = raw_line.split("//").next().unwrap_or("").trim();
                if line.is_empty() {
                    continue;
                }
                if line == ")" {
                    in_use_block = false;
                    continue;
                }
                if let Some((_, rhs)) = line.split_once("=>") {
                    if let Some(tok) = rhs.split_whitespace().next() {
                        push_candidate(dir, tok);
                    }
                    continue;
                }
                if manifest_name == "go.work" {
                    if let Some(rest) = line.strip_prefix("use") {
                        let rest = rest.trim();
                        if rest == "(" {
                            in_use_block = true;
                        } else if !rest.is_empty() {
                            push_candidate(dir, rest);
                        }
                    } else if in_use_block {
                        push_candidate(dir, line);
                    }
                }
            }
        }
    }
    found
}
