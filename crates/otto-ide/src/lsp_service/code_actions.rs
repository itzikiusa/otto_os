//! Concrete Text-Edit Code Actions (`POST /ide/lsp/code-actions`, `#77`).
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

use super::symbols::{match_brace_symbol, parse_python_symbols};
use super::{
    extract_identifier_at, is_valid_identifier, rename_identifier_in_text, resolve_language,
};

/// `POST /ide/lsp/code-actions` — Context-aware Quick Fixes & Refactorings with concrete `edits`.
pub async fn code_actions<S: IdeCtx>(
    ctx: &S,
    user: &User,
    req: IdeCodeActionsReq,
) -> Result<IdeCodeActionsResp> {
    req.target.validate()?;
    let content = match req.content.clone() {
        Some(c) => c,
        None => {
            let resolved = ctx.ide().targets.resolve(ctx, user, &req.target).await?;
            ctx.ide()
                .targets
                .read_file(&resolved, &req.path)
                .await?
                .content
        }
    };
    let lang = resolve_language(ctx, &req.path, None);
    let mut actions = Vec::new();

    // 1. Synthesize concrete Quick Fixes from live language diagnostics at `req.line`!
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

    // 2. Append deterministic AST/line code actions & caller-supplied diagnostic quick fixes.
    for action in generate_code_actions(
        &req.path,
        &lang,
        &content,
        req.line,
        req.column,
        &req.diagnostics,
    ) {
        if !actions.iter().any(|a| a.title == action.title) {
            actions.push(action);
        }
    }

    Ok(IdeCodeActionsResp { actions })
}

/// Compute minimal 1-based `IdeTextEdit` converting `original` into `replaced`.
pub(crate) fn compute_minimal_text_edit(
    path: &str,
    original: &str,
    replaced: &str,
) -> Vec<IdeTextEdit> {
    if original == replaced {
        return Vec::new();
    }
    let orig_chars: Vec<char> = original.chars().collect();
    let rep_chars: Vec<char> = replaced.chars().collect();
    let min_len = orig_chars.len().min(rep_chars.len());

    let mut prefix = 0usize;
    while prefix < min_len && orig_chars[prefix] == rep_chars[prefix] {
        prefix += 1;
    }

    let mut orig_end = orig_chars.len();
    let mut rep_end = rep_chars.len();
    while orig_end > prefix
        && rep_end > prefix
        && orig_chars[orig_end - 1] == rep_chars[rep_end - 1]
    {
        orig_end -= 1;
        rep_end -= 1;
    }

    let (start_line, start_col) = char_offset_to_line_col(&orig_chars, prefix);
    let (end_line, end_col) = char_offset_to_line_col(&orig_chars, orig_end);
    let new_text: String = rep_chars[prefix..rep_end].iter().collect();

    vec![IdeTextEdit {
        path: path.to_string(),
        start_line,
        start_col,
        end_line,
        end_col,
        new_text,
    }]
}

fn char_offset_to_line_col(chars: &[char], offset: usize) -> (u32, u32) {
    let mut line = 1u32;
    let mut col = 1u32;
    for &ch in chars.iter().take(offset) {
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn apply_edits_to_text(original: &str, edits: &[IdeTextEdit]) -> Option<String> {
    let first = edits.first()?;
    if edits.len() > 1 {
        return None;
    }
    let chars: Vec<char> = original.chars().collect();
    let start_off = line_col_to_char_offset(&chars, first.start_line, first.start_col);
    let end_off = line_col_to_char_offset(&chars, first.end_line, first.end_col);
    if start_off > end_off || end_off > chars.len() {
        return None;
    }
    let mut out = String::with_capacity(original.len() + first.new_text.len());
    out.extend(&chars[..start_off]);
    out.push_str(&first.new_text);
    out.extend(&chars[end_off..]);
    Some(out)
}

fn line_col_to_char_offset(chars: &[char], target_line: u32, target_col: u32) -> usize {
    let mut line = 1u32;
    let mut col = 1u32;
    for (idx, &ch) in chars.iter().enumerate() {
        if line == target_line && col == target_col {
            return idx;
        }
        if ch == '\n' {
            if line == target_line {
                return idx;
            }
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    chars.len()
}

fn make_action(
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

fn join_preserving_nl(lines: &[String], original: &str) -> String {
    let mut out = lines.join("\n");
    if original.ends_with('\n') && !out.is_empty() {
        out.push('\n');
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

fn build_organize_imports_action(
    lang: &str,
    lines: &[&str],
    content: &str,
) -> Option<(String, String)> {
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

fn build_doc_comment_action(
    lang: &str,
    lines: &[&str],
    idx: usize,
    content: &str,
) -> Option<(String, String)> {
    let line = lines[idx];
    let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    if idx > 0
        && ["///", "//", "*/", "#"]
            .iter()
            .any(|p| lines[idx - 1].trim().starts_with(p))
    {
        return None;
    }
    let sym_name = if lang == "python" {
        parse_python_symbols(&[line])
            .into_iter()
            .next()
            .map(|s| s.name)
    } else {
        match_brace_symbol(lang, line.trim()).map(|(n, _, _, _)| n)
    }?;
    let mut updated: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
    if lang == "python" {
        let doc = format!("{indent}    \"\"\"TODO: Document {sym_name}.\"\"\"");
        updated.insert(idx + 1, doc.clone());
        return Some((doc, join_preserving_nl(&updated, content)));
    }
    let comment = match lang {
        "rust" => format!("{indent}/// Documentation for `{sym_name}`."),
        "go" => format!("{indent}// {sym_name} performs ..."),
        _ => format!("{indent}/** Documentation for `{sym_name}`. */"),
    };
    updated.insert(idx, comment.clone());
    Some((comment, join_preserving_nl(&updated, content)))
}

fn build_extract_variable_action(
    lang: &str,
    lines: &[&str],
    idx: usize,
    content: &str,
) -> Option<(String, String)> {
    let line = lines[idx];
    let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
    let expr = line
        .trim()
        .strip_prefix("return ")?
        .trim()
        .trim_end_matches(';')
        .trim();
    if expr.is_empty() || is_valid_identifier(expr) {
        return None;
    }
    let (decl, ret) = match lang {
        "go" => (
            format!("{indent}result := {expr}"),
            format!("{indent}return result"),
        ),
        "rust" => (
            format!("{indent}let result = {expr};"),
            format!("{indent}result"),
        ),
        "python" => (
            format!("{indent}result = {expr}"),
            format!("{indent}return result"),
        ),
        "java" => (
            format!("{indent}final var result = {expr};"),
            format!("{indent}return result;"),
        ),
        _ => (
            format!("{indent}const result = {expr};"),
            format!("{indent}return result;"),
        ),
    };
    let mut updated: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
    updated[idx] = decl.clone();
    updated.insert(idx + 1, ret.clone());
    Some((
        format!("{decl}\n{ret}"),
        join_preserving_nl(&updated, content),
    ))
}

fn add_language_specific_actions(
    path: &str,
    lang: &str,
    lines: &[&str],
    idx: usize,
    current_line: &str,
    content: &str,
    out: &mut Vec<IdeCodeAction>,
) {
    let indent: String = current_line
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    let trimmed = current_line.trim();
    if lang == "rust" {
        if current_line.contains(".unwrap()") {
            let replaced = current_line.replacen(".unwrap()", "?", 1);
            let mut updated: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
            updated[idx] = replaced.clone();
            out.push(make_action(
                path,
                content,
                "quickfix.rust.unwrap_to_question",
                "Replace .unwrap() with '?' error propagation",
                "quickfix",
                replaced.trim().to_string(),
                join_preserving_nl(&updated, content),
            ));
        }
        if (trimmed.starts_with("pub struct ")
            || trimmed.starts_with("struct ")
            || trimmed.starts_with("pub enum ")
            || trimmed.starts_with("enum "))
            && !idx
                .checked_sub(1)
                .is_some_and(|p| lines[p].trim().starts_with("#[derive("))
        {
            let mut updated: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
            updated.insert(idx, format!("{indent}#[derive(Debug, Clone)]"));
            out.push(make_action(
                path,
                content,
                "quickfix.rust.derive_debug_clone",
                "Add #[derive(Debug, Clone)]",
                "quickfix",
                "#[derive(Debug, Clone)]".into(),
                join_preserving_nl(&updated, content),
            ));
        }
    }
    if lang == "java"
        && (trimmed.starts_with("public ") || trimmed.starts_with("protected "))
        && trimmed.contains('(')
        && !trimmed.contains("class ")
        && !idx
            .checked_sub(1)
            .is_some_and(|p| lines[p].trim() == "@Override")
    {
        let mut updated: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
        updated.insert(idx, format!("{indent}@Override"));
        out.push(make_action(
            path,
            content,
            "refactor.java.add_override",
            "Add @Override annotation",
            "refactor.rewrite",
            "@Override".into(),
            join_preserving_nl(&updated, content),
        ));
    }
    if lang == "go" {
        if (trimmed.contains(", err :=")
            || trimmed.starts_with("err :=")
            || trimmed.starts_with("err ="))
            && !lines
                .get(idx + 1)
                .is_some_and(|n| n.trim().starts_with("if err != nil"))
        {
            let mut updated: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
            updated.insert(
                idx + 1,
                format!("{indent}if err != nil {{\n{indent}\treturn err\n{indent}}}"),
            );
            out.push(make_action(
                path,
                content,
                "refactor.go.wrap_err_check",
                "Add 'if err != nil' error guard",
                "refactor.rewrite",
                "if err != nil { return err }".into(),
                join_preserving_nl(&updated, content),
            ));
        }
        // Go struct field JSON tag quick fix (`FieldName Type` -> ``FieldName Type `json:"field_name"` ``).
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() == 2
            && parts[0]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase())
            && !trimmed.contains('`')
            && !trimmed.starts_with("type ")
            && !trimmed.starts_with("func ")
        {
            let snake = to_snake_case(parts[0]);
            let mut updated: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
            updated[idx] = format!("{current_line} `json:\"{snake}\"`");
            out.push(make_action(
                path,
                content,
                "refactor.go.add_json_tag",
                format!("Add `json:\"{snake}\"` struct tag"),
                "refactor.rewrite",
                format!("`json:\"{snake}\"`"),
                join_preserving_nl(&updated, content),
            ));
        }
    }
    let fn_name = if lang == "python" {
        parse_python_symbols(&[current_line])
            .into_iter()
            .find(|s| s.kind == "function" || s.kind == "method")
            .map(|s| s.name)
    } else {
        match_brace_symbol(lang, trimmed)
            .filter(|(_, k, _, _)| k == "function" || k == "method")
            .map(|(n, _, _, _)| n)
    };
    if let Some(name) =
        fn_name.filter(|n| !n.to_ascii_lowercase().starts_with("test") && n != "main")
    {
        let mut chars = name.chars();
        let cap_name = chars
            .next()
            .map(|f| format!("{}{}", f.to_ascii_uppercase(), chars.as_str()))
            .unwrap_or_else(|| name.clone());
        let stub = match lang {
            "go" => Some(format!(
                "\nfunc Test{cap_name}(t *testing.T) {{\n\t// TODO: test {name}\n}}\n"
            )),
            "rust" => Some(format!(
                "\n#[test]\nfn test_{name}() {{\n    // TODO: test {name}\n}}\n"
            )),
            "python" => Some(format!(
                "\ndef test_{name}():\n    # TODO: test {name}\n    assert True\n"
            )),
            "javascript" | "typescript" => Some(format!(
                "\ntest('{name}', () => {{\n  // TODO: test {name}\n}});\n"
            )),
            _ => None,
        };
        if let Some(snippet) = stub {
            out.push(make_action(
                path,
                content,
                format!("refactor.generate_test.{name}"),
                format!("Generate unit test stub for '{name}'"),
                "refactor.TestStub",
                snippet.trim().to_string(),
                format!("{}{}", content.trim_end(), snippet),
            ));
        }
    }
    if matches!(lang, "shell" | "bash" | "sh")
        && !lines
            .iter()
            .take(8)
            .any(|l| l.contains("set -e") || l.contains("pipefail"))
    {
        let mut updated: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
        let insert_at = usize::from(lines.first().is_some_and(|l| l.starts_with("#!")));
        updated.insert(insert_at, "set -euo pipefail".into());
        out.push(make_action(
            path,
            content,
            "quickfix.shell.strict_mode",
            "Add 'set -euo pipefail' strict shell guard",
            "quickfix",
            "+ set -euo pipefail".into(),
            join_preserving_nl(&updated, content),
        ));
    }
    if lang == "json" {
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(content) {
            if let Ok(compact) = serde_json::to_string(&val) {
                let compact_nl = format!("{compact}\n");
                if compact_nl != content {
                    out.push(make_action(
                        path,
                        content,
                        "refactor.json.minify",
                        "Minify JSON to single line",
                        "refactor.rewrite",
                        compact.chars().take(60).collect(),
                        compact_nl,
                    ));
                }
            }
        }
    }
}

fn to_snake_case(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 4);
    for (i, ch) in input.chars().enumerate() {
        if ch.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}
