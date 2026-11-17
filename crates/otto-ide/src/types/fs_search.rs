//! Wire contracts for IDE target filesystem (`GET/PUT /ide/fs/*`, `POST /ide/fs/mutate`),
//! workspace search & replace (`POST /ide/search`, `POST /ide/replace`), and
//! gutter test/main runners (`POST /ide/gutter`).

use otto_core::{Id, Result};
use serde::{Deserialize, Serialize};

use super::{default_true, IdeTargetKind, IdeTargetSpec};

// ---------------------------------------------------------------------------
// A. Target Filesystem, Search & Gutter
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdeFsTreeQuery {
    #[serde(default)]
    pub kind: Option<IdeTargetKind>,
    #[serde(default)]
    pub workspace_id: Option<Id>,
    pub root: String,
    #[serde(default)]
    pub connection_id: Option<Id>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub show_hidden: bool,
}

impl IdeFsTreeQuery {
    pub fn target_spec(&self) -> Result<IdeTargetSpec> {
        let spec = IdeTargetSpec {
            kind: self.kind.unwrap_or(IdeTargetKind::Local),
            workspace_id: self.workspace_id.clone(),
            root: self.root.clone(),
            connection_id: self.connection_id.clone().filter(|c| !c.trim().is_empty()),
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        };
        spec.validate()?;
        Ok(spec)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeFsEntryKind {
    Dir,
    File,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFsEntry {
    pub name: String,
    pub path: String,
    pub kind: IdeFsEntryKind,
    pub size: u64,
    pub mtime: Option<String>,
    pub git_status: Option<String>,
    pub language: Option<String>,
    /// Git ignores it (local listings only; the Project tree dims it).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ignored: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFsTreeResp {
    pub root: String,
    pub path: String,
    pub parent: Option<String>,
    pub target_os: String,
    pub target_arch: String,
    pub entries: Vec<IdeFsEntry>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdeFileQuery {
    #[serde(default)]
    pub kind: Option<IdeTargetKind>,
    #[serde(default)]
    pub workspace_id: Option<Id>,
    pub root: String,
    #[serde(default)]
    pub connection_id: Option<Id>,
    pub path: String,
}

impl IdeFileQuery {
    pub fn target_spec(&self) -> Result<IdeTargetSpec> {
        let spec = IdeTargetSpec {
            kind: self.kind.unwrap_or(IdeTargetKind::Local),
            workspace_id: self.workspace_id.clone(),
            root: self.root.clone(),
            connection_id: self.connection_id.clone().filter(|c| !c.trim().is_empty()),
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        };
        spec.validate()?;
        Ok(spec)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdeGitBaseQuery {
    #[serde(default)]
    pub kind: Option<IdeTargetKind>,
    #[serde(default)]
    pub workspace_id: Option<Id>,
    #[serde(default)]
    pub root: String,
    #[serde(default)]
    pub connection_id: Option<Id>,
    pub path: String,
}

impl IdeGitBaseQuery {
    pub fn target_spec(&self) -> Result<IdeTargetSpec> {
        let inferred_root = if !self.root.trim().is_empty() {
            self.root.clone()
        } else {
            let p = std::path::Path::new(self.path.trim());
            p.parent()
                .filter(|d| !d.as_os_str().is_empty())
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let spec = IdeTargetSpec {
            kind: self.kind.unwrap_or(IdeTargetKind::Local),
            workspace_id: self.workspace_id.clone(),
            root: inferred_root,
            connection_id: self.connection_id.clone().filter(|c| !c.trim().is_empty()),
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        };
        spec.validate()?;
        Ok(spec)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeGitBaseResp {
    pub in_repo: bool,
    pub repo_root: Option<String>,
    pub rel_path: Option<String>,
    pub head_exists: bool,
    pub content: Option<String>,
    pub binary: bool,
    pub truncated: bool,
}

impl IdeGitBaseResp {
    pub const fn not_in_repo() -> Self {
        Self {
            in_repo: false,
            repo_root: None,
            rel_path: None,
            head_exists: false,
            content: None,
            binary: false,
            truncated: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFileReadResp {
    pub path: String,
    pub content: String,
    pub language: String,
    pub size: u64,
    pub sha256: String,
    #[serde(default)]
    pub binary: bool,
    pub read_only: bool,
    pub truncated: bool,
    pub line_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFileWriteReq {
    pub target: IdeTargetSpec,
    pub path: String,
    pub content: String,
    #[serde(default)]
    pub expected_sha256: Option<String>,
    #[serde(default)]
    pub create_dirs: bool,
    #[serde(default)]
    pub allow_overwrite_large: bool,
    #[serde(default = "default_true", alias = "record_local_history")]
    pub local_history_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_history_retention_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFileWriteResp {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeFsMutateOp {
    Mkdir,
    CreateFile,
    Rename,
    Delete,
    Duplicate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFsMutateReq {
    pub target: IdeTargetSpec,
    pub op: IdeFsMutateOp,
    pub path: String,
    #[serde(default)]
    pub dest_path: Option<String>,
    #[serde(default = "default_true", alias = "record_local_history")]
    pub local_history_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_history_retention_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFsMutateResp {
    pub ok: bool,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IdeSearchMode {
    Files,
    Text,
    Symbols,
    #[default]
    All,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSearchReq {
    pub target: IdeTargetSpec,
    pub query: String,
    #[serde(default)]
    pub mode: IdeSearchMode,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub whole_word: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_path: Option<String>,
    #[serde(default)]
    pub file_glob: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeSearchItemKind {
    File,
    Text,
    Symbol,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSearchItem {
    pub kind: IdeSearchItemKind,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    pub preview: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub symbol_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeSearchResp {
    pub items: Vec<IdeSearchItem>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeReplaceReq {
    pub target: IdeTargetSpec,
    pub query: String,
    pub replacement: String,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub regex: bool,
    #[serde(default)]
    pub whole_word: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_glob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_path: Option<String>,
    #[serde(default)]
    pub paths: Option<Vec<String>>,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeReplacePreview {
    pub path: String,
    pub count: usize,
    pub sample_before: String,
    pub sample_after: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeReplaceResp {
    pub files_changed: usize,
    pub replacements: usize,
    pub previews: Vec<IdeReplacePreview>,
}

// ---------------------------------------------------------------------------
// Gutter Test & Main Runners
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeGutterKind {
    Test,
    Suite,
    Main,
    Bench,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IdeRunnerKind {
    GoBuild,
    GoTest,
    #[serde(alias = "go")]
    GoRun,
    #[serde(alias = "junit")]
    JavaJunit,
    #[serde(alias = "java")]
    JavaMain,
    Maven,
    Gradle,
    CargoTest,
    #[serde(alias = "cargo")]
    CargoRun,
    Node,
    #[serde(alias = "npm_script")]
    Npm,
    Vitest,
    Pytest,
    Python,
    #[serde(alias = "sh", alias = "bash")]
    Shell,
    Compound,
    #[default]
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeQuickRunSpec {
    pub runner: IdeRunnerKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_filter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_program: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub debug_test_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeGutterItem {
    pub line: u32,
    pub kind: IdeGutterKind,
    pub label: String,
    pub language: String,
    pub run_spec: IdeQuickRunSpec,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeGutterReq {
    pub target: IdeTargetSpec,
    pub path: String,
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeGutterResp {
    pub items: Vec<IdeGutterItem>,
}
