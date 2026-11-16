//! External Read-Only SDK & Dependency Navigation (`node_modules`, `.venv`/`site-packages`,
//! Rust `rustlib`/`.cargo`, plus Java & Go re-exports).

use std::path::{Path, PathBuf};

use crate::types::IdeReferenceLocation;

#[allow(unused_imports)]
pub(crate) use super::go_mod_nav::{
    escape_go_module_path, parse_go_imports, parse_go_mod_for_file, resolve_go_package_dirs,
    search_go_package_dirs,
};
use super::navigation::{find_definitions_in_content, read_confined_nav_file};
pub(super) use super::navigation_java::{
    candidate_java_fqns_in_file, resolve_external_java_definition,
};

/// Resolve JS/TS import specifier (`./foo` or `node_modules/<pkg>`) to source/type-definition
/// file(s), confining directory walks to `project_roots` when non-empty.
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
        push_js_candidates(&raw, &exts, project_roots, &mut out);
        return out;
    }

    let mut nm_dirs = Vec::new();
    let mut cur = origin_file.parent();
    while let Some(d) = cur {
        if !project_roots.is_empty() && !project_roots.iter().any(|r| d.starts_with(r)) {
            break;
        }
        let nm = d.join("node_modules");
        if nm.is_dir() && !nm_dirs.contains(&nm) {
            nm_dirs.push(nm);
        }
        if project_roots.iter().any(|r| r == d) {
            break;
        }
        cur = d.parent();
    }
    for r in project_roots {
        let nm = r.join("node_modules");
        if nm.is_dir() && !nm_dirs.contains(&nm) {
            nm_dirs.push(nm);
        }
    }

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
                push_js_candidates(&candidate_pkg_dir.join(sub), &exts, project_roots, &mut out);
                continue;
            }
            let pkg_json = candidate_pkg_dir.join("package.json");
            if let Ok(canon_pkg_json) = pkg_json.canonicalize() {
                if project_roots.is_empty()
                    || project_roots.iter().any(|r| canon_pkg_json.starts_with(r))
                {
                    if let Ok(raw_json) = std::fs::read_to_string(&canon_pkg_json) {
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw_json) {
                            for field in ["types", "typings", "module", "main"] {
                                if let Some(rel) = val.get(field).and_then(|v| v.as_str()) {
                                    push_js_candidates(
                                        &candidate_pkg_dir.join(rel),
                                        &exts,
                                        project_roots,
                                        &mut out,
                                    );
                                }
                            }
                            if let Some(exp) = val.get("exports") {
                                collect_package_exports_paths(
                                    &candidate_pkg_dir,
                                    exp,
                                    &exts,
                                    project_roots,
                                    &mut out,
                                );
                            }
                        }
                    }
                }
            }
            push_js_candidates(
                &candidate_pkg_dir.join("index"),
                &exts,
                project_roots,
                &mut out,
            );
        }
    }
    out
}

fn collect_package_exports_paths(
    pkg_dir: &Path,
    val: &serde_json::Value,
    exts: &[&str],
    project_roots: &[PathBuf],
    out: &mut Vec<PathBuf>,
) {
    match val {
        serde_json::Value::String(s) => {
            push_js_candidates(&pkg_dir.join(s), exts, project_roots, out)
        }
        serde_json::Value::Object(map) => {
            for key in [".", "types", "import", "default", "require"] {
                if let Some(sub) = map.get(key) {
                    collect_package_exports_paths(pkg_dir, sub, exts, project_roots, out);
                }
            }
        }
        _ => {}
    }
}

fn push_js_candidates(
    base: &Path,
    exts: &[&str],
    project_roots: &[PathBuf],
    out: &mut Vec<PathBuf>,
) {
    let mut try_push = |p: &Path| {
        if p.is_file() {
            if let Ok(c) = p.canonicalize() {
                let allowed =
                    project_roots.is_empty() || project_roots.iter().any(|r| c.starts_with(r));
                if allowed
                    && !otto_core::secret_paths::is_denied_file(&c)
                    && !otto_core::secret_paths::is_denied_dir(&c)
                    && !out.contains(&c)
                {
                    out.push(c);
                }
            }
        }
    };
    try_push(base);
    if base.extension().and_then(|e| e.to_str()) == Some("js") {
        let stem = base.with_extension("");
        for alt in ["ts", "tsx", "d.ts"] {
            try_push(&PathBuf::from(format!("{}.{alt}", stem.to_string_lossy())));
        }
    }
    for ext in exts {
        try_push(&PathBuf::from(format!("{}.{ext}", base.to_string_lossy())));
    }
    if base.is_dir() {
        for ext in exts {
            try_push(&base.join(format!("index.{ext}")));
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
    let allowed_roots: Vec<PathBuf> = project_roots
        .iter()
        .chain(readonly_roots.iter())
        .cloned()
        .collect();

    if leading_dots > 0 {
        let mut base = origin_file.parent().map(Path::to_path_buf);
        for _ in 1..leading_dots {
            base = base.and_then(|b| b.parent().map(Path::to_path_buf));
        }
        if let Some(dir) = base {
            if allowed_roots.is_empty() || allowed_roots.iter().any(|r| dir.starts_with(r)) {
                push_py_candidates(&dir, &rel_mod, &allowed_roots, &mut out);
            }
        }
        return out;
    }

    for root in project_roots {
        push_py_candidates(root, &rel_mod, &allowed_roots, &mut out);
        push_py_candidates(&root.join("src"), &rel_mod, &allowed_roots, &mut out);
    }
    for rr in readonly_roots {
        push_py_candidates(rr, &rel_mod, &allowed_roots, &mut out);
    }
    out
}

fn push_py_candidates(
    base: &Path,
    rel_mod: &str,
    allowed_roots: &[PathBuf],
    out: &mut Vec<PathBuf>,
) {
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
                let allowed =
                    allowed_roots.is_empty() || allowed_roots.iter().any(|r| c.starts_with(r));
                if allowed
                    && !otto_core::secret_paths::is_denied_file(&c)
                    && !otto_core::secret_paths::is_denied_dir(&c)
                    && !out.contains(&c)
                {
                    out.push(c);
                }
            }
        }
    }
}

/// Resolve external Rust `std`/`core`/`alloc` or `.cargo/registry` crate symbol definitions.
#[allow(clippy::disallowed_methods)]
pub(super) fn search_external_rust_crates(
    crate_hint: Option<&str>,
    symbol: &str,
    readonly_roots: &[PathBuf],
    is_root_user: bool,
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
        scan_dir_for_defs(
            &dir,
            "rs",
            "rust",
            symbol,
            readonly_roots,
            is_root_user,
            0,
            4,
            &mut out,
        );
        if out.len() >= 16 {
            break;
        }
    }
    out
}

#[allow(clippy::disallowed_methods)]
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

#[allow(clippy::too_many_arguments, clippy::disallowed_methods)]
fn scan_dir_for_defs(
    dir: &Path,
    ext: &str,
    lang: &str,
    symbol: &str,
    readonly_roots: &[PathBuf],
    is_root_user: bool,
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
            scan_dir_for_defs(
                &p,
                ext,
                lang,
                symbol,
                readonly_roots,
                is_root_user,
                depth + 1,
                max_depth,
                out,
            );
        } else if p.extension().and_then(|e| e.to_str()) == Some(ext) {
            if let Some((canon, text)) =
                read_confined_nav_file(&p, &[], readonly_roots, is_root_user)
            {
                if text.contains(symbol) {
                    let ps = canon.to_string_lossy().into_owned();
                    out.extend(find_definitions_in_content(&ps, lang, &text, symbol));
                }
            }
        }
        if out.len() >= 16 {
            return;
        }
    }
}
