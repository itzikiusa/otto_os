//! Go `$GOROOT`, local `replace`, workspace module, and `$GOMODCACHE` directory
//! resolution and symbol search helpers for [`super::go_mod_nav`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::types::IdeReferenceLocation;

use super::go_mod_nav::{escape_go_module_path, parse_go_mod_file, parse_go_mod_for_file};

/// Read the authoritative `package <name>` clause from the first non-test `.go` file in `dir`.
#[allow(clippy::disallowed_methods)]
pub(crate) fn read_declared_go_package_name(dir: &Path) -> Option<String> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.extension().and_then(|e| e.to_str()) == Some("go")
                && !p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("_test.go"))
        })
        .collect();
    files.sort();
    for file in files.into_iter().take(6) {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for raw in text.lines().take(50) {
            let line = raw.split("//").next().unwrap_or("").trim();
            if let Some(rest) = line.strip_prefix("package ") {
                let pkg = rest
                    .split(|c: char| c.is_whitespace() || c == ';')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !pkg.is_empty() && pkg != "main" {
                    return Some(pkg.to_string());
                }
            }
        }
    }
    None
}

/// Discover `$GOROOT/src` directories without spawning subprocesses.
#[allow(clippy::disallowed_methods)]
pub(crate) fn discover_goroot_src_dirs(
    readonly_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
) -> Vec<PathBuf> {
    let mut goroot_src_dirs = Vec::new();
    if let Some(gr) = env_map.get("GOROOT").filter(|s| !s.is_empty()) {
        let gp = PathBuf::from(gr);
        goroot_src_dirs.push(gp.join("src"));
        if gp.join("libexec/src/runtime").is_dir() {
            goroot_src_dirs.push(gp.join("libexec/src"));
        }
    }
    let path_str = env_map
        .get("PATH")
        .cloned()
        .or_else(|| std::env::var("PATH").ok())
        .unwrap_or_default();
    for seg in path_str.split(':').filter(|s| !s.is_empty()) {
        let go_bin = Path::new(seg).join("go");
        if go_bin.is_file() {
            if let Ok(canon_go) = go_bin.canonicalize() {
                if let Some(go_home) = canon_go.parent().and_then(Path::parent) {
                    if go_home.join("src/runtime").is_dir() {
                        goroot_src_dirs.push(go_home.join("src"));
                    }
                    if go_home.join("libexec/src/runtime").is_dir() {
                        goroot_src_dirs.push(go_home.join("libexec/src"));
                    }
                }
            }
        }
    }
    for rr in readonly_roots {
        if rr.join("src/runtime").is_dir() {
            goroot_src_dirs.push(rr.join("src"));
        } else if rr.join("libexec/src/runtime").is_dir() {
            goroot_src_dirs.push(rr.join("libexec/src"));
        } else if rr.ends_with("src") && rr.join("runtime").is_dir() {
            goroot_src_dirs.push(rr.clone());
        } else if rr.ends_with("Cellar/go") {
            if let Ok(rd) = std::fs::read_dir(rr) {
                for entry in rd.flatten() {
                    let libexec_src = entry.path().join("libexec/src");
                    if libexec_src.join("runtime").is_dir() {
                        goroot_src_dirs.push(libexec_src);
                    }
                    let direct_src = entry.path().join("src");
                    if direct_src.join("runtime").is_dir() {
                        goroot_src_dirs.push(direct_src);
                    }
                }
            }
        } else if rr.file_name().and_then(|n| n.to_str()) == Some("toolchains") {
            let go_tc = rr.join("go");
            if let Ok(rd) = std::fs::read_dir(go_tc) {
                for entry in rd.flatten() {
                    let src = entry.path().join("src");
                    if src.is_dir() {
                        goroot_src_dirs.push(src);
                    }
                }
            }
        }
    }
    goroot_src_dirs
}

/// Resolve a Go `import_path` across local `replace` dirs, workspace `go.mod` modules,
/// `$GOROOT/src`, and `$GOMODCACHE` (local `replace` ALWAYS wins over `$GOMODCACHE`).
#[allow(clippy::disallowed_methods)]
pub(crate) fn resolve_go_package_dirs(
    import_path: &str,
    origin_file: &Path,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let push_dir = |dirs: &mut Vec<PathBuf>, d: PathBuf| {
        if let Ok(c) = d.canonicalize() {
            if c.is_dir() && !dirs.contains(&c) {
                dirs.push(c);
            }
        } else if d.is_dir() && !dirs.contains(&d) {
            dirs.push(d);
        }
    };

    let mut mod_infos = Vec::new();
    if let Some(info) = parse_go_mod_for_file(origin_file, project_roots) {
        mod_infos.push(info);
    }
    for r in project_roots {
        if mod_infos.iter().all(|m| m.mod_root != *r) {
            if let Some(info) = parse_go_mod_file(r, &r.join("go.mod")) {
                mod_infos.push(info);
            }
        }
    }

    // 1. Local `replace` directives first (highest priority).
    let mut replaced_locally = false;
    for info in &mod_infos {
        let mut best_rep: Option<(&str, &PathBuf)> = None;
        for (rep_mod, rep_dir) in &info.local_replaces {
            if (import_path == rep_mod || import_path.starts_with(&format!("{rep_mod}/")))
                && best_rep.is_none_or(|(prev, _)| rep_mod.len() > prev.len())
            {
                best_rep = Some((rep_mod.as_str(), rep_dir));
            }
        }
        if let Some((rep_mod, rep_dir)) = best_rep {
            let cand = if import_path == rep_mod {
                rep_dir.clone()
            } else {
                rep_dir.join(&import_path[rep_mod.len() + 1..])
            };
            if cand.is_dir() {
                push_dir(&mut dirs, cand);
                replaced_locally = true;
            }
        }
    }

    // 2. Workspace `go.mod` modules across all attached project folders.
    for info in &mod_infos {
        if let Some(mod_name) = &info.module_path {
            if import_path == mod_name {
                push_dir(&mut dirs, info.mod_root.clone());
                replaced_locally = true;
            } else if let Some(sub) = import_path
                .strip_prefix(mod_name.as_str())
                .and_then(|s| s.strip_prefix('/'))
            {
                let cand = info.mod_root.join(sub);
                if cand.is_dir() {
                    push_dir(&mut dirs, cand);
                    replaced_locally = true;
                }
            }
        }
    }

    if replaced_locally && !dirs.is_empty() {
        return dirs;
    }

    // 3. `$GOROOT/src/<import_path>`.
    for src_root in discover_goroot_src_dirs(readonly_roots, env_map) {
        let cand = src_root.join(import_path);
        if cand.is_dir() {
            push_dir(&mut dirs, cand);
            break;
        }
    }

    // 4. `$GOMODCACHE` / `$GOPATH/pkg/mod` external versioned modules.
    let mut mod_caches = Vec::new();
    if let Some(mc) = env_map.get("GOMODCACHE").filter(|s| !s.is_empty()) {
        mod_caches.push(PathBuf::from(mc));
    }
    if let Some(gp) = env_map.get("GOPATH").filter(|s| !s.is_empty()) {
        for seg in gp.split(':').filter(|s| !s.is_empty()) {
            mod_caches.push(PathBuf::from(seg).join("pkg/mod"));
        }
    }
    for rr in readonly_roots {
        if rr.ends_with("pkg/mod") && rr.is_dir() {
            mod_caches.push(rr.clone());
        }
    }

    for cache in mod_caches {
        if !cache.is_dir() {
            continue;
        }
        for info in &mod_infos {
            for (req_mod, ver) in &info.requires {
                if import_path == req_mod || import_path.starts_with(&format!("{req_mod}/")) {
                    let escaped = escape_go_module_path(req_mod);
                    let mod_dir = cache.join(format!("{escaped}@{ver}"));
                    let target_dir = if import_path == req_mod {
                        mod_dir
                    } else {
                        mod_dir.join(&import_path[req_mod.len() + 1..])
                    };
                    if target_dir.is_dir() {
                        push_dir(&mut dirs, target_dir);
                    }
                }
            }
        }
        if dirs.is_empty() {
            if let Some(found) = find_gomodcache_prefix_dir(&cache, import_path) {
                push_dir(&mut dirs, found);
            }
        }
    }

    dirs
}

#[allow(clippy::disallowed_methods)]
fn find_gomodcache_prefix_dir(cache: &Path, import_path: &str) -> Option<PathBuf> {
    let segs: Vec<&str> = import_path.split('/').collect();
    for split_at in (1..=segs.len()).rev() {
        let mod_prefix = segs[..split_at].join("/");
        let escaped = escape_go_module_path(&mod_prefix);
        let escaped_path = Path::new(&escaped);
        let parent = match escaped_path.parent() {
            Some(p) if !p.as_os_str().is_empty() => cache.join(p),
            _ => cache.to_path_buf(),
        };
        let leaf = escaped_path.file_name()?.to_str()?;
        let want_prefix = format!("{leaf}@");
        let Ok(rd) = std::fs::read_dir(&parent) else {
            continue;
        };
        let mut matches: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&want_prefix))
            })
            .collect();
        matches.sort();
        if let Some(best_mod_dir) = matches.pop() {
            let sub = if split_at == segs.len() {
                best_mod_dir
            } else {
                best_mod_dir.join(segs[split_at..].join("/"))
            };
            if let Ok(canon) = sub.canonicalize() {
                if canon.is_dir() {
                    return Some(canon);
                }
            }
        }
    }
    None
}

/// Search Go `.go` files inside resolved external/imported package directories
/// for package-level `symbol` definitions (excluding `*_test.go` and function-local `:=`).
#[allow(clippy::disallowed_methods)]
pub(crate) fn search_go_package_dirs(dirs: &[PathBuf], symbol: &str) -> Vec<IdeReferenceLocation> {
    let mut out = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut files: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension().and_then(|e| e.to_str()) == Some("go")
                    && !p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.ends_with("_test.go"))
            })
            .collect();
        files.sort();
        for file in files {
            let Ok(canon) = file.canonicalize() else {
                continue;
            };
            if otto_core::secret_paths::is_denied_file(&canon)
                || otto_core::secret_paths::is_denied_dir(&canon)
            {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&canon) else {
                continue;
            };
            let path_str = canon.to_string_lossy().into_owned();
            out.extend(
                super::navigation_matchers::find_go_package_level_definitions(
                    &path_str, &text, symbol, true,
                ),
            );
            if out.len() >= 32 {
                return out;
            }
        }
    }
    out
}

/// Search sibling `.go` files in the SAME Go package directory (`pkg_dir`) for
/// package-level `var`, `const`, `type`, or `func` declarations.
/// Includes sibling `*_test.go` files when `origin_file` is itself `*_test.go`.
#[allow(clippy::disallowed_methods)]
pub(crate) fn search_same_dir_go_siblings(
    pkg_dir: &Path,
    origin_file: &Path,
    symbol: &str,
    allow_receiver_methods: bool,
) -> Vec<IdeReferenceLocation> {
    let Ok(rd) = std::fs::read_dir(pkg_dir) else {
        return Vec::new();
    };
    let origin_canon = origin_file
        .canonicalize()
        .unwrap_or_else(|_| origin_file.to_path_buf());
    let is_test_origin = origin_file
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with("_test.go"));

    let mut files: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            if p.extension().and_then(|e| e.to_str()) != Some("go") {
                return false;
            }
            let is_test = p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("_test.go"));
            is_test_origin || !is_test
        })
        .collect();
    files.sort();

    let mut out = Vec::new();
    for file in files {
        let Ok(canon) = file.canonicalize() else {
            continue;
        };
        if canon == origin_canon {
            continue;
        }
        if otto_core::secret_paths::is_denied_file(&canon)
            || otto_core::secret_paths::is_denied_dir(&canon)
        {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&canon) else {
            continue;
        };
        let path_str = canon.to_string_lossy().into_owned();
        out.extend(
            super::navigation_matchers::find_go_package_level_definitions(
                &path_str,
                &text,
                symbol,
                allow_receiver_methods,
            ),
        );
        if out.len() >= 32 {
            return out;
        }
    }
    out
}
