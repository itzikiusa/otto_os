//! Structured Clean Quick-Doc Hover (`POST /ide/lsp/hover`).
//!
//! Resolves the exact symbol under cursor (local buffer + multi-repo workspace +
//! Go `replace` / `$GOROOT` / `$GOMODCACHE` + Java modules / `-sources.jar` / `src.zip`)
//! and extracts ONLY that single symbol's declaration signature and immediately
//! preceding doc-comment block — never bleeding into neighboring symbols below.

use std::path::Path;

use otto_core::domain::User;
use otto_core::{Error, Result};

use crate::types::{IdeHoverReq, IdeHoverResp, IdeReferencesReq, IdeTargetKind, IdeTargetSpec};
use crate::IdeCtx;

pub(crate) use super::hover_doc::extract_hover_from_definition;
use super::navigation::{
    extract_symbol_and_qualifier, find_definitions_in_content, goto_definition,
};
use super::{extract_identifier_at, is_ident_char, resolve_language};

/// `POST /ide/lsp/hover` — Structured Quick-Doc Hover without neighbor comment bleed.
pub async fn hover<S: IdeCtx>(ctx: &S, user: &User, req: IdeHoverReq) -> Result<IdeHoverResp> {
    if req.path.trim().is_empty() && req.content.is_none() {
        return Ok(IdeHoverResp::default());
    }

    let target_spec_opt = resolve_hover_target(&req);
    if !user.is_root && target_spec_opt.is_none() {
        return Err(Error::Forbidden("workspace target is required".into()));
    }
    let resolved_opt = match &target_spec_opt {
        Some(spec) => {
            let resolved = ctx.ide().targets.resolve(ctx, user, spec).await?;
            if !req.path.trim().is_empty() {
                let _ = ctx
                    .ide()
                    .targets
                    .resolve_readable_or_prospective_path(&resolved, &req.path)
                    .await?;
            }
            Some(resolved)
        }
        None => None,
    };

    let content = match req.content.clone() {
        Some(c) => c,
        None => {
            let Some(ref resolved) = resolved_opt else {
                return Err(Error::Invalid(
                    "target is required when content is omitted".into(),
                ));
            };
            ctx.ide()
                .targets
                .read_file(resolved, &req.path)
                .await?
                .content
        }
    };
    if content.is_empty() {
        return Ok(IdeHoverResp::default());
    }

    let lang = resolve_language(ctx, &req.path, None);
    let line = req.effective_line();
    let col_1based = resolve_hover_col_1based(&content, line, &req);
    let Some((symbol, qualifier)) =
        extract_symbol_and_qualifier(None, &lang, &content, line, col_1based)
    else {
        return Ok(IdeHoverResp::default());
    };

    let mut def_target: Option<(String, u32, String, String)> = None;

    if let (Some(spec), Some(ref resolved)) = (target_spec_opt.clone(), &resolved_opt) {
        if let Ok(def_resp) = goto_definition(
            ctx,
            user,
            IdeReferencesReq {
                target: spec,
                path: req.path.clone(),
                line,
                column: col_1based,
                character: None,
                symbol: None,
                content: Some(content.clone()),
                project_folders: req.project_folders.clone(),
            },
        )
        .await
        {
            if let Some(loc) = def_resp.locations.first() {
                if loc.path == req.path {
                    def_target = Some((loc.path.clone(), loc.line, content.clone(), lang.clone()));
                } else if let Ok(target_file) =
                    ctx.ide().targets.read_file(resolved, &loc.path).await
                {
                    let t_lang = resolve_language(ctx, &loc.path, Some(&target_file.language));
                    def_target = Some((loc.path.clone(), loc.line, target_file.content, t_lang));
                }
            }
        }
    }

    if def_target.is_none() && qualifier.is_none() {
        if let Some(loc) = find_definitions_in_content(&req.path, &lang, &content, &symbol)
            .into_iter()
            .next()
        {
            def_target = Some((req.path.clone(), loc.line, content.clone(), lang.clone()));
        }
    }

    let Some((def_path, def_line, def_content, def_lang)) = def_target else {
        return Ok(IdeHoverResp::default());
    };

    Ok(extract_hover_from_definition(
        &symbol,
        &def_path,
        def_line,
        &def_lang,
        &def_content,
    ))
}

fn resolve_hover_target(req: &IdeHoverReq) -> Option<IdeTargetSpec> {
    if let Some(mut spec) = req.target.clone() {
        if !spec.root.trim().is_empty() {
            spec.absorb_project_folders(&req.project_folders);
            return Some(spec);
        }
    }
    if let Some(first) = req.project_folders.iter().find(|s| !s.trim().is_empty()) {
        let mut spec = IdeTargetSpec {
            kind: IdeTargetKind::Local,
            root: first.trim().to_string(),
            ..Default::default()
        };
        spec.absorb_project_folders(&req.project_folders);
        return Some(spec);
    }
    let p = Path::new(&req.path);
    if p.is_absolute() {
        let mut cur = if p.is_dir() { Some(p) } else { p.parent() };
        let mut root_dir = cur.map(Path::to_path_buf);
        while let Some(d) = cur {
            if d.join(".git").exists()
                || d.join("go.mod").is_file()
                || d.join("pom.xml").is_file()
                || d.join("Cargo.toml").is_file()
                || d.join("package.json").is_file()
            {
                root_dir = Some(d.to_path_buf());
                break;
            }
            cur = d.parent();
        }
        if let Some(rd) = root_dir.filter(|d| d.is_dir()) {
            let mut spec = IdeTargetSpec {
                kind: IdeTargetKind::Local,
                root: rd.to_string_lossy().into_owned(),
                ..Default::default()
            };
            spec.absorb_project_folders(&req.project_folders);
            return Some(spec);
        }
    }
    None
}

fn resolve_hover_col_1based(content: &str, line: u32, req: &IdeHoverReq) -> u32 {
    if req.character == 0 {
        if let Some(col) = req.column.filter(|&c| c > 0) {
            return col;
        }
        if let Some(pos) = req.position {
            return pos.character.saturating_add(1);
        }
    }
    normalize_hover_column(content, line, req.effective_character())
}

/// Accept both 1-based `column` and 0-based `character` seamlessly.
fn normalize_hover_column(content: &str, line: u32, character: u32) -> u32 {
    if character == 0 {
        return 1;
    }
    let Some(line_str) = content.lines().nth(line.saturating_sub(1) as usize) else {
        return character;
    };
    let chars: Vec<char> = line_str.chars().collect();
    let idx0 = character as usize;
    let idx1 = character.saturating_sub(1) as usize;
    if idx0 < chars.len()
        && is_ident_char(chars[idx0])
        && idx1 < chars.len()
        && !is_ident_char(chars[idx1])
    {
        return character + 1;
    }
    if extract_identifier_at(content, line, character).is_some() {
        character
    } else {
        character + 1
    }
}
