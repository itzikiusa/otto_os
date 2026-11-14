//! Extended LSP & Code Intelligence Operations (`lsp_service`).
//!
//! Fast, reliable IDE intelligence working identically on Local workspaces and
//! Remote SSH hosts:
//! - `document_symbols`: hierarchical File Structure outline (`line`..`end_line`).
//! - `find_references`: word-boundary & comment/string-aware workspace usages.
//! - `rename_symbol`: workspace refactoring with `apply: false` preview & `apply: true` write.
//! - `format_document`: CLI formatters (`gofmt`, `rustfmt`, `prettier`, `ruff`/`black`,
//!   `google-java-format`, `buf`) with toolchain `PATH` + deterministic fallback.
//! - `code_actions`: concrete `IdeTextEdit` Quick Fixes & Refactorings (`#77`).
//! - `diagnostics`: `< 30ms` stateless Java/Go/Python/JSON/Shell/TS/Rust diagnostics (`#64`).
//! - `hover`: clean single-symbol Quick-Doc hover without neighbor bleed (`#64`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use otto_core::domain::User;
use otto_core::{Error, Result};
use tokio::process::Command;

use crate::target::ResolvedTarget;
use crate::types::{
    detect_language, IdeFormatReq, IdeFormatResp, IdeReferenceLocation, IdeReferencesReq,
    IdeReferencesResp, IdeRenameFileEdit, IdeRenameLinePreview, IdeRenameReq, IdeRenameResp,
    IdeSearchItemKind, IdeSearchMode, IdeSearchReq, IdeSymbolsReq, IdeSymbolsResp,
};
use crate::IdeCtx;

pub mod code_actions;
pub mod diagnostics;
mod diagnostics_polyglot;
mod formatting;
pub(crate) mod go_mod_nav;
pub mod hover;
mod java_diag;
mod java_diag_index;
mod navigation;
mod navigation_ext;
mod navigation_imports;
mod symbols;
mod symbols_data;

pub use code_actions::{code_actions, generate_code_actions};
pub use diagnostics::diagnostics;
pub use formatting::format_builtin_fallback;
use formatting::{formatter_candidates, run_stdin_formatter, try_native_data_format};
pub use hover::hover;
pub use navigation::{goto_definition, goto_implementation};
pub use symbols::parse_document_symbols;

/// `POST /ide/lsp/symbols` — Hierarchical Document Structure outline.
pub async fn document_symbols<S: IdeCtx>(
    ctx: &S,
    user: &User,
    req: IdeSymbolsReq,
) -> Result<IdeSymbolsResp> {
    req.target.validate()?;
    let content = match req.content {
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
    Ok(IdeSymbolsResp {
        symbols: parse_document_symbols(&req.path, &lang, &content),
    })
}

/// `POST /ide/lsp/references` — Word-boundary workspace identifier usages.
pub async fn find_references<S: IdeCtx>(
    ctx: &S,
    user: &User,
    mut req: IdeReferencesReq,
) -> Result<IdeReferencesResp> {
    req.target.absorb_project_folders(&req.project_folders);
    let resolved = ctx.ide().targets.resolve(ctx, user, &req.target).await?;
    let symbol = match req
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(s) => s.to_string(),
        None => {
            let text = match req.content.as_deref() {
                Some(c) => c.to_string(),
                None => {
                    ctx.ide()
                        .targets
                        .read_file(&resolved, &req.path)
                        .await?
                        .content
                }
            };
            extract_identifier_at(&text, req.line, req.column).ok_or_else(|| {
                Error::Invalid(format!(
                    "no identifier found at {}:{} in '{}'",
                    req.line, req.column, req.path
                ))
            })?
        }
    };
    if !is_valid_identifier(&symbol) {
        return Err(Error::Invalid(format!("invalid symbol '{symbol}'")));
    }

    let candidates =
        collect_candidate_files(ctx, &resolved, &req.target, &symbol, &req.path).await?;
    let mut locations = Vec::new();
    for path in candidates {
        let (file_lang_hint, file_content) = if path == req.path {
            if let Some(unsaved) = &req.content {
                (None, unsaved.clone())
            } else {
                let Ok(file) = ctx.ide().targets.read_file(&resolved, &path).await else {
                    continue;
                };
                (Some(file.language), file.content)
            }
        } else {
            let Ok(file) = ctx.ide().targets.read_file(&resolved, &path).await else {
                continue;
            };
            (Some(file.language), file.content)
        };
        let lang = resolve_language(ctx, &path, file_lang_hint.as_deref());
        locations.extend(find_symbol_occurrences_in_text(
            &path,
            &lang,
            &file_content,
            &symbol,
        ));
        if locations.len() >= 500 {
            locations.truncate(500);
            break;
        }
    }
    Ok(IdeReferencesResp { symbol, locations })
}

/// `POST /ide/lsp/rename` — Multi-file preview (`apply: false`) & atomic write (`apply: true`).
pub async fn rename_symbol<S: IdeCtx>(
    ctx: &S,
    user: &User,
    req: IdeRenameReq,
) -> Result<IdeRenameResp> {
    let new_name = req.new_name.trim();
    if !is_valid_identifier(new_name) {
        return Err(Error::Invalid(format!(
            "new_name '{new_name}' is not a valid identifier"
        )));
    }
    let resolved = ctx.ide().targets.resolve(ctx, user, &req.target).await?;
    let origin = ctx.ide().targets.read_file(&resolved, &req.path).await?;
    let symbol = extract_identifier_at(&origin.content, req.line, req.column).ok_or_else(|| {
        Error::Invalid(format!(
            "no identifier found at {}:{} in '{}'",
            req.line, req.column, req.path
        ))
    })?;
    if symbol == new_name {
        return Ok(IdeRenameResp {
            symbol,
            new_name: new_name.to_string(),
            applied: false,
            edits: Vec::new(),
        });
    }

    let candidates =
        collect_candidate_files(ctx, &resolved, &req.target, &symbol, &req.path).await?;
    let mut edits = Vec::new();
    let mut writes: Vec<(String, String, String)> = Vec::new();
    for path in candidates {
        let Ok(file) = ctx.ide().targets.read_file(&resolved, &path).await else {
            continue;
        };
        if file.read_only || file.truncated {
            continue;
        }
        let lang = resolve_language(ctx, &path, Some(&file.language));
        if let Some((updated, file_edit)) =
            rename_identifier_in_text(&path, &lang, &file.content, &symbol, new_name)
        {
            edits.push(file_edit);
            if req.apply {
                writes.push((path, updated, file.sha256));
            }
        }
    }

    if req.apply {
        for (path, content, expected_sha) in writes {
            ctx.ide()
                .targets
                .write_file(&resolved, &path, &content, Some(&expected_sha), false)
                .await?;
        }
    }
    Ok(IdeRenameResp {
        symbol,
        new_name: new_name.to_string(),
        applied: req.apply && !edits.is_empty(),
        edits,
    })
}

/// `POST /ide/lsp/format` — Language CLI formatter with toolchain PATH + clean fallback.
pub async fn format_document<S: IdeCtx>(
    ctx: &S,
    user: &User,
    req: IdeFormatReq,
) -> Result<IdeFormatResp> {
    let resolved = ctx.ide().targets.resolve(ctx, user, &req.target).await?;
    let lang = resolve_language(ctx, &req.path, req.language.as_deref());
    if matches!(lang.as_str(), "json" | "jsonl" | "csv" | "tsv") {
        if let Some(formatted) = try_native_data_format(&lang, &req.path, &req.content) {
            let changed = formatted != req.content;
            return Ok(IdeFormatResp {
                formatted,
                changed,
                formatter: "otto-native".into(),
            });
        }
    }
    let mut env_map = build_formatter_env(ctx, user, &resolved, &req.target).await;
    if !env_map.contains_key("PATH") {
        env_map.insert(
            "PATH".into(),
            crate::lsp_host::servers::augmented_local_path(ctx.data_dir()),
        );
    }

    for (label, cmd, args) in formatter_candidates(&lang, &req.path) {
        if let Some(formatted) =
            run_stdin_formatter(ctx, &resolved, &cmd, &args, &env_map, &req.content).await
        {
            let changed = formatted != req.content;
            return Ok(IdeFormatResp {
                formatted,
                changed,
                formatter: label.to_string(),
            });
        }
    }

    let formatted = format_builtin_fallback(&lang, &req.content);
    let changed = formatted != req.content;
    Ok(IdeFormatResp {
        formatted,
        changed,
        formatter: "builtin".into(),
    })
}

pub(super) async fn build_formatter_env<S: IdeCtx>(
    ctx: &S,
    _user: &User,
    _resolved: &ResolvedTarget,
    target: &crate::types::IdeTargetSpec,
) -> BTreeMap<String, String> {
    let mut env_map = ctx
        .ide()
        .environments
        .build_effective_env(&ctx.pool(), ctx.secrets(), target)
        .await
        .unwrap_or_default();
    if target.kind == crate::types::IdeTargetKind::Local {
        let shell_env =
            crate::environments::cached_effective_env_for_local_root(ctx.data_dir(), &target.root);
        for (k, v) in shell_env {
            env_map.entry(k).or_insert(v);
        }
        if !env_map.contains_key("PATH") {
            env_map.insert(
                "PATH".into(),
                crate::lsp_host::servers::augmented_local_path(ctx.data_dir()),
            );
        }
    }
    env_map
}

pub(super) fn resolve_language<S: IdeCtx>(ctx: &S, path: &str, hint: Option<&str>) -> String {
    if let Some(h) = hint
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "plaintext")
    {
        return h.to_ascii_lowercase();
    }
    if let Some(plugin_lang) = ctx.ide().plugins.detect_language_for_path(path) {
        return plugin_lang.id;
    }
    detect_language(path).unwrap_or("plaintext").to_string()
}

pub(super) async fn collect_candidate_files<S: IdeCtx>(
    ctx: &S,
    resolved: &ResolvedTarget,
    target_spec: &crate::types::IdeTargetSpec,
    symbol: &str,
    origin_path: &str,
) -> Result<Vec<String>> {
    if let ResolvedTarget::Local {
        root,
        additional_roots,
        is_root_user,
        ..
    } = resolved
    {
        let mut all_roots = vec![root.clone()];
        all_roots.extend(additional_roots.iter().cloned());
        if let Some(files) =
            rg_local_candidate_files_multi(&all_roots, root, *is_root_user, symbol, origin_path)
                .await
        {
            return Ok(files);
        }
    }
    let mut ordered = vec![origin_path.to_string()];
    let mut seen = BTreeSet::from([origin_path.to_string()]);
    let search_req = IdeSearchReq {
        target: target_spec.clone(),
        query: symbol.to_string(),
        mode: IdeSearchMode::Text,
        case_sensitive: true,
        whole_word: true,
        regex: false,
        file_glob: None,
        scope_path: None,
        limit: Some(250),
    };
    for item in ctx.ide().targets.search(resolved, &search_req).await?.items {
        if item.kind == IdeSearchItemKind::Text && seen.insert(item.path.clone()) {
            ordered.push(item.path);
        }
    }
    Ok(ordered)
}

#[cfg(test)]
async fn rg_local_candidate_files(
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

async fn rg_local_candidate_files_multi(
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
    cmd.args([
        "--files-with-matches",
        "--fixed-strings",
        "--hidden",
        "-g",
        "!.git",
        "-g",
        "!node_modules",
        "-g",
        "!target",
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
        if !roots.iter().any(|r| canon.starts_with(r)) {
            continue;
        }
        if !is_root_user
            && (otto_core::secret_paths::in_protected_dir(&canon)
                || otto_core::secret_paths::is_denied_dir(&canon)
                || otto_core::secret_paths::is_denied_file(&canon))
        {
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

pub fn is_ident_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_' || ch == '$'
}

pub fn is_valid_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_' || first == '$') && chars.all(is_ident_char)
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

pub(super) fn code_mask_for_line(lang: &str, line: &str, in_block_comment: &mut bool) -> Vec<bool> {
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
    let bytes = line.as_bytes();
    let mut from = 0;
    while let Some(rel) = line[from..].find(symbol) {
        let start = from + rel;
        let end = start + symbol.len();
        let code_ok = mask.get(start..end).is_some_and(|s| s.iter().all(|&b| b));
        let prev_ok = start == 0 || !is_ident_char(bytes[start - 1] as char);
        let next_ok = end == bytes.len() || !is_ident_char(bytes[end] as char);
        if code_ok && prev_ok && next_ok {
            ranges.push((start, end));
        }
        from = start + 1;
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
mod tests;
