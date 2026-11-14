//! Fast multi-file search (`files`, `text`, `symbols`, `all`) and multi-file
//! search-and-replace (with `dry_run` preview) across primary + attached
//! project roots on local and SSH targets.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use otto_core::{Error, Result};
use regex::{Regex, RegexBuilder};

use crate::lsp_host::pool::LspPool;
use crate::types::{
    IdeReplacePreview, IdeReplaceReq, IdeReplaceResp, IdeSearchItem, IdeSearchItemKind,
    IdeSearchMode, IdeSearchReq, IdeSearchResp,
};

use super::fs_ops::resolve_local_existing_in_roots;
use super::{
    enforce_non_root_path, join_remote, map_io_err, resolve_remote_in_roots, ResolvedTarget,
    TargetManager, MAX_SEARCH_FILE_BYTES,
};

pub(super) fn build_matcher(
    query: &str,
    use_regex: bool,
    case_sensitive: bool,
    whole_word: bool,
) -> Result<Regex> {
    let core = if use_regex {
        query.to_string()
    } else {
        regex::escape(query)
    };
    let pattern = if whole_word {
        let lead_word = use_regex
            || query
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let tail_word = use_regex
            || query
                .chars()
                .last()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
        let prefix = if lead_word { r"\b" } else { "" };
        let suffix = if tail_word { r"\b" } else { "" };
        format!("{prefix}(?:{core}){suffix}")
    } else {
        core
    };
    RegexBuilder::new(&pattern)
        .case_insensitive(!case_sensitive)
        .multi_line(true)
        .build()
        .map_err(|e| Error::Invalid(format!("invalid search pattern: {e}")))
}

enum CompiledMask {
    Any,
    Ext(String),
    Regex { full_path: bool, re: Regex },
    Never,
}

impl CompiledMask {
    fn compile(mask: &str) -> Self {
        if mask == "*" || mask == "**" || mask == "**/*" {
            return Self::Any;
        }
        if let Some(ext) = mask.strip_prefix("*.") {
            if !ext.contains('*') && !ext.contains('/') && !ext.contains('?') {
                return Self::Ext(ext.to_ascii_lowercase());
            }
        }
        let full_path = mask.contains('/');
        let mut rx = String::from("^");
        let chars: Vec<char> = mask.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            if chars[i] == '*' && i + 1 < chars.len() && chars[i + 1] == '*' {
                rx.push_str(".*");
                i += 2;
                if i < chars.len() && chars[i] == '/' {
                    i += 1;
                }
            } else if chars[i] == '*' {
                rx.push_str("[^/]*");
                i += 1;
            } else if chars[i] == '?' {
                rx.push('.');
                i += 1;
            } else {
                rx.push_str(&regex::escape(&chars[i].to_string()));
                i += 1;
            }
        }
        rx.push('$');
        match RegexBuilder::new(&rx).case_insensitive(true).build() {
            Ok(re) => Self::Regex { full_path, re },
            Err(_) => Self::Never,
        }
    }

    fn matches(&self, rel_path: &str) -> bool {
        let base = rel_path.rsplit('/').next().unwrap_or(rel_path);
        match self {
            Self::Any => true,
            Self::Ext(ext) => base
                .rsplit_once('.')
                .is_some_and(|(_, e)| e.eq_ignore_ascii_case(ext)),
            Self::Regex { full_path, re } => {
                let target = if *full_path { rel_path } else { base };
                re.is_match(target)
            }
            Self::Never => false,
        }
    }
}

pub(super) struct CompiledFileGlob {
    positives: Vec<CompiledMask>,
    negatives: Vec<CompiledMask>,
}

impl CompiledFileGlob {
    pub(super) fn compile(file_glob: Option<&str>) -> Self {
        let Some(glob) = file_glob.map(str::trim).filter(|g| !g.is_empty()) else {
            return Self {
                positives: Vec::new(),
                negatives: Vec::new(),
            };
        };
        let mut positives = Vec::new();
        let mut negatives = Vec::new();
        for raw_part in glob
            .split([',', ';'])
            .map(str::trim)
            .filter(|p| !p.is_empty())
        {
            if let Some(neg) = raw_part.strip_prefix('!').map(str::trim) {
                if !neg.is_empty() {
                    negatives.push(CompiledMask::compile(neg));
                }
            } else {
                positives.push(CompiledMask::compile(raw_part));
            }
        }
        Self {
            positives,
            negatives,
        }
    }

    pub(super) fn matches(&self, rel_path: &str) -> bool {
        if self.negatives.iter().any(|m| m.matches(rel_path)) {
            return false;
        }
        if self.positives.is_empty() {
            return true;
        }
        self.positives.iter().any(|m| m.matches(rel_path))
    }
}

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub(super) fn collect_local_files_multi(
    root: &Path,
    additional_roots: &[PathBuf],
    restricted: bool,
    scope_path: Option<&str>,
    cap: usize,
) -> Result<Vec<String>> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();

    if let Some(sp) = scope_path.map(str::trim).filter(|s| !s.is_empty()) {
        let (canon_scope, matched_root) =
            resolve_local_existing_in_roots(root, additional_roots, !restricted, sp)?;
        if canon_scope.is_file() {
            return Ok(vec![canon_scope.to_string_lossy().into_owned()]);
        }
        walk_one_root(
            &canon_scope,
            &matched_root,
            restricted,
            cap,
            &mut seen,
            &mut out,
        );
        return Ok(out);
    }

    for r in std::iter::once(root).chain(additional_roots.iter().map(PathBuf::as_path)) {
        if out.len() >= cap {
            break;
        }
        walk_one_root(r, r, restricted, cap, &mut seen, &mut out);
    }
    Ok(out)
}

#[allow(clippy::disallowed_methods)]
fn walk_one_root(
    scan_dir: &Path,
    owning_root: &Path,
    restricted: bool,
    cap: usize,
    seen: &mut HashSet<PathBuf>,
    out: &mut Vec<String>,
) {
    let walker = ignore::WalkBuilder::new(scan_dir)
        .hidden(false)
        .git_ignore(true)
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            !matches!(
                name.as_ref(),
                ".git"
                    | "node_modules"
                    | "target"
                    | "dist"
                    | "build"
                    | ".svelte-kit"
                    | "__pycache__"
                    | ".venv"
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
            if !canon.starts_with(owning_root) || enforce_non_root_path(&canon).is_err() {
                continue;
            }
            if !seen.insert(canon.clone()) {
                continue;
            }
            out.push(canon.to_string_lossy().into_owned());
        } else {
            let key = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
            if !seen.insert(key) {
                continue;
            }
            out.push(p.to_string_lossy().into_owned());
        }
    }
}

fn display_rel_path(p: &Path, root: &Path, additional_roots: &[PathBuf]) -> String {
    if let Ok(r) = p.strip_prefix(root) {
        return r.to_string_lossy().replace('\\', "/");
    }
    for extra in additional_roots {
        if let Ok(r) = p.strip_prefix(extra) {
            let prefix = extra
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("attached");
            return format!("{prefix}/{}", r.to_string_lossy().replace('\\', "/"));
        }
    }
    p.to_string_lossy().into_owned()
}

#[allow(clippy::disallowed_methods)] // runs inside spawn_blocking
pub(super) fn search_local_sync(
    root: &Path,
    additional_roots: &[PathBuf],
    restricted: bool,
    req: &IdeSearchReq,
    limit: usize,
) -> Result<IdeSearchResp> {
    let files = collect_local_files_multi(
        root,
        additional_roots,
        restricted,
        req.scope_path.as_deref(),
        15_000,
    )?;
    let matcher = build_matcher(&req.query, req.regex, req.case_sensitive, req.whole_word)?;
    let compiled_glob = CompiledFileGlob::compile(req.file_glob.as_deref());
    let q_lower = req.query.to_ascii_lowercase();
    let want_files = matches!(req.mode, IdeSearchMode::Files | IdeSearchMode::All);
    let want_symbols = matches!(req.mode, IdeSearchMode::Symbols | IdeSearchMode::All);
    let want_text = matches!(req.mode, IdeSearchMode::Text | IdeSearchMode::All);

    let mut items = Vec::new();
    let mut truncated = false;

    for full_path in &files {
        let p = Path::new(full_path);
        let rel = display_rel_path(p, root, additional_roots);
        if !compiled_glob.matches(&rel) {
            continue;
        }

        if want_files {
            let matched = if req.regex || req.whole_word {
                matcher.is_match(&rel)
            } else if req.case_sensitive {
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
    static GO_METHOD_RE: OnceLock<Regex> = OnceLock::new();
    static SHELL_FN_RE: OnceLock<Regex> = OnceLock::new();

    let sym_re = SYM_RE.get_or_init(|| {
        Regex::new(
            r"^\s*(?:pub(?:\([^)]*\))?\s+|public\s+|private\s+|protected\s+|export\s+|default\s+|async\s+|static\s+|final\s+|abstract\s+)*(?P<kw>fn|func|function|def|class|struct|enum|trait|interface|type|record|const)\s+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
        )
        .unwrap()
    });
    let go_method_re = GO_METHOD_RE.get_or_init(|| {
        Regex::new(r"^\s*func\s*\([^)]+\)\s*(?P<name>[A-Za-z_][A-Za-z0-9_]*)\s*\(").unwrap()
    });
    let is_shell =
        full_path.ends_with(".sh") || full_path.ends_with(".bash") || full_path.ends_with(".zsh");
    let shell_fn_re = SHELL_FN_RE.get_or_init(|| {
        Regex::new(r"^\s*(?:function\s+)?(?P<name>[A-Za-z_][A-Za-z0-9_-]*)\s*\(\s*\)").unwrap()
    });

    for (idx, line) in text.lines().enumerate() {
        let found: Option<(&str, &str)> = if let Some(cap) = sym_re.captures(line) {
            Some((
                cap.name("kw").map_or("symbol", |m| m.as_str()),
                cap.name("name").map_or("", |m| m.as_str()),
            ))
        } else if let Some(cap) = go_method_re.captures(line) {
            Some(("method", cap.name("name").map_or("", |m| m.as_str())))
        } else if is_shell {
            shell_fn_re
                .captures(line)
                .map(|cap| ("function", cap.name("name").map_or("", |m| m.as_str())))
        } else {
            None
        };

        let Some((kw, sym_name)) = found else {
            continue;
        };
        if sym_name.is_empty() || !matcher.is_match(sym_name) {
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
            symbol_kind: Some(kw.to_string()),
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
    let q_lower = req.query.to_ascii_lowercase();
    let compiled_glob = CompiledFileGlob::compile(req.file_glob.as_deref());
    let search_roots: Vec<String> = match req
        .scope_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
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
                rel.to_ascii_lowercase().contains(&q_lower)
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
                   rg --line-number --column --no-heading --color=never {case_flag} {fixed_flag} {word_flag} -- {q_escaped} . | head -n {cap}; \
                 else \
                   grep -rnI {} {} -- {q_escaped} . | head -n {cap}; \
                 fi",
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
            let existing_perms = meta.permissions();
            let tmp = p.with_extension("otto-replace-tmp");
            std::fs::write(&tmp, replaced_text.as_bytes())
                .map_err(|e| map_io_err("write replace temp", &tmp, e))?;
            let _ = std::fs::set_permissions(&tmp, existing_perms);
            if let Err(e) = std::fs::rename(&tmp, p) {
                let _ = std::fs::remove_file(&tmp);
                return Err(map_io_err("commit replace", p, e));
            }
            if let Some(pool) = lsp_pool {
                pool.notify_watched_file_if_build_manifest(p, 2);
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
