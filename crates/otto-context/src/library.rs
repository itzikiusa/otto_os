//! The Otto library store on disk: skills, souls, and reusable context
//! snippets, plus the global-default-soul pointer.
//!
//! Layout under `root`:
//! - `skills/<name>/SKILL.md`
//! - `souls/<name>.md`
//! - `context/<name>.md`
//! - `default-soul.txt`  (single line, the default soul name)
//!
//! Entry names are validated as safe single segments (alphanumeric / `-` / `_`,
//! non-empty, not `.` or `..`), mirroring `otto-improve::pathsafe`, to prevent
//! path traversal into (or out of) the library tree.
//!
//! ## Skill cache
//!
//! `list_skills` and `get_skill` parse the YAML frontmatter (description,
//! category, version) on every call. With many skills or frequent spawns this
//! adds up. We keep an in-process `Arc<Mutex<HashMap<name, LibrarySkill>>>` that
//! is populated on first parse and invalidated (evicted) when a skill is written
//! or deleted. Reads hold the lock only for the map lookup; the actual
//! `fs::read_to_string` + parse happen outside the lock, followed by a brief
//! re-acquire to insert.  The cache is entirely best-effort: a poisoned mutex
//! falls back to the direct-disk path.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use otto_core::api::{LibraryContext, LibrarySkill, LibrarySoul};

/// In-process skill cache: keyed by skill name, holds the last-read
/// `LibrarySkill`. Wrapped in `Arc<Mutex<…>>` so `Library::clone` shares the
/// same cache across all handle copies (e.g., the Axum state clone).
type SkillCache = Arc<Mutex<HashMap<String, LibrarySkill>>>;

/// Handle to the on-disk library rooted at `root`.
#[derive(Clone)]
pub struct Library {
    pub root: PathBuf,
    /// Shared across clones — invalidated on writes/deletes.
    skill_cache: SkillCache,
}

/// An entry name must be a single safe path segment.
fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Parse a single scalar `<key>:` value out of a YAML frontmatter block, if
/// present. Only looks inside a leading `---` / `---` fenced block, takes the
/// first matching key, and strips surrounding quotes. Returns `None` when there
/// is no frontmatter or the key is absent.
fn parse_frontmatter(body: &str, key: &str) -> Option<String> {
    let mut lines = body.lines();
    if lines.next().map(str::trim) != Some("---") {
        return None;
    }
    let prefix = format!("{key}:");
    for line in lines {
        let trimmed = line.trim();
        if trimmed == "---" {
            return None;
        }
        if let Some(rest) = trimmed.strip_prefix(&prefix) {
            let val = rest.trim();
            let val = val
                .strip_prefix('"')
                .and_then(|v| v.strip_suffix('"'))
                .or_else(|| val.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
                .unwrap_or(val);
            return Some(val.to_string());
        }
    }
    None
}

/// Parse the `description:` value from frontmatter; `""` when absent.
fn parse_description(body: &str) -> String {
    parse_frontmatter(body, "description").unwrap_or_default()
}

/// Parse the `category:` value from frontmatter; `""` when absent.
fn parse_category(body: &str) -> String {
    parse_frontmatter(body, "category").unwrap_or_default()
}

/// Parse the `version:` value from frontmatter; defaults to `1` when absent or
/// unparseable.
fn parse_version(body: &str) -> u32 {
    parse_frontmatter(body, "version")
        .and_then(|v| v.parse().ok())
        .unwrap_or(1)
}

impl Library {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), skill_cache: Arc::new(Mutex::new(HashMap::new())) }
    }

    // -- skills --------------------------------------------------------------

    fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    /// Absolute path of a skill file, regardless of whether it exists. Returns
    /// `None` for unsafe names. Used by the self-improvement repoint to target
    /// the library copy of a skill.
    pub fn skill_path(&self, name: &str) -> Option<PathBuf> {
        if !is_safe_segment(name) {
            return None;
        }
        Some(self.skills_dir().join(name).join("SKILL.md"))
    }

    pub fn list_skills(&self) -> Vec<LibrarySkill> {
        let mut out = Vec::new();
        let entries = match fs::read_dir(self.skills_dir()) {
            Ok(e) => e,
            Err(_) => return out,
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !is_safe_segment(&name) {
                continue;
            }
            if let Some(skill) = self.get_skill(&name) {
                out.push(skill);
            }
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    pub fn get_skill(&self, name: &str) -> Option<LibrarySkill> {
        let path = self.skill_path(name)?;

        // Cache hit: return without touching disk.
        if let Ok(cache) = self.skill_cache.lock() {
            if let Some(cached) = cache.get(name) {
                return Some(cached.clone());
            }
        }

        // Cache miss: read and parse outside the lock.
        let body = fs::read_to_string(&path).ok()?;
        let description = parse_description(&body);
        let category = parse_category(&body);
        let version = parse_version(&body);
        let skill = LibrarySkill {
            name: name.to_string(),
            category,
            version,
            description,
            body,
        };

        // Insert into cache (best-effort — a poisoned mutex is ignored).
        if let Ok(mut cache) = self.skill_cache.lock() {
            cache.insert(name.to_string(), skill.clone());
        }
        Some(skill)
    }

    pub fn put_skill(&self, name: &str, body: &str) -> io::Result<()> {
        let path = self
            .skill_path(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, body)?;
        // Evict the stale cached entry so the next read picks up the new content.
        if let Ok(mut cache) = self.skill_cache.lock() {
            cache.remove(name);
        }
        Ok(())
    }

    pub fn delete_skill(&self, name: &str) -> io::Result<()> {
        if !is_safe_segment(name) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"));
        }
        let dir = self.skills_dir().join(name);
        match fs::remove_dir_all(&dir) {
            Ok(()) => {
                if let Ok(mut cache) = self.skill_cache.lock() {
                    cache.remove(name);
                }
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }

    // -- souls ---------------------------------------------------------------

    fn souls_dir(&self) -> PathBuf {
        self.root.join("souls")
    }

    pub fn list_souls(&self) -> Vec<LibrarySoul> {
        list_md_entries(&self.souls_dir())
            .into_iter()
            .map(|(name, body)| LibrarySoul { name, body })
            .collect()
    }

    pub fn get_soul(&self, name: &str) -> Option<LibrarySoul> {
        if !is_safe_segment(name) {
            return None;
        }
        let body = fs::read_to_string(self.souls_dir().join(format!("{name}.md"))).ok()?;
        Some(LibrarySoul { name: name.to_string(), body })
    }

    pub fn put_soul(&self, name: &str, body: &str) -> io::Result<()> {
        write_md_entry(&self.souls_dir(), name, body)
    }

    pub fn delete_soul(&self, name: &str) -> io::Result<()> {
        delete_md_entry(&self.souls_dir(), name)
    }

    // -- context -------------------------------------------------------------

    fn context_dir(&self) -> PathBuf {
        self.root.join("context")
    }

    pub fn list_context(&self) -> Vec<LibraryContext> {
        list_md_entries(&self.context_dir())
            .into_iter()
            .map(|(name, body)| LibraryContext { name, body })
            .collect()
    }

    pub fn get_context(&self, name: &str) -> Option<LibraryContext> {
        if !is_safe_segment(name) {
            return None;
        }
        let body = fs::read_to_string(self.context_dir().join(format!("{name}.md"))).ok()?;
        Some(LibraryContext { name: name.to_string(), body })
    }

    pub fn put_context(&self, name: &str, body: &str) -> io::Result<()> {
        write_md_entry(&self.context_dir(), name, body)
    }

    pub fn delete_context(&self, name: &str) -> io::Result<()> {
        delete_md_entry(&self.context_dir(), name)
    }

    // -- default soul --------------------------------------------------------

    fn default_soul_path(&self) -> PathBuf {
        self.root.join("default-soul.txt")
    }

    /// The configured global default soul name, or `None` when unset/empty.
    pub fn default_soul(&self) -> Option<String> {
        let raw = fs::read_to_string(self.default_soul_path()).ok()?;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    }

    pub fn set_default_soul(&self, name: &str) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        fs::write(self.default_soul_path(), name.trim())
    }
}

/// List `<dir>/<name>.md` entries as `(name, body)`, sorted by name. Unsafe or
/// non-`.md` files are skipped.
fn list_md_entries(dir: &std::path::Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        let file = entry.file_name().to_string_lossy().into_owned();
        let Some(name) = file.strip_suffix(".md") else {
            continue;
        };
        if !is_safe_segment(name) {
            continue;
        }
        if let Ok(body) = fs::read_to_string(entry.path()) {
            out.push((name.to_string(), body));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn write_md_entry(dir: &std::path::Path, name: &str, body: &str) -> io::Result<()> {
    if !is_safe_segment(name) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe entry name"));
    }
    fs::create_dir_all(dir)?;
    fs::write(dir.join(format!("{name}.md")), body)
}

fn delete_md_entry(dir: &std::path::Path, name: &str) -> io::Result<()> {
    if !is_safe_segment(name) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe entry name"));
    }
    match fs::remove_file(dir.join(format!("{name}.md"))) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn lib() -> (TempDir, Library) {
        let dir = TempDir::new().unwrap();
        let lib = Library::new(dir.path());
        (dir, lib)
    }

    #[test]
    fn skill_round_trip_and_description() {
        let (_d, lib) = lib();
        let body = "---\ndescription: Triage support tickets\ncategory: review\nversion: 4\n---\n# body\n";
        lib.put_skill("support-triage", body).unwrap();

        let got = lib.get_skill("support-triage").unwrap();
        assert_eq!(got.name, "support-triage");
        assert_eq!(got.description, "Triage support tickets");
        assert_eq!(got.category, "review");
        assert_eq!(got.version, 4);
        assert_eq!(got.body, body);

        let listed = lib.list_skills();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].name, "support-triage");

        lib.delete_skill("support-triage").unwrap();
        assert!(lib.get_skill("support-triage").is_none());
        assert!(lib.list_skills().is_empty());
    }

    #[test]
    fn skill_without_frontmatter_has_empty_description() {
        let (_d, lib) = lib();
        lib.put_skill("plain", "# just markdown\n").unwrap();
        let got = lib.get_skill("plain").unwrap();
        assert_eq!(got.description, "");
        assert_eq!(got.category, "");
        assert_eq!(got.version, 1);
    }

    #[test]
    fn skill_path_is_independent_of_existence() {
        let (_d, lib) = lib();
        let p = lib.skill_path("ghost").unwrap();
        assert!(p.ends_with("skills/ghost/SKILL.md"));
        assert!(lib.skill_path("../x").is_none());
        assert!(lib.skill_path("a/b").is_none());
    }

    #[test]
    fn soul_round_trip() {
        let (_d, lib) = lib();
        lib.put_soul("otto", "Be terse.").unwrap();
        assert_eq!(lib.get_soul("otto").unwrap().body, "Be terse.");
        assert_eq!(lib.list_souls().len(), 1);
        lib.delete_soul("otto").unwrap();
        assert!(lib.get_soul("otto").is_none());
    }

    #[test]
    fn context_round_trip() {
        let (_d, lib) = lib();
        lib.put_context("house-rules", "No emojis.").unwrap();
        assert_eq!(lib.get_context("house-rules").unwrap().body, "No emojis.");
        assert_eq!(lib.list_context().len(), 1);
        lib.delete_context("house-rules").unwrap();
        assert!(lib.get_context("house-rules").is_none());
    }

    #[test]
    fn default_soul_file() {
        let (_d, lib) = lib();
        assert!(lib.default_soul().is_none());
        lib.set_default_soul("  otto  ").unwrap();
        assert_eq!(lib.default_soul().as_deref(), Some("otto"));
        lib.set_default_soul("").unwrap();
        assert!(lib.default_soul().is_none());
    }

    #[test]
    fn unsafe_names_are_rejected() {
        let (_d, lib) = lib();
        assert!(lib.put_skill("../x", "b").is_err());
        assert!(lib.put_skill("a/b", "b").is_err());
        assert!(lib.put_soul("..", "b").is_err());
        assert!(lib.put_context("", "b").is_err());
        assert!(lib.get_skill("../x").is_none());
        assert!(lib.get_soul("a/b").is_none());
        assert!(lib.delete_skill("../x").is_err());
    }
}
