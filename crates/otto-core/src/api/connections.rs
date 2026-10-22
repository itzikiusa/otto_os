//! Connections; SFTP file browser (over an SSH connection's existing auth).
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/connections.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Connections
// ---------------------------------------------------------------------------

/// `POST /api/v1/workspaces/{id}/connections` and PATCH variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertConnectionReq {
    pub name: String,
    pub kind: ConnectionKind,
    pub params: Value,
    /// Write-only secret; stored in Keychain, never echoed back.
    pub secret: Option<String>,
    pub first_command: Option<String>,
    /// Section to place this profile in; None/absent = ungrouped.
    #[serde(default)]
    pub section_id: Option<Id>,
    /// Deployment environment (dev/staging/prod). On create, absent = `Dev`.
    /// On PATCH, absent KEEPS the stored value (so a PATCH that omits it never
    /// silently downgrades a `Prod` connection to `Dev` and disables the guard).
    /// `Prod` connections are write-guarded in the DB Explorer.
    #[serde(default)]
    pub environment: Option<Environment>,
    /// Lock the profile against writes/DDL regardless of environment. On create,
    /// absent = `false`. On PATCH, absent KEEPS the stored value (so a PATCH that
    /// omits it never silently un-locks a read-only connection).
    #[serde(default)]
    pub read_only: Option<bool>,
}

/// `POST /api/v1/workspaces/{id}/connection-sections` and `PATCH /connection-sections/{id}`.
/// On create, `parent_id` nests the section (absent/None = top-level); rename ignores it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertSectionReq {
    pub name: String,
    #[serde(default)]
    pub parent_id: Option<Id>,
    /// Tree this section lives in on create: "connections" (default) or "db".
    /// Ignored on rename. See [`crate::domain::ConnectionSection::scope`].
    #[serde(default)]
    pub scope: Option<String>,
}

/// Query string for `GET /workspaces/{id}/connection-sections?scope=…`.
/// Absent → "connections" (the Connections page tree).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SectionScopeQuery {
    #[serde(default)]
    pub scope: Option<String>,
}

/// `POST /api/v1/workspaces/{id}/connection-sections/reorder`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReorderSectionsReq {
    pub ids: Vec<Id>,
}

/// `POST /api/v1/connection-sections/{id}/move` — reparent (None = top-level).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MoveSectionReq {
    #[serde(default)]
    pub parent_id: Option<Id>,
}

/// `POST /api/v1/workspaces/{id}/mcp-servers` — create a workspace MCP server.
/// `enabled` defaults to `false` (servers are never auto-enabled).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateMcpServerReq {
    pub name: String,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: std::collections::BTreeMap<String, String>,
    /// WRITE-ONLY: secret env values. Persisted to the Keychain (never the
    /// row); the response carries only the key names (`secret_env_keys`).
    #[serde(default)]
    pub secret_env: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// SFTP file browser (over an SSH connection's existing auth)
// ---------------------------------------------------------------------------

/// One entry in a remote directory listing (`GET /connections/{id}/sftp/list`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpEntry {
    pub name: String,
    /// "dir" | "file" | "symlink" | "other".
    pub kind: String,
    pub size: u64,
    /// Raw date/time field from the listing (e.g. "Jun 20 12:00"), if present.
    pub mtime: Option<String>,
    /// The 10-char permission string (e.g. "drwxr-xr-x").
    pub perms: String,
    /// For symlinks, the link target (part after " -> "); `None` otherwise.
    pub symlink_target: Option<String>,
}

/// `GET /api/v1/connections/{id}/sftp/list?path=` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpListResp {
    /// The absolute remote path that was listed (resolved from `pwd` when the
    /// request omitted `path`).
    pub path: String,
    pub entries: Vec<SftpEntry>,
    /// `true` when the directory held more than the daemon's cap (20k
    /// entries); `entries` is then the first 20k of the listing.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

/// `POST /api/v1/connections/{id}/sftp/download` — pull a remote file to local.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpDownloadReq {
    pub remote_path: String,
    /// Local destination (file path, or a dir for the default-name case).
    /// A leading `~` is expanded to the daemon user's home.
    pub local_path: String,
}

/// `POST /api/v1/connections/{id}/sftp/download` response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpDownloadResp {
    pub local_path: String,
    pub bytes: u64,
}

/// `POST /api/v1/connections/{id}/sftp/upload` — push a local file to remote.
/// Asynchronous SFTP transfer. Files stage under a unique partial name until
/// successful completion. Timeout covers the entire operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpTransferReq {
    pub direction: String,
    pub local_path: String,
    pub remote_path: String,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpTransfer {
    pub id: Id,
    pub direction: String,
    pub local_path: String,
    pub remote_path: String,
    /// running | finalizing | completed | cancelled | failed | timed_out | outcome_unknown
    pub status: String,
    pub bytes: u64,
    pub total_bytes: Option<u64>,
    pub elapsed_secs: u64,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpUploadReq {
    /// Local source (leading `~` expanded to the daemon user's home).
    pub local_path: String,
    pub remote_path: String,
}

/// `POST /api/v1/connections/{id}/sftp/mkdir`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpMkdirReq {
    pub path: String,
}

/// `POST /api/v1/connections/{id}/sftp/remove` — `dir=true` → rmdir, else rm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpRemoveReq {
    pub path: String,
    #[serde(default)]
    pub dir: bool,
}

/// `POST /api/v1/connections/{id}/sftp/rename`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpRenameReq {
    pub from: String,
    pub to: String,
}

/// `GET /api/v1/connections/{id}/sftp/read?path=` — text view of a small file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SftpReadResp {
    pub text: String,
    /// True when the file exceeded the read cap (content is the capped prefix).
    pub truncated: bool,
}
