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

/// Target workspace/host descriptor passed on IDE requests.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdeTargetSpec {
    #[serde(default)]
    pub kind: IdeTargetKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<Id>,
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_id: Option<Id>,
}

impl IdeTargetSpec {
    pub fn local(root: impl Into<String>) -> Self {
        Self {
            kind: IdeTargetKind::Local,
            workspace_id: None,
            root: root.into(),
            connection_id: None,
        }
    }

    pub fn ssh(connection_id: Id, root: impl Into<String>) -> Self {
        Self {
            kind: IdeTargetKind::Ssh,
            workspace_id: None,
            root: root.into(),
            connection_id: Some(connection_id),
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeReplaceReq {
    pub target: IdeTargetSpec,
    pub query: String,
    pub replacement: String,
    #[serde(default)]
    pub case_sensitive: bool,
    #[serde(default)]
    pub regex: bool,
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
    GoTest,
    GoRun,
    JavaJunit,
    JavaMain,
    Maven,
    Gradle,
    CargoTest,
    CargoRun,
    Node,
    Npm,
    Vitest,
    Pytest,
    Python,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeReferencesReq {
    pub target: IdeTargetSpec,
    pub path: String,
    pub line: u32,
    pub column: u32,
    #[serde(default)]
    pub symbol: Option<String>,
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
    Node,
    Python,
}

impl IdeEcosystemKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Java => "java",
            Self::Go => "go",
            Self::Rust => "rust",
            Self::Node => "node",
            Self::Python => "python",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Java => "Java (Temurin JDK)",
            Self::Go => "Go SDK",
            Self::Rust => "Rust (rustup)",
            Self::Node => "Node.js / TypeScript",
            Self::Python => "Python (uv)",
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
    /// `"system"` | `"otto"`.
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

// ---------------------------------------------------------------------------
// C. Project Environments & Run/Debug Configurations + Test Runner
// ---------------------------------------------------------------------------

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

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default)]
    pub env_vars: Vec<IdeEnvVarEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    pub env_vars: Vec<IdeEnvVarEntry>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default)]
    pub env_overrides: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_launch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub test_filter: Option<String>,
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
    let base = path.rsplit('/').next().unwrap_or(path);
    match base {
        "Makefile" | "GNUmakefile" | "makefile" => return Some("makefile"),
        "Dockerfile" => return Some("dockerfile"),
        _ if base.starts_with("Dockerfile.") => return Some("dockerfile"),
        _ => {}
    }
    let ext = base.rsplit_once('.')?.1.to_ascii_lowercase();
    let lang = match ext.as_str() {
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
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
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "html" | "htm" => "html",
        "svelte" => "svelte",
        "vue" => "vue",
        "xml" | "svg" => "xml",
        "css" => "css",
        "scss" | "less" => "scss",
        "md" | "markdown" => "markdown",
        "sh" | "bash" | "zsh" => "shell",
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
    fn upsert_run_config_flattens_cleanly() {
        let raw = r#"{
            "target": { "kind": "local", "root": "/repo" },
            "id": "rc_1",
            "name": "All Go Tests",
            "runner": "go_test",
            "args": ["./..."]
        }"#;
        let parsed: IdeUpsertRunConfigReq = serde_json::from_str(raw).unwrap();
        assert_eq!(parsed.target.root, "/repo");
        assert_eq!(parsed.config.id, "rc_1");
        assert_eq!(parsed.config.runner, IdeRunnerKind::GoTest);
        assert_eq!(parsed.config.args, vec!["./..."]);
    }

    #[test]
    fn detect_language_covers_first_class_and_infra_files() {
        assert_eq!(detect_language("/a/b/Main.java"), Some("java"));
        assert_eq!(detect_language("cmd/server/main.go"), Some("go"));
        assert_eq!(detect_language("src/lib.rs"), Some("rust"));
        assert_eq!(detect_language("src/app.tsx"), Some("typescript"));
        assert_eq!(detect_language("index.mjs"), Some("javascript"));
        assert_eq!(detect_language("tests/test_api.py"), Some("python"));
        assert_eq!(detect_language("Makefile"), Some("makefile"));
        assert_eq!(detect_language("Dockerfile.prod"), Some("dockerfile"));
        assert_eq!(detect_language("src/App.kt"), Some("kotlin"));
        assert_eq!(detect_language("README"), None);
    }
}
