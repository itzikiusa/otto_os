//! Go to Definition (`POST /ide/lsp/definition`) & Go to Implementation (`POST /ide/lsp/implementation`)
//! across all 8 first-class IDE languages (Go, Rust, TypeScript/JavaScript, Python, Java,
//! Shell, JSON/JSONC/JSONL, CSV/TSV) — including Multi-Folder Projects & External Read-Only SDKs.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use otto_core::domain::User;
use otto_core::{Error, Result};

use crate::target::ResolvedTarget;
use crate::types::{IdeReferencesReq, IdeReferencesResp};
use crate::IdeCtx;

use super::navigation_ext::{
    resolve_external_java_definition, search_external_rust_crates, search_go_package_dirs,
};
use super::navigation_imports::{
    collect_shell_sourced_files, format_result_path, push_unique_loc,
    resolve_local_language_imports, resolve_origin_abs,
};
use super::navigation_matchers::{
    extract_go_interface_methods, find_definitions_with_matcher, find_implementations_in_content,
};
pub(crate) use super::navigation_matchers::{
    extract_symbol_and_qualifier, find_definitions_in_content, CompiledDefMatcher,
};
use super::navigation_refs::{
    find_json_key_definitions, navigate_csv_column, try_path_or_ref_navigation,
};
use super::{build_formatter_env, collect_candidate_files, resolve_language};

/// Verify that `canon` resides inside one of the authorized `project_roots` or
/// `readonly_roots` on the target, never touches denied credential paths, and
/// passes `enforce_non_root_path` for non-root callers.
pub(super) fn is_confined_nav_path(
    canon: &Path,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    is_root_user: bool,
) -> bool {
    let within_allowed = project_roots
        .iter()
        .chain(readonly_roots.iter())
        .any(|r| canon.starts_with(r));
    if !within_allowed {
        return false;
    }
    if otto_core::secret_paths::is_denied_file(canon)
        || otto_core::secret_paths::is_denied_dir(canon)
    {
        return false;
    }
    if !is_root_user && crate::target::enforce_non_root_path(canon).is_err() {
        return false;
    }
    true
}

/// Canonicalize and read a local file ONLY if it passes [`is_confined_nav_path`].
#[allow(clippy::disallowed_methods)]
pub(super) fn read_confined_nav_file(
    candidate: &Path,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    is_root_user: bool,
) -> Option<(PathBuf, String)> {
    let canon = candidate.canonicalize().ok()?;
    if !canon.is_file()
        || !is_confined_nav_path(&canon, project_roots, readonly_roots, is_root_user)
    {
        return None;
    }
    let text = std::fs::read_to_string(&canon).ok()?;
    Some((canon, text))
}

/// `POST /ide/lsp/definition` — Resolve local, multi-folder, and external read-only SDK definitions.
pub async fn goto_definition<S: IdeCtx>(
    ctx: &S,
    user: &User,
    mut req: IdeReferencesReq,
) -> Result<IdeReferencesResp> {
    req.target.absorb_project_folders(&req.project_folders);
    let resolved = ctx.ide().targets.resolve(ctx, user, &req.target).await?;
    let col = req.effective_column().max(1);
    let content = match req.content.clone() {
        Some(c) => c,
        None => {
            ctx.ide()
                .targets
                .read_file(&resolved, &req.path)
                .await?
                .content
        }
    };
    let lang = resolve_language(ctx, &req.path, None);

    // 1. Path / `$ref` / `source` / `import '...` literal navigation under cursor.
    if let Some(resp) =
        try_path_or_ref_navigation(ctx, &resolved, &req.path, &lang, &content, req.line, col).await
    {
        return Ok(resp);
    }

    // 2. CSV / TSV header column jump.
    if matches!(lang.as_str(), "csv" | "tsv") {
        if let Some(resp) = navigate_csv_column(&req.path, &lang, &content, req.line, col) {
            return Ok(resp);
        }
    }

    let (symbol, qualifier) =
        extract_symbol_and_qualifier(req.symbol.as_deref(), &lang, &content, req.line, col)
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "no navigable symbol at {}:{} in '{}'",
                    req.line, col, req.path
                ))
            })?;

    // JSON key self-definition within current document.
    if matches!(lang.as_str(), "json" | "jsonc" | "jsonl") {
        let locations = find_json_key_definitions(&req.path, &content, &[symbol.as_str()]);
        return Ok(IdeReferencesResp { symbol, locations });
    }

    let mut locations = Vec::new();
    let keep_abs = req.path.starts_with('/') || req.path.starts_with("file://");

    // 3. Language-aware external & cross-module resolution when qualified or imported.
    if let ResolvedTarget::Local {
        root,
        readonly_roots,
        is_root_user,
        ..
    } = &resolved
    {
        let project_roots = resolved.local_project_roots();
        let origin_abs = resolve_origin_abs(&req.path, &project_roots, root);
        let env_map = build_formatter_env(ctx, user, &resolved, &req.target).await;
        let is_root = *is_root_user;

        let lang_c = lang.clone();
        let content_c = content.clone();
        let sym_c = symbol.clone();
        let qual_c = qualifier.clone();
        let root_c = root.clone();
        let ro_c = readonly_roots.clone();
        if let Ok(found) = tokio::task::spawn_blocking(move || {
            resolve_local_language_imports(
                &lang_c,
                &content_c,
                &sym_c,
                qual_c.as_deref(),
                &origin_abs,
                &root_c,
                keep_abs,
                &project_roots,
                &ro_c,
                &env_map,
                is_root,
            )
        })
        .await
        {
            locations.extend(found);
        }
    }

    // 4. Same-file definitions (skip if `qualifier` explicitly targeted another package/module and already matched).
    if locations.is_empty() || qualifier.is_none() {
        let same_file = find_definitions_in_content(&req.path, &lang, &content, &symbol);
        for loc in same_file {
            push_unique_loc(&mut locations, loc);
        }
    }

    // 5. Same-package (Go) & workspace multi-root definitions.
    if let ResolvedTarget::Local {
        root,
        readonly_roots,
        is_root_user,
        ..
    } = &resolved
    {
        let project_roots = resolved.local_project_roots();
        let origin_abs = resolve_origin_abs(&req.path, &project_roots, root);
        let is_root = *is_root_user;
        if lang == "go" && (locations.is_empty() || qualifier.is_none()) {
            if let Some(pkg_dir) = origin_abs
                .parent()
                .and_then(|p| p.canonicalize().ok())
                .filter(|d| is_confined_nav_path(d, &project_roots, readonly_roots, is_root))
            {
                let sym_c = symbol.clone();
                let root_c = root.clone();
                let pr_c = project_roots.clone();
                let ro_c = readonly_roots.clone();
                if let Ok(found) = tokio::task::spawn_blocking(move || {
                    search_go_package_dirs(&[pkg_dir], &sym_c)
                        .into_iter()
                        .filter(|loc| {
                            is_confined_nav_path(Path::new(&loc.path), &pr_c, &ro_c, is_root)
                        })
                        .map(|mut loc| {
                            loc.path = format_result_path(Path::new(&loc.path), &root_c, keep_abs);
                            loc
                        })
                        .collect::<Vec<_>>()
                })
                .await
                {
                    for loc in found {
                        push_unique_loc(&mut locations, loc);
                    }
                }
            }
        }
    }

    if locations.is_empty() {
        let candidates =
            collect_candidate_files(ctx, &resolved, &req.target, &symbol, &req.path).await?;
        let mut matchers: BTreeMap<String, CompiledDefMatcher> = BTreeMap::new();
        for path in candidates {
            if path == req.path {
                continue;
            }
            let Ok(file) = ctx.ide().targets.read_file(&resolved, &path).await else {
                continue;
            };
            let file_lang = resolve_language(ctx, &path, Some(&file.language));
            if !matchers.contains_key(&file_lang) {
                if let Some(m) = CompiledDefMatcher::new(&file_lang, &symbol) {
                    matchers.insert(file_lang.clone(), m);
                }
            }
            if let Some(matcher) = matchers.get(&file_lang) {
                for loc in find_definitions_with_matcher(&path, &file.content, &symbol, matcher) {
                    push_unique_loc(&mut locations, loc);
                }
            }
            if locations.len() >= 32 {
                break;
            }
        }
    }

    // 6. External stdlib/JAR/crate fallback (Rust `std`, Java `.m2`/`src.zip`, Shell sourced scripts).
    if locations.is_empty() {
        if let ResolvedTarget::Local {
            root,
            readonly_roots,
            is_root_user,
            ..
        } = &resolved
        {
            let env_map = build_formatter_env(ctx, user, &resolved, &req.target).await;
            let project_roots = resolved.local_project_roots();
            let origin_abs = resolve_origin_abs(&req.path, &project_roots, root);
            let is_root = *is_root_user;
            let lang_c = lang.clone();
            let content_c = content.clone();
            let sym_c = symbol.clone();
            let qual_c = qualifier.clone();
            let root_c = root.clone();
            let ro_c = readonly_roots.clone();
            if let Ok(found) = tokio::task::spawn_blocking(move || match lang_c.as_str() {
                "rust" => search_external_rust_crates(qual_c.as_deref(), &sym_c, &ro_c, is_root),
                "java" | "kotlin" => resolve_external_java_definition(
                    &content_c,
                    &sym_c,
                    qual_c.as_deref(),
                    &project_roots,
                    &ro_c,
                    &env_map,
                    is_root,
                ),
                "shell" => {
                    let mut out = Vec::new();
                    for sourced in collect_shell_sourced_files(
                        &content_c,
                        &origin_abs,
                        &project_roots,
                        is_root,
                    ) {
                        if let Some((canon_src, text)) =
                            read_confined_nav_file(&sourced, &project_roots, &ro_c, is_root)
                        {
                            let p = format_result_path(&canon_src, &root_c, keep_abs);
                            out.extend(find_definitions_in_content(&p, "shell", &text, &sym_c));
                        }
                    }
                    out
                }
                _ => Vec::new(),
            })
            .await
            {
                for loc in found {
                    push_unique_loc(&mut locations, loc);
                }
            }
        }
    }

    Ok(IdeReferencesResp { symbol, locations })
}

/// `POST /ide/lsp/implementation` — Concrete implementations of interfaces, traits, & base classes.
pub async fn goto_implementation<S: IdeCtx>(
    ctx: &S,
    user: &User,
    mut req: IdeReferencesReq,
) -> Result<IdeReferencesResp> {
    req.target.absorb_project_folders(&req.project_folders);
    let resolved = ctx.ide().targets.resolve(ctx, user, &req.target).await?;
    let col = req.effective_column().max(1);
    let content = match req.content.clone() {
        Some(c) => c,
        None => {
            ctx.ide()
                .targets
                .read_file(&resolved, &req.path)
                .await?
                .content
        }
    };
    let lang = resolve_language(ctx, &req.path, None);
    let Some((symbol, _)) =
        extract_symbol_and_qualifier(req.symbol.as_deref(), &lang, &content, req.line, col)
    else {
        return goto_definition(ctx, user, req).await;
    };

    let go_iface_methods = if lang == "go" {
        extract_go_interface_methods(&content, &symbol)
    } else {
        Vec::new()
    };

    let search_term = go_iface_methods
        .first()
        .map(String::as_str)
        .unwrap_or(&symbol);
    let candidates =
        collect_candidate_files(ctx, &resolved, &req.target, search_term, &req.path).await?;
    let mut locations = Vec::new();

    for path in &candidates {
        let file_content = if path == &req.path {
            content.clone()
        } else {
            let Ok(f) = ctx.ide().targets.read_file(&resolved, path).await else {
                continue;
            };
            f.content
        };
        let file_lang = resolve_language(ctx, path, None);
        for loc in find_implementations_in_content(
            path,
            &file_lang,
            &file_content,
            &symbol,
            &go_iface_methods,
        ) {
            if !(loc.path == req.path && loc.line == req.line) {
                push_unique_loc(&mut locations, loc);
            }
        }
        if locations.len() >= 100 {
            break;
        }
    }

    if locations.is_empty() {
        return goto_definition(ctx, user, req).await;
    }
    Ok(IdeReferencesResp { symbol, locations })
}
