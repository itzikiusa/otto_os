//! Pure signature & doc-comment extraction for [`super::hover`].
//!
//! Isolates ONLY the queried symbol's own declaration and contiguous upward
//! doc-comment block without bleeding into neighboring declarations below.

use std::path::Path;

use crate::types::{IdeHoverDefinedIn, IdeHoverResp};

/// Pure extractor: builds `IdeHoverResp` from `(def_path, def_line, def_lang, def_content)`
/// isolating ONLY `symbol`'s own declaration and contiguous upward doc-comment.
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
fn extract_strict_doc_comment(lang: &str, lines: &[&str], idx: usize) -> Option<String> {
    if lang == "python" {
        if let Some(doc) = extract_python_docstring(lines, idx) {
            return Some(doc);
        }
    }

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
