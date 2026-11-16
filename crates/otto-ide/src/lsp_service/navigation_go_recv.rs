//! Go same-package receiver and struct-field type resolution for [`super::navigation`].
//!
//! Disambiguates chained selector calls like `g.pool.Submit(task)` inside a method on
//! `GoRoutineConcurrencyHelper` by inspecting `type GoRoutineConcurrencyHelper struct { ... }`
//! across the current buffer and sibling `.go` files in the same package directory.

use std::path::{Path, PathBuf};

use crate::types::IdeReferenceLocation;

use super::is_ident_char;
use super::navigation::is_confined_nav_path;
use super::navigation_imports::go_preview_has_receiver_type;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GoRecvFilter {
    External,
    Exact(String),
    AnyMethodExcept(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum GoFieldType {
    External,
    SamePkg(String),
}

pub(crate) fn passes_go_recv_filter(
    loc: &IdeReferenceLocation,
    filter: Option<&GoRecvFilter>,
) -> bool {
    let Some(f) = filter else {
        return true;
    };
    let preview = loc.preview.trim_start();
    if !preview.starts_with("func (") && !preview.starts_with("func(") {
        return false;
    }
    match f {
        GoRecvFilter::External => false,
        GoRecvFilter::Exact(ty) => go_preview_has_receiver_type(preview, ty),
        GoRecvFilter::AnyMethodExcept(excluded) => excluded
            .as_deref()
            .is_none_or(|ex| !go_preview_has_receiver_type(preview, ex)),
    }
}

/// UTF-16 column-aware check for whether `qualifier` on `line` is itself preceded by `.`
/// (e.g. `g.pool.Submit` with `qualifier = "pool"`).
pub(crate) fn is_chained_selector_on_line(
    content: &str,
    line: u32,
    column: u32,
    qualifier: &str,
) -> bool {
    let Some(raw_line) = content.lines().nth(line.saturating_sub(1) as usize) else {
        return false;
    };
    let line_str = crate::text_pos::strip_cr(raw_line);
    if line_str.is_empty() {
        return false;
    }
    let mut b_pos =
        crate::text_pos::utf16_col_to_byte_idx(line_str, column.saturating_sub(1) as usize);
    if b_pos >= line_str.len() {
        b_pos = line_str
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }
    while let Some((prev_idx, prev_ch)) = line_str[..b_pos].char_indices().next_back() {
        if is_ident_char(prev_ch) {
            b_pos = prev_idx;
        } else {
            break;
        }
    }
    let Some(before_dot) = line_str[..b_pos].strip_suffix('.') else {
        return false;
    };
    let Some(before_qual) = before_dot.strip_suffix(qualifier) else {
        return false;
    };
    before_qual.ends_with('.')
}

/// Extract the dot-separated identifier chain preceding the hovered/clicked method on `line`
/// (e.g. `g.pool.Submit(f)` with `qualifier = "pool"` -> `["g", "pool"]`).
pub(crate) fn extract_go_selector_chain(
    content: &str,
    line: u32,
    column: u32,
    qualifier: &str,
) -> Vec<String> {
    let Some(raw_line) = content.lines().nth(line.saturating_sub(1) as usize) else {
        return vec![qualifier.to_string()];
    };
    let line_str = crate::text_pos::strip_cr(raw_line);
    if line_str.is_empty() {
        return vec![qualifier.to_string()];
    }
    let mut b_pos =
        crate::text_pos::utf16_col_to_byte_idx(line_str, column.saturating_sub(1) as usize);
    if b_pos >= line_str.len() {
        b_pos = line_str
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }
    while let Some((prev_idx, prev_ch)) = line_str[..b_pos].char_indices().next_back() {
        if is_ident_char(prev_ch) {
            b_pos = prev_idx;
        } else {
            break;
        }
    }
    let Some(mut cursor) = line_str[..b_pos].strip_suffix('.') else {
        return vec![qualifier.to_string()];
    };
    let mut rev_chain = Vec::new();
    loop {
        let end = cursor.len();
        let mut start = end;
        while let Some((prev_idx, prev_ch)) = cursor[..start].char_indices().next_back() {
            if is_ident_char(prev_ch) {
                start = prev_idx;
            } else {
                break;
            }
        }
        if start == end {
            break;
        }
        rev_chain.push(cursor[start..end].to_string());
        let Some(next_cursor) = cursor[..start].strip_suffix('.') else {
            break;
        };
        cursor = next_cursor;
        if rev_chain.len() >= 6 {
            break;
        }
    }
    rev_chain.reverse();
    if rev_chain.last().map(String::as_str) != Some(qualifier) {
        return vec![qualifier.to_string()];
    }
    rev_chain
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn read_sibling_go_sources(
    pkg_dir: &Path,
    origin_abs: &Path,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    is_root_user: bool,
) -> Vec<String> {
    const MAX_SIBLING_FILES: usize = 32;
    const MAX_SIBLING_BYTES: u64 = 512 * 1024;
    let Ok(rd) = std::fs::read_dir(pkg_dir) else {
        return Vec::new();
    };
    let origin_canon = origin_abs
        .canonicalize()
        .unwrap_or_else(|_| origin_abs.to_path_buf());
    let mut paths: Vec<_> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("go"))
        .collect();
    paths.sort();
    let mut out = Vec::new();
    for p in paths {
        if out.len() >= MAX_SIBLING_FILES {
            break;
        }
        let Ok(canon) = p.canonicalize() else {
            continue;
        };
        if canon == origin_canon
            || !is_confined_nav_path(&canon, project_roots, readonly_roots, is_root_user)
        {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&canon) else {
            continue;
        };
        if !meta.is_file() || meta.len() > MAX_SIBLING_BYTES {
            continue;
        }
        if let Ok(src) = std::fs::read_to_string(&canon) {
            out.push(src);
        }
    }
    out
}

pub(crate) fn enclosing_go_receiver(content: &str, line: u32) -> Option<(String, String)> {
    let max_idx = (line.saturating_sub(1) as usize).min(content.lines().count());
    let lines: Vec<&str> = content.lines().take(max_idx + 1).collect();
    for raw in lines.into_iter().rev() {
        let t = raw.trim_start();
        if let Some(rest) = t.strip_prefix("func") {
            if rest.chars().next().is_some_and(is_ident_char) {
                continue;
            }
            let rest = rest.trim_start();
            if let Some(inside) = rest.strip_prefix('(').and_then(|s| s.split(')').next()) {
                let mut parts = inside.split_whitespace();
                let first = parts.next()?;
                let (var, raw_ty) = match parts.next() {
                    Some(ty_tok) => (first.to_string(), ty_tok),
                    None => (String::new(), first),
                };
                let ty = raw_ty
                    .trim_start_matches('*')
                    .split('[')
                    .next()
                    .unwrap_or("")
                    .to_string();
                if !ty.is_empty() {
                    return Some((var, ty));
                }
            }
            return None;
        }
    }
    None
}

fn find_go_local_var_type(content: &str, var_name: &str) -> Option<GoFieldType> {
    let esc = regex::escape(var_name);
    if let Ok(ext_re) = regex::Regex::new(&format!(
        r"\b{esc}\s+\*?[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*\b|\b{esc}(?:\s*,\s*[A-Za-z_][A-Za-z0-9_]*)?\s*:?=\s*&?[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*\b"
    )) {
        for raw in content.lines() {
            let code = raw.split("//").next().unwrap_or("");
            if ext_re.is_match(code) {
                return Some(GoFieldType::External);
            }
        }
    }
    if let Ok(local_re) = regex::Regex::new(&format!(
        r"(?:^\s*{esc}\s+\*?([A-Z][A-Za-z0-9_]*)\b|\bvar\s+{esc}\s+\*?([A-Z][A-Za-z0-9_]*)\b|\b{esc}\s*:=\s*&?([A-Z][A-Za-z0-9_]*)\s*\{{|\(\s*{esc}\s+\*?([A-Z][A-Za-z0-9_]*)\b)"
    )) {
        for raw in content.lines() {
            let code = raw.split("//").next().unwrap_or("");
            if let Some(cap) = local_re.captures(code) {
                if let Some(m) = cap
                    .get(1)
                    .or_else(|| cap.get(2))
                    .or_else(|| cap.get(3))
                    .or_else(|| cap.get(4))
                {
                    return Some(GoFieldType::SamePkg(m.as_str().to_string()));
                }
            }
        }
    }
    None
}

pub(crate) fn classify_go_same_pkg_receiver(
    content: &str,
    sibling_sources: &[String],
    line: u32,
    qualifier: &str,
    chained: bool,
    chain: &[String],
) -> GoRecvFilter {
    let enc = enclosing_go_receiver(content, line);
    if !chained {
        if let Some((ref recv_var, ref recv_ty)) = enc {
            if !recv_var.is_empty() && recv_var == qualifier {
                return GoRecvFilter::Exact(recv_ty.clone());
            }
        }
        match find_go_local_var_type(content, qualifier) {
            Some(GoFieldType::External) => return GoRecvFilter::External,
            Some(GoFieldType::SamePkg(ty)) => return GoRecvFilter::Exact(ty),
            None => return GoRecvFilter::AnyMethodExcept(None),
        }
    }

    if chain.len() >= 2 {
        let root_ident = &chain[0];
        let root_ty = match &enc {
            Some((recv_var, recv_ty)) if !recv_var.is_empty() && recv_var == root_ident => {
                Some(GoFieldType::SamePkg(recv_ty.clone()))
            }
            _ => find_go_local_var_type(content, root_ident),
        };
        if let Some(GoFieldType::SamePkg(mut cur_ty)) = root_ty {
            let mut resolved_all = true;
            for field_name in &chain[1..] {
                match resolve_go_struct_field_type_in_pkg(
                    content,
                    sibling_sources,
                    &cur_ty,
                    field_name,
                ) {
                    Some(GoFieldType::SamePkg(next_ty)) => {
                        cur_ty = next_ty;
                    }
                    Some(GoFieldType::External) => {
                        return GoRecvFilter::External;
                    }
                    None => {
                        resolved_all = false;
                        break;
                    }
                }
            }
            if resolved_all {
                return GoRecvFilter::Exact(cur_ty);
            }
        }
    }

    GoRecvFilter::AnyMethodExcept(enc.map(|(_, ty)| ty))
}

/// Inspect `type <struct_name> struct { ... }` declarations across `primary` and `siblings`
/// to resolve the declared type of `field_name` when unambiguous.
pub(crate) fn resolve_go_struct_field_type_in_pkg(
    primary: &str,
    siblings: &[String],
    struct_name: &str,
    field_name: &str,
) -> Option<GoFieldType> {
    let mut matched: Option<GoFieldType> = None;
    for src in std::iter::once(primary).chain(siblings.iter().map(String::as_str)) {
        for cand in scan_go_struct_field_in_source(src, struct_name, field_name) {
            match &matched {
                None => matched = Some(cand),
                Some(existing) if existing == &cand => {}
                Some(_) => return None,
            }
        }
    }
    matched
}

fn scan_go_struct_field_in_source(
    src: &str,
    struct_name: &str,
    field_name: &str,
) -> Vec<GoFieldType> {
    let mut out = Vec::new();
    let mut in_block_comment = false;
    let mut active_struct_depth: Option<i32> = None;

    for raw_line in src.lines() {
        let code = strip_go_line_comments_and_tags(raw_line, &mut in_block_comment);
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }

        if let Some(mut depth) = active_struct_depth {
            if depth == 1 {
                if let Some(ft) = parse_go_struct_field_line(trimmed, field_name) {
                    out.push(ft);
                }
            }
            for ch in trimmed.chars() {
                if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    depth -= 1;
                }
            }
            if depth <= 0 {
                active_struct_depth = None;
            } else {
                active_struct_depth = Some(depth);
            }
            continue;
        }

        if let Some(after_header) = match_go_struct_header(trimmed, struct_name) {
            let mut depth = 0i32;
            for ch in after_header.chars() {
                if ch == '{' {
                    depth += 1;
                } else if ch == '}' {
                    depth -= 1;
                }
            }
            if depth > 0 {
                active_struct_depth = Some(depth);
            }
        }
    }
    out
}

fn match_go_struct_header<'a>(trimmed: &'a str, struct_name: &str) -> Option<&'a str> {
    let rest = trimmed.strip_prefix("type")?.trim_start();
    let rest = rest.strip_prefix(struct_name)?;
    if rest.chars().next().is_some_and(is_ident_char) {
        return None;
    }
    let mut rest = rest.trim_start();
    if rest.starts_with('[') {
        let mut b_depth = 0i32;
        let mut end_idx = None;
        for (i, ch) in rest.char_indices() {
            if ch == '[' {
                b_depth += 1;
            } else if ch == ']' {
                b_depth -= 1;
                if b_depth == 0 {
                    end_idx = Some(i + 1);
                    break;
                }
            }
        }
        rest = rest[end_idx?..].trim_start();
    }
    let rest = rest.strip_prefix('=').map(str::trim_start).unwrap_or(rest);
    let rest = rest.strip_prefix("struct")?;
    if rest.chars().next().is_some_and(is_ident_char) {
        return None;
    }
    let rest = rest.trim_start();
    rest.starts_with('{').then_some(rest)
}

fn parse_go_struct_field_line(line: &str, want_field: &str) -> Option<GoFieldType> {
    let clean = line.trim_end_matches(';').trim();
    if clean.is_empty() || clean.starts_with('}') {
        return None;
    }
    // Embedded field (`*Pool`, `Pool`, `*pkg.Pool`, `Pool[T]`) when single token without spaces:
    if !clean.contains(char::is_whitespace) && !clean.contains(',') {
        let ft = classify_go_type_expr(clean)?;
        let implicit_name = clean
            .trim_start_matches('*')
            .split('[')
            .next()
            .unwrap_or("")
            .rsplit('.')
            .next()
            .unwrap_or("");
        return (implicit_name == want_field).then_some(ft);
    }

    // Named field(s): `pool *Pool` or `a, pool *Pool`
    let mut idx = 0usize;
    let mut names = Vec::new();
    loop {
        let rest = clean[idx..].trim_start();
        let ws_skip = clean[idx..].len() - rest.len();
        idx += ws_skip;
        let ident_len = rest
            .char_indices()
            .take_while(|(_, c)| is_ident_char(*c))
            .map(|(i, c)| i + c.len_utf8())
            .last()?;
        let ident = &rest[..ident_len];
        names.push(ident);
        idx += ident_len;
        let after = clean[idx..].trim_start();
        let after_skip = clean[idx..].len() - after.len();
        if let Some(after_comma) = after.strip_prefix(',') {
            idx += after_skip + 1;
            let _ = after_comma;
        } else {
            idx += after_skip;
            break;
        }
    }
    if !names.contains(&want_field) {
        return None;
    }
    let type_expr = clean[idx..].trim();
    classify_go_type_expr(type_expr)
}

fn classify_go_type_expr(type_expr: &str) -> Option<GoFieldType> {
    let t = type_expr.trim().trim_start_matches('*').trim();
    if t.is_empty()
        || t.starts_with('[')
        || t.starts_with("map[")
        || t.starts_with("chan ")
        || t.starts_with("<-chan")
        || t.starts_with("func(")
        || t.starts_with("struct")
        || t.starts_with("interface")
    {
        return None;
    }
    let base = t.split('[').next().unwrap_or(t).trim();
    if let Some((pkg, ty)) = base.split_once('.') {
        if !pkg.is_empty()
            && !ty.is_empty()
            && pkg.chars().all(is_ident_char)
            && ty.chars().all(is_ident_char)
        {
            return Some(GoFieldType::External);
        }
        return None;
    }
    if base.chars().all(is_ident_char) {
        Some(GoFieldType::SamePkg(base.to_string()))
    } else {
        None
    }
}

fn strip_go_line_comments_and_tags(raw: &str, in_block_comment: &mut bool) -> String {
    let mut out = String::with_capacity(raw.len());
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0usize;
    let mut in_raw_tag = false;
    let mut in_str = false;
    while i < chars.len() {
        if *in_block_comment {
            if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                *in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if in_raw_tag {
            if chars[i] == '`' {
                in_raw_tag = false;
            }
            i += 1;
            continue;
        }
        if in_str {
            if chars[i] == '\\' && i + 1 < chars.len() {
                i += 2;
                continue;
            }
            if chars[i] == '"' {
                in_str = false;
            }
            i += 1;
            continue;
        }
        if chars[i] == '/' && chars.get(i + 1) == Some(&'/') {
            break;
        }
        if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            *in_block_comment = true;
            i += 2;
            continue;
        }
        if chars[i] == '`' {
            in_raw_tag = true;
            i += 1;
            continue;
        }
        if chars[i] == '"' {
            in_str = true;
            i += 1;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}
