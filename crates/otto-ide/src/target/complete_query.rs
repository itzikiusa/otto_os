//! Token formatting and scoped path query resolution (`~`, `/...`, `./`, `../`,
//! workspace aliases) for [`super::complete_fs_paths`].

#![allow(clippy::disallowed_methods)] // synchronous @ completion invoked inside spawn_blocking

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::{clean_lexical, expand_tilde};

#[derive(Clone)]
pub(crate) enum TokenFormat {
    Home,
    Absolute,
    Relative { prefix: String, base: PathBuf },
    Workspace { multi_root: bool, root: PathBuf },
}

impl TokenFormat {
    pub(crate) fn render(
        &self,
        abs_path: &Path,
        is_dir: bool,
        home: Option<&Path>,
    ) -> (String, String) {
        let trail = if is_dir { "/" } else { "" };
        match self {
            Self::Home => {
                let s = shorten_home(abs_path, home);
                let tok = format!("{}{}", s.trim_end_matches('/'), trail);
                (tok.clone(), tok)
            }
            Self::Absolute => {
                let s = abs_path.to_string_lossy();
                let tok = format!("{}{}", s.trim_end_matches('/'), trail);
                (tok.clone(), tok)
            }
            Self::Relative { prefix, base } => {
                let rel = abs_path
                    .strip_prefix(base)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| abs_path.to_string_lossy().into_owned());
                let joined = if prefix.is_empty() {
                    rel
                } else {
                    format!("{}{}", prefix, rel.trim_start_matches('/'))
                };
                let tok = format!("{}{}", joined.trim_end_matches('/'), trail);
                (tok.clone(), tok)
            }
            Self::Workspace { multi_root, root } => {
                let rel = abs_path
                    .strip_prefix(root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_else(|_| shorten_home(abs_path, home));
                let base_tok = if *multi_root {
                    let rname = root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if rname.is_empty() || rel.is_empty() {
                        rel
                    } else {
                        format!("{rname}/{rel}")
                    }
                } else {
                    rel
                };
                let tok = format!("{}{}", base_tok.trim_end_matches('/'), trail);
                (tok.clone(), tok)
            }
        }
    }
}

fn shorten_home(path: &Path, home: Option<&Path>) -> String {
    if let Some(h) = home {
        if path == h {
            return "~".to_string();
        }
        if let Ok(rel) = path.strip_prefix(h) {
            return format!("~/{}", rel.to_string_lossy().replace('\\', "/"));
        }
    }
    path.to_string_lossy().into_owned()
}

pub(super) fn resolve_scoped_query(
    raw_q: &str,
    cwd: Option<&Path>,
    roots: &[PathBuf],
    aliases: &BTreeMap<String, String>,
    is_root_user: bool,
    home: Option<&Path>,
) -> Option<(PathBuf, String, TokenFormat)> {
    let trailing_slash = raw_q.ends_with('/');
    if raw_q == "~" {
        let h = home?;
        return Some((h.to_path_buf(), String::new(), TokenFormat::Home));
    }
    if let Some(rest) = raw_q.strip_prefix("~/") {
        let h = home?;
        let (dir, leaf) = split_existing_dir(&h.join(rest), h, trailing_slash);
        return Some((dir, leaf, TokenFormat::Home));
    }
    if raw_q.starts_with('/') {
        let target = clean_lexical(Path::new(raw_q));
        let root = Path::new("/");
        let (dir, leaf) = split_existing_dir(&target, root, trailing_slash);
        return Some((dir, leaf, TokenFormat::Absolute));
    }
    if raw_q == "." || raw_q == ".." || raw_q.starts_with("./") || raw_q.starts_with("../") {
        let anchor = cwd.or_else(|| roots.first().map(PathBuf::as_path))?;
        let (rel_dir_tok, leaf_part) = if raw_q == "." {
            ("./".to_string(), "")
        } else if raw_q == ".." {
            ("../".to_string(), "")
        } else if trailing_slash {
            (raw_q.to_string(), "")
        } else if let Some((d, l)) = raw_q.rsplit_once('/') {
            (format!("{d}/"), l)
        } else {
            (String::new(), raw_q)
        };
        let resolved_base = clean_lexical(&anchor.join(&rel_dir_tok));
        return Some((
            resolved_base.clone(),
            leaf_part.to_string(),
            TokenFormat::Relative {
                prefix: rel_dir_tok,
                base: resolved_base,
            },
        ));
    }

    let (first_seg, rest) = raw_q.split_once('/')?;
    let base_repo = aliases
        .get(first_seg)
        .or_else(|| {
            aliases
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(first_seg))
                .map(|(_, v)| v)
        })
        .map(|s| clean_lexical(&expand_tilde(s)))
        .or_else(|| {
            roots
                .iter()
                .find(|r| {
                    r.file_name()
                        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(first_seg))
                })
                .cloned()
        })
        .or_else(|| {
            let cand = cwd
                .or_else(|| roots.first().map(PathBuf::as_path))?
                .join(first_seg);
            cand.is_dir().then(|| clean_lexical(&cand))
        })
        .or_else(|| {
            if !is_root_user {
                return None;
            }
            let cand = home?.join(first_seg);
            cand.is_dir().then_some(cand)
        })?;

    let full = if rest.is_empty() {
        base_repo.clone()
    } else {
        base_repo.join(rest)
    };
    let (dir, leaf) = split_existing_dir(&full, &base_repo, trailing_slash || rest.is_empty());
    let fmt = if home.is_some_and(|h| dir.starts_with(h)) {
        TokenFormat::Home
    } else {
        TokenFormat::Absolute
    };
    Some((dir, leaf, fmt))
}

fn split_existing_dir(full: &Path, floor: &Path, trailing_slash: bool) -> (PathBuf, String) {
    if trailing_slash && full.is_dir() {
        return (clean_lexical(full), String::new());
    }
    if let Some(parent) = full.parent() {
        let leaf = full
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if parent.is_dir() {
            return (clean_lexical(parent), leaf);
        }
        let mut cur = parent;
        while !cur.is_dir() && cur != floor {
            let Some(p) = cur.parent() else { break };
            cur = p;
        }
        if cur.is_dir() {
            let rem = full
                .strip_prefix(cur)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or(leaf);
            return (clean_lexical(cur), rem);
        }
    }
    (clean_lexical(floor), String::new())
}
