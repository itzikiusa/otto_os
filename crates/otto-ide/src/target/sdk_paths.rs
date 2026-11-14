//! Allowlisted read-only external SDK / module-cache path resolution and
//! virtual Java archive (`jar:file://...!/...`, `src.zip!/...`, `jdt://...`)
//! readers for "Dive In" (`Cmd+Click` / `F12` / Step-Into).
//!
//! Security invariants:
//! - External paths are strictly read-only (`read_only: true`); every write or
//!   mutation path continues to reject anything outside writable project roots.
//! - Credential directories (`~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.kube`, …) and
//!   secret filenames (`.env`, `id_rsa`, `*.pem`, `*.key`, …) are ALWAYS denied
//!   on external paths, even when `is_root_user == true`.
//! - Zero subprocesses are spawned during path discovery (`<0.3ms` pure `stat`),
//!   preserving instant IDE load times.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use otto_core::{Error, Result};
use tokio::process::Command;

use crate::types::IdeFileReadResp;

use super::fs_ops::{build_file_read_resp, enforce_non_root_path};
use super::{map_io_err, MAX_FILE_READ_BYTES};

/// Decode a `file://` URI into a filesystem path string, handling percent-encoded
/// bytes such as `%20` (spaces) and `%40` (`@` in Go `$GOMODCACHE` module paths).
pub(crate) fn strip_file_uri(raw: &str) -> String {
    let trimmed = raw.trim();
    let rest = if let Some(r) = trimmed.strip_prefix("file://localhost/") {
        format!("/{r}")
    } else if let Some(r) = trimmed.strip_prefix("file:///") {
        format!("/{r}")
    } else if let Some(r) = trimmed.strip_prefix("file://") {
        r.to_string()
    } else {
        return trimmed.to_string();
    };
    percent_decode(&rest)
}

pub(crate) fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push((hi << 4) | lo);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// True when `path` is a virtual Java source/class reference (`jdt://...`,
/// `jar:file://...!/...`, or `/path/to/archive.(jar|zip)!/...`).
pub(crate) fn is_virtual_archive_uri(raw: &str) -> bool {
    let t = raw.trim();
    t.starts_with("jdt://") || t.starts_with("jar:") || t.contains("!/")
}

/// Collect canonicalized read-only external SDK & package-cache roots on the
/// local machine without spawning any subprocesses.
pub(crate) fn collect_readonly_sdk_roots(extra_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::with_capacity(48);
    candidates.extend(extra_dirs.iter().cloned());

    let shell_env = crate::environments::cached_shell_env_snapshot();
    let env_val = |key: &str| -> Option<String> {
        std::env::var(key)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .or_else(|| shell_env.get(key).cloned().filter(|v| !v.trim().is_empty()))
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
        // Never allowlist a credential directory (`~/.ssh`, `~/.aws`, `~/.gnupg`, …).
        if otto_core::secret_paths::is_denied_dir(&canon) {
            continue;
        }
        if !out.contains(&canon) {
            out.push(canon);
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

    // 1. Check primary root & attached project folders first.
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

    // 2. External path: strictly enforce secret-file & credential-dir blocks
    //    EVEN for `is_root_user == true`, and require membership in `readonly_roots`.
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

    // Re-check dynamic env roots in case `GOROOT`/`JAVA_HOME` was updated after resolve.
    let in_allowlist = readonly_roots.iter().any(|r| canonical.starts_with(r))
        || collect_readonly_sdk_roots(&[])
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

/// Read a Java source/class entry from `jar:file:///...!/...`, `.../src.zip!/...`,
/// or `jdt://contents/...` as a read-only `java` buffer.
pub(crate) async fn read_virtual_java_archive(
    root: &Path,
    additional_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    is_root_user: bool,
    raw_uri: &str,
) -> Result<IdeFileReadResp> {
    let trimmed = raw_uri.trim();
    if let Some(rest) = trimmed.strip_prefix("jdt://") {
        return read_jdt_uri(
            root,
            additional_roots,
            readonly_roots,
            is_root_user,
            trimmed,
            rest,
        )
        .await;
    }

    let without_jar = trimmed.strip_prefix("jar:").unwrap_or(trimmed);
    let without_query = without_jar.split('?').next().unwrap_or(without_jar);
    let (archive_raw, inner_raw) = without_query
        .split_once("!/")
        .ok_or_else(|| Error::Invalid(format!("malformed archive URI '{trimmed}'")))?;

    let inner = validate_archive_entry(inner_raw)?;
    let (canonical_archive, _) = resolve_local_readable(
        root,
        additional_roots,
        readonly_roots,
        is_root_user,
        archive_raw,
    )?;

    read_archive_member_or_javap(trimmed.to_string(), &canonical_archive, &inner).await
}

fn validate_archive_entry(raw: &str) -> Result<String> {
    let decoded = percent_decode(raw).trim_start_matches('/').to_string();
    if decoded.is_empty() || decoded.chars().any(char::is_control) {
        return Err(Error::Invalid("invalid archive entry path".into()));
    }
    if Path::new(&decoded)
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(Error::Forbidden(
            "archive entry traversal is forbidden".into(),
        ));
    }
    Ok(decoded)
}

async fn read_jdt_uri(
    root: &Path,
    additional_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    is_root_user: bool,
    original_uri: &str,
    rest: &str,
) -> Result<IdeFileReadResp> {
    // Format: `contents/<jar_or_module>/<package>/<Class>.class?...`
    let path_part = percent_decode(rest.split('?').next().unwrap_or(rest));
    let stripped = path_part
        .strip_prefix("contents/")
        .unwrap_or(path_part.as_str());
    let mut segs = stripped.splitn(3, '/');
    let container = segs
        .next()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| Error::Invalid(format!("malformed jdt URI '{original_uri}'")))?;
    let pkg = segs.next().unwrap_or("");
    let file_part = segs.next().unwrap_or("");

    let pkg_slash = pkg.replace('.', "/");
    let stem = file_part
        .strip_suffix(".class")
        .or_else(|| file_part.strip_suffix(".java"))
        .unwrap_or(file_part);
    let outer_stem = stem.split('$').next().unwrap_or(stem);
    let rel_java = if pkg_slash.is_empty() {
        format!("{outer_stem}.java")
    } else {
        format!("{pkg_slash}/{outer_stem}.java")
    };
    let rel_class = if pkg_slash.is_empty() {
        format!("{outer_stem}.class")
    } else {
        format!("{pkg_slash}/{outer_stem}.class")
    };

    // 1. JDK module (`java.base`, `java.sql`, `jdk.compiler`, …) -> `lib/src.zip`
    if !container.ends_with(".jar") {
        let mod_entry = validate_archive_entry(&format!("{container}/{rel_java}"))?;
        let flat_entry = validate_archive_entry(&rel_java)?;
        for src_zip in discover_jdk_src_zips(readonly_roots) {
            if let Ok(bytes) = extract_zip_entry(&src_zip, &mod_entry).await {
                return build_java_virtual_resp(original_uri.to_string(), &bytes);
            }
            if let Ok(bytes) = extract_zip_entry(&src_zip, &flat_entry).await {
                return build_java_virtual_resp(original_uri.to_string(), &bytes);
            }
        }
    }

    // 2. Maven / Gradle / project `.jar` container
    if let Some(jar_path) = find_jar_by_name(
        container,
        root,
        additional_roots,
        readonly_roots,
        is_root_user,
    ) {
        return read_archive_member_or_javap(original_uri.to_string(), &jar_path, &rel_class).await;
    }

    Err(Error::NotFound(format!(
        "Java source not found for '{original_uri}'"
    )))
}

async fn read_archive_member_or_javap(
    display_path: String,
    archive: &Path,
    entry: &str,
) -> Result<IdeFileReadResp> {
    // 1. Direct entry extraction (works for `src.zip`, `*-sources.jar`, or source-bearing jars).
    if entry.ends_with(".java") {
        if let Ok(bytes) = extract_zip_entry(archive, entry).await {
            return build_java_virtual_resp(display_path, &bytes);
        }
    }

    // 2. If `archive` is `foo-1.0.jar`, probe sibling/Gradle `foo-1.0-sources.jar` for `.java`.
    let java_entry = entry
        .strip_suffix(".class")
        .map(|s| {
            let outer = s.split('$').next().unwrap_or(s);
            format!("{outer}.java")
        })
        .unwrap_or_else(|| entry.to_string());
    if let Some(sources_jar) = locate_sibling_sources_jar(archive) {
        if let Ok(bytes) = extract_zip_entry(&sources_jar, &java_entry).await {
            return build_java_virtual_resp(display_path, &bytes);
        }
    }
    if let Ok(bytes) = extract_zip_entry(archive, &java_entry).await {
        return build_java_virtual_resp(display_path, &bytes);
    }

    // 3. Fallback: decompile class signature via `javap -p` so drill-down ALWAYS opens readable code.
    let fqcn = java_entry
        .strip_suffix(".java")
        .unwrap_or(&java_entry)
        .replace('/', ".");
    if let Ok(sig) = run_javap_signature(archive, &fqcn).await {
        return build_java_virtual_resp(display_path, sig.as_bytes());
    }

    Err(Error::NotFound(format!(
        "entry '{entry}' not found in '{}'",
        archive.display()
    )))
}

fn build_java_virtual_resp(display_path: String, bytes: &[u8]) -> Result<IdeFileReadResp> {
    let truncated = bytes.len() as u64 > MAX_FILE_READ_BYTES;
    let slice = if truncated {
        &bytes[..MAX_FILE_READ_BYTES as usize]
    } else {
        bytes
    };
    let mut resp = build_file_read_resp(display_path, slice, bytes.len() as u64, true, truncated)?;
    resp.language = "java".into();
    resp.read_only = true;
    Ok(resp)
}

fn escape_unzip_wildcards(entry: &str) -> String {
    let mut out = String::with_capacity(entry.len() + 4);
    for ch in entry.chars() {
        if matches!(ch, '[' | ']' | '*' | '?' | '\\') {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

async fn extract_zip_entry(archive: &Path, entry: &str) -> Result<Vec<u8>> {
    let escaped = escape_unzip_wildcards(entry);
    let mut cmd = Command::new("unzip");
    cmd.arg("-p")
        .arg("--")
        .arg(archive)
        .arg(escaped)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let out = tokio::time::timeout(Duration::from_secs(5), cmd.output())
        .await
        .map_err(|_| Error::Upstream("unzip timed out".into()))?
        .map_err(|e| Error::Upstream(format!("unzip failed: {e}")))?;

    if !out.status.success() || out.stdout.is_empty() {
        return Err(Error::NotFound(format!(
            "archive entry '{entry}' missing in '{}'",
            archive.display()
        )));
    }
    Ok(out.stdout)
}

async fn run_javap_signature(archive: &Path, fqcn: &str) -> Result<String> {
    if fqcn.is_empty()
        || !fqcn
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '$'))
    {
        return Err(Error::Invalid("invalid Java class name".into()));
    }
    let javap_bin = std::env::var("JAVA_HOME")
        .ok()
        .map(|h| PathBuf::from(h).join("bin/javap"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("javap"));

    let mut cmd = Command::new(javap_bin);
    cmd.args(["-p", "-cp"])
        .arg(archive)
        .arg(fqcn)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let out = tokio::time::timeout(Duration::from_secs(5), cmd.output())
        .await
        .map_err(|_| Error::Upstream("javap timed out".into()))?
        .map_err(|e| Error::Upstream(format!("javap failed: {e}")))?;
    if !out.status.success() || out.stdout.is_empty() {
        return Err(Error::NotFound(format!("class '{fqcn}' not found")));
    }
    let body = String::from_utf8_lossy(&out.stdout);
    Ok(format!(
        "// Read-only class signature from {} (sources jar not installed)\n{body}",
        archive.display()
    ))
}

#[allow(clippy::disallowed_methods)] // shallow 1-level Gradle hash dir scan (<16 entries)
fn locate_sibling_sources_jar(jar: &Path) -> Option<PathBuf> {
    let name = jar.file_name()?.to_str()?;
    let stem = name.strip_suffix(".jar")?;
    if stem.ends_with("-sources") {
        return Some(jar.to_path_buf());
    }
    let sources_name = format!("{stem}-sources.jar");
    let direct = jar.with_file_name(&sources_name);
    if direct.is_file() {
        return Some(direct);
    }
    // Gradle cache layout: `<version>/<hash>/<artifact>-<version>.jar` -> sibling `<other_hash>/<artifact>-<version>-sources.jar`
    let ver_dir = jar.parent()?.parent()?;
    let entries = std::fs::read_dir(ver_dir).ok()?;
    for entry in entries.flatten() {
        let cand = entry.path().join(&sources_name);
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

#[allow(clippy::disallowed_methods)] // shallow 1-level toolchain root scan (<=16 entries)
pub(crate) fn discover_jdk_src_zips(readonly_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_if_file = |p: PathBuf| {
        if p.is_file() && !out.contains(&p) {
            out.push(p);
        }
    };

    let shell_env = crate::environments::cached_shell_env_snapshot();
    if let Some(jh) = std::env::var("JAVA_HOME")
        .ok()
        .or_else(|| shell_env.get("JAVA_HOME").cloned())
    {
        let p = PathBuf::from(jh);
        push_if_file(p.join("lib/src.zip"));
        push_if_file(p.join("src.zip"));
        push_if_file(p.join("Contents/Home/lib/src.zip"));
    }

    for r in readonly_roots {
        push_if_file(r.join("lib/src.zip"));
        push_if_file(r.join("Contents/Home/lib/src.zip"));
        push_if_file(r.join("libexec/openjdk.jdk/Contents/Home/lib/src.zip"));
        if let Ok(rd) = std::fs::read_dir(r) {
            for e in rd.flatten().take(16) {
                let p = e.path();
                push_if_file(p.join("lib/src.zip"));
                push_if_file(p.join("Contents/Home/lib/src.zip"));
                push_if_file(p.join("libexec/openjdk.jdk/Contents/Home/lib/src.zip"));
            }
        }
    }

    for fixed in [
        "/opt/homebrew/opt/openjdk/libexec/openjdk.jdk/Contents/Home/lib/src.zip",
        "/usr/local/opt/openjdk/libexec/openjdk.jdk/Contents/Home/lib/src.zip",
    ] {
        push_if_file(PathBuf::from(fixed));
    }
    out
}

fn find_jar_by_name(
    jar_name: &str,
    root: &Path,
    additional_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    is_root_user: bool,
) -> Option<PathBuf> {
    if !jar_name.ends_with(".jar") || jar_name.contains('/') || jar_name.contains('\\') {
        return None;
    }
    let mut search_bases: Vec<PathBuf> = vec![root.to_path_buf()];
    search_bases.extend(additional_roots.iter().cloned());
    if let Some(home) = dirs::home_dir() {
        search_bases.push(home.join(".m2/repository"));
        search_bases.push(home.join(".gradle/caches/modules-2/files-2.1"));
    }
    for base in search_bases {
        if !base.is_dir() {
            continue;
        }
        let walker = ignore::WalkBuilder::new(&base)
            .max_depth(Some(8))
            .git_ignore(false)
            .build();
        for entry in walker.flatten().take(4_000) {
            if entry.file_name().to_str() == Some(jar_name) && entry.path().is_file() {
                if let Ok((canon, _)) = resolve_local_readable(
                    root,
                    additional_roots,
                    readonly_roots,
                    is_root_user,
                    &entry.path().to_string_lossy(),
                ) {
                    return Some(canon);
                }
            }
        }
    }
    None
}

/// Discover local Go `replace` (`go.mod` / `go.work`) and `use` (`go.work`) target
/// directories referenced by `primary_root` or `additional_roots`, returning
/// canonicalized writable workspace directories (`<0.5ms`, zero subprocesses).
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
    if let Some(parent) = primary_root.parent() {
        if parent.join("go.work").is_file() && !inspect_dirs.iter().any(|d| d == parent) {
            inspect_dirs.push(parent.to_path_buf());
        }
    }

    let mut push_candidate = |base: &Path, raw_target: &str| {
        let trimmed = raw_target.trim().trim_matches('"').trim_matches('\'');
        if trimmed.is_empty() {
            return;
        }
        // Only local path targets (`./...`, `../...`, `/...`, `~/...`, or existing relative dir)
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
