//! Per-provider materialization: resolve the active skills/soul/context for a
//! workspace and write them into the CLI's native on-disk form.
//!
//! The flow is split into a pure-ish **plan** and an effectful **provision**:
//! [`plan`] resolves the active library entries and computes every artifact a
//! spawn would produce — the instruction file (`CLAUDE.md`/`AGENTS.md`), the
//! skill files, the activity-hook settings — without touching disk. [`provision`]
//! executes that plan, and the dry-run preview ([`preview`]) describes it. Both
//! the real spawn path and the preview endpoint share `plan`, so what you see in
//! the preview is exactly what a session would get.
//!
//! All filesystem operations in `provision` are best-effort: a failure is logged
//! via `tracing::warn!` and materialization continues. The spawn path never
//! panics — degraded context beats no session.

use std::fs;
use std::path::{Path, PathBuf};

use otto_core::api::{
    ContextEnforcement, ContextPlanFile, ContextPlanSkill, ContextPreviewProvider, LibrarySkill,
    MaterializeProviderResult, WorkspaceContextConfig,
};
use serde_json::{json, Value};

use crate::library::Library;
use crate::merge;

/// How many leading lines of an artifact's content the preview carries.
const PREVIEW_LINES: usize = 12;

/// One skill artifact the plan wants on disk. Either a recursive copy of the
/// library skill dir (multi-file skills with references/assets/scripts) or a
/// single `SKILL.md` written from the in-memory body (legacy/in-memory skills).
enum SkillArtifact {
    /// Copy every file under `lib_dir` into `dest_dir`.
    CopyDir { lib_dir: PathBuf, dest_dir: PathBuf },
    /// Write `body` to a single `SKILL.md` under `dest_dir`.
    Body { dest_dir: PathBuf, body: String },
}

/// The full set of artifacts a spawn would materialize for one provider,
/// computed without writing anything. Shared by [`provision`] and [`preview`].
struct ProviderPlan {
    provider: String,
    skipped: bool,
    /// The resolved active skills (in library order).
    skills: Vec<LibrarySkill>,
    /// The active soul name, if any.
    soul_name: Option<String>,
    /// Skill artifacts to materialize (claude only; empty for codex).
    skill_artifacts: Vec<SkillArtifact>,
    /// Managed-skill manifest names + the skills dir it lives under (claude only).
    skills_dir: Option<PathBuf>,
    /// The instruction file (CLAUDE.md/AGENTS.md) path and the fully-merged
    /// content that would be written into it.
    instruction: Option<InstructionPlan>,
    /// The activity-hooks settings file (claude only): path + merged JSON.
    hooks: Option<HooksPlan>,
}

/// The instruction file a provider writes: its path, the Otto region block, and
/// the merged file content (region merged into whatever exists on disk now).
struct InstructionPlan {
    path: PathBuf,
    merged: String,
}

/// The activity-hooks settings file: its path and the merged JSON content.
struct HooksPlan {
    path: PathBuf,
    merged: String,
}

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
    let plan = plan(library, cfg, cwd, provider);
    let result = execute(plan);
    if !result.skipped {
        // Keep the per-session context Otto injects into the working tree out of
        // the user's git status (it's NOT theirs to commit).
        ensure_git_excludes(cwd);
    }
    result
}

/// Best-effort: add the Otto-materialized artifacts to the repo's LOCAL
/// `.git/info/exclude` so the per-session context Otto writes into the working
/// tree (skills, `CLAUDE.md`/`AGENTS.md`, `settings.local.json`) never shows up
/// as uncommitted changes. `info/exclude` is local-only — never committed, never
/// touches the repo's tracked `.gitignore`. Idempotent (a marker guards re-adds);
/// a no-op when `cwd` isn't a git repo. Tracked files are unaffected (git ignore
/// rules only apply to untracked paths), so a repo that genuinely commits its own
/// `CLAUDE.md` keeps showing its changes.
fn ensure_git_excludes(cwd: &str) {
    const MARKER: &str = "# Otto-managed: per-session injected context";
    const BLOCK: &str = "\n# Otto-managed: per-session injected context (do not commit) — local-only.\n\
.claude/skills/\n\
.claude/settings.local.json\n\
CLAUDE.md\n\
AGENTS.md\n";

    // Resolve the exact exclude file (handles worktrees / nested cwd correctly).
    let out = match std::process::Command::new("git")
        .current_dir(cwd)
        .args(["rev-parse", "--git-path", "info/exclude"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return, // not a git repo (or git unavailable) — nothing to exclude
    };
    let rel = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if rel.is_empty() {
        return;
    }
    let exclude_path = Path::new(cwd).join(rel);
    let existing = fs::read_to_string(&exclude_path).unwrap_or_default();
    if existing.contains(MARKER) {
        return; // already present
    }
    if let Some(parent) = exclude_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let content = format!("{existing}{BLOCK}");
    if let Err(e) = fs::write(&exclude_path, content) {
        tracing::warn!(path = %exclude_path.display(), error = %e, "write git exclude failed");
    }
}

/// Dry-run: describe exactly what [`provision`] would write for `provider`,
/// without spawning a session or touching disk.
pub fn preview(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
    provider: &str,
) -> ContextPreviewProvider {
    describe(plan(library, cfg, cwd, provider))
}

/// Resolve every artifact `provider` would materialize for `cfg` into `cwd`,
/// without writing anything. The single source of truth shared by `provision`
/// and `preview`.
fn plan(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
    provider: &str,
) -> ProviderPlan {
    match provider {
        "claude" => plan_claude(library, cfg, cwd),
        "codex" => plan_codex(library, cfg, cwd),
        other => ProviderPlan {
            provider: other.to_string(),
            skipped: true,
            skills: Vec::new(),
            soul_name: None,
            skill_artifacts: Vec::new(),
            skills_dir: None,
            instruction: None,
            hooks: None,
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

/// Resolve the active soul name: the workspace soul if set, else the global
/// default soul, else none. (Resolved to a name; the body is fetched separately.)
fn active_soul_name(library: &Library, cfg: &WorkspaceContextConfig) -> Option<String> {
    match &cfg.soul {
        Some(n) => Some(n.clone()),
        None => library.default_soul(),
    }
}

/// Resolve the active soul body, given the resolved name. `None` when the soul
/// is unset or doesn't exist in the library.
fn active_soul_body(library: &Library, name: &Option<String>) -> Option<String> {
    let name = name.as_ref()?;
    library.get_soul(name).map(|s| s.body)
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
    soul_name: &Option<String>,
    skills: &[LibrarySkill],
    include_skill_bodies: bool,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    if let Some(soul) = active_soul_body(library, soul_name) {
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

    // Repo rules generalized from code-review findings (machine-managed, separate
    // from the user-owned `extra_context_md`). This is the Context-Engine half of
    // the review loop: a lesson learned in review is enforced as context on every
    // subsequent agent run.
    let rules = cfg.repo_rules_md.trim();
    if !rules.is_empty() {
        sections.push(format!("## Repo Rules (from code review)\n\n{rules}"));
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

/// Plan the claude materialization for `cfg` into `cwd` (no writes).
fn plan_claude(library: &Library, cfg: &WorkspaceContextConfig, cwd: &str) -> ProviderPlan {
    let skills = active_skills(library, cfg);
    let soul_name = active_soul_name(library, cfg);
    let skills_dir = Path::new(cwd).join(".claude").join("skills");

    // For each active skill, prefer copying the full library dir when present
    // (multi-file skills), else write skill.body as a single SKILL.md.
    let mut skill_artifacts = Vec::new();
    for skill in &skills {
        let dest_dir = skills_dir.join(&skill.name);
        let lib_skill_dir = library.root.join("skills").join(&skill.name);
        if lib_skill_dir.is_dir() {
            skill_artifacts.push(SkillArtifact::CopyDir { lib_dir: lib_skill_dir, dest_dir });
        } else {
            skill_artifacts
                .push(SkillArtifact::Body { dest_dir, body: skill.body.clone() });
        }
    }

    // The instruction file (CLAUDE.md): merge the context block (no skill bodies
    // for claude — they live as files) into the existing file's Otto region.
    let block = build_block(library, cfg, cwd, &soul_name, &skills, false);
    let claude_md = Path::new(cwd).join("CLAUDE.md");
    let existing = fs::read_to_string(&claude_md).unwrap_or_default();
    let merged = merge::merge_otto_region(&existing, &block);

    // The activity-hooks settings file (enforced by the runtime).
    let hooks = plan_claude_hooks(cwd);

    ProviderPlan {
        provider: "claude".to_string(),
        skipped: false,
        skills,
        soul_name,
        skill_artifacts,
        skills_dir: Some(skills_dir),
        instruction: Some(InstructionPlan { path: claude_md, merged }),
        hooks,
    }
}

/// Plan the codex materialization for `cfg` into `cwd` (no writes).
fn plan_codex(library: &Library, cfg: &WorkspaceContextConfig, cwd: &str) -> ProviderPlan {
    let skills = active_skills(library, cfg);
    let soul_name = active_soul_name(library, cfg);

    // Codex has no skills dir: inline skill bodies into the block.
    let block = build_block(library, cfg, cwd, &soul_name, &skills, true);
    let agents_md = Path::new(cwd).join("AGENTS.md");
    let existing = fs::read_to_string(&agents_md).unwrap_or_default();
    let merged = merge::merge_otto_region(&existing, &block);

    ProviderPlan {
        provider: "codex".to_string(),
        skipped: false,
        skills,
        soul_name,
        skill_artifacts: Vec::new(),
        skills_dir: None,
        instruction: Some(InstructionPlan { path: agents_md, merged }),
        hooks: None,
    }
}

/// Execute a plan: write every artifact best-effort and report what landed.
fn execute(plan: ProviderPlan) -> MaterializeProviderResult {
    if plan.skipped {
        return MaterializeProviderResult {
            provider: plan.provider,
            files_written: Vec::new(),
            skipped: true,
        };
    }

    let mut files_written: Vec<String> = Vec::new();

    // Skills + manifest reconciliation (claude).
    if let Some(skills_dir) = &plan.skills_dir {
        let active_names: Vec<String> = plan.skills.iter().map(|s| s.name.clone()).collect();
        for (artifact, skill) in plan.skill_artifacts.iter().zip(plan.skills.iter()) {
            match artifact {
                SkillArtifact::CopyDir { lib_dir, dest_dir } => {
                    // Remove the old managed copy and re-copy the whole library
                    // dir so references/assets/scripts stay in sync.
                    if dest_dir.exists() {
                        if let Err(e) = fs::remove_dir_all(dest_dir) {
                            tracing::warn!(skill = %skill.name, error = %e, "remove existing skill dir failed");
                            continue;
                        }
                    }
                    match copy_dir_all(lib_dir, dest_dir) {
                        Ok(copied) => files_written.extend(copied),
                        Err(e) => {
                            tracing::warn!(skill = %skill.name, error = %e, "copy skill dir failed")
                        }
                    }
                }
                SkillArtifact::Body { dest_dir, body } => {
                    if let Err(e) = fs::create_dir_all(dest_dir) {
                        tracing::warn!(skill = %skill.name, error = %e, "create skill dir failed");
                        continue;
                    }
                    let path = dest_dir.join("SKILL.md");
                    match fs::write(&path, body) {
                        Ok(()) => files_written.push(path.to_string_lossy().into_owned()),
                        Err(e) => {
                            tracing::warn!(path = %path.display(), error = %e, "write skill failed")
                        }
                    }
                }
            }
        }

        // Reconcile the manifest: remove skill dirs we previously managed that
        // are no longer active. Never touch a dir not in the old manifest.
        let old = merge::read_manifest(skills_dir);
        for stale in old.iter().filter(|n| !active_names.contains(n)) {
            let dir = skills_dir.join(stale);
            if let Err(e) = fs::remove_dir_all(&dir) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(skill = %stale, error = %e, "remove stale skill dir failed");
                }
            }
        }
        if let Err(e) = merge::write_manifest(skills_dir, &active_names) {
            tracing::warn!(error = %e, "write skill manifest failed");
        } else {
            files_written
                .push(skills_dir.join(".otto-managed.json").to_string_lossy().into_owned());
        }
    }

    // Instruction file (CLAUDE.md / AGENTS.md).
    if let Some(instr) = &plan.instruction {
        if write_file(&instr.path, &instr.merged) {
            files_written.push(instr.path.to_string_lossy().into_owned());
        }
    }

    // Activity hooks (enforced).
    if let Some(hooks) = &plan.hooks {
        if write_file(&hooks.path, &hooks.merged) {
            files_written.push(hooks.path.to_string_lossy().into_owned());
        }
    }

    MaterializeProviderResult {
        provider: plan.provider,
        files_written,
        skipped: false,
    }
}

/// Describe a plan as the dry-run preview the API returns.
fn describe(plan: ProviderPlan) -> ContextPreviewProvider {
    if plan.skipped {
        return ContextPreviewProvider {
            provider: plan.provider,
            skipped: true,
            skills: Vec::new(),
            soul: None,
            files: Vec::new(),
            generated_instructions: String::new(),
            instructions_file_name: None,
            generated_hooks: None,
        };
    }

    let mut files: Vec<ContextPlanFile> = Vec::new();

    // Skill files (advisory). For copy-dir skills, enumerate the source tree so
    // the preview lists each file the copy would land (SKILL.md + assets); for
    // body skills, the single SKILL.md.
    for artifact in &plan.skill_artifacts {
        match artifact {
            SkillArtifact::CopyDir { lib_dir, dest_dir } => {
                describe_copy_dir(lib_dir, lib_dir, dest_dir, &mut files);
            }
            SkillArtifact::Body { dest_dir, body } => {
                files.push(plan_file(
                    dest_dir.join("SKILL.md"),
                    "skill",
                    ContextEnforcement::Advisory,
                    body,
                ));
            }
        }
    }

    // Managed-skill manifest (advisory — it's Otto bookkeeping, not a runtime
    // constraint on the agent).
    if let Some(skills_dir) = &plan.skills_dir {
        let names: Vec<String> = plan.skills.iter().map(|s| s.name.clone()).collect();
        let manifest = serde_json::to_string_pretty(&json!({ "skills": names }))
            .unwrap_or_else(|_| "{}".to_string());
        files.push(plan_file(
            skills_dir.join(".otto-managed.json"),
            "manifest",
            ContextEnforcement::Advisory,
            &manifest,
        ));
    }

    // Instruction file (advisory).
    let (generated_instructions, instructions_file_name) = match &plan.instruction {
        Some(instr) => {
            files.push(plan_file(
                instr.path.clone(),
                "instructions",
                ContextEnforcement::Advisory,
                &instr.merged,
            ));
            let name = instr
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned());
            (instr.merged.clone(), name)
        }
        None => (String::new(), None),
    };

    // Hooks / settings (enforced).
    let generated_hooks = match &plan.hooks {
        Some(hooks) => {
            files.push(plan_file(
                hooks.path.clone(),
                "hooks",
                ContextEnforcement::Enforced,
                &hooks.merged,
            ));
            Some(hooks.merged.clone())
        }
        None => None,
    };

    ContextPreviewProvider {
        provider: plan.provider,
        skipped: false,
        skills: plan
            .skills
            .iter()
            .map(|s| ContextPlanSkill {
                name: s.name.clone(),
                description: s.description.clone(),
                version: s.version,
            })
            .collect(),
        soul: plan.soul_name,
        files,
        generated_instructions,
        instructions_file_name,
        generated_hooks,
    }
}

/// Walk `src` (relative to `root`) and push a `ContextPlanFile` for every file,
/// targeting the matching path under `dest`. Best-effort; unreadable entries are
/// skipped. Mirrors `copy_dir_all`'s traversal so the preview matches the copy.
fn describe_copy_dir(root: &Path, src: &Path, dest: &Path, out: &mut Vec<ContextPlanFile>) {
    let entries = match fs::read_dir(src) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            describe_copy_dir(root, &path, dest, out);
        } else if let Ok(rel) = path.strip_prefix(root) {
            let dest_path = dest.join(rel);
            let body = fs::read_to_string(&path).unwrap_or_default();
            // A SKILL.md at the skill root is the skill itself; everything else
            // is a supporting asset.
            let kind = if rel.as_os_str() == "SKILL.md" { "skill" } else { "skill_asset" };
            out.push(plan_file(dest_path, kind, ContextEnforcement::Advisory, &body));
        }
    }
}

/// Build a `ContextPlanFile` describing `content` at `path` without writing it.
fn plan_file(
    path: PathBuf,
    kind: &str,
    enforcement: ContextEnforcement,
    content: &str,
) -> ContextPlanFile {
    let size = content.len() as u64;
    let all: Vec<&str> = content.lines().collect();
    let truncated = all.len() > PREVIEW_LINES;
    let first_lines = all
        .iter()
        .take(PREVIEW_LINES)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    ContextPlanFile {
        path: path.to_string_lossy().into_owned(),
        kind: kind.to_string(),
        enforcement,
        size,
        first_lines,
        truncated,
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

/// Plan the merge of Otto's activity hooks into
/// `<cwd>/.claude/settings.local.json`, preserving every other setting and any
/// user-authored hooks. Returns the path + merged JSON, or `None` when the
/// existing file is malformed and must not be clobbered. No writes occur here.
fn plan_claude_hooks(cwd: &str) -> Option<HooksPlan> {
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
        let arr = hooks.entry(event.to_string()).or_insert_with(|| json!([]));
        if !arr.is_array() {
            *arr = json!([]);
        }
        let groups = arr.as_array_mut()?;
        groups.retain(|g| !is_otto_group(g));
        groups.push(otto_hook_group(*with_matcher));
    }

    let merged = serde_json::to_string_pretty(&doc).ok()?;
    Some(HooksPlan { path, merged })
}

/// Write `content` to `path`, creating parents as needed. Best-effort: logs and
/// returns `false` on failure.
fn write_file(path: &Path, content: &str) -> bool {
    if let Some(parent) = path.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            tracing::warn!(path = %path.display(), error = %e, "create parent dir failed");
            return false;
        }
    }
    match fs::write(path, content) {
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
    fn git_exclude_added_once_in_a_repo_noop_outside() {
        // Outside a git repo → no-op (no panic, nothing written).
        let plain = TempDir::new().unwrap();
        ensure_git_excludes(plain.path().to_str().unwrap());
        assert!(!plain.path().join(".git/info/exclude").exists());

        // Inside a repo → the Otto block lands in .git/info/exclude, idempotently.
        let repo = TempDir::new().unwrap();
        let p = repo.path().to_str().unwrap();
        let ok = std::process::Command::new("git")
            .current_dir(p)
            .arg("init")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            return; // git not available in this environment
        }
        ensure_git_excludes(p);
        ensure_git_excludes(p); // second call must not duplicate
        let excl = std::fs::read_to_string(repo.path().join(".git/info/exclude")).unwrap();
        assert!(excl.contains(".claude/skills/"));
        assert!(excl.contains("CLAUDE.md"));
        assert_eq!(
            excl.matches("# Otto-managed: per-session injected context").count(),
            1
        );
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
            repo_rules_md: String::new(),
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

    // -- preview (dry-run) ----------------------------------------------------

    #[test]
    fn preview_writes_nothing() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_skill("triage", "A").unwrap();

        let p = preview(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude");
        assert!(!p.skipped);
        // Nothing on disk.
        assert!(!cwd.path().join("CLAUDE.md").exists());
        assert!(!cwd.path().join(".claude").exists());
        // But the plan is described.
        assert!(p.files.iter().any(|f| f.kind == "skill"));
        assert!(p.files.iter().any(|f| f.kind == "instructions"));
        assert!(p.files.iter().any(|f| f.kind == "hooks"));
    }

    #[test]
    fn preview_matches_what_provision_writes() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("otto", "Persona.").unwrap();
        lib.put_skill("triage", "BODY").unwrap();
        let cfg = WorkspaceContextConfig { soul: Some("otto".into()), ..Default::default() };

        let p = preview(&lib, &cfg, &cwd_path, "claude");
        let res = provision(&lib, &cfg, &cwd_path, "claude");

        // The preview's instruction content equals the bytes on disk.
        let claude = fs::read_to_string(cwd.path().join("CLAUDE.md")).unwrap();
        assert_eq!(p.generated_instructions, claude);
        assert_eq!(p.instructions_file_name.as_deref(), Some("CLAUDE.md"));

        // Every previewed file path is among the files provision wrote.
        for f in &p.files {
            assert!(
                res.files_written.contains(&f.path),
                "previewed file {} not written by provision",
                f.path
            );
        }
        // Skill + soul surfaced.
        assert_eq!(p.skills.len(), 1);
        assert_eq!(p.skills[0].name, "triage");
        assert_eq!(p.soul.as_deref(), Some("otto"));
    }

    #[test]
    fn preview_labels_hooks_enforced_and_rest_advisory() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_skill("triage", "A").unwrap();

        let p = preview(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude");
        for f in &p.files {
            match f.kind.as_str() {
                "hooks" => assert_eq!(f.enforcement, ContextEnforcement::Enforced),
                _ => assert_eq!(f.enforcement, ContextEnforcement::Advisory),
            }
        }
        assert!(p.generated_hooks.is_some());
    }

    #[test]
    fn preview_skips_shell() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        let p = preview(&lib, &WorkspaceContextConfig::default(), &cwd_path, "shell");
        assert!(p.skipped);
        assert!(p.files.is_empty());
    }

    #[test]
    fn preview_enumerates_multi_file_skill() {
        let (_l, cwd, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        // A multi-file skill dir in the library (SKILL.md + a reference asset).
        let skill_dir = lib.root.join("skills").join("multi");
        fs::create_dir_all(skill_dir.join("references")).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "skill md").unwrap();
        fs::write(skill_dir.join("references").join("ref.md"), "ref body").unwrap();

        let p = preview(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude");
        let kinds: Vec<&str> = p.files.iter().map(|f| f.kind.as_str()).collect();
        assert!(kinds.contains(&"skill"), "SKILL.md described");
        assert!(kinds.contains(&"skill_asset"), "asset described");
    }
}
