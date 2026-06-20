//! Obsidian-compatible vault: each memory is a markdown note with YAML
//! frontmatter + `[[wikilinks]]`. Files are a human-facing, git-shareable
//! representation (the SQLite store is the derived index). Sharing a vault folder
//! (git / Dropbox / Syncthing) + `reindex` is the file-based way a team shares
//! memory across machines.

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
    let n = id.len().saturating_sub(8);
    id[n..].to_string()
}

/// Render a memory as an Obsidian note: frontmatter + wikilinks + body.
pub fn to_markdown(m: &Memory, links: &[String]) -> String {
    let mut s = String::from("---\n");
    s.push_str(&format!("id: {}\n", m.id));
    s.push_str(&format!("kind: {}\n", m.kind));
    s.push_str(&format!("collection: {}\n", m.collection));
    s.push_str(&format!("scope: {}\n", m.scope.as_str()));
    s.push_str(&format!("source_kind: {}\n", m.source_kind));
    if let Some(sr) = &m.source_ref {
        s.push_str(&format!("source_ref: {sr}\n"));
    }
    s.push_str(&format!("visibility: {}\n", m.visibility));
    s.push_str(&format!("tags: [{}]\n", m.tags.join(", ")));
    s.push_str(&format!("title: {}\n", m.title));
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
        let v = v.trim();
        match k.trim() {
            "kind" => kind = v.to_string(),
            "collection" => collection = v.to_string(),
            "scope" => scope = Scope::parse(v),
            "source_kind" => source_kind = v.to_string(),
            "source_ref" if !v.is_empty() => source_ref = Some(v.to_string()),
            "visibility" => visibility = v.to_string(),
            "title" => title = v.to_string(),
            "tags" => {
                let inner = v.trim_start_matches('[').trim_end_matches(']');
                tags = inner
                    .split(',')
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty())
                    .collect();
            }
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

    pub fn workspace_dir(&self, ws: &str) -> PathBuf {
        self.root.join(ws)
    }

    pub fn note_path(&self, ws: &str, m: &Memory) -> PathBuf {
        let name = format!("{}-{}.md", slug(&m.title), short(&m.id));
        self.workspace_dir(ws).join(name)
    }

    pub fn write(&self, ws: &str, m: &Memory, links: &[String]) -> Result<PathBuf> {
        let dir = self.workspace_dir(ws);
        std::fs::create_dir_all(&dir).map_err(io)?;
        let path = self.note_path(ws, m);
        std::fs::write(&path, to_markdown(m, links)).map_err(io)?;
        Ok(path)
    }
}

/// Read every `.md` note in a directory into `NewMemory` candidates.
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
