//! Fast Multi-Language Diagnostics (`POST /ide/lsp/diagnostics`) with Concrete Quick Fixes.
//!
//! Combines the `< 30ms` stateless Java import/classpath/Lombok analyzer (`java_diag`)
//! with fast syntax & import analyzers for Go, Python, JSON/JSONL, TypeScript/JS,
//! Rust, and Shell (`bash` / `zsh`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use otto_core::domain::User;
use otto_core::{Error, Result};

use crate::target::ResolvedTarget;
use crate::types::{
    IdeDiagnostic, IdeDiagnosticsReq, IdeDiagnosticsResp, IdeQuickFix, IdeTargetKind,
    IdeTargetSpec, IdeTextEdit,
};
use crate::IdeCtx;

pub(crate) use super::diagnostics_go::analyze_go_diagnostics;
use super::diagnostics_polyglot::{
    analyze_js_ts_diagnostics, analyze_python_diagnostics, analyze_rust_diagnostics,
    analyze_shell_diagnostics,
};
use super::java_diag::analyze_java_diagnostics;
use super::{build_formatter_env, resolve_language};

/// `POST /ide/lsp/diagnostics` — Stateless multi-language diagnostics + concrete `IdeQuickFix` edits.
pub async fn diagnostics<S: IdeCtx>(
    ctx: &S,
    user: &User,
    req: IdeDiagnosticsReq,
) -> Result<IdeDiagnosticsResp> {
    let target_spec_opt = resolve_effective_target_spec(&req);
    let resolved_opt = match target_spec_opt {
        Some(spec) => {
            let resolved = ctx.ide().targets.resolve(ctx, user, &spec).await?;
            Some((resolved, spec))
        }
        None => None,
    };

    let content = match req.content.clone() {
        Some(c) => c,
        None => {
            let Some((ref resolved, _)) = resolved_opt else {
                return Err(Error::Invalid(
                    "target is required when content is omitted".into(),
                ));
            };
            ctx.ide()
                .targets
                .read_file(resolved, &req.path)
                .await?
                .content
        }
    };

    let lang = resolve_language(ctx, &req.path, req.language.as_deref());
    let mut project_roots = Vec::new();
    let mut readonly_roots = Vec::new();
    if let Some((ref resolved, _)) = resolved_opt {
        project_roots = resolved.local_project_roots();
        if let ResolvedTarget::Local {
            readonly_roots: ro, ..
        } = resolved
        {
            readonly_roots = ro.clone();
        }
    }

    // Lazily materialize toolchain/project environment variables ONLY for languages whose
    // diagnostic analyzers actually read `env_map` (`java`, `go`, `python`).
    let diags = match lang.as_str() {
        "java" => {
            let env_map = match resolved_opt {
                Some((ref resolved, ref spec)) => {
                    build_formatter_env(ctx, user, resolved, spec).await
                }
                None => BTreeMap::new(),
            };
            let preferred_java_home = env_map
                .get("JAVA_HOME")
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from);
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
            let env_map = match resolved_opt {
                Some((ref resolved, ref spec)) => {
                    build_formatter_env(ctx, user, resolved, spec).await
                }
                None => BTreeMap::new(),
            };
            analyze_go_diagnostics(
                &req.path,
                &content,
                &project_roots,
                &readonly_roots,
                &env_map,
            )
            .await
        }
        "python" => {
            let env_map = match resolved_opt {
                Some((ref resolved, ref spec)) => {
                    build_formatter_env(ctx, user, resolved, spec).await
                }
                None => BTreeMap::new(),
            };
            analyze_python_diagnostics(&req.path, &content, &env_map).await
        }
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
    if let Some(first) = req.project_folders.iter().find(|s| !s.trim().is_empty()) {
        let mut spec = IdeTargetSpec {
            kind: IdeTargetKind::Local,
            root: first.trim().to_string(),
            ..Default::default()
        };
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

/// Convert `serde_json::Error::column()` (1-based UTF-8 byte offset on the error line)
/// into a 1-based UTF-16 editor column so multi-byte keys/strings never shift squiggles.
fn byte_col_to_utf16_col(raw_ln: &str, one_based_byte_col: usize) -> u32 {
    let mut idx = one_based_byte_col.saturating_sub(1).min(raw_ln.len());
    while idx > 0 && !raw_ln.is_char_boundary(idx) {
        idx -= 1;
    }
    (raw_ln[..idx].encode_utf16().count() + 1) as u32
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
                let col = byte_col_to_utf16_col(raw, err.column());
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
    let err_raw_ln = lines.get((ln - 1) as usize).copied().unwrap_or("");
    let col = byte_col_to_utf16_col(err_raw_ln, err.column());
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
                    let comma_col = (raw_ln[..comma_byte].encode_utf16().count() + 1) as u32;
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
