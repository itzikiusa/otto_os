//! Fast multi-file search (`files`, `text`, `symbols`, `all`) and multi-file
//! search-and-replace (with `dry_run` preview) across local and SSH targets.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use otto_core::{Error, Result};
use regex::{Regex, RegexBuilder};

use crate::types::{
    IdeReplacePreview, IdeReplaceReq, IdeReplaceResp, IdeSearchItem, IdeSearchItemKind,
    IdeSearchMode, IdeSearchReq, IdeSearchResp,
};

use super::{
    enforce_non_root_path, join_remote, map_io_err, resolve_local_existing, resolve_remote_subpath,
    ResolvedTarget, TargetManager, MAX_SEARCH_FILE_BYTES,
};

pub(super) fn build_matcher(query: &str, use_regex: bool, case_sensitive: bool) -> Result<Regex> {
    let pattern = if use_regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!case_sensitive)
        .build()
        .map_err(|e| Error::Invalid(format!("invalid search pattern: {e}")))
}

fn glob_matches(file_glob: Option<&str>, rel_path: &str) -> bool {
    let Some(glob) = file_glob.map(str::trim).filter(|g| !g.is_empty()) else {
        return true;
    };
    for part in glob.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if let Some(ext) = part.strip_prefix("*.") {
            if rel_path
                .rsplit_once('.')
                .is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext))
            {
                return true;
            }
        } else if rel_path.contains(part) {
            return true;
        }
    }
    false
}

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub(super) fn collect_local_files(
    root: &Path,
    restricted: bool,
    cap: usize,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                ".git" | "node_modules" | "target" | "dist" | ".svelte-kit" | "__pycache__"
            )
        })
        .build();

    for entry in walker.flatten() {
        if out.len() >= cap {
            break;
        }
        let Some(ft) = entry.file_type() else {
            continue;
        };
        if !ft.is_file() {
            continue;
        }
        let p = entry.path();
        if restricted {
            let Ok(canon) = p.canonicalize() else {
                continue;
            };
            if !canon.starts_with(root) || enforce_non_root_path(&canon).is_err() {
                continue;
            }
        }
        out.push(p.to_string_lossy().into_owned());
    }
    Ok(out)
}

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub(super) fn search_local_sync(
    root: &Path,
    restricted: bool,
    req: &IdeSearchReq,
    limit: usize,
) -> Result<IdeSearchResp> {
    let files = collect_local_files(root, restricted, 15_000)?;
    let matcher = build_matcher(&req.query, req.regex, req.case_sensitive)?;
    let q_lower = req.query.to_ascii_lowercase();
    let want_files = matches!(req.mode, IdeSearchMode::Files | IdeSearchMode::All);
    let want_symbols = matches!(req.mode, IdeSearchMode::Symbols | IdeSearchMode::All);
    let want_text = matches!(req.mode, IdeSearchMode::Text | IdeSearchMode::All);

    let mut items = Vec::new();
    let mut truncated = false;

    for full_path in &files {
        let p = Path::new(full_path);
        let rel = p
            .strip_prefix(root)
            .ok()
            .map(|r| r.to_string_lossy().replace('\\', "/"))
            .unwrap_or_else(|| full_path.clone());
        if !glob_matches(req.file_glob.as_deref(), &rel) {
            continue;
        }

        if want_files {
            let matched = if req.case_sensitive {
                rel.contains(&req.query)
            } else {
                rel.to_ascii_lowercase().contains(&q_lower)
            };
            if matched {
                if items.len() >= limit {
                    truncated = true;
                    break;
                }
                items.push(IdeSearchItem {
                    kind: IdeSearchItemKind::File,
                    path: full_path.clone(),
                    line: None,
                    column: None,
                    preview: rel.clone(),
                    symbol_kind: None,
                    container: None,
                });
            }
        }

        if !want_symbols && !want_text {
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
        if bytes[..bytes.len().min(4096)].contains(&0) {
            continue;
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            continue;
        };

        if want_symbols {
            scan_symbols_in_file(full_path, text, &matcher, limit, &mut items, &mut truncated);
            if truncated {
                break;
            }
        }
        if want_text {
            scan_text_in_file(full_path, text, &matcher, limit, &mut items, &mut truncated);
            if truncated {
                break;
            }
        }
    }

    Ok(IdeSearchResp { items, truncated })
}

fn scan_symbols_in_file(
    full_path: &str,
    text: &str,
    matcher: &Regex,
    limit: usize,
    items: &mut Vec<IdeSearchItem>,
    truncated: &mut bool,
) {
    static SYM_RE: OnceLock<Regex> = OnceLock::new();
    let sym_re = SYM_RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?:pub(?:\([^)]*\))?\s+|export\s+|default\s+|async\s+|static\s+|final\s+|abstract\s+)*(?P<kw>fn|func|def|class|struct|enum|trait|interface|type|record)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
        )
        .unwrap()
    });

    for (idx, line) in text.lines().enumerate() {
        let Some(cap) = sym_re.captures(line) else {
            continue;
        };
        let sym_name = &cap["name"];
        if !matcher.is_match(sym_name) {
            continue;
        }
        if items.len() >= limit {
            *truncated = true;
            return;
        }
        items.push(IdeSearchItem {
            kind: IdeSearchItemKind::Symbol,
            path: full_path.to_string(),
            line: Some((idx + 1) as u32),
            column: Some(1),
            preview: line.trim().chars().take(180).collect(),
            symbol_kind: Some(cap["kw"].to_string()),
            container: None,
        });
    }
}

fn scan_text_in_file(
    full_path: &str,
    text: &str,
    matcher: &Regex,
    limit: usize,
    items: &mut Vec<IdeSearchItem>,
    truncated: &mut bool,
) {
    for (idx, line) in text.lines().enumerate() {
        let Some(m) = matcher.find(line) else {
            continue;
        };
        if items.len() >= limit {
            *truncated = true;
            return;
        }
        items.push(IdeSearchItem {
            kind: IdeSearchItemKind::Text,
            path: full_path.to_string(),
            line: Some((idx + 1) as u32),
            column: Some((m.start() + 1) as u32),
            preview: line.trim().chars().take(200).collect(),
            symbol_kind: None,
            container: None,
        });
    }
}

pub(super) async fn search_remote_ssh(
    mgr: &TargetManager,
    target: &ResolvedTarget,
    root: &str,
    req: &IdeSearchReq,
    limit: usize,
) -> Result<IdeSearchResp> {
    let mut items = Vec::new();
    let mut truncated = false;
    let q_lower = req.query.to_ascii_lowercase();

    if matches!(req.mode, IdeSearchMode::Files | IdeSearchMode::All) {
        let files = mgr.list_files(target, 5_000).await?;
        for f in files {
            let rel = f.strip_prefix(root).unwrap_or(&f).trim_start_matches('/');
            let matched = if req.case_sensitive {
                rel.contains(&req.query)
            } else {
                rel.to_ascii_lowercase().contains(&q_lower)
            };
            if matched && glob_matches(req.file_glob.as_deref(), rel) {
                if items.len() >= limit {
                    truncated = true;
                    break;
                }
                let preview = rel.to_string();
                items.push(IdeSearchItem {
                    kind: IdeSearchItemKind::File,
                    path: f,
                    line: None,
                    column: None,
                    preview,
                    symbol_kind: None,
                    container: None,
                });
            }
        }
    }

    if !truncated && !matches!(req.mode, IdeSearchMode::Files) {
        let cap = limit.saturating_sub(items.len()) + 1;
        let case_flag = if req.case_sensitive { "-s" } else { "-i" };
        let fixed_flag = if req.regex { "" } else { "-F" };
        let q_escaped = shell_words::quote(&req.query);
        let script = format!(
            "if command -v rg >/dev/null 2>&1; then \
               rg --line-number --column --no-heading --color=never {case_flag} {fixed_flag} -- {q_escaped} . | head -n {cap}; \
             else \
               grep -rnI {} -- {q_escaped} . | head -n {cap}; \
             fi",
            if req.case_sensitive { "" } else { "-i" }
        );
        if let Ok(out) = mgr
            .exec_shell(
                target,
                &script,
                Some(root),
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
                if let Some(parsed) = parse_rg_or_grep_line(root, raw_line) {
                    items.push(parsed);
                }
            }
        }
    }

    Ok(IdeSearchResp { items, truncated })
}

fn parse_rg_or_grep_line(root: &str, raw: &str) -> Option<IdeSearchItem> {
    let mut parts = raw.splitn(4, ':');
    let file_part = parts.next()?.trim().trim_start_matches("./");
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

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub(super) fn replace_local_sync(
    root: &Path,
    restricted: bool,
    req: &IdeReplaceReq,
    matcher: &Regex,
) -> Result<IdeReplaceResp> {
    let candidates = match &req.paths {
        Some(explicit) if !explicit.is_empty() => {
            let mut v = Vec::with_capacity(explicit.len());
            for p in explicit {
                v.push(
                    resolve_local_existing(root, !restricted, p)?
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            v
        }
        _ => collect_local_files(root, restricted, 10_000)?,
    };

    let mut files_changed = 0usize;
    let mut total_replacements = 0usize;
    let mut previews = Vec::new();

    for full_path in candidates {
        let p = Path::new(&full_path);
        let Ok(meta) = std::fs::metadata(p) else {
            continue;
        };
        if meta.len() > MAX_SEARCH_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(p) else {
            continue;
        };
        if bytes[..bytes.len().min(4096)].contains(&0) {
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
            let tmp = p.with_extension("otto-replace-tmp");
            std::fs::write(&tmp, replaced_text.as_bytes())
                .map_err(|e| map_io_err("write replace temp", &tmp, e))?;
            std::fs::rename(&tmp, p).map_err(|e| map_io_err("commit replace", p, e))?;
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
    req: &IdeReplaceReq,
    matcher: &Regex,
) -> Result<IdeReplaceResp> {
    let candidates = match &req.paths {
        Some(explicit) if !explicit.is_empty() => {
            let mut out = Vec::with_capacity(explicit.len());
            for p in explicit {
                out.push(resolve_remote_subpath(root, p)?);
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
                file_glob: None,
                limit: Some(500),
            };
            let hits = search_remote_ssh(mgr, target, root, &search_req, 500).await?;
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
