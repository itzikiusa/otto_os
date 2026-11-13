//! Extended LSP & Code Intelligence Operations (`lsp_service`).
//!
//! Fast, reliable IDE intelligence working identically on Local workspaces and
//! Remote SSH hosts:
//! - `document_symbols`: hierarchical File Structure outline (`line`..`end_line`).
//! - `find_references`: word-boundary & comment/string-aware workspace usages.
//! - `rename_symbol`: workspace refactoring with `apply: false` preview & `apply: true` write.
//! - `format_document`: CLI formatters (`gofmt`, `rustfmt`, `prettier`, `ruff`/`black`,
//!   `google-java-format`, `buf`) with toolchain `PATH` + deterministic fallback.
//! - `code_actions`: context-aware Quick Fixes & Refactorings.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use otto_core::domain::User;
use otto_core::{Error, Result};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::target::ResolvedTarget;
use crate::types::{
    detect_language, IdeCodeAction, IdeCodeActionsReq, IdeCodeActionsResp, IdeDiagnosticInput,
    IdeFormatReq, IdeFormatResp, IdeReferenceLocation, IdeReferencesReq, IdeReferencesResp,
    IdeRenameFileEdit, IdeRenameLinePreview, IdeRenameReq, IdeRenameResp, IdeSearchItemKind,
    IdeSearchMode, IdeSearchReq, IdeSymbolsReq, IdeSymbolsResp,
};
use crate::IdeCtx;

mod navigation;
mod navigation_ext;
mod symbols;
mod symbols_data;

pub use navigation::{goto_definition, goto_implementation};
pub use symbols::parse_document_symbols;
use symbols::{match_brace_symbol, parse_python_symbols};

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
    req: IdeReferencesReq,
) -> Result<IdeReferencesResp> {
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
    // Fast-path native Rust formatters for JSON, JSONL, and CSV/TSV (<0.2ms, zero subprocess).
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

/// `POST /ide/lsp/code-actions` — Context-aware Quick Fixes & Refactorings.
pub async fn code_actions<S: IdeCtx>(
    ctx: &S,
    user: &User,
    req: IdeCodeActionsReq,
) -> Result<IdeCodeActionsResp> {
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
    let actions = generate_code_actions(
        &req.path,
        &lang,
        &content,
        req.line,
        req.column,
        &req.diagnostics,
    );
    Ok(IdeCodeActionsResp { actions })
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

// ===========================================================================
// 2. Identifier Extraction, Word-Boundary References & Multi-File Rename
// ===========================================================================

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

// ===========================================================================
// 3. Language Formatter Runner + Deterministic Fallback
// ===========================================================================

fn formatter_candidates(lang: &str, path: &str) -> Vec<(&'static str, String, Vec<String>)> {
    match lang {
        "go" => vec![
            ("goimports", "goimports".into(), vec![]),
            ("gofmt", "gofmt".into(), vec![]),
        ],
        "rust" => vec![(
            "rustfmt",
            "rustfmt".into(),
            vec!["--edition".into(), "2021".into()],
        )],
        "python" => vec![
            (
                "ruff",
                "ruff".into(),
                vec![
                    "format".into(),
                    "--stdin-filename".into(),
                    path.to_string(),
                    "-".into(),
                ],
            ),
            ("black", "black".into(), vec!["-q".into(), "-".into()]),
        ],
        "java" => vec![(
            "google-java-format",
            "google-java-format".into(),
            vec!["-".into()],
        )],
        "shell" | "bash" | "sh" | "zsh" => vec![(
            "shfmt",
            "shfmt".into(),
            vec!["-i".into(), "2".into(), "-ci".into(), "-".into()],
        )],
        "javascript" | "typescript" | "json" | "jsonc" | "css" | "scss" | "html" | "markdown"
        | "yaml" | "graphql" | "svelte" => vec![
            (
                "prettier",
                "prettier".into(),
                vec!["--stdin-filepath".into(), path.to_string()],
            ),
            (
                "biome",
                "biome".into(),
                vec![
                    "format".into(),
                    "--stdin-file-path".into(),
                    path.to_string(),
                ],
            ),
        ],
        "protobuf" => vec![("buf", "buf".into(), vec!["format".into(), "-".into()])],
        _ => Vec::new(),
    }
}

fn try_native_data_format(lang: &str, path: &str, content: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    if lang == "jsonl" || lower.ends_with(".jsonl") || lower.ends_with(".ndjson") {
        let mut out = Vec::new();
        for raw in content.lines() {
            let t = raw.trim();
            if t.is_empty() {
                continue;
            }
            let val: serde_json::Value = serde_json::from_str(t).ok()?;
            out.push(serde_json::to_string(&val).ok()?);
        }
        return Some(if out.is_empty() {
            String::new()
        } else {
            format!("{}\n", out.join("\n"))
        });
    }
    if lang == "json" && !lower.ends_with(".jsonc") {
        let val: serde_json::Value = serde_json::from_str(content).ok()?;
        let pretty = serde_json::to_string_pretty(&val).ok()?;
        return Some(format!("{pretty}\n"));
    }
    if matches!(lang, "csv" | "tsv") {
        let first = content.lines().find(|l| !l.trim().is_empty())?;
        let delim = symbols_data::detect_csv_delimiter(path, lang, first);
        let mut rows = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let cells: Vec<String> = symbols_data::split_delimited_row(line, delim)
                .into_iter()
                .map(|c| {
                    let t = c.trim();
                    if t.contains(delim) || t.contains('"') {
                        format!("\"{}\"", t.replace('"', "\"\""))
                    } else {
                        t.to_string()
                    }
                })
                .collect();
            rows.push(cells.join(&delim.to_string()));
        }
        return Some(if rows.is_empty() {
            String::new()
        } else {
            format!("{}\n", rows.join("\n"))
        });
    }
    None
}

async fn run_stdin_formatter<S: IdeCtx>(
    ctx: &S,
    resolved: &ResolvedTarget,
    cmd: &str,
    args: &[String],
    env_map: &BTreeMap<String, String>,
    input: &str,
) -> Option<String> {
    let mut child = ctx
        .ide()
        .targets
        .spawn_piped(resolved, cmd, args, None, env_map)
        .await
        .ok()?;
    if let Some(mut stdin) = child.stdin.take() {
        if stdin.write_all(input.as_bytes()).await.is_err() {
            let _ = child.kill().await;
            return None;
        }
        drop(stdin);
    }
    let out = tokio::time::timeout(Duration::from_secs(10), child.wait_with_output())
        .await
        .ok()?
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8(out.stdout).ok())
        .flatten()
}

pub fn format_builtin_fallback(lang: &str, content: &str) -> String {
    if matches!(lang, "json" | "jsonl" | "csv" | "tsv") {
        let fake_path = format!("doc.{lang}");
        if let Some(formatted) = try_native_data_format(lang, &fake_path, content) {
            return formatted;
        }
    }
    if matches!(lang, "shell" | "bash" | "sh" | "zsh") {
        return format_shell_fallback(content);
    }
    let normalized = content.replace("\r\n", "\n");
    let brace_indent = matches!(
        lang,
        "java"
            | "kotlin"
            | "javascript"
            | "typescript"
            | "json"
            | "jsonc"
            | "rust"
            | "go"
            | "c"
            | "cpp"
            | "protobuf"
    );
    let unit = if lang == "go" {
        "\t"
    } else if matches!(lang, "json" | "jsonc") {
        "  "
    } else {
        "    "
    };
    let mut depth = 0usize;
    let mut out_lines: Vec<String> = Vec::new();
    let mut blanks = 0usize;
    let mut in_block = false;

    for raw in normalized.lines() {
        let trimmed_end = raw.trim_end();
        if trimmed_end.trim().is_empty() {
            blanks += 1;
            if blanks <= 2 {
                out_lines.push(String::new());
            }
            continue;
        }
        blanks = 0;
        if !brace_indent {
            out_lines.push(trimmed_end.to_string());
            continue;
        }
        let trimmed = trimmed_end.trim_start();
        let mask = code_mask_for_line(lang, trimmed, &mut in_block);
        let code: String = trimmed
            .char_indices()
            .filter(|(i, _)| mask.get(*i).copied().unwrap_or(false))
            .map(|(_, c)| c)
            .collect();
        let leading = code
            .chars()
            .take_while(|&c| matches!(c, '}' | ')' | ']'))
            .count();
        out_lines.push(format!(
            "{}{}",
            unit.repeat(depth.saturating_sub(leading)),
            trimmed
        ));
        let opens = code.chars().filter(|&c| c == '{').count();
        let closes = code.chars().filter(|&c| c == '}').count();
        depth = if opens >= closes {
            depth + (opens - closes)
        } else {
            depth.saturating_sub(closes - opens)
        };
    }

    while out_lines.last().is_some_and(|l| l.is_empty()) {
        out_lines.pop();
    }
    if out_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", out_lines.join("\n"))
    }
}

fn format_shell_fallback(content: &str) -> String {
    let normalized = content.replace("\r\n", "\n");
    let mut depth = 0usize;
    let mut out_lines: Vec<String> = Vec::new();
    let mut blanks = 0usize;
    for raw in normalized.lines() {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            blanks += 1;
            if blanks <= 2 {
                out_lines.push(String::new());
            }
            continue;
        }
        blanks = 0;
        let deindent_before = matches!(trimmed, "fi" | "done" | "esac" | "}")
            || trimmed.starts_with("elif ")
            || trimmed == "else"
            || trimmed.starts_with(";;");
        let line_depth = if deindent_before {
            depth.saturating_sub(1)
        } else {
            depth
        };
        out_lines.push(format!("{}{}", "  ".repeat(line_depth), trimmed));

        if matches!(trimmed, "fi" | "done" | "esac" | "}") {
            depth = depth.saturating_sub(1);
        }
        let code_part = trimmed.split('#').next().unwrap_or("").trim();
        if code_part.ends_with("; then")
            || code_part == "then"
            || code_part.ends_with("; do")
            || code_part == "do"
            || code_part.ends_with('{')
            || (code_part.starts_with("case ") && code_part.ends_with(" in"))
        {
            depth += 1;
        }
    }
    while out_lines.last().is_some_and(|l| l.is_empty()) {
        out_lines.pop();
    }
    if out_lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", out_lines.join("\n"))
    }
}

// ===========================================================================
// 4. Context-Aware Quick Fixes & Refactoring Code Actions
// ===========================================================================

fn make_action(
    id: impl Into<String>,
    title: impl Into<String>,
    kind: &str,
    preview: String,
    replaced: String,
) -> IdeCodeAction {
    IdeCodeAction {
        id: id.into(),
        title: title.into(),
        kind: kind.into(),
        edit_preview: Some(preview),
        replacement_content: Some(replaced),
    }
}

pub fn generate_code_actions(
    _path: &str,
    lang: &str,
    content: &str,
    line: u32,
    column: u32,
    diagnostics: &[IdeDiagnosticInput],
) -> Vec<IdeCodeAction> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }
    let idx = (line.saturating_sub(1) as usize).min(lines.len() - 1);
    let mut actions = Vec::new();

    add_diagnostic_quick_fixes(
        lang,
        &lines,
        idx,
        column,
        content,
        diagnostics,
        &mut actions,
    );
    if let Some((p, r)) = build_organize_imports_action(lang, &lines, content) {
        actions.push(make_action(
            "refactor.organize_imports",
            "Organize and sort imports",
            "source.organizeImports",
            p,
            r,
        ));
    }
    if let Some((p, r)) = build_doc_comment_action(lang, &lines, idx, content) {
        actions.push(make_action(
            "refactor.add_doc_comment",
            "Add documentation comment",
            "refactor.rewrite",
            p,
            r,
        ));
    }
    if let Some((p, r)) = build_extract_variable_action(lang, &lines, idx, content) {
        actions.push(make_action(
            "refactor.extract_variable",
            "Extract expression to local variable",
            "refactor.extract",
            p,
            r,
        ));
    }
    add_language_specific_actions(lang, &lines, idx, lines[idx], content, &mut actions);
    actions
}

fn join_preserving_nl(lines: &[String], original: &str) -> String {
    let mut out = lines.join("\n");
    if original.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    out
}

fn add_diagnostic_quick_fixes(
    lang: &str,
    lines: &[&str],
    idx: usize,
    column: u32,
    content: &str,
    diagnostics: &[IdeDiagnosticInput],
    out: &mut Vec<IdeCodeAction>,
) {
    let ln = (idx + 1) as u32;
    for diag in diagnostics {
        if diag.line != ln || !diag.message.to_ascii_lowercase().contains("unused") {
            continue;
        }
        if let Some(ident) =
            extract_identifier_at(content, ln, column.max(1)).filter(|s| !s.starts_with('_'))
        {
            if let Some((replaced, _)) =
                rename_identifier_in_text("buffer", lang, content, &ident, &format!("_{ident}"))
            {
                out.push(make_action(
                    format!("quickfix.prefix_unused.{ident}"),
                    format!("Prefix unused '{ident}' with '_'"),
                    "quickfix",
                    format!("{ident} -> _{ident}"),
                    replaced,
                ));
            }
        }
        let mut without: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
        let removed = without.remove(idx);
        out.push(make_action(
            "quickfix.remove_unused_line",
            "Remove unused statement / import",
            "quickfix",
            format!("- {}", removed.trim()),
            join_preserving_nl(&without, content),
        ));
    }
}

fn build_organize_imports_action(
    lang: &str,
    lines: &[&str],
    content: &str,
) -> Option<(String, String)> {
    let is_import = |l: &str| {
        let t = l.trim();
        match lang {
            "rust" => t.starts_with("use ") && t.ends_with(';'),
            "java" | "kotlin" => t.starts_with("import "),
            "javascript" | "typescript" => t.starts_with("import ") && t.contains("from "),
            "python" => t.starts_with("import ") || t.starts_with("from "),
            _ => false,
        }
    };
    let mut start = None;
    let mut end = 0usize;
    for (i, &l) in lines.iter().enumerate() {
        if is_import(l) {
            start.get_or_insert(i);
            end = i;
        } else if start.is_some() && !l.trim().is_empty() {
            break;
        }
    }
    let s = start?;
    let orig: Vec<&str> = lines[s..=end]
        .iter()
        .copied()
        .filter(|l| !l.trim().is_empty())
        .collect();
    if orig.len() < 2 {
        return None;
    }
    let mut sorted = orig.clone();
    sorted.sort_unstable();
    sorted.dedup();
    if sorted == orig && (end - s + 1) == sorted.len() {
        return None;
    }
    let mut rebuilt: Vec<String> = Vec::with_capacity(lines.len());
    rebuilt.extend(lines[..s].iter().map(|v| (*v).to_string()));
    rebuilt.extend(sorted.iter().map(|v| (*v).to_string()));
    rebuilt.extend(lines[end + 1..].iter().map(|v| (*v).to_string()));
    Some((sorted.join("\n"), join_preserving_nl(&rebuilt, content)))
}

fn build_doc_comment_action(
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

fn build_extract_variable_action(
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

fn add_language_specific_actions(
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
    if lang == "rust" && current_line.contains(".unwrap()") {
        let replaced = current_line.replacen(".unwrap()", "?", 1);
        let mut updated: Vec<String> = lines.iter().map(|s| (*s).to_string()).collect();
        updated[idx] = replaced.clone();
        out.push(make_action(
            "quickfix.rust.unwrap_to_question",
            "Replace .unwrap() with '?' error propagation",
            "quickfix",
            replaced.trim().to_string(),
            join_preserving_nl(&updated, content),
        ));
    }
    if lang == "go"
        && (trimmed.contains(", err :=")
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
            "refactor.go.wrap_err_check",
            "Add 'if err != nil' error guard",
            "refactor.rewrite",
            "if err != nil { return err }".into(),
            join_preserving_nl(&updated, content),
        ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_symbols_hierarchical_across_5_languages() {
        let rs = "pub struct Engine {\n    id: u64,\n}\n\nimpl Engine {\n    pub fn new() -> Self {\n        Self { id: 1 }\n    }\n    pub fn run(&self) {}\n}\n";
        let rs_syms = parse_document_symbols("src/lib.rs", "rust", rs);
        assert_eq!(rs_syms.len(), 2);
        assert_eq!(rs_syms[1].name, "impl Engine");
        assert_eq!(rs_syms[1].children.len(), 2);
        assert_eq!(rs_syms[1].children[1].name, "run");

        let go_src = "package calc\ntype Adder struct {\n\tbase int\n}\nfunc (a *Adder) Add(v int) int {\n\treturn a.base + v\n}\n";
        let go_syms = parse_document_symbols("calc.go", "go", go_src);
        assert_eq!(go_syms.len(), 2);
        assert_eq!(go_syms[1].name, "Add");

        let java_src = "public class OrderService {\n    public int createOrder(String item) {\n        return 1;\n    }\n}\n";
        let java_syms = parse_document_symbols("OrderService.java", "java", java_src);
        assert_eq!(java_syms[0].children[0].name, "createOrder");

        let ts_src = "export class Store {\n  load(id: string) {\n    return id;\n  }\n}\nexport const helper = (x: number) => x + 1;\n";
        let ts_syms = parse_document_symbols("store.ts", "typescript", ts_src);
        assert_eq!(ts_syms.len(), 2);
        assert_eq!(ts_syms[0].children[0].name, "load");

        let py_src = "MAX_RETRIES = 3\n\nclass Worker:\n    def __init__(self):\n        self.ok = True\n    async def execute(self, job):\n        return job\n";
        let py_syms = parse_document_symbols("worker.py", "python", py_src);
        assert_eq!(py_syms[1].children.len(), 2);
    }

    #[test]
    fn references_and_rename_skip_comments_strings_and_substrings() {
        let src = "// computeTax comment mentions computeTax\nconst label = \"computeTax\";\nfunction computeTax(amount) {\n  return computeTaxHelper(amount) + computeTax(0);\n}\n";
        assert_eq!(
            extract_identifier_at(src, 3, 12).as_deref(),
            Some("computeTax")
        );
        let refs = find_symbol_occurrences_in_text("tax.ts", "typescript", src, "computeTax");
        assert_eq!(refs.len(), 2);
        let (renamed, edit) =
            rename_identifier_in_text("tax.ts", "typescript", src, "computeTax", "calcDuty")
                .expect("rename");
        assert_eq!(edit.occurrences, 2);
        assert!(renamed.contains("// computeTax comment"));
        assert!(renamed.contains("\"computeTax\""));
        assert!(renamed.contains("computeTaxHelper(amount)"));
        assert!(renamed.contains("function calcDuty(amount)"));
    }

    #[test]
    fn code_actions_and_fallback_formatter_work() {
        let messy_go = "package main\nfunc calc(x int) (int, error) {\nres, err := step(x)\nreturn res, err\n}\n";
        assert!(format_builtin_fallback("go", messy_go).contains("\tres, err := step(x)"));
        let actions = generate_code_actions("main.go", "go", messy_go, 3, 2, &[]);
        assert!(actions.iter().any(|a| a.id == "refactor.go.wrap_err_check"));

        let rs_src = "use std::sync::Arc;\nuse std::collections::HashMap;\n\npub fn parse_num(s: &str) -> i32 {\n    s.parse::<i32>().unwrap()\n}\n";
        let rs_actions = generate_code_actions("lib.rs", "rust", rs_src, 5, 10, &[]);
        assert!(rs_actions
            .iter()
            .any(|a| a.id == "refactor.organize_imports"));
        assert!(rs_actions
            .iter()
            .any(|a| a.id == "quickfix.rust.unwrap_to_question"));
    }

    #[tokio::test]
    async fn rg_local_candidate_files_filters_ignored_and_unmatched() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join("src/origin.rs"), "pub fn computeInvoice() {}\n").unwrap();
        std::fs::write(root.join("src/caller.rs"), "fn f() { computeInvoice(); }\n").unwrap();
        std::fs::write(root.join("src/unrelated.rs"), "fn other() {}\n").unwrap();
        std::fs::write(
            root.join("node_modules/pkg/index.js"),
            "computeInvoice();\n",
        )
        .unwrap();
        std::fs::write(root.join("target/debug/out.rs"), "computeInvoice();\n").unwrap();

        if let Some(files) =
            rg_local_candidate_files(&root, false, "computeInvoice", "src/origin.rs").await
        {
            assert_eq!(files, vec!["src/origin.rs", "src/caller.rs"]);
            let none_match =
                rg_local_candidate_files(&root, false, "neverExistsAnywhere", "src/origin.rs")
                    .await
                    .unwrap();
            assert_eq!(none_match, vec!["src/origin.rs"]);
        }
    }

    #[test]
    fn document_symbols_and_formatters_cover_shell_json_jsonl_and_csv_tsv() {
        let sh_src = "#!/usr/bin/env bash\nexport DEPLOY_ENV=\"prod\"\n\ndeploy_service() {\n  if [ -n \"$DEPLOY_ENV\" ]; then\n  echo \"ok\"\n  fi\n}\n";
        let sh_syms = parse_document_symbols("scripts/deploy.sh", "shell", sh_src);
        assert_eq!(sh_syms.len(), 2);
        assert_eq!(sh_syms[0].name, "DEPLOY_ENV");
        assert_eq!(sh_syms[1].name, "deploy_service");
        assert_eq!(sh_syms[1].line, 4);
        assert_eq!(sh_syms[1].end_line, 8);
        let sh_fmt = format_builtin_fallback("shell", sh_src);
        assert!(sh_fmt.contains("  if [ -n \"$DEPLOY_ENV\" ]; then\n    echo \"ok\"\n  fi"));

        let jsonc_src = "{\n  // service port\n  \"port\": 8080,\n  \"routes\": [\n    { \"path\": \"/health\" }\n  ]\n}\n";
        let json_syms = parse_document_symbols("config.jsonc", "jsonc", jsonc_src);
        assert_eq!(json_syms.len(), 2);
        assert_eq!(json_syms[0].name, "port");
        assert_eq!(json_syms[1].name, "routes");
        assert_eq!(json_syms[1].children.len(), 1);
        assert_eq!(json_syms[1].children[0].children[0].name, "path");

        let json_pretty = format_builtin_fallback("json", "{\"b\":1,\"a\":[true,false]}");
        assert!(json_pretty.contains("  \"b\": 1"));

        let jsonl_src = "{\"id\":\"evt-1\",\"ok\":true}\n{\"id\":\"evt-2\",\"ok\":false}\n";
        let jsonl_syms = parse_document_symbols("events.jsonl", "jsonl", jsonl_src);
        assert_eq!(jsonl_syms.len(), 2);
        assert!(jsonl_syms[0].name.contains("evt-1"));

        let csv_src = "player_id,brand_id,balance\n1001,20,50.25\n1002,20,99.00\n";
        let csv_syms = parse_document_symbols("players.csv", "csv", csv_src);
        assert_eq!(csv_syms.len(), 3);
        assert_eq!(csv_syms[0].name, "player_id");
        assert!(csv_syms[0]
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("2 rows"));
    }

    #[test]
    fn external_sdk_navigation_resolves_goroot_gomodcache_and_node_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("go_admission");
        let goroot = tmp.path().join("goroot");
        let gomodcache = tmp.path().join("pkg/mod");
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::create_dir_all(goroot.join("src/fmt")).unwrap();
        std::fs::create_dir_all(gomodcache.join("github.com/!azure/sdk@v1.2.0")).unwrap();

        std::fs::write(
            ws.join("go.mod"),
            "module bitbucket.org/org/go_admission\n\nrequire (\n\tgithub.com/Azure/sdk v1.2.0\n)\n",
        )
        .unwrap();
        let main_go = ws.join("main.go");
        let main_src = "package main\n\nimport (\n\t\"fmt\"\n\t\"github.com/Azure/sdk\"\n)\n\nfunc main() {\n\tfmt.Println(sdk.NewClient())\n}\n";
        std::fs::write(&main_go, main_src).unwrap();
        std::fs::write(
            goroot.join("src/fmt/print.go"),
            "package fmt\n\nfunc Println(a ...any) (n int, err error) { return 0, nil }\n",
        )
        .unwrap();
        std::fs::write(
            gomodcache.join("github.com/!azure/sdk@v1.2.0/client.go"),
            "package sdk\n\nfunc NewClient() string { return \"ok\" }\n",
        )
        .unwrap();

        let imports = navigation_ext::parse_go_imports(main_src);
        assert_eq!(imports.get("fmt").map(String::as_str), Some("fmt"));
        assert_eq!(
            imports.get("sdk").map(String::as_str),
            Some("github.com/Azure/sdk")
        );

        let mut env = BTreeMap::new();
        env.insert("GOROOT".into(), goroot.to_string_lossy().into_owned());
        env.insert(
            "GOMODCACHE".into(),
            gomodcache.to_string_lossy().into_owned(),
        );
        let roots = vec![ws.canonicalize().unwrap()];
        let ro = vec![
            goroot.canonicalize().unwrap(),
            gomodcache.canonicalize().unwrap(),
        ];

        let fmt_dirs = navigation_ext::resolve_go_package_dirs("fmt", &main_go, &roots, &ro, &env);
        let fmt_defs = navigation_ext::search_go_package_dirs(&fmt_dirs, "Println");
        assert_eq!(fmt_defs.len(), 1);
        assert!(fmt_defs[0].path.ends_with("src/fmt/print.go"));
        assert_eq!(fmt_defs[0].line, 3);

        let sdk_dirs = navigation_ext::resolve_go_package_dirs(
            "github.com/Azure/sdk",
            &main_go,
            &roots,
            &ro,
            &env,
        );
        let sdk_defs = navigation_ext::search_go_package_dirs(&sdk_dirs, "NewClient");
        assert_eq!(sdk_defs.len(), 1);
        assert_eq!(sdk_defs[0].line, 3);

        // Node `node_modules` `.d.ts` resolution.
        let nm_pkg = ws.join("node_modules/zod");
        std::fs::create_dir_all(&nm_pkg).unwrap();
        std::fs::write(
            nm_pkg.join("package.json"),
            "{\"name\":\"zod\",\"types\":\"index.d.ts\"}",
        )
        .unwrap();
        std::fs::write(
            nm_pkg.join("index.d.ts"),
            "export declare function object(shape: unknown): unknown;\n",
        )
        .unwrap();
        let ts_files =
            navigation_ext::resolve_js_ts_module_files("zod", &ws.join("app.ts"), &roots);
        assert_eq!(ts_files.len(), 1);
        assert!(ts_files[0].ends_with("node_modules/zod/index.d.ts"));

        // Explicit `symbol = Some("Printf")` still extracts `"fmt"` qualifier from `content`,
        // AND Homebrew `Cellar/go/<ver>/libexec/src` + `PATH` `go` canonicalization resolve `fmt/print.go`
        // even when `GOROOT` is unset.
        let go_call = "package main\nimport \"fmt\"\nfunc main() {\n\tfmt.Printf(\"ok\\n\")\n}\n";
        assert_eq!(
            navigation::extract_symbol_and_qualifier(Some("Printf"), "go", go_call, 4, 6),
            Some(("Printf".into(), Some("fmt".into())))
        );
        assert_eq!(
            navigation::extract_symbol_and_qualifier(Some("Printf"), "go", go_call, 4, 1),
            Some(("Printf".into(), Some("fmt".into())))
        );

        let brew_cellar_ver = tmp.path().join("Cellar/go/1.27.0");
        std::fs::create_dir_all(brew_cellar_ver.join("libexec/src/runtime")).unwrap();
        std::fs::create_dir_all(brew_cellar_ver.join("libexec/src/fmt")).unwrap();
        std::fs::create_dir_all(brew_cellar_ver.join("libexec/bin")).unwrap();
        std::fs::write(
            brew_cellar_ver.join("libexec/src/fmt/print.go"),
            "package fmt\n\nfunc Printf(format string, a ...any) (int, error) { return 0, nil }\n",
        )
        .unwrap();
        std::fs::write(brew_cellar_ver.join("libexec/bin/go"), "#!/bin/sh\n").unwrap();
        let mut no_goroot_env = BTreeMap::new();
        no_goroot_env.insert(
            "PATH".into(),
            brew_cellar_ver
                .join("libexec/bin")
                .to_string_lossy()
                .into_owned(),
        );
        let brew_dirs =
            navigation_ext::resolve_go_package_dirs("fmt", &main_go, &roots, &[], &no_goroot_env);
        let brew_defs = navigation_ext::search_go_package_dirs(&brew_dirs, "Printf");
        assert_eq!(brew_defs.len(), 1);
        assert!(brew_defs[0].path.ends_with("libexec/src/fmt/print.go"));
        assert_eq!(brew_defs[0].line, 3);
    }
}
