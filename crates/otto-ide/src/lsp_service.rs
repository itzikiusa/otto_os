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
    IdeRenameReq, IdeRenameResp, IdeSearchItemKind, IdeSearchMode, IdeSearchReq, IdeSymbolsReq,
    IdeSymbolsResp,
};
use crate::IdeCtx;

pub mod code_actions;
mod code_actions_lang;
pub mod diagnostics;
mod diagnostics_go;
mod diagnostics_polyglot;
mod formatting;
pub(crate) mod go_mod_nav;
mod go_mod_resolve;
pub mod hover;
mod hover_doc;
mod java_diag;
mod java_diag_index;
mod navigation;
mod navigation_ext;
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
    req: IdeRenameReq,
) -> Result<IdeRenameResp> {
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
    let symbol = extract_identifier_at(&origin.content, req.line, col).ok_or_else(|| {
        Error::Invalid(format!(
            "no identifier found at {}:{} in '{}'",
            req.line, col, req.path
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
    if !env_map.contains_key("PATH") && req.target.kind == crate::types::IdeTargetKind::Local {
        env_map.insert(
            "PATH".into(),
            crate::lsp_host::servers::augmented_local_path_for_user(
                ctx.data_dir(),
                None,
                user.is_root,
            ),
        );
    }

    for (label, cmd, args) in formatter_candidates(&lang, &req.path, req.organize_imports) {
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
    let mut env_map = match ctx
        .ide()
        .environments
        .build_effective_env_for_run_as(&ctx.pool(), ctx.secrets(), target, None, user.is_root)
        .await
    {
        Ok(map) => map,
        Err(_) if target.kind == crate::types::IdeTargetKind::Local => ctx
            .ide()
            .environments
            .cached_effective_env_for_local_root_as(&target.root, user.is_root),
        Err(_) => BTreeMap::new(),
    };
    if target.kind == crate::types::IdeTargetKind::Local && !env_map.contains_key("PATH") {
        env_map.insert(
            "PATH".into(),
            crate::lsp_host::servers::augmented_local_path_for_user(
                ctx.data_dir(),
                None,
                user.is_root,
            ),
        );
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
mod tests;
#[cfg(test)]
mod tests_polyglot;
