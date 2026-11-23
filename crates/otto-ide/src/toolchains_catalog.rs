//! Architecture-aware (`macos`/`linux` × `aarch64`/`x86_64`) SDK, LSP, and DAP
//! version catalogs for Java, Go, Rust, Node.js (JS/TS), Python, Shell, and JSON.
//!
//! Combines live vendor HTTPS metadata fetchers (`api.adoptium.net`,
//! `go.dev/dl/?mode=json&include=all`, `static.rust-lang.org`,
//! `nodejs.org/dist/index.json`, and Astral `uv` releases) with an in-memory
//! TTL cache, SHA-256 archive checksums where published by upstream vendors,
//! and zero-HTTPS `<1ms` curated catalogs for instant IDE startup and offline
//! or isolated E2E runs.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const CACHE_TTL: Duration = Duration::from_secs(900);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_millis(1800);
const HTTP_READ_TIMEOUT: Duration = Duration::from_millis(2800);
const MAX_GO_CATALOG_VERSIONS: usize = 40;

pub const JDTLS_TARBALL_URL: &str =
    "https://download.eclipse.org/jdtls/milestones/1.40.0/jdt-language-server-1.40.0-202409261450.tar.gz";
pub const JAVA_DEBUG_JAR_URL: &str =
    "https://repo1.maven.org/maven2/com/microsoft/java/com.microsoft.java.debug.plugin/0.53.1/com.microsoft.java.debug.plugin-0.53.1.jar";
pub const JAVA_DEBUG_JAR_NAME: &str = "com.microsoft.java.debug.plugin-0.53.1.jar";
pub const GOPLS_PKG: &str = "golang.org/x/tools/gopls@latest";
pub const DLV_PKG: &str = "github.com/go-delve/delve/cmd/dlv@latest";
pub const JS_DEBUG_TARBALL_URL: &str =
    "https://github.com/microsoft/vscode-js-debug/releases/download/v1.97.1/js-debug-dap-v1.97.1.tar.gz";
pub const TS_LSP_NPM_PACKAGES: &[&str] = &["typescript", "typescript-language-server"];
pub const BASH_LSP_NPM_PACKAGES: &[&str] = &["bash-language-server"];
pub const JSON_LSP_NPM_PACKAGES: &[&str] = &["vscode-langservers-extracted"];
pub const PYRIGHT_PKG: &str = "pyright";
pub const DEBUGPY_PKG: &str = "debugpy";

/// Normalized target operating system (`macos` or `linux`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetOs {
    Macos,
    Linux,
}

impl TargetOs {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "darwin" | "macos" | "mac" | "osx" => Some(Self::Macos),
            "linux" | "gnu/linux" => Some(Self::Linux),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Macos => "macos",
            Self::Linux => "linux",
        }
    }

    pub const fn adoptium_os(self) -> &'static str {
        match self {
            Self::Macos => "mac",
            Self::Linux => "linux",
        }
    }

    pub const fn go_os(self) -> &'static str {
        match self {
            Self::Macos => "darwin",
            Self::Linux => "linux",
        }
    }

    pub const fn node_os(self) -> &'static str {
        match self {
            Self::Macos => "darwin",
            Self::Linux => "linux",
        }
    }
}

/// Normalized target CPU architecture (`aarch64` or `x86_64`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TargetArch {
    Aarch64,
    X86_64,
}

impl TargetArch {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "arm64" | "aarch64" | "arm64e" => Some(Self::Aarch64),
            "x86_64" | "amd64" | "x64" => Some(Self::X86_64),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Aarch64 => "aarch64",
            Self::X86_64 => "x86_64",
        }
    }

    pub const fn adoptium_arch(self) -> &'static str {
        match self {
            Self::Aarch64 => "aarch64",
            Self::X86_64 => "x64",
        }
    }

    pub const fn go_arch(self) -> &'static str {
        match self {
            Self::Aarch64 => "arm64",
            Self::X86_64 => "amd64",
        }
    }

    pub const fn node_arch(self) -> &'static str {
        match self {
            Self::Aarch64 => "arm64",
            Self::X86_64 => "x64",
        }
    }
}

/// Concrete `(os, arch)` pair of either the local host or a remote SSH host.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TargetPlatform {
    pub os: TargetOs,
    pub arch: TargetArch,
}

impl TargetPlatform {
    pub fn host() -> Self {
        let os = TargetOs::parse(std::env::consts::OS).unwrap_or(TargetOs::Linux);
        let arch = TargetArch::parse(std::env::consts::ARCH).unwrap_or(TargetArch::X86_64);
        Self { os, arch }
    }

    pub fn from_uname(sysname: &str, machine: &str) -> Option<Self> {
        Some(Self {
            os: TargetOs::parse(sysname)?,
            arch: TargetArch::parse(machine)?,
        })
    }

    pub fn from_combined_uname(line: &str) -> Option<Self> {
        let mut parts = line.split_whitespace();
        let sysname = parts.next()?;
        let machine = parts.next()?;
        Self::from_uname(sysname, machine)
    }

    pub const fn rust_triple(self) -> &'static str {
        match (self.os, self.arch) {
            (TargetOs::Macos, TargetArch::Aarch64) => "aarch64-apple-darwin",
            (TargetOs::Macos, TargetArch::X86_64) => "x86_64-apple-darwin",
            (TargetOs::Linux, TargetArch::Aarch64) => "aarch64-unknown-linux-gnu",
            (TargetOs::Linux, TargetArch::X86_64) => "x86_64-unknown-linux-gnu",
        }
    }

    pub const fn codelldb_slug(self) -> &'static str {
        match (self.os, self.arch) {
            (TargetOs::Macos, TargetArch::Aarch64) => "darwin-arm64",
            (TargetOs::Macos, TargetArch::X86_64) => "darwin-x64",
            (TargetOs::Linux, TargetArch::Aarch64) => "linux-arm64",
            (TargetOs::Linux, TargetArch::X86_64) => "linux-x64",
        }
    }

    pub const fn node_dist_file_token(self) -> &'static str {
        match (self.os, self.arch) {
            (TargetOs::Macos, TargetArch::Aarch64) => "osx-arm64-tar",
            (TargetOs::Macos, TargetArch::X86_64) => "osx-x64-tar",
            (TargetOs::Linux, TargetArch::Aarch64) => "linux-arm64",
            (TargetOs::Linux, TargetArch::X86_64) => "linux-x64",
        }
    }
}

/// First-class supported language toolchain ecosystems.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EcosystemKind {
    Java,
    Go,
    Rust,
    Node,
    Python,
    Shell,
    Json,
    Ios,
    Android,
    Flutter,
}

impl EcosystemKind {
    pub const ALL: [Self; 10] = [
        Self::Java,
        Self::Go,
        Self::Rust,
        Self::Node,
        Self::Python,
        Self::Shell,
        Self::Json,
        Self::Ios,
        Self::Android,
        Self::Flutter,
    ];

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "java" | "jdk" => Some(Self::Java),
            "go" | "golang" => Some(Self::Go),
            "rust" | "cargo" => Some(Self::Rust),
            "node" | "nodejs" | "javascript" | "typescript" | "js" | "ts" => Some(Self::Node),
            "python" | "py" | "uv" => Some(Self::Python),
            "shell" | "bash" | "sh" | "zsh" => Some(Self::Shell),
            "json" | "jsonc" | "jsonl" => Some(Self::Json),
            "ios" | "xcode" | "swift" => Some(Self::Ios),
            "android" | "android_sdk" => Some(Self::Android),
            "flutter" | "dart" => Some(Self::Flutter),
            _ => None,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Java => "java",
            Self::Go => "go",
            Self::Rust => "rust",
            Self::Node => "node",
            Self::Python => "python",
            Self::Shell => "shell",
            Self::Json => "json",
            Self::Ios => "ios",
            Self::Android => "android",
            Self::Flutter => "flutter",
        }
    }

    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Java => "Java (Temurin JDK)",
            Self::Go => "Go",
            Self::Rust => "Rust",
            Self::Node => "Node.js (JS / TS)",
            Self::Python => "Python (uv)",
            Self::Shell => "Shell (Bash / Zsh)",
            Self::Json => "JSON / JSONC / JSONL",
            Self::Ios => "iOS / Xcode",
            Self::Android => "Android SDK",
            Self::Flutter => "Flutter / Dart",
        }
    }

    pub const fn lsp_name(self) -> &'static str {
        match self {
            Self::Java => "jdtls",
            Self::Go => "gopls",
            Self::Rust => "rust-analyzer",
            Self::Node => "typescript-language-server",
            Self::Python => "pyright",
            Self::Shell => "bash-language-server",
            Self::Json => "vscode-json-language-server",
            Self::Ios => "sourcekit-lsp",
            Self::Android => "kotlin-language-server",
            Self::Flutter => "dart",
        }
    }

    pub const fn dap_name(self) -> &'static str {
        match self {
            Self::Java => "java-debug",
            Self::Go => "dlv",
            Self::Rust => "codelldb",
            Self::Node => "js-debug",
            Self::Python => "debugpy",
            Self::Shell => "bashdb",
            Self::Json => "jq",
            Self::Ios => "lldb-dap",
            Self::Android => "adb",
            Self::Flutter => "flutter",
        }
    }

    pub const fn default_sdk_version(self) -> &'static str {
        match self {
            Self::Java => "21",
            Self::Go => "1.24.2",
            Self::Rust => "stable",
            Self::Node => "22.14.0",
            Self::Python => "3.13",
            Self::Shell => "3.10.0",
            Self::Json => "1.7.1",
            Self::Ios => "xcode",
            Self::Android => "sdk",
            Self::Flutter => "stable",
        }
    }
}

/// Downloadable architecture-matched SDK descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogSdkEntry {
    pub version: String,
    pub label: String,
    pub lts: bool,
    pub os: String,
    pub arch: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

impl From<crate::types::IdeEcosystemKind> for EcosystemKind {
    fn from(k: crate::types::IdeEcosystemKind) -> Self {
        match k {
            crate::types::IdeEcosystemKind::Java => Self::Java,
            crate::types::IdeEcosystemKind::Go => Self::Go,
            crate::types::IdeEcosystemKind::Rust => Self::Rust,
            crate::types::IdeEcosystemKind::Node => Self::Node,
            crate::types::IdeEcosystemKind::Python => Self::Python,
            crate::types::IdeEcosystemKind::Shell => Self::Shell,
            crate::types::IdeEcosystemKind::Json => Self::Json,
            crate::types::IdeEcosystemKind::Ios => Self::Ios,
            crate::types::IdeEcosystemKind::Android => Self::Android,
            crate::types::IdeEcosystemKind::Flutter => Self::Flutter,
        }
    }
}

impl From<EcosystemKind> for crate::types::IdeEcosystemKind {
    fn from(k: EcosystemKind) -> Self {
        match k {
            EcosystemKind::Java => Self::Java,
            EcosystemKind::Go => Self::Go,
            EcosystemKind::Rust => Self::Rust,
            EcosystemKind::Node => Self::Node,
            EcosystemKind::Python => Self::Python,
            EcosystemKind::Shell => Self::Shell,
            EcosystemKind::Json => Self::Json,
            EcosystemKind::Ios => Self::Ios,
            EcosystemKind::Android => Self::Android,
            EcosystemKind::Flutter => Self::Flutter,
        }
    }
}

impl From<CatalogSdkEntry> for crate::types::IdeDownloadableVersion {
    fn from(e: CatalogSdkEntry) -> Self {
        Self {
            version: e.version,
            label: e.label,
            lts: e.lts,
            os: e.os,
            arch: e.arch,
            url: e.url,
            size_hint: e.size_hint,
            sha256: e.sha256,
        }
    }
}

/// Validate that a version identifier contains only safe version characters
/// (never path traversal, leading `-` CLI flags, or shell metacharacters).
pub fn is_safe_version_token(version: &str) -> bool {
    let v = version.trim();
    !v.is_empty()
        && v.len() <= 64
        && !v.starts_with('-')
        && !v.contains("..")
        && v.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
}

pub fn temurin_jdk_tarball_url(feature_version: u32, platform: TargetPlatform) -> String {
    format!(
        "https://api.adoptium.net/v3/binary/latest/{feature_version}/ga/{}/{}/jdk/hotspot/normal/eclipse?project=jdk",
        platform.os.adoptium_os(),
        platform.arch.adoptium_arch()
    )
}

pub fn go_tarball_url(version: &str, platform: TargetPlatform) -> String {
    let bare = version.trim().trim_start_matches("go");
    format!(
        "https://go.dev/dl/go{bare}.{}-{}.tar.gz",
        platform.os.go_os(),
        platform.arch.go_arch()
    )
}

pub fn rustup_init_url(platform: TargetPlatform) -> String {
    format!(
        "https://static.rust-lang.org/rustup/dist/{}/rustup-init",
        platform.rust_triple()
    )
}

pub fn rust_analyzer_gz_url(platform: TargetPlatform) -> String {
    format!(
        "https://github.com/rust-lang/rust-analyzer/releases/latest/download/rust-analyzer-{}.gz",
        platform.rust_triple()
    )
}

pub fn codelldb_vsix_url(platform: TargetPlatform) -> String {
    format!(
        "https://github.com/vadimcn/codelldb/releases/download/v1.11.4/codelldb-{}.vsix",
        platform.codelldb_slug()
    )
}

pub fn node_tarball_url(version: &str, platform: TargetPlatform) -> String {
    let bare = version.trim().trim_start_matches('v');
    format!(
        "https://nodejs.org/dist/v{bare}/node-v{bare}-{}-{}.tar.gz",
        platform.os.node_os(),
        platform.arch.node_arch()
    )
}

pub fn uv_tarball_url(platform: TargetPlatform) -> String {
    format!(
        "https://github.com/astral-sh/uv/releases/latest/download/uv-{}.tar.gz",
        platform.rust_triple()
    )
}

pub fn shfmt_binary_url(version: &str, platform: TargetPlatform) -> String {
    let clean = version.trim().trim_start_matches('v');
    format!(
        "https://github.com/mvdan/sh/releases/download/v{clean}/shfmt_v{clean}_{}_{}",
        platform.os.go_os(),
        platform.arch.go_arch()
    )
}

pub fn jq_binary_url(version: &str, platform: TargetPlatform) -> String {
    let clean = version.trim().trim_start_matches("jq-");
    let slug = match (platform.os, platform.arch) {
        (TargetOs::Macos, TargetArch::Aarch64) => "macos-arm64",
        (TargetOs::Macos, TargetArch::X86_64) => "macos-amd64",
        (TargetOs::Linux, TargetArch::Aarch64) => "linux-arm64",
        (TargetOs::Linux, TargetArch::X86_64) => "linux-amd64",
    };
    format!("https://github.com/jqlang/jq/releases/download/jq-{clean}/jq-{slug}")
}

fn format_mb(bytes: u64) -> String {
    let mb = (bytes as f64) / (1024.0 * 1024.0);
    format!("{mb:.0} MB")
}

fn normalize_sha256(raw: Option<&str>) -> Option<String> {
    let s = raw?.trim();
    (s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}

/// Curated offline/airgapped catalog tailored to `(os, arch)`.
pub fn curated_catalog(ecosystem: EcosystemKind, platform: TargetPlatform) -> Vec<CatalogSdkEntry> {
    let os = platform.os.as_str().to_string();
    let arch = platform.arch.as_str().to_string();
    match ecosystem {
        EcosystemKind::Java => [
            (25, false, "208 MB"),
            (21, true, "198 MB"),
            (17, true, "192 MB"),
            (11, true, "186 MB"),
            (8, true, "112 MB"),
        ]
        .into_iter()
        .map(|(ver, lts, sz)| CatalogSdkEntry {
            version: ver.to_string(),
            label: if lts {
                format!("Eclipse Temurin JDK {ver} LTS")
            } else {
                format!("Eclipse Temurin JDK {ver}")
            },
            lts,
            os: os.clone(),
            arch: arch.clone(),
            url: temurin_jdk_tarball_url(ver, platform),
            size_hint: Some(sz.to_string()),
            sha256: None,
        })
        .collect(),
        EcosystemKind::Go => [
            ("1.24.2", true, "73 MB"),
            ("1.24.1", true, "72 MB"),
            ("1.23.8", true, "70 MB"),
            ("1.23.7", false, "70 MB"),
            ("1.22.12", false, "67 MB"),
            ("1.21.13", false, "65 MB"),
        ]
        .into_iter()
        .map(|(ver, lts, sz)| CatalogSdkEntry {
            version: ver.to_string(),
            label: format!("Go {ver}"),
            lts,
            os: os.clone(),
            arch: arch.clone(),
            url: go_tarball_url(ver, platform),
            size_hint: Some(sz.to_string()),
            sha256: None,
        })
        .collect(),
        EcosystemKind::Rust => [
            ("stable", "Rust stable (rustup)", true),
            ("beta", "Rust beta (rustup)", false),
            ("nightly", "Rust nightly (rustup)", false),
        ]
        .into_iter()
        .map(|(ver, label, lts)| CatalogSdkEntry {
            version: ver.to_string(),
            label: label.to_string(),
            lts,
            os: os.clone(),
            arch: arch.clone(),
            url: rustup_init_url(platform),
            size_hint: Some("180 MB".to_string()),
            sha256: None,
        })
        .collect(),
        EcosystemKind::Node => [
            ("24.0.0", "Node.js 24.0.0 (Current)", false, "44 MB"),
            ("22.14.0", "Node.js 22.14.0 LTS (Jod)", true, "42 MB"),
            ("20.19.0", "Node.js 20.19.0 LTS (Iron)", true, "40 MB"),
            ("18.20.7", "Node.js 18.20.7 LTS (Hydrogen)", true, "39 MB"),
        ]
        .into_iter()
        .map(|(ver, label, lts, sz)| CatalogSdkEntry {
            version: ver.to_string(),
            label: label.to_string(),
            lts,
            os: os.clone(),
            arch: arch.clone(),
            url: node_tarball_url(ver, platform),
            size_hint: Some(sz.to_string()),
            sha256: None,
        })
        .collect(),
        EcosystemKind::Python => [
            ("3.13", "Python 3.13 (Astral uv managed)", true, "34 MB"),
            ("3.12", "Python 3.12 (Astral uv managed)", true, "32 MB"),
            ("3.11", "Python 3.11 (Astral uv managed)", false, "31 MB"),
            ("3.10", "Python 3.10 (Astral uv managed)", false, "30 MB"),
        ]
        .into_iter()
        .map(|(ver, label, lts, sz)| CatalogSdkEntry {
            version: ver.to_string(),
            label: label.to_string(),
            lts,
            os: os.clone(),
            arch: arch.clone(),
            url: uv_tarball_url(platform),
            size_hint: Some(sz.to_string()),
            sha256: None,
        })
        .collect(),
        EcosystemKind::Shell => vec![CatalogSdkEntry {
            version: "3.10.0".to_string(),
            label: "shfmt 3.10.0 (Shell formatter & parser)".to_string(),
            lts: true,
            os,
            arch,
            url: shfmt_binary_url("3.10.0", platform),
            size_hint: Some("4 MB".to_string()),
            sha256: None,
        }],
        EcosystemKind::Json => vec![CatalogSdkEntry {
            version: "1.7.1".to_string(),
            label: "jq 1.7.1 (JSON / JSONL processor)".to_string(),
            lts: true,
            os,
            arch,
            url: jq_binary_url("1.7.1", platform),
            size_hint: Some("3 MB".to_string()),
            sha256: None,
        }],
        EcosystemKind::Ios | EcosystemKind::Android | EcosystemKind::Flutter => Vec::new(),
    }
}

/// Resolve the exact download entry for `(ecosystem, requested_version, platform)`,
/// accepting either an exact catalog match or a safe explicit version string.
pub fn resolve_sdk_download(
    ecosystem: EcosystemKind,
    requested_version: Option<&str>,
    platform: TargetPlatform,
    catalog: &[CatalogSdkEntry],
) -> Result<CatalogSdkEntry, String> {
    let ver = requested_version
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ecosystem.default_sdk_version());
    if !is_safe_version_token(ver) {
        return Err(format!("invalid toolchain version '{ver}'"));
    }
    if let Some(hit) = catalog.iter().find(|e| {
        e.version == ver
            || e.version.trim_start_matches('v') == ver.trim_start_matches('v')
            || e.version.trim_start_matches("go") == ver.trim_start_matches("go")
    }) {
        return Ok(hit.clone());
    }
    // Also support prefix match for major/minor tracks like "22" -> "22.14.0" or "1.24" -> "1.24.2".
    if let Some(hit) = catalog
        .iter()
        .find(|e| e.version.starts_with(&format!("{ver}.")))
    {
        return Ok(hit.clone());
    }

    let os = platform.os.as_str().to_string();
    let arch = platform.arch.as_str().to_string();
    match ecosystem {
        EcosystemKind::Java => {
            let feature: u32 = ver
                .split('.')
                .next()
                .and_then(|s| s.parse().ok())
                .filter(|&n| (8..=30).contains(&n))
                .ok_or_else(|| format!("unsupported Java feature version '{ver}'"))?;
            Ok(CatalogSdkEntry {
                version: feature.to_string(),
                label: format!("Eclipse Temurin JDK {feature}"),
                lts: matches!(feature, 8 | 11 | 17 | 21 | 25),
                os,
                arch,
                url: temurin_jdk_tarball_url(feature, platform),
                size_hint: None,
                sha256: None,
            })
        }
        EcosystemKind::Go => {
            let clean = ver.trim_start_matches("go").to_string();
            Ok(CatalogSdkEntry {
                version: clean.clone(),
                label: format!("Go {clean}"),
                lts: true,
                os,
                arch,
                url: go_tarball_url(&clean, platform),
                size_hint: None,
                sha256: None,
            })
        }
        EcosystemKind::Rust => Ok(CatalogSdkEntry {
            version: ver.to_string(),
            label: format!("Rust {ver} (rustup)"),
            lts: ver == "stable",
            os,
            arch,
            url: rustup_init_url(platform),
            size_hint: None,
            sha256: None,
        }),
        EcosystemKind::Node => {
            let clean = ver.trim_start_matches('v').to_string();
            Ok(CatalogSdkEntry {
                version: clean.clone(),
                label: format!("Node.js {clean}"),
                lts: clean.starts_with("18.")
                    || clean.starts_with("20.")
                    || clean.starts_with("22.")
                    || clean.starts_with("24."),
                os,
                arch,
                url: node_tarball_url(&clean, platform),
                size_hint: None,
                sha256: None,
            })
        }
        EcosystemKind::Python => Ok(CatalogSdkEntry {
            version: ver.to_string(),
            label: format!("Python {ver} (Astral uv managed)"),
            lts: true,
            os,
            arch,
            url: uv_tarball_url(platform),
            size_hint: None,
            sha256: None,
        }),
        EcosystemKind::Shell => {
            let clean = ver.trim_start_matches('v').to_string();
            Ok(CatalogSdkEntry {
                version: clean.clone(),
                label: format!("shfmt {clean}"),
                lts: true,
                os,
                arch,
                url: shfmt_binary_url(&clean, platform),
                size_hint: Some("4 MB".to_string()),
                sha256: None,
            })
        }
        EcosystemKind::Json => {
            let clean = ver.trim_start_matches("jq-").to_string();
            Ok(CatalogSdkEntry {
                version: clean.clone(),
                label: format!("jq {clean}"),
                lts: true,
                os,
                arch,
                url: jq_binary_url(&clean, platform),
                size_hint: Some("3 MB".to_string()),
                sha256: None,
            })
        }
        EcosystemKind::Ios => Err(
            "Xcode / iOS Simulator is installed from the Mac App Store (not downloadable as a tarball)".into(),
        ),
        EcosystemKind::Android => Err(
            "Android SDK is installed via Android Studio or Homebrew (`android-commandlinetools`)".into(),
        ),
        EcosystemKind::Flutter => Err(
            "Flutter SDK is installed via https://docs.flutter.dev or `brew install --cask flutter`".into(),
        ),
    }
}

// ── Live Vendor Metadata JSON Parsers ────────────────────────────────────────

pub fn parse_adoptium_release_json(
    feature_version: u32,
    raw: &str,
    platform: TargetPlatform,
) -> Option<CatalogSdkEntry> {
    let arr: serde_json::Value = serde_json::from_str(raw).ok()?;
    let first = arr.as_array()?.first()?;
    let semver = first
        .pointer("/version_data/semver")
        .and_then(|v| v.as_str())
        .or_else(|| first.get("release_name").and_then(|v| v.as_str()))?;
    let pkg = first.pointer("/binaries/0/package")?;
    let link = pkg
        .get("link")
        .and_then(|v| v.as_str())
        .filter(|u| u.starts_with("https://"))?;
    let size_hint = pkg.get("size").and_then(|v| v.as_u64()).map(format_mb);
    let sha256 = normalize_sha256(pkg.get("checksum").and_then(|v| v.as_str()));
    let lts = matches!(feature_version, 8 | 11 | 17 | 21);
    Some(CatalogSdkEntry {
        version: feature_version.to_string(),
        label: if lts {
            format!("Eclipse Temurin JDK {feature_version} LTS ({semver})")
        } else {
            format!("Eclipse Temurin JDK {feature_version} ({semver})")
        },
        lts,
        os: platform.os.as_str().to_string(),
        arch: platform.arch.as_str().to_string(),
        url: link.to_string(),
        size_hint,
        sha256,
    })
}

pub fn parse_go_dl_json(raw: &str, platform: TargetPlatform) -> Vec<CatalogSdkEntry> {
    let Ok(releases) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for rel in releases {
        if rel.get("stable").and_then(|v| v.as_bool()) == Some(false) {
            continue;
        }
        let Some(ver_raw) = rel.get("version").and_then(|v| v.as_str()) else {
            continue;
        };
        let clean_ver = ver_raw.trim_start_matches("go");
        if !is_safe_version_token(clean_ver) {
            continue;
        }
        let Some(files) = rel.get("files").and_then(|v| v.as_array()) else {
            continue;
        };
        for f in files {
            let matches_os = f.get("os").and_then(|v| v.as_str()) == Some(platform.os.go_os());
            let matches_arch =
                f.get("arch").and_then(|v| v.as_str()) == Some(platform.arch.go_arch());
            let is_archive = f.get("kind").and_then(|v| v.as_str()) == Some("archive");
            if matches_os && matches_arch && is_archive {
                if let Some(filename) = f.get("filename").and_then(|v| v.as_str()) {
                    let size_hint = f.get("size").and_then(|v| v.as_u64()).map(format_mb);
                    let sha256 = normalize_sha256(f.get("sha256").and_then(|v| v.as_str()));
                    out.push(CatalogSdkEntry {
                        version: clean_ver.to_string(),
                        label: format!("Go {clean_ver}"),
                        lts: out.len() < 2,
                        os: platform.os.as_str().to_string(),
                        arch: platform.arch.as_str().to_string(),
                        url: format!("https://go.dev/dl/{filename}"),
                        size_hint,
                        sha256,
                    });
                    break;
                }
            }
        }
        if out.len() >= MAX_GO_CATALOG_VERSIONS {
            break;
        }
    }
    out
}

pub fn parse_rust_channel_toml(raw: &str, platform: TargetPlatform) -> Vec<CatalogSdkEntry> {
    let mut stable_ver: Option<String> = None;
    let mut in_pkg_rust = false;
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_pkg_rust = trimmed == "[pkg.rust]";
            continue;
        }
        if in_pkg_rust && trimmed.starts_with("version") {
            if let Some((_, rhs)) = trimmed.split_once('=') {
                let unquoted = rhs.trim().trim_matches('"');
                if let Some(num) = unquoted.split_whitespace().next() {
                    if is_safe_version_token(num) {
                        stable_ver = Some(num.to_string());
                        break;
                    }
                }
            }
        }
    }
    let mut entries = curated_catalog(EcosystemKind::Rust, platform);
    if let Some(v) = stable_ver {
        if let Some(first) = entries.first_mut() {
            first.label = format!("Rust stable ({v} via rustup)");
        }
    }
    entries
}

pub fn parse_node_dist_json(raw: &str, platform: TargetPlatform) -> Vec<CatalogSdkEntry> {
    let Ok(items) = serde_json::from_str::<Vec<serde_json::Value>>(raw) else {
        return Vec::new();
    };
    let wanted_majors = ["24", "22", "20", "18"];
    let file_token = platform.node_dist_file_token();
    let mut out = Vec::new();
    for major in wanted_majors {
        let prefix = format!("v{major}.");
        for item in &items {
            let Some(ver_raw) = item.get("version").and_then(|v| v.as_str()) else {
                continue;
            };
            if !ver_raw.starts_with(&prefix) {
                continue;
            }
            let has_file = item
                .get("files")
                .and_then(|v| v.as_array())
                .is_some_and(|arr| arr.iter().any(|x| x.as_str() == Some(file_token)));
            if !has_file {
                continue;
            }
            let clean = ver_raw.trim_start_matches('v');
            if !is_safe_version_token(clean) {
                continue;
            }
            let lts_val = item.get("lts");
            let lts_name = lts_val.and_then(|v| v.as_str());
            let is_lts = lts_name.is_some() || lts_val.and_then(|v| v.as_bool()) == Some(true);
            let label = match lts_name {
                Some(codename) => format!("Node.js {clean} LTS ({codename})"),
                None if is_lts => format!("Node.js {clean} LTS"),
                None => format!("Node.js {clean} (Current)"),
            };
            out.push(CatalogSdkEntry {
                version: clean.to_string(),
                label,
                lts: is_lts,
                os: platform.os.as_str().to_string(),
                arch: platform.arch.as_str().to_string(),
                url: node_tarball_url(clean, platform),
                size_hint: Some("42 MB".to_string()),
                sha256: None,
            });
            break;
        }
    }
    out
}

pub fn parse_uv_release_json(raw: &str, platform: TargetPlatform) -> Vec<CatalogSdkEntry> {
    let mut entries = curated_catalog(EcosystemKind::Python, platform);
    let Ok(val) = serde_json::from_str::<serde_json::Value>(raw) else {
        return entries;
    };
    if let Some(tag) = val.get("tag_name").and_then(|v| v.as_str()) {
        let clean = tag.trim_start_matches('v');
        if is_safe_version_token(clean) {
            for e in &mut entries {
                e.label = format!("Python {} (Astral uv {clean})", e.version);
            }
        }
    }
    let expected_asset = format!("uv-{}.tar.gz", platform.rust_triple());
    if let Some(assets) = val.get("assets").and_then(|v| v.as_array()) {
        for asset in assets {
            if asset.get("name").and_then(|v| v.as_str()) != Some(expected_asset.as_str()) {
                continue;
            }
            let asset_url = asset
                .get("browser_download_url")
                .and_then(|v| v.as_str())
                .filter(|u| u.starts_with("https://github.com/astral-sh/uv/releases/download/"))
                .map(str::to_string);
            let asset_sha = asset
                .get("digest")
                .and_then(|v| v.as_str())
                .and_then(|d| d.strip_prefix("sha256:").or(Some(d)))
                .and_then(|hex_str| normalize_sha256(Some(hex_str)));
            for e in &mut entries {
                if let Some(ref url) = asset_url {
                    e.url = url.clone();
                }
                if asset_sha.is_some() {
                    e.sha256 = asset_sha.clone();
                }
            }
            break;
        }
    }
    entries
}

// ── Live + Cached Catalog Client ─────────────────────────────────────────────

struct CachedPlatformCatalog {
    fetched_at: Instant,
    by_ecosystem: HashMap<EcosystemKind, Vec<CatalogSdkEntry>>,
}

/// Fetches live architecture-matched catalogs over HTTPS with a 15-minute
/// per-platform cache and zero-HTTPS curated fallback for instant IDE loads.
pub struct CatalogClient {
    http: reqwest::Client,
    cache: Mutex<HashMap<TargetPlatform, CachedPlatformCatalog>>,
}

impl Default for CatalogClient {
    fn default() -> Self {
        Self::new()
    }
}

impl CatalogClient {
    pub fn new() -> Self {
        let http = otto_netguard::guarded_client_builder()
            .https_only(true)
            .user_agent("otto-ide/0.2")
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(HTTP_READ_TIMEOUT)
            .build()
            .expect("build netguard-protected catalog HTTP client");
        Self {
            http,
            cache: Mutex::new(HashMap::new()),
        }
    }

    #[doc(hidden)]
    pub fn seed_cached_entry(
        &self,
        platform: TargetPlatform,
        eco: EcosystemKind,
        entry: CatalogSdkEntry,
    ) {
        if let Ok(mut guard) = self.cache.lock() {
            let slot = guard
                .entry(platform)
                .or_insert_with(|| CachedPlatformCatalog {
                    fetched_at: Instant::now(),
                    by_ecosystem: EcosystemKind::ALL
                        .into_iter()
                        .map(|k| (k, curated_catalog(k, platform)))
                        .collect(),
                });
            slot.fetched_at = Instant::now();
            slot.by_ecosystem.entry(eco).or_default().insert(0, entry);
        }
    }

    fn offline_mode() -> bool {
        otto_core::e2e::isolated()
            || std::env::var("OTTO_IDE_OFFLINE_CATALOG")
                .is_ok_and(|v| v.trim() == "1" || v.trim().eq_ignore_ascii_case("true"))
    }

    /// Return warm cached live entries if already fetched within `CACHE_TTL`,
    /// otherwise return the zero-HTTPS curated catalog immediately (`<0.1ms`).
    pub fn cached_or_curated(
        &self,
        platform: TargetPlatform,
    ) -> HashMap<EcosystemKind, Vec<CatalogSdkEntry>> {
        if let Ok(guard) = self.cache.lock() {
            if let Some(hit) = guard.get(&platform) {
                if hit.fetched_at.elapsed() < CACHE_TTL {
                    return hit.by_ecosystem.clone();
                }
            }
        }
        EcosystemKind::ALL
            .into_iter()
            .map(|eco| (eco, curated_catalog(eco, platform)))
            .collect()
    }

    pub async fn fetch_all(
        &self,
        platform: TargetPlatform,
    ) -> HashMap<EcosystemKind, Vec<CatalogSdkEntry>> {
        if Self::offline_mode() {
            return self.cached_or_curated(platform);
        }
        if let Ok(guard) = self.cache.lock() {
            if let Some(hit) = guard.get(&platform) {
                if hit.fetched_at.elapsed() < CACHE_TTL {
                    return hit.by_ecosystem.clone();
                }
            }
        }

        let (java, go, rust, node, python) = tokio::join!(
            self.fetch_java(platform),
            self.fetch_go(platform),
            self.fetch_rust(platform),
            self.fetch_node(platform),
            self.fetch_python(platform),
        );
        let mut map = HashMap::with_capacity(10);
        map.insert(EcosystemKind::Java, java);
        map.insert(EcosystemKind::Go, go);
        map.insert(EcosystemKind::Rust, rust);
        map.insert(EcosystemKind::Node, node);
        map.insert(EcosystemKind::Python, python);
        map.insert(
            EcosystemKind::Shell,
            curated_catalog(EcosystemKind::Shell, platform),
        );
        map.insert(
            EcosystemKind::Json,
            curated_catalog(EcosystemKind::Json, platform),
        );
        map.insert(EcosystemKind::Ios, Vec::new());
        map.insert(EcosystemKind::Android, Vec::new());
        map.insert(EcosystemKind::Flutter, Vec::new());

        if let Ok(mut guard) = self.cache.lock() {
            guard.insert(
                platform,
                CachedPlatformCatalog {
                    fetched_at: Instant::now(),
                    by_ecosystem: map.clone(),
                },
            );
        }
        map
    }

    async fn get_text(&self, url: &str) -> Option<String> {
        otto_netguard::check_url(url).await.ok()?;
        let resp = self.http.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.text().await.ok()
    }

    async fn fetch_java(&self, platform: TargetPlatform) -> Vec<CatalogSdkEntry> {
        let os = platform.os.adoptium_os();
        let arch = platform.arch.adoptium_arch();
        let url_for = |ver: u32| {
            format!(
                "https://api.adoptium.net/v3/assets/feature_releases/{ver}/ga?architecture={arch}&image_type=jdk&jvm_impl=hotspot&os={os}&page_size=1&project=jdk&vendor=eclipse"
            )
        };
        let (u25, u21, u17, u11, u8) = (
            url_for(25),
            url_for(21),
            url_for(17),
            url_for(11),
            url_for(8),
        );
        let (r25, r21, r17, r11, r8) = tokio::join!(
            self.get_text(&u25),
            self.get_text(&u21),
            self.get_text(&u17),
            self.get_text(&u11),
            self.get_text(&u8),
        );
        let fallback = curated_catalog(EcosystemKind::Java, platform);
        let mut out = Vec::with_capacity(5);
        for (idx, (ver, raw_opt)) in [(25, r25), (21, r21), (17, r17), (11, r11), (8, r8)]
            .into_iter()
            .enumerate()
        {
            if let Some(entry) =
                raw_opt.and_then(|t| parse_adoptium_release_json(ver, &t, platform))
            {
                out.push(entry);
            } else if let Some(fb) = fallback.get(idx) {
                out.push(fb.clone());
            }
        }
        out
    }

    async fn fetch_go(&self, platform: TargetPlatform) -> Vec<CatalogSdkEntry> {
        if let Some(raw) = self
            .get_text("https://go.dev/dl/?mode=json&include=all")
            .await
        {
            let parsed = parse_go_dl_json(&raw, platform);
            if !parsed.is_empty() {
                return parsed;
            }
        }
        curated_catalog(EcosystemKind::Go, platform)
    }

    async fn fetch_rust(&self, platform: TargetPlatform) -> Vec<CatalogSdkEntry> {
        if let Some(raw) = self
            .get_text("https://static.rust-lang.org/dist/channel-rust-stable.toml")
            .await
        {
            return parse_rust_channel_toml(&raw, platform);
        }
        curated_catalog(EcosystemKind::Rust, platform)
    }

    async fn fetch_node(&self, platform: TargetPlatform) -> Vec<CatalogSdkEntry> {
        if let Some(raw) = self.get_text("https://nodejs.org/dist/index.json").await {
            let parsed = parse_node_dist_json(&raw, platform);
            if !parsed.is_empty() {
                return parsed;
            }
        }
        curated_catalog(EcosystemKind::Node, platform)
    }

    async fn fetch_python(&self, platform: TargetPlatform) -> Vec<CatalogSdkEntry> {
        if let Some(raw) = self
            .get_text("https://api.github.com/repos/astral-sh/uv/releases/latest")
            .await
        {
            return parse_uv_release_json(&raw, platform);
        }
        curated_catalog(EcosystemKind::Python, platform)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_PLATFORMS: [TargetPlatform; 4] = [
        TargetPlatform {
            os: TargetOs::Macos,
            arch: TargetArch::Aarch64,
        },
        TargetPlatform {
            os: TargetOs::Macos,
            arch: TargetArch::X86_64,
        },
        TargetPlatform {
            os: TargetOs::Linux,
            arch: TargetArch::Aarch64,
        },
        TargetPlatform {
            os: TargetOs::Linux,
            arch: TargetArch::X86_64,
        },
    ];

    #[test]
    fn uname_matrix_parses_all_supported_os_arch_combinations() {
        assert_eq!(
            TargetPlatform::from_combined_uname("Darwin arm64\n"),
            Some(ALL_PLATFORMS[0])
        );
        assert_eq!(
            TargetPlatform::from_combined_uname("Darwin x86_64"),
            Some(ALL_PLATFORMS[1])
        );
        assert_eq!(
            TargetPlatform::from_combined_uname("Linux aarch64"),
            Some(ALL_PLATFORMS[2])
        );
        assert_eq!(
            TargetPlatform::from_combined_uname("Linux x86_64"),
            Some(ALL_PLATFORMS[3])
        );
        assert_eq!(TargetPlatform::from_combined_uname("FreeBSD riscv64"), None);
    }

    #[test]
    fn curated_catalogs_cover_all_seven_ecosystems_and_four_platforms() {
        for platform in ALL_PLATFORMS {
            for eco in EcosystemKind::ALL {
                let items = curated_catalog(eco, platform);
                if matches!(
                    eco,
                    EcosystemKind::Ios | EcosystemKind::Android | EcosystemKind::Flutter
                ) {
                    assert!(
                        items.is_empty(),
                        "{eco:?} is a native SDK and should not expose archive tarballs"
                    );
                    continue;
                }
                assert!(!items.is_empty(), "{eco:?} on {platform:?} empty");
                for item in &items {
                    assert_eq!(item.os, platform.os.as_str());
                    assert_eq!(item.arch, platform.arch.as_str());
                    assert!(
                        item.url.starts_with("https://"),
                        "insecure url: {}",
                        item.url
                    );
                }
            }
        }
    }

    #[test]
    fn live_metadata_parsers_extract_exact_os_arch_artifacts_and_sha256() {
        let mac_arm = ALL_PLATFORMS[0];
        let linux_x64 = ALL_PLATFORMS[3];

        let adoptium_json = r#"[{
            "release_name": "jdk-21.0.6+7",
            "version_data": { "semver": "21.0.6+7" },
            "binaries": [{
                "package": {
                    "link": "https://github.com/adoptium/temurin21-binaries/releases/download/jdk-21.0.6/OpenJDK21U-jdk_aarch64_mac_hotspot_21.0.6_7.tar.gz",
                    "checksum": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                    "size": 207618048
                }
            }]
        }]"#;
        let jdk = parse_adoptium_release_json(21, adoptium_json, mac_arm).unwrap();
        assert_eq!(jdk.version, "21");
        assert!(jdk.lts);
        assert!(jdk.label.contains("21.0.6+7"));
        assert!(jdk.url.contains("aarch64_mac"));
        assert_eq!(
            jdk.sha256.as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );

        let go_json = r#"[{
            "version": "go1.24.2",
            "stable": true,
            "files": [
                {"filename": "go1.24.2.darwin-arm64.tar.gz", "os": "darwin", "arch": "arm64", "kind": "archive", "sha256": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210", "size": 73400320},
                {"filename": "go1.24.2.linux-amd64.tar.gz", "os": "linux", "arch": "amd64", "kind": "archive", "size": 75497472}
            ]
        }]"#;
        let go_mac = parse_go_dl_json(go_json, mac_arm);
        assert_eq!(go_mac.len(), 1);
        assert_eq!(go_mac[0].version, "1.24.2");
        assert_eq!(
            go_mac[0].sha256.as_deref(),
            Some("fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210")
        );
        assert_eq!(
            go_mac[0].url,
            "https://go.dev/dl/go1.24.2.darwin-arm64.tar.gz"
        );
        let go_lin = parse_go_dl_json(go_json, linux_x64);
        assert_eq!(
            go_lin[0].url,
            "https://go.dev/dl/go1.24.2.linux-amd64.tar.gz"
        );

        let node_json = r#"[
            {"version": "v24.1.0", "lts": false, "files": ["osx-arm64-tar", "linux-x64"]},
            {"version": "v22.15.0", "lts": "Jod", "files": ["osx-arm64-tar", "linux-x64"]},
            {"version": "v20.19.1", "lts": "Iron", "files": ["osx-arm64-tar"]}
        ]"#;
        let node_mac = parse_node_dist_json(node_json, mac_arm);
        assert_eq!(node_mac.len(), 3);
        assert!(node_mac[1].lts);
        assert!(node_mac[1]
            .url
            .ends_with("node-v22.15.0-darwin-arm64.tar.gz"));
        let node_lin = parse_node_dist_json(node_json, linux_x64);
        assert_eq!(node_lin.len(), 2);
        assert!(node_lin[0].url.ends_with("node-v24.1.0-linux-x64.tar.gz"));

        let rust_toml = "[pkg.cargo]\nversion = \"0.86.0\"\n[pkg.rust]\nversion = \"1.85.1 (4eb161250 2025-03-15)\"\n";
        let rust_entries = parse_rust_channel_toml(rust_toml, mac_arm);
        assert!(rust_entries[0].label.contains("1.85.1"));

        let uv_json = r#"{
            "tag_name": "0.6.9",
            "assets": [{
                "name": "uv-aarch64-apple-darwin.tar.gz",
                "browser_download_url": "https://github.com/astral-sh/uv/releases/download/0.6.9/uv-aarch64-apple-darwin.tar.gz",
                "digest": "sha256:11223344556677889900aabbccddeeff11223344556677889900aabbccddeeff"
            }]
        }"#;
        let py_entries = parse_uv_release_json(uv_json, mac_arm);
        assert!(py_entries[0].label.contains("0.6.9"));
        assert_eq!(
            py_entries[0].url,
            "https://github.com/astral-sh/uv/releases/download/0.6.9/uv-aarch64-apple-darwin.tar.gz"
        );
        assert_eq!(
            py_entries[0].sha256.as_deref(),
            Some("11223344556677889900aabbccddeeff11223344556677889900aabbccddeeff")
        );
    }

    #[test]
    fn version_sanitizer_blocks_traversal_and_flags() {
        assert!(is_safe_version_token("21"));
        assert!(is_safe_version_token("1.24.1"));
        assert!(is_safe_version_token("stable"));
        assert!(!is_safe_version_token("-rf"));
        assert!(!is_safe_version_token("../etc"));
        assert!(!is_safe_version_token("21; rm -rf /"));
    }
}
