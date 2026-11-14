//! External Read-Only SDK & Dependency Navigation (`node_modules`, `.venv`/`site-packages`,
//! Rust `rustlib`/`.cargo`, Java Maven/Gradle `-sources.jar`/`.jar` & JDK `src.zip`, plus Go
//! module/replace re-exports from `go_mod_nav`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::java_workspace::{
    read_jar_classes_and_packages, resolve_external_dep_jar, resolve_m2_repo, scan_java_workspace,
};
use crate::types::{IdeReferenceLocation, IdeReferencesResp};

#[allow(unused_imports)]
pub(crate) use super::go_mod_nav::{
    escape_go_module_path, parse_go_imports, parse_go_mod_for_file, resolve_go_package_dirs,
    search_go_package_dirs,
};
use super::navigation::find_definitions_in_content;
use super::symbols_data::{detect_csv_delimiter, split_delimited_row};

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
#[allow(clippy::disallowed_methods)]
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

#[allow(clippy::disallowed_methods)]
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

/// Extract candidate Java FQNs (`com.foo.Bar`) for `target_class` from `package` + `import` lines.
pub(super) fn candidate_java_fqns_in_file(content: &str, target_class: &str) -> Vec<String> {
    let mut fqns = Vec::new();
    let mut pkg_name = None;
    for raw in content.lines() {
        let t = raw.split("//").next().unwrap_or("").trim();
        if let Some(p) = t.strip_prefix("package ").and_then(|s| s.strip_suffix(';')) {
            pkg_name = Some(p.trim().to_string());
        } else if let Some(imp) = t.strip_prefix("import ").and_then(|s| s.strip_suffix(';')) {
            let imp = imp.trim().trim_start_matches("static ").trim();
            if imp.ends_with(&format!(".{target_class}")) {
                fqns.push(imp.to_string());
            } else if let Some(pkg) = imp.strip_suffix(".*") {
                fqns.push(format!("{pkg}.{target_class}"));
            }
        }
    }
    if let Some(p) = pkg_name.filter(|p| !p.is_empty()) {
        let same_pkg = format!("{p}.{target_class}");
        if !fqns.contains(&same_pkg) {
            fqns.push(same_pkg);
        }
    }
    if target_class
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_uppercase())
    {
        let jl = format!("java.lang.{target_class}");
        if !fqns.contains(&jl) {
            fqns.push(jl);
        }
    }
    fqns
}

/// Resolve external Java Maven/Gradle `*-sources.jar`/`.jar` or JDK `src.zip` class/method
/// definition, returning a virtual `jar:file:///...!/...` path readable via `TargetManager::read_file`.
pub(super) fn resolve_external_java_definition(
    content: &str,
    symbol: &str,
    qualifier: Option<&str>,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
) -> Vec<IdeReferenceLocation> {
    let target_class = qualifier.unwrap_or(symbol);
    let candidate_fqns = candidate_java_fqns_in_file(content, target_class);
    if candidate_fqns.is_empty() {
        return Vec::new();
    }

    // 1. Check external Maven / Gradle dependency `-sources.jar` (and `.jar` binary fallback)
    //    ONLY for dependencies NOT already present as local workspace modules.
    let ws = scan_java_workspace(project_roots);
    let m2_repo = resolve_m2_repo(env_map);
    for module in &ws.modules {
        for dep in &module.dependencies {
            if ws.find_dep(&dep.group_id, &dep.artifact_id).is_some() {
                continue;
            }
            if let Some(src_jar) = resolve_external_dep_jar(dep, m2_repo.as_deref(), true) {
                if let Ok(canon_jar) = src_jar.canonicalize() {
                    for fqn in &candidate_fqns {
                        let entry = format!("{}.java", fqn.replace('.', "/"));
                        if let Some(src_text) = unzip_single_entry(&canon_jar, &entry) {
                            let uri =
                                format!("jar:file://{}!/{entry}", canon_jar.to_string_lossy());
                            let defs = find_definitions_in_content(&uri, "java", &src_text, symbol);
                            if !defs.is_empty() {
                                return defs;
                            }
                            return vec![IdeReferenceLocation {
                                path: uri,
                                line: 1,
                                column: 1,
                                preview: format!("class {target_class}"),
                            }];
                        }
                    }
                }
            }
            if let Some(bin_jar) = resolve_external_dep_jar(dep, m2_repo.as_deref(), false) {
                if let Ok(canon_jar) = bin_jar.canonicalize() {
                    let (classes, _) = read_jar_classes_and_packages(&canon_jar);
                    for fqn in &candidate_fqns {
                        if classes.contains(fqn) {
                            let entry = format!("{}.class", fqn.replace('.', "/"));
                            return vec![IdeReferenceLocation {
                                path: format!(
                                    "jar:file://{}!/{entry}",
                                    canon_jar.to_string_lossy()
                                ),
                                line: 1,
                                column: 1,
                                preview: format!("class {fqn}"),
                            }];
                        }
                    }
                }
            }
        }
    }

    // 2. Check JDK `src.zip`.
    let mut src_zips = Vec::new();
    if let Some(jh) = env_map.get("JAVA_HOME").filter(|s| !s.is_empty()) {
        let p = PathBuf::from(jh);
        for rel in ["lib/src.zip", "src.zip", "Contents/Home/lib/src.zip"] {
            if p.join(rel).is_file() {
                src_zips.push(p.join(rel));
            }
        }
    }
    for z in crate::target::sdk_paths::discover_jdk_src_zips(readonly_roots) {
        if !src_zips.contains(&z) {
            src_zips.push(z);
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

#[allow(clippy::disallowed_methods)]
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

pub(super) fn find_json_key_definitions(
    path: &str,
    content: &str,
    keys: &[&str],
) -> Vec<IdeReferenceLocation> {
    let Some(&target_key) = keys.last() else {
        return Vec::new();
    };
    let needle = format!("\"{target_key}\"");
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if let Some(pos) = line.find(&needle) {
            if line[pos + needle.len()..].trim_start().starts_with(':') {
                out.push(IdeReferenceLocation {
                    path: path.to_string(),
                    line: (idx + 1) as u32,
                    column: line[..pos].chars().count() as u32 + 2,
                    preview: line.trim().to_string(),
                });
            }
        }
    }
    out
}

pub(super) fn navigate_csv_column(
    path: &str,
    lang: &str,
    content: &str,
    line: u32,
    column: u32,
) -> Option<IdeReferencesResp> {
    let mut non_empty = content
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty());
    let (header_idx, header_line) = non_empty.next()?;
    let delim = detect_csv_delimiter(path, lang, header_line);
    let headers = split_delimited_row(header_line, delim);
    let cur_line = content.lines().nth(line.saturating_sub(1) as usize)?;
    let col_idx = column_index_at_char_offset(cur_line, delim, column.saturating_sub(1) as usize);
    let col_name = headers.get(col_idx)?.trim().to_string();
    let header_char_col = char_offset_of_column(header_line, delim, col_idx) as u32 + 1;
    Some(IdeReferencesResp {
        symbol: if col_name.is_empty() {
            format!("Column {}", col_idx + 1)
        } else {
            col_name.clone()
        },
        locations: vec![IdeReferenceLocation {
            path: path.to_string(),
            line: (header_idx + 1) as u32,
            column: header_char_col,
            preview: format!("Column #{}: {}", col_idx + 1, col_name),
        }],
    })
}

fn column_index_at_char_offset(line: &str, delim: char, target_char: usize) -> usize {
    let mut in_q = false;
    let mut col = 0usize;
    for (i, ch) in line.chars().enumerate() {
        if i >= target_char {
            break;
        }
        if ch == '"' {
            in_q = !in_q;
        } else if ch == delim && !in_q {
            col += 1;
        }
    }
    col
}

fn char_offset_of_column(line: &str, delim: char, want_col: usize) -> usize {
    if want_col == 0 {
        return 0;
    }
    let mut in_q = false;
    let mut col = 0usize;
    for (i, ch) in line.chars().enumerate() {
        if ch == '"' {
            in_q = !in_q;
        } else if ch == delim && !in_q {
            col += 1;
            if col == want_col {
                return i + 1;
            }
        }
    }
    0
}

pub(super) fn extract_quoted_string_at(line: &str, column: u32) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let target = (column.saturating_sub(1) as usize).min(chars.len() - 1);
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i], '"' | '\'' | '`') {
            let q = chars[i];
            let start = i;
            i += 1;
            let mut val = String::new();
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    val.push(chars[i + 1]);
                    i += 2;
                } else if chars[i] == q {
                    if (start..=i).contains(&target) {
                        return Some(val);
                    }
                    i += 1;
                    break;
                } else {
                    val.push(chars[i]);
                    i += 1;
                }
            }
            continue;
        }
        i += 1;
    }
    None
}

pub(super) fn extract_ref_value(line: &str) -> Option<String> {
    let after = line.split("\"$ref\"").nth(1)?;
    let q1 = after.find('"')?;
    let rest = &after[q1 + 1..];
    let q2 = rest.find('"')?;
    Some(rest[..q2].to_string())
}
