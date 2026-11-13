//! Shared wire contracts and domain types for `otto-ide`.
//!
//! Mirrors `docs/contracts/api.md` ("IDE") and `ui/src/lib/api/types.ts`.

use std::collections::BTreeMap;

use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod runtime;
pub use runtime::*;

// ---------------------------------------------------------------------------
// SSH connection spec & Target specification
// ---------------------------------------------------------------------------

/// Resolved SSH connection metadata handed to `otto-ide` by [`crate::IdeCtx`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdeSshSpec {
    pub connection_id: Id,
    pub name: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_jump: Option<String>,
}

/// Execution / filesystem target kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IdeTargetKind {
    #[default]
    Local,
    Ssh,
}

impl IdeTargetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Ssh => "ssh",
        }
    }
}

fn default_true() -> bool {
    true
}

/// Target workspace/host descriptor passed on IDE requests.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdeTargetSpec {
    #[serde(default)]
    pub kind: IdeTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<Id>,
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<Id>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_roots: Vec<String>,
}

impl IdeTargetSpec {
    pub fn local(root: impl Into<String>) -> Self {
        Self {
            kind: IdeTargetKind::Local,
            workspace_id: None,
            root: root.into(),
            connection_id: None,
            additional_roots: Vec::new(),
        }
    }

    pub fn ssh(connection_id: Id, root: impl Into<String>) -> Self {
        Self {
            kind: IdeTargetKind::Ssh,
            workspace_id: None,
            root: root.into(),
            connection_id: Some(connection_id),
            additional_roots: Vec::new(),
        }
    }

    /// Validate basic structural invariants before touching local FS or SSH.
    pub fn validate(&self) -> Result<()> {
        if self.root.trim().is_empty() {
            return Err(Error::Invalid("target root must not be empty".into()));
        }
        if self.root.chars().any(char::is_control) {
            return Err(Error::Invalid(
                "target root contains control characters".into(),
            ));
        }
        for extra in &self.additional_roots {
            if extra.chars().any(char::is_control) {
                return Err(Error::Invalid(
                    "additional_roots entry contains control characters".into(),
                ));
            }
        }
        if self.kind == IdeTargetKind::Ssh {
            let ok = self
                .connection_id
                .as_deref()
                .is_some_and(|s| !s.trim().is_empty());
            if !ok {
                return Err(Error::Invalid(
                    "connection_id is required when target kind is 'ssh'".into(),
                ));
            }
        }
        Ok(())
    }

    /// Canonical persistence key (`local:/abs/path` or `ssh:<conn_id>:/remote/path`).
    pub fn project_key(&self) -> String {
        let trimmed = self.root.trim_end_matches('/');
        let norm_root = if trimmed.is_empty() { "/" } else { trimmed };
        match self.kind {
            IdeTargetKind::Local => format!("local:{norm_root}"),
            IdeTargetKind::Ssh => {
                let cid = self.connection_id.as_deref().unwrap_or("");
                format!("ssh:{cid}:{norm_root}")
            }
        }
    }

    /// Stable 16-hex SHA-256 hash suitable for Keychain secret namespace keys
    /// (`ide-env:<project_hash>:<key>`).
    pub fn project_hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.project_key().as_bytes());
        let full = hex::encode(hasher.finalize());
        full[..16].to_string()
    }
}

/// Common query-string representation of [`IdeTargetSpec`] for `GET` endpoints.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdeTargetQuery {
    #[serde(default)]
    pub kind: Option<IdeTargetKind>,
    #[serde(default)]
    pub workspace_id: Option<Id>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub connection_id: Option<Id>,
    /// When `true` on `GET /ide/toolchains`, fetch live vendor release catalogs;
    /// defaults to `false` so workspace open never waits on external HTTPS.
    #[serde(default)]
    pub include_catalog: bool,
}

impl IdeTargetQuery {
    pub fn into_target_spec(self) -> Result<IdeTargetSpec> {
        let root = self
            .root
            .filter(|r| !r.trim().is_empty())
            .ok_or_else(|| Error::Invalid("missing 'root' query parameter".into()))?;
        let kind = self.kind.unwrap_or(IdeTargetKind::Local);
        let spec = IdeTargetSpec {
            kind,
            workspace_id: self.workspace_id,
            root,
            connection_id: self.connection_id.filter(|c| !c.trim().is_empty()),
            additional_roots: Vec::new(),
        };
        spec.validate()?;
        Ok(spec)
    }
}

/// Detected OS and CPU architecture on the target machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdePlatformInfo {
    /// Normalized OS: `"macos"` | `"linux"`.
    pub os: String,
    /// Normalized CPU architecture: `"aarch64"` | `"x86_64"`.
    pub arch: String,
}

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
        };
        spec.validate()?;
        Ok(spec)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeFileReadResp {
    pub path: String,
    pub content: String,
    pub language: String,
    pub size: u64,
    pub sha256: String,
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
    pub column: u32,
    #[serde(default)]
    pub symbol: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
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
    pub path: String,
    pub line: u32,
    pub column: u32,
    pub new_name: String,
    #[serde(default)]
    pub apply: bool,
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
    pub column: u32,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<IdeDiagnosticInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCodeAction {
    pub id: String,
    pub title: String,
    pub kind: String,
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
// B. OS/Arch Toolchains & SDK Downloader
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeEcosystemKind {
    Java,
    Go,
    Rust,
    #[serde(alias = "javascript", alias = "typescript", alias = "js", alias = "ts")]
    Node,
    Python,
    #[serde(alias = "bash", alias = "sh", alias = "zsh")]
    Shell,
    Json,
}

impl IdeEcosystemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Java => "java",
            Self::Go => "go",
            Self::Rust => "rust",
            Self::Node => "node",
            Self::Python => "python",
            Self::Shell => "shell",
            Self::Json => "json",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Java => "Java (Temurin JDK)",
            Self::Go => "Go SDK",
            Self::Rust => "Rust (rustup)",
            Self::Node => "Node.js / TypeScript",
            Self::Python => "Python (uv)",
            Self::Shell => "Shell (Bash / Zsh)",
            Self::Json => "JSON / Schema",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum IdeInstallComponent {
    Sdk,
    Lsp,
    Dap,
    #[default]
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeInstalledSdk {
    pub id: String,
    pub version: String,
    pub path: String,
    /// `"system"` | `"otto"` | `"local"`.
    pub source: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeToolBinaryStatus {
    pub name: String,
    pub available: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeDownloadableVersion {
    pub version: String,
    pub label: String,
    pub lts: bool,
    pub os: String,
    pub arch: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeToolchainEcosystem {
    pub ecosystem: IdeEcosystemKind,
    pub display_name: String,
    pub installed: Vec<IdeInstalledSdk>,
    pub lsp_status: IdeToolBinaryStatus,
    pub dap_status: IdeToolBinaryStatus,
    pub downloadable: Vec<IdeDownloadableVersion>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeInstallJobStatus {
    Running,
    Done,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeInstallJob {
    pub id: String,
    pub ecosystem: IdeEcosystemKind,
    pub component: IdeInstallComponent,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub target_os: String,
    pub target_arch: String,
    pub status: IdeInstallJobStatus,
    pub progress_pct: u8,
    pub log_lines: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeToolchainsResp {
    pub target_os: String,
    pub target_arch: String,
    pub ecosystems: Vec<IdeToolchainEcosystem>,
    pub jobs: Vec<IdeInstallJob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeInstallToolchainReq {
    pub target: IdeTargetSpec,
    pub ecosystem: IdeEcosystemKind,
    #[serde(default)]
    pub component: IdeInstallComponent,
    #[serde(default)]
    pub version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeInstallJobResp {
    pub job: IdeInstallJob,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRegisterLocalSdkReq {
    pub target: IdeTargetSpec,
    pub ecosystem: IdeEcosystemKind,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRegisterLocalSdkResp {
    pub sdk: IdeInstalledSdk,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdeDeleteLocalSdkQuery {
    #[serde(default)]
    pub kind: Option<IdeTargetKind>,
    #[serde(default)]
    pub workspace_id: Option<Id>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub connection_id: Option<Id>,
    #[serde(default)]
    pub ecosystem: Option<IdeEcosystemKind>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
}

// ---------------------------------------------------------------------------
// C. Project Environments, Shell Env, Multi-Folder Roots & Run Configurations
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeShellEnvResp {
    /// `"ready"` | `"loading"` | `"fallback"` | `"error"`.
    pub status: String,
    pub shell: String,
    pub vars: BTreeMap<String, String>,
    pub env: BTreeMap<String, String>,
    pub var_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loaded_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeEnvVarEntry {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub has_secret: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeProjectEnvironment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub java_home: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub go_root: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rust_toolchain: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node_home: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub python_interpreter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dotenv_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_path: Option<String>,
    #[serde(default = "default_true")]
    pub inherit_shell_env: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub additional_roots: Vec<String>,
    #[serde(default)]
    pub env_vars: Vec<IdeEnvVarEntry>,
}

impl Default for IdeProjectEnvironment {
    fn default() -> Self {
        Self {
            java_home: None,
            go_root: None,
            rust_toolchain: None,
            node_home: None,
            python_interpreter: None,
            dotenv_path: None,
            extra_path: None,
            inherit_shell_env: true,
            additional_roots: Vec::new(),
            env_vars: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeUpdateEnvironmentReq {
    pub target: IdeTargetSpec,
    #[serde(default)]
    pub java_home: Option<String>,
    #[serde(default)]
    pub go_root: Option<String>,
    #[serde(default)]
    pub rust_toolchain: Option<String>,
    #[serde(default)]
    pub node_home: Option<String>,
    #[serde(default)]
    pub python_interpreter: Option<String>,
    #[serde(default)]
    pub dotenv_path: Option<String>,
    #[serde(default)]
    pub extra_path: Option<String>,
    #[serde(default)]
    pub inherit_shell_env: Option<bool>,
    #[serde(default)]
    pub additional_roots: Option<Vec<String>>,
    #[serde(default)]
    pub env_vars: Vec<IdeEnvVarEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeProjectFolderReq {
    pub target: IdeTargetSpec,
    #[serde(alias = "path")]
    pub folder_path: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IdeDeleteProjectFolderQuery {
    #[serde(default)]
    pub kind: Option<IdeTargetKind>,
    #[serde(default)]
    pub workspace_id: Option<Id>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub connection_id: Option<Id>,
    #[serde(alias = "path")]
    pub folder_path: String,
}

impl IdeDeleteProjectFolderQuery {
    pub fn target_spec(&self) -> Result<IdeTargetSpec> {
        let root = self
            .root
            .clone()
            .filter(|r| !r.trim().is_empty())
            .ok_or_else(|| Error::Invalid("missing 'root' query parameter".into()))?;
        let spec = IdeTargetSpec {
            kind: self.kind.unwrap_or(IdeTargetKind::Local),
            workspace_id: self.workspace_id.clone(),
            root,
            connection_id: self.connection_id.clone().filter(|c| !c.trim().is_empty()),
            additional_roots: Vec::new(),
        };
        spec.validate()?;
        Ok(spec)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdePythonVenvInfo {
    pub path: String,
    pub version: String,
    /// `"venv"` | `"conda"` | `"poetry"` | `"uv"`.
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCreatePythonVenvReq {
    pub target: IdeTargetSpec,
    #[serde(default)]
    pub venv_dir: Option<String>,
    #[serde(default)]
    pub python_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeCreatePythonVenvResp {
    pub path: String,
    pub interpreter_path: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRunConfig {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub runner: IdeRunnerKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub build_flags: Vec<String>,
    #[serde(
        default,
        alias = "working_dir",
        skip_serializing_if = "Option::is_none"
    )]
    pub cwd: Option<String>,
    #[serde(default, alias = "env")]
    pub env_overrides: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_file: Option<String>,
    #[serde(default = "default_true")]
    pub inherit_shell_env: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sdk_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_launch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_filter: Option<String>,
    #[serde(default, alias = "compound_ids", skip_serializing_if = "Vec::is_empty")]
    pub compound_configs: Vec<String>,
    #[serde(default)]
    pub auto_detected: bool,
}

impl Default for IdeRunConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            runner: IdeRunnerKind::Custom,
            command: None,
            args: Vec::new(),
            build_flags: Vec::new(),
            cwd: None,
            env_overrides: BTreeMap::new(),
            env_file: None,
            inherit_shell_env: true,
            sdk_path: None,
            toolchain_id: None,
            before_launch: None,
            target_path: None,
            test_filter: None,
            compound_configs: Vec::new(),
            auto_detected: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeUpsertRunConfigReq {
    pub target: IdeTargetSpec,
    #[serde(flatten)]
    pub config: IdeRunConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRunConfigResp {
    pub config: IdeRunConfig,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeEnvironmentsResp {
    pub env: IdeProjectEnvironment,
    pub python_venvs: Vec<IdePythonVenvInfo>,
    pub dotenv_files: Vec<String>,
    pub configs: Vec<IdeRunConfig>,
    #[serde(default)]
    pub detected_configs: Vec<IdeRunConfig>,
    #[serde(default)]
    pub effective_env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeEnvironmentResp {
    pub env: IdeProjectEnvironment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeRunStatus {
    Running,
    Passed,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdeTestCaseStatus {
    Passed,
    Failed,
    Skipped,
    Running,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeTestCaseResult {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suite: Option<String>,
    pub status: IdeTestCaseStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRunState {
    pub id: String,
    pub title: String,
    pub runner: IdeRunnerKind,
    pub status: IdeRunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub output_seq: u64,
    pub output: Vec<String>,
    pub tests: Vec<IdeTestCaseResult>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeStartRunReq {
    pub target: IdeTargetSpec,
    #[serde(default)]
    pub config_id: Option<String>,
    #[serde(default)]
    pub adhoc: Option<IdeRunConfig>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRunResp {
    pub run: IdeRunState,
}

// ---------------------------------------------------------------------------
// Shared language detection helper
// ---------------------------------------------------------------------------

/// Canonical file-path to IDE language identifier mapping.
pub fn detect_language(path: &str) -> Option<&'static str> {
    let trimmed = path.trim();
    if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("jdt://") {
        return Some("java");
    }
    let clean = trimmed.split('?').next().unwrap_or(trimmed);
    let base = clean.rsplit('/').next().unwrap_or(clean);
    match base {
        "Makefile" | "GNUmakefile" | "makefile" => return Some("makefile"),
        "Dockerfile" => return Some("dockerfile"),
        ".bashrc" | ".bash_profile" | ".zshrc" | ".zshenv" | ".zprofile" => return Some("shell"),
        _ if base.starts_with("Dockerfile.") => return Some("dockerfile"),
        _ => {}
    }
    let ext = base.rsplit_once('.')?.1.to_ascii_lowercase();
    let lang = match ext.as_str() {
        "rs" => "rust",
        "go" => "go",
        "java" | "class" => "java",
        "py" | "pyi" => "python",
        "ts" | "tsx" | "mts" | "cts" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "kt" | "kts" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hxx" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "sql" => "sql",
        "json" | "jsonc" => "json",
        "jsonl" | "ndjson" => "jsonl",
        "csv" => "csv",
        "tsv" => "tsv",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "html" | "htm" => "html",
        "svelte" => "svelte",
        "vue" => "vue",
        "xml" | "svg" => "xml",
        "css" => "css",
        "scss" | "less" => "scss",
        "md" | "markdown" => "markdown",
        "sh" | "bash" | "zsh" | "ksh" | "bats" => "shell",
        "proto" => "protobuf",
        "graphql" | "gql" => "graphql",
        _ => return None,
    };
    Some(lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_spec_validation_and_project_hash() {
        let local = IdeTargetSpec::local("/tmp/demo/");
        assert!(local.validate().is_ok());
        assert_eq!(local.project_key(), "local:/tmp/demo");
        assert_eq!(local.project_hash().len(), 16);

        let missing_conn = IdeTargetSpec {
            kind: IdeTargetKind::Ssh,
            workspace_id: None,
            root: "/srv/app".into(),
            connection_id: None,
            additional_roots: Vec::new(),
        };
        assert!(missing_conn.validate().is_err());

        let ssh = IdeTargetSpec::ssh("conn_1".into(), "/srv/app");
        assert!(ssh.validate().is_ok());
        assert_eq!(ssh.project_key(), "ssh:conn_1:/srv/app");
        assert_ne!(local.project_hash(), ssh.project_hash());
    }

    #[test]
    fn debug_snapshot_and_collab_event_serde_roundtrip() {
        let snap = IdeDebugSessionSnapshot {
            id: "dbg_1".into(),
            title: "Debug TestAdd (Go)".into(),
            language: "go".into(),
            adapter: "dlv".into(),
            state: IdeDebugState::Paused,
            stopped_reason: Some("breakpoint".into()),
            active_thread_id: Some(1),
            active_frame_id: Some(1000),
            paused_file: Some("/repo/calc_test.go".into()),
            paused_line: Some(24),
            threads: vec![IdeThreadInfo {
                id: 1,
                name: "goroutine 1".into(),
            }],
            stack_frames: vec![IdeStackFrame {
                id: 1000,
                name: "pkg.TestAdd".into(),
                path: Some("/repo/calc_test.go".into()),
                line: 24,
                column: 1,
            }],
            scopes: vec![IdeScope {
                name: "Locals".into(),
                variables_reference: 2001,
                variables: vec![IdeVariable {
                    name: "sum".into(),
                    value: "42".into(),
                    var_type: Some("int".into()),
                    variables_reference: 0,
                    indexed_variables: None,
                }],
            }],
            inline_values: vec![IdeInlineValue {
                line: 23,
                name: "sum".into(),
                value: "42".into(),
            }],
            breakpoints: vec![IdeBreakpoint {
                path: "/repo/calc_test.go".into(),
                line: 24,
                enabled: true,
                condition: Some("sum > 10".into()),
                hit_condition: None,
                log_message: None,
                verified: Some(true),
                message: None,
            }],
            output: vec![IdeDebugOutputEntry {
                category: "stdout".into(),
                text: "PASS\n".into(),
            }],
            error: None,
        };

        let json = serde_json::to_string(&snap).unwrap();
        assert!(json.contains(r#""state":"paused""#));
        let back: IdeDebugSessionSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back, snap);

        let ev = IdeCollabEvent::DebugUpdated { session: snap };
        let ev_json = serde_json::to_string(&ev).unwrap();
        assert!(ev_json.contains(r#""type":"debug_updated""#));
        let ev_back: IdeCollabEvent = serde_json::from_str(&ev_json).unwrap();
        assert_eq!(ev_back, ev);
    }

    #[test]
    fn upsert_run_config_flattens_and_defaults_inherit_shell_env_true() {
        let raw = r#"{
            "target": { "kind": "local", "root": "/repo", "additional_roots": ["/lib/kit"] },
            "id": "rc_1",
            "name": "All Go Tests",
            "runner": "go_test",
            "args": ["./..."],
            "build_flags": ["-race"],
            "env_file": ".env.test"
        }"#;
        let parsed: IdeUpsertRunConfigReq = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.target.root, "/repo");
        assert_eq!(parsed.target.additional_roots, vec!["/lib/kit"]);
        assert_eq!(parsed.config.id, "rc_1");
        assert_eq!(parsed.config.runner, IdeRunnerKind::GoTest);
        assert_eq!(parsed.config.args, vec!["./..."]);
        assert_eq!(parsed.config.build_flags, vec!["-race"]);
        assert_eq!(parsed.config.env_file.as_deref(), Some(".env.test"));
        assert!(parsed.config.inherit_shell_env);
    }

    #[test]
    fn detect_language_covers_all_eight_mandatory_formats_and_infra_files() {
        assert_eq!(detect_language("/a/b/Main.java"), Some("java"));
        assert_eq!(detect_language("cmd/server/main.go"), Some("go"));
        assert_eq!(detect_language("src/lib.rs"), Some("rust"));
        assert_eq!(detect_language("src/app.tsx"), Some("typescript"));
        assert_eq!(detect_language("index.mjs"), Some("javascript"));
        assert_eq!(detect_language("tests/test_api.py"), Some("python"));
        assert_eq!(detect_language("scripts/deploy.sh"), Some("shell"));
        assert_eq!(detect_language("/Users/dev/.zshrc"), Some("shell"));
        assert_eq!(detect_language("schema/config.jsonc"), Some("json"));
        assert_eq!(detect_language("events/trace.jsonl"), Some("jsonl"));
        assert_eq!(detect_language("data/users.csv"), Some("csv"));
        assert_eq!(detect_language("data/metrics.tsv"), Some("tsv"));
        assert_eq!(
            detect_language("jdt://contents/java.base/java.util/List.class?=proj"),
            Some("java")
        );
        assert_eq!(detect_language("Makefile"), Some("makefile"));
        assert_eq!(detect_language("Dockerfile.prod"), Some("dockerfile"));
        assert_eq!(detect_language("src/App.kt"), Some("kotlin"));
        assert_eq!(detect_language("README"), None);
    }
}
