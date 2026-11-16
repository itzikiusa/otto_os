//! Manifest parsing and symlink-safe directory copy helpers for [`super::IdePluginRegistry`].

#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};

use otto_core::{Error, Result};
use serde::Deserialize;

use crate::types::{IdePluginContributions, IdePluginInfo, IdePluginManifest};

use super::{is_valid_slug, IdePluginRegistry, MANIFEST_FILE, SIDECAR_MANIFEST_FILE};

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

impl IdePluginRegistry {
    pub(super) fn load_from_dir(dir: &Path, source_kind: &str) -> Result<Option<IdePluginInfo>> {
        let enforce_dir_slug = matches!(source_kind, "managed" | "otto-plugin");
        let dir_name = dir.file_name().and_then(|n| n.to_str());
        let direct = dir.join(MANIFEST_FILE);
        if direct.is_file() {
            let raw = std::fs::read_to_string(&direct)
                .map_err(|e| Error::Invalid(format!("read {}: {e}", direct.display())))?;
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
            if enforce_dir_slug && dir_name != Some(m.slug.as_str()) {
                return Err(Error::Invalid(format!(
                    "plugin manifest slug '{}' does not match directory '{}'",
                    m.slug,
                    dir_name.unwrap_or_default()
                )));
            }
            return Ok(Some(IdePluginInfo {
                slug: m.slug,
                name: m.name,
                version: m.version,
                description: m.description,
                author: m.author,
                enabled: source_kind != "managed",
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
            if enforce_dir_slug && dir_name != Some(env.slug.as_str()) {
                return Err(Error::Invalid(format!(
                    "plugin manifest slug '{}' does not match directory '{}'",
                    env.slug,
                    dir_name.unwrap_or_default()
                )));
            }
            return Ok(Some(IdePluginInfo {
                slug: env.slug,
                name: env.name,
                version: env.version,
                description: env.description,
                author: env.author,
                enabled: source_kind != "managed",
                source: source_kind.to_string(),
                install_path: dir.to_string_lossy().into_owned(),
                contributions: ide,
            }));
        }
        Ok(None)
    }
}

fn parse_flexible_manifest(
    val: &serde_json::Value,
) -> std::result::Result<FlexibleManifest, serde_json::Error> {
    let str_field = |k: &str| {
        val.get(k)
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let contributions = match val.get("contributions") {
        Some(c) if !c.is_null() => {
            Some(serde_json::from_value::<IdePluginContributions>(c.clone())?)
        }
        _ => None,
    };
    let flattened: IdePluginContributions = serde_json::from_value(val.clone()).unwrap_or_default();
    Ok(FlexibleManifest {
        slug: str_field("slug"),
        name: str_field("name"),
        version: val
            .get("version")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .unwrap_or_else(default_version),
        description: str_field("description"),
        author: val
            .get("author")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        contributions,
        flattened,
    })
}

pub(super) fn write_normalized_manifest_if_needed(dest: &Path, info: &IdePluginInfo) -> Result<()> {
    let ide_manifest_path = dest.join(MANIFEST_FILE);
    if !ide_manifest_path.exists() {
        let normalized = IdePluginManifest {
            slug: info.slug.clone(),
            name: info.name.clone(),
            version: info.version.clone(),
            description: info.description.clone(),
            author: info.author.clone(),
            contributions: info.contributions.clone(),
        };
        let raw = serde_json::to_string_pretty(&normalized)?;
        std::fs::write(&ide_manifest_path, raw)
            .map_err(|e| Error::Internal(format!("write normalized manifest: {e}")))?;
    }
    Ok(())
}

pub(super) fn resolve_local_source_path(raw: &str) -> Result<PathBuf> {
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

pub(super) fn copy_plugin_dir(src: &Path, dst: &Path) -> Result<()> {
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
