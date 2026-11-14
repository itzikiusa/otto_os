//! Structured Clean Quick-Doc Hover (`POST /ide/lsp/hover`).
//!
//! Resolves the exact symbol under cursor (local buffer + multi-repo workspace +
//! Go `replace` / `$GOROOT` / `$GOMODCACHE` + Java modules / `-sources.jar` / `src.zip`)
//! and extracts ONLY that single symbol's declaration signature and immediately
//! preceding doc-comment block — never bleeding into neighboring symbols below!

use std::path::Path;

use otto_core::domain::User;
use otto_core::{Error, Result};

use crate::types::{
    IdeHoverDefinedIn, IdeHoverReq, IdeHoverResp, IdeReferencesReq, IdeTargetKind, IdeTargetSpec,
};
use crate::IdeCtx;

use super::navigation::{
    extract_symbol_and_qualifier, find_definitions_in_content, goto_definition,
};
use super::{extract_identifier_at, is_ident_char, resolve_language};

/// `POST /ide/lsp/hover` — Structured Quick-Doc Hover without neighbor comment bleed.
pub async fn hover<S: IdeCtx>(ctx: &S, user: &User, req: IdeHoverReq) -> Result<IdeHoverResp> {
    let target_spec_opt = resolve_hover_target(&req);
    let resolved_opt = match &target_spec_opt {
        Some(spec) => Some(ctx.ide().targets.resolve(ctx, user, spec).await?),
        None => None,
    };

    let content = match req.content.clone() {
        Some(c) => c,
        None => {
            let Some(ref resolved) = resolved_opt else {
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
    if content.is_empty() {
        return Ok(IdeHoverResp::default());
    }

    let lang = resolve_language(ctx, &req.path, None);
    let col_1based = normalize_hover_column(&content, req.line, req.character);
    let Some((symbol, qualifier)) =
        extract_symbol_and_qualifier(None, &lang, &content, req.line, col_1based)
    else {
        return Ok(IdeHoverResp::default());
    };

    // 1. Resolve target definition `(def_path, def_line, def_content, def_lang)`!
    let mut def_target: Option<(String, u32, String, String)> = None;

    if let (Some(spec), Some(ref resolved)) = (target_spec_opt.clone(), &resolved_opt) {
        if let Ok(def_resp) = goto_definition(
            ctx,
            user,
            IdeReferencesReq {
                target: spec,
                path: req.path.clone(),
                line: req.line,
                column: col_1based,
                symbol: None,
                content: Some(content.clone()),
                project_folders: req.project_folders.clone(),
            },
        )
        .await
        {
            if let Some(loc) = def_resp.locations.first() {
                if loc.path == req.path {
                    def_target = Some((loc.path.clone(), loc.line, content.clone(), lang.clone()));
                } else if let Ok(target_file) =
                    ctx.ide().targets.read_file(resolved, &loc.path).await
                {
                    let t_lang = resolve_language(ctx, &loc.path, Some(&target_file.language));
                    def_target = Some((loc.path.clone(), loc.line, target_file.content, t_lang));
                }
            }
        }
    }

    // Fallback to current buffer when `goto_definition` had no external target.
    if def_target.is_none() && qualifier.is_none() {
        if let Some(loc) = find_definitions_in_content(&req.path, &lang, &content, &symbol)
            .into_iter()
            .next()
        {
            def_target = Some((req.path.clone(), loc.line, content.clone(), lang.clone()));
        }
    }

    let Some((def_path, def_line, def_content, def_lang)) = def_target else {
        return Ok(IdeHoverResp::default());
    };

    Ok(extract_hover_from_definition(
        &symbol,
        &def_path,
        def_line,
        &def_lang,
        &def_content,
    ))
}

fn resolve_hover_target(req: &IdeHoverReq) -> Option<IdeTargetSpec> {
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
                || d.join("go.mod").is_file()
                || d.join("pom.xml").is_file()
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

/// Accept both 1-based `column` and 0-based `character` seamlessly.
fn normalize_hover_column(content: &str, line: u32, character: u32) -> u32 {
    if character == 0 {
        return 1;
    }
    let Some(line_str) = content.lines().nth(line.saturating_sub(1) as usize) else {
        return character;
    };
    let chars: Vec<char> = line_str.chars().collect();
    // Prefer 0-based `character + 1` when `chars[character]` is an identifier char AND
    // `chars[character - 1]` is NOT (e.g. start of `DivisionPrecision` at 0-based index `character`).
    let idx0 = character as usize;
    let idx1 = character.saturating_sub(1) as usize;
    if idx0 < chars.len()
        && is_ident_char(chars[idx0])
        && idx1 < chars.len()
        && !is_ident_char(chars[idx1])
    {
        return character + 1;
    }
    if extract_identifier_at(content, line, character).is_some() {
        character
    } else {
        character + 1
    }
}

/// Pure extractor: builds `IdeHoverResp` from `(def_path, def_line, def_lang, def_content)`
/// isolating ONLY `symbol`'s own declaration and contiguous upward doc-comment!
pub(crate) fn extract_hover_from_definition(
    symbol: &str,
    def_path: &str,
    def_line: u32,
    def_lang: &str,
    def_content: &str,
) -> IdeHoverResp {
    let lines: Vec<&str> = def_content.lines().collect();
    if lines.is_empty() || def_line == 0 || (def_line as usize) > lines.len() {
        return IdeHoverResp::default();
    }
    let idx = (def_line - 1) as usize;

    let package_name = extract_package_or_module_name(def_path, def_lang, &lines);
    let (kind, signature) = extract_clean_signature(symbol, def_lang, &lines, idx);
    let raw_doc = extract_strict_doc_comment(def_lang, &lines, idx);
    let (summary, full_doc) = summarize_doc(raw_doc.as_deref());

    IdeHoverResp {
        found: true,
        symbol: Some(symbol.to_string()),
        kind: Some(kind),
        package_name,
        signature: Some(signature),
        summary,
        full_doc,
        defined_in: Some(IdeHoverDefinedIn {
            path: def_path.to_string(),
            line: def_line,
        }),
    }
}

fn extract_package_or_module_name(path: &str, lang: &str, lines: &[&str]) -> Option<String> {
    for raw in lines.iter().take(40) {
        let t = raw.trim();
        match lang {
            "go" => {
                if let Some(pkg) = t.strip_prefix("package ") {
                    let clean = pkg.split_whitespace().next().unwrap_or("").trim();
                    if !clean.is_empty() {
                        return Some(clean.to_string());
                    }
                }
            }
            "java" | "kotlin" => {
                if let Some(pkg) = t.strip_prefix("package ") {
                    let clean = pkg.trim_end_matches(';').trim();
                    if !clean.is_empty() {
                        return Some(clean.to_string());
                    }
                }
            }
            _ => {}
        }
    }
    Path::new(path)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .map(String::from)
}

fn extract_clean_signature(
    symbol: &str,
    lang: &str,
    lines: &[&str],
    idx: usize,
) -> (String, String) {
    let raw = lines[idx].trim();
    // Strip trailing inline `//` comment on single-line declarations (`DivisionPrecision = 16 // ...`).
    let clean_code = if matches!(
        lang,
        "go" | "java" | "kotlin" | "rust" | "typescript" | "javascript"
    ) && !raw.contains('"')
    {
        raw.split("//").next().unwrap_or(raw).trim()
    } else {
        raw
    };

    if lang == "go" {
        if clean_code.starts_with("func ") {
            let kind = if clean_code.starts_with("func (") {
                "method"
            } else {
                "function"
            };
            let sig = collect_signature_until_body(lines, idx)
                .trim_end_matches('{')
                .trim()
                .to_string();
            return (kind.into(), sig);
        }
        if clean_code.starts_with("type ") {
            let kind = if clean_code.contains(" interface") {
                "interface"
            } else if clean_code.contains(" struct") {
                "struct"
            } else {
                "type"
            };
            let sig = clean_code.trim_end_matches('{').trim().to_string();
            return (kind.into(), sig);
        }
        if clean_code.starts_with("const ") {
            return ("const".into(), clean_code.to_string());
        }
        if clean_code.starts_with("var ") {
            return ("var".into(), clean_code.to_string());
        }
        // Inside a Go `const ( ... )` or `var ( ... )` group: determine `const` vs `var` by scanning upward
        // for the enclosing `const (` or `var (` header, and emit ONLY this single symbol's line!
        for up in (0..idx).rev() {
            let ut = lines[up].split("//").next().unwrap_or("").trim();
            if ut == ")" {
                break;
            }
            if ut.starts_with("const") && ut.ends_with('(') {
                return ("const".into(), format!("const {clean_code}"));
            }
            if ut.starts_with("var") && ut.ends_with('(') {
                return ("var".into(), format!("var {clean_code}"));
            }
        }
    }

    let kind = if clean_code.contains("interface ") {
        "interface"
    } else if clean_code.contains("struct ") {
        "struct"
    } else if clean_code.contains("enum ") {
        "enum"
    } else if clean_code.contains("class ") || clean_code.contains("record ") {
        "class"
    } else if clean_code.contains("fn ")
        || clean_code.contains("def ")
        || clean_code.contains("function ")
        || (clean_code.contains('(') && clean_code.contains(')'))
    {
        if matches!(lang, "java" | "kotlin") && !clean_code.contains("class ") {
            "method"
        } else {
            "function"
        }
    } else if clean_code.starts_with("const ") || clean_code.contains("static final ") {
        "const"
    } else {
        let _ = symbol;
        "symbol"
    };

    let sig = collect_signature_until_body(lines, idx)
        .trim_end_matches('{')
        .trim_end_matches(':')
        .trim()
        .to_string();
    (kind.into(), sig)
}

fn collect_signature_until_body(lines: &[&str], idx: usize) -> String {
    let first = lines[idx].trim();
    if !first.contains('(') || first.contains(')') || first.contains('{') || first.ends_with(';') {
        return first.to_string();
    }
    let mut parts = vec![first];
    for next in lines.iter().skip(idx + 1).take(5) {
        let nt = next.trim();
        parts.push(nt);
        if nt.contains('{') || nt.contains(';') || nt.ends_with(':') {
            break;
        }
    }
    parts.join(" ")
}

/// Extract ONLY the contiguous doc-comment block immediately preceding `idx`
/// (or Python triple-quoted docstring immediately following `def`/`class` at `idx`).
/// NEVER reads comments below `idx` for non-Python languages!
fn extract_strict_doc_comment(lang: &str, lines: &[&str], idx: usize) -> Option<String> {
    if lang == "python" {
        if let Some(doc) = extract_python_docstring(lines, idx) {
            return Some(doc);
        }
    }

    // Skip directly-attached annotations/attributes (`@Override`, `@Slf4j`, `#[...]`, `//go:...`) above `idx`.
    let mut cur = idx;
    while cur > 0 {
        let prev = lines[cur - 1].trim();
        if prev.starts_with('@') || prev.starts_with("#[") || prev.starts_with("//go:") {
            cur -= 1;
        } else {
            break;
        }
    }
    if cur == 0 {
        return None;
    }

    let above = lines[cur - 1].trim();

    // Case A: Block / Javadoc comment ending with `*/` immediately above declaration.
    if above.ends_with("*/") {
        let mut block_start = cur - 1;
        while block_start > 0 && !lines[block_start].trim().starts_with("/*") {
            block_start -= 1;
        }
        if lines[block_start].trim().starts_with("/*") {
            let mut cleaned = Vec::new();
            for raw_ln in &lines[block_start..cur] {
                let mut s = raw_ln.trim();
                s = s
                    .strip_prefix("/**")
                    .or_else(|| s.strip_prefix("/*"))
                    .unwrap_or(s);
                s = s.strip_suffix("*/").unwrap_or(s);
                s = s.trim_start();
                if let Some(rest) = s.strip_prefix('*') {
                    s = rest.strip_prefix(' ').unwrap_or(rest);
                }
                cleaned.push(s.trim_end().to_string());
            }
            let joined = cleaned.join("\n").trim().to_string();
            return (!joined.is_empty()).then_some(joined);
        }
    }

    // Case B: Contiguous single-line comments (`///`, `//`, `#`) immediately above declaration,
    // stopping at the FIRST blank line or non-comment code line!
    let is_line_comment = |t: &str| {
        if lang == "python" || lang == "shell" {
            t.starts_with('#') && !t.starts_with("#!")
        } else {
            t.starts_with("//") && !t.starts_with("//go:")
        }
    };
    if is_line_comment(above) {
        let mut start = cur - 1;
        while start > 0 && is_line_comment(lines[start - 1].trim()) {
            start -= 1;
        }
        let cleaned: Vec<String> = lines[start..cur]
            .iter()
            .map(|l| {
                let t = l.trim();
                let body = t
                    .strip_prefix("///")
                    .or_else(|| t.strip_prefix("//"))
                    .or_else(|| t.strip_prefix('#'))
                    .unwrap_or(t);
                body.strip_prefix(' ')
                    .unwrap_or(body)
                    .trim_end()
                    .to_string()
            })
            .collect();
        let joined = cleaned.join("\n").trim().to_string();
        return (!joined.is_empty()).then_some(joined);
    }

    None
}

fn extract_python_docstring(lines: &[&str], idx: usize) -> Option<String> {
    let mut body_idx = idx + 1;
    while body_idx < lines.len()
        && !lines[body_idx - 1].trim_end().ends_with(':')
        && body_idx <= idx + 4
    {
        body_idx += 1;
    }
    let first = lines.get(body_idx)?.trim();
    let quote = if first.starts_with("\"\"\"") {
        "\"\"\""
    } else if first.starts_with("'''") {
        "'''"
    } else {
        return None;
    };
    let rest = &first[3..];
    if let Some(end_pos) = rest.find(quote) {
        let single = rest[..end_pos].trim().to_string();
        return (!single.is_empty()).then_some(single);
    }
    let mut doc_lines = Vec::new();
    if !rest.trim().is_empty() {
        doc_lines.push(rest.trim().to_string());
    }
    for ln in lines.iter().skip(body_idx + 1).take(40) {
        let t = ln.trim();
        if let Some(before_end) = t.find(quote) {
            if !t[..before_end].trim().is_empty() {
                doc_lines.push(t[..before_end].trim().to_string());
            }
            break;
        }
        doc_lines.push(t.to_string());
    }
    let joined = doc_lines.join("\n").trim().to_string();
    (!joined.is_empty()).then_some(joined)
}

fn summarize_doc(raw: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(doc) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return (None, None);
    };
    let mut paragraphs = Vec::new();
    let mut cur = Vec::new();
    for line in doc.lines() {
        let t = line.trim();
        if t.is_empty() {
            if !cur.is_empty() {
                paragraphs.push(cur.join(" "));
                cur.clear();
            }
        } else if t.starts_with("@param")
            || t.starts_with("@return")
            || t.starts_with("@throws")
            || t.starts_with("@see")
        {
            if !cur.is_empty() {
                paragraphs.push(cur.join(" "));
                cur.clear();
            }
            paragraphs.push(t.to_string());
        } else {
            cur.push(t.to_string());
        }
    }
    if !cur.is_empty() {
        paragraphs.push(cur.join(" "));
    }
    let summary = paragraphs.first().cloned();
    (summary, Some(doc.to_string()))
}
