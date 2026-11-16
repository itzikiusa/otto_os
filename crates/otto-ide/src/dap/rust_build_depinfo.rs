//! Cargo workspace root discovery, Makefile-style `.d` dep-info parser, and
//! source-tree fallback hashing for [`super::rust_build_cache`].

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use sha2::{Digest, Sha256};

#[allow(clippy::disallowed_methods)]
pub(super) fn find_workspace_root(
    canon_cwd: &Path,
    canon_target_root: &Path,
    allowed_roots: &[PathBuf],
) -> PathBuf {
    let within_allowed = |cand: &Path| -> bool {
        allowed_roots.is_empty() || allowed_roots.iter().any(|r| cand.starts_with(r))
    };
    let mut best = if within_allowed(canon_target_root) && canon_cwd.starts_with(canon_target_root)
    {
        canon_target_root.to_path_buf()
    } else {
        canon_cwd.to_path_buf()
    };
    let mut cur = Some(canon_cwd);
    while let Some(dir) = cur {
        if !within_allowed(dir) {
            break;
        }
        let manifest = dir.join("Cargo.toml");
        if manifest.is_file() {
            best = dir.to_path_buf();
            if fs::read_to_string(&manifest)
                .map(|txt| txt.lines().any(|l| l.trim().starts_with("[workspace]")))
                .unwrap_or(false)
            {
                return dir.to_path_buf();
            }
        }
        cur = dir.parent();
    }
    best
}

/// Parse Cargo/rustc Makefile-style `.d` dep-info content into prerequisite paths,
/// handling `\ ` escaped spaces, `\#`, `\:`, and `\` line continuations.
pub(super) fn parse_cargo_dep_info(raw: &str) -> Vec<PathBuf> {
    let joined = raw.replace("\\\r\n", " ").replace("\\\n", " ");
    let mut out = BTreeSet::new();
    for line in joined.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let bytes = trimmed.as_bytes();
        let mut colon_idx = None;
        let mut escaped = false;
        for (i, &b) in bytes.iter().enumerate() {
            if escaped {
                escaped = false;
                continue;
            }
            if b == b'\\' {
                escaped = true;
                continue;
            }
            if b == b':' && (i + 1 == bytes.len() || bytes[i + 1].is_ascii_whitespace()) {
                colon_idx = Some(i);
                break;
            }
        }
        let Some(sep) = colon_idx else {
            continue;
        };
        let rhs = trimmed[sep + 1..].trim();
        if rhs.is_empty() {
            continue;
        }
        let mut cur = String::new();
        let mut in_escape = false;
        for ch in rhs.chars() {
            if in_escape {
                cur.push(ch);
                in_escape = false;
            } else if ch == '\\' {
                in_escape = true;
            } else if ch.is_ascii_whitespace() {
                if !cur.is_empty() {
                    out.insert(PathBuf::from(std::mem::take(&mut cur)));
                }
            } else {
                cur.push(ch);
            }
        }
        if in_escape {
            cur.push('\\');
        }
        if !cur.is_empty() {
            out.insert(PathBuf::from(cur));
        }
    }
    out.into_iter().collect()
}

#[allow(clippy::disallowed_methods)]
pub(super) fn hash_dep_info_inputs(
    dep_file: &Path,
    ws_root: &Path,
    canon_cwd: &Path,
    within_allowed: &dyn Fn(&Path) -> bool,
    h: &mut Sha256,
) -> bool {
    let Ok(raw) = fs::read_to_string(dep_file) else {
        return false;
    };
    let deps = parse_cargo_dep_info(&raw);
    if deps.is_empty() {
        return false;
    }
    let mut dep_h = Sha256::new();
    for dep in deps {
        let resolved = if dep.is_absolute() {
            dep
        } else {
            let from_ws = ws_root.join(&dep);
            if from_ws.exists() {
                from_ws
            } else {
                canon_cwd.join(&dep)
            }
        };
        if !within_allowed(&resolved) {
            return false;
        }
        let Ok(meta) = fs::metadata(&resolved) else {
            // A listed input file was deleted or moved -> force cache miss / full walk.
            return false;
        };
        dep_h.update(b"\nD:");
        dep_h.update(resolved.to_string_lossy().as_bytes());
        dep_h.update(meta.len().to_le_bytes());
        if let Ok(mt) = meta
            .modified()
            .and_then(|t| t.duration_since(UNIX_EPOCH).map_err(std::io::Error::other))
        {
            dep_h.update(mt.as_secs().to_le_bytes());
            dep_h.update(mt.subsec_nanos().to_le_bytes());
        }
    }
    h.update(b"\nDEPINFO:");
    h.update(dep_h.finalize());
    true
}

#[allow(clippy::disallowed_methods)] // runs inside tokio::task::spawn_blocking
pub(super) fn hash_rust_tree(
    root: &Path,
    dir: &Path,
    within_allowed: &dyn Fn(&Path) -> bool,
    h: &mut Sha256,
) {
    if !within_allowed(dir) {
        return;
    }
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
            hash_rust_tree(root, &path, within_allowed, h);
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

#[allow(clippy::disallowed_methods)]
pub(super) fn hash_single_file(h: &mut Sha256, path: &Path) {
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
