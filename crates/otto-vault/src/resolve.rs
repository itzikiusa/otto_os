//! Link-target resolution with Obsidian's "shortest path" semantics plus
//! OKF's `/`-bundle-absolute form:
//!
//! 1. `/abs/from/root.md` — vault-root-relative (leading `/`).
//! 2. `rel/to/src.md` or `../up.md` — relative to the linking note's folder.
//! 3. `root/relative.md` — relative to the vault root (Obsidian's canonical
//!    wikilink form is the full vault path without extension).
//! 4. `Basename` — unique basename anywhere in the vault (case-insensitive,
//!    `.md` optional). Ambiguous basenames do NOT resolve (surfaced as
//!    unresolved rather than silently picking one).

use std::collections::HashMap;

/// A snapshot of every addressable file in a vault (notes + attachments),
/// built once per scan / rename pass.
#[derive(Default)]
pub struct ResolveIndex {
    /// lowercased rel path → actual rel path
    by_path: HashMap<String, String>,
    /// lowercased basename (with extension) → actual rel paths
    by_basename: HashMap<String, Vec<String>>,
}

impl ResolveIndex {
    pub fn new(paths: impl IntoIterator<Item = String>) -> Self {
        let mut ix = Self::default();
        for p in paths {
            ix.insert(p);
        }
        ix
    }

    pub fn insert(&mut self, path: String) {
        let lower = path.to_lowercase();
        if let Some(base) = lower.rsplit('/').next() {
            self.by_basename.entry(base.to_string()).or_default().push(path.clone());
        }
        self.by_path.insert(lower, path);
    }

    fn lookup_path(&self, rel: &str) -> Option<String> {
        self.by_path.get(&rel.to_lowercase()).cloned()
    }

    /// Unique-basename lookup; ambiguous → None.
    fn lookup_basename(&self, base: &str) -> Option<String> {
        match self.by_basename.get(&base.to_lowercase()) {
            Some(v) if v.len() == 1 => Some(v[0].clone()),
            _ => None,
        }
    }

    /// Resolve `raw` as linked from `src_path` (a vault-relative note path).
    pub fn resolve(&self, src_path: &str, raw: &str) -> Option<String> {
        let raw = raw.trim();
        if raw.is_empty() {
            return None;
        }
        let has_ext = raw.rsplit('/').next().is_some_and(|b| b.contains('.'));
        let candidates: &[String] = &if has_ext {
            vec![raw.to_string()]
        } else {
            vec![format!("{raw}.md"), raw.to_string()]
        };

        for cand in candidates {
            // 1) `/`-absolute → vault-root-relative.
            if let Some(stripped) = cand.strip_prefix('/') {
                if let Some(hit) = self.lookup_path(stripped) {
                    return Some(hit);
                }
                continue;
            }
            // 2) Relative to the source note's folder.
            let src_dir = src_path.rsplit_once('/').map(|(d, _)| d).unwrap_or("");
            if let Some(joined) = join_normalize(src_dir, cand) {
                if let Some(hit) = self.lookup_path(&joined) {
                    return Some(hit);
                }
            }
            // 3) Vault-root-relative (Obsidian canonical wikilink).
            if let Some(hit) = self.lookup_path(cand) {
                return Some(hit);
            }
            // 4) Unique basename anywhere.
            if !cand.contains('/') {
                if let Some(hit) = self.lookup_basename(cand) {
                    return Some(hit);
                }
            }
        }
        None
    }
}

/// Join `dir` + `rel`, normalizing `.` / `..` — returns None if it escapes the
/// vault root.
pub fn join_normalize(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = if dir.is_empty() { Vec::new() } else { dir.split('/').collect() };
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ix() -> ResolveIndex {
        ResolveIndex::new(
            [
                "index.md",
                "services/auth-api.md",
                "services/orders-api.md",
                "tables/Customers.md",
                "notes/deep/Unique Note.md",
                "a/dup.md",
                "b/dup.md",
                "assets/img.png",
            ]
            .into_iter()
            .map(String::from),
        )
    }

    #[test]
    fn absolute_form() {
        assert_eq!(
            ix().resolve("services/auth-api.md", "/tables/customers.md"),
            Some("tables/Customers.md".into())
        );
    }

    #[test]
    fn relative_form() {
        assert_eq!(
            ix().resolve("services/auth-api.md", "orders-api.md"),
            Some("services/orders-api.md".into())
        );
        assert_eq!(
            ix().resolve("services/auth-api.md", "../tables/Customers.md"),
            Some("tables/Customers.md".into())
        );
        assert_eq!(ix().resolve("services/auth-api.md", "../../escape.md"), None);
    }

    #[test]
    fn root_relative_wikilink_without_ext() {
        assert_eq!(
            ix().resolve("index.md", "services/auth-api"),
            Some("services/auth-api.md".into())
        );
    }

    #[test]
    fn unique_basename_case_insensitive() {
        assert_eq!(
            ix().resolve("index.md", "unique note"),
            Some("notes/deep/Unique Note.md".into())
        );
        assert_eq!(ix().resolve("index.md", "customers"), Some("tables/Customers.md".into()));
    }

    #[test]
    fn ambiguous_basename_unresolved() {
        assert_eq!(ix().resolve("index.md", "dup"), None);
    }

    #[test]
    fn attachment_with_extension() {
        assert_eq!(ix().resolve("index.md", "img.png"), Some("assets/img.png".into()));
    }
}
