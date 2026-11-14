//! Fast Multi-Language Diagnostics (`POST /ide/lsp/diagnostics`) with Concrete Quick Fixes.
//!
//! Combines the `< 30ms` stateless Java import/classpath/Lombok analyzer (`java_diag`)
//! with fast syntax & import analyzers for Go, Python, JSON/JSONL, TypeScript/JS,
//! Rust, and Shell (`bash` / `zsh`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use otto_core::domain::User;
use otto_core::Result;
use regex::Regex;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::target::ResolvedTarget;
use crate::types::{
    IdeDiagnostic, IdeDiagnosticsReq, IdeDiagnosticsResp, IdeQuickFix, IdeTargetKind,
    IdeTargetSpec, IdeTextEdit,
};
use crate::IdeCtx;

use super::diagnostics_polyglot::{
    analyze_js_ts_diagnostics, analyze_python_diagnostics, analyze_rust_diagnostics,
    analyze_shell_diagnostics,
};
use super::go_mod_nav::{discover_goroot_src_dirs, parse_go_imports, parse_go_mod_for_file};
use super::java_diag::{analyze_java_diagnostics, make_delete_line_edit};
use super::{build_formatter_env, code_mask_for_line, resolve_language};

const COMMON_GO_STDLIB: &[(&str, &str)] = &[
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
    ("atomic", "sync/atomic"),
    ("testing", "testing"),
    ("time", "time"),
    ("unicode", "unicode"),
    ("url", "net/url"),
    ("utf8", "unicode/utf8"),
];

/// `POST /ide/lsp/diagnostics` — Stateless multi-language diagnostics + concrete `IdeQuickFix` edits.
pub async fn diagnostics<S: IdeCtx>(
    ctx: &S,
    user: &User,
    req: IdeDiagnosticsReq,
) -> Result<IdeDiagnosticsResp> {
    let target_spec_opt = resolve_effective_target_spec(&req);
    let mut resolved_opt: Option<(ResolvedTarget, IdeTargetSpec)> = None;
    if let Some(spec) = target_spec_opt {
        if let Ok(r) = ctx.ide().targets.resolve(ctx, user, &spec).await {
            resolved_opt = Some((r, spec));
        }
    }

    let content = match req.content.clone() {
        Some(c) => c,
        None => {
            if let Some((ref resolved, _)) = resolved_opt {
                ctx.ide()
                    .targets
                    .read_file(resolved, &req.path)
                    .await
                    .map(|f| f.content)
                    .unwrap_or_default()
            } else if Path::new(&req.path).is_file() {
                tokio::fs::read_to_string(&req.path)
                    .await
                    .unwrap_or_default()
            } else {
                String::new()
            }
        }
    };

    let lang = resolve_language(ctx, &req.path, req.language.as_deref());
    let mut env_map = BTreeMap::new();
    let mut project_roots = Vec::new();
    let mut readonly_roots = Vec::new();
    let mut preferred_java_home: Option<PathBuf> = None;

    if let Some((ref resolved, ref spec)) = resolved_opt {
        env_map = build_formatter_env(ctx, user, resolved, spec).await;
        if let Ok(pe) = ctx
            .ide()
            .environments
            .get_project_env(&ctx.pool(), spec)
            .await
        {
            preferred_java_home = pe
                .java_home
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from);
        }
        project_roots = resolved.local_project_roots();
        if let ResolvedTarget::Local {
            readonly_roots: ro, ..
        } = resolved
        {
            readonly_roots = ro.clone();
        }
    } else {
        for pf in &req.project_folders {
            let p = PathBuf::from(pf);
            if p.is_dir() {
                project_roots.push(p);
            }
        }
        if let Some(parent) = Path::new(&req.path).parent() {
            if parent.is_dir() && project_roots.is_empty() {
                project_roots.push(parent.to_path_buf());
            }
        }
    }
    if preferred_java_home.is_none() {
        preferred_java_home = env_map
            .get("JAVA_HOME")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
    }

    let diags = match lang.as_str() {
        "java" => {
            let path_c = req.path.clone();
            let content_c = content.clone();
            tokio::task::spawn_blocking(move || {
                analyze_java_diagnostics(
                    &path_c,
                    &content_c,
                    &project_roots,
                    preferred_java_home.as_deref(),
                    &env_map,
                )
            })
            .await
            .unwrap_or_default()
        }
        "json" | "jsonl" => analyze_json_diagnostics(&req.path, &lang, &content),
        "go" => {
            analyze_go_diagnostics(
                &req.path,
                &content,
                &project_roots,
                &readonly_roots,
                &env_map,
            )
            .await
        }
        "python" => analyze_python_diagnostics(&req.path, &content, &env_map).await,
        "shell" | "bash" | "sh" | "zsh" => {
            analyze_shell_diagnostics(&req.path, &lang, &content).await
        }
        "javascript" | "typescript" | "svelte" => {
            analyze_js_ts_diagnostics(&req.path, &content, &project_roots)
        }
        "rust" => analyze_rust_diagnostics(&req.path, &content),
        _ => Vec::new(),
    };

    Ok(IdeDiagnosticsResp { diagnostics: diags })
}

fn resolve_effective_target_spec(req: &IdeDiagnosticsReq) -> Option<IdeTargetSpec> {
    if let Some(mut spec) = req.target.clone() {
        spec.absorb_project_folders(&req.project_folders);
        return Some(spec);
    }
    let p = Path::new(&req.path);
    if p.is_absolute() {
        let mut cur = if p.is_dir() { Some(p) } else { p.parent() };
        let mut root_dir = cur.map(Path::to_path_buf);
        while let Some(d) = cur {
            if d.join(".git").exists()
                || d.join("pom.xml").is_file()
                || d.join("build.gradle").is_file()
                || d.join("build.gradle.kts").is_file()
                || d.join("go.mod").is_file()
                || d.join("Cargo.toml").is_file()
                || d.join("package.json").is_file()
            {
                root_dir = Some(d.to_path_buf());
                break;
            }
            cur = d.parent();
        }
        if let Some(rd) = root_dir.filter(|d| d.is_dir()) {
            let mut spec = IdeTargetSpec {
                kind: IdeTargetKind::Local,
                root: rd.to_string_lossy().into_owned(),
                ..Default::default()
            };
            spec.absorb_project_folders(&req.project_folders);
            return Some(spec);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// JSON / JSONL Diagnostics (`serde_json` exact line/col + trailing comma fix)
// ---------------------------------------------------------------------------

pub(crate) fn analyze_json_diagnostics(
    path: &str,
    lang: &str,
    content: &str,
) -> Vec<IdeDiagnostic> {
    let lines: Vec<&str> = content.lines().collect();
    if lang == "jsonl" || path.ends_with(".jsonl") {
        let mut out = Vec::new();
        for (idx, raw) in lines.iter().enumerate() {
            if raw.trim().is_empty() {
                continue;
            }
            if let Err(err) = serde_json::from_str::<serde_json::Value>(raw) {
                let col = err.column().max(1) as u32;
                out.push(IdeDiagnostic {
                    line: (idx + 1) as u32,
                    col,
                    end_line: (idx + 1) as u32,
                    end_col: col + 1,
                    severity: "error".into(),
                    code: "json_syntax".into(),
                    source: "otto-json".into(),
                    message: err.to_string(),
                    quick_fixes: Vec::new(),
                });
            }
        }
        return out;
    }

    let Err(err) = serde_json::from_str::<serde_json::Value>(content) else {
        return Vec::new();
    };
    let ln = (err.line().max(1) as u32).min(lines.len().max(1) as u32);
    let col = err.column().max(1) as u32;
    let mut fixes = Vec::new();
    let mut diag_line = ln;
    let mut diag_col = col;

    // Detect trailing comma on `ln` or `ln - 1` (`trailing comma` in serde_json error).
    if err.to_string().contains("trailing comma") {
        for check_ln in [ln.saturating_sub(1), ln] {
            if check_ln == 0 || (check_ln as usize) > lines.len() {
                continue;
            }
            let raw_ln = lines[(check_ln - 1) as usize];
            if let Some(comma_byte) = raw_ln.rfind(',') {
                if raw_ln[comma_byte + 1..].trim().is_empty() {
                    let comma_col = raw_ln[..comma_byte].chars().count() as u32 + 1;
                    diag_line = check_ln;
                    diag_col = comma_col;
                    let mut fixed_line = raw_ln[..comma_byte].to_string();
                    fixed_line.push_str(&raw_ln[comma_byte + 1..]);
                    fixes.push(IdeQuickFix {
                        title: "Remove trailing comma".into(),
                        line: check_ln,
                        end_line: check_ln,
                        replacement: fixed_line,
                        delete_line: false,
                        is_preferred: true,
                        edits: vec![IdeTextEdit {
                            path: path.to_string(),
                            start_line: check_ln,
                            start_col: comma_col,
                            end_line: check_ln,
                            end_col: comma_col + 1,
                            new_text: String::new(),
                        }],
                    });
                    break;
                }
            }
        }
    }

    vec![IdeDiagnostic {
        line: diag_line,
        col: diag_col,
        end_line: diag_line,
        end_col: diag_col + 1,
        severity: "error".into(),
        code: "json_syntax".into(),
        source: "otto-json".into(),
        message: err.to_string(),
        quick_fixes: fixes,
    }]
}

// ---------------------------------------------------------------------------
// Go Diagnostics (`gofmt -e` syntax + unused imports + unimported package fixes)
// ---------------------------------------------------------------------------

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

    let lines: Vec<&str> = content.lines().collect();
    let imports = parse_go_imports(content);
    let mut import_lines: HashMap<String, (u32, String)> = HashMap::new();
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
                    if prefix != "_" && prefix != "." {
                        let alias = if prefix.is_empty() {
                            imp_path.rsplit('/').next().unwrap_or(imp_path).to_string()
                        } else {
                            prefix.to_string()
                        };
                        import_lines.insert(alias, ((idx + 1) as u32, imp_path.to_string()));
                    }
                }
            }
        }
    }

    // Scan body code for `ident.Member` qualifiers and declared identifiers.
    static RE_QUAL: OnceLock<Regex> = OnceLock::new();
    static RE_DECL: OnceLock<Regex> = OnceLock::new();
    let qual_re = RE_QUAL
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
        if t.starts_with("package ")
            || t.starts_with("import ")
            || import_lines.values().any(|(il, _)| *il == ln)
        {
            continue;
        }
        for cap in decl_re.captures_iter(&code_ln) {
            declared_locals.insert(cap[1].to_string());
        }
        for cap in qual_re.captures_iter(&code_ln) {
            let q = cap[1].to_string();
            used_qualifiers.insert(q.clone());
            if let Some(m) = cap.get(1) {
                let col = raw[..m.start()].chars().count() as u32 + 1;
                body_qual_sites.push((ln, col, q));
            }
        }
    }

    // Unused Go imports (`"imported and not used"`).
    for (alias, (imp_ln, imp_path)) in &import_lines {
        if !used_qualifiers.contains(alias) {
            let del_edit = make_delete_line_edit(path, &lines, *imp_ln);
            out.push(IdeDiagnostic {
                line: *imp_ln,
                col: 1,
                end_line: *imp_ln,
                end_col: (lines[(*imp_ln - 1) as usize].chars().count() as u32).max(1),
                severity: "warning".into(),
                code: "unused_import".into(),
                source: "otto-go".into(),
                message: format!("\"{imp_path}\" imported and not used"),
                quick_fixes: vec![IdeQuickFix {
                    title: format!("Remove unused import \"{imp_path}\""),
                    line: *imp_ln,
                    end_line: *imp_ln,
                    replacement: String::new(),
                    delete_line: true,
                    is_preferred: true,
                    edits: vec![del_edit],
                }],
            });
        }
    }

    // Undeclared Go package qualifiers (`fmt.Println` when `"fmt"` is missing).
    let stdlib_dirs = discover_goroot_src_dirs(readonly_roots, env_map);
    let mod_info = parse_go_mod_for_file(Path::new(path), project_roots);
    let mut flagged_quals = HashSet::new();

    for (ln, col, qual) in body_qual_sites {
        if imports.contains_key(&qual) || declared_locals.contains(&qual) {
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

pub(crate) fn resolve_go_auto_import_candidate(
    qual: &str,
    stdlib_dirs: &[PathBuf],
    mod_info: Option<&super::go_mod_nav::GoModInfo>,
) -> Option<String> {
    if let Some(info) = mod_info {
        for rep_mod in info.local_replaces.keys() {
            if rep_mod.rsplit('/').next() == Some(qual) {
                return Some(rep_mod.clone());
            }
        }
        for req_mod in info.requires.keys() {
            if req_mod.rsplit('/').next() == Some(qual) {
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
