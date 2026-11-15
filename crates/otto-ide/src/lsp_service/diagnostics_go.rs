//! Go syntax, unused-import, and unimported-qualifier diagnostics with Go module
//! `/vN` major-version, `.vN`, hyphenated repo, and on-disk `package <name>` resolution.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

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
    let mut out = run_gofmt_syntax_check(content, env_map).await;
    if !out.is_empty() {
        return out;
    }

    let origin_path = Path::new(path);
    let lines: Vec<&str> = content.lines().collect();
    let imports = parse_go_imports(content);
    let import_entries =
        collect_go_import_entries(&lines, origin_path, project_roots, readonly_roots, env_map);
    let import_line_nums: HashSet<u32> = import_entries.iter().map(|e| e.line).collect();

    static RE_ANY_QUAL: OnceLock<Regex> = OnceLock::new();
    static RE_EXPORTED_QUAL: OnceLock<Regex> = OnceLock::new();
    static RE_DECL: OnceLock<Regex> = OnceLock::new();
    let any_qual_re = RE_ANY_QUAL.get_or_init(|| {
        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\.([A-Za-z_][A-Za-z0-9_]*)\b").unwrap()
    });
    let exported_qual_re = RE_EXPORTED_QUAL
        .get_or_init(|| Regex::new(r"\b([a-z][A-Za-z0-9_]*)\.([A-Z][A-Za-z0-9_]*)\b").unwrap());
    let decl_re = RE_DECL.get_or_init(|| {
        Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*(?::=|=|\s+\*?[A-Za-z0-9_.\[\]]+)").unwrap()
    });

    let mut declared_locals = HashSet::new();
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
        for cap in decl_re.captures_iter(&code_ln) {
            declared_locals.insert(cap[1].to_string());
        }
        for cap in any_qual_re.captures_iter(&code_ln) {
            used_qualifiers.insert(cap[1].to_string());
        }
        for cap in exported_qual_re.captures_iter(&code_ln) {
            let q = cap[1].to_string();
            if let Some(m) = cap.get(1) {
                let col = code_ln[..m.start()].chars().count() as u32 + 1;
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
                end_col: (lines[(imp_ln - 1) as usize].chars().count() as u32).max(1),
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
                end_col: col + qual.chars().count() as u32,
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
