//! Read-only access to on-disk **provider-global** skills — the ones the coding
//! CLIs discover from `~/.claude/skills`, `~/.codex/skills`, `~/.agy/skills`.
//!
//! The Skills Lab surfaces these alongside the Otto library + bundled catalog so
//! a user can view and review a skill that lives in a provider's dir (installed
//! by Otto, authored by hand, or shipped by the CLI). Otto never *edits* them
//! here — that stays the provider's / the user's job — it only reads them.

use std::path::PathBuf;

use otto_core::api::{ProviderSkillContent, ProviderSkillInfo, SkillFileContentResp};

use crate::library::{collect_files, is_binary, is_safe_segment, parse_category, parse_description, safe_rel};

/// Largest single provider-skill file returned to the viewer (2 MiB).
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

/// The providers whose `~/.<provider>/skills` dir we enumerate.
pub const PROVIDERS: [&str; 3] = ["claude", "codex", "agy"];

/// `~/.<provider>/skills` (codex honors `$CODEX_HOME`), only for a recognized
/// provider. Uses `$HOME` directly to avoid a `dirs` dependency, mirroring
/// [`crate::user_skills`].
pub fn provider_root(provider: &str) -> Option<PathBuf> {
    if !PROVIDERS.contains(&provider) {
        return None;
    }
    let home = std::env::var("HOME").ok().filter(|h| !h.is_empty()).map(PathBuf::from)?;
    if provider == "codex" {
        let base = std::env::var("CODEX_HOME")
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        Some(base.join("skills"))
    } else {
        Some(home.join(format!(".{provider}")).join("skills"))
    }
}

/// Absolute dir of a provider skill (need not exist); `None` for unsafe input.
pub fn skill_dir(provider: &str, name: &str) -> Option<PathBuf> {
    if !is_safe_segment(name) {
        return None;
    }
    Some(provider_root(provider)?.join(name))
}

/// Every provider skill across all providers, sorted by `(provider, name)`.
pub fn list() -> Vec<ProviderSkillInfo> {
    let mut out = Vec::new();
    for provider in PROVIDERS {
        let Some(root) = provider_root(provider) else {
            continue;
        };
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for e in entries.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            let name = e.file_name().to_string_lossy().into_owned();
            if !is_safe_segment(&name) {
                continue;
            }
            let skill_md = e.path().join("SKILL.md");
            let Ok(body) = std::fs::read_to_string(&skill_md) else {
                continue;
            };
            out.push(ProviderSkillInfo {
                provider: provider.to_string(),
                name,
                category: {
                    let c = parse_category(&body);
                    if c.is_empty() { "provider".to_string() } else { c }
                },
                description: parse_description(&body),
            });
        }
    }
    out.sort_by(|a, b| (a.provider.as_str(), a.name.as_str()).cmp(&(&b.provider, &b.name)));
    out
}

/// A provider skill's SKILL.md body + its file tree.
pub fn content(provider: &str, name: &str) -> Option<ProviderSkillContent> {
    let dir = skill_dir(provider, name)?;
    let body = std::fs::read_to_string(dir.join("SKILL.md")).ok()?;
    let mut files = Vec::new();
    collect_files(&dir, &dir, &mut files);
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Some(ProviderSkillContent {
        provider: provider.to_string(),
        name: name.to_string(),
        category: {
            let c = parse_category(&body);
            if c.is_empty() { "provider".to_string() } else { c }
        },
        description: parse_description(&body),
        body,
        files,
    })
}

/// Read one file inside a provider skill (path-safe, size-capped).
pub fn read_file(provider: &str, name: &str, rel: &str) -> Option<SkillFileContentResp> {
    let dir = skill_dir(provider, name)?;
    let target = dir.join(safe_rel(rel)?);
    let bytes = std::fs::read(&target).ok()?;
    let bytes = &bytes[..bytes.len().min(MAX_FILE_BYTES)];
    Some(SkillFileContentResp {
        path: rel.to_string(),
        content: String::from_utf8_lossy(bytes).into_owned(),
        binary: is_binary(bytes),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_known_providers_resolve() {
        // Unknown providers never resolve (no fs access needed).
        assert!(provider_root("gemini").is_none());
        assert!(provider_root("../etc").is_none());
        assert!(skill_dir("gemini", "x").is_none());
        // Known providers resolve to a ".../skills" root.
        for p in PROVIDERS {
            if let Some(root) = provider_root(p) {
                assert!(root.ends_with("skills"), "{p} root should end in skills");
            }
        }
    }

    #[test]
    fn skill_dir_rejects_unsafe_names() {
        // A known provider + unsafe name never escapes the skills root.
        assert!(skill_dir("claude", "../evil").is_none());
        assert!(skill_dir("claude", "a/b").is_none());
        assert!(skill_dir("claude", "").is_none());
        // A safe name resolves under ~/.claude/skills.
        if let Some(dir) = skill_dir("claude", "my-skill") {
            assert!(dir.ends_with(".claude/skills/my-skill"));
        }
    }
}
