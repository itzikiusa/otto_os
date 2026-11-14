//! Shared wire contracts and domain types for `otto-ide`.
//!
//! Mirrors `docs/contracts/api.md` ("IDE") and `ui/src/lib/api/types.ts`.

use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod deps_types;
mod history;
mod lsp_projects;
mod runtime;
mod toolchains_env;

pub use deps_types::*;
pub use history::*;
pub use lsp_projects::*;
pub use runtime::*;
pub use toolchains_env::*;

// ---------------------------------------------------------------------------
// SSH connection spec & Target specification
// ---------------------------------------------------------------------------

/// Resolved SSH connection metadata handed to `otto-ide` by [`crate::IdeCtx`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdeSshSpec {
    pub connection_id: Id,
    pub name: String,
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_jump: Option<String>,
}

impl IdeSshSpec {
    pub fn effective_port(&self) -> Option<u16> {
        self.port.filter(|&p| p > 0)
    }
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

pub(crate) fn default_true() -> bool {
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
    #[serde(
        default,
        alias = "projectFolders",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub project_folders: Vec<String>,
}

impl IdeTargetSpec {
    pub fn local(root: impl Into<String>) -> Self {
        Self {
            kind: IdeTargetKind::Local,
            workspace_id: None,
            root: root.into(),
            connection_id: None,
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        }
    }

    pub fn ssh(connection_id: Id, root: impl Into<String>) -> Self {
        Self {
            kind: IdeTargetKind::Ssh,
            workspace_id: None,
            root: root.into(),
            connection_id: Some(connection_id),
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        }
    }

    /// Deduplicated union of `additional_roots` and `project_folders` (excluding `root`).
    pub fn merged_extra_roots(&self) -> Vec<String> {
        let root_trim = self.root.trim().trim_end_matches('/');
        let mut out = Vec::new();
        for raw in self
            .additional_roots
            .iter()
            .chain(self.project_folders.iter())
        {
            let clean = raw.trim();
            if clean.is_empty() || clean.trim_end_matches('/') == root_trim {
                continue;
            }
            if !out.iter().any(|existing: &String| {
                existing.trim_end_matches('/') == clean.trim_end_matches('/')
            }) {
                out.push(clean.to_string());
            }
        }
        out
    }

    /// Merge extra project folders into `additional_roots` in-place.
    pub fn absorb_project_folders(&mut self, extra: &[String]) {
        for raw in extra {
            let clean = raw.trim();
            if !clean.is_empty() && !self.project_folders.iter().any(|x| x == clean) {
                self.project_folders.push(clean.to_string());
            }
        }
        self.additional_roots = self.merged_extra_roots();
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
        for extra in self
            .additional_roots
            .iter()
            .chain(self.project_folders.iter())
        {
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
            project_folders: Vec::new(),
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
            project_folders: Vec::new(),
        };
        assert!(missing_conn.validate().is_err());

        let ssh = IdeTargetSpec::ssh("conn_1".into(), "/srv/app");
        assert!(ssh.validate().is_ok());
        assert_eq!(ssh.project_key(), "ssh:conn_1:/srv/app");
        assert_ne!(local.project_hash(), ssh.project_hash());
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
