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
    /// Opaque change token; stable across unchanged scans, changes after restart.
    #[serde(default)]
    pub generation: Option<String>,
    /// Opaque graph-shape token: changes only when links, titles, tags, OKF
    /// types or the note set change — a body-only save keeps it, so graph
    /// views refetch on this rather than `generation`.
    #[serde(default)]
    pub graph_generation: Option<String>,
    pub id: i64,
    pub scan_state: String,
    pub last_scan_at: Option<String>,
    pub notes: i64,
    pub links: i64,
    pub unresolved: i64,
    pub tags: i64,
    pub attachments: i64,
    /// Recovery dirs (`.otto-history`, `.trash`) the vault's git repo ALREADY
    /// tracks: the `.gitignore` Otto writes only stops NEW files, so private
    /// note history committed before it keeps being pushed (S7-07). The UI
    /// shows `git rm --cached -r <dir>`. Omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tracked_recovery: Vec<String>,
    /// Present only while the full-text index is incomplete (rebuilding
    /// after an upgrade or an explicit rebuild): search still answers, from
    /// the part indexed so far. Omitted when the index is complete.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_index: Option<SearchIndexStatus>,
}

/// Progress of an incomplete full-text index.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchIndexStatus {
    /// `rebuilding` while the background rebuild runs; `pending` when notes
    /// are missing from the index and the rebuild has not started yet.
    pub state: String,
    /// Notes in the full-text index so far.
    pub indexed: i64,
    /// Notes in the vault.
    pub total: i64,
}

/// `POST …/search` with `with_index: true`: the hits plus, while the index
/// is incomplete, how much of the vault they were drawn from.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchPage {
    pub hits: Vec<SearchHit>,
    /// `None` = the index is complete; otherwise the hits may be missing
    /// notes that are not indexed yet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<SearchIndexStatus>,
}

/// `POST …/attachment` result: the saved file and the embed to insert.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultAttachment {
    /// Vault-relative path of the new file (never an existing one).
    pub path: String,
    pub size: i64,
    /// The embed in the vault's link style, e.g. `![[attachments/x.png]]`.
    pub embed: String,
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

/// Whether the note's body participates in the derived content index.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContentIndexStatus {
    #[default]
    Full,
    SizeLimited,
}
impl ContentIndexStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::SizeLimited => "size_limited",
        }
    }
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
    #[serde(default)]
    pub content_index_status: ContentIndexStatus,
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

/// `GET …/backlinks?with_total=true`: one page of backlinks (ordered by
/// source path, then link kind) and how many there are in all.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BacklinkPage {
    pub backlinks: Vec<Backlink>,
    pub total: u64,
    pub offset: usize,
    pub limit: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RenameResult {
    pub from: String,
    pub to: String,
    /// Number of links rewritten in OTHER notes.
    pub links_updated: i64,
    /// Sources whose links could not be rewritten (the move itself still
    /// succeeded and the index was refreshed). Empty on a clean rename.
    #[serde(default)]
    pub links_failed: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default)]
pub struct SearchReq {
    pub query: String,
    pub tag: Option<String>,
    pub path_prefix: Option<String>,
    pub okf_type: Option<String>,
    pub limit: usize,
    /// Answer `{hits, index}` instead of the bare hit array, so a caller can
    /// tell a partial result (index still rebuilding) from a complete one.
    pub with_index: bool,
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
}

/// Compact wire format: parallel arrays for nodes, a flat `[src,dst,...]`
/// index-pair array for edges. `flags` bit 0 = ghost (unresolved), bit 1 =
/// tag node, bit 2 = reserved, bit 3 = attachment.
///
/// `types`/`services`/`tag_*` are the per-node attributes the client filters and
/// groups on — they are indices into the parallel `*_labels` tables, so the wire
/// stays compact on a 9k-note vault. Tags are CSR-encoded: node `i` owns
/// `tag_ids[tag_off[i]..tag_off[i + 1]]`, and `tag_off` has `n + 1` entries.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GraphPayload {
    pub paths: Vec<String>,
    pub titles: Vec<String>,
    /// Normalized OKF frontmatter type per node (case-folded into one bucket).
    pub types: Vec<u16>,
    pub type_labels: Vec<String>,
    /// Top-level folder per node — the repo/service bundle in an OKF vault.
    pub services: Vec<u16>,
    pub service_labels: Vec<String>,
    pub tag_off: Vec<u32>,
    pub tag_ids: Vec<u16>,
    pub tag_labels: Vec<String>,
    pub flags: Vec<u8>,
    pub edges: Vec<u32>,
    /// A cap left nodes or links out: the full graph's edge budget, or the
    /// local graph's node/link caps (the nearest notes win).
    pub truncated: bool,
    /// When `truncated` in `local` mode: the notes the walk found before it
    /// stopped — at least this many are in the neighbourhood.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_nodes: Option<u64>,
    /// When `truncated`: the links among the walked notes (local) or in
    /// the vault (full) before the cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total_edges: Option<u64>,
}

/// Synthetic attribute buckets for nodes that are not notes, and for notes
/// whose frontmatter carries no `type`.
pub const TYPE_UNTYPED: &str = "untyped";
pub const TYPE_GHOST: &str = "unresolved";
pub const TYPE_TAG: &str = "tag";
pub const SERVICE_ROOT: &str = "/";
pub const SERVICE_GHOST: &str = "unresolved";
pub const SERVICE_TAGS: &str = "tags";

pub const NODE_GHOST: u8 = 1;
pub const NODE_TAG: u8 = 2;
pub const NODE_RESERVED: u8 = 4;
pub const NODE_ATTACHMENT: u8 = 8;

/// One OKF conformance finding.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OkfFinding {
    /// `E1` no/unparseable frontmatter · `E2` missing/empty `type` · `E3`
    /// reserved-file structure · `W1` missing title/description · `W2` broken
    /// internal link · `W3` no content-change timestamp · `W4` directory missing index.md ·
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

/// A recoverable write, stored with its before/after bytes in .otto-history.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultRevision {
    pub id: String,
    pub path: String,
    pub created_at: String,
    pub before_hash: Option<String>,
    pub after_hash: String,
    pub reason: String,
    /// False means the snapshots were saved but the write was not confirmed.
    pub committed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultRevisionDetail {
    #[serde(flatten)]
    pub revision: VaultRevision,
    pub before: Option<String>,
    pub after: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VaultTrashEntry {
    pub id: String,
    pub original_path: String,
    pub stored_path: String,
    pub deleted_at: String,
    pub kind: String,
}
