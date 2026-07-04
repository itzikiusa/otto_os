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

use otto_core::api::{LibraryContext, LibrarySkill, LibrarySoul, SkillFileEntry};

/// Largest single skill file read/written through the editor (2 MiB).
const MAX_SKILL_FILE_BYTES: usize = 2 * 1024 * 1024;
/// Largest total uncompressed size accepted from a skill-package import (16 MiB).
const MAX_SKILL_IMPORT_BYTES: u64 = 16 * 1024 * 1024;

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

/// Validate a relative path *within* a skill dir: non-empty, no absolute root,
/// no `.`/`..` segments, and each segment restricted to safe filename chars
/// (alphanumeric plus `-`, `_`, `.`). Returns the assembled relative `PathBuf`,
/// or `None` when the path is unsafe. Unlike [`is_safe_segment`], this permits
/// the `/` separators of nested paths like `references/rubric.md`.
fn safe_rel(rel: &str) -> Option<PathBuf> {
    let rel = rel.trim();
    if rel.is_empty() || rel.starts_with('/') {
        return None;
    }
    // Unix-style separators only; a backslash is treated as an (illegal) segment
    // char, so Windows-style input is rejected here — the zip importer normalizes
    // `\`→`/` before calling this.
    let mut out = PathBuf::new();
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return None;
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return None;
        }
        out.push(seg);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Best-effort binary sniff: a NUL byte in the leading window ⇒ binary.
fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|&b| b == 0)
}

/// Recursively collect files under `root` as [`SkillFileEntry`] with paths
/// relative to `base`. Symlinks and unreadable entries are skipped.
fn collect_files(base: &std::path::Path, dir: &std::path::Path, out: &mut Vec<SkillFileEntry>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_dir() {
            collect_files(base, &path, out);
        } else if ft.is_file() {
            let Ok(rel) = path.strip_prefix(base) else { continue };
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            // Sniff only a small head for the binary flag.
            let binary = {
                let mut head = [0u8; 512];
                fs::File::open(&path)
                    .and_then(|mut f| std::io::Read::read(&mut f, &mut head))
                    .map(|n| is_binary(&head[..n]))
                    .unwrap_or(false)
            };
            out.push(SkillFileEntry {
                path: rel.to_string_lossy().replace('\\', "/"),
                size,
                binary,
            });
        }
    }
}

/// A minimal starter `SKILL.md` for a newly created library skill.
fn starter_skill_md(name: &str, category: &str, description: &str) -> String {
    let category = if category.trim().is_empty() {
        "development"
    } else {
        category.trim()
    };
    let description = if description.trim().is_empty() {
        format!("What {name} does and when to use it.")
    } else {
        description.trim().to_string()
    };
    format!(
        "---\ndescription: {description}\ncategory: {category}\nversion: 1\n---\n\n# {name}\n\nDescribe the method here.\n"
    )
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

    // -- skill files (multi-file editor) -------------------------------------

    /// Absolute path of a skill's root dir in the library, or `None` for an
    /// unsafe name. The dir need not exist.
    pub fn skill_dir(&self, name: &str) -> Option<PathBuf> {
        if !is_safe_segment(name) {
            return None;
        }
        Some(self.skills_dir().join(name))
    }

    /// List every file inside a library skill (recursively), relative to the
    /// skill dir, sorted by path. Empty when the skill does not exist.
    pub fn list_skill_files(&self, name: &str) -> Vec<SkillFileEntry> {
        let Some(root) = self.skill_dir(name) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        collect_files(&root, &root, &mut out);
        out.sort_by(|a, b| a.path.cmp(&b.path));
        out
    }

    /// Read one file inside a library skill. Returns `(content, binary)`; binary
    /// files are returned lossily so the UI can display them read-only. Reads at
    /// most `MAX_SKILL_FILE_BYTES`.
    pub fn read_skill_file(&self, name: &str, rel: &str) -> Option<(String, bool)> {
        let root = self.skill_dir(name)?;
        let target = root.join(safe_rel(rel)?);
        let bytes = fs::read(&target).ok()?;
        let bytes = &bytes[..bytes.len().min(MAX_SKILL_FILE_BYTES)];
        let binary = is_binary(bytes);
        Some((String::from_utf8_lossy(bytes).into_owned(), binary))
    }

    /// Create or overwrite one file inside an existing library skill. Rejects
    /// unsafe paths and content over the size cap. Creates intermediate dirs and
    /// evicts the skill cache (in case `SKILL.md` changed).
    pub fn write_skill_file(&self, name: &str, rel: &str, content: &str) -> io::Result<()> {
        let root = self
            .skill_dir(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        let rel_path =
            safe_rel(rel).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe path"))?;
        if content.len() > MAX_SKILL_FILE_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "file too large"));
        }
        let target = root.join(rel_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(target, content)?;
        self.evict(name);
        Ok(())
    }

    /// Delete one file inside a library skill. `SKILL.md` cannot be deleted (use
    /// `delete_skill` to remove the whole skill). Missing file is a no-op.
    pub fn delete_skill_file(&self, name: &str, rel: &str) -> io::Result<()> {
        let root = self
            .skill_dir(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        let rel_path =
            safe_rel(rel).ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe path"))?;
        if rel_path.as_os_str() == "SKILL.md" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SKILL.md cannot be deleted",
            ));
        }
        match fs::remove_file(root.join(rel_path)) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        self.evict(name);
        Ok(())
    }

    /// Create a new library skill with a synthesized (or provided) `SKILL.md`.
    /// Fails with `AlreadyExists` if the skill dir already exists.
    pub fn create_skill(
        &self,
        name: &str,
        category: &str,
        description: &str,
        body: Option<&str>,
    ) -> io::Result<()> {
        let dir = self
            .skill_dir(name)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"))?;
        if dir.join("SKILL.md").exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "skill already exists",
            ));
        }
        let content = match body {
            Some(b) if !b.trim().is_empty() => b.to_string(),
            _ => starter_skill_md(name, category, description),
        };
        self.put_skill(name, &content)
    }

    /// Import a skill package from an in-memory zip. The archive must contain a
    /// `SKILL.md`; the skill name is `name_override` or the wrapping directory
    /// name. Zip-slip and size limits are enforced. Returns the skill name.
    pub fn import_zip(&self, bytes: &[u8], name_override: Option<&str>) -> io::Result<String> {
        use std::io::Cursor;
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes))
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("bad zip: {e}")))?;

        // Locate the top-most SKILL.md to find the package root inside the zip.
        let mut best: Option<(String, usize)> = None; // (prefix, depth)
        for i in 0..zip.len() {
            let f = zip
                .by_index(i)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            let Some(name) = f.enclosed_name() else { continue };
            if name.file_name().and_then(|s| s.to_str()) == Some("SKILL.md") {
                let prefix = name
                    .parent()
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                let depth = name.components().count();
                if best.as_ref().map(|(_, d)| depth < *d).unwrap_or(true) {
                    best = Some((prefix, depth));
                }
            }
        }
        let (prefix, _) =
            best.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no SKILL.md in zip"))?;

        // Resolve the skill name: explicit override, else the wrapper dir name.
        let derived = prefix.rsplit('/').next().filter(|s| !s.is_empty());
        let name = name_override
            .filter(|s| !s.is_empty())
            .or(derived)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "cannot derive skill name"))?
            .to_string();
        if !is_safe_segment(&name) {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "unsafe skill name"));
        }
        let dest_root = self.skills_dir().join(&name);
        // Strip the wrapping prefix so entries land at the skill root.
        let strip = if prefix.is_empty() {
            String::new()
        } else {
            format!("{prefix}/")
        };

        let mut total: u64 = 0;
        for i in 0..zip.len() {
            let mut f = zip
                .by_index(i)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            if f.is_dir() {
                continue;
            }
            let Some(entry) = f.enclosed_name() else {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "unsafe zip entry"));
            };
            let entry = entry.to_string_lossy().replace('\\', "/");
            let Some(rel) = entry.strip_prefix(&strip) else {
                continue; // outside the package root
            };
            let Some(rel_path) = safe_rel(rel) else {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "unsafe zip entry"));
            };
            total += f.size();
            if total > MAX_SKILL_IMPORT_BYTES {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "zip too large"));
            }
            let out = dest_root.join(rel_path);
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut f, &mut buf)?;
            fs::write(&out, &buf)?;
            #[cfg(unix)]
            if out.extension().and_then(|e| e.to_str()) == Some("sh") {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&out, fs::Permissions::from_mode(0o755));
            }
        }
        self.evict(&name);
        Ok(name)
    }

    /// Evict a skill's cached metadata (best-effort).
    fn evict(&self, name: &str) {
        if let Ok(mut cache) = self.skill_cache.lock() {
            cache.remove(name);
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

    #[test]
    fn safe_rel_accepts_nested_and_rejects_traversal() {
        assert!(safe_rel("SKILL.md").is_some());
        assert!(safe_rel("references/rubric.md").is_some());
        assert!(safe_rel("scripts/a/b/c.sh").is_some());
        // Rejected:
        assert!(safe_rel("").is_none());
        assert!(safe_rel("/etc/passwd").is_none());
        assert!(safe_rel("../secret").is_none());
        assert!(safe_rel("a/../../b").is_none());
        assert!(safe_rel("a/./b").is_none());
        assert!(safe_rel("a//b").is_none());
        assert!(safe_rel("a\\b").is_none());
        assert!(safe_rel("references/..").is_none());
    }

    #[test]
    fn skill_file_crud_round_trip_and_cache_evict() {
        let (_d, lib) = lib();
        lib.create_skill("editable", "review", "Reviews stuff", None).unwrap();
        // Starter SKILL.md is well-formed and cached.
        let got = lib.get_skill("editable").unwrap();
        assert_eq!(got.category, "review");
        assert_eq!(got.description, "Reviews stuff");

        // Add a reference file, then it shows up in the tree.
        lib.write_skill_file("editable", "references/notes.md", "# notes").unwrap();
        let files: Vec<String> = lib.list_skill_files("editable").into_iter().map(|f| f.path).collect();
        assert!(files.contains(&"SKILL.md".to_string()));
        assert!(files.contains(&"references/notes.md".to_string()));
        assert_eq!(lib.read_skill_file("editable", "references/notes.md").unwrap().0, "# notes");

        // Overwriting SKILL.md evicts the cache so metadata re-parses.
        lib.write_skill_file("editable", "SKILL.md", "---\ndescription: New desc\ncategory: design\nversion: 2\n---\n# x").unwrap();
        let got2 = lib.get_skill("editable").unwrap();
        assert_eq!(got2.description, "New desc");
        assert_eq!(got2.category, "design");
        assert_eq!(got2.version, 2);

        // SKILL.md cannot be deleted; other files can.
        assert!(lib.delete_skill_file("editable", "SKILL.md").is_err());
        lib.delete_skill_file("editable", "references/notes.md").unwrap();
        assert!(lib.read_skill_file("editable", "references/notes.md").is_none());

        // Unsafe paths rejected.
        assert!(lib.write_skill_file("editable", "../evil.md", "x").is_err());
        assert!(lib.read_skill_file("editable", "../../etc/passwd").is_none());
    }

    #[test]
    fn create_skill_conflicts_on_existing() {
        let (_d, lib) = lib();
        lib.create_skill("dup", "review", "d", None).unwrap();
        let err = lib.create_skill("dup", "review", "d", None).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn import_zip_strips_wrapper_and_lands_files() {
        use std::io::Write;
        let (_d, lib) = lib();
        // Build a wrapped skill package in memory.
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<()> =
                zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
            zw.start_file("my-skill/SKILL.md", opts).unwrap();
            zw.write_all(b"---\ndescription: Imported\ncategory: review\nversion: 1\n---\n# body").unwrap();
            zw.start_file("my-skill/references/x.md", opts).unwrap();
            zw.write_all(b"ref").unwrap();
            zw.finish().unwrap();
        }
        let name = lib.import_zip(&buf, None).unwrap();
        assert_eq!(name, "my-skill");
        assert_eq!(lib.get_skill("my-skill").unwrap().description, "Imported");
        assert_eq!(lib.read_skill_file("my-skill", "references/x.md").unwrap().0, "ref");
    }

    #[test]
    fn import_zip_requires_skill_md() {
        use std::io::Write;
        let (_d, lib) = lib();
        let mut buf = Vec::new();
        {
            let mut zw = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let opts: zip::write::FileOptions<()> =
                zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
            zw.start_file("nope/readme.md", opts).unwrap();
            zw.write_all(b"x").unwrap();
            zw.finish().unwrap();
        }
        assert!(lib.import_zip(&buf, None).is_err());
    }
}
