//! Lexical identifier extraction, comment/string masking, word-boundary
//! occurrence/rename scanning, and `rg` local candidate discovery for [`super`] (`lsp_service`).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::Command;

use crate::types::{IdeReferenceLocation, IdeRenameFileEdit, IdeRenameLinePreview};

pub fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' || (!ch.is_ascii() && ch.is_alphabetic())
}

pub fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_alphabetic() || first == '_' || first == '$') && chars.all(is_ident_char)
}

pub fn extract_identifier_at(content: &str, line: u32, column: u32) -> Option<String> {
    let line_str = content.lines().nth(line.saturating_sub(1) as usize)?;
    let chars: Vec<char> = line_str.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let mut pos = (column.saturating_sub(1) as usize).min(chars.len() - 1);
    if !is_ident_char(chars[pos]) {
        if pos > 0 && is_ident_char(chars[pos - 1]) {
            pos -= 1;
        } else if pos + 1 < chars.len() && is_ident_char(chars[pos + 1]) {
            pos += 1;
        } else {
            return None;
        }
    }
    let mut start = pos;
    while start > 0 && is_ident_char(chars[start - 1]) {
        start -= 1;
    }
    let mut end = pos;
    while end + 1 < chars.len() && is_ident_char(chars[end + 1]) {
        end += 1;
    }
    let ident: String = chars[start..=end].iter().collect();
    is_valid_identifier(&ident).then_some(ident)
}

pub(crate) fn code_mask_for_line(lang: &str, line: &str, in_block_comment: &mut bool) -> Vec<bool> {
    let bytes = line.as_bytes();
    let mut mask = vec![false; bytes.len()];
    let hash_comment = matches!(
        lang,
        "python" | "shell" | "yaml" | "toml" | "makefile" | "graphql"
    );
    let mut i = 0;

    while i < bytes.len() {
        if *in_block_comment {
            if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                *in_block_comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }
        if !hash_comment && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            break;
        }
        if !hash_comment && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            *in_block_comment = true;
            i += 2;
            continue;
        }
        if hash_comment && bytes[i] == b'#' {
            break;
        }
        if matches!(bytes[i], b'"' | b'\'' | b'`') {
            let q = bytes[i];
            if lang == "rust"
                && q == b'\''
                && (i + 2 >= bytes.len() || bytes[i + 2] != b'\'')
                && (i + 3 >= bytes.len() || bytes[i + 1] != b'\\' || bytes[i + 3] != b'\'')
            {
                mask[i] = true;
                i += 1;
                continue;
            }
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' {
                    i += 2;
                } else if bytes[i] == q {
                    i += 1;
                    break;
                } else {
                    i += 1;
                }
            }
            continue;
        }
        mask[i] = true;
        i += 1;
    }
    mask
}

fn find_word_byte_ranges_on_line(line: &str, mask: &[bool], symbol: &str) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    if symbol.is_empty() || line.len() < symbol.len() {
        return ranges;
    }
    let mut from = 0;
    while let Some(rel) = line[from..].find(symbol) {
        let start = from + rel;
        let end = start + symbol.len();
        let code_ok = mask.get(start..end).is_some_and(|s| s.iter().all(|&b| b));
        let prev_ok = line[..start]
            .chars()
            .next_back()
            .is_none_or(|c| !is_ident_char(c));
        let next_ok = line[end..].chars().next().is_none_or(|c| !is_ident_char(c));
        if code_ok && prev_ok && next_ok {
            ranges.push((start, end));
        }
        from = start + line[start..].chars().next().map_or(1, char::len_utf8);
    }
    ranges
}

pub fn find_symbol_occurrences_in_text(
    path: &str,
    lang: &str,
    content: &str,
    symbol: &str,
) -> Vec<IdeReferenceLocation> {
    let mut out = Vec::new();
    let mut in_block = false;
    for (idx, line) in content.lines().enumerate() {
        let mask = code_mask_for_line(lang, line, &mut in_block);
        for (byte_start, _) in find_word_byte_ranges_on_line(line, &mask, symbol) {
            let col = line[..byte_start].chars().count() as u32 + 1;
            out.push(IdeReferenceLocation {
                path: path.to_string(),
                line: (idx + 1) as u32,
                column: col,
                preview: line.trim().to_string(),
            });
        }
    }
    out
}

pub fn rename_identifier_in_text(
    path: &str,
    lang: &str,
    content: &str,
    symbol: &str,
    new_name: &str,
) -> Option<(String, IdeRenameFileEdit)> {
    let trailing_nl = content.ends_with('\n');
    let (mut in_block, mut occurrences) = (false, 0usize);
    let (mut new_lines, mut preview_lines) = (Vec::new(), Vec::new());
    for (idx, line) in content.lines().enumerate() {
        let mask = code_mask_for_line(lang, line, &mut in_block);
        let ranges = find_word_byte_ranges_on_line(line, &mask, symbol);
        if ranges.is_empty() {
            new_lines.push(line.to_string());
            continue;
        }
        occurrences += ranges.len();
        let (mut rebuilt, mut cur) = (
            String::with_capacity(line.len() + ranges.len() * new_name.len()),
            0,
        );
        for (s, e) in ranges {
            rebuilt.push_str(&line[cur..s]);
            rebuilt.push_str(new_name);
            cur = e;
        }
        rebuilt.push_str(&line[cur..]);
        if preview_lines.len() < 25 {
            preview_lines.push(IdeRenameLinePreview {
                line: (idx + 1) as u32,
                before: line.to_string(),
                after: rebuilt.clone(),
            });
        }
        new_lines.push(rebuilt);
    }
    if occurrences == 0 {
        return None;
    }
    let mut updated = new_lines.join("\n");
    if trailing_nl {
        updated.push('\n');
    }
    Some((
        updated,
        IdeRenameFileEdit {
            path: path.to_string(),
            occurrences,
            preview_lines,
        },
    ))
}

#[cfg(test)]
pub(super) async fn rg_local_candidate_files(
    root: &Path,
    is_root_user: bool,
    symbol: &str,
    origin_path: &str,
) -> Option<Vec<String>> {
    rg_local_candidate_files_multi(
        &[root.to_path_buf()],
        root,
        is_root_user,
        symbol,
        origin_path,
    )
    .await
}

pub(super) async fn rg_local_candidate_files_multi(
    roots: &[PathBuf],
    primary_root: &Path,
    is_root_user: bool,
    symbol: &str,
    origin_path: &str,
) -> Option<Vec<String>> {
    let rg_bin =
        crate::toolchains::find_executable_in(&crate::toolchains::system_search_dirs(), &["rg"])
            .unwrap_or_else(|| PathBuf::from("rg"));
    let mut cmd = Command::new(rg_bin);
    if !is_root_user {
        cmd.env_clear()
            .envs(crate::environments::shell_env::collect_safe_non_root_base_env());
    }
    // Anchor `!/target` to root so tracked source packages named `target` (such as
    // `src/target/` or `internal/target/`) remain searchable and refactorable.
    cmd.args([
        "--files-with-matches",
        "--fixed-strings",
        "--hidden",
        "-g",
        "!.git",
        "-g",
        "!node_modules",
        "-g",
        "!/target",
        "--",
        symbol,
    ]);
    for r in roots {
        cmd.arg(r);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);

    let out = tokio::time::timeout(Duration::from_secs(5), cmd.output())
        .await
        .ok()?
        .ok()?;
    match out.status.code() {
        Some(1) => return Some(vec![origin_path.to_string()]),
        Some(0) => {}
        _ => return None,
    }

    let mut ordered = vec![origin_path.to_string()];
    let origin_abs = if Path::new(origin_path).is_absolute() {
        PathBuf::from(origin_path)
    } else {
        primary_root.join(origin_path)
    };
    let mut seen_canon = BTreeSet::new();
    if let Ok(c) = origin_abs.canonicalize() {
        seen_canon.insert(c);
    }
    let keep_abs = origin_path.starts_with('/');

    for raw in String::from_utf8_lossy(&out.stdout).lines() {
        if ordered.len() >= 250 {
            break;
        }
        let Ok(canon) = Path::new(raw.trim()).canonicalize() else {
            continue;
        };
        let Some(owning_root) = roots.iter().find(|r| canon.starts_with(r)) else {
            continue;
        };
        if has_generated_build_ancestor(&canon, owning_root) {
            continue;
        }
        if !is_root_user && crate::target::enforce_non_root_path(&canon).is_err() {
            continue;
        }
        if seen_canon.insert(canon.clone()) {
            let formatted = if !keep_abs && canon.starts_with(primary_root) {
                canon
                    .strip_prefix(primary_root)
                    .map(|r| r.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| canon.to_string_lossy().into_owned())
            } else {
                canon.to_string_lossy().into_owned()
            };
            ordered.push(formatted);
        }
    }
    Some(ordered)
}

fn has_generated_build_ancestor(canon_file: &Path, owning_root: &Path) -> bool {
    let mut cur = canon_file.parent();
    while let Some(dir) = cur {
        if dir == owning_root || !dir.starts_with(owning_root) {
            break;
        }
        if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
            if matches!(name, "target" | "build" | "dist")
                && crate::target::is_generated_build_dir(dir, name, owning_root)
            {
                return true;
            }
        }
        cur = dir.parent();
    }
    false
}
