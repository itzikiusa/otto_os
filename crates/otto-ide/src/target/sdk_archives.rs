//! Virtual Java archive (`jar:file://...!/...`, `src.zip!/...`, `jdt://...`)
//! readers and JDK `src.zip` / Maven / Gradle source-jar discovery.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use otto_core::{Error, Result};
use tokio::process::Command;

use crate::types::IdeFileReadResp;

use super::fs_ops::build_file_read_resp;
use super::sdk_paths::{percent_decode, resolve_local_readable};
use super::MAX_FILE_READ_BYTES;

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

    let without_jar = trimmed
        .get(..4)
        .filter(|p| p.eq_ignore_ascii_case("jar:"))
        .map(|_| &trimmed[4..])
        .unwrap_or(trimmed);
    let (archive_raw, inner_with_qf) = split_archive_bang_slash(without_jar)
        .ok_or_else(|| Error::Invalid(format!("malformed archive URI '{trimmed}'")))?;
    let inner_raw = inner_with_qf
        .split(['?', '#'])
        .next()
        .unwrap_or(inner_with_qf);

    let inner = validate_archive_entry(inner_raw)?;
    let (canonical_archive, _) = resolve_local_readable(
        root,
        additional_roots,
        readonly_roots,
        is_root_user,
        archive_raw,
    )?;

    read_archive_member_or_javap(
        trimmed.to_string(),
        &canonical_archive,
        &inner,
        is_root_user,
    )
    .await
}

fn split_archive_bang_slash(s: &str) -> Option<(&str, &str)> {
    if let Some(pair) = s.split_once("!/") {
        return Some(pair);
    }
    if let Some(pair) = s.split_once("%21/") {
        return Some(pair);
    }
    let bytes = s.as_bytes();
    for i in 0..bytes.len().saturating_sub(5) {
        if bytes[i..i + 6].eq_ignore_ascii_case(b"%21%2f") {
            return Some((&s[..i], &s[i + 6..]));
        }
    }
    None
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
    let path_part = percent_decode(rest.split(['?', '#']).next().unwrap_or(rest));
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

    if !container.ends_with(".jar") {
        let mod_entry = validate_archive_entry(&format!("{container}/{rel_java}"))?;
        let flat_entry = validate_archive_entry(&rel_java)?;
        for src_zip in discover_jdk_src_zips_with_env(readonly_roots, None) {
            if let Ok(bytes) = extract_zip_entry(&src_zip, &mod_entry, is_root_user).await {
                return build_java_virtual_resp(original_uri.to_string(), &bytes);
            }
            if let Ok(bytes) = extract_zip_entry(&src_zip, &flat_entry, is_root_user).await {
                return build_java_virtual_resp(original_uri.to_string(), &bytes);
            }
        }
    }

    if let Some(jar_path) = find_jar_by_name(
        container,
        root,
        additional_roots,
        readonly_roots,
        is_root_user,
    ) {
        return read_archive_member_or_javap(
            original_uri.to_string(),
            &jar_path,
            &rel_class,
            is_root_user,
        )
        .await;
    }

    Err(Error::NotFound(format!(
        "Java source not found for '{original_uri}'"
    )))
}

async fn read_archive_member_or_javap(
    display_path: String,
    archive: &Path,
    entry: &str,
    is_root_user: bool,
) -> Result<IdeFileReadResp> {
    if entry.ends_with(".java") {
        if let Ok(bytes) = extract_zip_entry(archive, entry, is_root_user).await {
            return build_java_virtual_resp(display_path, &bytes);
        }
    }

    let java_entry = entry
        .strip_suffix(".class")
        .map(|s| {
            let outer = s.split('$').next().unwrap_or(s);
            format!("{outer}.java")
        })
        .unwrap_or_else(|| entry.to_string());
    if let Some(sources_jar) = locate_sibling_sources_jar(archive) {
        if let Ok(bytes) = extract_zip_entry(&sources_jar, &java_entry, is_root_user).await {
            return build_java_virtual_resp(display_path, &bytes);
        }
    }
    if let Ok(bytes) = extract_zip_entry(archive, &java_entry, is_root_user).await {
        return build_java_virtual_resp(display_path, &bytes);
    }

    let fqcn = java_entry
        .strip_suffix(".java")
        .unwrap_or(&java_entry)
        .replace('/', ".");
    if let Ok(sig) = run_javap_signature(archive, &fqcn, is_root_user).await {
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

fn apply_helper_env(cmd: &mut Command, is_root_user: bool) {
    if !is_root_user {
        cmd.env_clear();
        cmd.envs(crate::environments::shell_env::collect_safe_non_root_base_env());
    }
}

async fn extract_zip_entry(archive: &Path, entry: &str, is_root_user: bool) -> Result<Vec<u8>> {
    use tokio::io::AsyncReadExt as _;

    let escaped = escape_unzip_wildcards(entry);
    let mut cmd = Command::new("unzip");
    apply_helper_env(&mut cmd, is_root_user);
    cmd.arg("-p")
        .arg("--")
        .arg(archive)
        .arg(escaped)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Upstream(format!("unzip spawn failed: {e}")))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Upstream("unzip stdout missing".into()))?;
    let cap = MAX_FILE_READ_BYTES.saturating_add(1);

    let read_res = tokio::time::timeout(Duration::from_secs(5), async {
        let mut limited = stdout.take(cap);
        let mut buf = Vec::new();
        limited
            .read_to_end(&mut buf)
            .await
            .map_err(|e| Error::Upstream(format!("unzip read failed: {e}")))?;
        if buf.len() as u64 > MAX_FILE_READ_BYTES {
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Ok::<Option<Vec<u8>>, Error>(Some(buf));
        }
        let status = child
            .wait()
            .await
            .map_err(|e| Error::Upstream(format!("unzip wait failed: {e}")))?;
        if !status.success() || buf.is_empty() {
            return Ok(None);
        }
        Ok(Some(buf))
    })
    .await;

    match read_res {
        Ok(Ok(Some(buf))) => Ok(buf),
        Ok(Ok(None)) => Err(Error::NotFound(format!(
            "archive entry '{entry}' missing in '{}'",
            archive.display()
        ))),
        Ok(Err(e)) => Err(e),
        Err(_) => {
            let _ = child.kill().await;
            let _ = child.wait().await;
            Err(Error::Upstream("unzip timed out".into()))
        }
    }
}

async fn run_javap_signature(archive: &Path, fqcn: &str, is_root_user: bool) -> Result<String> {
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
    apply_helper_env(&mut cmd, is_root_user);
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

pub(crate) fn discover_jdk_src_zips(readonly_roots: &[PathBuf]) -> Vec<PathBuf> {
    discover_jdk_src_zips_with_env(readonly_roots, None)
}

#[allow(clippy::disallowed_methods)] // shallow 1-level toolchain root scan (<=16 entries)
pub(crate) fn discover_jdk_src_zips_with_env(
    readonly_roots: &[PathBuf],
    shell_env: Option<&BTreeMap<String, String>>,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut push_if_file = |p: PathBuf| {
        if p.is_file() && !out.contains(&p) {
            out.push(p);
        }
    };

    if let Some(jh) = std::env::var("JAVA_HOME")
        .ok()
        .or_else(|| shell_env.and_then(|m| m.get("JAVA_HOME").cloned()))
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
