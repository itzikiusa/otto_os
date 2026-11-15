//! Precompiled definition/implementation line matchers and cursor qualifier
//! extraction for [`super::navigation`].

use std::sync::OnceLock;

use regex::Regex;

use crate::types::IdeReferenceLocation;

use super::{code_mask_for_line, extract_identifier_at, is_ident_char};

/// Precompiled `(lang, symbol)` regex pair reused across all candidate files and
/// lines during `goto_definition` so `Regex::new` runs once per request rather
/// than once per matching line.
pub(crate) struct CompiledDefMatcher {
    lang: String,
    def_re: Regex,
    go_group_re: Option<Regex>,
}

impl CompiledDefMatcher {
    pub(crate) fn new(lang: &str, sym: &str) -> Option<Self> {
        let escaped = regex::escape(sym);
        let go_group_re = if lang == "go" {
            Regex::new(&format!(
                r"^{escaped}(?:\s+[A-Za-z0-9_*.\[\]]+)?\s*(?:=.*)?$"
            ))
            .ok()
        } else {
            None
        };
        let pat = match lang {
            "go" => format!(
                r"^(?:func\s+(?:\([^)]+\)\s*)?{escaped}\s*(?:\[[^\]]*\])?\s*\(|type\s+{escaped}\b|(?:const|var)\s+{escaped}\b|{escaped}\s*:=)"
            ),
            "rust" => format!(
                r"^(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:fn|struct|enum|trait|type|const|static|mod|union)\s+{escaped}\b|^macro_rules!\s*{escaped}\b"
            ),
            "python" => {
                format!(r"^(?:async\s+)?(?:def|class)\s+{escaped}\b|^{escaped}\s*(?::[^=]+)?=")
            }
            "java" | "kotlin" => format!(
                r"^(?:(?:public|private|protected|static|final|abstract|synchronized|default|sealed|open|data|internal|override|suspend)\s+)*(?:class|interface|enum|record|object|fun)\s+{escaped}\b|^(?:(?:public|private|protected|static|final|synchronized|abstract|default)\s+)*(?:<[^>]+>\s+)?[A-Za-z0-9_<>,.\[\]?]+\s+{escaped}\s*\(|^(?:(?:public|private|protected|static|final|volatile|transient)\s+)+[A-Za-z0-9_<>,.\[\]?]+\s+{escaped}\s*(?:=|;)"
            ),
            "shell" | "bash" | "sh" | "zsh" => format!(
                r"^(?:function\s+)?{escaped}\s*\(\s*\)|^function\s+{escaped}(?:\s*\{{|\s*$)|^(?:export\s+|readonly\s+|local\s+)?{escaped}="
            ),
            _ => format!(
                r"^(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\s*\*?|class|interface|type|enum|namespace|const|let|var)\s+{escaped}\b|^(?:(?:public|private|protected|static|readonly|async|get|set)\s+)*{escaped}\s*(?:<[^>]*>)?\s*\("
            ),
        };
        let def_re = Regex::new(&pat).ok()?;
        Some(Self {
            lang: lang.to_string(),
            def_re,
            go_group_re,
        })
    }

    pub(crate) fn matches_line(&self, trimmed: &str, in_go_decl_group: bool) -> bool {
        if in_go_decl_group {
            if let Some(ref group_re) = self.go_group_re {
                if group_re.is_match(trimmed) {
                    return true;
                }
            }
        }
        self.def_re.is_match(trimmed)
    }
}

pub(crate) fn find_definitions_in_content(
    path: &str,
    lang: &str,
    content: &str,
    symbol: &str,
) -> Vec<IdeReferenceLocation> {
    let Some(matcher) = CompiledDefMatcher::new(lang, symbol) else {
        return Vec::new();
    };
    find_definitions_with_matcher(path, content, symbol, &matcher)
}

pub(super) fn find_definitions_with_matcher(
    path: &str,
    content: &str,
    symbol: &str,
    matcher: &CompiledDefMatcher,
) -> Vec<IdeReferenceLocation> {
    let lang = matcher.lang.as_str();
    let mut out = Vec::new();
    let mut in_block = false;
    let mut go_decl_group = false;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mask = code_mask_for_line(lang, line, &mut in_block);
        let masked_code: String = line
            .char_indices()
            .map(|(bi, ch)| {
                if mask.get(bi).copied().unwrap_or(false) {
                    ch
                } else {
                    ' '
                }
            })
            .collect();
        let code_head = masked_code.trim();
        if code_head.is_empty() {
            continue;
        }
        if lang == "go" {
            if (code_head.starts_with("const") || code_head.starts_with("var"))
                && code_head.ends_with('(')
            {
                go_decl_group = true;
                continue;
            }
            if go_decl_group && code_head == ")" {
                go_decl_group = false;
                continue;
            }
        }
        if !masked_code.contains(symbol) {
            continue;
        }
        let is_def = matcher.matches_line(code_head, go_decl_group)
            || (code_head.contains(';')
                && code_head
                    .split(';')
                    .map(str::trim)
                    .filter(|stmt| !stmt.is_empty())
                    .any(|stmt| matcher.matches_line(stmt, go_decl_group)));
        if is_def {
            let col = find_symbol_column(line, &mask, symbol).unwrap_or(1);
            out.push(IdeReferenceLocation {
                path: path.to_string(),
                line: (idx + 1) as u32,
                column: col,
                preview: trimmed.to_string(),
            });
        }
    }
    out
}

/// Find ONLY package-level Go declarations (`brace_depth == 0`: `var`, `const`,
/// `type`, `func`, and grouped `var (...)` / `const (...)` / `type (...)`),
/// NEVER matching function-local `:=` or local variables inside `{ ... }`.
pub(crate) fn find_go_package_level_definitions(
    path: &str,
    content: &str,
    symbol: &str,
    allow_receiver_methods: bool,
) -> Vec<IdeReferenceLocation> {
    let escaped = regex::escape(symbol);
    let fn_pat = if allow_receiver_methods {
        format!(r"^func\s+(?:\([^)]+\)\s*)?{escaped}\s*(?:\[[^\]]*\])?\s*\(")
    } else {
        format!(r"^func\s+{escaped}\s*(?:\[[^\]]*\])?\s*\(")
    };
    let top_pat = format!(
        r"{fn_pat}|^type\s+{escaped}\b|^(?:const|var)\s+(?:[A-Za-z_][A-Za-z0-9_]*\s*,\s*)*{escaped}\b"
    );
    let group_pat = format!(r"^(?:[A-Za-z_][A-Za-z0-9_]*\s*,\s*)*{escaped}(?:\s+|\s*=|$)");
    let (Ok(top_re), Ok(group_re)) = (Regex::new(&top_pat), Regex::new(&group_pat)) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    let mut in_block = false;
    let mut brace_depth: i32 = 0;
    let mut in_decl_group = false;

    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mask = code_mask_for_line("go", line, &mut in_block);
        let masked_code: String = line
            .char_indices()
            .map(|(bi, ch)| {
                if mask.get(bi).copied().unwrap_or(false) {
                    ch
                } else {
                    ' '
                }
            })
            .collect();
        let code_head = masked_code.trim();
        if code_head.is_empty() {
            continue;
        }

        if brace_depth == 0 {
            if (code_head.starts_with("const")
                || code_head.starts_with("var")
                || code_head.starts_with("type"))
                && code_head.ends_with('(')
            {
                in_decl_group = true;
            } else if in_decl_group && (code_head == ")" || code_head.starts_with(')')) {
                in_decl_group = false;
            } else if masked_code.contains(symbol) {
                let matched = if in_decl_group {
                    group_re.is_match(code_head)
                } else {
                    top_re.is_match(code_head)
                };
                if matched {
                    let col = find_symbol_column(line, &mask, symbol).unwrap_or(1);
                    out.push(IdeReferenceLocation {
                        path: path.to_string(),
                        line: (idx + 1) as u32,
                        column: col,
                        preview: trimmed.to_string(),
                    });
                }
            }
        }

        for ch in masked_code.chars() {
            if ch == '{' {
                brace_depth += 1;
            } else if ch == '}' {
                brace_depth = (brace_depth - 1).max(0);
            }
        }
    }
    out
}

#[allow(dead_code)]
pub(crate) fn is_definition_line(
    lang: &str,
    trimmed: &str,
    sym: &str,
    in_go_decl_group: bool,
) -> bool {
    CompiledDefMatcher::new(lang, sym).is_some_and(|m| m.matches_line(trimmed, in_go_decl_group))
}

pub(super) fn find_implementations_in_content(
    path: &str,
    lang: &str,
    content: &str,
    symbol: &str,
    go_iface_methods: &[String],
) -> Vec<IdeReferenceLocation> {
    let mut out = Vec::new();
    let escaped = regex::escape(symbol);
    let pat = match lang {
        "go" => {
            if !go_iface_methods.is_empty() {
                let alts = go_iface_methods
                    .iter()
                    .map(|m| regex::escape(m))
                    .collect::<Vec<_>>()
                    .join("|");
                format!(r"^func\s*\([^)]+\)\s*(?:{alts})\s*(?:\[[^\]]*\])?\s*\(")
            } else {
                format!(r"^func\s*\([^)]+\)\s*{escaped}\s*(?:\[[^\]]*\])?\s*\(")
            }
        }
        "rust" => format!(
            r"^(?:unsafe\s+)?impl(?:<[^>]*>)?\s+(?:[A-Za-z0-9_:]+::)*{escaped}(?:<[^>]*>)?\s+for\b|^\s*(?:pub\s+)?(?:async\s+)?fn\s+{escaped}\s*(?:<[^>]*>)?\s*\("
        ),
        "java" | "kotlin" => format!(
            r"\b(?:implements|extends)\s+[^{{]*\b{escaped}\b|^(?:(?:public|protected|private|override|open)\s+)+[A-Za-z0-9_<>,.\[\]?\s]+\b{escaped}\s*\("
        ),
        "python" => format!(
            r"^class\s+[A-Za-z_][A-Za-z0-9_]*\s*\([^)]*\b{escaped}\b|^\s+(?:async\s+)?def\s+{escaped}\s*\("
        ),
        _ => format!(
            r"\b(?:implements|extends)\s+[^{{]*\b{escaped}\b|^\s*(?:public\s+|async\s+)*{escaped}\s*\("
        ),
    };
    let Ok(re) = Regex::new(&pat) else {
        return out;
    };
    let mut in_block = false;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let mask = code_mask_for_line(lang, line, &mut in_block);
        if re.is_match(trimmed) {
            let col = find_symbol_column(line, &mask, symbol).unwrap_or(1);
            out.push(IdeReferenceLocation {
                path: path.to_string(),
                line: (idx + 1) as u32,
                column: col,
                preview: trimmed.to_string(),
            });
        }
    }
    out
}

pub(super) fn extract_go_interface_methods(content: &str, iface_name: &str) -> Vec<String> {
    let header_re = Regex::new(&format!(
        r"^type\s+{}\s+interface\s*\{{",
        regex::escape(iface_name)
    ))
    .ok();
    let Some(re) = header_re else {
        return Vec::new();
    };
    let mut in_iface = false;
    let mut methods = Vec::new();
    static RE_M: OnceLock<Regex> = OnceLock::new();
    let m_re = RE_M.get_or_init(|| Regex::new(r"^([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap());
    for line in content.lines() {
        let t = line.split("//").next().unwrap_or("").trim();
        if !in_iface {
            if re.is_match(t) {
                in_iface = true;
            }
            continue;
        }
        if t.starts_with('}') {
            break;
        }
        if let Some(cap) = m_re.captures(t) {
            methods.push(cap[1].to_string());
        }
    }
    methods
}

pub(crate) fn extract_symbol_and_qualifier(
    explicit: Option<&str>,
    lang: &str,
    content: &str,
    line: u32,
    column: u32,
) -> Option<(String, Option<String>)> {
    let explicit_clean = explicit.map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = explicit_clean {
        if let Some((q, sym)) = s.rsplit_once("::").or_else(|| s.rsplit_once('.')) {
            return Some((sym.to_string(), Some(q.to_string())));
        }
    }
    let sym = match explicit_clean {
        Some(s) => s.to_string(),
        None => {
            if lang == "shell" {
                extract_shell_ident_at(content, line, column)?
            } else {
                extract_identifier_at(content, line, column)?
            }
        }
    };
    let Some(line_str) = content.lines().nth(line.saturating_sub(1) as usize) else {
        return Some((sym, None));
    };
    let chars: Vec<char> = line_str.chars().collect();
    if chars.is_empty() {
        return Some((sym, None));
    }
    let mut pos = (column.saturating_sub(1) as usize).min(chars.len().saturating_sub(1));
    while pos > 0 && (is_ident_char(chars[pos - 1]) || (lang == "shell" && chars[pos - 1] == '-')) {
        pos -= 1;
    }
    let qual_end = if pos >= 1 && chars[pos - 1] == '.' {
        Some(pos - 1)
    } else if pos >= 2 && chars[pos - 2] == ':' && chars[pos - 1] == ':' {
        Some(pos - 2)
    } else {
        None
    };
    let mut qualifier = qual_end.and_then(|qe| {
        if qe == 0 {
            return None;
        }
        let mut qs = qe;
        while qs > 0 && is_ident_char(chars[qs - 1]) {
            qs -= 1;
        }
        (qs < qe).then(|| chars[qs..qe].iter().collect::<String>())
    });

    if qualifier.is_none() && explicit_clean.is_some() {
        for sep in ["::", "."] {
            let needle = format!("{sep}{sym}");
            let mut search_from = 0usize;
            while let Some(rel_idx) = line_str[search_from..].find(&needle) {
                let sep_byte = search_from + rel_idx;
                let after_byte = sep_byte + needle.len();
                let boundary_ok = line_str[after_byte..]
                    .chars()
                    .next()
                    .is_none_or(|c| !is_ident_char(c));
                if boundary_ok && sep_byte > 0 {
                    let prefix_chars: Vec<char> = line_str[..sep_byte].chars().collect();
                    let qe = prefix_chars.len();
                    let mut qs = qe;
                    while qs > 0 && is_ident_char(prefix_chars[qs - 1]) {
                        qs -= 1;
                    }
                    if qs < qe {
                        qualifier = Some(prefix_chars[qs..qe].iter().collect());
                        break;
                    }
                }
                search_from = after_byte;
            }
            if qualifier.is_some() {
                break;
            }
        }
    }

    Some((sym, qualifier))
}

fn extract_shell_ident_at(content: &str, line: u32, column: u32) -> Option<String> {
    let line_str = content.lines().nth(line.saturating_sub(1) as usize)?;
    let chars: Vec<char> = line_str.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let is_sh = |c: char| is_ident_char(c) || c == '-';
    let mut pos = (column.saturating_sub(1) as usize).min(chars.len() - 1);
    if !is_sh(chars[pos]) {
        if pos > 0 && is_sh(chars[pos - 1]) {
            pos -= 1;
        } else {
            return None;
        }
    }
    let mut s = pos;
    while s > 0 && is_sh(chars[s - 1]) {
        s -= 1;
    }
    let mut e = pos;
    while e + 1 < chars.len() && is_sh(chars[e + 1]) {
        e += 1;
    }
    let out: String = chars[s..=e].iter().collect();
    (!out.is_empty()).then_some(out)
}

fn find_symbol_column(line: &str, mask: &[bool], symbol: &str) -> Option<u32> {
    let mut from = 0;
    while let Some(rel) = line[from..].find(symbol) {
        let s = from + rel;
        let e = s + symbol.len();
        let code_ok = mask.get(s..e).is_some_and(|sl| sl.iter().all(|&b| b));
        if code_ok {
            return Some(line[..s].chars().count() as u32 + 1);
        }
        from = s + line[s..].chars().next().map_or(1, char::len_utf8);
    }
    line.find(symbol)
        .map(|s| line[..s].chars().count() as u32 + 1)
}
