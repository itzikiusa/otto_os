//! JSON `$ref`, shell `source`, JS/TS relative-import, and CSV column-header
//! navigation helpers for [`super::navigation`].

use crate::target::ResolvedTarget;
use crate::types::{IdeReferenceLocation, IdeReferencesResp};
use crate::IdeCtx;

use super::navigation::{is_confined_nav_path, read_confined_nav_file};
use super::navigation_ext::resolve_js_ts_module_files;
use super::navigation_imports::{
    format_result_path, resolve_origin_abs, resolve_shell_source_target,
};
use super::symbols_data::{detect_csv_delimiter, split_delimited_row};

pub(super) async fn try_path_or_ref_navigation<S: IdeCtx>(
    ctx: &S,
    resolved: &ResolvedTarget,
    origin_path: &str,
    lang: &str,
    content: &str,
    line: u32,
    column: u32,
) -> Option<IdeReferencesResp> {
    let line_str = content.lines().nth(line.saturating_sub(1) as usize)?;

    if matches!(lang, "json" | "jsonc" | "jsonl") {
        let lit = extract_quoted_string_at(line_str, column).or_else(|| {
            line_str
                .contains("\"$ref\"")
                .then(|| extract_ref_value(line_str))
                .flatten()
        })?;
        if lit.starts_with('#')
            || lit.starts_with("./")
            || lit.starts_with("../")
            || lit.ends_with(".json")
        {
            let (file_part, frag) = match lit.split_once('#') {
                Some((f, p)) => (f, Some(p)),
                None => (lit.as_str(), None),
            };
            let keys: Vec<&str> = frag
                .unwrap_or("")
                .trim_start_matches('/')
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();
            if file_part.is_empty() {
                let locs = find_json_key_definitions(origin_path, content, &keys);
                if !locs.is_empty() {
                    return Some(IdeReferencesResp {
                        symbol: lit,
                        locations: locs,
                    });
                }
            } else if let ResolvedTarget::Local {
                root,
                readonly_roots,
                is_root_user,
                ..
            } = resolved
            {
                let roots = resolved.local_project_roots();
                let origin_abs = resolve_origin_abs(origin_path, &roots, root);
                if let Some(target_abs) = origin_abs
                    .parent()
                    .map(|p| p.join(file_part))
                    .and_then(|p| p.canonicalize().ok())
                    .filter(|c| is_confined_nav_path(c, &roots, readonly_roots, *is_root_user))
                {
                    let target_str =
                        format_result_path(&target_abs, root, origin_path.starts_with('/'));
                    if let Ok(target_file) =
                        ctx.ide().targets.read_file(resolved, &target_str).await
                    {
                        let mut locs =
                            find_json_key_definitions(&target_str, &target_file.content, &keys);
                        if locs.is_empty() {
                            locs.push(IdeReferenceLocation {
                                path: target_str,
                                line: 1,
                                column: 1,
                                preview: target_file
                                    .content
                                    .lines()
                                    .next()
                                    .unwrap_or("")
                                    .trim()
                                    .to_string(),
                            });
                        }
                        return Some(IdeReferencesResp {
                            symbol: lit,
                            locations: locs,
                        });
                    }
                }
            }
        }
    }

    if lang == "shell" {
        let trimmed = line_str.trim();
        if let Some(rest) = trimmed
            .strip_prefix("source ")
            .or_else(|| trimmed.strip_prefix(". "))
        {
            if let ResolvedTarget::Local {
                root, is_root_user, ..
            } = resolved
            {
                let roots = resolved.local_project_roots();
                let origin_abs = resolve_origin_abs(origin_path, &roots, root);
                if let Some(target_abs) =
                    resolve_shell_source_target(rest, &origin_abs, &roots, *is_root_user)
                {
                    let target_str =
                        format_result_path(&target_abs, root, origin_path.starts_with('/'));
                    if let Ok(f) = ctx.ide().targets.read_file(resolved, &target_str).await {
                        return Some(IdeReferencesResp {
                            symbol: rest.trim().to_string(),
                            locations: vec![IdeReferenceLocation {
                                path: target_str,
                                line: 1,
                                column: 1,
                                preview: f.content.lines().next().unwrap_or("").trim().to_string(),
                            }],
                        });
                    }
                }
            }
        }
    }

    if matches!(lang, "javascript" | "typescript" | "svelte")
        && (line_str.contains("from ")
            || line_str.contains("import(")
            || line_str.contains("require("))
    {
        if let Some(spec) = extract_quoted_string_at(line_str, column) {
            if let ResolvedTarget::Local {
                root,
                readonly_roots,
                is_root_user,
                ..
            } = resolved
            {
                let roots = resolved.local_project_roots();
                let origin_abs = resolve_origin_abs(origin_path, &roots, root);
                for cand in resolve_js_ts_module_files(&spec, &origin_abs, &roots) {
                    if let Some((canon_first, text)) =
                        read_confined_nav_file(&cand, &roots, readonly_roots, *is_root_user)
                    {
                        let p =
                            format_result_path(&canon_first, root, origin_path.starts_with('/'));
                        return Some(IdeReferencesResp {
                            symbol: spec,
                            locations: vec![IdeReferenceLocation {
                                path: p,
                                line: 1,
                                column: 1,
                                preview: text.lines().next().unwrap_or("").trim().to_string(),
                            }],
                        });
                    }
                }
            }
        }
    }

    None
}

pub(super) fn find_json_key_definitions(
    path: &str,
    content: &str,
    keys: &[&str],
) -> Vec<IdeReferenceLocation> {
    let Some(&target_key) = keys.last() else {
        return Vec::new();
    };
    let needle = format!("\"{target_key}\"");
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if let Some(pos) = line.find(&needle) {
            if line[pos + needle.len()..].trim_start().starts_with(':') {
                out.push(IdeReferenceLocation {
                    path: path.to_string(),
                    line: (idx + 1) as u32,
                    column: line[..pos].chars().count() as u32 + 2,
                    preview: line.trim().to_string(),
                });
            }
        }
    }
    out
}

pub(super) fn navigate_csv_column(
    path: &str,
    lang: &str,
    content: &str,
    line: u32,
    column: u32,
) -> Option<IdeReferencesResp> {
    let mut non_empty = content
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty());
    let (header_idx, header_line) = non_empty.next()?;
    let delim = detect_csv_delimiter(path, lang, header_line);
    let headers = split_delimited_row(header_line, delim);
    let cur_line = content.lines().nth(line.saturating_sub(1) as usize)?;
    let col_idx = column_index_at_char_offset(cur_line, delim, column.saturating_sub(1) as usize);
    let col_name = headers.get(col_idx)?.trim().to_string();
    let header_char_col = char_offset_of_column(header_line, delim, col_idx) as u32 + 1;
    Some(IdeReferencesResp {
        symbol: if col_name.is_empty() {
            format!("Column {}", col_idx + 1)
        } else {
            col_name.clone()
        },
        locations: vec![IdeReferenceLocation {
            path: path.to_string(),
            line: (header_idx + 1) as u32,
            column: header_char_col,
            preview: format!("Column #{}: {}", col_idx + 1, col_name),
        }],
    })
}

fn column_index_at_char_offset(line: &str, delim: char, target_char: usize) -> usize {
    let mut in_q = false;
    let mut col = 0usize;
    for (i, ch) in line.chars().enumerate() {
        if i >= target_char {
            break;
        }
        if ch == '"' {
            in_q = !in_q;
        } else if ch == delim && !in_q {
            col += 1;
        }
    }
    col
}

fn char_offset_of_column(line: &str, delim: char, want_col: usize) -> usize {
    if want_col == 0 {
        return 0;
    }
    let mut in_q = false;
    let mut col = 0usize;
    for (i, ch) in line.chars().enumerate() {
        if ch == '"' {
            in_q = !in_q;
        } else if ch == delim && !in_q {
            col += 1;
            if col == want_col {
                return i + 1;
            }
        }
    }
    0
}

pub(super) fn extract_quoted_string_at(line: &str, column: u32) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let target = (column.saturating_sub(1) as usize).min(chars.len() - 1);
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars[i], '"' | '\'' | '`') {
            let q = chars[i];
            let start = i;
            i += 1;
            let mut val = String::new();
            while i < chars.len() {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    val.push(chars[i + 1]);
                    i += 2;
                } else if chars[i] == q {
                    if (start..=i).contains(&target) {
                        return Some(val);
                    }
                    i += 1;
                    break;
                } else {
                    val.push(chars[i]);
                    i += 1;
                }
            }
            continue;
        }
        i += 1;
    }
    None
}

pub(super) fn extract_ref_value(line: &str) -> Option<String> {
    let after = line.split("\"$ref\"").nth(1)?;
    let q1 = after.find('"')?;
    let rest = &after[q1 + 1..];
    let q2 = rest.find('"')?;
    Some(rest[..q2].to_string())
}
