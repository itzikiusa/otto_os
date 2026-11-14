//! Go Module, `go.work`, Local `replace`, `$GOROOT`, and `$GOMODCACHE` Resolution.
//!
//! Priority order (mandatory for Multi-Repo Go development):
//! 1. Local `replace` target directories in `go.work` and `go.mod` (editable workspace code).
//! 2. In-workspace `go.mod` / `go.work` `use` modules across all attached project folders.
//! 3. `$GOROOT/src` standard library.
//! 4. `$GOMODCACHE` / `$GOPATH/pkg/mod` external versioned modules.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::types::IdeReferenceLocation;

use super::navigation::find_definitions_in_content;

/// Escape uppercase letters in a Go module path per `$GOMODCACHE` layout (`GitHub` -> `!git!hub`).
pub(crate) fn escape_go_module_path(module_path: &str) -> String {
    let mut out = String::with_capacity(module_path.len() + 4);
    for ch in module_path.chars() {
        if ch.is_ascii_uppercase() {
            out.push('!');
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub(crate) struct GoModInfo {
    pub module_path: Option<String>,
    pub mod_root: PathBuf,
    /// `module_path -> version` (e.g. `"github.com/gin-gonic/gin" -> "v1.9.1"`).
    pub requires: BTreeMap<String, String>,
    /// `module_path -> local replacement dir` (canonicalized).
    pub local_replaces: BTreeMap<String, PathBuf>,
}

/// Locate and parse the nearest `go.mod` (plus any enclosing `go.work`) for `origin_file`.
#[allow(clippy::disallowed_methods)]
pub(crate) fn parse_go_mod_for_file(
    origin_file: &Path,
    project_roots: &[PathBuf],
) -> Option<GoModInfo> {
    let start_dir = if origin_file.is_dir() {
        Some(origin_file)
    } else {
        origin_file.parent()
    };
    let mut dir = start_dir;
    let mut found_mod = None;
    while let Some(d) = dir {
        let candidate = d.join("go.mod");
        if candidate.is_file() {
            found_mod = Some((d.to_path_buf(), candidate));
            break;
        }
        if project_roots.iter().any(|r| r == d) {
            break;
        }
        dir = d.parent();
    }
    if found_mod.is_none() {
        for r in project_roots {
            let candidate = r.join("go.mod");
            if candidate.is_file() {
                found_mod = Some((r.clone(), candidate));
                break;
            }
        }
    }
    let (mod_root, go_mod_file) = found_mod?;
    let mut info = parse_go_mod_file(&mod_root, &go_mod_file)?;

    // Layer any `go.work` `replace` / `use` directives (go.work overrides go.mod replaces).
    for work_file in find_go_work_files(&mod_root, project_roots) {
        if let Some(work_dir) = work_file.parent() {
            apply_go_work_replaces_and_uses(work_dir, &work_file, &mut info);
        }
    }
    Some(info)
}

#[allow(clippy::disallowed_methods)]
fn parse_go_mod_file(mod_root: &Path, go_mod_file: &Path) -> Option<GoModInfo> {
    let text = std::fs::read_to_string(go_mod_file).ok()?;
    let mut info = GoModInfo {
        mod_root: mod_root
            .canonicalize()
            .unwrap_or_else(|_| mod_root.to_path_buf()),
        ..Default::default()
    };

    let mut in_require = false;
    let mut in_replace = false;
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line == ")" {
            in_require = false;
            in_replace = false;
            continue;
        }
        if let Some(rest) = line.strip_prefix("module ") {
            info.module_path = Some(rest.trim().trim_matches('"').to_string());
            continue;
        }
        if line.starts_with("require") && line.ends_with('(') {
            in_require = true;
            continue;
        }
        if line.starts_with("replace") && line.ends_with('(') {
            in_replace = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("require ") {
            parse_go_require_spec(rest, &mut info.requires);
            continue;
        }
        if let Some(rest) = line.strip_prefix("replace ") {
            parse_go_replace_spec(rest, mod_root, &mut info.local_replaces);
            continue;
        }
        if in_require {
            parse_go_require_spec(line, &mut info.requires);
        } else if in_replace {
            parse_go_replace_spec(line, mod_root, &mut info.local_replaces);
        }
    }
    Some(info)
}

#[allow(clippy::disallowed_methods)]
fn find_go_work_files(mod_root: &Path, project_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut cur = Some(mod_root);
    let mut depth = 0usize;
    while let Some(d) = cur {
        let gw = d.join("go.work");
        if gw.is_file() && !out.contains(&gw) {
            out.push(gw);
        }
        depth += 1;
        if depth >= 4 {
            break;
        }
        cur = d.parent();
    }
    for r in project_roots {
        for cand in [
            r.join("go.work"),
            r.parent().map(|p| p.join("go.work")).unwrap_or_default(),
        ] {
            if !cand.as_os_str().is_empty() && cand.is_file() && !out.contains(&cand) {
                out.push(cand);
            }
        }
    }
    out
}

#[allow(clippy::disallowed_methods)]
fn apply_go_work_replaces_and_uses(work_dir: &Path, work_file: &Path, info: &mut GoModInfo) {
    let Ok(text) = std::fs::read_to_string(work_file) else {
        return;
    };
    let mut in_use = false;
    let mut in_replace = false;
    let mut use_dirs = Vec::new();
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line == ")" {
            in_use = false;
            in_replace = false;
            continue;
        }
        if line.starts_with("use") && line.ends_with('(') {
            in_use = true;
            continue;
        }
        if line.starts_with("replace") && line.ends_with('(') {
            in_replace = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("use ") {
            push_go_work_use_dir(work_dir, rest.trim(), &mut use_dirs);
            continue;
        }
        if let Some(rest) = line.strip_prefix("replace ") {
            parse_go_replace_spec(rest, work_dir, &mut info.local_replaces);
            continue;
        }
        if in_use {
            push_go_work_use_dir(work_dir, line, &mut use_dirs);
        } else if in_replace {
            parse_go_replace_spec(line, work_dir, &mut info.local_replaces);
        }
    }

    for ud in use_dirs {
        if let Some(use_mod) = parse_go_mod_file(&ud, &ud.join("go.mod")) {
            if let Some(mp) = use_mod.module_path {
                info.local_replaces.entry(mp).or_insert(use_mod.mod_root);
            }
        }
    }
}

fn push_go_work_use_dir(work_dir: &Path, raw: &str, out: &mut Vec<PathBuf>) {
    let tok = raw.trim().trim_matches('"').trim_matches('\'');
    if tok.is_empty() {
        return;
    }
    let p = if Path::new(tok).is_absolute() {
        PathBuf::from(tok)
    } else {
        work_dir.join(tok)
    };
    if let Ok(c) = p.canonicalize() {
        if c.is_dir() && !out.contains(&c) {
            out.push(c);
        }
    }
}

fn parse_go_require_spec(spec: &str, out: &mut BTreeMap<String, String>) {
    let mut parts = spec.split_whitespace();
    if let (Some(m), Some(v)) = (parts.next(), parts.next()) {
        out.insert(m.trim_matches('"').to_string(), v.to_string());
    }
}

fn parse_go_replace_spec(spec: &str, base_dir: &Path, out: &mut BTreeMap<String, PathBuf>) {
    let Some((left, right)) = spec.split_once("=>") else {
        return;
    };
    let old_mod = left
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"');
    let target = right
        .split_whitespace()
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"')
        .trim_matches('\'');
    if old_mod.is_empty() || target.is_empty() {
        return;
    }
    let resolved_opt = if let Some(rest) = target.strip_prefix("~/") {
        dirs::home_dir().map(|h| h.join(rest))
    } else if Path::new(target).is_absolute() {
        Some(PathBuf::from(target))
    } else if target.starts_with('.') || base_dir.join(target).join("go.mod").is_file() {
        Some(base_dir.join(target))
    } else {
        None
    };
    if let Some(resolved) = resolved_opt {
        if let Ok(canon) = resolved.canonicalize() {
            if canon.is_dir() {
                out.insert(old_mod.to_string(), canon);
            }
        }
    }
}

/// Parse `import` specs in a Go source file: `alias_or_pkg_name -> import_path`.
pub(crate) fn parse_go_imports(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut in_block = false;
    for raw in content.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("import") && line.ends_with('(') {
            in_block = true;
            continue;
        }
        if in_block {
            if line == ")" {
                in_block = false;
                continue;
            }
            insert_go_import_spec(line, &mut map);
            continue;
        }
        if let Some(rest) = line.strip_prefix("import ") {
            insert_go_import_spec(rest.trim(), &mut map);
        }
        if !in_block
            && (line.starts_with("func ") || line.starts_with("type ") || line.starts_with("var "))
        {
            break;
        }
    }
    map
}

fn insert_go_import_spec(spec: &str, out: &mut BTreeMap<String, String>) {
    let Some(q1) = spec.find('"') else {
        return;
    };
    let rest = &spec[q1 + 1..];
    let Some(q2) = rest.find('"') else {
        return;
    };
    let import_path = &rest[..q2];
    if import_path.is_empty() {
        return;
    }
    let prefix = spec[..q1].trim();
    let alias = if prefix.is_empty() || prefix == "_" || prefix == "." {
        let last = import_path.rsplit('/').next().unwrap_or(import_path);
        if last.starts_with('v') && last.len() <= 3 && last[1..].chars().all(|c| c.is_ascii_digit())
        {
            let segs: Vec<&str> = import_path.split('/').collect();
            if segs.len() >= 2 {
                out.insert(segs[segs.len() - 2].to_string(), import_path.to_string());
                out.insert(last.to_string(), import_path.to_string());
                return;
            }
        }
        last.to_string()
    } else {
        prefix.to_string()
    };
    out.insert(alias, import_path.to_string());
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
/// `$GOROOT/src`, and `$GOMODCACHE` (local `replace` ALWAYS wins over `$GOMODCACHE`!).
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

    // 1. Local `replace` directives FIRST (highest priority: overrides both workspace & $GOMODCACHE).
    let mut replaced_locally = false;
    for info in &mod_infos {
        // Match longest matching replace module prefix first.
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

    // If a local `replace` or workspace module matched, NEVER fall back to stale `$GOMODCACHE`!
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

/// Search Go `.go` files inside resolved package directories for `symbol` definitions.
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
            let Ok(text) = std::fs::read_to_string(&canon) else {
                continue;
            };
            let path_str = canon.to_string_lossy().into_owned();
            out.extend(find_definitions_in_content(&path_str, "go", &text, symbol));
            if out.len() >= 32 {
                return out;
            }
        }
    }
    out
}
