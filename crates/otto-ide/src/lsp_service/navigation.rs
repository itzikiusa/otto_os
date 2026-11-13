//! Go to Definition (`POST /ide/lsp/definition`) & Go to Implementation (`POST /ide/lsp/implementation`)
//! across all 8 first-class IDE languages (Go, Rust, TypeScript/JavaScript, Python, Java,
//! Shell, JSON/JSONC/JSONL, CSV/TSV) — including Multi-Folder Projects & External Read-Only SDKs.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use otto_core::domain::User;
use otto_core::{Error, Result};
use regex::Regex;

use crate::target::ResolvedTarget;
use crate::types::{IdeReferenceLocation, IdeReferencesReq, IdeReferencesResp};
use crate::IdeCtx;

use super::navigation_ext::{
    parse_go_imports, resolve_external_java_definition, resolve_go_package_dirs,
    resolve_js_ts_module_files, resolve_python_module_files, search_external_rust_crates,
    search_go_package_dirs,
};
use super::symbols_data::{detect_csv_delimiter, split_delimited_row};
use super::{
    build_formatter_env, code_mask_for_line, collect_candidate_files, extract_identifier_at,
    is_ident_char, resolve_language,
};

/// `POST /ide/lsp/definition` — Resolve local, multi-folder, and external read-only SDK definitions.
pub async fn goto_definition<S: IdeCtx>(
    ctx: &S,
    user: &User,
    req: IdeReferencesReq,
) -> Result<IdeReferencesResp> {
    let resolved = ctx.ide().targets.resolve(ctx, user, &req.target).await?;
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
    if let Some(resp) = try_path_or_ref_navigation(
        ctx, &resolved, &req.path, &lang, &content, req.line, req.column,
    )
    .await
    {
        return Ok(resp);
    }

    // 2. CSV / TSV header column jump.
    if matches!(lang.as_str(), "csv" | "tsv") {
        if let Some(resp) = navigate_csv_column(&req.path, &lang, &content, req.line, req.column) {
            return Ok(resp);
        }
    }

    let (symbol, qualifier) =
        extract_symbol_and_qualifier(req.symbol.as_deref(), &lang, &content, req.line, req.column)
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "no navigable symbol at {}:{} in '{}'",
                    req.line, req.column, req.path
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
        ..
    } = &resolved
    {
        let project_roots: Vec<PathBuf> = resolved
            .local_project_roots()
            .into_iter()
            .map(Path::to_path_buf)
            .collect();
        let origin_abs = resolve_origin_abs(&req.path, &project_roots, root);
        let env_map = build_formatter_env(ctx, user, &resolved, &req.target).await;

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
    if let ResolvedTarget::Local { root, .. } = &resolved {
        let project_roots: Vec<PathBuf> = resolved
            .local_project_roots()
            .into_iter()
            .map(Path::to_path_buf)
            .collect();
        let origin_abs = resolve_origin_abs(&req.path, &project_roots, root);
        if lang == "go" {
            if let Some(pkg_dir) = origin_abs.parent().map(Path::to_path_buf) {
                let sym_c = symbol.clone();
                let root_c = root.clone();
                if let Ok(found) = tokio::task::spawn_blocking(move || {
                    search_go_package_dirs(&[pkg_dir], &sym_c)
                        .into_iter()
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
        for path in candidates {
            if path == req.path {
                continue;
            }
            let Ok(file) = ctx.ide().targets.read_file(&resolved, &path).await else {
                continue;
            };
            let file_lang = resolve_language(ctx, &path, Some(&file.language));
            for loc in find_definitions_in_content(&path, &file_lang, &file.content, &symbol) {
                push_unique_loc(&mut locations, loc);
            }
            if locations.len() >= 32 {
                break;
            }
        }
    }

    // 6. Unqualified external stdlib/crate fallback (Rust `std`, Java `java.lang.*`, Shell sourced scripts).
    if locations.is_empty() {
        if let ResolvedTarget::Local {
            root,
            readonly_roots,
            ..
        } = &resolved
        {
            let env_map = build_formatter_env(ctx, user, &resolved, &req.target).await;
            let project_roots: Vec<PathBuf> = resolved
                .local_project_roots()
                .into_iter()
                .map(Path::to_path_buf)
                .collect();
            let origin_abs = resolve_origin_abs(&req.path, &project_roots, root);
            let lang_c = lang.clone();
            let content_c = content.clone();
            let sym_c = symbol.clone();
            let qual_c = qualifier.clone();
            let root_c = root.clone();
            let ro_c = readonly_roots.clone();
            if let Ok(found) = tokio::task::spawn_blocking(move || match lang_c.as_str() {
                "rust" => search_external_rust_crates(qual_c.as_deref(), &sym_c, &ro_c),
                "java" | "kotlin" => resolve_external_java_definition(
                    &content_c,
                    &sym_c,
                    qual_c.as_deref(),
                    &ro_c,
                    &env_map,
                ),
                "shell" => {
                    let mut out = Vec::new();
                    for sourced in
                        collect_shell_sourced_files(&content_c, &origin_abs, &project_roots)
                    {
                        if let Ok(text) = std::fs::read_to_string(&sourced) {
                            let p = format_result_path(&sourced, &root_c, keep_abs);
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
    req: IdeReferencesReq,
) -> Result<IdeReferencesResp> {
    let resolved = ctx.ide().targets.resolve(ctx, user, &req.target).await?;
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
        extract_symbol_and_qualifier(req.symbol.as_deref(), &lang, &content, req.line, req.column)
    else {
        return goto_definition(ctx, user, req).await;
    };

    // For Go interfaces: if cursor is on `type Foo interface { MethodA(...) ... }`, also collect method names!
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
            // Exclude the cursor's own line when jumping from an interface/trait declaration.
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

// ---------------------------------------------------------------------------
// Path / `$ref` / Shell `source` / JS import string navigation
// ---------------------------------------------------------------------------

async fn try_path_or_ref_navigation<S: IdeCtx>(
    ctx: &S,
    resolved: &ResolvedTarget,
    origin_path: &str,
    lang: &str,
    content: &str,
    line: u32,
    column: u32,
) -> Option<IdeReferencesResp> {
    let line_str = content.lines().nth(line.saturating_sub(1) as usize)?;

    // JSON / JSONC `$ref` or relative file pointer (`"./schema.json#/definitions/User"`).
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
            } else if let ResolvedTarget::Local { root, .. } = resolved {
                let roots: Vec<PathBuf> = resolved
                    .local_project_roots()
                    .into_iter()
                    .map(Path::to_path_buf)
                    .collect();
                let origin_abs = resolve_origin_abs(origin_path, &roots, root);
                if let Some(target_abs) = origin_abs
                    .parent()
                    .map(|p| p.join(file_part))
                    .and_then(|p| p.canonicalize().ok())
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

    // Shell `source ./lib.sh` or `. ./lib.sh`.
    if lang == "shell" {
        let trimmed = line_str.trim();
        if let Some(rest) = trimmed
            .strip_prefix("source ")
            .or_else(|| trimmed.strip_prefix(". "))
        {
            if let ResolvedTarget::Local { root, .. } = resolved {
                let roots: Vec<PathBuf> = resolved
                    .local_project_roots()
                    .into_iter()
                    .map(Path::to_path_buf)
                    .collect();
                let origin_abs = resolve_origin_abs(origin_path, &roots, root);
                if let Some(target_abs) = resolve_shell_source_target(rest, &origin_abs, &roots) {
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

    // JS / TS cursor directly inside module string of `import ... from 'spec'`.
    if matches!(lang, "javascript" | "typescript" | "svelte")
        && (line_str.contains("from ")
            || line_str.contains("import(")
            || line_str.contains("require("))
    {
        if let Some(spec) = extract_quoted_string_at(line_str, column) {
            if let ResolvedTarget::Local { root, .. } = resolved {
                let roots: Vec<PathBuf> = resolved
                    .local_project_roots()
                    .into_iter()
                    .map(Path::to_path_buf)
                    .collect();
                let origin_abs = resolve_origin_abs(origin_path, &roots, root);
                let files = resolve_js_ts_module_files(&spec, &origin_abs, &roots);
                if let Some(first) = files.first() {
                    let p = format_result_path(first, root, origin_path.starts_with('/'));
                    return Some(IdeReferencesResp {
                        symbol: spec,
                        locations: vec![IdeReferenceLocation {
                            path: p,
                            line: 1,
                            column: 1,
                            preview: std::fs::read_to_string(first)
                                .ok()
                                .and_then(|s| s.lines().next().map(|l| l.trim().to_string()))
                                .unwrap_or_default(),
                        }],
                    });
                }
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// Cross-module & External SDK resolution per language
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments, clippy::disallowed_methods)] // runs inside spawn_blocking
fn resolve_local_language_imports(
    lang: &str,
    content: &str,
    symbol: &str,
    qualifier: Option<&str>,
    origin_abs: &Path,
    primary_root: &Path,
    keep_abs: bool,
    project_roots: &[PathBuf],
    readonly_roots: &[PathBuf],
    env_map: &std::collections::BTreeMap<String, String>,
) -> Vec<IdeReferenceLocation> {
    let mut out = Vec::new();
    match lang {
        "go" => {
            let imports = parse_go_imports(content);
            let matched_import = qualifier
                .and_then(|q| imports.get(q))
                .or_else(|| imports.get(symbol));
            if let Some(import_path) = matched_import {
                let pkg_dirs = resolve_go_package_dirs(
                    import_path,
                    origin_abs,
                    project_roots,
                    readonly_roots,
                    env_map,
                );
                if qualifier.is_some() {
                    for mut loc in search_go_package_dirs(&pkg_dirs, symbol) {
                        loc.path = format_result_path(Path::new(&loc.path), primary_root, keep_abs);
                        push_unique_loc(&mut out, loc);
                    }
                } else if let Some(dir) = pkg_dirs.first() {
                    // Clicked directly on package alias `fmt` -> open first `.go` file in package.
                    if let Ok(rd) = std::fs::read_dir(dir) {
                        let mut go_files: Vec<PathBuf> = rd
                            .flatten()
                            .map(|e| e.path())
                            .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("go"))
                            .collect();
                        go_files.sort();
                        if let Some(first_go) = go_files.first().and_then(|p| p.canonicalize().ok())
                        {
                            out.push(IdeReferenceLocation {
                                path: format_result_path(&first_go, primary_root, keep_abs),
                                line: 1,
                                column: 1,
                                preview: format!("package {symbol}"),
                            });
                        }
                    }
                }
            }
        }
        "javascript" | "typescript" | "svelte" => {
            for (imported_orig, specifier) in
                find_js_import_sources_for_symbol(content, symbol, qualifier)
            {
                let target_sym = imported_orig.as_deref().unwrap_or(symbol);
                for file in resolve_js_ts_module_files(&specifier, origin_abs, project_roots) {
                    if let Ok(text) = std::fs::read_to_string(&file) {
                        let p = format_result_path(&file, primary_root, keep_abs);
                        let defs = find_definitions_in_content(&p, "typescript", &text, target_sym);
                        if !defs.is_empty() {
                            for d in defs {
                                push_unique_loc(&mut out, d);
                            }
                        } else {
                            // Follow 1-hop `export * from './...'` / `export { X } from './...'` in `.d.ts` barrels!
                            for re_spec in extract_js_reexports(&text, target_sym) {
                                for sub in
                                    resolve_js_ts_module_files(&re_spec, &file, project_roots)
                                {
                                    if let Ok(sub_text) = std::fs::read_to_string(&sub) {
                                        let sp = format_result_path(&sub, primary_root, keep_abs);
                                        for d in find_definitions_in_content(
                                            &sp,
                                            "typescript",
                                            &sub_text,
                                            target_sym,
                                        ) {
                                            push_unique_loc(&mut out, d);
                                        }
                                    }
                                }
                            }
                        }
                        if out.is_empty() {
                            out.push(IdeReferenceLocation {
                                path: p,
                                line: 1,
                                column: 1,
                                preview: text.lines().next().unwrap_or("").trim().to_string(),
                            });
                        }
                    }
                }
            }
        }
        "python" => {
            for (orig_sym, mod_spec) in find_python_import_sources(content, symbol, qualifier) {
                let target_sym = orig_sym.as_deref().unwrap_or(symbol);
                for file in resolve_python_module_files(
                    &mod_spec,
                    origin_abs,
                    project_roots,
                    readonly_roots,
                ) {
                    if let Ok(text) = std::fs::read_to_string(&file) {
                        let p = format_result_path(&file, primary_root, keep_abs);
                        let defs = find_definitions_in_content(&p, "python", &text, target_sym);
                        if !defs.is_empty() {
                            for d in defs {
                                push_unique_loc(&mut out, d);
                            }
                        } else if orig_sym.is_none() {
                            out.push(IdeReferenceLocation {
                                path: p,
                                line: 1,
                                column: 1,
                                preview: text.lines().next().unwrap_or("").trim().to_string(),
                            });
                        }
                    }
                }
            }
        }
        "rust" => {
            if let Some(q) = qualifier {
                if matches!(q, "std" | "core" | "alloc") {
                    out.extend(search_external_rust_crates(Some(q), symbol, readonly_roots));
                }
            }
            if out.is_empty() {
                if let Some(crate_name) = find_rust_use_crate(content, symbol, qualifier) {
                    out.extend(search_external_rust_crates(
                        Some(&crate_name),
                        symbol,
                        readonly_roots,
                    ));
                }
            }
        }
        _ => {}
    }
    out
}

// ---------------------------------------------------------------------------
// Definition & Implementation line matchers across all 8 languages
// ---------------------------------------------------------------------------

pub(super) fn find_definitions_in_content(
    path: &str,
    lang: &str,
    content: &str,
    symbol: &str,
) -> Vec<IdeReferenceLocation> {
    let mut out = Vec::new();
    let mut in_block = false;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mask = code_mask_for_line(lang, line, &mut in_block);
        if !line.contains(symbol) {
            continue;
        }
        if is_definition_line(lang, trimmed, symbol) {
            let col = find_symbol_column(line, &mask, symbol).unwrap_or(1);
            out.push(IdeReferenceLocation {
                path: path.to_string(),
                line: (idx + 1) as u32,
                column: col,
                preview: trimmed.to_string(),
            });
        }
    }
    out
}

fn is_definition_line(lang: &str, trimmed: &str, sym: &str) -> bool {
    let escaped = regex::escape(sym);
    let pat = match lang {
        "go" => format!(
            r"^(?:func\s+(?:\([^)]+\)\s*)?{escaped}\s*(?:\[[^\]]*\])?\s*\(|type\s+{escaped}\b|(?:const|var)\s+{escaped}\b|{escaped}\s*:=)"
        ),
        "rust" => format!(
            r"^(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:fn|struct|enum|trait|type|const|static|mod|union)\s+{escaped}\b|^macro_rules!\s*{escaped}\b"
        ),
        "python" => format!(r"^(?:async\s+)?(?:def|class)\s+{escaped}\b|^{escaped}\s*(?::[^=]+)?="),
        "java" | "kotlin" => format!(
            r"^(?:(?:public|private|protected|static|final|abstract|synchronized|default|sealed|open|data|internal|override|suspend)\s+)*(?:class|interface|enum|record|object|fun)\s+{escaped}\b|^(?:(?:public|private|protected|static|final|synchronized|abstract|default)\s+)*(?:<[^>]+>\s+)?[A-Za-z0-9_<>,.\[\]?]+\s+{escaped}\s*\("
        ),
        "shell" | "bash" | "sh" | "zsh" => format!(
            r"^(?:function\s+)?{escaped}\s*\(\s*\)|^function\s+{escaped}(?:\s*\{{|\s*$)|^(?:export\s+|readonly\s+|local\s+)?{escaped}="
        ),
        _ => format!(
            r"^(?:export\s+)?(?:default\s+)?(?:declare\s+)?(?:abstract\s+)?(?:async\s+)?(?:function\s*\*?|class|interface|type|enum|namespace|const|let|var)\s+{escaped}\b|^(?:(?:public|private|protected|static|readonly|async|get|set)\s+)*{escaped}\s*(?:<[^>]*>)?\s*\("
        ),
    };
    Regex::new(&pat).is_ok_and(|re| re.is_match(trimmed))
}

fn find_implementations_in_content(
    path: &str,
    lang: &str,
    content: &str,
    symbol: &str,
    go_iface_methods: &[String],
) -> Vec<IdeReferenceLocation> {
    let mut out = Vec::new();
    let escaped = regex::escape(symbol);
    let pat = match lang {
        "go" => {
            if !go_iface_methods.is_empty() {
                let alts = go_iface_methods
                    .iter()
                    .map(|m| regex::escape(m))
                    .collect::<Vec<_>>()
                    .join("|");
                format!(r"^func\s*\([^)]+\)\s*(?:{alts})\s*(?:\[[^\]]*\])?\s*\(")
            } else {
                format!(r"^func\s*\([^)]+\)\s*{escaped}\s*(?:\[[^\]]*\])?\s*\(")
            }
        }
        "rust" => format!(
            r"^(?:unsafe\s+)?impl(?:<[^>]*>)?\s+(?:[A-Za-z0-9_:]+::)*{escaped}(?:<[^>]*>)?\s+for\b|^\s*(?:pub\s+)?(?:async\s+)?fn\s+{escaped}\s*(?:<[^>]*>)?\s*\("
        ),
        "java" | "kotlin" => format!(
            r"\b(?:implements|extends)\s+[^{{]*\b{escaped}\b|^(?:(?:public|protected|private|override|open)\s+)+[A-Za-z0-9_<>,.\[\]?\s]+\b{escaped}\s*\("
        ),
        "python" => format!(
            r"^class\s+[A-Za-z_][A-Za-z0-9_]*\s*\([^)]*\b{escaped}\b|^\s+(?:async\s+)?def\s+{escaped}\s*\("
        ),
        _ => format!(
            r"\b(?:implements|extends)\s+[^{{]*\b{escaped}\b|^\s*(?:public\s+|async\s+)*{escaped}\s*\("
        ),
    };
    let Ok(re) = Regex::new(&pat) else {
        return out;
    };
    let mut in_block = false;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        let mask = code_mask_for_line(lang, line, &mut in_block);
        if re.is_match(trimmed) {
            let col = find_symbol_column(line, &mask, symbol).unwrap_or(1);
            out.push(IdeReferenceLocation {
                path: path.to_string(),
                line: (idx + 1) as u32,
                column: col,
                preview: trimmed.to_string(),
            });
        }
    }
    out
}

fn extract_go_interface_methods(content: &str, iface_name: &str) -> Vec<String> {
    let header_re = Regex::new(&format!(
        r"^type\s+{}\s+interface\s*\{{",
        regex::escape(iface_name)
    ))
    .ok();
    let Some(re) = header_re else {
        return Vec::new();
    };
    let mut in_iface = false;
    let mut methods = Vec::new();
    static RE_M: OnceLock<Regex> = OnceLock::new();
    let m_re = RE_M.get_or_init(|| Regex::new(r"^([A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap());
    for line in content.lines() {
        let t = line.split("//").next().unwrap_or("").trim();
        if !in_iface {
            if re.is_match(t) {
                in_iface = true;
            }
            continue;
        }
        if t.starts_with('}') {
            break;
        }
        if let Some(cap) = m_re.captures(t) {
            methods.push(cap[1].to_string());
        }
    }
    methods
}

// ---------------------------------------------------------------------------
// Helpers: Qualifier extraction, JSON keys, CSV columns, JS/Python/Rust/Shell imports
// ---------------------------------------------------------------------------

fn extract_symbol_and_qualifier(
    explicit: Option<&str>,
    lang: &str,
    content: &str,
    line: u32,
    column: u32,
) -> Option<(String, Option<String>)> {
    if let Some(s) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        if let Some((q, sym)) = s.rsplit_once("::").or_else(|| s.rsplit_once('.')) {
            return Some((sym.to_string(), Some(q.to_string())));
        }
        return Some((s.to_string(), None));
    }
    let sym = if lang == "shell" {
        extract_shell_ident_at(content, line, column)
    } else {
        extract_identifier_at(content, line, column)
    }?;
    let line_str = content.lines().nth(line.saturating_sub(1) as usize)?;
    let chars: Vec<char> = line_str.chars().collect();
    let mut pos = (column.saturating_sub(1) as usize).min(chars.len().saturating_sub(1));
    while pos > 0 && (is_ident_char(chars[pos - 1]) || (lang == "shell" && chars[pos - 1] == '-')) {
        pos -= 1;
    }
    // Inspect `.` or `::` immediately preceding `sym`.
    let qual_end = if pos >= 1 && chars[pos - 1] == '.' {
        Some(pos - 1)
    } else if pos >= 2 && chars[pos - 2] == ':' && chars[pos - 1] == ':' {
        Some(pos - 2)
    } else {
        None
    };
    let qualifier = qual_end.and_then(|qe| {
        if qe == 0 {
            return None;
        }
        let mut qs = qe;
        while qs > 0 && is_ident_char(chars[qs - 1]) {
            qs -= 1;
        }
        (qs < qe).then(|| chars[qs..qe].iter().collect::<String>())
    });
    Some((sym, qualifier))
}

fn extract_shell_ident_at(content: &str, line: u32, column: u32) -> Option<String> {
    let line_str = content.lines().nth(line.saturating_sub(1) as usize)?;
    let chars: Vec<char> = line_str.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let is_sh = |c: char| is_ident_char(c) || c == '-';
    let mut pos = (column.saturating_sub(1) as usize).min(chars.len() - 1);
    if !is_sh(chars[pos]) {
        if pos > 0 && is_sh(chars[pos - 1]) {
            pos -= 1;
        } else {
            return None;
        }
    }
    let mut s = pos;
    while s > 0 && is_sh(chars[s - 1]) {
        s -= 1;
    }
    let mut e = pos;
    while e + 1 < chars.len() && is_sh(chars[e + 1]) {
        e += 1;
    }
    let out: String = chars[s..=e].iter().collect();
    (!out.is_empty()).then_some(out)
}

fn find_symbol_column(line: &str, mask: &[bool], symbol: &str) -> Option<u32> {
    let mut from = 0;
    while let Some(rel) = line[from..].find(symbol) {
        let s = from + rel;
        let e = s + symbol.len();
        let code_ok = mask.get(s..e).is_some_and(|sl| sl.iter().all(|&b| b));
        if code_ok {
            return Some(line[..s].chars().count() as u32 + 1);
        }
        from = s + 1;
    }
    line.find(symbol)
        .map(|s| line[..s].chars().count() as u32 + 1)
}

fn find_json_key_definitions(
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

fn navigate_csv_column(
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

fn extract_quoted_string_at(line: &str, column: u32) -> Option<String> {
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

fn extract_ref_value(line: &str) -> Option<String> {
    let after = line.split("\"$ref\"").nth(1)?;
    let q1 = after.find('"')?;
    let rest = &after[q1 + 1..];
    let q2 = rest.find('"')?;
    Some(rest[..q2].to_string())
}

fn find_js_import_sources_for_symbol(
    content: &str,
    symbol: &str,
    qualifier: Option<&str>,
) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    let lookup = qualifier.unwrap_or(symbol);
    for raw in content.lines() {
        let t = raw.trim();
        if !t.starts_with("import ") && !t.starts_with("export ") && !t.contains("require(") {
            continue;
        }
        let Some(spec) = extract_last_quoted(t) else {
            continue;
        };
        if qualifier.is_some()
            && (t.contains(&format!("* as {lookup}")) || t.contains(&format!("import {lookup}")))
        {
            out.push((Some(symbol.to_string()), spec));
            continue;
        }
        if let Some(brace_s) = t.find('{') {
            if let Some(brace_e) = t[brace_s..].find('}') {
                for item in t[brace_s + 1..brace_s + brace_e].split(',') {
                    let clean = item.trim().trim_start_matches("type ").trim();
                    if let Some((orig, alias)) = clean.split_once(" as ") {
                        if alias.trim() == lookup {
                            out.push((Some(orig.trim().to_string()), spec.clone()));
                        }
                    } else if clean == lookup {
                        out.push((Some(clean.to_string()), spec.clone()));
                    }
                }
            }
        } else if t.contains(&format!("import {lookup}")) || t.contains(&format!("const {lookup}"))
        {
            out.push((Some(symbol.to_string()), spec));
        }
    }
    out
}

fn extract_js_reexports(content: &str, symbol: &str) -> Vec<String> {
    let mut specs = Vec::new();
    for raw in content.lines() {
        let t = raw.trim();
        if t.starts_with("export *") || (t.starts_with("export ") && t.contains(symbol)) {
            if let Some(s) = extract_last_quoted(t) {
                specs.push(s);
            }
        }
    }
    specs
}

fn extract_last_quoted(line: &str) -> Option<String> {
    for q in ['\'', '"'] {
        if let Some(end) = line.rfind(q) {
            if let Some(start) = line[..end].rfind(q) {
                return Some(line[start + 1..end].to_string());
            }
        }
    }
    None
}

fn find_python_import_sources(
    content: &str,
    symbol: &str,
    qualifier: Option<&str>,
) -> Vec<(Option<String>, String)> {
    let mut out = Vec::new();
    let lookup = qualifier.unwrap_or(symbol);
    for raw in content.lines() {
        let t = raw.split('#').next().unwrap_or("").trim();
        if let Some(rest) = t.strip_prefix("from ") {
            if let Some((mod_part, imp_part)) = rest.split_once(" import ") {
                let mod_spec = mod_part.trim().to_string();
                for item in imp_part.trim_matches(['(', ')']).split(',') {
                    let clean = item.trim();
                    if let Some((orig, alias)) = clean.split_once(" as ") {
                        if alias.trim() == lookup {
                            out.push((
                                Some(
                                    if qualifier.is_some() {
                                        symbol
                                    } else {
                                        orig.trim()
                                    }
                                    .to_string(),
                                ),
                                mod_spec.clone(),
                            ));
                        }
                    } else if clean == lookup {
                        out.push((Some(symbol.to_string()), mod_spec.clone()));
                    }
                }
            }
        } else if let Some(rest) = t.strip_prefix("import ") {
            for item in rest.split(',') {
                let clean = item.trim();
                let (mod_spec, alias) = clean
                    .split_once(" as ")
                    .map(|(m, a)| (m.trim(), a.trim()))
                    .unwrap_or((clean, clean.rsplit('.').next().unwrap_or(clean)));
                if alias == lookup {
                    out.push((qualifier.map(|_| symbol.to_string()), mod_spec.to_string()));
                }
            }
        }
    }
    out
}

fn find_rust_use_crate(content: &str, symbol: &str, qualifier: Option<&str>) -> Option<String> {
    if let Some(q) = qualifier {
        if !matches!(q, "self" | "super" | "crate" | "Self") {
            return Some(q.to_string());
        }
    }
    for raw in content.lines() {
        let t = raw.trim();
        if let Some(rest) = t.strip_prefix("use ") {
            if rest.contains(symbol) {
                let root_seg = rest.split("::").next()?.trim();
                if !matches!(root_seg, "self" | "super" | "crate") {
                    return Some(root_seg.to_string());
                }
            }
        }
    }
    None
}

fn collect_shell_sourced_files(
    content: &str,
    origin_abs: &Path,
    project_roots: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for raw in content.lines() {
        let t = raw.trim();
        if let Some(rest) = t.strip_prefix("source ").or_else(|| t.strip_prefix(". ")) {
            if let Some(p) = resolve_shell_source_target(rest, origin_abs, project_roots) {
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    out
}

fn resolve_shell_source_target(
    raw_arg: &str,
    origin_abs: &Path,
    project_roots: &[PathBuf],
) -> Option<PathBuf> {
    let token = raw_arg
        .split('#')
        .next()
        .unwrap_or("")
        .split_whitespace()
        .next()?
        .trim_matches(['"', '\'']);
    let base_dir = origin_abs.parent()?;
    let dir_str = base_dir.to_string_lossy();
    let mut cleaned = token.to_string();
    for prefix in [
        "${BASH_SOURCE%/*}",
        "$(dirname \"$0\")",
        "$(dirname $0)",
        "${SCRIPT_DIR}",
        "$SCRIPT_DIR",
        "${DIR}",
        "$DIR",
    ] {
        if cleaned.starts_with(prefix) {
            cleaned = format!("{dir_str}{}", &cleaned[prefix.len()..]);
        }
    }
    let cand = if Path::new(&cleaned).is_absolute() {
        PathBuf::from(&cleaned)
    } else {
        base_dir.join(&cleaned)
    };
    if let Ok(c) = cand.canonicalize() {
        if c.is_file() {
            return Some(c);
        }
    }
    for r in project_roots {
        if let Ok(c) = r.join(&cleaned).canonicalize() {
            if c.is_file() {
                return Some(c);
            }
        }
    }
    None
}

fn resolve_origin_abs(path: &str, project_roots: &[PathBuf], primary_root: &Path) -> PathBuf {
    let clean = crate::target::sdk_paths::strip_file_uri(path);
    let p = Path::new(&clean);
    if p.is_absolute() {
        return p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    }
    for r in project_roots {
        let cand = r.join(&clean);
        if let Ok(c) = cand.canonicalize() {
            return c;
        }
    }
    primary_root.join(&clean)
}

fn format_result_path(canon: &Path, primary_root: &Path, keep_abs: bool) -> String {
    if !keep_abs {
        if let Ok(rel) = canon.strip_prefix(primary_root) {
            return rel.to_string_lossy().replace('\\', "/");
        }
    }
    canon.to_string_lossy().into_owned()
}

fn push_unique_loc(out: &mut Vec<IdeReferenceLocation>, loc: IdeReferenceLocation) {
    let mut seen: BTreeSet<(&str, u32)> = out.iter().map(|x| (x.path.as_str(), x.line)).collect();
    if seen.insert((loc.path.as_str(), loc.line)) {
        out.push(loc);
    }
}
