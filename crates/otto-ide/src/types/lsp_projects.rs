//! Wire contracts for LSP Extended Operations, Named Multi-Repo IDE Projects
//! (`/ide/projects/*`), and `<15ms` `@` File & Path Completion (`POST /ide/fs/complete`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{IdeTargetSpec, IdeTextEdit};

// ---------------------------------------------------------------------------
// LSP Extended Operations (Structure, References, Rename, Format, Actions)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSymbolsReq {
    pub target: IdeTargetSpec,
    pub path: String,
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSymbolNode {
    pub name: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub line: u32,
    pub end_line: u32,
    #[serde(default)]
    pub children: Vec<IdeSymbolNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSymbolsResp {
    pub symbols: Vec<IdeSymbolNode>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeReferencesReq {
    pub target: IdeTargetSpec,
    pub path: String,
    pub line: u32,
    #[serde(default)]
    pub column: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<u32>,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(
        default,
        alias = "projectFolders",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub project_folders: Vec<String>,
}

impl IdeReferencesReq {
    pub fn effective_column(&self) -> u32 {
        if self.column > 0 {
            self.column
        } else {
            self.character.unwrap_or(0)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRenameLocation {
    pub path: String,
    pub line: u32,
    pub column: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeReferenceLocation {
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeReferencesResp {
    pub symbol: String,
    pub locations: Vec<IdeReferenceLocation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRenameReq {
    pub target: IdeTargetSpec,
    #[serde(
        default,
        alias = "projectFolders",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub project_folders: Vec<String>,
    pub path: String,
    pub line: u32,
    #[serde(default)]
    pub column: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    pub new_name: String,
    #[serde(default)]
    pub apply: bool,
    #[serde(default = "super::default_true", alias = "record_local_history")]
    pub local_history_enabled: bool,
    #[serde(
        default,
        alias = "retention_days",
        alias = "localHistoryRetentionDays",
        skip_serializing_if = "Option::is_none"
    )]
    pub local_history_retention_days: Option<u32>,
}

impl IdeRenameReq {
    pub fn effective_column(&self) -> u32 {
        if self.column > 0 {
            self.column
        } else {
            self.character.unwrap_or(0)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRenameLinePreview {
    pub line: u32,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRenameFileEdit {
    pub path: String,
    pub occurrences: usize,
    pub preview_lines: Vec<IdeRenameLinePreview>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRenameResp {
    pub symbol: String,
    pub new_name: String,
    pub applied: bool,
    pub edits: Vec<IdeRenameFileEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFormatReq {
    pub target: IdeTargetSpec,
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub organize_imports: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFormatResp {
    pub formatted: String,
    pub changed: bool,
    pub formatter: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDiagnosticInput {
    pub line: u32,
    #[serde(default)]
    pub column: Option<u32>,
    pub message: String,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCodeActionsReq {
    pub target: IdeTargetSpec,
    pub path: String,
    pub line: u32,
    #[serde(default)]
    pub column: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<u32>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<IdeDiagnosticInput>,
}

impl IdeCodeActionsReq {
    pub fn effective_column(&self) -> u32 {
        if self.column > 0 {
            self.column
        } else {
            self.character.unwrap_or(0)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCodeAction {
    pub id: String,
    pub title: String,
    pub kind: String,
    #[serde(default)]
    pub is_preferred: bool,
    #[serde(default)]
    pub edits: Vec<IdeTextEdit>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit_preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement_content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCodeActionsResp {
    pub actions: Vec<IdeCodeAction>,
}

// ---------------------------------------------------------------------------
// Named Multi-Repo IDE Projects (`/ide/projects/*`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeNamedProject {
    pub id: String,
    pub name: String,
    pub primary_root: String,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub updated_at: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeProjectsResp {
    pub projects: Vec<IdeNamedProject>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeUpsertProjectReq {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub primary_root: String,
    #[serde(default)]
    pub folders: Vec<String>,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeUpsertProjectResp {
    pub project: IdeNamedProject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeScannedRepo {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub has_go_mod: bool,
    pub has_pom_or_gradle: bool,
    pub has_git: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeScanReposReq {
    #[serde(alias = "parent_dir", alias = "path")]
    pub parent_path: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeScanReposResp {
    pub parent_path: String,
    pub repos: Vec<IdeScannedRepo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeBulkFoldersReq {
    pub target: IdeTargetSpec,
    #[serde(default, alias = "folders")]
    pub folder_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeBulkFoldersResp {
    pub additional_roots: Vec<String>,
}

// ---------------------------------------------------------------------------
// `<15ms` `@` File & Path Completion (`POST /ide/fs/complete`)
// ---------------------------------------------------------------------------

fn default_complete_limit() -> usize {
    40
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFsCompleteReq {
    #[serde(default, alias = "prefix")]
    pub query: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub roots: Vec<String>,
    #[serde(default)]
    pub target: Option<IdeTargetSpec>,
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    #[serde(default = "default_complete_limit")]
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFsCompleteItem {
    pub path: String,
    pub display: String,
    pub insert_token: String,
    pub name: String,
    pub kind: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFsCompleteResp {
    pub items: Vec<IdeFsCompleteItem>,
    pub base_dir: String,
    pub elapsed_ms: u64,
}

// ---------------------------------------------------------------------------
// LSP Pool & RSS Budget Status (`GET /ide/lsp/status`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeLspAvailableServer {
    pub language: String,
    pub available: bool,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeLspServerStat {
    pub language: String,
    pub root: String,
    pub owner: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub rss_bytes: u64,
    pub active_clients: usize,
    pub open_documents: usize,
    pub remote_ssh: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeLspStatusResp {
    pub active_server_count: usize,
    pub max_servers_per_owner: usize,
    pub max_global_servers: usize,
    pub total_rss_bytes: u64,
    pub rss_budget_bytes: u64,
    pub servers: Vec<IdeLspServerStat>,
    #[serde(default)]
    pub available_servers: Vec<IdeLspAvailableServer>,
}
