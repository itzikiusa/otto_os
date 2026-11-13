//! Runtime IDE Extension System (`IdePluginRegistry`).
//!
//! Loads IDE extension packs at runtime with zero daemon or UI rebuild from:
//! 1. `<data_dir>/ide-plugins/<slug>/otto-ide-plugin.json` (managed IDE packs), and
//! 2. `~/otto-plugins/*/otto-plugin.json` when an optional `"ide"` contribution
//!    object is present on a general Otto runtime plugin.
//!
//! Supports installing from:
//! - Curated built-in catalog (`catalog:kotlin-ide`, `catalog:makefile-docker-pack`,
//!   `catalog:graphql-ide`, `catalog:protobuf-grpc-ide`),
//! - Local filesystem directory containing `otto-ide-plugin.json` (or `otto-plugin.json`
//!   with `"ide"`),
//! - Hardened `https://` Git repository URL via `git clone --depth 1 -- <url>`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use std::time::Duration;

use otto_core::{new_id, Error, Result};
use serde::{Deserialize, Serialize};

use crate::types::{
    IdePluginCatalogEntry, IdePluginContributions, IdePluginInfo, IdePluginManifest,
    PluginDapAdapterSpec, PluginGutterDetector, PluginLanguageSpec, PluginLspServerSpec,
};

const STATE_FILE: &str = "state.json";
const MANIFEST_FILE: &str = "otto-ide-plugin.json";
const SIDECAR_MANIFEST_FILE: &str = "otto-plugin.json";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct PersistedPluginState {
    /// Map of `slug -> enabled`. Newly installed plugins default to `true`.
    #[serde(default)]
    enabled: HashMap<String, bool>,
}

/// Flexible on-disk `otto-ide-plugin.json` reader accepting either `contributions: { ... }`
/// (canonical [`IdePluginManifest`]) or flat top-level contribution arrays.
#[derive(Debug, Clone, Deserialize)]
struct FlexibleManifest {
    slug: String,
    name: String,
    #[serde(default = "default_version")]
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    contributions: Option<IdePluginContributions>,
    #[serde(default)]
    flattened: IdePluginContributions,
}

impl FlexibleManifest {
    fn into_manifest(self) -> IdePluginManifest {
        let mut contributions = self.contributions.unwrap_or_default();
        if contributions.languages.is_empty() {
            contributions.languages = self.flattened.languages;
        }
        if contributions.lsp_servers.is_empty() {
            contributions.lsp_servers = self.flattened.lsp_servers;
        }
        if contributions.dap_adapters.is_empty() {
            contributions.dap_adapters = self.flattened.dap_adapters;
        }
        if contributions.gutter_detectors.is_empty() {
            contributions.gutter_detectors = self.flattened.gutter_detectors;
        }
        if contributions.keymaps.is_empty() {
            contributions.keymaps = self.flattened.keymaps;
        }
        if contributions.snippets.is_empty() {
            contributions.snippets = self.flattened.snippets;
        }
        if contributions.panels.is_empty() {
            contributions.panels = self.flattened.panels;
        }
        IdePluginManifest {
            slug: self.slug,
            name: self.name,
            version: self.version,
            description: self.description,
            author: self.author,
            contributions,
        }
    }
}

/// Envelope for `~/otto-plugins/<slug>/otto-plugin.json` with an optional `"ide"` block.
#[derive(Debug, Clone, Deserialize)]
struct SidecarManifestEnvelope {
    slug: String,
    name: String,
    #[serde(default = "default_version")]
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    ide: Option<IdePluginContributions>,
}

fn default_version() -> String {
    "1.0.0".to_string()
}

/// Validate a plugin slug (`^[a-z][a-z0-9-]{0,63}$`).
pub fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 64
        && slug.bytes().next().is_some_and(|b| b.is_ascii_lowercase())
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Thread-safe runtime IDE plugin registry.
pub struct IdePluginRegistry {
    plugins_dir: PathBuf,
    sidecar_plugins_dir: Option<PathBuf>,
    cache: RwLock<Vec<IdePluginInfo>>,
}

impl Default for IdePluginRegistry {
    fn default() -> Self {
        let fallback = std::env::temp_dir().join("otto-ide-default");
        Self::new(fallback)
    }
}

impl IdePluginRegistry {
    /// Create a registry rooted at `<data_dir>/ide-plugins` and inspecting `~/otto-plugins`.
    pub fn new(data_dir: PathBuf) -> Self {
        let sidecar = dirs::home_dir().map(|h| h.join("otto-plugins"));
        Self::with_dirs(data_dir, sidecar)
    }

    /// Create a registry with explicit IDE plugins root and optional sidecar plugins root.
    pub fn with_dirs(data_dir: PathBuf, sidecar_plugins_dir: Option<PathBuf>) -> Self {
        let plugins_dir = data_dir.join("ide-plugins");
        let _ = std::fs::create_dir_all(&plugins_dir);
        let registry = Self {
            plugins_dir,
            sidecar_plugins_dir,
            cache: RwLock::new(Vec::new()),
        };
        registry.reload();
        registry
    }

    /// Reload installed plugins from disk into the in-memory cache.
    // Bounded shallow scan of local plugin manifest folders (<data_dir>/ide-plugins, ~/otto-plugins).
    #[allow(clippy::disallowed_methods)]
    pub fn reload(&self) -> Vec<IdePluginInfo> {
        let state = self.load_state();
        let mut by_slug: HashMap<String, IdePluginInfo> = HashMap::new();

        // 1. Inspect ~/otto-plugins/*/otto-plugin.json for optional `ide` block.
        if let Some(sidecar_root) = &self.sidecar_plugins_dir {
            if let Ok(entries) = std::fs::read_dir(sidecar_root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_dir() {
                        continue;
                    }
                    if let Ok(Some(mut info)) = Self::load_from_dir(&path, "otto-plugin") {
                        info.enabled = *state.enabled.get(&info.slug).unwrap_or(&true);
                        by_slug.insert(info.slug.clone(), info);
                    }
                }
            }
        }

        // 2. Load managed IDE packs from <data_dir>/ide-plugins/<slug>/otto-ide-plugin.json.
        if let Ok(entries) = std::fs::read_dir(&self.plugins_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let name = entry.file_name();
                if name.to_string_lossy().starts_with('.') || !path.is_dir() {
                    continue;
                }
                if let Ok(Some(mut info)) = Self::load_from_dir(&path, "managed") {
                    info.enabled = *state.enabled.get(&info.slug).unwrap_or(&true);
                    by_slug.insert(info.slug.clone(), info);
                }
            }
        }

        let mut list: Vec<IdePluginInfo> = by_slug.into_values().collect();
        list.sort_by(|a, b| a.slug.cmp(&b.slug));
        if let Ok(mut guard) = self.cache.write() {
            *guard = list.clone();
        }
        list
    }

    /// List installed plugins and built-in curated catalog entries.
    pub fn list(&self) -> (Vec<IdePluginInfo>, Vec<IdePluginCatalogEntry>) {
        let installed = self.reload();
        let installed_slugs: std::collections::HashSet<&str> =
            installed.iter().map(|p| p.slug.as_str()).collect();
        let mut catalog = builtin_catalog_entries();
        for entry in &mut catalog {
            entry.installed = installed_slugs.contains(entry.slug.as_str());
        }
        (installed, catalog)
    }

    /// Return snapshot of currently cached plugins without hitting disk.
    pub fn cached_plugins(&self) -> Vec<IdePluginInfo> {
        self.cache.read().map(|g| g.clone()).unwrap_or_default()
    }

    /// Active languages contributed by enabled IDE plugins.
    pub fn active_languages(&self) -> Vec<PluginLanguageSpec> {
        self.cached_plugins()
            .into_iter()
            .filter(|p| p.enabled)
            .flat_map(|p| p.contributions.languages)
            .collect()
    }

    /// Match a file path to an enabled plugin language by filename or extension.
    pub fn detect_language_for_path(&self, path: &str) -> Option<PluginLanguageSpec> {
        let file_name = Path::new(path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(path);
        let lower_name = file_name.to_ascii_lowercase();
        self.active_languages().into_iter().find(|lang| {
            lang.extensions.iter().any(|ext| {
                if ext.starts_with('.') {
                    lower_name.ends_with(&ext.to_ascii_lowercase())
                } else {
                    file_name == ext
                        || lower_name == ext.to_ascii_lowercase()
                        || lower_name.ends_with(&format!(".{}", ext.to_ascii_lowercase()))
                }
            })
        })
    }

    /// Active LSP servers contributed by enabled IDE plugins.
    pub fn active_lsp_servers(&self) -> Vec<PluginLspServerSpec> {
        self.cached_plugins()
            .into_iter()
            .filter(|p| p.enabled)
            .flat_map(|p| p.contributions.lsp_servers)
            .collect()
    }

    /// Active DAP debug adapters contributed by enabled IDE plugins.
    pub fn active_dap_adapters(&self) -> Vec<PluginDapAdapterSpec> {
        self.cached_plugins()
            .into_iter()
            .filter(|p| p.enabled)
            .flat_map(|p| p.contributions.dap_adapters)
            .collect()
    }

    /// Active gutter test/main detectors contributed by enabled IDE plugins.
    pub fn active_gutter_detectors(&self) -> Vec<PluginGutterDetector> {
        self.cached_plugins()
            .into_iter()
            .filter(|p| p.enabled)
            .flat_map(|p| p.contributions.gutter_detectors)
            .collect()
    }

    /// Enable or disable an installed IDE plugin at runtime.
    pub fn toggle(&self, slug: &str, enabled: bool) -> Result<IdePluginInfo> {
        if !is_valid_slug(slug) {
            return Err(Error::Invalid(format!("invalid plugin slug '{slug}'")));
        }
        let current = self.reload();
        if !current.iter().any(|p| p.slug == slug) {
            return Err(Error::NotFound(format!("IDE plugin '{slug}' not found")));
        }
        let mut state = self.load_state();
        state.enabled.insert(slug.to_string(), enabled);
        self.save_state(&state)?;
        self.reload()
            .into_iter()
            .find(|p| p.slug == slug)
            .ok_or_else(|| Error::NotFound(format!("IDE plugin '{slug}' not found")))
    }

    /// Uninstall a managed IDE plugin from `<data_dir>/ide-plugins/<slug>`.
    // Bounded removal of a single tiny JSON manifest folder under <data_dir>/ide-plugins.
    #[allow(clippy::disallowed_methods)]
    pub fn uninstall(&self, slug: &str) -> Result<()> {
        if !is_valid_slug(slug) {
            return Err(Error::Invalid(format!("invalid plugin slug '{slug}'")));
        }
        let target_dir = self.plugins_dir.join(slug);
        if !target_dir.exists() {
            let current = self.reload();
            if current.iter().any(|p| p.slug == slug) {
                return Err(Error::Invalid(format!(
                    "plugin '{slug}' comes from ~/otto-plugins; disable it here or remove the sidecar plugin from Plugins"
                )));
            }
            return Err(Error::NotFound(format!("IDE plugin '{slug}' not found")));
        }
        std::fs::remove_dir_all(&target_dir)
            .map_err(|e| Error::Internal(format!("failed to remove plugin '{slug}': {e}")))?;
        let mut state = self.load_state();
        state.enabled.remove(slug);
        let _ = self.save_state(&state);
        self.reload();
        Ok(())
    }

    /// Install an IDE plugin from `catalog:<slug>`, a local directory path, or an `https://` Git URL.
    pub async fn install(&self, raw_source: &str) -> Result<IdePluginInfo> {
        let source = raw_source.trim();
        if source.is_empty() {
            return Err(Error::Invalid("plugin source must not be empty".into()));
        }
        if source.starts_with('-') {
            return Err(Error::Invalid(
                "plugin source must not start with '-'".into(),
            ));
        }

        std::fs::create_dir_all(&self.plugins_dir)
            .map_err(|e| Error::Internal(format!("create ide-plugins dir: {e}")))?;

        let slug = if let Some(cat_slug) = source.strip_prefix("catalog:") {
            self.install_from_catalog(cat_slug.trim())?
        } else if source.contains("://") {
            self.install_from_git_url(source).await?
        } else if builtin_catalog_manifest(source).is_some() && !Path::new(source).exists() {
            // Convenience: allow bare catalog slug when not an existing local path.
            self.install_from_catalog(source)?
        } else {
            self.install_from_local_path(source)?
        };

        let mut state = self.load_state();
        state.enabled.insert(slug.clone(), true);
        self.save_state(&state)?;

        self.reload()
            .into_iter()
            .find(|p| p.slug == slug)
            .ok_or_else(|| Error::Internal(format!("installed plugin '{slug}' failed to load")))
    }

    // Bounded local catalog manifest write under <data_dir>/ide-plugins.
    #[allow(clippy::disallowed_methods)]
    fn install_from_catalog(&self, slug: &str) -> Result<String> {
        if !is_valid_slug(slug) {
            return Err(Error::Invalid(format!("invalid catalog slug '{slug}'")));
        }
        let manifest = builtin_catalog_manifest(slug).ok_or_else(|| {
            Error::NotFound(format!("unknown catalog IDE plugin 'catalog:{slug}'"))
        })?;
        let dest = self.plugins_dir.join(&manifest.slug);
        if dest.exists() {
            std::fs::remove_dir_all(&dest)
                .map_err(|e| Error::Internal(format!("replace existing plugin dir: {e}")))?;
        }
        std::fs::create_dir_all(&dest)
            .map_err(|e| Error::Internal(format!("create plugin dir: {e}")))?;
        let json = serde_json::to_string_pretty(&manifest)?;
        std::fs::write(dest.join(MANIFEST_FILE), json)
            .map_err(|e| Error::Internal(format!("write manifest: {e}")))?;
        Ok(manifest.slug)
    }

    // Bounded local manifest copy (skips .git, node_modules, target).
    #[allow(clippy::disallowed_methods)]
    fn install_from_local_path(&self, raw_path: &str) -> Result<String> {
        let resolved = resolve_local_source_path(raw_path)?;
        let info = Self::load_from_dir(&resolved, "local")?.ok_or_else(|| {
            Error::Invalid(format!(
                "no valid '{MANIFEST_FILE}' (or '{SIDECAR_MANIFEST_FILE}' with 'ide' contributions) found in '{}'",
                resolved.display()
            ))
        })?;

        let dest = self.plugins_dir.join(&info.slug);
        if dest != resolved {
            if dest.exists() {
                std::fs::remove_dir_all(&dest)
                    .map_err(|e| Error::Internal(format!("replace plugin dir: {e}")))?;
            }
            copy_plugin_dir(&resolved, &dest)?;
            let ide_manifest_path = dest.join(MANIFEST_FILE);
            if !ide_manifest_path.exists() {
                let normalized = manifest_from_info(&info);
                let raw = serde_json::to_string_pretty(&normalized)?;
                std::fs::write(&ide_manifest_path, raw)
                    .map_err(|e| Error::Internal(format!("write normalized manifest: {e}")))?;
            }
        }
        Ok(info.slug)
    }

    async fn install_from_git_url(&self, url: &str) -> Result<String> {
        if !url.starts_with("https://") {
            return Err(Error::Invalid(
                "git plugin URL must use https:// scheme".into(),
            ));
        }
        if url.chars().any(char::is_whitespace) {
            return Err(Error::Invalid(
                "git plugin URL must not contain whitespace".into(),
            ));
        }

        let staging = self.plugins_dir.join(format!(".staging-{}", new_id()));
        let clone_res = tokio::time::timeout(
            Duration::from_secs(60),
            otto_git_cmd::hardened_command()
                .args(["-c", "protocol.allow=https", "clone", "--depth", "1", "--"])
                .arg(url)
                .arg(&staging)
                .output(),
        )
        .await;

        let output = match clone_res {
            Ok(Ok(out)) => out,
            Ok(Err(e)) => {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err(Error::Upstream(format!("failed to invoke git clone: {e}")));
            }
            Err(_) => {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err(Error::Upstream("git clone timed out after 60s".into()));
            }
        };

        if !output.status.success() {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(Error::Upstream(format!(
                "git clone failed: {}",
                stderr.trim()
            )));
        }

        let info = match Self::load_from_dir(&staging, "git") {
            Ok(Some(info)) => info,
            Ok(None) => {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err(Error::Invalid(format!(
                    "cloned repository does not contain '{MANIFEST_FILE}'"
                )));
            }
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(&staging).await;
                return Err(e);
            }
        };

        let dest = self.plugins_dir.join(&info.slug);
        if dest.exists() {
            let _ = tokio::fs::remove_dir_all(&dest).await;
        }
        let _ = tokio::fs::remove_dir_all(staging.join(".git")).await;
        if let Err(e) = tokio::fs::rename(&staging, &dest).await {
            let _ = tokio::fs::remove_dir_all(&staging).await;
            return Err(Error::Internal(format!("finalize git plugin install: {e}")));
        }
        Ok(info.slug)
    }

    fn load_from_dir(dir: &Path, source_kind: &str) -> Result<Option<IdePluginInfo>> {
        let direct = dir.join(MANIFEST_FILE);
        if direct.is_file() {
            let raw = std::fs::read_to_string(&direct)
                .map_err(|e| Error::Invalid(format!("read {}: {e}", direct.display())))?;
            // Parse via serde_json::Value so both `{ "contributions": {...} }` and
            // flat top-level contribution arrays work seamlessly.
            let val: serde_json::Value = serde_json::from_str(&raw)
                .map_err(|e| Error::Invalid(format!("parse {}: {e}", direct.display())))?;
            let flex: FlexibleManifest = parse_flexible_manifest(&val)
                .map_err(|e| Error::Invalid(format!("parse {}: {e}", direct.display())))?;
            let m = flex.into_manifest();
            if !is_valid_slug(&m.slug) {
                return Err(Error::Invalid(format!(
                    "invalid plugin slug '{}' (expected ^[a-z][a-z0-9-]*$)",
                    m.slug
                )));
            }
            return Ok(Some(IdePluginInfo {
                slug: m.slug,
                name: m.name,
                version: m.version,
                description: m.description,
                author: m.author,
                enabled: true,
                source: source_kind.to_string(),
                install_path: dir.to_string_lossy().into_owned(),
                contributions: m.contributions,
            }));
        }

        let sidecar = dir.join(SIDECAR_MANIFEST_FILE);
        if sidecar.is_file() {
            let raw = std::fs::read_to_string(&sidecar)
                .map_err(|e| Error::Invalid(format!("read {}: {e}", sidecar.display())))?;
            let env: SidecarManifestEnvelope = serde_json::from_str(&raw)
                .map_err(|e| Error::Invalid(format!("parse {}: {e}", sidecar.display())))?;
            let Some(ide) = env.ide else {
                return Ok(None);
            };
            if !is_valid_slug(&env.slug) {
                return Err(Error::Invalid(format!(
                    "invalid plugin slug '{}'",
                    env.slug
                )));
            }
            return Ok(Some(IdePluginInfo {
                slug: env.slug,
                name: env.name,
                version: env.version,
                description: env.description,
                author: env.author,
                enabled: true,
                source: source_kind.to_string(),
                install_path: dir.to_string_lossy().into_owned(),
                contributions: ide,
            }));
        }

        Ok(None)
    }

    fn load_state(&self) -> PersistedPluginState {
        let state_path = self.plugins_dir.join(STATE_FILE);
        std::fs::read_to_string(state_path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn save_state(&self, state: &PersistedPluginState) -> Result<()> {
        let _ = std::fs::create_dir_all(&self.plugins_dir);
        let state_path = self.plugins_dir.join(STATE_FILE);
        let raw = serde_json::to_string_pretty(state)?;
        std::fs::write(&state_path, raw)
            .map_err(|e| Error::Internal(format!("save plugin state: {e}")))
    }
}

fn parse_flexible_manifest(
    val: &serde_json::Value,
) -> std::result::Result<FlexibleManifest, serde_json::Error> {
    let slug = val
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let version = val
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(default_version);
    let description = val
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let author = val
        .get("author")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let contributions = match val.get("contributions") {
        Some(c) if !c.is_null() => {
            Some(serde_json::from_value::<IdePluginContributions>(c.clone())?)
        }
        _ => None,
    };
    let flattened: IdePluginContributions = serde_json::from_value(val.clone()).unwrap_or_default();
    Ok(FlexibleManifest {
        slug,
        name,
        version,
        description,
        author,
        contributions,
        flattened,
    })
}

fn manifest_from_info(info: &IdePluginInfo) -> IdePluginManifest {
    IdePluginManifest {
        slug: info.slug.clone(),
        name: info.name.clone(),
        version: info.version.clone(),
        description: info.description.clone(),
        author: info.author.clone(),
        contributions: info.contributions.clone(),
    }
}

fn resolve_local_source_path(raw: &str) -> Result<PathBuf> {
    let expanded = if let Some(rest) = raw.strip_prefix("~/") {
        dirs::home_dir()
            .ok_or_else(|| Error::Invalid("cannot expand '~': HOME is not set".into()))?
            .join(rest)
    } else {
        PathBuf::from(raw)
    };
    if expanded.is_dir() {
        return std::fs::canonicalize(&expanded)
            .map_err(|e| Error::Invalid(format!("invalid plugin path '{}': {e}", raw)));
    }
    Err(Error::NotFound(format!(
        "plugin directory '{}' does not exist",
        raw
    )))
}

// Bounded recursive copy of plugin manifest/snippet files (skips .git, node_modules, target).
#[allow(clippy::disallowed_methods)]
fn copy_plugin_dir(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)
        .map_err(|e| Error::Internal(format!("create '{}': {e}", dst.display())))?;
    let entries = std::fs::read_dir(src)
        .map_err(|e| Error::Internal(format!("read '{}': {e}", src.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| Error::Internal(format!("dir entry: {e}")))?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if matches!(name_str.as_ref(), ".git" | "node_modules" | "target") {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|e| Error::Internal(format!("stat '{}': {e}", name_str)))?;
        if file_type.is_symlink() {
            continue;
        }
        let dst_path = dst.join(&name);
        if file_type.is_dir() {
            copy_plugin_dir(&entry.path(), &dst_path)?;
        } else if file_type.is_file() {
            std::fs::copy(entry.path(), &dst_path)
                .map_err(|e| Error::Internal(format!("copy '{}': {e}", name_str)))?;
        }
    }
    Ok(())
}

// ===========================================================================
// Built-in Curated IDE Plugin Catalog
// ===========================================================================

fn builtin_catalog_entries() -> Vec<IdePluginCatalogEntry> {
    vec![
        IdePluginCatalogEntry {
            slug: "kotlin-ide".into(),
            name: "Kotlin & Gradle IDE Pack".into(),
            version: "1.0.0".into(),
            description: "Kotlin (.kt, .kts) language support, kotlin-language-server LSP, kotlin-debug-adapter DAP, JUnit/KotlinTest gutter runners, and Gradle snippets.".into(),
            tags: vec!["kotlin".into(), "jvm".into(), "gradle".into(), "lsp".into(), "dap".into()],
            installed: false,
        },
        IdePluginCatalogEntry {
            slug: "makefile-docker-pack".into(),
            name: "Makefile, Dockerfile & Compose Pack".into(),
            version: "1.0.0".into(),
            description: "Makefile target gutter runners, Dockerfile & Containerfile support, docker-langserver / autotools-language-server LSP bindings, and container snippets.".into(),
            tags: vec!["makefile".into(), "dockerfile".into(), "containers".into(), "lsp".into()],
            installed: false,
        },
        IdePluginCatalogEntry {
            slug: "graphql-ide".into(),
            name: "GraphQL Schema & Operations Pack".into(),
            version: "1.0.0".into(),
            description: "GraphQL (.graphql, .gql) syntax, graphql-lsp stdio integration, query/mutation outline symbols, and Apollo/Relay operation snippets.".into(),
            tags: vec!["graphql".into(), "api".into(), "lsp".into()],
            installed: false,
        },
        IdePluginCatalogEntry {
            slug: "protobuf-grpc-ide".into(),
            name: "Protobuf & gRPC (Buf) Pack".into(),
            version: "1.0.0".into(),
            description: "Protocol Buffers (.proto) language support, buf beta lsp / protols language server, buf format integration, and gRPC service/RPC snippets.".into(),
            tags: vec!["protobuf".into(), "grpc".into(), "buf".into(), "lsp".into()],
            installed: false,
        },
    ]
}

fn builtin_catalog_manifest(slug: &str) -> Option<IdePluginManifest> {
    let raw = match slug {
        "kotlin-ide" => {
            include_str!("../../../examples/ide-plugins/kotlin-ide/otto-ide-plugin.json")
        }
        "makefile-docker-pack" => {
            include_str!("../../../examples/ide-plugins/makefile-docker-pack/otto-ide-plugin.json")
        }
        "graphql-ide" => GRAPHQL_CATALOG_JSON,
        "protobuf-grpc-ide" => PROTOBUF_CATALOG_JSON,
        _ => return None,
    };
    serde_json::from_str(raw).ok()
}

const GRAPHQL_CATALOG_JSON: &str = r##"{
  "slug": "graphql-ide",
  "name": "GraphQL Schema & Operations Pack",
  "version": "1.0.0",
  "description": "GraphQL (.graphql, .gql) syntax, graphql-lsp stdio integration, query/mutation outline symbols, and operation snippets.",
  "author": "Otto IDE",
  "contributions": {
    "languages": [
      {
        "id": "graphql",
        "display_name": "GraphQL",
        "extensions": [".graphql", ".gql", "schema.graphql"],
        "line_comment": "#"
      }
    ],
    "lsp_servers": [
      {
        "language": "graphql",
        "command": "graphql-lsp",
        "args": ["server", "-m", "stream"],
        "extensions": [".graphql", ".gql"],
        "install_hint": "npm install -g graphql-language-service-cli"
      }
    ],
    "dap_adapters": [],
    "gutter_detectors": [],
    "keymaps": [],
    "snippets": [
      {
        "language": "graphql",
        "prefix": "gqlquery",
        "body": "query GetItem($id: ID!) {\n  item(id: $id) {\n    id\n  }\n}",
        "description": "GraphQL typed query operation"
      }
    ],
    "panels": []
  }
}"##;

const PROTOBUF_CATALOG_JSON: &str = r#"{
  "slug": "protobuf-grpc-ide",
  "name": "Protobuf & gRPC (Buf) Pack",
  "version": "1.0.0",
  "description": "Protocol Buffers (.proto) language support, buf beta lsp / protols language server, buf format integration, and gRPC service snippets.",
  "author": "Otto IDE",
  "contributions": {
    "languages": [
      {
        "id": "protobuf",
        "display_name": "Protocol Buffers",
        "extensions": [".proto", "buf.yaml", "buf.gen.yaml"],
        "line_comment": "//"
      }
    ],
    "lsp_servers": [
      {
        "language": "protobuf",
        "command": "buf",
        "args": ["beta", "lsp"],
        "extensions": [".proto"],
        "install_hint": "brew install bufbuild/buf/buf"
      }
    ],
    "dap_adapters": [],
    "gutter_detectors": [],
    "keymaps": [],
    "snippets": [
      {
        "language": "protobuf",
        "prefix": "grpcservice",
        "body": "service GreeterService {\n  rpc SayHello (HelloRequest) returns (HelloReply);\n}",
        "description": "gRPC service definition"
      }
    ],
    "panels": []
  }
}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn catalog_install_toggle_and_uninstall_lifecycle() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let reg = IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);

        let (initial, catalog) = reg.list();
        assert!(initial.is_empty());
        assert_eq!(catalog.len(), 4);
        assert!(catalog.iter().all(|c| !c.installed));

        let kt = reg
            .install("catalog:kotlin-ide")
            .await
            .expect("install kotlin-ide");
        assert_eq!(kt.slug, "kotlin-ide");
        assert!(kt.enabled);
        assert_eq!(kt.contributions.languages.len(), 1);
        assert_eq!(kt.contributions.lsp_servers.len(), 1);
        assert_eq!(kt.contributions.dap_adapters.len(), 1);
        assert_eq!(kt.contributions.gutter_detectors.len(), 2);

        let detected = reg
            .detect_language_for_path("/repo/src/Main.kt")
            .expect("detect .kt");
        assert_eq!(detected.id, "kotlin");
        assert_eq!(reg.active_gutter_detectors().len(), 2);

        let disabled = reg.toggle("kotlin-ide", false).expect("disable");
        assert!(!disabled.enabled);
        assert!(reg.detect_language_for_path("/repo/src/Main.kt").is_none());
        assert!(reg.active_gutter_detectors().is_empty());

        let reg2 = IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);
        assert!(reg2.detect_language_for_path("/repo/src/Main.kt").is_none());

        let enabled = reg2.toggle("kotlin-ide", true).expect("re-enable");
        assert!(enabled.enabled);
        assert!(reg2
            .detect_language_for_path("/repo/build.gradle.kts")
            .is_some());

        reg2.uninstall("kotlin-ide").expect("uninstall");
        let (after, _) = reg2.list();
        assert!(after.is_empty());
    }

    #[tokio::test]
    async fn local_path_and_sidecar_otto_plugin_discovery() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let data_dir = tmp.path().join("data");
        let sidecar_dir = tmp.path().join("otto-plugins");
        std::fs::create_dir_all(sidecar_dir.join("my-sidecar")).unwrap();
        std::fs::create_dir_all(sidecar_dir.join("plain-sidecar")).unwrap();

        std::fs::write(
            sidecar_dir.join("my-sidecar/otto-plugin.json"),
            r#"{
              "slug": "my-sidecar",
              "name": "My Sidecar + IDE",
              "version": "0.3.0",
              "exec": ["node", "server.js"],
              "ide": {
                "languages": [{ "id": "cue", "display_name": "CUE", "extensions": [".cue"] }],
                "lsp_servers": [{ "language": "cue", "command": "cue", "args": ["lsp"], "extensions": [".cue"] }]
              }
            }"#,
        )
        .unwrap();

        std::fs::write(
            sidecar_dir.join("plain-sidecar/otto-plugin.json"),
            r#"{"slug":"plain-sidecar","name":"Plain","exec":["./run"]}"#,
        )
        .unwrap();

        let reg = IdePluginRegistry::with_dirs(data_dir, Some(sidecar_dir));
        let (plugins, _) = reg.list();
        assert_eq!(plugins.len(), 1);
        assert_eq!(plugins[0].slug, "my-sidecar");
        assert!(reg.detect_language_for_path("policy.cue").is_some());

        let local_pack = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../examples/ide-plugins/makefile-docker-pack");
        let installed = reg
            .install(&local_pack.to_string_lossy())
            .await
            .expect("install local pack");
        assert_eq!(installed.slug, "makefile-docker-pack");
        assert!(reg.detect_language_for_path("/app/Makefile").is_some());
        assert!(reg.detect_language_for_path("/app/Dockerfile").is_some());
    }

    #[tokio::test]
    async fn rejects_unsafe_sources_and_non_https_git_urls() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let reg = IdePluginRegistry::with_dirs(tmp.path().to_path_buf(), None);

        assert!(reg.install("--upload-pack=evil").await.is_err());
        assert!(reg.install("-oProxyCommand=evil").await.is_err());
        assert!(reg.install("http://example.com/repo.git").await.is_err());
        assert!(reg.install("ssh://git@example.com/repo.git").await.is_err());
        assert!(reg.install("file:///tmp/repo").await.is_err());
        assert!(reg.toggle("../traversal", false).is_err());
    }
}
