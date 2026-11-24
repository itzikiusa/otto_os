//! Read-only access to on-disk **provider-global** skills — the ones the coding
//! CLIs discover from `~/.claude/skills`, `~/.codex/skills`, `~/.gemini/skills`
//! (agy's home is `~/.gemini` — see [`crate::user_skills`]) and Muse Code's
//! `$XDG_CONFIG_HOME/muse/skills` (default `~/.config/muse/skills`).
//!
//! The Skills Lab surfaces these alongside the Otto library + bundled catalog so
//! a user can view and review a skill that lives in a provider's dir (installed
//! by Otto, authored by hand, or shipped by the CLI). Otto never *edits* them
//! here — that stays the provider's / the user's job — it only reads them.

use std::path::PathBuf;

use otto_core::api::{ProviderSkillContent, ProviderSkillInfo, SkillFileContentResp};

use crate::library::{
    collect_files, is_binary, is_safe_segment, parse_category, parse_description, safe_rel,
};

/// Largest single provider-skill file returned to the viewer (2 MiB).
const MAX_FILE_BYTES: usize = 2 * 1024 * 1024;

/// The providers whose skills dir we enumerate (see [`provider_root`]).
pub const PROVIDERS: [&str; 4] = ["claude", "codex", "agy", "muse"];

/// The provider's skills dir, only for a recognized provider: `~/.claude/skills`,
/// `$CODEX_HOME/skills` (default `~/.codex`), and `~/.gemini/skills` for agy —
/// where Otto installs agy skills. It used to read `~/.agy/skills`, a dir
/// nothing writes, so agy skills never showed up in the Skills Lab. Uses `$HOME` directly to avoid a `dirs` dependency, mirroring
/// [`crate::user_skills`].
pub fn provider_root(provider: &str) -> Option<PathBuf> {
    if !PROVIDERS.contains(&provider) {
        return None;
    }
    let home = std::env::var("HOME")
        .ok()
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)?;
    if provider == "codex" {
        let base = std::env::var("CODEX_HOME")
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".codex"));
        Some(base.join("skills"))
    } else if provider == "agy" {
        Some(home.join(".gemini").join("skills"))
    } else if provider == "muse" {
        // Where `muse skills install` puts a user skill. (Muse also reads
        // ~/.claude/skills and ~/.codex/skills, so Otto installs nothing
        // here — see `user_skills`.)
        let base = std::env::var("XDG_CONFIG_HOME")
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"));
        Some(base.join("muse").join("skills"))
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
    list_from(PROVIDERS.iter().filter_map(|p| Some((*p, provider_root(p)?))))
}

/// [`list`] over explicit `(provider, skills root)` pairs.
fn list_from<'a>(roots: impl Iterator<Item = (&'a str, PathBuf)>) -> Vec<ProviderSkillInfo> {
    let mut out = Vec::new();
    for (provider, root) in roots {
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
                    if c.is_empty() {
                        "provider".to_string()
                    } else {
                        c
                    }
                },
                description: parse_description(&body),
                body_hash: otto_core::text::skill_body_hash(&body),
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
            if c.is_empty() {
                "provider".to_string()
            } else {
                c
            }
        },
        description: parse_description(&body),
        body,
        files,
    })
}

/// Read one file inside a provider skill (path-safe, symlink-safe, size-capped).
pub fn read_file(provider: &str, name: &str, rel: &str) -> Option<SkillFileContentResp> {
    let dir = skill_dir(provider, name)?;
    read_file_in(&dir, rel)
}

/// Inner bounded, symlink-rejecting file reader rooted at `dir`.
fn read_file_in(dir: &std::path::Path, rel: &str) -> Option<SkillFileContentResp> {
    use std::io::Read as _;

    let rel_path = safe_rel(rel)?;
    // Reject any symlink component along `rel` inside `dir` before opening.
    let mut cur = dir.to_path_buf();
    for comp in rel_path.components() {
        cur.push(comp);
        let meta = std::fs::symlink_metadata(&cur).ok()?;
        if meta.file_type().is_symlink() {
            return None;
        }
    }
    // Verify canonical confinement under canonical `dir` as belt-and-braces.
    let canon_dir = dir.canonicalize().ok()?;
    let target = otto_core::paths::confine_join(&canon_dir, rel)?;
    let canon_target = target.canonicalize().ok()?;
    if !canon_target.starts_with(&canon_dir) {
        return None;
    }
    let file = std::fs::File::open(&canon_target).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut buf = Vec::new();
    file.take((MAX_FILE_BYTES + 1) as u64)
        .read_to_end(&mut buf)
        .ok()?;
    buf.truncate(MAX_FILE_BYTES);
    Some(SkillFileContentResp {
        path: rel.to_string(),
        content: String::from_utf8_lossy(&buf).into_owned(),
        binary: is_binary(&buf),
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
        // agy lives under ~/.gemini (where user_skills installs it).
        if let Some(root) = provider_root("agy") {
            assert!(root.ends_with(".gemini/skills"), "{root:?}");
        }
        if let Some(root) = provider_root("muse") {
            assert!(root.ends_with("muse/skills"), "{root:?}");
        }
    }

    #[test]
    fn list_carries_the_normalised_body_hash() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill = tmp.path().join("my-skill");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "---\r\ndescription: d  \r\n---\r\n# Body\r\n").unwrap();
        let listed = list_from([("claude", tmp.path().to_path_buf())].into_iter());
        assert_eq!(listed.len(), 1);
        // The same text as the library holds it (LF, no trailing space).
        assert_eq!(listed[0].body_hash, otto_core::text::skill_body_hash("---\ndescription: d\n---\n# Body\n"));
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

    #[cfg(unix)]
    #[test]
    fn read_file_rejects_symlink_escape() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill = tmp.path().join("my-skill");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(skill.join("nested")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let secret = outside.join("secret.txt");
        std::fs::write(&secret, "top-secret").unwrap();
        std::fs::write(skill.join("SKILL.md"), "# ok").unwrap();

        // Direct file symlink escaping the skill dir.
        std::os::unix::fs::symlink(&secret, skill.join("escape.txt")).unwrap();
        // Directory symlink escaping the skill dir.
        std::os::unix::fs::symlink(&outside, skill.join("linked_dir")).unwrap();
        // Even an intra-skill symlink is rejected.
        std::os::unix::fs::symlink(skill.join("SKILL.md"), skill.join("alias.md")).unwrap();

        assert_eq!(read_file_in(&skill, "SKILL.md").unwrap().content, "# ok");
        assert!(read_file_in(&skill, "escape.txt").is_none());
        assert!(read_file_in(&skill, "linked_dir/secret.txt").is_none());
        assert!(read_file_in(&skill, "alias.md").is_none());
        assert!(read_file_in(&skill, "../outside/secret.txt").is_none());
    }

    #[test]
    fn read_file_bounds_read_to_max_file_bytes() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill = tmp.path().join("big-skill");
        std::fs::create_dir_all(&skill).unwrap();

        // Write more than MAX_FILE_BYTES and verify read_file_in caps at MAX_FILE_BYTES.
        let oversized = vec![b'a'; MAX_FILE_BYTES + 4096];
        std::fs::write(skill.join("big.txt"), &oversized).unwrap();

        let resp = read_file_in(&skill, "big.txt").expect("bounded read succeeds");
        assert_eq!(resp.path, "big.txt");
        assert_eq!(resp.content.len(), MAX_FILE_BYTES);
        assert!(!resp.binary);
    }
}
