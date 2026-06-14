//! Resolve a proposal `target_ref` to a guarded absolute path. Rejects any
//! ref that could escape the workspace's skill dir or the workspace's memory
//! dir (no traversal, no absolute paths, single safe segments only).

use std::path::{Path, PathBuf};

use otto_core::domain::ImprovementTarget;
use otto_core::{Error, Result};
use otto_orchestrator::claude_pty::project_dir;

/// A skill `target_ref` must be a single safe name segment.
fn is_safe_segment(s: &str) -> bool {
    !s.is_empty()
        && s != "."
        && s != ".."
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// A memory `target_ref` must be a single safe `*.md` filename.
fn is_safe_memory_file(s: &str) -> bool {
    s.ends_with(".md") && is_safe_segment(s.trim_end_matches(".md"))
}

/// Resolve the absolute file path for an edit target.
///
/// * skill  → the Otto **library** entry `<library_root>/skills/<ref>/SKILL.md`
///   when `library_root` is given and that file exists (the library is the
///   source of truth — see the context-provisioning spec §8); otherwise the
///   workspace copy `<root>/.claude/skills/<ref>/SKILL.md`.
/// * memory → `<project_dir(root)>/memory/<ref>`  (claude's per-project memory)
pub fn resolve_target(
    root: &str,
    target: ImprovementTarget,
    target_ref: &str,
    library_root: Option<&Path>,
) -> Result<PathBuf> {
    match target {
        ImprovementTarget::Skill => {
            if !is_safe_segment(target_ref) {
                return Err(Error::Invalid(format!("unsafe skill ref '{target_ref}'")));
            }
            if let Some(lib) = library_root {
                let lib_path = lib.join("skills").join(target_ref).join("SKILL.md");
                if lib_path.exists() {
                    return Ok(lib_path);
                }
            }
            let base = Path::new(root).join(".claude").join("skills");
            Ok(base.join(target_ref).join("SKILL.md"))
        }
        ImprovementTarget::Memory => {
            if !is_safe_memory_file(target_ref) {
                return Err(Error::Invalid(format!("unsafe memory ref '{target_ref}'")));
            }
            Ok(project_dir(root).join("memory").join(target_ref))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_ref_resolves_under_workspace_without_library() {
        let p = resolve_target("/ws", ImprovementTarget::Skill, "support-triage-router", None)
            .unwrap();
        assert!(p.ends_with(".claude/skills/support-triage-router/SKILL.md"), "got {p:?}");
    }

    #[test]
    fn skill_ref_prefers_library_when_present() {
        let dir = tempfile::tempdir().unwrap();
        let lib = dir.path().join("library");
        let skill = lib.join("skills").join("support-triage-router");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), "x").unwrap();
        let p = resolve_target(
            "/ws",
            ImprovementTarget::Skill,
            "support-triage-router",
            Some(lib.as_path()),
        )
        .unwrap();
        assert_eq!(p, skill.join("SKILL.md"));

        // Falls back to the workspace when the library has no such skill.
        let p2 = resolve_target("/ws", ImprovementTarget::Skill, "other", Some(lib.as_path()))
            .unwrap();
        assert!(p2.ends_with(".claude/skills/other/SKILL.md"), "got {p2:?}");
    }

    #[test]
    fn memory_ref_resolves_under_project_memory() {
        let p = resolve_target("/ws", ImprovementTarget::Memory, "MEMORY.md", None).unwrap();
        let s = p.to_string_lossy();
        assert!(s.contains("/.claude/projects/"), "got {s}");
        assert!(s.ends_with("/memory/MEMORY.md"), "got {s}");
    }

    #[test]
    fn traversal_is_rejected() {
        assert!(resolve_target("/ws", ImprovementTarget::Skill, "../../etc", None).is_err());
        assert!(resolve_target("/ws", ImprovementTarget::Skill, "a/b", None).is_err());
        assert!(resolve_target("/ws", ImprovementTarget::Memory, "../secret.md", None).is_err());
        assert!(resolve_target("/ws", ImprovementTarget::Memory, "notes.txt", None).is_err());
    }
}
