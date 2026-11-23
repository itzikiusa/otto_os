//! Obsidian-compatible vault: each memory is a markdown note with YAML
//! frontmatter + `[[wikilinks]]`. Files are a human-facing, git-shareable
//! representation (the SQLite store is the derived index). Sharing a vault folder
//! (git / Dropbox / Syncthing) + `reindex` is the file-based way a team shares
//! memory across machines.

use std::io::Write;
use std::path::{Path, PathBuf};

use otto_core::{Error, Result};
use otto_state::memory::{Memory, NewMemory, Scope};

fn io(e: std::io::Error) -> Error {
    Error::Internal(format!("vault io: {e}"))
}

fn slug(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
        if out.chars().filter(|c| *c != '-').count() >= 40 {
            break;
        }
    }
    out.trim_matches('-').to_string()
}

fn short(id: &str) -> String {
    let mut tail: Vec<char> = id.chars().rev().take(8).collect();
    tail.reverse();
    tail.into_iter().collect()
}

/// Format a YAML scalar safely: keep simple single-line scalars plain (for
/// Obsidian readability and backward compatibility), and JSON/YAML double-quote
/// anything containing newlines, colons, flow indicators, quotes, leading
/// markers (`-`, `?`, `#`, `&`, `*`, `!`, `|`, `>`, `%`, `@`, `` ` ``),
/// frontmatter delimiters (`---` / `...`), control chars, or leading/trailing
/// whitespace so untrusted text can never inject frontmatter keys.
fn yaml_scalar(s: &str) -> String {
    if is_safe_plain_yaml_scalar(s) {
        s.to_string()
    } else {
        serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
    }
}

fn is_safe_plain_yaml_scalar(s: &str) -> bool {
    if s.is_empty() || s.trim() != s || s == "---" || s == "..." {
        return false;
    }
    let first = s.chars().next().unwrap();
    if matches!(
        first,
        '-' | '?'
            | ':'
            | ','
            | '['
            | ']'
            | '{'
            | '}'
            | '#'
            | '&'
            | '*'
            | '!'
            | '|'
            | '>'
            | '\''
            | '"'
            | '%'
            | '@'
            | '`'
    ) {
        return false;
    }
    !s.chars().any(|c| {
        c.is_control()
            || matches!(
                c,
                '\n' | '\r' | '\0' | ':' | '#' | '"' | '\'' | '\\' | '[' | ']' | '{' | '}' | ','
            )
    })
}

fn unquote_yaml_scalar(raw: &str) -> String {
    let v = raw.trim();
    if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
        if let Ok(decoded) = serde_json::from_str::<String>(v) {
            return decoded;
        }
    }
    v.to_string()
}

fn parse_yaml_flow_list(raw: &str) -> Vec<String> {
    let inner = raw.trim().trim_start_matches('[').trim_end_matches(']');
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    for ch in inner.chars() {
        if in_quotes {
            cur.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_quotes = false;
            }
        } else {
            match ch {
                '"' => {
                    in_quotes = true;
                    cur.push(ch);
                }
                ',' => {
                    let item = unquote_yaml_scalar(&cur);
                    if !item.is_empty() {
                        out.push(item);
                    }
                    cur.clear();
                }
                _ => cur.push(ch),
            }
        }
    }
    let item = unquote_yaml_scalar(&cur);
    if !item.is_empty() {
        out.push(item);
    }
    out
}

/// Validate a workspace identifier before joining it as a single directory
/// component under the vault root. Rejects empty strings, `.`, `..`, path
/// separators (`/` / `\`), NUL bytes, and any non-normal path component.
fn validate_workspace_id(ws: &str) -> Result<&str> {
    if ws.is_empty()
        || ws == "."
        || ws == ".."
        || ws.contains('/')
        || ws.contains('\\')
        || ws.contains('\0')
    {
        return Err(Error::Invalid(format!(
            "invalid vault workspace id: {ws:?}"
        )));
    }
    let mut comps = Path::new(ws).components();
    match (comps.next(), comps.next()) {
        (Some(std::path::Component::Normal(_)), None) => Ok(ws),
        _ => Err(Error::Invalid(format!(
            "invalid vault workspace id: {ws:?}"
        ))),
    }
}

/// Strict fallback sanitization for infallible path formatting (`note_path`).
fn sanitize_workspace_id(ws: &str) -> String {
    let cleaned = slug(ws);
    if cleaned.is_empty() {
        "invalid".to_string()
    } else {
        cleaned
    }
}

/// Render a memory as an Obsidian note: frontmatter + wikilinks + body.
pub fn to_markdown(m: &Memory, links: &[String]) -> String {
    let mut s = String::from("---\n");
    s.push_str(&format!("id: {}\n", yaml_scalar(&m.id)));
    s.push_str(&format!("kind: {}\n", yaml_scalar(&m.kind)));
    s.push_str(&format!("collection: {}\n", yaml_scalar(&m.collection)));
    s.push_str(&format!("scope: {}\n", yaml_scalar(m.scope.as_str())));
    s.push_str(&format!("source_kind: {}\n", yaml_scalar(&m.source_kind)));
    if let Some(sr) = &m.source_ref {
        s.push_str(&format!("source_ref: {}\n", yaml_scalar(sr)));
    }
    s.push_str(&format!("visibility: {}\n", yaml_scalar(&m.visibility)));
    let tags: Vec<String> = m.tags.iter().map(|t| yaml_scalar(t)).collect();
    s.push_str(&format!("tags: [{}]\n", tags.join(", ")));
    s.push_str(&format!("title: {}\n", yaml_scalar(&m.title)));
    s.push_str("---\n\n");
    if !links.is_empty() {
        let joined: Vec<String> = links.iter().map(|l| format!("[[{l}]]")).collect();
        s.push_str(&joined.join(" "));
        s.push_str("\n\n");
    }
    s.push_str(&m.body);
    s.push('\n');
    s
}

/// Parse a vault note back into a `NewMemory` (for re-indexing external edits /
/// a shared vault). Unknown/missing fields fall back to sensible defaults.
pub fn parse_to_new(content: &str) -> Option<NewMemory> {
    let rest = content.strip_prefix("---")?;
    let end = rest.find("\n---")?;
    let front = &rest[..end];
    let body_raw = &rest[end + 4..];

    let mut kind = "fact".to_string();
    let mut collection = "product".to_string();
    let mut scope = Scope::Workspace;
    let mut source_kind = "manual".to_string();
    let mut source_ref: Option<String> = None;
    let mut visibility = "shared".to_string();
    let mut tags: Vec<String> = vec![];
    let mut title = "note".to_string();

    for line in front.lines() {
        let line = line.trim();
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v_raw = v.trim();
        let v = unquote_yaml_scalar(v_raw);
        match k.trim() {
            "kind" => kind = v,
            "collection" => collection = v,
            "scope" => scope = Scope::parse(&v),
            "source_kind" => source_kind = v,
            "source_ref" if !v.is_empty() => source_ref = Some(v),
            "visibility" => visibility = v,
            "title" => title = v,
            "tags" => tags = parse_yaml_flow_list(v_raw),
            _ => {}
        }
    }

    // Body = note text minus the leading wikilink line(s).
    let body: String = body_raw
        .lines()
        .skip_while(|l| l.trim().is_empty())
        .skip_while(|l| {
            let t = l.trim();
            t.starts_with("[[") && t.ends_with("]]")
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if body.is_empty() {
        return None;
    }

    Some(NewMemory {
        collection,
        record_type: "item".into(),
        visibility,
        scope,
        story_id: None,
        kind,
        title,
        body,
        entities: vec![],
        tags,
        source_kind,
        source_ref,
        refs: vec![],
        confidence: None,
        salience: None,
    })
}

/// Writes notes into `<root>/<workspace>/`.
#[derive(Clone)]
pub struct VaultWriter {
    root: PathBuf,
}

impl VaultWriter {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn workspace_dir(&self, ws: &str) -> Result<PathBuf> {
        let ws = validate_workspace_id(ws)?;
        Ok(self.root.join(ws))
    }

    pub fn note_path(&self, ws: &str, m: &Memory) -> PathBuf {
        let name = format!("{}-{}.md", slug(&m.title), short(&m.id));
        let dir = self
            .workspace_dir(ws)
            .unwrap_or_else(|_| self.root.join(sanitize_workspace_id(ws)));
        dir.join(name)
    }

    pub fn write(&self, ws: &str, m: &Memory, links: &[String]) -> Result<PathBuf> {
        self.write_atomic(ws, m, links, false)
    }

    /// Idempotent mirror export. Existing files may contain human edits and
    /// are never replaced by a duplicate save or a retry after partial success.
    pub fn write_missing(&self, ws: &str, m: &Memory, links: &[String]) -> Result<PathBuf> {
        self.write_atomic(ws, m, links, true)
    }

    fn write_atomic(
        &self,
        ws: &str,
        m: &Memory,
        links: &[String],
        missing_only: bool,
    ) -> Result<PathBuf> {
        let dir = self.workspace_dir(ws)?;
        std::fs::create_dir_all(&dir).map_err(io)?;
        let path = dir.join(format!("{}-{}.md", slug(&m.title), short(&m.id)));
        if missing_only && existing_export(&path).map_err(io)? {
            return Ok(path);
        }
        let temp = dir.join(format!(".otto-export-{}", otto_core::new_id()));
        let result = (|| -> std::io::Result<()> {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(to_markdown(m, links).as_bytes())?;
            file.sync_all()?;
            drop(file);
            if missing_only {
                // A hard link publishes a complete file without replacing a
                // winner from another exporter/editor (same directory/device).
                match std::fs::hard_link(&temp, &path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                        if !existing_export(&path)? {
                            return Err(e);
                        }
                    }
                    Err(e) => return Err(e),
                }
            } else {
                std::fs::rename(&temp, &path)?;
            }
            std::fs::File::open(&dir)?.sync_all()
        })();
        let _ = std::fs::remove_file(&temp);
        result.map_err(io)?;
        Ok(path)
    }
}

fn existing_export(path: &Path) -> std::io::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.is_file() => Ok(true),
        Ok(_) => Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "vault export destination is not a regular file",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

/// Read every `.md` note in a directory into `NewMemory` candidates.
/// Blocking: async callers run it via `spawn_blocking`.
#[allow(clippy::disallowed_methods)] // sync by contract (see above)
pub fn read_dir_notes(dir: &Path) -> Result<Vec<NewMemory>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let p = entry.map_err(io)?.path();
        if p.extension().and_then(|e| e.to_str()) == Some("md") {
            let content = std::fs::read_to_string(&p).map_err(io)?;
            if let Some(nm) = parse_to_new(&content) {
                out.push(nm);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_memory() -> Memory {
        Memory {
            id: "01JABCDEF123456789".into(),
            workspace_id: "ws-1".into(),
            collection: "product".into(),
            record_type: "item".into(),
            scope: Scope::Workspace,
            story_id: None,
            kind: "fact".into(),
            title: "Safe title".into(),
            body: "Body text".into(),
            entities: vec![],
            tags: vec!["payments".into(), "ops".into()],
            source_kind: "manual".into(),
            source_ref: None,
            refs: vec![],
            confidence: 0.9,
            salience: 0.8,
            content_hash: "h".into(),
            active: true,
            superseded_by: None,
            version: 1,
            created_by: "u1".into(),
            created_at: "2026-01-01T00:00:00Z".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            last_accessed_at: None,
            access_count: 0,
            expires_at: None,
            visibility: "shared".into(),
            state: "accepted".into(),
            provenance_json: None,
            forgotten_at: None,
            undo_token: None,
        }
    }

    #[test]
    fn workspace_dir_rejects_traversal_and_invalid_names() {
        let tmp = tempfile::tempdir().unwrap();
        let writer = VaultWriter::new(tmp.path());
        for bad in ["", ".", "..", "../escape", "a/b", "a\\b", "ws\0id", "/abs"] {
            assert!(
                matches!(writer.workspace_dir(bad), Err(Error::Invalid(_))),
                "expected Invalid for {bad:?}"
            );
            assert!(
                matches!(
                    writer.write(bad, &sample_memory(), &[]),
                    Err(Error::Invalid(_))
                ),
                "write must reject {bad:?}"
            );
        }
        assert_eq!(
            writer.workspace_dir("ws-123").unwrap(),
            tmp.path().join("ws-123")
        );
    }

    #[test]
    fn to_markdown_escapes_yaml_frontmatter_injection_and_round_trips() {
        let mut m = sample_memory();
        m.title = "Evil: title\nvisibility: private\n---\ninjected body".into();
        m.tags = vec!["a, b".into(), "x: y\n---\nz".into()];
        m.source_ref = Some("ref: with\nnewline".into());

        let md = to_markdown(&m, &[]);
        // Frontmatter must contain only the opening and closing `---` delimiters.
        let delimiter_lines = md.lines().filter(|l| l.trim() == "---").count();
        assert_eq!(
            delimiter_lines, 2,
            "frontmatter injection broke delimiters:\n{md}"
        );

        let parsed = parse_to_new(&md).expect("round-trip parse");
        assert_eq!(
            parsed.visibility, "shared",
            "injected visibility key must not take effect"
        );
        assert_eq!(parsed.title, m.title);
        assert_eq!(parsed.tags, m.tags);
        assert_eq!(parsed.source_ref, m.source_ref);
        assert_eq!(parsed.body, "Body text");
    }

    #[test]
    fn short_handles_multibyte_utf8_without_panicking() {
        assert_eq!(short("abcdef12345678"), "12345678");
        assert_eq!(short("short"), "short");
        // Multibyte emoji & Hebrew characters where byte_len - 8 would land mid-char.
        let utf8_id = "🔥🚀✨אבגדהוזחט";
        assert_eq!(short(utf8_id), "בגדהוזחט");
        assert_eq!(short("אבג"), "אבג");
    }
}
