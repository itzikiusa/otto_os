//! Per-provider materialization: resolve the active skills/soul/context for a
//! workspace and write them into the CLI's native on-disk form.
//!
//! All filesystem operations are best-effort: a failure is logged via
//! `tracing::warn!` and materialization continues. This function never panics —
//! it is reachable from the spawn path, where degraded context beats no session.

use std::fs;
use std::path::Path;

use otto_core::api::{LibrarySkill, MaterializeProviderResult, WorkspaceContextConfig};

use crate::library::Library;
use crate::merge;

/// Materialize the active context for `provider` into `cwd`.
///
/// Providers other than `claude`/`codex` (shell/agy/…) are skipped and return
/// `skipped = true` with no files written.
pub fn provision(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
    provider: &str,
) -> MaterializeProviderResult {
    match provider {
        "claude" => provision_claude(library, cfg, cwd),
        "codex" => provision_codex(library, cfg, cwd),
        other => MaterializeProviderResult {
            provider: other.to_string(),
            files_written: Vec::new(),
            skipped: true,
        },
    }
}

/// Resolve the active skill entries for `cfg`: the explicit allow-list if set,
/// else every library skill. Names that don't resolve to a library skill are
/// dropped.
fn active_skills(library: &Library, cfg: &WorkspaceContextConfig) -> Vec<LibrarySkill> {
    match &cfg.skills {
        Some(names) => names.iter().filter_map(|n| library.get_skill(n)).collect(),
        None => library.list_skills(),
    }
}

/// Resolve the active soul body: the workspace soul if set and present, else
/// the global default soul, else none.
fn active_soul_body(library: &Library, cfg: &WorkspaceContextConfig) -> Option<String> {
    let name = match &cfg.soul {
        Some(n) => Some(n.clone()),
        None => library.default_soul(),
    }?;
    library.get_soul(&name).map(|s| s.body)
}

/// Encode a cwd into claude's per-project directory name: every non-alphanumeric
/// char becomes `-`. Mirrors `otto_orchestrator::claude_pty::project_dir`.
fn enc_project(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// Best-effort read of the workspace `MEMORY.md` at
/// `~/.claude/projects/<enc(cwd)>/memory/MEMORY.md`. Missing ⇒ `None`.
fn read_memory(cwd: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let path = Path::new(&home)
        .join(".claude")
        .join("projects")
        .join(enc_project(cwd))
        .join("memory")
        .join("MEMORY.md");
    fs::read_to_string(path).ok()
}

/// Build the markdown context block. When `include_skill_bodies` is true (codex)
/// each active skill's body is inlined under `## Skills`; for claude the skill
/// bodies live as files and are omitted here.
fn build_block(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
    skills: &[LibrarySkill],
    include_skill_bodies: bool,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    if let Some(soul) = active_soul_body(library, cfg) {
        sections.push(format!("## Soul\n\n{}", soul.trim_end()));
    }

    if include_skill_bodies && !skills.is_empty() {
        let mut buf = String::from("## Skills");
        for skill in skills {
            buf.push_str(&format!("\n\n### {}\n\n{}", skill.name, skill.body.trim_end()));
        }
        sections.push(buf);
    }

    let extra = cfg.extra_context_md.trim();
    if !extra.is_empty() {
        sections.push(format!("## Context\n\n{extra}"));
    }

    if cfg.include_memory {
        if let Some(mem) = read_memory(cwd) {
            let mem = mem.trim();
            if !mem.is_empty() {
                sections.push(format!("## Memory\n\n{mem}"));
            }
        }
    }

    sections.join("\n\n")
}

fn provision_claude(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
) -> MaterializeProviderResult {
    let mut files_written: Vec<String> = Vec::new();
    let skills = active_skills(library, cfg);
    let skills_dir = Path::new(cwd).join(".claude").join("skills");

    // Write each active skill's SKILL.md (overwrite — library is source).
    let active_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
    for skill in &skills {
        let dir = skills_dir.join(&skill.name);
        if let Err(e) = fs::create_dir_all(&dir) {
            tracing::warn!(skill = %skill.name, error = %e, "create skill dir failed");
            continue;
        }
        let path = dir.join("SKILL.md");
        match fs::write(&path, &skill.body) {
            Ok(()) => files_written.push(path.to_string_lossy().into_owned()),
            Err(e) => tracing::warn!(path = %path.display(), error = %e, "write skill failed"),
        }
    }

    // Reconcile the manifest: remove skill dirs we previously managed that are
    // no longer active. Never touch a dir not in the old manifest.
    let old = merge::read_manifest(&skills_dir);
    for stale in old.iter().filter(|n| !active_names.contains(n)) {
        let dir = skills_dir.join(stale);
        if let Err(e) = fs::remove_dir_all(&dir) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(skill = %stale, error = %e, "remove stale skill dir failed");
            }
        }
    }
    if let Err(e) = merge::write_manifest(&skills_dir, &active_names) {
        tracing::warn!(error = %e, "write skill manifest failed");
    } else {
        files_written.push(skills_dir.join(".otto-managed.json").to_string_lossy().into_owned());
    }

    // Merge the context block (no skill bodies) into CLAUDE.md.
    let block = build_block(library, cfg, cwd, &skills, false);
    let claude_md = Path::new(cwd).join("CLAUDE.md");
    if write_region(&claude_md, &block) {
        files_written.push(claude_md.to_string_lossy().into_owned());
    }

    MaterializeProviderResult {
        provider: "claude".to_string(),
        files_written,
        skipped: false,
    }
}

fn provision_codex(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
) -> MaterializeProviderResult {
    let mut files_written: Vec<String> = Vec::new();
    let skills = active_skills(library, cfg);

    // Codex has no skills dir: inline skill bodies into the block.
    let block = build_block(library, cfg, cwd, &skills, true);
    let agents_md = Path::new(cwd).join("AGENTS.md");
    if write_region(&agents_md, &block) {
        files_written.push(agents_md.to_string_lossy().into_owned());
    }

    MaterializeProviderResult {
        provider: "codex".to_string(),
        files_written,
        skipped: false,
    }
}

/// Read `path`, merge the Otto region with `block`, and write it back.
/// Best-effort: logs and returns `false` on write failure.
fn write_region(path: &Path, block: &str) -> bool {
    let existing = fs::read_to_string(path).unwrap_or_default();
    let merged = merge::merge_otto_region(&existing, block);
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!(path = %path.display(), error = %e, "create parent dir failed");
            return false;
        }
    }
    match fs::write(path, merged) {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "write context file failed");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn setup() -> (TempDir, TempDir, Library) {
        let lib_dir = TempDir::new().unwrap();
        let cwd_dir = TempDir::new().unwrap();
        let library = Library::new(lib_dir.path());
        (lib_dir, cwd_dir, library)
    }

    #[test]
    fn enc_replaces_non_alphanumeric() {
        assert_eq!(enc_project("/Users/x/my proj"), "-Users-x-my-proj");
        assert_eq!(enc_project("abc123"), "abc123");
    }

    #[test]
    fn claude_writes_skills_manifest_and_region() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("otto", "Be terse and helpful.").unwrap();
        lib.put_skill("triage", "---\ndescription: x\n---\nskill body A").unwrap();
        lib.put_skill("router", "router body B").unwrap();

        // A pre-existing hand-written CLAUDE.md line outside the region.
        fs::write(cwd.path().join("CLAUDE.md"), "# Project rules\nKEEP THIS LINE\n").unwrap();

        let cfg = WorkspaceContextConfig {
            skills: None, // all
            soul: Some("otto".into()),
            extra_context_md: String::new(),
            include_memory: false,
        };
        let res = provision(&lib, &cfg, &cwd_path, "claude");
        assert!(!res.skipped);

        // Skill files written.
        let triage = cwd.path().join(".claude/skills/triage/SKILL.md");
        let router = cwd.path().join(".claude/skills/router/SKILL.md");
        assert_eq!(fs::read_to_string(&triage).unwrap(), "---\ndescription: x\n---\nskill body A");
        assert!(router.exists());

        // Manifest correct (sorted by list_skills).
        let manifest = merge::read_manifest(&cwd.path().join(".claude/skills"));
        assert_eq!(manifest, vec!["router".to_string(), "triage".to_string()]);

        // CLAUDE.md OTTO region has the soul; hand-written line survives.
        let claude = fs::read_to_string(cwd.path().join("CLAUDE.md")).unwrap();
        assert!(claude.contains("KEEP THIS LINE"));
        assert!(claude.contains("## Soul"));
        assert!(claude.contains("Be terse and helpful."));
        // Skill bodies are NOT inlined for claude.
        assert!(!claude.contains("skill body A"));
    }

    #[test]
    fn claude_deactivating_skill_removes_its_dir() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_skill("triage", "A").unwrap();
        lib.put_skill("router", "B").unwrap();

        // First run: both active.
        let cfg_all = WorkspaceContextConfig::default();
        provision(&lib, &cfg_all, &cwd_path, "claude");
        assert!(cwd.path().join(".claude/skills/triage/SKILL.md").exists());
        assert!(cwd.path().join(".claude/skills/router/SKILL.md").exists());

        // Second run: only triage active.
        let cfg_one = WorkspaceContextConfig {
            skills: Some(vec!["triage".into()]),
            ..Default::default()
        };
        provision(&lib, &cfg_one, &cwd_path, "claude");
        assert!(cwd.path().join(".claude/skills/triage/SKILL.md").exists());
        assert!(!cwd.path().join(".claude/skills/router").exists());
        assert_eq!(
            merge::read_manifest(&cwd.path().join(".claude/skills")),
            vec!["triage".to_string()]
        );
    }

    #[test]
    fn claude_never_touches_foreign_skill_dir() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_skill("triage", "A").unwrap();

        // A user-authored skill dir not in any Otto manifest.
        let foreign = cwd.path().join(".claude/skills/user-skill");
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join("SKILL.md"), "mine").unwrap();

        provision(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude");
        // Foreign skill survives even though it isn't active.
        assert!(foreign.join("SKILL.md").exists());
    }

    #[test]
    fn codex_inlines_skills_and_writes_agents_md_only() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("otto", "Persona text.").unwrap();
        lib.put_skill("triage", "SKILL BODY ALPHA").unwrap();

        let cfg = WorkspaceContextConfig {
            soul: Some("otto".into()),
            ..Default::default()
        };
        let res = provision(&lib, &cfg, &cwd_path, "codex");
        assert!(!res.skipped);

        let agents = fs::read_to_string(cwd.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("## Soul"));
        assert!(agents.contains("Persona text."));
        assert!(agents.contains("## Skills"));
        assert!(agents.contains("SKILL BODY ALPHA"));

        // No .claude/skills created for codex.
        assert!(!cwd.path().join(".claude/skills").exists());
    }

    #[test]
    fn unknown_provider_is_skipped() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        let res = provision(&lib, &WorkspaceContextConfig::default(), &cwd_path, "shell");
        assert!(res.skipped);
        assert!(res.files_written.is_empty());
        assert_eq!(res.provider, "shell");
    }

    #[test]
    fn soul_falls_back_to_default() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("global", "Default persona.").unwrap();
        lib.set_default_soul("global").unwrap();

        let cfg = WorkspaceContextConfig { soul: None, ..Default::default() };
        provision(&lib, &cfg, &cwd_path, "codex");
        let agents = fs::read_to_string(cwd.path().join("AGENTS.md")).unwrap();
        assert!(agents.contains("Default persona."));
    }
}
