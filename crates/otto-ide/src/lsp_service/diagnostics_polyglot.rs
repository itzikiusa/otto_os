//! Fast Python, Shell (`bash`/`zsh`), JS/TS, and Rust syntax & import diagnostics.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::types::{IdeDiagnostic, IdeQuickFix};

use super::java_diag::make_delete_line_edit;
use super::navigation_ext::resolve_js_ts_module_files;

pub(super) async fn analyze_python_diagnostics(
    path: &str,
    content: &str,
    env_map: &BTreeMap<String, String>,
) -> Vec<IdeDiagnostic> {
    let py_bin = env_map
        .get("VIRTUAL_ENV")
        .map(|v| PathBuf::from(v).join("bin/python3"))
        .filter(|p| p.is_file())
        .unwrap_or_else(|| PathBuf::from("python3"));
    let script = "import ast,json,sys\nsrc=sys.stdin.read()\ntry:\n ast.parse(src)\nexcept SyntaxError as e:\n print(json.dumps({'line':e.lineno or 1,'col':e.offset or 1,'msg':e.msg}))\n";
    let mut cmd = Command::new(py_bin);
    cmd.args(["-c", script])
        .env_clear()
        .envs(crate::environments::shell_env::collect_safe_non_root_base_env())
        .envs(env_map)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if let Ok(mut child) = cmd.spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(content.as_bytes()).await;
        }
        if let Ok(Ok(out)) =
            tokio::time::timeout(Duration::from_secs(2), child.wait_with_output()).await
        {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(stdout.trim()) {
                let ln = val.get("line").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                let col = val.get("col").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                let msg = val
                    .get("msg")
                    .and_then(|v| v.as_str())
                    .unwrap_or("SyntaxError")
                    .to_string();
                return vec![IdeDiagnostic {
                    line: ln,
                    col,
                    end_line: ln,
                    end_col: col + 1,
                    severity: "error".into(),
                    code: "syntax_error".into(),
                    source: "python".into(),
                    message: msg,
                    quick_fixes: Vec::new(),
                }];
            }
        }
    }
    find_simple_unused_imports(path, "python", content)
}

pub(super) async fn analyze_shell_diagnostics(
    path: &str,
    lang: &str,
    content: &str,
) -> Vec<IdeDiagnostic> {
    let is_zsh = lang == "zsh"
        || path.ends_with(".zsh")
        || content
            .lines()
            .next()
            .is_some_and(|l| l.starts_with("#!") && l.contains("zsh"));
    let bin = if is_zsh { "/bin/zsh" } else { "/bin/bash" };
    let mut cmd = Command::new(bin);
    cmd.arg("-n")
        .env_clear()
        .envs(crate::environments::shell_env::collect_safe_non_root_base_env())
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
    static RE_SH: OnceLock<Regex> = OnceLock::new();
    let re = RE_SH.get_or_init(|| Regex::new(r"(?:line |:)(\d+):\s*(.+)$").unwrap());
    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut diags = Vec::new();
    for line in stderr.lines() {
        if let Some(cap) = re.captures(line) {
            let ln: u32 = cap[1].parse().unwrap_or(1);
            diags.push(IdeDiagnostic {
                line: ln,
                col: 1,
                end_line: ln,
                end_col: 2,
                severity: "error".into(),
                code: "syntax_error".into(),
                source: if is_zsh { "zsh" } else { "bash" }.into(),
                message: cap[2].trim().to_string(),
                quick_fixes: Vec::new(),
            });
        }
    }
    diags
}

pub(super) fn analyze_js_ts_diagnostics(
    path: &str,
    content: &str,
    project_roots: &[PathBuf],
) -> Vec<IdeDiagnostic> {
    let lines: Vec<&str> = content.lines().collect();
    let mut out = Vec::new();
    let origin = Path::new(path);
    if origin.is_absolute() && origin.parent().is_some_and(|p| p.is_dir()) {
        for (idx, &raw) in lines.iter().enumerate() {
            let t = raw.trim();
            if (t.starts_with("import ") || t.starts_with("export ")) && t.contains("from ") {
                if let Some(spec) = super::navigation_imports::extract_last_quoted(t) {
                    if spec.starts_with('.')
                        && resolve_js_ts_module_files(&spec, origin, project_roots).is_empty()
                    {
                        let ln = (idx + 1) as u32;
                        let del_edit = make_delete_line_edit(path, &lines, ln);
                        out.push(IdeDiagnostic {
                            line: ln,
                            col: 1,
                            end_line: ln,
                            end_col: (raw.chars().count() as u32).max(1),
                            severity: "error".into(),
                            code: "unresolved_import".into(),
                            source: "otto-ts".into(),
                            message: format!("Cannot find module '{spec}'"),
                            quick_fixes: vec![IdeQuickFix {
                                title: format!("Remove unresolved import '{spec}'"),
                                line: ln,
                                end_line: ln,
                                replacement: String::new(),
                                delete_line: true,
                                is_preferred: true,
                                edits: vec![del_edit],
                            }],
                        });
                    }
                }
            }
        }
    }
    out.extend(find_simple_unused_imports(path, "typescript", content));
    out
}

pub(super) fn analyze_rust_diagnostics(path: &str, content: &str) -> Vec<IdeDiagnostic> {
    find_simple_unused_imports(path, "rust", content)
}

fn find_simple_unused_imports(path: &str, lang: &str, content: &str) -> Vec<IdeDiagnostic> {
    let lines: Vec<&str> = content.lines().collect();
    let mut imports: Vec<(u32, String)> = Vec::new();
    for (idx, &raw) in lines.iter().enumerate() {
        let ln = (idx + 1) as u32;
        let t = raw.trim();
        match lang {
            "rust" => {
                if let Some(rest) = t.strip_prefix("use ").and_then(|s| s.strip_suffix(';')) {
                    if !rest.contains('{') && !rest.contains('*') {
                        let sym = rest
                            .split(" as ")
                            .last()
                            .and_then(|s| s.rsplit("::").next())
                            .unwrap_or("")
                            .trim();
                        if !sym.is_empty() && sym != "self" {
                            imports.push((ln, sym.to_string()));
                        }
                    }
                }
            }
            "python" => {
                if let Some(rest) = t.strip_prefix("import ") {
                    if !rest.contains(',') {
                        let sym = rest
                            .split(" as ")
                            .last()
                            .and_then(|s| s.rsplit('.').next())
                            .unwrap_or("")
                            .trim();
                        if !sym.is_empty() {
                            imports.push((ln, sym.to_string()));
                        }
                    }
                }
            }
            "typescript" | "javascript"
                if t.starts_with("import ")
                    && t.contains(" from ")
                    && !t.contains(',')
                    && !t.contains('*') =>
            {
                let head = t.split(" from ").next().unwrap_or("");
                let sym = head
                    .trim_start_matches("import")
                    .trim_start_matches("type")
                    .trim_matches(|c: char| matches!(c, '{' | '}' | ' '))
                    .split(" as ")
                    .last()
                    .unwrap_or("")
                    .trim();
                if !sym.is_empty() {
                    imports.push((ln, sym.to_string()));
                }
            }
            _ => {}
        }
    }

    static RE_ID: OnceLock<Regex> = OnceLock::new();
    let id_re = RE_ID.get_or_init(|| Regex::new(r"\b([A-Za-z_][A-Za-z0-9_]*)\b").unwrap());
    let import_lns: HashSet<u32> = imports.iter().map(|(l, _)| *l).collect();
    let mut used = HashSet::new();
    for (idx, &raw) in lines.iter().enumerate() {
        if import_lns.contains(&((idx + 1) as u32)) {
            continue;
        }
        for cap in id_re.captures_iter(raw) {
            used.insert(cap[1].to_string());
        }
    }

    let mut out = Vec::new();
    for (ln, sym) in imports {
        if !used.contains(&sym) {
            let del_edit = make_delete_line_edit(path, &lines, ln);
            out.push(IdeDiagnostic {
                line: ln,
                col: 1,
                end_line: ln,
                end_col: (lines[(ln - 1) as usize].chars().count() as u32).max(1),
                severity: "warning".into(),
                code: "unused_import".into(),
                source: format!("otto-{lang}"),
                message: format!("Unused import '{sym}'"),
                quick_fixes: vec![IdeQuickFix {
                    title: format!("Remove unused import '{sym}'"),
                    line: ln,
                    end_line: ln,
                    replacement: String::new(),
                    delete_line: true,
                    is_preferred: true,
                    edits: vec![del_edit],
                }],
            });
        }
    }
    out
}
