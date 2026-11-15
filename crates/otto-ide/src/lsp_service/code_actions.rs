//! Concrete Text-Edit Code Actions (`POST /ide/lsp/code-actions`).
//!
//! Every returned `IdeCodeAction` carries concrete machine-applicable `edits: Vec<IdeTextEdit>`
//! (`1`-based `start_line`, `start_col`, `end_line`, `end_col`, `new_text`) so `⌥⏎` and
//! Quick-Fix buttons apply directly in `< 5ms` without forcing agent chat.

use std::sync::OnceLock;

use otto_core::domain::User;
use otto_core::Result;
use regex::Regex;

use crate::types::{
    IdeCodeAction, IdeCodeActionsReq, IdeCodeActionsResp, IdeDiagnosticInput, IdeDiagnosticsReq,
    IdeTextEdit,
};
use crate::IdeCtx;

use super::code_actions_lang::{
    add_language_specific_actions, build_doc_comment_action, build_extract_variable_action,
};
use super::{extract_identifier_at, rename_identifier_in_text, resolve_language};

/// `POST /ide/lsp/code-actions` — Context-aware Quick Fixes & Refactorings with concrete `edits`.
pub async fn code_actions<S: IdeCtx>(
    ctx: &S,
    user: &User,
    req: IdeCodeActionsReq,
) -> Result<IdeCodeActionsResp> {
    req.target.validate()?;
    let resolved_opt =
        if !req.target.root.trim().is_empty() || !user.is_root || req.content.is_none() {
            let resolved = ctx.ide().targets.resolve(ctx, user, &req.target).await?;
            let _ = ctx
                .ide()
                .targets
                .resolve_readable_or_prospective_path(&resolved, &req.path)
                .await?;
            Some(resolved)
        } else {
            None
        };
    let content = match req.content.clone() {
        Some(c) => c,
        None => {
            let resolved = resolved_opt
                .as_ref()
                .expect("resolved when content is None");
            ctx.ide()
                .targets
                .read_file(resolved, &req.path)
                .await?
                .content
        }
    };
    let lang = resolve_language(ctx, &req.path, None);
    let col = req.effective_column().max(1);
    let mut actions = Vec::new();

    if let Ok(diag_resp) = super::diagnostics::diagnostics(
        ctx,
        user,
        IdeDiagnosticsReq {
            target: Some(req.target.clone()),
            path: req.path.clone(),
            content: Some(content.clone()),
            language: Some(lang.clone()),
            project_folders: req.target.project_folders.clone(),
        },
    )
    .await
    {
        for (i, d) in diag_resp
            .diagnostics
            .into_iter()
            .filter(|d| (d.line..=d.end_line).contains(&req.line))
            .enumerate()
        {
            for (j, qf) in d.quick_fixes.into_iter().enumerate() {
                if actions.iter().any(|a: &IdeCodeAction| a.title == qf.title) {
                    continue;
                }
                let replaced = apply_edits_to_text(&content, &qf.edits);
                actions.push(IdeCodeAction {
                    id: format!("quickfix.diag.{}.{i}.{j}", d.code),
                    title: qf.title.clone(),
                    kind: "quickfix".into(),
                    is_preferred: qf.is_preferred || (i == 0 && j == 0),
                    edits: qf.edits,
                    command: None,
                    edit_preview: Some(if qf.replacement.is_empty() {
                        format!("- line {}", qf.line)
                    } else {
                        qf.replacement
                    }),
                    replacement_content: replaced,
                });
            }
        }
    }

    for action in generate_code_actions(&req.path, &lang, &content, req.line, col, &req.diagnostics)
    {
        if !actions.iter().any(|a| a.title == action.title) {
            actions.push(action);
        }
    }

    Ok(IdeCodeActionsResp { actions })
}

/// Compute minimal 1-based UTF-16 `IdeTextEdit` converting `original` into `replaced`.
pub(crate) fn compute_minimal_text_edit(
    path: &str,
    original: &str,
    replaced: &str,
) -> Vec<IdeTextEdit> {
    if original == replaced {
        return Vec::new();
    }
    let orig_bytes = original.as_bytes();
    let rep_bytes = replaced.as_bytes();
    let min_len = orig_bytes.len().min(rep_bytes.len());

    let mut prefix = 0usize;
    while prefix < min_len && orig_bytes[prefix] == rep_bytes[prefix] {
        prefix += 1;
    }
    while prefix > 0 && (!original.is_char_boundary(prefix) || !replaced.is_char_boundary(prefix)) {
        prefix -= 1;
    }
    if prefix > 0
        && orig_bytes.get(prefix - 1) == Some(&b'\r')
        && (orig_bytes.get(prefix) == Some(&b'\n') || rep_bytes.get(prefix) == Some(&b'\n'))
    {
        prefix -= 1;
    }

    let mut orig_end = orig_bytes.len();
    let mut rep_end = rep_bytes.len();
    while orig_end > prefix
        && rep_end > prefix
        && orig_bytes[orig_end - 1] == rep_bytes[rep_end - 1]
    {
        orig_end -= 1;
        rep_end -= 1;
    }
    while orig_end < orig_bytes.len()
        && rep_end < rep_bytes.len()
        && (!original.is_char_boundary(orig_end) || !replaced.is_char_boundary(rep_end))
    {
        orig_end += 1;
        rep_end += 1;
    }
    if orig_end < orig_bytes.len()
        && rep_end < rep_bytes.len()
        && orig_bytes.get(orig_end) == Some(&b'\n')
        && ((orig_end > prefix && orig_bytes.get(orig_end - 1) == Some(&b'\r'))
            || (rep_end > prefix && rep_bytes.get(rep_end - 1) == Some(&b'\r')))
    {
        orig_end += 1;
        rep_end += 1;
    }

    let (start_line, start_col) = crate::text_pos::byte_offset_to_line_utf16_col(original, prefix);
    let (end_line, end_col) = crate::text_pos::byte_offset_to_line_utf16_col(original, orig_end);
    let new_text = replaced[prefix..rep_end].to_string();

    vec![IdeTextEdit {
        path: path.to_string(),
        start_line,
        start_col,
        end_line,
        end_col,
        new_text,
    }]
}

pub(crate) fn apply_edits_to_text(original: &str, edits: &[IdeTextEdit]) -> Option<String> {
    if edits.is_empty() {
        return None;
    }
    let mut resolved: Vec<(usize, usize, &str)> = Vec::with_capacity(edits.len());
    for e in edits {
        let start_off =
            crate::text_pos::line_utf16_col_to_byte_offset(original, e.start_line, e.start_col);
        let end_off =
            crate::text_pos::line_utf16_col_to_byte_offset(original, e.end_line, e.end_col);
        if start_off > end_off || end_off > original.len() {
            return None;
        }
        resolved.push((start_off, end_off, e.new_text.as_str()));
    }
    resolved.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
    for pair in resolved.windows(2) {
        if pair[0].1 > pair[1].0 {
            return None;
        }
    }
    let mut out = original.to_string();
    for (start_off, end_off, new_text) in resolved.into_iter().rev() {
        out.replace_range(start_off..end_off, new_text);
    }
    Some(out)
}

pub(super) fn make_action(
    path: &str,
    original: &str,
    id: impl Into<String>,
    title: impl Into<String>,
    kind: &str,
    preview: String,
    replaced: String,
) -> IdeCodeAction {
    let edits = compute_minimal_text_edit(path, original, &replaced);
    IdeCodeAction {
        id: id.into(),
        title: title.into(),
        kind: kind.into(),
        is_preferred: kind == "quickfix",
        edits,
        command: None,
        edit_preview: Some(preview),
        replacement_content: Some(replaced),
    }
}

pub fn generate_code_actions(
    path: &str,
    lang: &str,
    content: &str,
    line: u32,
    column: u32,
    diagnostics: &[IdeDiagnosticInput],
) -> Vec<IdeCodeAction> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let idx = (line.saturating_sub(1) as usize).min(lines.len() - 1);
    let mut actions = Vec::new();

    add_diagnostic_quick_fixes(
        path,
        lang,
        &lines,
        idx,
        column,
        content,
        diagnostics,
        &mut actions,
    );
    add_manifest_version_actions(path, &lines, idx, content, &mut actions);
    if let Some((p, r)) = build_organize_imports_action(lang, &lines, content) {
        actions.push(make_action(
            path,
            content,
            "refactor.organize_imports",
            "Organize and sort imports",
            "source.organizeImports",
            p,
            r,
        ));
    }
    if let Some((p, r)) = build_doc_comment_action(lang, &lines, idx, content) {
        actions.push(make_action(
            path,
            content,
            "refactor.add_doc_comment",
            "Add documentation comment",
            "refactor.rewrite",
            p,
            r,
        ));
    }
    if let Some((p, r)) = build_extract_variable_action(lang, &lines, idx, content) {
        actions.push(make_action(
            path,
            content,
            "refactor.extract_variable",
            "Extract expression to local variable",
            "refactor.extract",
            p,
            r,
        ));
    }
    add_language_specific_actions(path, lang, &lines, idx, lines[idx], content, &mut actions);
    actions
}

pub(super) fn join_preserving_nl(lines: &[String], original: &str) -> String {
    let eol = crate::text_pos::detect_eol(original);
    let mut out = lines.join(eol);
    if original.ends_with('\n') && !out.is_empty() {
        out.push_str(eol);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn add_diagnostic_quick_fixes(
    path: &str,
    lang: &str,
    lines: &[&str],
    idx: usize,
    column: u32,
    content: &str,
    diagnostics: &[IdeDiagnosticInput],
    out: &mut Vec<IdeCodeAction>,
) {
    let ln = (idx + 1) as u32;
    for diag in diagnostics {
        if diag.line != ln {
            continue;
        }
        let msg_lower = diag.message.to_ascii_lowercase();
        if msg_lower.contains("unused") || msg_lower.contains("unresolved import") {
            if msg_lower.contains("unused") {
                if let Some(ident) = extract_identifier_at(content, ln, column.max(1))
                    .filter(|s| !s.starts_with('_'))
                {
                    if let Some((replaced, _)) =
                        rename_identifier_in_text(path, lang, content, &ident, &format!("_{ident}"))
                    {
                        out.push(make_action(
                            path,
                            content,
                            format!("quickfix.prefix_unused.{ident}"),
                            format!("Prefix unused '{ident}' with '_'"),
                            "quickfix",
                            format!("{ident} -> _{ident}"),
                            replaced,
                        ));
                    }
                }
            }
            let mut without: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
            let removed = without.remove(idx);
            out.push(make_action(
                path,
                content,
                "quickfix.remove_unused_line",
                "Remove unused statement / import",
                "quickfix",
                format!("- {}", removed.trim()),
                join_preserving_nl(&without, content),
            ));
        }
    }
}

fn add_manifest_version_actions(
    path: &str,
    lines: &[&str],
    idx: usize,
    content: &str,
    out: &mut Vec<IdeCodeAction>,
) {
    let fname = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !matches!(
        fname,
        "go.mod"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "package.json"
            | "Cargo.toml"
            | "requirements.txt"
            | "pyproject.toml"
    ) {
        return;
    }
    static RE_SEMVER: OnceLock<Regex> = OnceLock::new();
    let re = RE_SEMVER.get_or_init(|| Regex::new(r"(v?\d+)\.(\d+)\.(\d+)").unwrap());
    let cur = lines[idx];
    if let Some(cap) = re.captures(cur) {
        let m = cap.get(0).unwrap();
        let patch: u64 = cap[3].parse().unwrap_or(0);
        let bumped = format!("{}.{}.{}", &cap[1], &cap[2], patch + 1);
        let mut updated: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
        updated[idx] = format!("{}{}{}", &cur[..m.start()], bumped, &cur[m.end()..]);
        out.push(make_action(
            path,
            content,
            "quickfix.deps.bump_patch",
            format!("Bump dependency version {} -> {bumped}", m.as_str()),
            "quickfix",
            format!("{} -> {bumped}", m.as_str()),
            join_preserving_nl(&updated, content),
        ));
    }
}

pub(super) fn build_organize_imports_action(
    lang: &str,
    lines: &[&str],
    content: &str,
) -> Option<(String, String)> {
    if lang == "go" {
        return super::code_actions_lang::organize_go_imports_block(lines, content);
    }
    let is_import = |l: &str| {
        let t = l.trim();
        match lang {
            "rust" => t.starts_with("use ") && t.ends_with(';'),
            "java" | "kotlin" => t.starts_with("import "),
            "javascript" | "typescript" => t.starts_with("import ") && t.contains("from "),
            "python" => t.starts_with("import ") || t.starts_with("from "),
            _ => false,
        }
    };
    let mut start = None;
    let mut end = 0usize;
    for (i, &l) in lines.iter().enumerate() {
        if is_import(l) {
            start.get_or_insert(i);
            end = i;
        } else if start.is_some() && !l.trim().is_empty() {
            break;
        }
    }
    let s = start?;
    let orig: Vec<&str> = lines[s..=end]
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if orig.len() < 2 {
        return None;
    }
    let mut sorted = orig.clone();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted == orig && (end - s + 1) == sorted.len() {
        return None;
    }
    let mut rebuilt: Vec<String> = Vec::with_capacity(lines.len());
    rebuilt.extend(lines[..s].iter().map(|v| (*v).to_string()));
    rebuilt.extend(sorted.iter().map(|v| (*v).to_string()));
    rebuilt.extend(lines[end + 1..].iter().map(|v| (*v).to_string()));
    Some((sorted.join("\n"), join_preserving_nl(&rebuilt, content)))
}
