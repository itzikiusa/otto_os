//! Extended LSP & Code Intelligence Operations (`lsp_service`).
//!
//! Fast, reliable IDE intelligence working identically on Local workspaces and
//! Remote SSH hosts:
//! - `document_symbols`: hierarchical File Structure outline (`line`..`end_line`).
//! - `find_references`: word-boundary & comment/string-aware workspace usages.
//! - `rename_symbol`: workspace refactoring with `apply: false` preview & `apply: true` write.
//! - `format_document`: CLI formatters (`goimports`/`gofmt`, `rustfmt`, `prettier`, `ruff`/`black`,
//!   `google-java-format`, `buf`) with toolchain `PATH` + deterministic fallback.
//! - `code_actions`: concrete `IdeTextEdit` Quick Fixes & Refactorings.
//! - `diagnostics`: `< 30ms` stateless Java/Go/Python/JSON/Shell/TS/Rust diagnostics.
//! - `hover`: clean single-symbol Quick-Doc hover without neighbor bleed.

use std::collections::{BTreeMap, BTreeSet};

use otto_core::domain::User;
use otto_core::{Error, Result};

use crate::target::{HistoryWriteOpts, ResolvedTarget};
use crate::types::{
    detect_language, IdeFormatReq, IdeFormatResp, IdeReferencesReq, IdeReferencesResp,
    IdeRenameFileEdit, IdeRenameLinePreview, IdeRenameReq, IdeRenameResp, IdeSearchItemKind,
    IdeSearchMode, IdeSearchReq, IdeSymbolsReq, IdeSymbolsResp,
};
use crate::IdeCtx;

pub mod code_actions;
mod code_actions_lang;
pub mod diagnostics;
mod diagnostics_go;
mod diagnostics_polyglot;
mod formatting;
pub mod go_mod_nav;
pub use crate::go_build_tags::{
    canonicalize_go_build_tags, extract_go_file_build_tags, is_gopls_build_tag_exclusion_msg,
    is_valid_go_build_tag, merge_go_tags_into_args, strip_go_tags_from_args,
    unify_go_build_flags_and_args, validate_go_build_tags,
};
mod go_mod_resolve;
pub mod hover;
mod hover_doc;
mod java_diag;
mod java_diag_index;
mod navigation;
mod navigation_ext;
mod navigation_go_recv;
mod navigation_imports;
mod navigation_java;
mod navigation_matchers;
mod navigation_refs;
mod symbols;
mod symbols_data;
mod text_scan;

pub use code_actions::{code_actions, generate_code_actions};
pub use diagnostics::diagnostics;
pub use formatting::{format_builtin_fallback, format_builtin_fallback_opts};
use formatting::{
    formatter_candidates, organize_imports_if_requested, run_stdin_formatter,
    try_native_data_format,
};
pub use hover::hover;
pub use navigation::{goto_definition, goto_implementation};
pub use symbols::parse_document_symbols;
pub(crate) use text_scan::code_mask_for_line;
#[cfg(test)]
use text_scan::rg_local_candidate_files;
use text_scan::rg_local_candidate_files_multi;
pub use text_scan::{
    extract_identifier_at, find_symbol_occurrences_in_text, is_ident_char, is_valid_identifier,
    rename_identifier_in_text,
};

/// `POST /ide/lsp/symbols` — Hierarchical Document Structure outline.
pub async fn document_symbols<S: IdeCtx>(
    ctx: &S,
    user: &User,
    req: IdeSymbolsReq,
) -> Result<IdeSymbolsResp> {
    req.target.validate()?;
    let has_root_auth = otto_core::auth::root_authority(user);
    let resolved_opt =
        if !req.target.root.trim().is_empty() || !has_root_auth || req.content.is_none() {
            let resolved = ctx.ide().targets.resolve(ctx, user, &req.target).await?;
            if !req.path.trim().is_empty() {
                let _ = ctx
                    .ide()
                    .targets
                    .resolve_readable_or_prospective_path(&resolved, &req.path)
                    .await?;
            }
            Some(resolved)
        } else {
            None
        };
    let content = match req.content {
        Some(c) => c,
        None => {
            let resolved = resolved_opt
                .as_ref()
                .expect("resolved when content is None");
            ctx.ide()
                .targets
                .read_file(resolved, &req.path)
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
    if !req.path.trim().is_empty() {
        let _ = ctx
            .ide()
            .targets
            .resolve_readable_or_prospective_path(&resolved, &req.path)
            .await?;
    }
    let col = req.effective_column().max(1);
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
            extract_identifier_at(&text, req.line, col).ok_or_else(|| {
                Error::Invalid(format!(
                    "no identifier found at {}:{} in '{}'",
                    req.line, col, req.path
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
    mut req: IdeRenameReq,
) -> Result<IdeRenameResp> {
    req.target.absorb_project_folders(&req.project_folders);
    let new_name = req.new_name.trim();
    if !is_valid_identifier(new_name) {
        return Err(Error::Invalid(format!(
            "new_name '{new_name}' is not a valid identifier"
        )));
    }
    let resolved = if req.apply {
        ctx.ide()
            .targets
            .resolve_write(ctx, user, &req.target)
            .await?
    } else {
        ctx.ide().targets.resolve(ctx, user, &req.target).await?
    };
    let col = req.effective_column().max(1);
    let origin = ctx.ide().targets.read_file(&resolved, &req.path).await?;
    let symbol = if let Some(explicit) = req
        .symbol
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && is_valid_identifier(s))
    {
        explicit.to_string()
    } else {
        let source = req.content.as_deref().unwrap_or(&origin.content);
        extract_identifier_at(source, req.line, col)
            .or_else(|| extract_identifier_at(&origin.content, req.line, col))
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "no identifier found at {}:{} in '{}'",
                    req.line, col, req.path
                ))
            })?
    };
    if symbol == new_name {
        return Ok(IdeRenameResp {
            symbol,
            new_name: new_name.to_string(),
            applied: false,
            edits: Vec::new(),
            source: "text".into(),
        });
    }

    let lsp_edits = req.lsp_edits.take().filter(|e| !e.is_empty());
    let source = if lsp_edits.is_some() { "lsp" } else { "text" };
    let mut candidates = match &lsp_edits {
        Some(edits) => edits.iter().fold(Vec::new(), |mut acc, e| {
            if !acc.contains(&e.path) {
                acc.push(e.path.clone());
            }
            acc
        }),
        None => collect_candidate_files(ctx, &resolved, &req.target, &symbol, &req.path).await?,
    };
    if !candidates
        .iter()
        .any(|c| c == &req.path || c == &origin.path)
    {
        candidates.insert(0, req.path.clone());
    }
    let mut seen_paths = std::collections::HashSet::with_capacity(candidates.len());
    let mut edits = Vec::new();
    let mut writes: Vec<(String, String, String)> = Vec::new();
    // The text fallback only looks at files of the origin's language: a word match
    // in a README or another language's source is never the same symbol.
    let lang_family = |lang: &str| match crate::lsp_host::servers::normalize_lsp_lang(lang) {
        "javascript" => "typescript".to_string(),
        other => other.to_string(),
    };
    let origin_lang = lang_family(&resolve_language(ctx, &req.path, Some(&origin.language)));
    for path in candidates {
        let Ok(file) = ctx.ide().targets.read_file(&resolved, &path).await else {
            continue;
        };
        if !seen_paths.insert(file.path.clone()) || file.read_only || file.truncated {
            continue;
        }
        if let Some(all) = &lsp_edits {
            let unsaved = req.content.as_ref().filter(|_| file.path == origin.path);
            let content = unsaved.unwrap_or(&file.content);
            let mine: Vec<_> = all.iter().filter(|e| e.path == path).cloned().collect();
            if mine.is_empty() {
                continue;
            }
            let Some(updated) = code_actions::apply_edits_to_text(content, &mine) else {
                return Err(Error::Invalid(format!(
                    "the language server's rename edits don't fit '{path}'; save it and try again"
                )));
            };
            edits.push(rename_preview(&path, content, &updated, &mine));
            if req.apply {
                writes.push((path, updated, file.sha256));
            }
            continue;
        }
        let lang = resolve_language(ctx, &path, Some(&file.language));
        if file.path != origin.path && lang_family(&lang) != origin_lang {
            continue;
        }
        if file.path == origin.path {
            if let Some(ref unsaved) = req.content {
                if let Some((updated, file_edit)) =
                    rename_identifier_in_text(&path, &lang, unsaved, &symbol, new_name)
                {
                    edits.push(file_edit);
                    if req.apply {
                        writes.push((path, updated, file.sha256));
                    }
                } else if unsaved != &file.content {
                    let file_edit =
                        rename_identifier_in_text(&path, &lang, &file.content, &symbol, new_name)
                            .map(|(_, fe)| fe)
                            .unwrap_or_else(|| {
                                let ln_idx = req.line.saturating_sub(1) as usize;
                                let before =
                                    file.content.lines().nth(ln_idx).unwrap_or("").to_string();
                                let after = unsaved.lines().nth(ln_idx).unwrap_or("").to_string();
                                IdeRenameFileEdit {
                                    path: path.clone(),
                                    occurrences: 1,
                                    preview_lines: vec![IdeRenameLinePreview {
                                        line: req.line.max(1),
                                        before,
                                        after,
                                    }],
                                }
                            });
                    edits.push(file_edit);
                    if req.apply {
                        writes.push((path, unsaved.clone(), file.sha256));
                    }
                }
                continue;
            }
        }
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
        let history_opts = HistoryWriteOpts {
            enabled: req.local_history_enabled,
            retention_days: req.local_history_retention_days,
        };
        for (path, content, expected_sha) in writes {
            ctx.ide()
                .targets
                .write_file_with_history(
                    &resolved,
                    &path,
                    &content,
                    Some(&expected_sha),
                    false,
                    history_opts,
                )
                .await?;
        }
    }
    Ok(IdeRenameResp {
        symbol,
        new_name: new_name.to_string(),
        applied: req.apply && !edits.is_empty(),
        edits,
        source: source.into(),
    })
}

/// Preview of a language server's rename in one file: each edited line, before and after.
fn rename_preview(
    path: &str,
    before: &str,
    after: &str,
    edits: &[crate::types::IdeTextEdit],
) -> IdeRenameFileEdit {
    let mut lines: Vec<u32> = edits.iter().map(|e| e.start_line).collect();
    lines.sort_unstable();
    lines.dedup();
    let line_at = |text: &str, n: u32| text.lines().nth(n as usize - 1).unwrap_or("").to_string();
    IdeRenameFileEdit {
        path: path.to_string(),
        occurrences: edits.len(),
        preview_lines: lines
            .into_iter()
            .take(25)
            .map(|line| IdeRenameLinePreview {
                line,
                before: line_at(before, line),
                after: line_at(after, line),
            })
            .collect(),
    }
}

/// `POST /ide/lsp/format` — Language CLI formatter with toolchain PATH + clean fallback.
pub async fn format_document<S: IdeCtx>(
    ctx: &S,
    user: &User,
    req: IdeFormatReq,
) -> Result<IdeFormatResp> {
    let resolved = ctx.ide().targets.resolve(ctx, user, &req.target).await?;
    let confined_path = if req.path.trim().is_empty() {
        String::new()
    } else {
        ctx.ide()
            .targets
            .resolve_path(&resolved, &req.path, false)
            .await?
    };
    let effective_path = if confined_path.is_empty() {
        req.path.as_str()
    } else {
        confined_path.as_str()
    };
    let lang = resolve_language(ctx, effective_path, req.language.as_deref());
    if matches!(lang.as_str(), "json" | "jsonl" | "csv" | "tsv") {
        if let Some(formatted) = try_native_data_format(&lang, effective_path, &req.content) {
            let changed = formatted != req.content;
            return Ok(IdeFormatResp {
                formatted,
                changed,
                formatter: "otto-native".into(),
            });
        }
    }
    let has_root_auth = otto_core::auth::root_authority(user);
    let mut env_map = build_formatter_env(ctx, user, &resolved, &req.target).await;
    if !env_map.contains_key("PATH") && req.target.kind == crate::types::IdeTargetKind::Local {
        env_map.insert(
            "PATH".into(),
            crate::lsp_host::servers::augmented_local_path_for_user(
                ctx.data_dir(),
                None,
                has_root_auth,
            ),
        );
    }

    for (label, cmd, args) in
        formatter_candidates(&lang, effective_path, req.organize_imports, has_root_auth)
    {
        if let Some(raw_formatted) =
            run_stdin_formatter(ctx, &resolved, &cmd, &args, &env_map, &req.content).await
        {
            let formatted = if req.organize_imports && label != "goimports" {
                organize_imports_if_requested(&lang, &raw_formatted)
            } else {
                raw_formatted
            };
            let changed = formatted != req.content;
            return Ok(IdeFormatResp {
                formatted,
                changed,
                formatter: label.to_string(),
            });
        }
    }

    let formatted = format_builtin_fallback_opts(&lang, &req.content, req.organize_imports);
    let changed = formatted != req.content;
    Ok(IdeFormatResp {
        formatted,
        changed,
        formatter: "builtin".into(),
    })
}

pub(super) async fn build_formatter_env<S: IdeCtx>(
    ctx: &S,
    user: &User,
    _resolved: &ResolvedTarget,
    target: &crate::types::IdeTargetSpec,
) -> BTreeMap<String, String> {
    let has_root_auth = otto_core::auth::root_authority(user);
    if target.kind == crate::types::IdeTargetKind::Local {
        ctx.ide().environments.ensure_hydrated(&ctx.pool()).await;
        let mut env_map = ctx
            .ide()
            .environments
            .cached_probe_env_for_local_root_as(&target.root, has_root_auth);
        if !env_map.contains_key("PATH") {
            env_map.insert(
                "PATH".into(),
                crate::lsp_host::servers::augmented_local_path_for_user(
                    ctx.data_dir(),
                    None,
                    has_root_auth,
                ),
            );
        }
        return env_map;
    }
    ctx.ide()
        .environments
        .build_effective_env_for_run_as(&ctx.pool(), ctx.secrets(), target, None, has_root_auth)
        .await
        .unwrap_or_default()
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
mod tests;
#[cfg(test)]
mod tests_polyglot;
