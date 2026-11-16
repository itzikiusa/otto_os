//! Scratches: each user's scratch files, as REAL files in one folder per user
//! — `<scratches_home>/<user id>`, where `scratches_home` is
//! `$OTTO_SCRATCHES_HOME` or `~/Otto/Scratches`. The IDE opens that folder as
//! the built-in "Scratches" project in every workspace; otto-server's
//! `/scratches` routes are thin handlers over this module (the file API the
//! MCP `workbench_*` tools and the UI's project switcher use).
//!
//! Why a user-visible folder under `$HOME` and not the data dir: Otto's data
//! dir (and `~/.otto`) are protected dirs — the IDE, the file routes and the
//! agent sandbox all refuse or write-deny them — while `~/Otto/Scratches` is an
//! ordinary folder the user can also open in Finder, a terminal or another
//! editor. It is per user (never shared through a workspace): the IDE root
//! check (`lsp::ide_local_root_authorized`) admits a non-root caller's own
//! folder and refuses anyone else's, whatever workspace root contains it.
//!
//! Every call first copies the caller's Workbench docs that are still only in
//! the DB (all workspaces, live docs only) into the folder — see [`export`].

use std::path::{Component, Path, PathBuf};

use otto_core::{Error, Result};
use otto_state::{ArchivedDoc, DbPool, WorkbenchArchive};
use serde::{Deserialize, Serialize};

use crate::local_history::LocalHistoryStore;

/// Largest scratch file read or written through these routes (UTF-8 bytes).
pub const MAX_FILE_BYTES: usize = 5 * 1024 * 1024;
/// Files listed at most (the IDE tree has no such cap).
const MAX_LIST: usize = 2_000;

/// `$OTTO_SCRATCHES_HOME`, else `~/Otto/Scratches`.
pub fn default_home() -> PathBuf {
    if let Some(d) = std::env::var_os("OTTO_SCRATCHES_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(d);
    }
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join("Otto").join("Scratches")
}

/// The caller's folder (not created). A user id is a ULID or a short slug;
/// anything else is reduced to its safe characters so it stays one segment.
pub fn user_root(home: &Path, user_id: &str) -> PathBuf {
    let seg: String = user_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    home.join(if seg.is_empty() { "_" } else { &seg })
}

/// The caller's folder, created on first use, with any not-yet-copied
/// Workbench docs exported into it. Returns its canonical path.
pub async fn ensure_root(pool: &DbPool, home: &Path, user_id: &str) -> Result<PathBuf> {
    let root = user_root(home, user_id);
    let dir = root.clone();
    let root = tokio::task::spawn_blocking(move || {
        std::fs::create_dir_all(&dir)?;
        dir.canonicalize()
    })
    .await
    .map_err(|e| Error::Internal(format!("scratches folder: {e}")))?
    .map_err(|e| Error::Internal(format!("create scratches folder {}: {e}", root.display())))?;
    export(pool, user_id, &root).await?;
    Ok(root)
}

// ---------------------------------------------------------------------------
// One-time copy of the DB Workbench docs into the folder
// ---------------------------------------------------------------------------

/// Serializes exports so two first requests can't copy a doc twice.
static EXPORT_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Copy every live, unexported Workbench doc of `user` (from every workspace)
/// into `root`: folder → sub-folders, name + language → file name + extension,
/// an image doc → its picture file. Never overwrites: a clash gets ` (2)`, ` (3)`…
/// Each doc is marked with its new path, so this runs once per doc; trashed
/// docs stay in the DB archive only.
async fn export(pool: &DbPool, user_id: &str, root: &Path) -> Result<()> {
    let _guard = EXPORT_LOCK.lock().await;
    let archive = WorkbenchArchive::new(pool.clone());
    for doc in archive.unexported(user_id).await? {
        let (root, d) = (root.to_path_buf(), doc.clone());
        let rel = tokio::task::spawn_blocking(move || write_new(&root, &d))
            .await
            .map_err(|e| Error::Internal(format!("export scratch: {e}")))??;
        archive.mark_exported(&doc.id, &rel).await?;
    }
    Ok(())
}

/// Write `doc` under `root` at a free name; returns its `/`-relative path.
fn write_new(root: &Path, doc: &ArchivedDoc) -> Result<String> {
    use std::io::Write;
    let mut dir = root.to_path_buf();
    let mut rel_dir = String::new();
    for seg in doc
        .folder
        .split('/')
        .map(clean_segment)
        .filter(|s| !s.is_empty())
    {
        dir.push(&seg);
        rel_dir.push_str(&seg);
        rel_dir.push('/');
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| Error::Internal(format!("create {}: {e}", dir.display())))?;
    let (stem, ext) = file_name_parts(doc);
    for n in 1..1000 {
        let name = match (n, ext.is_empty()) {
            (1, true) => stem.clone(),
            (1, false) => format!("{stem}.{ext}"),
            (_, true) => format!("{stem} ({n})"),
            (_, false) => format!("{stem} ({n}).{ext}"),
        };
        // `create_new` asks the filesystem, so a case-only clash on APFS
        // (`Report` vs `report`) also moves on to the next name.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dir.join(&name))
        {
            Ok(mut f) => {
                f.write_all(&doc.bytes)
                    .map_err(|e| Error::Internal(format!("write {name}: {e}")))?;
                return Ok(format!("{rel_dir}{name}"));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Error::Internal(format!("create {name}: {e}"))),
        }
    }
    Err(Error::Internal(format!("no free file name for {stem}")))
}

/// One path segment: no separators or control characters, no leading dot (a
/// doc must not become a hidden file), at most 120 characters.
fn clean_segment(raw: &str) -> String {
    let s: String = raw
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':') {
                '-'
            } else {
                c
            }
        })
        .take(120)
        .collect();
    s.trim_start_matches('.').trim().to_string()
}

/// `(stem, extension)` for a doc: a name that already carries an extension
/// keeps it; otherwise the language (or, for `auto`, a sniff of the content)
/// picks one.
fn file_name_parts(doc: &ArchivedDoc) -> (String, String) {
    let name = clean_segment(&doc.name);
    let name = if name.is_empty() {
        "untitled".to_string()
    } else {
        name
    };
    if let Some((stem, ext)) = name.rsplit_once('.') {
        if !stem.is_empty()
            && (1..=8).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
        {
            return (stem.to_string(), ext.to_string());
        }
    }
    let ext = match doc.mime.as_deref() {
        Some(mime) => match mime {
            "image/jpeg" => "jpg",
            "image/gif" => "gif",
            "image/webp" => "webp",
            "image/svg+xml" => "svg",
            _ => "png",
        },
        None => language_ext(&doc.language)
            .unwrap_or_else(|| sniff_ext(&String::from_utf8_lossy(&doc.bytes))),
    };
    (name, ext.to_string())
}

/// Extension of an explicit Workbench language id (`None` = `auto`/unknown).
fn language_ext(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "txt" => "txt",
        "md" => "md",
        "json" => "json",
        "yaml" => "yaml",
        "toml" => "toml",
        "csv" => "csv",
        "tsv" => "tsv",
        "sql" => "sql",
        "html" => "html",
        "xml" => "xml",
        "svg" => "svg",
        "css" => "css",
        "js" => "js",
        "ts" => "ts",
        "py" => "py",
        "sh" => "sh",
        "go" => "go",
        "rs" => "rs",
        "java" => "java",
        "mermaid" => "mmd",
        "d2" => "d2",
        "http" => "http",
        "diff" => "diff",
        _ => return None,
    })
}

/// A light content sniff for `auto` docs (the Workbench's detector, reduced to
/// what picks a sensible extension).
fn sniff_ext(text: &str) -> &'static str {
    let t = text.trim_start_matches('\u{feff}').trim();
    let first = t.lines().next().unwrap_or("").trim();
    let word = first
        .split(|c: char| !c.is_ascii_alphabetic())
        .next()
        .unwrap_or("")
        .to_ascii_uppercase();
    if first.starts_with("#!") {
        return if first.contains("python") {
            "py"
        } else if first.contains("node") {
            "js"
        } else {
            "sh"
        };
    }
    if (t.starts_with('{') || t.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(t).is_ok()
    {
        return "json";
    }
    if t.starts_with('<') {
        let lower = t.to_ascii_lowercase();
        return if lower.contains("<svg") {
            "svg"
        } else if lower.starts_with("<!doctype html")
            || lower.contains("<html")
            || lower.contains("<div")
        {
            "html"
        } else {
            "xml"
        };
    }
    if first.to_ascii_lowercase().starts_with("curl ") {
        return "http";
    }
    const SQL: &[&str] = &[
        "SELECT", "INSERT", "UPDATE", "DELETE", "WITH", "CREATE", "ALTER", "DROP", "TRUNCATE",
        "EXPLAIN", "SHOW", "DESCRIBE", "REPLACE", "MERGE", "GRANT", "USE",
    ];
    if SQL.contains(&word.as_str()) {
        return "sql";
    }
    if first.starts_with("# ") || first.starts_with("## ") || first.starts_with("```") {
        return "md";
    }
    "txt"
}

// ---------------------------------------------------------------------------
// File API
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct ScratchFile {
    /// `/`-separated, relative to `root`.
    pub path: String,
    pub size: u64,
    /// RFC 3339.
    pub modified_at: String,
}

#[derive(Serialize)]
pub struct ScratchList {
    /// The caller's folder — the IDE opens it as the Scratches project.
    pub root: String,
    /// Newest first.
    pub files: Vec<ScratchFile>,
}

/// `rel` resolved inside `root`: relative, only plain segments, and — after
/// resolving symlinks of whatever part exists — still inside `root`.
pub fn resolve(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel.trim());
    if rel.trim().is_empty()
        || !rel_path
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
    {
        return Err(Error::Invalid(
            "`path` must be a relative file path inside the Scratches folder (no `..`)".into(),
        ));
    }
    let full = otto_core::secret_paths::canonicalize_prospective(&root.join(rel_path))
        .filter(|p| p.starts_with(root) && p != root)
        .ok_or_else(|| Error::Invalid("`path` leaves the Scratches folder".into()))?;
    Ok(full)
}

fn file_meta(root: &Path, path: &Path, meta: &std::fs::Metadata) -> ScratchFile {
    let modified: chrono::DateTime<chrono::Utc> =
        meta.modified().map(Into::into).unwrap_or_default();
    ScratchFile {
        path: path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/"),
        size: meta.len(),
        modified_at: modified.to_rfc3339(),
    }
}

/// Every non-hidden file under `root` (dot files and dot dirs are skipped).
#[allow(clippy::disallowed_methods)] // only called inside spawn_blocking
fn walk(root: &Path) -> Vec<ScratchFile> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
            } else if ft.is_file() {
                if let Ok(meta) = e.metadata() {
                    out.push(file_meta(root, &e.path(), &meta));
                }
                if out.len() >= MAX_LIST {
                    return out;
                }
            }
        }
    }
    out
}

/// The folder's files, newest first, those whose path contains `query`
/// (case-insensitive) when one is given.
pub async fn list(root: &Path, query: Option<&str>) -> Result<Vec<ScratchFile>> {
    let r = root.to_path_buf();
    let mut files = tokio::task::spawn_blocking(move || walk(&r))
        .await
        .map_err(|e| Error::Internal(format!("list scratches: {e}")))?;
    if let Some(q) = query
        .map(|q| q.trim().to_lowercase())
        .filter(|q| !q.is_empty())
    {
        files.retain(|f| f.path.to_lowercase().contains(&q));
    }
    files.sort_by(|a, b| {
        b.modified_at
            .cmp(&a.modified_at)
            .then_with(|| a.path.cmp(&b.path))
    });
    Ok(files)
}

#[derive(Serialize)]
pub struct ScratchFileFull {
    #[serde(flatten)]
    pub meta: ScratchFile,
    pub content: String,
}

/// Read the UTF-8 text file `rel` (at most [`MAX_FILE_BYTES`]).
pub async fn read(root: PathBuf, rel: String) -> Result<ScratchFileFull> {
    let full = resolve(&root, &rel)?;
    tokio::task::spawn_blocking(move || -> Result<ScratchFileFull> {
        let meta = std::fs::metadata(&full)
            .ok()
            .filter(|m| m.is_file())
            .ok_or_else(|| Error::NotFound(format!("no scratch file `{rel}`")))?;
        if meta.len() as usize > MAX_FILE_BYTES {
            return Err(Error::Invalid("the file is larger than 5 MB".into()));
        }
        let bytes = std::fs::read(&full).map_err(|e| Error::Internal(format!("read: {e}")))?;
        let content = String::from_utf8(bytes)
            .map_err(|_| Error::Invalid("not a text file (it is not valid UTF-8)".into()))?;
        Ok(ScratchFileFull {
            meta: file_meta(&root, &full, &meta),
            content,
        })
    })
    .await
    .map_err(|e| Error::Internal(format!("read scratch: {e}")))?
}

#[derive(Serialize)]
pub struct WriteResp {
    #[serde(flatten)]
    pub meta: ScratchFile,
    pub created: bool,
}

/// Create or replace a scratch file (parent folders are created). The old and
/// new content both land in the IDE's Local History for the Scratches project,
/// so an agent's write can be diffed and undone there.
pub async fn write(
    history: LocalHistoryStore,
    root: PathBuf,
    rel: String,
    content: String,
) -> Result<WriteResp> {
    if content.len() > MAX_FILE_BYTES {
        return Err(Error::Invalid("content is larger than 5 MB".into()));
    }
    let full = resolve(&root, &rel)?;
    tokio::task::spawn_blocking(move || -> Result<WriteResp> {
        if full.is_dir() {
            return Err(Error::Invalid(format!("`{rel}` is a folder")));
        }
        let previous = std::fs::read(&full).ok();
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| Error::Internal(format!("create folder: {e}")))?;
        }
        if let Some(old) = &previous {
            let _ =
                history.record_bytes_sync(&root, &full, old, "saved", "Before agent write", true);
        }
        let tmp = full.with_file_name(format!(
            ".{}.otto-tmp",
            full.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        ));
        std::fs::write(&tmp, content.as_bytes())
            .and_then(|()| std::fs::rename(&tmp, &full))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                Error::Internal(format!("write: {e}"))
            })?;
        let _ = history.record_bytes_sync(
            &root,
            &full,
            content.as_bytes(),
            "saved",
            "Agent write",
            true,
        );
        let meta = std::fs::metadata(&full).map_err(|e| Error::Internal(format!("stat: {e}")))?;
        Ok(WriteResp {
            meta: file_meta(&root, &full, &meta),
            created: previous.is_none(),
        })
    })
    .await
    .map_err(|e| Error::Internal(format!("write scratch: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(name: &str, language: &str, folder: &str, content: &str) -> ArchivedDoc {
        ArchivedDoc {
            id: "d".into(),
            name: name.into(),
            language: language.into(),
            folder: folder.into(),
            bytes: content.as_bytes().to_vec(),
            mime: None,
        }
    }

    #[test]
    fn names_keep_their_extension_or_get_one_from_the_language_or_content() {
        let parts = |d: &ArchivedDoc| file_name_parts(d);
        assert_eq!(
            parts(&doc("q.sql", "auto", "", "")),
            ("q".into(), "sql".into())
        );
        assert_eq!(
            parts(&doc("notes", "md", "", "")),
            ("notes".into(), "md".into())
        );
        assert_eq!(
            parts(&doc("flow", "mermaid", "", "")),
            ("flow".into(), "mmd".into())
        );
        assert_eq!(
            parts(&doc("DimCasinos", "auto", "", "CREATE VIEW x AS SELECT 1")).1,
            "sql"
        );
        assert_eq!(parts(&doc("cfg", "auto", "", "{\"a\": 1}")).1, "json");
        assert_eq!(
            parts(&doc("page", "auto", "", "<!doctype html><p>")).1,
            "html"
        );
        assert_eq!(
            parts(&doc("run", "auto", "", "#!/usr/bin/env python3\n")).1,
            "py"
        );
        assert_eq!(
            parts(&doc("v1.2 plan", "auto", "", "hello")),
            ("v1.2 plan".into(), "txt".into())
        );
        assert_eq!(
            parts(&doc(" ../.env ", "auto", "", "")),
            ("-".into(), "env".into()),
            "no separators, no leading dot"
        );
        assert_eq!(parts(&doc("", "auto", "", "")).0, "untitled");
        let mut img = doc("shot", "image", "", "");
        img.mime = Some("image/jpeg".into());
        assert_eq!(parts(&img), ("shot".into(), "jpg".into()));
    }

    #[test]
    fn export_writes_into_folders_and_never_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::write(root.join("Report.sql"), "mine").unwrap();
        let a = write_new(root, &doc("Report", "sql", "", "SELECT 1")).unwrap();
        // Case-only clash: a separate file on a case-sensitive FS, a
        // numbered one on APFS — never the user's own file.
        let b = write_new(root, &doc("report", "sql", "", "SELECT 2")).unwrap();
        let c = write_new(root, &doc("x", "md", "ops/../daily", "# hi")).unwrap();
        assert_eq!(a, "Report (2).sql");
        assert!(b == "report.sql" || b == "report (3).sql", "{b}");
        assert_eq!(c, "ops/daily/x.md", "`..` segments are dropped");
        assert_eq!(
            std::fs::read_to_string(root.join("Report.sql")).unwrap(),
            "mine"
        );
        assert_eq!(std::fs::read_to_string(root.join(&a)).unwrap(), "SELECT 1");
        assert_eq!(std::fs::read_to_string(root.join(&c)).unwrap(), "# hi");
    }

    #[test]
    fn resolve_keeps_paths_inside_the_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        std::fs::create_dir_all(root.join("a")).unwrap();
        assert_eq!(resolve(&root, "a/b.md").unwrap(), root.join("a/b.md"));
        for bad in ["", "../x", "/etc/passwd", "a/../../x", "./a"] {
            assert!(resolve(&root, bad).is_err(), "{bad}");
        }
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
            assert!(
                resolve(&root, "link/x.md").is_err(),
                "a symlink out of the folder is refused"
            );
        }
    }

    #[test]
    fn user_root_is_one_safe_segment() {
        let home = Path::new("/h");
        assert_eq!(user_root(home, "01KABC"), Path::new("/h/01KABC"));
        assert_eq!(user_root(home, "../../etc"), Path::new("/h/etc"));
        assert_eq!(user_root(home, "///"), Path::new("/h/_"));
    }
}
