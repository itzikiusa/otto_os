//! Go syntax, unused-import, and unimported-qualifier diagnostics with Go module
//! `/vN` major-version, `.vN`, hyphenated repo, and on-disk `package <name>` resolution.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime};

use regex::Regex;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::types::{IdeDiagnostic, IdeQuickFix, IdeTextEdit};

use super::code_mask_for_line;
use super::go_mod_nav::{
    candidate_go_import_identifiers, discover_goroot_src_dirs, parse_go_imports,
    parse_go_mod_for_file, read_declared_go_package_name, resolve_go_package_dirs,
};
use super::java_diag::make_delete_line_edit;

const COMMON_GO_STDLIB: &[(&str, &str)] = &[
    ("atomic", "sync/atomic"),
    ("bufio", "bufio"),
    ("bytes", "bytes"),
    ("context", "context"),
    ("csv", "encoding/csv"),
    ("errors", "errors"),
    ("filepath", "path/filepath"),
    ("fmt", "fmt"),
    ("hex", "encoding/hex"),
    ("http", "net/http"),
    ("io", "io"),
    ("json", "encoding/json"),
    ("log", "log"),
    ("math", "math"),
    ("net", "net"),
    ("os", "os"),
    ("path", "path"),
    ("rand", "math/rand"),
    ("reflect", "reflect"),
    ("regexp", "regexp"),
    ("runtime", "runtime"),
    ("slices", "slices"),
    ("sort", "sort"),
    ("sql", "database/sql"),
    ("strconv", "strconv"),
    ("strings", "strings"),
    ("sync", "sync"),
    ("testing", "testing"),
    ("time", "time"),
    ("unicode", "unicode"),
    ("url", "net/url"),
    ("utf8", "unicode/utf8"),
];

#[derive(Debug, Clone)]
struct ParsedGoImportLine {
    line: u32,
    import_path: String,
    candidate_idents: Vec<String>,
}

pub(crate) async fn analyze_go_diagnostics(
    path: &str,
    content: &str,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
) -> Vec<IdeDiagnostic> {
    let out = run_gofmt_syntax_check(content, env_map).await;
    if !out.is_empty() {
        return out;
    }

    let path_owned = path.to_string();
    let content_owned = content.to_string();
    let project_roots_owned = project_roots.to_vec();
    let readonly_roots_owned = readonly_roots.to_vec();
    let env_owned = env_map.clone();
    tokio::task::spawn_blocking(move || {
        analyze_go_diagnostics_sync(
            &path_owned,
            &content_owned,
            &project_roots_owned,
            &readonly_roots_owned,
            &env_owned,
        )
    })
    .await
    .unwrap_or_default()
}

fn analyze_go_diagnostics_sync(
    path: &str,
    content: &str,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
) -> Vec<IdeDiagnostic> {
    let mut out = Vec::new();
    let origin_path = Path::new(path);
    let lines: Vec<&str> = content.lines().collect();
    let imports = parse_go_imports(content);
    let import_entries =
        collect_go_import_entries(&lines, origin_path, project_roots, readonly_roots, env_map);
    let import_line_nums: HashSet<u32> = import_entries.iter().map(|e| e.line).collect();

    static RE_ANY_QUAL: OnceLock<Regex> = OnceLock::new();
    static RE_EXPORTED_QUAL: OnceLock<Regex> = OnceLock::new();
    static RE_DECL: OnceLock<Regex> = OnceLock::new();
    static RE_FUNC_RECV: OnceLock<Regex> = OnceLock::new();
    let any_qual_re = RE_ANY_QUAL.get_or_init(|| {
        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\.([A-Za-z_][A-Za-z0-9_]*)\b").unwrap()
    });
    let exported_qual_re = RE_EXPORTED_QUAL
        .get_or_init(|| Regex::new(r"\b([a-z][A-Za-z0-9_]*)\.([A-Z][A-Za-z0-9_]*)\b").unwrap());
    let decl_re = RE_DECL.get_or_init(|| {
        Regex::new(
            r"\b([A-Za-z_][A-Za-z0-9_]*)\s*(?::=|=|\s+\*?(?:\[\])?[A-Za-z_][A-Za-z0-9_.\[\]]*)",
        )
        .unwrap()
    });
    let func_recv_re = RE_FUNC_RECV
        .get_or_init(|| Regex::new(r"^\s*func\s*\(\s*([A-Za-z_][A-Za-z0-9_]*)\s+[^)]+\)").unwrap());

    let mut declared_locals: HashSet<String> = collect_same_dir_go_package_decls(origin_path)
        .as_ref()
        .clone();
    extract_go_top_and_grouped_decls(content, true, &mut declared_locals);

    let mut used_qualifiers: HashSet<String> = HashSet::new();
    let mut in_block = false;
    let mut body_qual_sites: Vec<(u32, u32, String)> = Vec::new();

    for (idx, &raw) in lines.iter().enumerate() {
        let ln = (idx + 1) as u32;
        let mask = code_mask_for_line("go", raw, &mut in_block);
        let code_ln: String = raw
            .char_indices()
            .map(|(i, c)| {
                if mask.get(i).copied().unwrap_or(false) {
                    c
                } else {
                    ' '
                }
            })
            .collect();
        let t = code_ln.trim();
        if t.starts_with("package ") || t.starts_with("import ") || import_line_nums.contains(&ln) {
            continue;
        }
        if let Some(rcap) = func_recv_re.captures(&code_ln) {
            declared_locals.insert(rcap[1].to_string());
        }
        for cap in decl_re.captures_iter(&code_ln) {
            let ident = &cap[1];
            if !is_go_keyword(ident) {
                declared_locals.insert(ident.to_string());
            }
        }
        if let Some((lhs, _)) = t.split_once(":=") {
            let lhs_clean = lhs.trim_start_matches("if ").trim_start_matches("for ");
            for part in lhs_clean.split(',') {
                let id = part.trim();
                if is_go_ident(id) && !is_go_keyword(id) {
                    declared_locals.insert(id.to_string());
                }
            }
        }
        for cap in any_qual_re.captures_iter(&code_ln) {
            used_qualifiers.insert(cap[1].to_string());
        }
        for cap in exported_qual_re.captures_iter(&code_ln) {
            let q = cap[1].to_string();
            if let Some(m) = cap.get(1) {
                let col = code_ln[..m.start()].encode_utf16().count() as u32 + 1;
                body_qual_sites.push((ln, col, q));
            }
        }
    }

    let mut all_imported_idents: HashSet<String> = imports.keys().cloned().collect();
    for entry in &import_entries {
        for ident in &entry.candidate_idents {
            all_imported_idents.insert(ident.clone());
        }
    }

    for entry in &import_entries {
        if entry.candidate_idents.is_empty() {
            continue;
        }
        if !entry
            .candidate_idents
            .iter()
            .any(|cand| used_qualifiers.contains(cand))
        {
            let imp_ln = entry.line;
            let imp_path = &entry.import_path;
            let del_edit = make_delete_line_edit(path, &lines, imp_ln);
            out.push(IdeDiagnostic {
                line: imp_ln,
                col: 1,
                end_line: imp_ln,
                end_col: (lines[(imp_ln - 1) as usize].encode_utf16().count() as u32).max(1),
                severity: "warning".into(),
                code: "unused_import".into(),
                source: "otto-go".into(),
                message: format!("\"{imp_path}\" imported and not used"),
                quick_fixes: vec![IdeQuickFix {
                    title: format!("Remove unused import \"{imp_path}\""),
                    line: imp_ln,
                    end_line: imp_ln,
                    replacement: String::new(),
                    delete_line: true,
                    is_preferred: true,
                    edits: vec![del_edit],
                }],
            });
        }
    }

    let stdlib_dirs = discover_goroot_src_dirs(readonly_roots, env_map);
    let mod_info = parse_go_mod_for_file(origin_path, project_roots);
    let mut flagged_quals = HashSet::new();

    for (ln, col, qual) in body_qual_sites {
        if all_imported_idents.contains(&qual) || declared_locals.contains(&qual) {
            continue;
        }
        if let Some(pkg_path) =
            resolve_go_auto_import_candidate(&qual, &stdlib_dirs, mod_info.as_ref())
        {
            if !flagged_quals.insert((ln, qual.clone())) {
                continue;
            }
            let edit = make_go_add_import_edit(path, &lines, &pkg_path);
            let ins_ln = edit.start_line;
            out.push(IdeDiagnostic {
                line: ln,
                col,
                end_line: ln,
                end_col: col + qual.encode_utf16().count() as u32,
                severity: "error".into(),
                code: "undeclared_name".into(),
                source: "otto-go".into(),
                message: format!("undefined: {qual}"),
                quick_fixes: vec![IdeQuickFix {
                    title: format!("Import \"{pkg_path}\""),
                    line: ins_ln,
                    end_line: ins_ln,
                    replacement: format!("import \"{pkg_path}\""),
                    delete_line: false,
                    is_preferred: true,
                    edits: vec![edit],
                }],
            });
        }
    }

    out
}

fn collect_go_import_entries(
    lines: &[&str],
    origin_file: &Path,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    env_map: &BTreeMap<String, String>,
) -> Vec<ParsedGoImportLine> {
    let mut out = Vec::new();
    let mut in_import_block = false;
    for (idx, &raw) in lines.iter().enumerate() {
        let t = raw.split("//").next().unwrap_or("").trim();
        if t.starts_with("import") && t.ends_with('(') {
            in_import_block = true;
            continue;
        }
        if in_import_block && t == ")" {
            in_import_block = false;
            continue;
        }
        if in_import_block || t.starts_with("import ") {
            if let Some(q1) = t.find('"') {
                if let Some(q2) = t[q1 + 1..].find('"') {
                    let imp_path = &t[q1 + 1..q1 + 1 + q2];
                    let prefix = t[..q1].trim().trim_start_matches("import").trim();
                    if prefix == "_" || prefix == "." || imp_path.is_empty() {
                        continue;
                    }
                    let mut candidate_idents = if !prefix.is_empty() {
                        vec![prefix.to_string()]
                    } else {
                        let mut cands = candidate_go_import_identifiers(imp_path);
                        for dir in resolve_go_package_dirs(
                            imp_path,
                            origin_file,
                            project_roots,
                            readonly_roots,
                            env_map,
                        ) {
                            if let Some(declared) = read_declared_go_package_name(&dir) {
                                if !cands.contains(&declared) {
                                    cands.insert(0, declared);
                                }
                            }
                        }
                        for root in project_roots {
                            let vendored = root.join("vendor").join(imp_path);
                            if let Some(declared) = read_declared_go_package_name(&vendored) {
                                if !cands.contains(&declared) {
                                    cands.insert(0, declared);
                                }
                            }
                        }
                        cands
                    };
                    candidate_idents.dedup();
                    out.push(ParsedGoImportLine {
                        line: (idx + 1) as u32,
                        import_path: imp_path.to_string(),
                        candidate_idents,
                    });
                }
            }
        }
    }
    out
}

pub(crate) fn resolve_go_auto_import_candidate(
    qual: &str,
    stdlib_dirs: &[PathBuf],
    mod_info: Option<&super::go_mod_nav::GoModInfo>,
) -> Option<String> {
    if let Some(info) = mod_info {
        for (rep_mod, rep_dir) in &info.local_replaces {
            if candidate_go_import_identifiers(rep_mod)
                .iter()
                .any(|c| c == qual)
                || read_declared_go_package_name(rep_dir).as_deref() == Some(qual)
            {
                return Some(rep_mod.clone());
            }
        }
        for req_mod in info.requires.keys() {
            if candidate_go_import_identifiers(req_mod)
                .iter()
                .any(|c| c == qual)
            {
                return Some(req_mod.clone());
            }
        }
    }
    for &(short, full) in COMMON_GO_STDLIB {
        if short == qual {
            return Some(full.to_string());
        }
    }
    for sdir in stdlib_dirs {
        if sdir.join(qual).is_dir() {
            return Some(qual.to_string());
        }
    }
    None
}

pub(crate) fn make_go_add_import_edit(path: &str, lines: &[&str], pkg_path: &str) -> IdeTextEdit {
    for (idx, raw) in lines.iter().enumerate() {
        let t = raw.trim();
        if t.starts_with("import") && t.ends_with('(') {
            let ins_ln = (idx + 2) as u32;
            return IdeTextEdit {
                path: path.to_string(),
                start_line: ins_ln,
                start_col: 1,
                end_line: ins_ln,
                end_col: 1,
                new_text: format!("\t\"{pkg_path}\"\n"),
            };
        }
    }
    for (idx, raw) in lines.iter().enumerate() {
        if raw.trim().starts_with("package ") {
            let ins_ln = (idx + 2) as u32;
            return IdeTextEdit {
                path: path.to_string(),
                start_line: ins_ln,
                start_col: 1,
                end_line: ins_ln,
                end_col: 1,
                new_text: format!("\nimport \"{pkg_path}\"\n"),
            };
        }
    }
    IdeTextEdit {
        path: path.to_string(),
        start_line: 1,
        start_col: 1,
        end_line: 1,
        end_col: 1,
        new_text: format!("import \"{pkg_path}\"\n"),
    }
}

async fn run_gofmt_syntax_check(
    content: &str,
    env_map: &BTreeMap<String, String>,
) -> Vec<IdeDiagnostic> {
    let gofmt_bin =
        crate::toolchains::find_executable_in(&crate::toolchains::system_search_dirs(), &["gofmt"])
            .unwrap_or_else(|| PathBuf::from("gofmt"));
    let mut cmd = Command::new(gofmt_bin);
    cmd.arg("-e")
        .env_clear()
        .envs(crate::environments::shell_env::collect_safe_non_root_base_env())
        .envs(env_map)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let Ok(mut child) = cmd.spawn() else {
        return Vec::new();
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(content.as_bytes()).await;
    }
    let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(2), child.wait_with_output()).await
    else {
        return Vec::new();
    };
    if out.status.success() {
        return Vec::new();
    }
    static RE_GOFMT: OnceLock<Regex> = OnceLock::new();
    let re = RE_GOFMT.get_or_init(|| Regex::new(r":(\d+):(\d+):\s*(.+)$").unwrap());
    let mut diags = Vec::new();
    for line in String::from_utf8_lossy(&out.stderr).lines() {
        if let Some(cap) = re.captures(line) {
            let ln: u32 = cap[1].parse().unwrap_or(1);
            let col: u32 = cap[2].parse().unwrap_or(1);
            diags.push(IdeDiagnostic {
                line: ln,
                col,
                end_line: ln,
                end_col: col + 1,
                severity: "error".into(),
                code: "syntax_error".into(),
                source: "gofmt".into(),
                message: cap[3].trim().to_string(),
                quick_fixes: Vec::new(),
            });
        }
    }
    diags
}

const MAX_GO_SIBLING_CACHE_DIRS: usize = 128;

type CachedGoDirFileDecls = Vec<(PathBuf, bool, Vec<String>)>;
type GoSiblingDirCache = Vec<(PathBuf, Option<SystemTime>, Arc<CachedGoDirFileDecls>)>;

static GO_SIBLING_DIR_CACHE: OnceLock<Mutex<GoSiblingDirCache>> = OnceLock::new();

/// Collect top-level package declarations (`var`, `const`, `type`, `func`) from
/// sibling `.go` files in `origin_path.parent()`, mtime-cached up to 128 dirs.
#[allow(clippy::disallowed_methods)]
pub(crate) fn collect_same_dir_go_package_decls(origin_path: &Path) -> Arc<HashSet<String>> {
    let Some(parent) = origin_path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Arc::new(HashSet::new());
    };
    let dir_key = parent
        .canonicalize()
        .unwrap_or_else(|_| parent.to_path_buf());
    if !dir_key.is_dir() {
        return Arc::new(HashSet::new());
    }
    let origin_canon = origin_path
        .canonicalize()
        .unwrap_or_else(|_| origin_path.to_path_buf());
    let is_test_origin = origin_path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.ends_with("_test.go"));

    let dir_mtime = compute_go_dir_stamp(&dir_key);
    let cache = GO_SIBLING_DIR_CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let cached_files = {
        let guard = cache.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .iter()
            .find(|(d, mt, _)| d == &dir_key && *mt == dir_mtime)
            .map(|(_, _, files)| Arc::clone(files))
    };

    let per_file = match cached_files {
        Some(hit) => hit,
        None => {
            let scanned = Arc::new(scan_go_dir_top_decls(&dir_key));
            let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
            guard.retain(|(d, _, _)| d != &dir_key);
            if guard.len() >= MAX_GO_SIBLING_CACHE_DIRS {
                guard.remove(0);
            }
            guard.push((dir_key, dir_mtime, Arc::clone(&scanned)));
            scanned
        }
    };

    let mut merged = HashSet::new();
    for (file_path, is_test_file, decls) in per_file.iter() {
        if file_path == &origin_canon {
            continue;
        }
        if *is_test_file && !is_test_origin {
            continue;
        }
        for d in decls {
            merged.insert(d.clone());
        }
    }
    Arc::new(merged)
}

#[allow(clippy::disallowed_methods)]
fn compute_go_dir_stamp(dir: &Path) -> Option<SystemTime> {
    let mut latest = std::fs::metadata(dir).ok().and_then(|m| m.modified().ok());
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten().take(256) {
            let p = entry.path();
            if p.extension().and_then(|e| e.to_str()) != Some("go") {
                continue;
            }
            if let Ok(mt) = entry.metadata().and_then(|m| m.modified()) {
                latest = Some(match latest {
                    Some(prev) if prev >= mt => prev,
                    _ => mt,
                });
            }
        }
    }
    latest
}

#[allow(clippy::disallowed_methods)]
fn scan_go_dir_top_decls(dir: &Path) -> CachedGoDirFileDecls {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = rd
        .flatten()
        .take(256)
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("go"))
        .collect();
    files.sort();
    let mut out = Vec::with_capacity(files.len());
    for file in files {
        let canon = file.canonicalize().unwrap_or(file);
        if otto_core::secret_paths::is_denied_file(&canon) {
            continue;
        }
        let is_test = canon
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_test.go"));
        let Ok(src) = std::fs::read_to_string(&canon) else {
            continue;
        };
        let mut set = HashSet::new();
        extract_go_top_and_grouped_decls(&src, false, &mut set);
        let mut decls: Vec<String> = set.into_iter().collect();
        decls.sort();
        out.push((canon, is_test, decls));
    }
    out
}

fn extract_go_top_and_grouped_decls(
    content: &str,
    include_local_groups_and_params: bool,
    out: &mut HashSet<String>,
) {
    let mut in_block = false;
    let mut brace_depth: i32 = 0;
    let mut decl_group: Option<&'static str> = None;

    for raw in content.lines() {
        let mask = code_mask_for_line("go", raw, &mut in_block);
        let code_ln: String = raw
            .char_indices()
            .map(|(i, c)| {
                if mask.get(i).copied().unwrap_or(false) {
                    c
                } else {
                    ' '
                }
            })
            .collect();
        let t = code_ln.trim();
        if t.is_empty() {
            continue;
        }

        if brace_depth == 0 || include_local_groups_and_params {
            if let Some(group_kind) = decl_group {
                if t == ")" || t.starts_with(')') {
                    decl_group = None;
                } else {
                    extract_go_decl_lhs_idents(t, group_kind, out);
                }
            } else if let Some(rest) = strip_go_kw(t, "var") {
                if rest.starts_with('(') {
                    if !rest.contains(')') {
                        decl_group = Some("var");
                    }
                } else {
                    extract_go_decl_lhs_idents(rest, "var", out);
                }
            } else if let Some(rest) = strip_go_kw(t, "const") {
                if rest.starts_with('(') {
                    if !rest.contains(')') {
                        decl_group = Some("const");
                    }
                } else {
                    extract_go_decl_lhs_idents(rest, "const", out);
                }
            } else if let Some(rest) = strip_go_kw(t, "type") {
                if rest.starts_with('(') {
                    if !rest.contains(')') {
                        decl_group = Some("type");
                    }
                } else {
                    extract_go_decl_lhs_idents(rest, "type", out);
                }
            }
        }

        if brace_depth == 0 {
            if let Some(rest) = strip_go_kw(t, "func") {
                if let Some(stripped) = rest.strip_prefix('(') {
                    if include_local_groups_and_params {
                        if let Some((recv_inner, after_recv)) = stripped.split_once(')') {
                            let recv_id = recv_inner.split_whitespace().next().unwrap_or("");
                            if is_go_ident(recv_id) && !is_go_keyword(recv_id) {
                                out.insert(recv_id.to_string());
                            }
                            extract_go_func_params(after_recv, out);
                        }
                    }
                } else {
                    let fn_name: String = rest
                        .chars()
                        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                        .collect();
                    if is_go_ident(&fn_name) && !is_go_keyword(&fn_name) {
                        out.insert(fn_name.clone());
                    }
                    if include_local_groups_and_params {
                        extract_go_func_params(&rest[fn_name.len()..], out);
                    }
                }
            }
        }

        for ch in code_ln.chars() {
            if ch == '{' {
                brace_depth += 1;
            } else if ch == '}' {
                brace_depth = (brace_depth - 1).max(0);
            }
        }
    }
}

fn strip_go_kw<'a>(trimmed: &'a str, kw: &str) -> Option<&'a str> {
    let rest = trimmed.strip_prefix(kw)?;
    let first = rest.chars().next()?;
    if first.is_whitespace() || first == '(' {
        Some(rest.trim_start())
    } else {
        None
    }
}

fn extract_go_decl_lhs_idents(spec: &str, kind: &str, out: &mut HashSet<String>) {
    let lhs = spec.split('=').next().unwrap_or(spec).trim();
    if lhs.is_empty() {
        return;
    }
    if kind == "type" {
        let id: String = lhs
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if is_go_ident(&id) && !is_go_keyword(&id) {
            out.insert(id);
        }
        return;
    }
    for part in lhs.split(',') {
        let tok = part.split_whitespace().next().unwrap_or("").trim();
        if is_go_ident(tok) && !is_go_keyword(tok) {
            out.insert(tok.to_string());
        }
    }
}

fn extract_go_func_params(sig_after_name: &str, out: &mut HashSet<String>) {
    let s = sig_after_name.trim_start();
    let s = if s.starts_with('[') {
        match s.split_once(']') {
            Some((_, after)) => after.trim_start(),
            None => return,
        }
    } else {
        s
    };
    let Some(after_open) = s.strip_prefix('(') else {
        return;
    };
    let Some((params_inside, _)) = after_open.split_once(')') else {
        return;
    };
    for seg in params_inside.split(',') {
        let words: Vec<&str> = seg.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        // In `a, b int` -> `words` for `a` is `["a"]`, for `b int` is `["b", "int"]`.
        // In unnamed param `*testing.T` or `context.Context`, single token has `*`/`.` or is a builtin type.
        if words.len() >= 2
            || (words.len() == 1 && is_go_ident(words[0]) && !is_go_builtin_type(words[0]))
        {
            let cand = words[0].trim();
            if is_go_ident(cand) && !is_go_keyword(cand) {
                out.insert(cand.to_string());
            }
        }
    }
}

fn is_go_ident(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if s == "_" {
        return false;
    }
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_go_keyword(s: &str) -> bool {
    matches!(
        s,
        "break"
            | "case"
            | "chan"
            | "const"
            | "continue"
            | "default"
            | "defer"
            | "else"
            | "fallthrough"
            | "for"
            | "func"
            | "go"
            | "goto"
            | "if"
            | "import"
            | "interface"
            | "map"
            | "package"
            | "range"
            | "return"
            | "select"
            | "struct"
            | "switch"
            | "type"
            | "var"
    )
}

fn is_go_builtin_type(s: &str) -> bool {
    matches!(
        s,
        "any"
            | "bool"
            | "byte"
            | "comparable"
            | "complex64"
            | "complex128"
            | "error"
            | "float32"
            | "float64"
            | "int"
            | "int8"
            | "int16"
            | "int32"
            | "int64"
            | "rune"
            | "string"
            | "uint"
            | "uint8"
            | "uint16"
            | "uint32"
            | "uint64"
            | "uintptr"
    )
}

#[cfg(test)]
mod sibling_decl_tests {
    use super::*;

    #[tokio::test]
    async fn sibling_go_package_var_is_not_flagged_as_undeclared_import() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        std::fs::write(
            repo.join("go.mod"),
            "module example.com/svc\n\ngo 1.24\n\nrequire (\n\tgithub.com/moby/moby/client v0.1.0\n)\n",
        )
        .unwrap();
        let tests_dir = repo.join("tests");
        std::fs::create_dir_all(&tests_dir).unwrap();
        std::fs::write(
            tests_dir.join("main_test.go"),
            "//go:build component\n\npackage tests\n\nvar (\n\tclient *HttpClient\n\tbaseURL = \"http://localhost\"\n)\n",
        )
        .unwrap();
        let target_file = tests_dir.join("assign_frb_component_test.go");
        let content = "//go:build component\n\npackage tests\n\nimport \"testing\"\n\nfunc TestAssign(t *testing.T) {\n\t_ = client.R()\n\t_ = baseURL\n}\n";
        std::fs::write(&target_file, content).unwrap();

        let diags = analyze_go_diagnostics(
            target_file.to_str().unwrap(),
            content,
            &[repo.to_path_buf()],
            &[],
            &BTreeMap::new(),
        )
        .await;
        assert!(
            !diags
                .iter()
                .any(|d| d.message.contains("undefined: client")),
            "unexpected diags: {diags:?}"
        );
    }
}
