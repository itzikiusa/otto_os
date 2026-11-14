//! Go to Definition (`POST /ide/lsp/definition`) & Go to Implementation (`POST /ide/lsp/implementation`)
//! across all 8 first-class IDE languages (Go, Rust, TypeScript/JavaScript, Python, Java,
//! Shell, JSON/JSONC/JSONL, CSV/TSV) — including Multi-Folder Projects & External Read-Only SDKs.

use std::path::Path;
use std::sync::OnceLock;

use otto_core::domain::User;
use otto_core::{Error, Result};
use regex::Regex;

use crate::target::ResolvedTarget;
use crate::types::{IdeReferenceLocation, IdeReferencesReq, IdeReferencesResp};
use crate::IdeCtx;

use super::navigation_ext::{
    extract_quoted_string_at, extract_ref_value, find_json_key_definitions, navigate_csv_column,
    resolve_external_java_definition, resolve_js_ts_module_files, search_external_rust_crates,
    search_go_package_dirs,
};
use super::navigation_imports::{
    collect_shell_sourced_files, format_result_path, push_unique_loc,
    resolve_local_language_imports, resolve_origin_abs, resolve_shell_source_target,
};
use super::{
    build_formatter_env, code_mask_for_line, collect_candidate_files, extract_identifier_at,
    is_ident_char, resolve_language,
};

/// `POST /ide/lsp/definition` — Resolve local, multi-folder, and external read-only SDK definitions.
pub async fn goto_definition<S: IdeCtx>(
    ctx: &S,
    user: &User,
    mut req: IdeReferencesReq,
) -> Result<IdeReferencesResp> {
    req.target.absorb_project_folders(&req.project_folders);
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
        let project_roots = resolved.local_project_roots();
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
        let project_roots = resolved.local_project_roots();
        let origin_abs = resolve_origin_abs(&req.path, &project_roots, root);
        if lang == "go" && (locations.is_empty() || qualifier.is_none()) {
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

    // 6. External stdlib/JAR/crate fallback (Rust `std`, Java `.m2`/`src.zip`, Shell sourced scripts).
    if locations.is_empty() {
        if let ResolvedTarget::Local {
            root,
            readonly_roots,
            ..
        } = &resolved
        {
            let env_map = build_formatter_env(ctx, user, &resolved, &req.target).await;
            let project_roots = resolved.local_project_roots();
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
                    &project_roots,
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
    mut req: IdeReferencesReq,
) -> Result<IdeReferencesResp> {
    req.target.absorb_project_folders(&req.project_folders);
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
                let roots = resolved.local_project_roots();
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

    if lang == "shell" {
        let trimmed = line_str.trim();
        if let Some(rest) = trimmed
            .strip_prefix("source ")
            .or_else(|| trimmed.strip_prefix(". "))
        {
            if let ResolvedTarget::Local { root, .. } = resolved {
                let roots = resolved.local_project_roots();
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

    if matches!(lang, "javascript" | "typescript" | "svelte")
        && (line_str.contains("from ")
            || line_str.contains("import(")
            || line_str.contains("require("))
    {
        if let Some(spec) = extract_quoted_string_at(line_str, column) {
            if let ResolvedTarget::Local { root, .. } = resolved {
                let roots = resolved.local_project_roots();
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
// Definition & Implementation line matchers across all 8 languages
// ---------------------------------------------------------------------------

pub(crate) fn find_definitions_in_content(
    path: &str,
    lang: &str,
    content: &str,
    symbol: &str,
) -> Vec<IdeReferenceLocation> {
    let mut out = Vec::new();
    let mut in_block = false;
    let mut go_decl_group = false;
    for (idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mask = code_mask_for_line(lang, line, &mut in_block);
        let code_head = trimmed.split("//").next().unwrap_or("").trim();
        if lang == "go" {
            if (code_head.starts_with("const") || code_head.starts_with("var"))
                && code_head.ends_with('(')
            {
                go_decl_group = true;
                continue;
            }
            if go_decl_group && code_head == ")" {
                go_decl_group = false;
                continue;
            }
        }
        if !line.contains(symbol) {
            continue;
        }
        if is_definition_line(lang, code_head, symbol, go_decl_group) {
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

pub(crate) fn is_definition_line(
    lang: &str,
    trimmed: &str,
    sym: &str,
    in_go_decl_group: bool,
) -> bool {
    let escaped = regex::escape(sym);
    if lang == "go" && in_go_decl_group {
        let group_pat = format!(r"^{escaped}(?:\s+[A-Za-z0-9_*.\[\]]+)?\s*(?:=.*)?$");
        if Regex::new(&group_pat).is_ok_and(|re| re.is_match(trimmed)) {
            return true;
        }
    }
    let pat = match lang {
        "go" => format!(
            r"^(?:func\s+(?:\([^)]+\)\s*)?{escaped}\s*(?:\[[^\]]*\])?\s*\(|type\s+{escaped}\b|(?:const|var)\s+{escaped}\b|{escaped}\s*:=)"
        ),
        "rust" => format!(
            r"^(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?(?:fn|struct|enum|trait|type|const|static|mod|union)\s+{escaped}\b|^macro_rules!\s*{escaped}\b"
        ),
        "python" => format!(r"^(?:async\s+)?(?:def|class)\s+{escaped}\b|^{escaped}\s*(?::[^=]+)?="),
        "java" | "kotlin" => format!(
            r"^(?:(?:public|private|protected|static|final|abstract|synchronized|default|sealed|open|data|internal|override|suspend)\s+)*(?:class|interface|enum|record|object|fun)\s+{escaped}\b|^(?:(?:public|private|protected|static|final|synchronized|abstract|default)\s+)*(?:<[^>]+>\s+)?[A-Za-z0-9_<>,.\[\]?]+\s+{escaped}\s*\(|^(?:(?:public|private|protected|static|final|volatile|transient)\s+)+[A-Za-z0-9_<>,.\[\]?]+\s+{escaped}\s*(?:=|;)"
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

pub(crate) fn extract_symbol_and_qualifier(
    explicit: Option<&str>,
    lang: &str,
    content: &str,
    line: u32,
    column: u32,
) -> Option<(String, Option<String>)> {
    let explicit_clean = explicit.map(str::trim).filter(|s| !s.is_empty());
    if let Some(s) = explicit_clean {
        if let Some((q, sym)) = s.rsplit_once("::").or_else(|| s.rsplit_once('.')) {
            return Some((sym.to_string(), Some(q.to_string())));
        }
    }
    let sym = match explicit_clean {
        Some(s) => s.to_string(),
        None => {
            if lang == "shell" {
                extract_shell_ident_at(content, line, column)?
            } else {
                extract_identifier_at(content, line, column)?
            }
        }
    };
    let Some(line_str) = content.lines().nth(line.saturating_sub(1) as usize) else {
        return Some((sym, None));
    };
    let chars: Vec<char> = line_str.chars().collect();
    if chars.is_empty() {
        return Some((sym, None));
    }
    let mut pos = (column.saturating_sub(1) as usize).min(chars.len().saturating_sub(1));
    while pos > 0 && (is_ident_char(chars[pos - 1]) || (lang == "shell" && chars[pos - 1] == '-')) {
        pos -= 1;
    }
    let qual_end = if pos >= 1 && chars[pos - 1] == '.' {
        Some(pos - 1)
    } else if pos >= 2 && chars[pos - 2] == ':' && chars[pos - 1] == ':' {
        Some(pos - 2)
    } else {
        None
    };
    let mut qualifier = qual_end.and_then(|qe| {
        if qe == 0 {
            return None;
        }
        let mut qs = qe;
        while qs > 0 && is_ident_char(chars[qs - 1]) {
            qs -= 1;
        }
        (qs < qe).then(|| chars[qs..qe].iter().collect::<String>())
    });

    if qualifier.is_none() && explicit_clean.is_some() {
        for sep in ["::", "."] {
            let needle = format!("{sep}{sym}");
            let mut search_from = 0usize;
            while let Some(rel_idx) = line_str[search_from..].find(&needle) {
                let sep_byte = search_from + rel_idx;
                let after_byte = sep_byte + needle.len();
                let boundary_ok = line_str[after_byte..]
                    .chars()
                    .next()
                    .is_none_or(|c| !is_ident_char(c));
                if boundary_ok && sep_byte > 0 {
                    let prefix_chars: Vec<char> = line_str[..sep_byte].chars().collect();
                    let qe = prefix_chars.len();
                    let mut qs = qe;
                    while qs > 0 && is_ident_char(prefix_chars[qs - 1]) {
                        qs -= 1;
                    }
                    if qs < qe {
                        qualifier = Some(prefix_chars[qs..qe].iter().collect());
                        break;
                    }
                }
                search_from = after_byte;
            }
            if qualifier.is_some() {
                break;
            }
        }
    }

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
