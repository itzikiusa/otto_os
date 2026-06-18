//! Per-provider materialization: resolve the active skills/soul/context for a
//! workspace and write them into the CLI's native on-disk form.
//!
//! All filesystem operations are best-effort: a failure is logged via
//! `tracing::warn!` and materialization continues. This function never panics —
//! it is reachable from the spawn path, where degraded context beats no session.

use std::fs;
use std::path::Path;

use otto_core::api::{LibrarySkill, MaterializeProviderResult, WorkspaceContextConfig};
use serde_json::{json, Value};

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

/// Recursively copy all files from `src` into `dest`, creating parent
/// directories as needed. Returns the list of destination paths written.
/// On Unix, `.sh` files receive the executable bit (0o755).
fn copy_dir_all(src: &Path, dest: &Path) -> std::io::Result<Vec<String>> {
    let mut written = Vec::new();
    fs::create_dir_all(dest)?;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dest_path = dest.join(entry.file_name());

        if src_path.is_dir() {
            written.extend(copy_dir_all(&src_path, &dest_path)?);
        } else {
            fs::write(&dest_path, fs::read(&src_path)?)?;

            #[cfg(unix)]
            if dest_path.extension().and_then(|e| e.to_str()) == Some("sh") {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&dest_path, fs::Permissions::from_mode(0o755))?;
            }

            written.push(dest_path.to_string_lossy().into_owned());
        }
    }
    Ok(written)
}

fn provision_claude(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
) -> MaterializeProviderResult {
    let mut files_written: Vec<String> = Vec::new();
    let skills = active_skills(library, cfg);
    let skills_dir = Path::new(cwd).join(".claude").join("skills");

    // Write each active skill: prefer a full recursive dir copy from the library
    // when the library skill dir exists (multi-file skills with references/
    // assets/ scripts/). Fall back to writing skill.body as SKILL.md alone for
    // legacy skills that have only a SKILL.md in the library (or were created via
    // put_skill) — backward-compatible.
    let active_names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
    for skill in &skills {
        let dest_dir = skills_dir.join(&skill.name);
        let lib_skill_dir = library.root.join("skills").join(&skill.name);

        if lib_skill_dir.is_dir() {
            // Multi-file skill: remove the old managed copy and re-copy the whole
            // library dir so references/ assets/ scripts/ stay in sync.
            if dest_dir.exists() {
                if let Err(e) = fs::remove_dir_all(&dest_dir) {
                    tracing::warn!(skill = %skill.name, error = %e, "remove existing skill dir failed");
                    continue;
                }
            }
            match copy_dir_all(&lib_skill_dir, &dest_dir) {
                Ok(copied) => files_written.extend(copied),
                Err(e) => tracing::warn!(skill = %skill.name, error = %e, "copy skill dir failed"),
            }
        } else {
            // Legacy / in-memory skill: write only SKILL.md.
            if let Err(e) = fs::create_dir_all(&dest_dir) {
                tracing::warn!(skill = %skill.name, error = %e, "create skill dir failed");
                continue;
            }
            let path = dest_dir.join("SKILL.md");
            match fs::write(&path, &skill.body) {
                Ok(()) => files_written.push(path.to_string_lossy().into_owned()),
                Err(e) => tracing::warn!(path = %path.display(), error = %e, "write skill failed"),
            }
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

    // Wire Otto's activity hooks into .claude/settings.local.json so the agent
    // forwards skills/commands/tools/prompts/tasks to the per-session ingest
    // endpoint (the "live trail" + task tracker). Best-effort.
    if let Some(p) = write_claude_hooks(cwd) {
        files_written.push(p);
    }

    MaterializeProviderResult {
        provider: "claude".to_string(),
        files_written,
        skipped: false,
    }
}

/// Sentinel present in every Otto-managed hook command, used to detect and
/// reconcile our entries without disturbing user-authored hooks.
const OTTO_HOOK_SENTINEL: &str = "OTTO_INGEST_TOKEN";

/// The single shell command every Otto hook runs: forward the hook's JSON
/// payload (on stdin) to the per-session ingest endpoint. It reads the daemon
/// URL + session id + token from env vars Otto sets on the agent's PTY, never
/// blocks the tool (always `exit 0`), and is a no-op when run outside Otto
/// (env unset → the `[ -n … ]` guard short-circuits).
fn otto_hook_command() -> String {
    "[ -n \"$OTTO_INGEST_TOKEN\" ] && curl -sS -m 3 -X POST \
     \"$OTTO_INGEST_BASE/api/v1/ingest/claude\" \
     -H \"X-Otto-Session: $OTTO_SESSION_ID\" -H \"X-Otto-Token: $OTTO_INGEST_TOKEN\" \
     -H \"Content-Type: application/json\" --data-binary @- >/dev/null 2>&1; exit 0"
        .to_string()
}

/// True when `group` is an Otto-managed hook matcher-group (its command carries
/// our sentinel) — so reconciliation can drop+rewrite ours and keep the rest.
fn is_otto_group(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(|h| h.as_array())
        .is_some_and(|hooks| {
            hooks.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|c| c.contains(OTTO_HOOK_SENTINEL))
            })
        })
}

/// One Otto matcher-group. `with_matcher` is set for tool-name events
/// (PostToolUse) so it fires for every tool; omitted for the others.
fn otto_hook_group(with_matcher: bool) -> Value {
    let entry = json!({ "type": "command", "command": otto_hook_command(), "timeout": 5 });
    if with_matcher {
        json!({ "matcher": "*", "hooks": [entry] })
    } else {
        json!({ "hooks": [entry] })
    }
}

/// Merge Otto's activity hooks into `<cwd>/.claude/settings.local.json`,
/// preserving every other setting and any user-authored hooks. Idempotent:
/// re-running replaces only Otto's groups. Returns the file path on success.
fn write_claude_hooks(cwd: &str) -> Option<String> {
    let path = Path::new(cwd).join(".claude").join("settings.local.json");

    let mut doc: Value = match fs::read_to_string(&path) {
        Ok(s) if !s.trim().is_empty() => serde_json::from_str(&s).unwrap_or_else(|e| {
            tracing::warn!(path = %path.display(), error = %e, "settings.local.json is not valid JSON; leaving hooks unset");
            Value::Null
        }),
        _ => json!({}),
    };
    // Don't risk clobbering a malformed file we couldn't parse.
    if !doc.is_object() {
        if doc.is_null() {
            return None;
        }
        doc = json!({});
    }

    let obj = doc.as_object_mut()?;
    let hooks = obj
        .entry("hooks")
        .or_insert_with(|| json!({}))
        .as_object_mut()?;

    // (event name, fire for every tool via matcher "*")
    const EVENTS: &[(&str, bool)] = &[
        ("PostToolUse", true),
        ("UserPromptSubmit", false),
        ("SessionStart", false),
        ("Stop", false),
        ("Notification", false),
    ];
    for (event, with_matcher) in EVENTS {
        let arr = hooks
            .entry(event.to_string())
            .or_insert_with(|| json!([]));
        if !arr.is_array() {
            *arr = json!([]);
        }
        let groups = arr.as_array_mut()?;
        groups.retain(|g| !is_otto_group(g));
        groups.push(otto_hook_group(*with_matcher));
    }

    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!(path = %path.display(), error = %e, "create .claude dir failed");
            return None;
        }
    }
    let body = serde_json::to_string_pretty(&doc).ok()?;
    match fs::write(&path, body) {
        Ok(()) => Some(path.to_string_lossy().into_owned()),
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "write settings.local.json failed");
            None
        }
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
    fn claude_hooks_are_added_preserved_and_idempotent() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        let settings = cwd.path().join(".claude").join("settings.local.json");

        // Pre-seed a user-authored settings file with a top-level key AND a
        // user PostToolUse hook that Otto must never disturb.
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(
            &settings,
            r#"{ "model": "opus", "hooks": { "PostToolUse": [
                { "matcher": "Bash", "hooks": [ { "type": "command", "command": "echo mine" } ] }
            ] } }"#,
        )
        .unwrap();

        // Provision twice — the second run must not duplicate Otto's groups.
        provision(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude");
        provision(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude");

        let doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();

        // User settings survive.
        assert_eq!(doc["model"], "opus");

        let post = doc["hooks"]["PostToolUse"].as_array().unwrap();
        let user_groups = post
            .iter()
            .filter(|g| g["hooks"][0]["command"] == "echo mine")
            .count();
        let otto_groups = post
            .iter()
            .filter(|g| {
                g["hooks"][0]["command"]
                    .as_str()
                    .is_some_and(|c| c.contains(OTTO_HOOK_SENTINEL))
            })
            .count();
        assert_eq!(user_groups, 1, "user hook preserved");
        assert_eq!(otto_groups, 1, "exactly one Otto group (idempotent)");

        // Other lifecycle events got an Otto hook too.
        assert!(doc["hooks"]["UserPromptSubmit"].is_array());
        assert!(doc["hooks"]["Stop"].is_array());
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
