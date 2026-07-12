//! DTOs for the Vault v3 docs home. Everything here mirrors
//! `ui/src/lib/api/types.ts` (contracts-first — `docs/contracts/api.md`).

use serde::{Deserialize, Serialize};

/// A registered vault: a local directory of markdown files. Files are the
/// source of truth; the DB rows are a derived, rebuildable index.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultRec {
    pub id: i64,
    pub ws_id: String,
    pub name: String,
    pub root_path: String,
    pub okf: bool,
    pub created_at: String,
    pub last_scan_at: Option<String>,
    pub scan_state: String,
    pub notes: i64,
    pub links: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultStatus {
    pub id: i64,
    pub scan_state: String,
    pub last_scan_at: Option<String>,
    pub notes: i64,
    pub links: i64,
    pub unresolved: i64,
    pub tags: i64,
    pub attachments: i64,
}

/// One entry of a lazy directory listing (folders first, then notes/files).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirEntry {
    pub name: String,
    pub path: String,
    /// `dir` | `note` | `file`
    pub kind: String,
    /// Direct children count for dirs; 0 otherwise.
    pub children: i64,
    pub title: Option<String>,
    pub okf_type: Option<String>,
    pub reserved: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirListing {
    pub path: String,
    pub entries: Vec<DirEntry>,
}

/// A heading inside a note (outline + anchor targets).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Heading {
    pub level: u8,
    pub text: String,
    pub line: u32,
}

/// Indexed metadata of a note (everything except the raw markdown).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoteMeta {
    pub path: String,
    pub title: String,
    pub okf_type: Option<String>,
    pub description: Option<String>,
    pub frontmatter: serde_json::Value,
    pub tags: Vec<String>,
    pub aliases: Vec<String>,
    pub headings: Vec<Heading>,
    pub word_count: i64,
    pub size: i64,
    pub hash: String,
    pub reserved: bool,
    pub has_frontmatter: bool,
    pub parse_error: bool,
}

/// An outgoing link of a note.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct OutgoingLink {
    pub raw_target: String,
    pub dst_path: Option<String>,
    pub kind: String, // wiki | md | embed
    pub anchor: Option<String>,
    pub alias: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NoteFull {
    pub meta: NoteMeta,
    pub raw: String,
    pub outgoing: Vec<OutgoingLink>,
}

/// Metadata returned after writing a guarded, non-Markdown text artifact.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultTextFile {
    pub path: String,
    pub size: i64,
    pub hash: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct WriteTextFileReq {
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub if_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Backlink {
    pub path: String,
    pub title: String,
    /// A short context snippet around the mention.
    pub context: String,
    pub kind: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenameResult {
    pub from: String,
    pub to: String,
    /// Number of links rewritten in OTHER notes.
    pub links_updated: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct SearchReq {
    pub query: String,
    pub tag: Option<String>,
    pub path_prefix: Option<String>,
    pub okf_type: Option<String>,
    pub limit: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchHit {
    pub path: String,
    pub title: String,
    pub snippet: String,
    pub score: f32,
    pub reserved: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SwitchHit {
    pub path: String,
    pub title: String,
    /// The alias that matched (insert `[[path|alias]]`), if any.
    pub alias: Option<String>,
    pub score: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TagCount {
    pub tag: String,
    pub count: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct GraphOpts {
    /// `full` (default) | `local`
    pub mode: String,
    /// Focus note for `local` mode.
    pub path: Option<String>,
    /// BFS depth for `local` mode (1..=3, default 1).
    pub depth: usize,
    /// Include tag nodes.
    pub tags: bool,
    /// Include orphan notes (no links either way). Default true.
    pub orphans: Option<bool>,
    /// Include reserved files (index.md / log.md). Default false.
    pub reserved: bool,
    /// Include unresolved link targets as ghost nodes.
    pub ghosts: bool,
    /// Server-side edge budget for `full` mode (default 2_000_000).
    pub edge_budget: usize,
    /// Group nodes by `folder` (default) or `type` (OKF frontmatter type).
    pub group_by: String,
}

/// Compact wire format: parallel arrays for nodes, a flat `[src,dst,...]`
/// index-pair array for edges. `flags` bit 0 = ghost (unresolved), bit 1 =
/// tag node, bit 2 = reserved, bit 3 = attachment.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphPayload {
    pub paths: Vec<String>,
    pub titles: Vec<String>,
    pub groups: Vec<u16>,
    pub group_labels: Vec<String>,
    pub flags: Vec<u8>,
    pub edges: Vec<u32>,
    pub truncated: bool,
}

pub const NODE_GHOST: u8 = 1;
pub const NODE_TAG: u8 = 2;
pub const NODE_RESERVED: u8 = 4;
pub const NODE_ATTACHMENT: u8 = 8;

/// One OKF conformance finding.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OkfFinding {
    /// `E1` no/unparseable frontmatter · `E2` missing/empty `type` · `E3`
    /// reserved-file structure · `W1` missing title/description · `W2` broken
    /// internal link · `W3` no timestamp · `W4` directory missing index.md ·
    /// `W5` log dates not ISO.
    pub rule: String,
    pub path: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OkfReport {
    pub conformant: bool,
    pub errors: Vec<OkfFinding>,
    pub warnings: Vec<OkfFinding>,
    pub checked_notes: i64,
}
