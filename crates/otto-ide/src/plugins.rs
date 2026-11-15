//! Runtime IDE Extension System (`IdePluginRegistry`).
//!
//! Loads IDE extension packs at runtime with zero daemon or UI rebuild from:
//! 1. `<data_dir>/ide-plugins/<slug>/otto-ide-plugin.json` (managed IDE packs), and
//! 2. `~/otto-plugins/*/otto-plugin.json` when an optional `"ide"` contribution
//!    object is present on a general Otto runtime plugin.

#[path = "plugins/catalog.rs"]
mod catalog;
#[path = "plugins/fs_loader.rs"]
mod fs_loader;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use otto_core::{new_id, Error, Result};
use serde::{Deserialize, Serialize};

use crate::types::{
    IdePluginCatalogEntry, IdePluginInfo, PluginDapAdapterSpec, PluginGutterDetector,
    PluginLanguageSpec, PluginLspServerSpec,
};

use self::catalog::{builtin_catalog_entries, builtin_catalog_manifest};
use self::fs_loader::{
    copy_plugin_dir, resolve_local_source_path, write_normalized_manifest_if_needed,
};

const STATE_FILE: &str = "state.json";
const MANIFEST_FILE: &str = "otto-ide-plugin.json";
const SIDECAR_MANIFEST_FILE: &str = "otto-plugin.json";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct PersistedPluginState {
    #[serde(default)]
    enabled: HashMap<String, bool>,
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
#[derive(Clone)]
pub struct IdePluginRegistry {
    plugins_dir: PathBuf,
    sidecar_plugins_dir: Option<PathBuf>,
    cache: Arc<RwLock<Vec<IdePluginInfo>>>,
}

impl Default for IdePluginRegistry {
    fn default() -> Self {
        let fallback = std::env::temp_dir().join("otto-ide-default");
        Self::new(fallback)
    }
}

impl IdePluginRegistry {
    pub fn new(data_dir: PathBuf) -> Self {
        let sidecar = dirs::home_dir().map(|h| h.join("otto-plugins"));
        Self::with_dirs(data_dir, sidecar)
    }

    pub fn with_dirs(data_dir: PathBuf, sidecar_plugins_dir: Option<PathBuf>) -> Self {
        let plugins_dir = data_dir.join("ide-plugins");
        let _ = std::fs::create_dir_all(&plugins_dir);
        let registry = Self {
            plugins_dir,
            sidecar_plugins_dir,
            cache: Arc::new(RwLock::new(Vec::new())),
        };
        registry.reload();
        registry
    }

    // Bounded shallow scan of local plugin manifest folders (<data_dir>/ide-plugins, ~/otto-plugins).
    #[allow(clippy::disallowed_methods)]
    pub fn reload(&self) -> Vec<IdePluginInfo> {
        let state = self.load_state();
        let mut by_slug: HashMap<String, IdePluginInfo> = HashMap::new();

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

    pub fn cached_plugins(&self) -> Vec<IdePluginInfo> {
        self.cache.read().map(|g| g.clone()).unwrap_or_default()
    }

    pub fn active_languages(&self) -> Vec<PluginLanguageSpec> {
        self.cached_plugins()
            .into_iter()
            .filter(|p| p.enabled)
            .flat_map(|p| p.contributions.languages)
            .collect()
    }

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

    pub fn active_lsp_servers(&self) -> Vec<PluginLspServerSpec> {
        self.cached_plugins()
            .into_iter()
            .filter(|p| p.enabled)
            .flat_map(|p| p.contributions.lsp_servers)
            .collect()
    }

    pub fn active_dap_adapters(&self) -> Vec<PluginDapAdapterSpec> {
        self.cached_plugins()
            .into_iter()
            .filter(|p| p.enabled)
            .flat_map(|p| p.contributions.dap_adapters)
            .collect()
    }

    pub fn active_gutter_detectors(&self) -> Vec<PluginGutterDetector> {
        self.cached_plugins()
            .into_iter()
            .filter(|p| p.enabled)
            .flat_map(|p| p.contributions.gutter_detectors)
            .collect()
    }

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
            write_normalized_manifest_if_needed(&dest, &info)?;
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
        otto_netguard::check_url(url)
            .await
            .map_err(Error::Forbidden)?;

        let staging = self.plugins_dir.join(format!(".staging-{}", new_id()));
        let clone_res = tokio::time::timeout(
            Duration::from_secs(60),
            otto_git_cmd::hardened_command()
                .args([
                    "-c",
                    "protocol.allow=https",
                    "-c",
                    "http.followRedirects=false",
                    "clone",
                    "--depth",
                    "1",
                    "--",
                ])
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
        // Copy via `copy_plugin_dir` rather than renaming the raw git checkout so
        // symlinks, `.git`, `node_modules`, and `target` never enter `<data_dir>/ide-plugins`
        // (preserving state-archive snapshot safety).
        let staging_c = staging.clone();
        let dest_c = dest.clone();
        let info_c = info.clone();
        let copy_res = tokio::task::spawn_blocking(move || -> Result<()> {
            copy_plugin_dir(&staging_c, &dest_c)?;
            write_normalized_manifest_if_needed(&dest_c, &info_c)
        })
        .await
        .map_err(|e| Error::Internal(format!("finalize git plugin install task: {e}")))?;
        let _ = tokio::fs::remove_dir_all(&staging).await;
        copy_res?;
        Ok(info.slug)
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
