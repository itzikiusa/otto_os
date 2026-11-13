//! External Read-Only SDK & Dependency Navigation (`$GOROOT`, `$GOMODCACHE`,
//! `node_modules`, `.venv`/`site-packages`, Rust `rustlib`/`.cargo`, Java `src.zip`/`-sources.jar`).
//!
//! Zero retained caches, bounded directory walks, and canonicalized read-only paths
//! compatible with `TargetManager::read_file`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::types::IdeReferenceLocation;

use super::navigation::find_definitions_in_content;

/// Escape uppercase letters in a Go module path per `$GOMODCACHE` layout (`GitHub` -> `!git!hub`).
pub(super) fn escape_go_module_path(module_path: &str) -> String {
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

#[derive(Default)]
pub(super) struct GoModInfo {
    pub module_path: Option<String>,
    pub mod_root: PathBuf,
    /// `module_path -> version` (e.g. `"github.com/gin-gonic/gin" -> "v1.9.1"`).
    pub requires: BTreeMap<String, String>,
    /// `module_path -> local replacement dir`.
    pub local_replaces: BTreeMap<String, PathBuf>,
}

pub(super) fn parse_go_mod_for_file(
    origin_file: &Path,
    project_roots: &[PathBuf],
) -> Option<GoModInfo> {
    let mut dir = origin_file.parent();
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
    let text = std::fs::read_to_string(&go_mod_file).ok()?;
    let mut info = GoModInfo {
        mod_root: mod_root.clone(),
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
        if line.starts_with("require (") || line == "require(" {
            in_require = true;
            continue;
        }
        if line.starts_with("replace (") || line == "replace(" {
            in_replace = true;
            continue;
        }
        if let Some(rest) = line.strip_prefix("require ") {
            parse_go_require_spec(rest, &mut info.requires);
            continue;
        }
        if let Some(rest) = line.strip_prefix("replace ") {
            parse_go_replace_spec(rest, &mod_root, &mut info.local_replaces);
            continue;
        }
        if in_require {
            parse_go_require_spec(line, &mut info.requires);
        } else if in_replace {
            parse_go_replace_spec(line, &mod_root, &mut info.local_replaces);
        }
    }
    Some(info)
}

fn parse_go_require_spec(spec: &str, out: &mut BTreeMap<String, String>) {
    let mut parts = spec.split_whitespace();
    if let (Some(m), Some(v)) = (parts.next(), parts.next()) {
        out.insert(m.trim_matches('"').to_string(), v.to_string());
    }
}

fn parse_go_replace_spec(spec: &str, mod_root: &Path, out: &mut BTreeMap<String, PathBuf>) {
    let Some((left, right)) = spec.split_once("=>") else {
        return;
    };
    let old_mod = left.split_whitespace().next().unwrap_or("").trim();
    let target = right.split_whitespace().next().unwrap_or("").trim();
    if old_mod.is_empty() || target.is_empty() {
        return;
    }
    if target.starts_with('.') || target.starts_with('/') {
        let resolved = if Path::new(target).is_absolute() {
            PathBuf::from(target)
        } else {
            mod_root.join(target)
        };
        if let Ok(canon) = resolved.canonicalize() {
            out.insert(old_mod.to_string(), canon);
        }
    }
}

/// Parse `import` specs in a Go source file: `alias_or_pkg_name -> import_path`.
pub(super) fn parse_go_imports(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let mut in_block = false;
    for raw in content.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with("import (") || line == "import(" {
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
        // Handle `.../v2` major-version suffixes when package name is preceding segment.
        if last.starts_with('v') && last.len() <= 3 && last[1..].chars().all(|c| c.is_ascii_digit())
        {
            let segs: Vec<&str> = import_path.split('/').collect();
            if segs.len() >= 2 {
                map_insert_both(out, segs[segs.len() - 2], last, import_path);
                return;
            }
        }
        last.to_string()
    } else {
        prefix.to_string()
    };
    out.insert(alias, import_path.to_string());
}

fn map_insert_both(out: &mut BTreeMap<String, String>, a: &str, b: &str, import_path: &str) {
    out.insert(a.to_string(), import_path.to_string());
    out.insert(b.to_string(), import_path.to_string());
}

/// Resolve a Go `import_path` across workspace `go.mod` modules, `$GOROOT/src`, and `$GOMODCACHE`.
#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub(super) fn resolve_go_package_dirs(
    import_path: &str,
    origin_file: &Path,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // 1. Workspace `go.mod` module(s) & `replace` directives across all attached roots.
    let mut mod_infos = Vec::new();
    if let Some(info) = parse_go_mod_for_file(origin_file, project_roots) {
        mod_infos.push(info);
    }
    for r in project_roots {
        if mod_infos.iter().all(|m| m.mod_root != *r) {
            if let Some(info) = parse_go_mod_for_file(&r.join("go.mod"), project_roots) {
                mod_infos.push(info);
            }
        }
    }

    for info in &mod_infos {
        if let Some(mod_name) = &info.module_path {
            if import_path == mod_name {
                dirs.push(info.mod_root.clone());
            } else if let Some(sub) = import_path
                .strip_prefix(mod_name.as_str())
                .and_then(|s| s.strip_prefix('/'))
            {
                let cand = info.mod_root.join(sub);
                if cand.is_dir() {
                    dirs.push(cand);
                }
            }
        }
        for (rep_mod, rep_dir) in &info.local_replaces {
            if import_path == rep_mod {
                dirs.push(rep_dir.clone());
            } else if let Some(sub) = import_path
                .strip_prefix(rep_mod.as_str())
                .and_then(|s| s.strip_prefix('/'))
            {
                let cand = rep_dir.join(sub);
                if cand.is_dir() {
                    dirs.push(cand);
                }
            }
        }
    }

    // 2. `$GOROOT/src/<import_path>` (standard library, e.g. `fmt`, `net/http`, `context`).
    let mut goroot_src_dirs = Vec::new();
    if let Some(gr) = env_map.get("GOROOT").filter(|s| !s.is_empty()) {
        let gp = PathBuf::from(gr);
        goroot_src_dirs.push(gp.join("src"));
        if gp.join("libexec/src/runtime").is_dir() {
            goroot_src_dirs.push(gp.join("libexec/src"));
        }
    }
    // Zero-subprocess `PATH` `go` binary canonicalization (resolves macOS Homebrew
    // `/opt/homebrew/bin/go` -> `.../Cellar/go/<ver>/libexec/bin/go` -> `.../libexec/src`
    // and `/usr/local/go/bin/go` -> `/usr/local/go/src` when `GOROOT` is unset).
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
    for src_root in goroot_src_dirs {
        let cand = src_root.join(import_path);
        if cand.is_dir() {
            if let Ok(canon) = cand.canonicalize() {
                if !dirs.contains(&canon) {
                    dirs.push(canon);
                }
            }
        }
    }

    // 3. `$GOMODCACHE` / `$GOPATH/pkg/mod` / `~/go/pkg/mod` external modules.
    let mut mod_caches = Vec::new();
    if let Some(mc) = env_map.get("GOMODCACHE").filter(|s| !s.is_empty()) {
        mod_caches.push(PathBuf::from(mc));
    }
    if let Some(gp) = env_map.get("GOPATH").filter(|s| !s.is_empty()) {
        for seg in gp.split(':') {
            if !seg.is_empty() {
                mod_caches.push(PathBuf::from(seg).join("pkg/mod"));
            }
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
        // Match exact `require` version from `go.mod` first (`<escaped_mod>@<ver>/<subpath>`).
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
                    if let Ok(canon) = target_dir.canonicalize() {
                        if canon.is_dir() && !dirs.contains(&canon) {
                            dirs.push(canon);
                        }
                    }
                }
            }
        }
        // Fallback when transitive dependency isn't directly listed in root `go.mod`:
        // walk prefixes of `import_path` looking for `<prefix>@*` in `pkg/mod`.
        if dirs.is_empty() {
            if let Some(found) = find_gomodcache_prefix_dir(&cache, import_path) {
                if !dirs.contains(&found) {
                    dirs.push(found);
                }
            }
        }
    }

    dirs
}

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
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
#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub(super) fn search_go_package_dirs(dirs: &[PathBuf], symbol: &str) -> Vec<IdeReferenceLocation> {
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

/// Resolve JS/TS import specifier (`./foo` or `node_modules/<pkg>`) to source/type-definition file(s).
pub(super) fn resolve_js_ts_module_files(
    specifier: &str,
    origin_file: &Path,
    project_roots: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let exts = [
        "ts", "tsx", "d.ts", "mts", "cts", "js", "jsx", "mjs", "json",
    ];

    if specifier.starts_with('.') {
        let Some(base_dir) = origin_file.parent() else {
            return out;
        };
        let raw = base_dir.join(specifier);
        push_js_candidates(&raw, &exts, &mut out);
        return out;
    }

    // Walk up from `origin_file` and across `project_roots` looking for `node_modules/<specifier>`.
    let mut nm_dirs = Vec::new();
    let mut cur = origin_file.parent();
    while let Some(d) = cur {
        let nm = d.join("node_modules");
        if nm.is_dir() {
            nm_dirs.push(nm);
        }
        cur = d.parent();
    }
    for r in project_roots {
        let nm = r.join("node_modules");
        if nm.is_dir() && !nm_dirs.contains(&nm) {
            nm_dirs.push(nm);
        }
    }

    // Split `@scope/pkg/sub` vs `pkg/sub`.
    let (pkg_name, subpath) = if specifier.starts_with('@') {
        let mut parts = specifier.splitn(3, '/');
        let s = parts.next().unwrap_or("");
        let p = parts.next().unwrap_or("");
        let rest = parts.next();
        (format!("{s}/{p}"), rest)
    } else {
        match specifier.split_once('/') {
            Some((p, rest)) => (p.to_string(), Some(rest)),
            None => (specifier.to_string(), None),
        }
    };

    for nm in &nm_dirs {
        let pkg_dir = nm.join(&pkg_name);
        // Also check `@types/<pkg>` for DefinitelyTyped packages!
        let types_pkg_name = pkg_name.trim_start_matches('@').replace('/', "__");
        let at_types_dir = nm.join("@types").join(types_pkg_name);

        for candidate_pkg_dir in [Some(pkg_dir), at_types_dir.is_dir().then_some(at_types_dir)]
            .into_iter()
            .flatten()
        {
            if !candidate_pkg_dir.is_dir() {
                continue;
            }
            if let Some(sub) = subpath {
                push_js_candidates(&candidate_pkg_dir.join(sub), &exts, &mut out);
                continue;
            }
            let pkg_json = candidate_pkg_dir.join("package.json");
            if let Ok(raw_json) = std::fs::read_to_string(&pkg_json) {
                if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw_json) {
                    for field in ["types", "typings", "module", "main"] {
                        if let Some(rel) = val.get(field).and_then(|v| v.as_str()) {
                            push_js_candidates(&candidate_pkg_dir.join(rel), &exts, &mut out);
                        }
                    }
                    if let Some(exp) = val.get("exports") {
                        collect_package_exports_paths(&candidate_pkg_dir, exp, &exts, &mut out);
                    }
                }
            }
            push_js_candidates(&candidate_pkg_dir.join("index"), &exts, &mut out);
        }
    }
    out
}

fn collect_package_exports_paths(
    pkg_dir: &Path,
    val: &serde_json::Value,
    exts: &[&str],
    out: &mut Vec<PathBuf>,
) {
    match val {
        serde_json::Value::String(s) => push_js_candidates(&pkg_dir.join(s), exts, out),
        serde_json::Value::Object(map) => {
            for key in [".", "types", "import", "default", "require"] {
                if let Some(sub) = map.get(key) {
                    collect_package_exports_paths(pkg_dir, sub, exts, out);
                }
            }
        }
        _ => {}
    }
}

fn push_js_candidates(base: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    if base.is_file() {
        if let Ok(c) = base.canonicalize() {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    // Strip `.js` -> `.ts`/`.d.ts` for TypeScript ESM imports (`import { x } from './foo.js'`).
    if base.extension().and_then(|e| e.to_str()) == Some("js") {
        let stem = base.with_extension("");
        for alt in ["ts", "tsx", "d.ts"] {
            let p = PathBuf::from(format!("{}.{alt}", stem.to_string_lossy()));
            if p.is_file() {
                if let Ok(c) = p.canonicalize() {
                    if !out.contains(&c) {
                        out.push(c);
                    }
                }
            }
        }
    }
    for ext in exts {
        let p = PathBuf::from(format!("{}.{ext}", base.to_string_lossy()));
        if p.is_file() {
            if let Ok(c) = p.canonicalize() {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
    }
    if base.is_dir() {
        for ext in exts {
            let p = base.join(format!("index.{ext}"));
            if p.is_file() {
                if let Ok(c) = p.canonicalize() {
                    if !out.contains(&c) {
                        out.push(c);
                    }
                }
            }
        }
    }
}

/// Resolve Python module (`from pkg.sub import Foo` or `import pkg`) across workspace, `.venv`, and stdlib.
pub(super) fn resolve_python_module_files(
    module_spec: &str,
    origin_file: &Path,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let leading_dots = module_spec.chars().take_while(|&c| c == '.').count();
    let rel_mod = module_spec[leading_dots..].replace('.', "/");

    if leading_dots > 0 {
        let mut base = origin_file.parent().map(Path::to_path_buf);
        for _ in 1..leading_dots {
            base = base.and_then(|b| b.parent().map(Path::to_path_buf));
        }
        if let Some(dir) = base {
            push_py_candidates(&dir, &rel_mod, &mut out);
        }
        return out;
    }

    for root in project_roots {
        push_py_candidates(root, &rel_mod, &mut out);
        push_py_candidates(&root.join("src"), &rel_mod, &mut out);
    }
    for rr in readonly_roots {
        push_py_candidates(rr, &rel_mod, &mut out);
    }
    out
}

fn push_py_candidates(base: &Path, rel_mod: &str, out: &mut Vec<PathBuf>) {
    let target = if rel_mod.is_empty() {
        base.to_path_buf()
    } else {
        base.join(rel_mod)
    };
    for cand in [
        PathBuf::from(format!("{}.py", target.to_string_lossy())),
        PathBuf::from(format!("{}.pyi", target.to_string_lossy())),
        target.join("__init__.py"),
        target.join("__init__.pyi"),
    ] {
        if cand.is_file() {
            if let Ok(c) = cand.canonicalize() {
                if !out.contains(&c) {
                    out.push(c);
                }
            }
        }
    }
}

/// Resolve external Rust `std`/`core`/`alloc` or `.cargo/registry` crate symbol definitions.
#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub(super) fn search_external_rust_crates(
    crate_hint: Option<&str>,
    symbol: &str,
    readonly_roots: &[PathBuf],
) -> Vec<IdeReferenceLocation> {
    let mut search_dirs = Vec::new();
    for rr in readonly_roots {
        let s = rr.to_string_lossy();
        if s.contains("rustlib") || s.contains("toolchains") {
            let std_lib = if rr.join("library").is_dir() {
                Some(rr.join("library"))
            } else {
                find_subdir_bounded(rr, "library", 5)
            };
            if let Some(lib_dir) = std_lib {
                for std_crate in ["std", "core", "alloc"] {
                    if crate_hint.is_none_or(|h| h == std_crate) {
                        let d = lib_dir.join(std_crate).join("src");
                        if d.is_dir() {
                            search_dirs.push(d);
                        }
                    }
                }
            }
        } else if s.contains(".cargo/registry") || s.ends_with("registry/src") {
            let src_root = if rr.ends_with("src") {
                rr.clone()
            } else {
                rr.join("src")
            };
            if let Some(crate_name) = crate_hint {
                let want = format!("{}-", crate_name.replace('_', "-"));
                let want_under = format!("{crate_name}-");
                if let Ok(idx_rd) = std::fs::read_dir(&src_root) {
                    for idx_entry in idx_rd.flatten() {
                        if let Ok(crate_rd) = std::fs::read_dir(idx_entry.path()) {
                            for ce in crate_rd.flatten() {
                                let name = ce.file_name().to_string_lossy().into_owned();
                                if name.starts_with(&want) || name.starts_with(&want_under) {
                                    let d = ce.path().join("src");
                                    if d.is_dir() {
                                        search_dirs.push(d);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut out = Vec::new();
    for dir in search_dirs {
        scan_dir_for_defs(&dir, "rs", "rust", symbol, 0, 4, &mut out);
        if out.len() >= 16 {
            break;
        }
    }
    out
}

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
fn find_subdir_bounded(root: &Path, target_name: &str, max_depth: usize) -> Option<PathBuf> {
    if max_depth == 0 {
        return None;
    }
    let rd = std::fs::read_dir(root).ok()?;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            if p.file_name().and_then(|n| n.to_str()) == Some(target_name) {
                return Some(p);
            }
            if let Some(found) = find_subdir_bounded(&p, target_name, max_depth - 1) {
                return Some(found);
            }
        }
    }
    None
}

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
fn scan_dir_for_defs(
    dir: &Path,
    ext: &str,
    lang: &str,
    symbol: &str,
    depth: usize,
    max_depth: usize,
    out: &mut Vec<IdeReferenceLocation>,
) {
    if depth > max_depth || out.len() >= 16 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let p = entry.path();
        if p.is_dir() {
            scan_dir_for_defs(&p, ext, lang, symbol, depth + 1, max_depth, out);
        } else if p.extension().and_then(|e| e.to_str()) == Some(ext) {
            if let Ok(text) = std::fs::read_to_string(&p) {
                if text.contains(symbol) {
                    if let Ok(canon) = p.canonicalize() {
                        let ps = canon.to_string_lossy().into_owned();
                        out.extend(find_definitions_in_content(&ps, lang, &text, symbol));
                    }
                }
            }
        }
        if out.len() >= 16 {
            return;
        }
    }
}

/// Resolve external Java JDK `src.zip` or Maven/Gradle `*-sources.jar` class/method definition,
/// returning a virtual `jar:file:///...!/...` path readable via `TargetManager::read_file`.
pub(super) fn resolve_external_java_definition(
    content: &str,
    symbol: &str,
    qualifier: Option<&str>,
    readonly_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
) -> Vec<IdeReferenceLocation> {
    let target_class = qualifier.unwrap_or(symbol);
    let mut candidate_fqns = Vec::new();
    for raw in content.lines() {
        let t = raw.trim();
        if let Some(imp) = t.strip_prefix("import ").and_then(|s| s.strip_suffix(';')) {
            let imp = imp.trim().trim_start_matches("static ").trim();
            if imp.ends_with(&format!(".{target_class}")) {
                candidate_fqns.push(imp.to_string());
            } else if let Some(pkg) = imp.strip_suffix(".*") {
                candidate_fqns.push(format!("{pkg}.{target_class}"));
            }
        }
    }
    if target_class
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
    {
        candidate_fqns.push(format!("java.lang.{target_class}"));
    }
    if candidate_fqns.is_empty() {
        return Vec::new();
    }

    let mut src_zips = Vec::new();
    if let Some(jh) = env_map.get("JAVA_HOME").filter(|s| !s.is_empty()) {
        let p = PathBuf::from(jh);
        for rel in ["lib/src.zip", "src.zip"] {
            if p.join(rel).is_file() {
                src_zips.push(p.join(rel));
            }
        }
    }
    for rr in readonly_roots {
        for rel in ["lib/src.zip", "src.zip", "Contents/Home/lib/src.zip"] {
            let z = rr.join(rel);
            if z.is_file() && !src_zips.contains(&z) {
                src_zips.push(z);
            }
        }
    }

    let mut out = Vec::new();
    for fqn in &candidate_fqns {
        let rel_java = format!("{}.java", fqn.replace('.', "/"));
        for zip in &src_zips {
            let Ok(canon_zip) = zip.canonicalize() else {
                continue;
            };
            for entry in [
                format!("java.base/{rel_java}"),
                format!("java.logging/{rel_java}"),
                format!("java.sql/{rel_java}"),
                format!("java.net.http/{rel_java}"),
                rel_java.clone(),
            ] {
                if let Some(src_text) = unzip_single_entry(&canon_zip, &entry) {
                    let uri = format!("jar:file://{}!/{entry}", canon_zip.to_string_lossy());
                    let defs = find_definitions_in_content(&uri, "java", &src_text, symbol);
                    if !defs.is_empty() {
                        return defs;
                    }
                    out.push(IdeReferenceLocation {
                        path: uri,
                        line: 1,
                        column: 1,
                        preview: format!("class {target_class}"),
                    });
                    return out;
                }
            }
        }
    }
    out
}

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
fn unzip_single_entry(archive: &Path, entry: &str) -> Option<String> {
    let out = Command::new("/usr/bin/unzip")
        .arg("-p")
        .arg(archive)
        .arg(entry)
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}
