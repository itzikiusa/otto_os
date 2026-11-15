//! Wire contracts for Local History (`/ide/history/*`), LSP Diagnostics
//! (`POST /ide/lsp/diagnostics`), and Structured Quick-Doc Hover
//! (`POST /ide/lsp/hover`).

use serde::{Deserialize, Serialize};

use super::{default_true, IdeTargetSpec};

// ---------------------------------------------------------------------------
// LSP Diagnostics (`POST /ide/lsp/diagnostics`) & Structured Hover (`/ide/lsp/hover`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDiagnosticsReq {
    #[serde(default)]
    pub target: Option<IdeTargetSpec>,
    pub path: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(
        default,
        alias = "projectFolders",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub project_folders: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeTextEdit {
    pub path: String,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeQuickFix {
    pub title: String,
    pub line: u32,
    pub end_line: u32,
    pub replacement: String,
    pub delete_line: bool,
    #[serde(default)]
    pub is_preferred: bool,
    #[serde(default)]
    pub edits: Vec<IdeTextEdit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDiagnostic {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub severity: String,
    pub code: String,
    pub source: String,
    pub message: String,
    #[serde(default)]
    pub quick_fixes: Vec<IdeQuickFix>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDiagnosticsResp {
    pub diagnostics: Vec<IdeDiagnostic>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeHoverPosition {
    #[serde(default)]
    pub line: u32,
    #[serde(default)]
    pub character: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeHoverReq {
    #[serde(default)]
    pub target: Option<IdeTargetSpec>,
    #[serde(default, alias = "uri", alias = "file_path")]
    pub path: String,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub line: u32,
    #[serde(default)]
    pub character: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub position: Option<IdeHoverPosition>,
    #[serde(
        default,
        alias = "projectFolders",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub project_folders: Vec<String>,
}

impl IdeHoverReq {
    pub fn effective_line(&self) -> u32 {
        if self.line > 0 {
            self.line
        } else if let Some(pos) = self.position {
            pos.line.saturating_add(1)
        } else {
            1
        }
    }

    pub fn effective_character(&self) -> u32 {
        if self.character > 0 {
            self.character
        } else if let Some(col) = self.column.filter(|&c| c > 0) {
            col
        } else if let Some(pos) = self.position {
            pos.character
        } else {
            0
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeHoverDefinedIn {
    pub path: String,
    pub line: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeHoverResp {
    pub found: bool,
    pub symbol: Option<String>,
    pub kind: Option<String>,
    pub package_name: Option<String>,
    pub signature: Option<String>,
    pub summary: Option<String>,
    pub full_doc: Option<String>,
    pub defined_in: Option<IdeHoverDefinedIn>,
}

// ---------------------------------------------------------------------------
// Local History (`/ide/history/*`)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeLocalHistoryEntry {
    pub id: String,
    pub root: String,
    pub path: String,
    pub rel_path: String,
    pub event_kind: String,
    pub label: String,
    pub timestamp_ms: u64,
    pub size_bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdeHistoryListQuery {
    pub root: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default = "default_true")]
    pub include_deleted: bool,
    #[serde(default)]
    pub limit: Option<usize>,
    #[serde(default)]
    pub q: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeHistoryListResp {
    pub entries: Vec<IdeLocalHistoryEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdeHistoryEntryQuery {
    pub root: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeHistoryEntryResp {
    pub entry: IdeLocalHistoryEntry,
    pub content: String,
    pub current_content: Option<String>,
    pub current_exists: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeHistoryRestoreReq {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub target: Option<IdeTargetSpec>,
    pub id: String,
}

impl IdeHistoryRestoreReq {
    pub fn effective_root(&self) -> &str {
        if !self.root.trim().is_empty() {
            self.root.trim()
        } else {
            self.target
                .as_ref()
                .map(|t| t.root.trim())
                .unwrap_or_default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeHistoryRestoreResp {
    pub ok: bool,
    pub path: String,
    pub content: String,
    pub recreated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeHistorySnapshotReq {
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub target: Option<IdeTargetSpec>,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default = "default_true", alias = "record_local_history")]
    pub local_history_enabled: bool,
    #[serde(
        default,
        alias = "retention_days",
        alias = "localHistoryRetentionDays",
        skip_serializing_if = "Option::is_none"
    )]
    pub local_history_retention_days: Option<u32>,
}

impl IdeHistorySnapshotReq {
    pub fn effective_root(&self) -> &str {
        if !self.root.trim().is_empty() {
            self.root.trim()
        } else {
            self.target
                .as_ref()
                .map(|t| t.root.trim())
                .unwrap_or_default()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeHistorySnapshotResp {
    pub entry: IdeLocalHistoryEntry,
}
