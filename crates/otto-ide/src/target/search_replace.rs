//! Multi-file search-and-replace (`dry_run` preview and atomic writes) across
//! primary and attached workspace roots on local and SSH targets.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use otto_core::Result;
use regex::Regex;

use crate::lsp_host::pool::LspPool;
use crate::types::{
    IdeReplacePreview, IdeReplaceReq, IdeReplaceResp, IdeSearchItem, IdeSearchItemKind,
    IdeSearchMode, IdeSearchReq, IdeSearchResp,
};

use super::fs_ops::resolve_local_existing_in_roots;
use super::search::{collect_local_files_multi, display_rel_path, CompiledFileGlob};
use super::{
    join_remote, map_io_err, resolve_remote_in_roots, ResolvedTarget, TargetManager,
    MAX_SEARCH_FILE_BYTES,
};

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub(super) fn replace_local_sync(
    root: &Path,
    additional_roots: &[PathBuf],
    restricted: bool,
    req: &IdeReplaceReq,
    matcher: &Regex,
    lsp_pool: Option<&LspPool>,
) -> Result<IdeReplaceResp> {
    let candidates = match &req.paths {
        Some(explicit) if !explicit.is_empty() => {
            let mut v = Vec::with_capacity(explicit.len());
            for p in explicit {
                let (canon, _) =
                    resolve_local_existing_in_roots(root, additional_roots, !restricted, p)?;
                v.push(canon.to_string_lossy().into_owned());
            }
            v
        }
        _ => collect_local_files_multi(
            root,
            additional_roots,
            restricted,
            req.scope_path.as_deref(),
            10_000,
        )?,
    };

    let compiled_glob = CompiledFileGlob::compile(req.file_glob.as_deref());
    let mut files_changed = 0usize;
    let mut total_replacements = 0usize;
    let mut previews = Vec::new();

    for full_path in candidates {
        let p = Path::new(&full_path);
        let rel = display_rel_path(p, root, additional_roots);
        if !compiled_glob.matches(&rel) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(p) else {
            continue;
        };
        if meta.len() > MAX_SEARCH_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(p) else {
            continue;
        };
        if bytes.contains(&0) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };

        let Some((replaced_text, count, sample_before, sample_after)) =
            compute_replacement(text, matcher, &req.replacement, req.regex)
        else {
            continue;
        };

        if !req.dry_run {
            use std::io::Write;
            let Ok(canon_p) = p.canonicalize() else {
                continue;
            };
            let in_roots = std::iter::once(root)
                .chain(additional_roots.iter().map(PathBuf::as_path))
                .any(|r| canon_p.starts_with(r));
            if !in_roots || (restricted && super::fs_ops::enforce_non_root_path(&canon_p).is_err())
            {
                continue;
            }
            let parent = canon_p.parent().ok_or_else(|| {
                otto_core::Error::Invalid("replace target path has no parent directory".into())
            })?;
            let existing_perms = meta.permissions();
            let mut tmp = tempfile::NamedTempFile::new_in(parent)
                .map_err(|e| map_io_err("create replace temp", &canon_p, e))?;
            tmp.write_all(replaced_text.as_bytes())
                .map_err(|e| map_io_err("write replace temp", tmp.path(), e))?;
            let _ = tmp.as_file().sync_all();
            let _ = std::fs::set_permissions(tmp.path(), existing_perms);
            tmp.persist(&canon_p)
                .map_err(|e| map_io_err("commit replace", &canon_p, e.error))?;
            if let Some(pool) = lsp_pool {
                pool.notify_watched_file_if_build_manifest(&canon_p, 2);
            }
        }

        files_changed += 1;
        total_replacements += count;
        previews.push(IdeReplacePreview {
            path: full_path,
            count,
            sample_before,
            sample_after,
        });
    }

    Ok(IdeReplaceResp {
        files_changed,
        replacements: total_replacements,
        previews,
    })
}

pub(super) async fn replace_remote_ssh(
    mgr: &TargetManager,
    target: &ResolvedTarget,
    root: &str,
    additional_roots: &[String],
    req: &IdeReplaceReq,
    matcher: &Regex,
) -> Result<IdeReplaceResp> {
    let candidates = match &req.paths {
        Some(explicit) if !explicit.is_empty() => {
            let mut out = Vec::with_capacity(explicit.len());
            for p in explicit {
                out.push(resolve_remote_in_roots(root, additional_roots, p)?.0);
            }
            out
        }
        _ => {
            let search_req = IdeSearchReq {
                target: req.target.clone(),
                query: req.query.clone(),
                mode: IdeSearchMode::Text,
                case_sensitive: req.case_sensitive,
                regex: req.regex,
                whole_word: req.whole_word,
                file_glob: req.file_glob.clone(),
                scope_path: req.scope_path.clone(),
                limit: Some(500),
            };
            let hits =
                search_remote_ssh(mgr, target, root, additional_roots, &search_req, 500).await?;
            let mut uniq: Vec<String> = hits.items.into_iter().map(|i| i.path).collect();
            uniq.sort();
            uniq.dedup();
            uniq
        }
    };

    let mut files_changed = 0usize;
    let mut total_replacements = 0usize;
    let mut previews = Vec::new();

    for remote_path in candidates {
        let Ok(file) = mgr.read_file(target, &remote_path).await else {
            continue;
        };
        if file.truncated {
            continue;
        }
        let Some((replaced_text, count, sample_before, sample_after)) =
            compute_replacement(&file.content, matcher, &req.replacement, req.regex)
        else {
            continue;
        };
        if !req.dry_run {
            mgr.write_file(
                target,
                &remote_path,
                &replaced_text,
                Some(&file.sha256),
                false,
            )
            .await?;
        }
        files_changed += 1;
        total_replacements += count;
        previews.push(IdeReplacePreview {
            path: remote_path,
            count,
            sample_before,
            sample_after,
        });
    }

    Ok(IdeReplaceResp {
        files_changed,
        replacements: total_replacements,
        previews,
    })
}

fn compute_replacement(
    text: &str,
    matcher: &Regex,
    replacement: &str,
    is_regex: bool,
) -> Option<(String, usize, String, String)> {
    let count = matcher.find_iter(text).count();
    if count == 0 {
        return None;
    }
    let mut sample_before = String::new();
    let mut sample_after = String::new();
    for line in text.lines() {
        if matcher.is_match(line) {
            sample_before = line.trim().chars().take(180).collect();
            let replaced_line = if is_regex {
                matcher.replace_all(line, replacement)
            } else {
                matcher.replace_all(line, regex::NoExpand(replacement))
            };
            sample_after = replaced_line.trim().chars().take(180).collect();
            break;
        }
    }
    let full = if is_regex {
        matcher.replace_all(text, replacement).into_owned()
    } else {
        matcher
            .replace_all(text, regex::NoExpand(replacement))
            .into_owned()
    };
    Some((full, count, sample_before, sample_after))
}

fn display_remote_rel_path(path: &str, root: &str, additional_roots: &[String]) -> String {
    if path == root {
        return String::new();
    }
    let root_prefix = format!("{}/", root.trim_end_matches('/'));
    if let Some(rel) = path.strip_prefix(&root_prefix) {
        return rel.to_string();
    }
    for extra in additional_roots {
        let extra_clean = extra.trim_end_matches('/');
        let extra_prefix = format!("{extra_clean}/");
        if let Some(rel) = path.strip_prefix(&extra_prefix) {
            let base = extra_clean.rsplit('/').next().unwrap_or("attached");
            return format!("{base}/{rel}");
        }
    }
    path.trim_start_matches('/').to_string()
}

pub(super) async fn search_remote_ssh(
    mgr: &TargetManager,
    target: &ResolvedTarget,
    root: &str,
    additional_roots: &[String],
    req: &IdeSearchReq,
    limit: usize,
) -> Result<IdeSearchResp> {
    let mut items = Vec::new();
    let mut truncated = false;
    let q_lower = req.query.to_lowercase();
    let compiled_glob = CompiledFileGlob::compile(req.file_glob.as_deref());
    let search_roots: Vec<String> = match req
        .scope_path
        .as_deref()
        .filter(|s| !s.is_empty() && *s != ".")
    {
        Some(sp) => vec![resolve_remote_in_roots(root, additional_roots, sp)?.0],
        None => std::iter::once(root.to_string())
            .chain(additional_roots.iter().cloned())
            .collect(),
    };

    if matches!(req.mode, IdeSearchMode::Files | IdeSearchMode::All) {
        let files = mgr.list_files(target, 5_000).await?;
        for f in files {
            if !search_roots
                .iter()
                .any(|sr| f == *sr || f.starts_with(&format!("{}/", sr.trim_end_matches('/'))))
            {
                continue;
            }
            let rel = display_remote_rel_path(&f, root, additional_roots);
            let matched = if req.case_sensitive {
                rel.contains(&req.query)
            } else {
                rel.to_lowercase().contains(&q_lower)
            };
            if matched && compiled_glob.matches(&rel) {
                if items.len() >= limit {
                    truncated = true;
                    break;
                }
                items.push(IdeSearchItem {
                    kind: IdeSearchItemKind::File,
                    path: f,
                    line: None,
                    column: None,
                    preview: rel,
                    symbol_kind: None,
                    container: None,
                });
            }
        }
    }

    if !truncated && !matches!(req.mode, IdeSearchMode::Files) {
        for search_root in &search_roots {
            if items.len() >= limit {
                truncated = true;
                break;
            }
            let cap = limit.saturating_sub(items.len()) + 1;
            let case_flag = if req.case_sensitive { "-s" } else { "-i" };
            let fixed_flag = if req.regex { "" } else { "-F" };
            let word_flag = if req.whole_word { "-w" } else { "" };
            let q_escaped = shell_words::quote(&req.query);
            let script = format!(
                "if command -v rg >/dev/null 2>&1; then \
                   rg --null --line-number --column --no-heading --color=never {case_flag} {fixed_flag} {word_flag} -- {q_escaped} . | head -n {cap}; \
                 else \
                   grep -rnIZ {} {} -- {q_escaped} . 2>/dev/null || grep -rnI {} {} -- {q_escaped} . | head -n {cap}; \
                 fi",
                if req.case_sensitive { "" } else { "-i" },
                if req.whole_word { "-w" } else { "" },
                if req.case_sensitive { "" } else { "-i" },
                if req.whole_word { "-w" } else { "" }
            );
            if let Ok(out) = mgr
                .exec_shell(
                    target,
                    &script,
                    Some(search_root),
                    &BTreeMap::new(),
                    Duration::from_secs(15),
                )
                .await
            {
                for raw_line in out.stdout.lines() {
                    if items.len() >= limit {
                        truncated = true;
                        break;
                    }
                    if let Some(parsed) = parse_rg_or_grep_line(search_root, raw_line) {
                        let rel = display_remote_rel_path(&parsed.path, root, additional_roots);
                        if compiled_glob.matches(&rel) {
                            items.push(parsed);
                        }
                    }
                }
            }
        }
    }

    Ok(IdeSearchResp { items, truncated })
}

fn parse_rg_or_grep_line(root: &str, raw: &str) -> Option<IdeSearchItem> {
    if let Some((file_raw, after_nul)) = raw.split_once('\0') {
        let file_part = file_raw.strip_prefix("./").unwrap_or(file_raw);
        let mut parts = after_nul.splitn(3, ':');
        let line_no: u32 = parts.next()?.trim().parse().ok()?;
        let second = parts.next()?;
        let (col, preview) = match (second.trim().parse::<u32>(), parts.next()) {
            (Ok(c), Some(rest)) => (Some(c), rest.trim()),
            _ => (Some(1), second.trim()),
        };
        return Some(IdeSearchItem {
            kind: IdeSearchItemKind::Text,
            path: join_remote(root, file_part),
            line: Some(line_no),
            column: col,
            preview: preview.chars().take(200).collect(),
            symbol_kind: None,
            container: None,
        });
    }
    let mut parts = raw.splitn(4, ':');
    let file_raw = parts.next()?;
    let file_part = file_raw.strip_prefix("./").unwrap_or(file_raw);
    let line_no: u32 = parts.next()?.trim().parse().ok()?;
    let third = parts.next()?;
    let (col, preview) = match (third.trim().parse::<u32>(), parts.next()) {
        (Ok(c), Some(rest)) => (Some(c), rest.trim()),
        _ => (Some(1), raw.splitn(3, ':').nth(2).unwrap_or("").trim()),
    };
    Some(IdeSearchItem {
        kind: IdeSearchItemKind::Text,
        path: join_remote(root, file_part),
        line: Some(line_no),
        column: col,
        preview: preview.chars().take(200).collect(),
        symbol_kind: None,
        container: None,
    })
}
