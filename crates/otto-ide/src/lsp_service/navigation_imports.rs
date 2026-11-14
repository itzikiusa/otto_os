//! Cross-module & External SDK import resolution helpers for `goto_definition`
//! across Go, Java/Kotlin (in-project Maven/Gradle modules FIRST!), JS/TS, Python,
//! Rust, and Shell.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::java_workspace::scan_java_workspace;
use crate::types::IdeReferenceLocation;

use super::navigation::find_definitions_in_content;
use super::navigation_ext::{
    candidate_java_fqns_in_file, parse_go_imports, resolve_go_package_dirs,
    resolve_js_ts_module_files, resolve_python_module_files, search_external_rust_crates,
    search_go_package_dirs,
};

#[allow(clippy::too_many_arguments, clippy::disallowed_methods)]
pub(super) fn resolve_local_language_imports(
    lang: &str,
    content: &str,
    symbol: &str,
    qualifier: Option<&str>,
    origin_abs: &Path,
    primary_root: &Path,
    keep_abs: bool,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
) -> Vec<IdeReferenceLocation> {
    let mut out = Vec::new();
    match lang {
        "java" | "kotlin" => {
            // In-project Maven/Gradle modules (`project_roots`) ALWAYS resolve from local
            // `.java`/`.kt` source files FIRST ahead of `~/.m2` JARs or JDK `src.zip`!
            let ws = scan_java_workspace(project_roots);
            let target_class = qualifier.unwrap_or(symbol);
            let candidate_fqns = candidate_java_fqns_in_file(content, target_class);
            for src_root in ws.all_source_roots(true) {
                for fqn in &candidate_fqns {
                    for ext in ["java", "kt"] {
                        let cand = src_root.join(format!("{}.{ext}", fqn.replace('.', "/")));
                        let Ok(canon) = cand.canonicalize() else {
                            continue;
                        };
                        if !canon.is_file() {
                            continue;
                        }
                        let Ok(text) = std::fs::read_to_string(&canon) else {
                            continue;
                        };
                        let p = format_result_path(&canon, primary_root, keep_abs);
                        let defs = find_definitions_in_content(&p, lang, &text, symbol);
                        if !defs.is_empty() {
                            for d in defs {
                                push_unique_loc(&mut out, d);
                            }
                        } else {
                            for d in find_definitions_in_content(&p, lang, &text, target_class) {
                                push_unique_loc(&mut out, d);
                            }
                        }
                    }
                }
            }
        }
        "go" => {
            let imports = parse_go_imports(content);
            let matched_import = qualifier
                .and_then(|q| imports.get(q))
                .or_else(|| imports.get(symbol));
            if let Some(import_path) = matched_import {
                let pkg_dirs = resolve_go_package_dirs(
                    import_path,
                    origin_abs,
                    project_roots,
                    readonly_roots,
                    env_map,
                );
                if qualifier.is_some() {
                    for mut loc in search_go_package_dirs(&pkg_dirs, symbol) {
                        loc.path = format_result_path(Path::new(&loc.path), primary_root, keep_abs);
                        push_unique_loc(&mut out, loc);
                    }
                } else if let Some(dir) = pkg_dirs.first() {
                    if let Ok(rd) = std::fs::read_dir(dir) {
                        let mut go_files: Vec<PathBuf> = rd
                            .flatten()
                            .map(|e| e.path())
                            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("go"))
                            .collect();
                        go_files.sort();
                        if let Some(first_go) = go_files.first().and_then(|p| p.canonicalize().ok())
                        {
                            out.push(IdeReferenceLocation {
                                path: format_result_path(&first_go, primary_root, keep_abs),
                                line: 1,
                                column: 1,
                                preview: format!("package {symbol}"),
                            });
                        }
                    }
                }
            }
        }
        "javascript" | "typescript" | "svelte" => {
            for (imported_orig, specifier) in
                find_js_import_sources_for_symbol(content, symbol, qualifier)
            {
                let target_sym = imported_orig.as_deref().unwrap_or(symbol);
                for file in resolve_js_ts_module_files(&specifier, origin_abs, project_roots) {
                    if let Ok(text) = std::fs::read_to_string(&file) {
                        let p = format_result_path(&file, primary_root, keep_abs);
                        let defs = find_definitions_in_content(&p, "typescript", &text, target_sym);
                        if !defs.is_empty() {
                            for d in defs {
                                push_unique_loc(&mut out, d);
                            }
                        } else {
                            for re_spec in extract_js_reexports(&text, target_sym) {
                                for sub in
                                    resolve_js_ts_module_files(&re_spec, &file, project_roots)
                                {
                                    if let Ok(sub_text) = std::fs::read_to_string(&sub) {
                                        let sp = format_result_path(&sub, primary_root, keep_abs);
                                        for d in find_definitions_in_content(
                                            &sp,
                                            "typescript",
                                            &sub_text,
                                            target_sym,
                                        ) {
                                            push_unique_loc(&mut out, d);
                                        }
                                    }
                                }
                            }
                        }
                        if out.is_empty() {
                            out.push(IdeReferenceLocation {
                                path: p,
                                line: 1,
                                column: 1,
                                preview: text.lines().next().unwrap_or("").trim().to_string(),
                            });
                        }
                    }
                }
            }
        }
        "python" => {
            for (orig_sym, mod_spec) in find_python_import_sources(content, symbol, qualifier) {
                let target_sym = orig_sym.as_deref().unwrap_or(symbol);
                for file in resolve_python_module_files(
                    &mod_spec,
                    origin_abs,
                    project_roots,
                    readonly_roots,
                ) {
                    if let Ok(text) = std::fs::read_to_string(&file) {
                        let p = format_result_path(&file, primary_root, keep_abs);
                        let defs = find_definitions_in_content(&p, "python", &text, target_sym);
                        if !defs.is_empty() {
                            for d in defs {
                                push_unique_loc(&mut out, d);
                            }
                        } else if orig_sym.is_none() {
                            out.push(IdeReferenceLocation {
                                path: p,
                                line: 1,
                                column: 1,
                                preview: text.lines().next().unwrap_or("").trim().to_string(),
                            });
                        }
                    }
                }
            }
        }
        "rust" => {
            if let Some(q) = qualifier {
                if matches!(q, "std" | "core" | "alloc") {
                    out.extend(search_external_rust_crates(Some(q), symbol, readonly_roots));
                }
            }
            if out.is_empty() {
                if let Some(crate_name) = find_rust_use_crate(content, symbol, qualifier) {
                    out.extend(search_external_rust_crates(
                        Some(&crate_name),
                        symbol,
                        readonly_roots,
                    ));
                }
            }
        }
        _ => {}
    }
    out
}

pub(super) fn find_js_import_sources_for_symbol(
    content: &str,
    symbol: &str,
    qualifier: Option<&str>,
) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    let lookup = qualifier.unwrap_or(symbol);
    for raw in content.lines() {
        let t = raw.trim();
        if !t.starts_with("import ") && !t.starts_with("export ") && !t.contains("require(") {
            continue;
        }
        let Some(spec) = extract_last_quoted(t) else {
            continue;
        };
        if qualifier.is_some()
            && (t.contains(&format!("* as {lookup}")) || t.contains(&format!("import {lookup}")))
        {
            out.push((Some(symbol.to_string()), spec));
            continue;
        }
        if let Some(brace_s) = t.find('{') {
            if let Some(brace_e) = t[brace_s..].find('}') {
                for item in t[brace_s + 1..brace_s + brace_e].split(',') {
                    let clean = item.trim().trim_start_matches("type ").trim();
                    if let Some((orig, alias)) = clean.split_once(" as ") {
                        if alias.trim() == lookup {
                            out.push((Some(orig.trim().to_string()), spec.clone()));
                        }
                    } else if clean == lookup {
                        out.push((Some(clean.to_string()), spec.clone()));
                    }
                }
            }
        } else if t.contains(&format!("import {lookup}")) || t.contains(&format!("const {lookup}"))
        {
            out.push((Some(symbol.to_string()), spec));
        }
    }
    out
}

pub(super) fn extract_js_reexports(content: &str, symbol: &str) -> Vec<String> {
    let mut specs = Vec::new();
    for raw in content.lines() {
        let t = raw.trim();
        if t.starts_with("export *") || (t.starts_with("export ") && t.contains(symbol)) {
            if let Some(s) = extract_last_quoted(t) {
                specs.push(s);
            }
        }
    }
    specs
}

pub(super) fn extract_last_quoted(line: &str) -> Option<String> {
    for q in ['\'', '"'] {
        if let Some(end) = line.rfind(q) {
            if let Some(start) = line[..end].rfind(q) {
                return Some(line[start + 1..end].to_string());
            }
        }
    }
    None
}

pub(super) fn find_python_import_sources(
    content: &str,
    symbol: &str,
    qualifier: Option<&str>,
) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    let lookup = qualifier.unwrap_or(symbol);
    for raw in content.lines() {
        let t = raw.split('#').next().unwrap_or("").trim();
        if let Some(rest) = t.strip_prefix("from ") {
            if let Some((mod_part, imp_part)) = rest.split_once(" import ") {
                let mod_spec = mod_part.trim().to_string();
                for item in imp_part.trim_matches(['(', ')']).split(',') {
                    let clean = item.trim();
                    if let Some((orig, alias)) = clean.split_once(" as ") {
                        if alias.trim() == lookup {
                            out.push((
                                Some(
                                    if qualifier.is_some() {
                                        symbol
                                    } else {
                                        orig.trim()
                                    }
                                    .to_string(),
                                ),
                                mod_spec.clone(),
                            ));
                        }
                    } else if clean == lookup {
                        out.push((Some(symbol.to_string()), mod_spec.clone()));
                    }
                }
            }
        } else if let Some(rest) = t.strip_prefix("import ") {
            for item in rest.split(',') {
                let clean = item.trim();
                let (mod_spec, alias) = clean
                    .split_once(" as ")
                    .map(|(m, a)| (m.trim(), a.trim()))
                    .unwrap_or((clean, clean.rsplit('.').next().unwrap_or(clean)));
                if alias == lookup {
                    out.push((qualifier.map(|_| symbol.to_string()), mod_spec.to_string()));
                }
            }
        }
    }
    out
}

pub(super) fn find_rust_use_crate(
    content: &str,
    symbol: &str,
    qualifier: Option<&str>,
) -> Option<String> {
    if let Some(q) = qualifier {
        if !matches!(q, "self" | "super" | "crate" | "Self") {
            return Some(q.to_string());
        }
    }
    for raw in content.lines() {
        let t = raw.trim();
        if let Some(rest) = t.strip_prefix("use ") {
            if rest.contains(symbol) {
                let root_seg = rest.split("::").next()?.trim();
                if !matches!(root_seg, "self" | "super" | "crate") {
                    return Some(root_seg.to_string());
                }
            }
        }
    }
    None
}

pub(super) fn collect_shell_sourced_files(
    content: &str,
    origin_abs: &Path,
    project_roots: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for raw in content.lines() {
        let t = raw.trim();
        if let Some(rest) = t.strip_prefix("source ").or_else(|| t.strip_prefix(". ")) {
            if let Some(p) = resolve_shell_source_target(rest, origin_abs, project_roots) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    out
}

pub(super) fn resolve_shell_source_target(
    raw_arg: &str,
    origin_abs: &Path,
    project_roots: &[PathBuf],
) -> Option<PathBuf> {
    let token = raw_arg
        .split('#')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .next()?
        .trim_matches(['"', '\'']);
    let base_dir = origin_abs.parent()?;
    let dir_str = base_dir.to_string_lossy();
    let mut cleaned = token.to_string();
    for prefix in [
        "${BASH_SOURCE%/*}",
        "$(dirname \"$0\")",
        "$(dirname $0)",
        "${SCRIPT_DIR}",
        "$SCRIPT_DIR",
        "${DIR}",
        "$DIR",
    ] {
        if cleaned.starts_with(prefix) {
            cleaned = format!("{dir_str}{}", &cleaned[prefix.len()..]);
        }
    }
    let cand = if Path::new(&cleaned).is_absolute() {
        PathBuf::from(&cleaned)
    } else {
        base_dir.join(&cleaned)
    };
    if let Ok(c) = cand.canonicalize() {
        if c.is_file() {
            return Some(c);
        }
    }
    for r in project_roots {
        if let Ok(c) = r.join(&cleaned).canonicalize() {
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

pub(super) fn resolve_origin_abs(
    path: &str,
    project_roots: &[PathBuf],
    primary_root: &Path,
) -> PathBuf {
    let clean = crate::target::sdk_paths::strip_file_uri(path);
    let p = Path::new(&clean);
    if p.is_absolute() {
        return p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    }
    for r in project_roots {
        let cand = r.join(&clean);
        if let Ok(c) = cand.canonicalize() {
            return c;
        }
    }
    primary_root.join(&clean)
}

pub(super) fn format_result_path(canon: &Path, primary_root: &Path, keep_abs: bool) -> String {
    if !keep_abs {
        if let Ok(rel) = canon.strip_prefix(primary_root) {
            return rel.to_string_lossy().replace('\\', "/");
        }
    }
    canon.to_string_lossy().into_owned()
}

pub(super) fn push_unique_loc(out: &mut Vec<IdeReferenceLocation>, loc: IdeReferenceLocation) {
    let mut seen: BTreeSet<(&str, u32)> = out.iter().map(|x| (x.path.as_str(), x.line)).collect();
    if seen.insert((loc.path.as_str(), loc.line)) {
        out.push(loc);
    }
}
