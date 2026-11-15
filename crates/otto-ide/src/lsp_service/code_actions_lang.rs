//! Language-specific code actions and Go import block grouping/sorting for
//! [`super::code_actions`].

use crate::types::IdeCodeAction;

use super::code_actions::{join_preserving_nl, make_action};
use super::is_valid_identifier;
use super::symbols::{match_brace_symbol, parse_python_symbols};

pub(super) fn organize_go_imports_block(lines: &[&str], content: &str) -> Option<(String, String)> {
    let open_idx = lines.iter().position(|l| l.trim() == "import (")?;
    let rel_close = lines[open_idx + 1..].iter().position(|l| l.trim() == ")")?;
    let close_idx = open_idx + 1 + rel_close;
    let body = &lines[open_idx + 1..close_idx];
    if body.iter().any(|l| l.trim().starts_with("//")) {
        return None;
    }

    let mut stdlib: Vec<String> = Vec::new();
    let mut external: Vec<String> = Vec::new();
    for &raw in body {
        let t = raw.trim();
        if t.is_empty() {
            continue;
        }
        let formatted_line = format!("\t{t}");
        let pkg_path = t
            .rsplit_once('"')
            .and_then(|(before, _)| before.rsplit_once('"').map(|(_, p)| p))
            .unwrap_or(t);
        let first_seg = pkg_path.split('/').next().unwrap_or("");
        if first_seg.contains('.') {
            external.push(formatted_line);
        } else {
            stdlib.push(formatted_line);
        }
    }
    if stdlib.len() + external.len() < 2 {
        return None;
    }
    stdlib.sort_by(|a, b| go_import_key(a).cmp(go_import_key(b)));
    stdlib.dedup();
    external.sort_by(|a, b| go_import_key(a).cmp(go_import_key(b)));
    external.dedup();

    let mut new_body: Vec<String> = Vec::new();
    new_body.extend(stdlib.clone());
    if !stdlib.is_empty() && !external.is_empty() {
        new_body.push(String::new());
    }
    new_body.extend(external);

    let orig_body: Vec<String> = body.iter().map(|s| (*s).to_string()).collect();
    if new_body == orig_body {
        return None;
    }

    let mut rebuilt: Vec<String> = Vec::with_capacity(lines.len());
    rebuilt.extend(lines[..=open_idx].iter().map(|s| (*s).to_string()));
    rebuilt.extend(new_body.clone());
    rebuilt.extend(lines[close_idx..].iter().map(|s| (*s).to_string()));
    Some((new_body.join("\n"), join_preserving_nl(&rebuilt, content)))
}

fn go_import_key(line: &str) -> &str {
    line.rsplit_once('"')
        .and_then(|(before, _)| before.rsplit_once('"').map(|(_, p)| p))
        .unwrap_or_else(|| line.trim())
}

pub(super) fn add_language_specific_actions(
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

pub(super) fn build_doc_comment_action(
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

pub(super) fn build_extract_variable_action(
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
