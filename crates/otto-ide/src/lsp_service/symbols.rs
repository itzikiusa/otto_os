//! Hierarchical Document Structure (`document_symbols`) parser across Built-in & Plugin Languages.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;

use crate::types::{detect_language, IdeSymbolNode};

use super::code_mask_for_line;

struct RawSymbol {
    name: String,
    kind: String,
    detail: Option<String>,
    line: u32,
    end_line: u32,
    indent: usize,
    is_container: bool,
}

impl RawSymbol {
    fn into_node(self) -> IdeSymbolNode {
        IdeSymbolNode {
            name: self.name,
            kind: self.kind,
            detail: self.detail,
            line: self.line,
            end_line: self.end_line,
            children: Vec::new(),
        }
    }
}

pub fn parse_document_symbols(path: &str, lang: &str, content: &str) -> Vec<IdeSymbolNode> {
    let effective = if lang.is_empty() || lang == "plaintext" {
        detect_language(path).unwrap_or("plaintext")
    } else {
        lang
    };
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    match effective {
        "python" => parse_python_symbols(&lines),
        "markdown" => parse_markdown_symbols(&lines),
        "makefile" => parse_makefile_symbols(&lines),
        _ => parse_brace_language_symbols(effective, &lines),
    }
}

fn parse_brace_language_symbols(lang: &str, lines: &[&str]) -> Vec<IdeSymbolNode> {
    let brace_ends = compute_brace_block_ends(lang, lines);
    let mut raw = Vec::new();
    for (idx, &line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty()
            || ["//", "/*", "*", "#"]
                .iter()
                .any(|p| trimmed.starts_with(p))
        {
            continue;
        }
        let line_no = (idx + 1) as u32;
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        if let Some((name, kind, detail, is_container)) = match_brace_symbol(lang, trimmed) {
            let end_line = brace_ends
                .get(&line_no)
                .copied()
                .unwrap_or(line_no)
                .max(line_no);
            raw.push(RawSymbol {
                name,
                kind,
                detail,
                line: line_no,
                end_line,
                indent,
                is_container,
            });
        }
    }
    nest_symbols(raw, false)
}

pub(super) fn match_brace_symbol(
    lang: &str,
    trimmed: &str,
) -> Option<(String, String, Option<String>, bool)> {
    match lang {
        "rust" => match_rust_symbol(trimmed),
        "go" => match_go_symbol(trimmed),
        "java" | "kotlin" => match_jvm_symbol(trimmed),
        "javascript" | "typescript" | "svelte" | "vue" => match_ts_symbol(trimmed),
        "protobuf" => match_proto_symbol(trimmed),
        "graphql" => match_graphql_symbol(trimmed),
        _ => match_ts_symbol(trimmed).or_else(|| match_rust_symbol(trimmed)),
    }
}

fn match_rust_symbol(trimmed: &str) -> Option<(String, String, Option<String>, bool)> {
    static RE_C: OnceLock<Regex> = OnceLock::new();
    static RE_IMPL: OnceLock<Regex> = OnceLock::new();
    static RE_FN: OnceLock<Regex> = OnceLock::new();
    static RE_CONST: OnceLock<Regex> = OnceLock::new();

    if let Some(cap) = RE_C
        .get_or_init(|| {
            Regex::new(
                r"^(?:pub(?:\([^)]*\))?\s+)?(?:unsafe\s+)?(struct|enum|trait|mod)\s+([A-Za-z_][A-Za-z0-9_]*)",
            )
            .unwrap()
        })
        .captures(trimmed)
    {
        return Some((cap[2].to_string(), cap[1].to_string(), None, true));
    }
    if let Some(cap) = RE_IMPL
        .get_or_init(|| Regex::new(r"^(?:unsafe\s+)?impl(?:<[^>]*>)?\s+([^{]+?)\s*\{").unwrap())
        .captures(trimmed)
    {
        let target = cap[1].split_whitespace().collect::<Vec<_>>().join(" ");
        return Some((format!("impl {target}"), "impl".into(), None, true));
    }
    if let Some(cap) = RE_FN
        .get_or_init(|| {
            Regex::new(
                r"^(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:<[^>]*>)?\s*\(([^)]*)\)",
            )
            .unwrap()
        })
        .captures(trimmed)
    {
        let params = cap[2].trim();
        let is_m = params.starts_with("&self")
            || params.starts_with("&mut self")
            || params.starts_with("self");
        let kind = if is_m { "method" } else { "function" };
        return Some((
            cap[1].to_string(),
            kind.into(),
            Some(format!("({params})")),
            false,
        ));
    }
    let cap = RE_CONST
        .get_or_init(|| {
            Regex::new(r"^(?:pub(?:\([^)]*\))?\s+)?(const|static|type)\s+([A-Za-z_][A-Za-z0-9_]*)")
                .unwrap()
        })
        .captures(trimmed)?;
    Some((cap[2].to_string(), cap[1].to_string(), None, false))
}

fn match_go_symbol(trimmed: &str) -> Option<(String, String, Option<String>, bool)> {
    static RE_TYPE: OnceLock<Regex> = OnceLock::new();
    static RE_M: OnceLock<Regex> = OnceLock::new();
    static RE_F: OnceLock<Regex> = OnceLock::new();
    static RE_C: OnceLock<Regex> = OnceLock::new();

    if let Some(cap) = RE_TYPE
        .get_or_init(|| {
            Regex::new(r"^type\s+([A-Za-z_][A-Za-z0-9_]*)\s+(struct|interface)\b").unwrap()
        })
        .captures(trimmed)
    {
        return Some((cap[1].to_string(), cap[2].to_string(), None, true));
    }
    if let Some(cap) = RE_M
        .get_or_init(|| {
            Regex::new(r"^func\s*\(([^)]+)\)\s*([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)\)").unwrap()
        })
        .captures(trimmed)
    {
        return Some((
            cap[2].to_string(),
            "method".into(),
            Some(format!("({})", cap[1].trim())),
            false,
        ));
    }
    if let Some(cap) = RE_F
        .get_or_init(|| {
            Regex::new(r"^func\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?:\[[^\]]*\])?\s*\(([^)]*)\)").unwrap()
        })
        .captures(trimmed)
    {
        return Some((
            cap[1].to_string(),
            "function".into(),
            Some(format!("({})", cap[2].trim())),
            false,
        ));
    }
    let cap = RE_C
        .get_or_init(|| Regex::new(r"^(const|var)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap())
        .captures(trimmed)?;
    Some((cap[2].to_string(), cap[1].to_string(), None, false))
}

fn match_jvm_symbol(trimmed: &str) -> Option<(String, String, Option<String>, bool)> {
    static RE_CLS: OnceLock<Regex> = OnceLock::new();
    static RE_KT: OnceLock<Regex> = OnceLock::new();
    static RE_M: OnceLock<Regex> = OnceLock::new();

    if let Some(cap) = RE_CLS
        .get_or_init(|| {
            Regex::new(
                r"^(?:(?:public|private|protected|abstract|final|static|sealed|open|data|internal)\s+)*(class|interface|enum|record|object)\s+([A-Za-z_][A-Za-z0-9_]*)",
            )
            .unwrap()
        })
        .captures(trimmed)
    {
        return Some((cap[2].to_string(), cap[1].to_string(), None, true));
    }
    if let Some(cap) = RE_KT
        .get_or_init(|| {
            Regex::new(
                r"^(?:(?:public|private|protected|internal|override|suspend|inline)\s+)*fun\s+`?([A-Za-z0-9_ ]+?)`?\s*\(([^)]*)\)",
            )
            .unwrap()
        })
        .captures(trimmed)
    {
        return Some((
            cap[1].trim().to_string(),
            "method".into(),
            Some(format!("({})", cap[2].trim())),
            false,
        ));
    }
    if ["if ", "for ", "while ", "switch ", "return ", "new "]
        .iter()
        .any(|p| trimmed.starts_with(p))
    {
        return None;
    }
    let cap = RE_M
        .get_or_init(|| {
            Regex::new(
                r"^(?:(?:public|private|protected|static|final|synchronized|abstract|default)\s+)+(?:<[^>]+>\s+)?(?:[A-Za-z_][A-Za-z0-9_<>,.\[\]?]*\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)\)\s*(?:throws\s+[^{]+)?\{",
            )
            .unwrap()
        })
        .captures(trimmed)?;
    Some((
        cap[1].to_string(),
        "method".into(),
        Some(format!("({})", cap[2].trim())),
        false,
    ))
}

fn match_ts_symbol(trimmed: &str) -> Option<(String, String, Option<String>, bool)> {
    static RE_T: OnceLock<Regex> = OnceLock::new();
    static RE_FN: OnceLock<Regex> = OnceLock::new();
    static RE_ARR: OnceLock<Regex> = OnceLock::new();
    static RE_M: OnceLock<Regex> = OnceLock::new();

    if let Some(cap) = RE_T
        .get_or_init(|| {
            Regex::new(
                r"^(?:export\s+)?(?:default\s+)?(?:abstract\s+)?(class|interface|enum|namespace|type)\s+([A-Za-z_$][A-Za-z0-9_$]*)",
            )
            .unwrap()
        })
        .captures(trimmed)
    {
        let kind = cap[1].to_string();
        return Some((cap[2].to_string(), kind.clone(), None, kind != "type"));
    }
    if let Some(cap) = RE_FN
        .get_or_init(|| {
            Regex::new(
                r"^(?:export\s+)?(?:default\s+)?(?:async\s+)?function\s*\*?\s*([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>]*>)?\s*\(([^)]*)\)",
            )
            .unwrap()
        })
        .captures(trimmed)
    {
        return Some((
            cap[1].to_string(),
            "function".into(),
            Some(format!("({})", cap[2].trim())),
            false,
        ));
    }
    if let Some(cap) = RE_ARR
        .get_or_init(|| {
            Regex::new(
                r"^(?:export\s+)?(const|let)\s+([A-Za-z_$][A-Za-z0-9_$]*)\s*(?::[^=]+)?=\s*(?:async\s*)?\(([^)]*)\)\s*=>",
            )
            .unwrap()
        })
        .captures(trimmed)
    {
        return Some((
            cap[2].to_string(),
            "function".into(),
            Some(format!("({})", cap[3].trim())),
            false,
        ));
    }
    if ["if", "for", "while", "switch", "catch"]
        .iter()
        .any(|kw| trimmed.starts_with(kw) && trimmed[kw.len()..].starts_with([' ', '(']))
    {
        return None;
    }
    let cap = RE_M
        .get_or_init(|| {
            Regex::new(
                r"^(?:(?:public|private|protected|static|readonly|async|get|set)\s+)*([A-Za-z_$][A-Za-z0-9_$]*)\s*(?:<[^>]*>)?\s*\(([^)]*)\)\s*(?::\s*[^{]+)?\{",
            )
            .unwrap()
        })
        .captures(trimmed)?;
    Some((
        cap[1].to_string(),
        "method".into(),
        Some(format!("({})", cap[2].trim())),
        false,
    ))
}

fn match_proto_symbol(trimmed: &str) -> Option<(String, String, Option<String>, bool)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let cap = RE
        .get_or_init(|| {
            Regex::new(r"^(service|message|enum|rpc)\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap()
        })
        .captures(trimmed)?;
    let kind = cap[1].to_string();
    Some((cap[2].to_string(), kind.clone(), None, kind != "rpc"))
}

fn match_graphql_symbol(trimmed: &str) -> Option<(String, String, Option<String>, bool)> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let cap = RE
        .get_or_init(|| {
            Regex::new(
                r"^(type|input|interface|enum|union|query|mutation|subscription)\s+([A-Za-z_][A-Za-z0-9_]*)",
            )
            .unwrap()
        })
        .captures(trimmed)?;
    Some((cap[2].to_string(), cap[1].to_string(), None, true))
}

fn leaf_symbol(name: impl Into<String>, kind: impl Into<String>, ln: u32) -> IdeSymbolNode {
    IdeSymbolNode {
        name: name.into(),
        kind: kind.into(),
        detail: None,
        line: ln,
        end_line: ln,
        children: Vec::new(),
    }
}

pub(super) fn parse_python_symbols(lines: &[&str]) -> Vec<IdeSymbolNode> {
    static RE_CLS: OnceLock<Regex> = OnceLock::new();
    static RE_DEF: OnceLock<Regex> = OnceLock::new();
    static RE_CONST: OnceLock<Regex> = OnceLock::new();
    let re_cls = RE_CLS.get_or_init(|| Regex::new(r"^class\s+([A-Za-z_][A-Za-z0-9_]*)").unwrap());
    let re_def = RE_DEF.get_or_init(|| {
        Regex::new(r"^(?:async\s+)?def\s+([A-Za-z_][A-Za-z0-9_]*)\s*\(([^)]*)\)").unwrap()
    });
    let re_const = RE_CONST.get_or_init(|| Regex::new(r"^([A-Z][A-Z0-9_]{1,})\s*=").unwrap());
    let mut raw = Vec::new();
    for (idx, &line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();
        let line_no = (idx + 1) as u32;
        if let Some(cap) = re_cls.captures(trimmed) {
            let end_line = find_python_block_end(lines, idx, indent);
            raw.push(RawSymbol {
                name: cap[1].to_string(),
                kind: "class".into(),
                detail: None,
                line: line_no,
                end_line,
                indent,
                is_container: true,
            });
        } else if let Some(cap) = re_def.captures(trimmed) {
            let end_line = find_python_block_end(lines, idx, indent);
            let kind = if indent > 0 { "method" } else { "function" }.into();
            raw.push(RawSymbol {
                name: cap[1].to_string(),
                kind,
                detail: Some(format!("({})", cap[2].trim())),
                line: line_no,
                end_line,
                indent,
                is_container: true,
            });
        } else if indent == 0 {
            if let Some(cap) = re_const.captures(trimmed) {
                raw.push(RawSymbol {
                    name: cap[1].to_string(),
                    kind: "constant".into(),
                    detail: None,
                    line: line_no,
                    end_line: line_no,
                    indent: 0,
                    is_container: false,
                });
            }
        }
    }
    nest_symbols(raw, true)
}

fn find_python_block_end(lines: &[&str], start_idx: usize, base_indent: usize) -> u32 {
    let mut last_line = (start_idx + 1) as u32;
    for (offset, &line) in lines.iter().enumerate().skip(start_idx + 1) {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if line.chars().take_while(|c| c.is_whitespace()).count() <= base_indent {
            break;
        }
        last_line = (offset + 1) as u32;
    }
    last_line
}

fn parse_markdown_symbols(lines: &[&str]) -> Vec<IdeSymbolNode> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(idx, &line)| {
            let trimmed = line.trim_start();
            let level = trimmed.chars().take_while(|&c| c == '#').count();
            let title = (1..=6).contains(&level).then(|| trimmed[level..].trim())?;
            (!title.is_empty()).then(|| leaf_symbol(title, format!("h{level}"), (idx + 1) as u32))
        })
        .collect()
}

fn parse_makefile_symbols(lines: &[&str]) -> Vec<IdeSymbolNode> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"^([A-Za-z][A-Za-z0-9._-]*)\s*:(?:[^=]|$)").unwrap());
    lines
        .iter()
        .enumerate()
        .filter_map(|(idx, &line)| {
            Some(leaf_symbol(
                &re.captures(line)?[1],
                "target",
                (idx + 1) as u32,
            ))
        })
        .collect()
}

fn compute_brace_block_ends(lang: &str, lines: &[&str]) -> BTreeMap<u32, u32> {
    let mut stack: Vec<u32> = Vec::new();
    let mut ends = BTreeMap::new();
    let mut in_block = false;
    for (idx, &line) in lines.iter().enumerate() {
        let ln = (idx + 1) as u32;
        let mask = code_mask_for_line(lang, line, &mut in_block);
        for (byte_idx, ch) in line.char_indices() {
            if !mask.get(byte_idx).copied().unwrap_or(false) {
                continue;
            }
            if ch == '{' {
                stack.push(ln);
            } else if ch == '}' {
                if let Some(open) = stack.pop() {
                    ends.entry(open).or_insert(ln);
                }
            }
        }
    }
    ends
}

fn nest_symbols(raw: Vec<RawSymbol>, by_indent: bool) -> Vec<IdeSymbolNode> {
    let mut stack: Vec<(usize, IdeSymbolNode)> = Vec::new();
    let mut roots: Vec<IdeSymbolNode> = Vec::new();
    let pop_to_parent = |stack: &mut Vec<(usize, IdeSymbolNode)>,
                         roots: &mut Vec<IdeSymbolNode>| {
        if let Some((_, done)) = stack.pop() {
            if let Some((_, p)) = stack.last_mut() {
                p.children.push(done);
            } else {
                roots.push(done);
            }
        }
    };
    for sym in raw {
        while stack.last().is_some_and(|(ind, top)| {
            if by_indent {
                sym.indent <= *ind
            } else {
                sym.line > top.end_line
            }
        }) {
            pop_to_parent(&mut stack, &mut roots);
        }
        let push_container = sym.is_container && (by_indent || sym.end_line > sym.line);
        let indent = sym.indent;
        let node = sym.into_node();
        if push_container {
            stack.push((indent, node));
        } else if let Some((_, p)) = stack.last_mut() {
            p.children.push(node);
        } else {
            roots.push(node);
        }
    }
    while !stack.is_empty() {
        pop_to_parent(&mut stack, &mut roots);
    }
    roots
}
