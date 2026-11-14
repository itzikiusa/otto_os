//! Wire contracts for OS/Arch Toolchains, Detected SDKs, Project Environments,
//! Shell Env, Multi-Folder Roots, and Run Configurations.

use std::collections::BTreeMap;

use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};

use super::{default_true, IdeRunnerKind, IdeTargetKind, IdeTargetSpec};

// ---------------------------------------------------------------------------
// B. OS/Arch Toolchains, Detected SDKs & SDK Downloader
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
pub struct IdeDetectedSdk {
    pub ecosystem: IdeEcosystemKind,
    pub name: String,
    pub subtitle: String,
    pub version: String,
    pub arch: String,
    pub home_path: String,
    pub bin_path: String,
    pub path: String,
    pub source: String,
    pub valid: bool,
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
    #[serde(default)]
    pub detected: Vec<IdeDetectedSdk>,
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
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub module_sdks: BTreeMap<String, String>,
    #[serde(default, alias = "vars")]
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
            module_sdks: BTreeMap::new(),
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
    pub module_sdks: Option<BTreeMap<String, String>>,
    #[serde(default, alias = "vars")]
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
            project_folders: Vec::new(),
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
    #[serde(default)]
    pub target: IdeTargetSpec,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default, alias = "path")]
    pub venv_dir: Option<String>,
    #[serde(default, alias = "base_python")]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub maven_profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spring_profiles: Vec<String>,
    #[serde(default)]
    pub skip_tests: bool,
    #[serde(default)]
    pub offline: bool,
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
            maven_profiles: Vec::new(),
            spring_profiles: Vec::new(),
            skip_tests: false,
            offline: false,
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
    #[serde(default, alias = "ad_hoc")]
    pub adhoc: Option<IdeRunConfig>,
    #[serde(
        default,
        alias = "projectFolders",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub project_folders: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub maven_profiles: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spring_profiles: Vec<String>,
    #[serde(default)]
    pub skip_tests: bool,
    #[serde(default)]
    pub offline: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeToolchainPythonVenvReq {
    #[serde(default)]
    pub target: Option<IdeTargetSpec>,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default, alias = "venv_dir")]
    pub path: Option<String>,
    #[serde(default)]
    pub base_python: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeToolchainPythonVenvResp {
    pub ok: bool,
    pub venv: IdePythonVenvInfo,
    pub interpreter_path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeRunResp {
    pub run: IdeRunState,
}
