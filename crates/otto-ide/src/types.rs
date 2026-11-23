//! Shared wire contracts and domain types for `otto-ide`.
//!
//! Mirrors `docs/contracts/api.md` ("IDE") and `ui/src/lib/api/types.ts`.

use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod deps_types;
mod fs_search;
mod history;
mod lsp_projects;
mod runtime;
mod toolchains_env;

pub use deps_types::*;
pub use fs_search::*;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
}

impl IdeSshSpec {
    pub fn effective_port(&self) -> Option<u16> {
        self.port.filter(|&p| p > 0)
    }

    /// Validate SSH parameters, expand a leading `~/` on `identity_file`, and
    /// construct a validated [`otto_ssh::SshTunnelConfig`].
    pub fn to_ssh_tunnel_config(&self) -> Result<otto_ssh::SshTunnelConfig> {
        crate::target::ssh_exec::validate_ssh_spec(self)?;
        let identity_file = match self.identity_file.as_deref().map(str::trim) {
            Some(s) if !s.is_empty() => Some(
                otto_core::paths::expand_home_strict(s)
                    .map_err(Error::Invalid)?
                    .to_string_lossy()
                    .into_owned(),
            ),
            _ => None,
        };
        let cfg = otto_ssh::SshTunnelConfig {
            host: self.host.clone(),
            port: self.effective_port().unwrap_or(22),
            user: self.user.clone(),
            identity_file,
            proxy_jump: self.proxy_jump.clone(),
        };
        cfg.validate()?;
        Ok(cfg)
    }

    /// Returns `true` when the SSH connection's environment or name targets production.
    pub fn is_production(&self) -> bool {
        is_production_label(self.environment.as_deref(), &self.name)
    }
}

/// Returns `true` if `env` is `"prod"` / `"production"` or `name` has a whole-word
/// `prod` / `production` token (e.g. `"prod-eks"`, `"Prod Bastion"`).
pub fn is_production_label(env: Option<&str>, name: &str) -> bool {
    if let Some(e) = env.map(str::trim) {
        if e.eq_ignore_ascii_case("prod") || e.eq_ignore_ascii_case("production") {
            return true;
        }
    }
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .any(|tok| tok.eq_ignore_ascii_case("prod") || tok.eq_ignore_ascii_case("production"))
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
        let root_key = root_equiv_key(self.kind, &self.root);
        let mut out: Vec<String> = Vec::new();
        for raw in self
            .additional_roots
            .iter()
            .chain(self.project_folders.iter())
        {
            let clean = trim_root_slash(raw);
            if clean.is_empty() {
                continue;
            }
            let cand_key = root_equiv_key(self.kind, &clean);
            if cand_key == root_key {
                continue;
            }
            if !out
                .iter()
                .any(|existing| root_equiv_key(self.kind, existing) == cand_key)
            {
                out.push(clean);
            }
        }
        out
    }

    /// Merge extra project folders into `additional_roots` in-place.
    pub fn absorb_project_folders(&mut self, extra: &[String]) {
        self.root = trim_root_slash(&self.root);
        for raw in extra {
            let clean = trim_root_slash(raw);
            if clean.is_empty() {
                continue;
            }
            let cand_key = root_equiv_key(self.kind, &clean);
            if !self
                .project_folders
                .iter()
                .any(|x| root_equiv_key(self.kind, x) == cand_key)
            {
                self.project_folders.push(clean);
            }
        }
        let deduped = self.merged_extra_roots();
        self.project_folders.retain(|x| {
            let k = root_equiv_key(self.kind, x);
            k != root_equiv_key(self.kind, &self.root)
        });
        self.additional_roots = deduped;
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
        if self.kind == IdeTargetKind::Local {
            let rt = self.root.trim();
            let is_win_drive = rt.len() >= 3
                && rt.as_bytes()[0].is_ascii_alphabetic()
                && rt.as_bytes()[1] == b':'
                && matches!(rt.as_bytes()[2], b'/' | b'\\');
            let is_valid_local_prefix = rt.starts_with('/')
                || rt.starts_with('~')
                || rt.get(..5).is_some_and(|p| p.eq_ignore_ascii_case("file:"))
                || is_win_drive;
            if !is_valid_local_prefix {
                return Err(Error::Invalid(
                    "local target root must be an absolute or ~/ path".into(),
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
// Shared language detection helper
// ---------------------------------------------------------------------------

pub(crate) fn trim_root_slash(raw: &str) -> String {
    let t = raw.trim();
    if t.is_empty() {
        return String::new();
    }
    let stripped = t.trim_end_matches('/');
    if stripped.is_empty() && t.starts_with('/') {
        "/".to_string()
    } else {
        stripped.to_string()
    }
}

pub(crate) fn root_equiv_key(kind: IdeTargetKind, raw: &str) -> String {
    let clean = trim_root_slash(raw);
    if kind != IdeTargetKind::Local {
        return crate::target::normalize_posix_clean_root(&clean);
    }
    let expanded = trim_root_slash(&otto_core::paths::expand_tilde(&clean));
    if expanded == "/private/var" {
        return "/var".to_string();
    }
    if let Some(rest) = expanded.strip_prefix("/private/var/") {
        return format!("/var/{rest}");
    }
    if expanded == "/private/tmp" {
        return "/tmp".to_string();
    }
    if let Some(rest) = expanded.strip_prefix("/private/tmp/") {
        return format!("/tmp/{rest}");
    }
    if expanded == "/private/etc" {
        return "/etc".to_string();
    }
    if let Some(rest) = expanded.strip_prefix("/private/etc/") {
        return format!("/etc/{rest}");
    }
    expanded
}

/// Canonical file-path to IDE language identifier mapping.
pub fn detect_language(path: &str) -> Option<&'static str> {
    let trimmed = path.trim();
    if trimmed
        .get(..6)
        .is_some_and(|p| p.eq_ignore_ascii_case("jdt://"))
    {
        return Some("java");
    }
    let has_uri_scheme = trimmed
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("file://"))
        || trimmed
            .get(..4)
            .is_some_and(|p| p.eq_ignore_ascii_case("jar:"))
        || trimmed
            .get(..8)
            .is_some_and(|p| p.eq_ignore_ascii_case("zipfile:"))
        || trimmed
            .get(..6)
            .is_some_and(|p| p.eq_ignore_ascii_case("ssh://"));
    let clean = if has_uri_scheme {
        trimmed.split(['?', '#']).next().unwrap_or(trimmed)
    } else {
        path
    };
    let base = clean.rsplit('/').next().unwrap_or(clean);
    match base {
        "Makefile" | "GNUmakefile" | "makefile" => return Some("makefile"),
        "Dockerfile" => return Some("dockerfile"),
        "Podfile" | "Fastfile" | "Appfile" | "Matchfile" => return Some("ruby"),
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
        "dart" => "dart",
        "m" | "mm" => "objc",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hxx" => "cpp",
        "cs" => "csharp",
        "rb" | "podspec" => "ruby",
        "php" => "php",
        "sql" => "sql",
        "json" | "jsonc" | "arb" => "json",
        "jsonl" | "ndjson" => "jsonl",
        "csv" => "csv",
        "tsv" => "tsv",
        "yml" | "yaml" => "yaml",
        "toml" => "toml",
        "html" | "htm" => "html",
        "svelte" => "svelte",
        "vue" => "vue",
        "xml" | "svg" | "plist" | "storyboard" | "xib" => "xml",
        "css" => "css",
        "scss" | "less" => "scss",
        "md" | "markdown" => "markdown",
        "sh" | "bash" | "zsh" | "ksh" | "bats" => "shell",
        "proto" => "protobuf",
        "graphql" | "gql" => "graphql",
        "gradle" => "groovy",
        "xcconfig" => "ini",
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
    fn hover_and_lsp_requests_accept_both_character_and_column_fields() {
        let hover_both: IdeHoverReq =
            serde_json::from_str(r#"{"path":"main.go","line":10,"character":0,"column":14}"#)
                .expect("hover with both character and column must deserialize");
        assert_eq!(hover_both.effective_character(), 14);

        let hover_char: IdeHoverReq =
            serde_json::from_str(r#"{"path":"main.go","line":10,"character":7,"column":14}"#)
                .expect("hover with positive character");
        assert_eq!(hover_char.effective_character(), 7);

        let refs_both: IdeReferencesReq = serde_json::from_str(
            r#"{"target":{"root":"/repo"},"path":"main.go","line":5,"character":9,"column":0}"#,
        )
        .expect("references with both character and column must deserialize");
        assert_eq!(refs_both.effective_column(), 9);

        let rename_both: IdeRenameReq = serde_json::from_str(
            r#"{"target":{"root":"/repo"},"path":"main.go","line":5,"character":11,"column":11,"new_name":"next"}"#,
        )
        .expect("rename with both character and column must deserialize");
        assert_eq!(rename_both.effective_column(), 11);
        assert!(rename_both.local_history_enabled);
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

    #[test]
    fn local_history_alias_record_local_history_and_ssh_tilde_expansion() {
        let w1: IdeFileWriteReq = serde_json::from_str(
            r#"{"target":{"root":"/repo"},"path":"a.rs","content":"x","record_local_history":false}"#,
        )
        .unwrap();
        assert!(!w1.local_history_enabled);

        let w2: IdeFileWriteReq = serde_json::from_str(
            r#"{"target":{"root":"/repo"},"path":"a.rs","content":"x","local_history_enabled":false}"#,
        )
        .unwrap();
        assert!(!w2.local_history_enabled);

        let w3: IdeFileWriteReq =
            serde_json::from_str(r#"{"target":{"root":"/repo"},"path":"a.rs","content":"x"}"#)
                .unwrap();
        assert!(w3.local_history_enabled);

        let m1: IdeFsMutateReq = serde_json::from_str(
            r#"{"target":{"root":"/repo"},"op":"delete","path":"a.rs","record_local_history":false}"#,
        )
        .unwrap();
        assert!(!m1.local_history_enabled);

        let r1: IdeRenameReq = serde_json::from_str(
            r#"{"target":{"root":"/repo"},"path":"a.rs","line":1,"new_name":"b","record_local_history":false}"#,
        )
        .unwrap();
        assert!(!r1.local_history_enabled);

        let s1: IdeHistorySnapshotReq =
            serde_json::from_str(r#"{"root":"/repo","path":"a.rs","record_local_history":false}"#)
                .unwrap();
        assert!(!s1.local_history_enabled);

        let spec = IdeSshSpec {
            connection_id: "c1".into(),
            name: "Prod Bastion".into(),
            host: "10.0.0.5".into(),
            port: Some(2222),
            user: "ops".into(),
            identity_file: Some("~/.ssh/id_ed25519".into()),
            proxy_jump: None,
            environment: Some("dev".into()),
        };
        assert!(spec.is_production());
        let tcfg = spec.to_ssh_tunnel_config().unwrap();
        assert_eq!(tcfg.port, 2222);
        assert!(
            !tcfg
                .identity_file
                .as_deref()
                .unwrap_or_default()
                .starts_with('~'),
            "identity_file tilde must be expanded"
        );
    }
}
