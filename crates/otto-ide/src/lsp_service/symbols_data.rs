//! Structured Document Symbols for Shell (`.sh`/`.bash`/`.zsh`), JSON/JSONC/JSONL, and CSV/TSV.
//!
//! Zero external subprocesses, zero retained caches, O(N) single-pass scanners
//! capped so large data files parse in `<1ms` with negligible allocations.

use std::sync::OnceLock;

use regex::Regex;

use crate::types::IdeSymbolNode;

use super::code_mask_for_line;
use super::symbols::{compute_brace_block_ends, nest_symbols, RawSymbol};

const MAX_JSON_CHILDREN_PER_CONTAINER: usize = 200;
const MAX_JSONL_RECORDS: usize = 300;
const MAX_CSV_COLUMNS: usize = 256;

/// Parse Shell (`.sh`, `.bash`, `.zsh`) functions & exported/readonly top-level constants.
pub(super) fn parse_shell_symbols(lines: &[&str]) -> Vec<IdeSymbolNode> {
    static RE_FN: OnceLock<Regex> = OnceLock::new();
    static RE_FN_KW: OnceLock<Regex> = OnceLock::new();
    static RE_EXPORT: OnceLock<Regex> = OnceLock::new();

    let re_fn = RE_FN.get_or_init(|| {
        Regex::new(r"^(?:function\s+)?([A-Za-z_][A-Za-z0-9_.-]*)\s*\(\s*\)\s*\{?").unwrap()
    });
    let re_fn_kw = RE_FN_KW.get_or_init(|| {
        Regex::new(r"^function\s+([A-Za-z_][A-Za-z0-9_.-]*)(?:\s*\{|\s*$)").unwrap()
    });
    let re_export = RE_EXPORT.get_or_init(|| {
        Regex::new(
            r"^(?:export|readonly|declare\s+-[A-Za-z]*r[A-Za-z]*)\s+([A-Za-z_][A-Za-z0-9_]*)=",
        )
        .unwrap()
    });

    let brace_ends = compute_brace_block_ends("shell", lines);
    let mut raw = Vec::new();

    for (idx, &line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let line_no = (idx + 1) as u32;
        let indent = line.chars().take_while(|c| c.is_whitespace()).count();

        let fn_name = re_fn
            .captures(trimmed)
            .map(|c| c[1].to_string())
            .or_else(|| re_fn_kw.captures(trimmed).map(|c| c[1].to_string()));

        if let Some(name) = fn_name {
            // Brace may sit on this line or the next non-empty line (`foo()\n{`).
            let brace_line = if trimmed.contains('{') {
                line_no
            } else {
                lines
                    .iter()
                    .enumerate()
                    .skip(idx + 1)
                    .find(|(_, l)| !l.trim().is_empty() && !l.trim().starts_with('#'))
                    .and_then(|(n_idx, l)| l.trim().starts_with('{').then_some((n_idx + 1) as u32))
                    .unwrap_or(line_no)
            };
            let end_line = brace_ends
                .get(&brace_line)
                .copied()
                .unwrap_or(line_no)
                .max(line_no);
            raw.push(RawSymbol {
                name,
                kind: "function".into(),
                detail: Some("()".into()),
                line: line_no,
                end_line,
                indent,
                is_container: true,
            });
            continue;
        }

        if indent == 0 {
            if let Some(cap) = re_export.captures(trimmed) {
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

    nest_symbols(raw, false)
}

/// Parse JSON, JSONC (with `//` and `/* */` comments), or JSONL/NDJSON into a tree of keys & items.
pub(super) fn parse_json_symbols(path: &str, lang: &str, content: &str) -> Vec<IdeSymbolNode> {
    let lower_path = path.to_ascii_lowercase();
    if lang == "jsonl" || lower_path.ends_with(".jsonl") || lower_path.ends_with(".ndjson") {
        return parse_jsonl_symbols(content);
    }

    let mut scanner = JsonSymbolScanner::new(content);
    scanner.parse_root()
}

fn parse_jsonl_symbols(content: &str) -> Vec<IdeSymbolNode> {
    let mut out = Vec::new();
    for (idx, raw) in content.lines().enumerate() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if out.len() >= MAX_JSONL_RECORDS {
            break;
        }
        let line_no = (idx + 1) as u32;
        let children = JsonSymbolScanner::new(trimmed)
            .with_fixed_line(line_no)
            .parse_root();
        let hint = children
            .iter()
            .find(|c| matches!(c.name.as_str(), "id" | "name" | "type" | "event" | "key"))
            .and_then(|c| c.detail.as_deref())
            .unwrap_or("");
        let name = if hint.is_empty() {
            format!("Record #{}", out.len() + 1)
        } else {
            format!("Record #{} ({hint})", out.len() + 1)
        };
        out.push(IdeSymbolNode {
            name,
            kind: "object".into(),
            detail: Some(format!("line {line_no}")),
            line: line_no,
            end_line: line_no,
            children,
        });
    }
    out
}

/// Parse CSV or TSV headers into navigable column symbols with row-count summary.
pub(super) fn parse_csv_symbols(path: &str, lang: &str, lines: &[&str]) -> Vec<IdeSymbolNode> {
    let Some((header_idx, header_line)) =
        lines.iter().enumerate().find(|(_, l)| !l.trim().is_empty())
    else {
        return Vec::new();
    };
    let delim = detect_csv_delimiter(path, lang, header_line);
    let cols = split_delimited_row(header_line, delim);
    let total_lines = lines.len() as u32;
    let data_rows = lines
        .iter()
        .skip(header_idx + 1)
        .filter(|l| !l.trim().is_empty())
        .count();
    let header_ln = (header_idx + 1) as u32;

    cols.into_iter()
        .take(MAX_CSV_COLUMNS)
        .enumerate()
        .map(|(idx, raw_name)| {
            let clean = raw_name.trim();
            let col_label = if clean.is_empty() {
                format!("Column {}", idx + 1)
            } else {
                clean.to_string()
            };
            IdeSymbolNode {
                name: col_label,
                kind: "field".into(),
                detail: Some(format!("col #{} · {} rows", idx + 1, data_rows)),
                line: header_ln,
                end_line: total_lines.max(header_ln),
                children: Vec::new(),
            }
        })
        .collect()
}

pub(crate) fn detect_csv_delimiter(path: &str, lang: &str, sample_line: &str) -> char {
    let lower = path.to_ascii_lowercase();
    if lang == "tsv" || lower.ends_with(".tsv") {
        return '\t';
    }
    let mut best = (',', 0usize);
    for cand in [',', '\t', ';', '|'] {
        let count = count_unquoted_delim(sample_line, cand);
        if count > best.1 {
            best = (cand, count);
        }
    }
    best.0
}

fn count_unquoted_delim(line: &str, delim: char) -> usize {
    let mut in_quotes = false;
    let mut count = 0usize;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if in_quotes && chars.peek() == Some(&'"') {
                let _ = chars.next();
            } else {
                in_quotes = !in_quotes;
            }
        } else if ch == delim && !in_quotes {
            count += 1;
        }
    }
    count
}

pub(crate) fn split_delimited_row(line: &str, delim: char) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if in_quotes && chars.peek() == Some(&'"') {
                cur.push('"');
                let _ = chars.next();
            } else {
                in_quotes = !in_quotes;
            }
        } else if ch == delim && !in_quotes {
            fields.push(std::mem::take(&mut cur));
        } else {
            cur.push(ch);
        }
    }
    fields.push(cur);
    fields
}

// ---------------------------------------------------------------------------
// Single-pass JSON / JSONC symbol scanner (tracks exact 1-based line spans)
// ---------------------------------------------------------------------------

struct JsonSymbolScanner<'a> {
    bytes: &'a [u8],
    pos: usize,
    line: u32,
    fixed_line: Option<u32>,
}

impl<'a> JsonSymbolScanner<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            bytes: src.as_bytes(),
            pos: 0,
            line: 1,
            fixed_line: None,
        }
    }

    fn with_fixed_line(mut self, ln: u32) -> Self {
        self.fixed_line = Some(ln);
        self
    }

    fn current_line(&self) -> u32 {
        self.fixed_line.unwrap_or(self.line)
    }

    fn parse_root(&mut self) -> Vec<IdeSymbolNode> {
        self.skip_ws_and_comments();
        match self.peek() {
            Some(b'{') => self.parse_object(0).0,
            Some(b'[') => self.parse_array(0).0,
            _ => Vec::new(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let b = self.bytes.get(self.pos).copied()?;
        self.pos += 1;
        if b == b'\n' {
            self.line += 1;
        }
        Some(b)
    }

    fn skip_ws_and_comments(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() {
                self.bump();
                continue;
            }
            if b == b'/' && self.bytes.get(self.pos + 1) == Some(&b'/') {
                while let Some(ch) = self.bump() {
                    if ch == b'\n' {
                        break;
                    }
                }
                continue;
            }
            if b == b'/' && self.bytes.get(self.pos + 1) == Some(&b'*') {
                self.bump();
                self.bump();
                while let Some(ch) = self.bump() {
                    if ch == b'*' && self.peek() == Some(b'/') {
                        self.bump();
                        break;
                    }
                }
                continue;
            }
            break;
        }
    }

    fn parse_string(&mut self) -> Option<String> {
        if self.peek() != Some(b'"') {
            return None;
        }
        let start_content = self.pos + 1;
        self.bump();
        let mut has_escape = false;
        while let Some(b) = self.bump() {
            if b == b'\\' {
                has_escape = true;
                let _ = self.bump();
            } else if b == b'"' {
                let end_content = self.pos - 1;
                let raw = std::str::from_utf8(&self.bytes[start_content..end_content]).ok()?;
                if !has_escape {
                    return Some(raw.to_string());
                }
                return Some(unescape_json_label(raw));
            }
        }
        None
    }

    fn parse_object(&mut self, depth: usize) -> (Vec<IdeSymbolNode>, u32) {
        let start_ln = self.current_line();
        if self.peek() == Some(b'{') {
            self.bump();
        }
        let mut children = Vec::new();
        while self.pos < self.bytes.len() {
            self.skip_ws_and_comments();
            match self.peek() {
                None => break,
                Some(b'}') => {
                    let end_ln = self.current_line();
                    self.bump();
                    return (children, end_ln);
                }
                Some(b',') => {
                    self.bump();
                    continue;
                }
                Some(b'"') => {
                    let key_line = self.current_line();
                    let key = self.parse_string().unwrap_or_default();
                    self.skip_ws_and_comments();
                    if self.peek() == Some(b':') {
                        self.bump();
                    }
                    let (kind, detail, sub_children, val_end) = self.parse_value(depth + 1);
                    if children.len() < MAX_JSON_CHILDREN_PER_CONTAINER && depth <= 8 {
                        children.push(IdeSymbolNode {
                            name: if key.is_empty() { "\"\"".into() } else { key },
                            kind,
                            detail,
                            line: key_line,
                            end_line: val_end.max(key_line),
                            children: sub_children,
                        });
                    }
                }
                _ => {
                    self.bump();
                }
            }
        }
        (children, self.current_line().max(start_ln))
    }

    fn parse_array(&mut self, depth: usize) -> (Vec<IdeSymbolNode>, u32) {
        let start_ln = self.current_line();
        if self.peek() == Some(b'[') {
            self.bump();
        }
        let mut items = Vec::new();
        let mut index = 0usize;
        while self.pos < self.bytes.len() {
            self.skip_ws_and_comments();
            match self.peek() {
                None => break,
                Some(b']') => {
                    let end_ln = self.current_line();
                    self.bump();
                    return (items, end_ln);
                }
                Some(b',') => {
                    self.bump();
                    continue;
                }
                _ => {
                    let item_ln = self.current_line();
                    let (kind, detail, sub_children, end_ln) = self.parse_value(depth + 1);
                    if items.len() < MAX_JSON_CHILDREN_PER_CONTAINER && depth <= 8 {
                        items.push(IdeSymbolNode {
                            name: format!("[{index}]"),
                            kind,
                            detail,
                            line: item_ln,
                            end_line: end_ln.max(item_ln),
                            children: sub_children,
                        });
                    }
                    index += 1;
                }
            }
        }
        (items, self.current_line().max(start_ln))
    }

    fn parse_value(&mut self, depth: usize) -> (String, Option<String>, Vec<IdeSymbolNode>, u32) {
        self.skip_ws_and_comments();
        let start_ln = self.current_line();
        match self.peek() {
            Some(b'{') => {
                let (ch, end_ln) = self.parse_object(depth);
                let detail = Some(format!("{{{} keys}}", ch.len()));
                ("object".into(), detail, ch, end_ln)
            }
            Some(b'[') => {
                let (ch, end_ln) = self.parse_array(depth);
                let detail = Some(format!("[{} items]", ch.len()));
                ("array".into(), detail, ch, end_ln)
            }
            Some(b'"') => {
                let s = self.parse_string().unwrap_or_default();
                let preview = if s.chars().count() > 36 {
                    let short: String = s.chars().take(36).collect();
                    format!("\"{short}…\"")
                } else {
                    format!("\"{s}\"")
                };
                (
                    "string".into(),
                    Some(preview),
                    Vec::new(),
                    self.current_line(),
                )
            }
            Some(_) => {
                let start = self.pos;
                while let Some(b) = self.peek() {
                    if b.is_ascii_whitespace() || matches!(b, b',' | b'}' | b']' | b'/') {
                        break;
                    }
                    self.bump();
                }
                let tok = std::str::from_utf8(&self.bytes[start..self.pos])
                    .unwrap_or("")
                    .trim();
                let kind = match tok {
                    "true" | "false" => "boolean",
                    "null" => "null",
                    _ => "number",
                };
                let detail = (!tok.is_empty()).then(|| tok.to_string());
                (
                    kind.into(),
                    detail,
                    Vec::new(),
                    self.current_line().max(start_ln),
                )
            }
            None => ("null".into(), None, Vec::new(), start_ln),
        }
    }
}

fn unescape_json_label(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n') => out.push(' '),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => break,
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[allow(dead_code)]
fn _keep_mask_used() {
    let mut b = false;
    let _ = code_mask_for_line("shell", "", &mut b);
}
