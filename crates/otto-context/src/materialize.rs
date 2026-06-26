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
use otto_core::hooks::SpawnInjection;
use serde_json::{json, Value};

use crate::library::Library;
use crate::merge;

/// How many leading lines of an artifact's content the preview carries.
const PREVIEW_LINES: usize = 12;

/// The bundle file holding the materialized context block. Claude appends it to
/// the system prompt (`--append-system-prompt-file`), codex reads it inline
/// (`-c developer_instructions=…`). Agy instead uses `AGENTS.md` (see
/// [`context_file_name`]) because it auto-loads that name from an added dir.
const CONTEXT_FILE: &str = "CONTEXT.md";

/// The claude activity-hooks settings file, loaded out-of-tree via `--settings`.
const SETTINGS_FILE: &str = "settings.json";

/// Hard ceiling (in lines) on the context block Otto injects into EVERY session.
/// Otto must never bloat the agent's context window, so the assembled block is
/// capped here and the cap is asserted by a unit test — i.e. enforced on every
/// `cargo test` run (the commit/CI gate). This bounds ONLY Otto's injected
/// context; a user's own in-repo `CLAUDE.md`/`AGENTS.md` stays theirs and
/// uncapped (the CLIs still read it as project docs).
const MAX_CONTEXT_LINES: usize = 1000;

/// The bundle's context-file name for `provider`. Agy auto-loads `AGENTS.md`
/// from an `--add-dir` directory (verified), so it gets that name; claude/codex
/// receive the context another way and use the neutral [`CONTEXT_FILE`].
fn context_file_name(provider: &str) -> &'static str {
    match provider {
        "agy" => "AGENTS.md",
        _ => CONTEXT_FILE,
    }
}

/// The bundle subdirectory each CLI scans for skills (verified per client):
/// claude → `.claude/skills`, agy → `.agents/skills`, codex reads its skills
/// on demand from a plain `skills/` dir referenced by the context index.
fn skills_subdir(provider: &str) -> &'static str {
    match provider {
        "claude" => ".claude/skills",
        "agy" => ".agents/skills",
        _ => "skills",
    }
}

/// How `build_block` should render the skills section.
enum SkillIndex<'a> {
    /// Omit it — the CLI auto-loads the skill files as first-class skills
    /// (claude via `--add-dir` → `.claude/skills`, agy → `.agents/skills`).
    None,
    /// Emit a compact index (name + description + absolute `SKILL.md` path under
    /// `skills_dir`) for codex, which has no first-class out-of-tree skill
    /// loading and reads the referenced file on demand.
    Codex { skills_dir: &'a Path },
}

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
    /// The out-of-tree bundle dir this provider materializes into
    /// (`~/.otto/context/<provider>/<enc(cwd)>`). `None` when skipped.
    bundle: Option<PathBuf>,
    /// The resolved active skills (in library order).
    skills: Vec<LibrarySkill>,
    /// The active soul name, if any.
    soul_name: Option<String>,
    /// Skill artifacts to materialize into the bundle's per-provider skills dir.
    skill_artifacts: Vec<SkillArtifact>,
    /// The bundle skills dir the manifest is reconciled under.
    skills_dir: Option<PathBuf>,
    /// The bundle context file (`CONTEXT.md`/`AGENTS.md`): path + content. Unlike
    /// before, this is an Otto-owned file written verbatim — no merge into a
    /// user file, because nothing is written into the working tree anymore.
    instruction: Option<InstructionPlan>,
    /// The activity-hooks settings file (claude only): bundle path + JSON.
    hooks: Option<HooksPlan>,
}

/// The bundle context file a provider writes: its path and full content.
struct InstructionPlan {
    path: PathBuf,
    merged: String,
}

/// The activity-hooks settings file: its path and the merged JSON content.
struct HooksPlan {
    path: PathBuf,
    merged: String,
}

/// Default Otto context-bundle root: `~/.otto/context`. Falls back to the system
/// temp dir when `$HOME` is unset so a bundle root always exists.
pub fn default_context_root() -> PathBuf {
    match std::env::var("HOME").ok().filter(|h| !h.is_empty()) {
        Some(home) => Path::new(&home).join(".otto").join("context"),
        None => std::env::temp_dir().join("otto").join("context"),
    }
}

/// The out-of-tree bundle dir for `(provider, cwd)` under `ctx_root`:
/// `<ctx_root>/<provider>/<enc(cwd)>`. Deterministic, so the launch injection
/// can be reconstructed on resume without re-materializing.
fn bundle_dir(ctx_root: &Path, provider: &str, cwd: &str) -> PathBuf {
    ctx_root.join(provider).join(enc_project(cwd))
}

/// Materialize the active context for `provider` into its out-of-tree bundle
/// under `ctx_root` and return both the result and the [`SpawnInjection`] the
/// CLI needs to load that bundle. Nothing is written into `cwd`.
///
/// Providers other than `claude`/`codex`/`agy` (shell/…) are skipped: empty
/// result + empty injection.
pub fn provision(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
    provider: &str,
    ctx_root: &Path,
) -> (MaterializeProviderResult, SpawnInjection) {
    let plan = plan(library, cfg, cwd, provider, ctx_root);
    let bundle = plan.bundle.clone();
    let result = execute(plan);
    // Compute the injection AFTER writing — codex reads the materialized context
    // file back to pass it inline.
    let injection = match (&bundle, result.skipped) {
        (Some(dir), false) => injection_for(provider, dir),
        _ => SpawnInjection::default(),
    };
    (result, injection)
}

/// Recompute the launch injection for a RESUME from the already-materialized
/// bundle under `ctx_root` (the restart path has no `Workspace` and the bundle
/// persists across daemon restarts). Empty when no bundle exists.
pub fn resume_injection(ctx_root: &Path, cwd: &str, provider: &str) -> SpawnInjection {
    let dir = bundle_dir(ctx_root, provider, cwd);
    if dir.is_dir() {
        injection_for(provider, &dir)
    } else {
        SpawnInjection::default()
    }
}

/// The launch flags/env that make `provider`'s CLI load the bundle at `dir`.
/// Mirrors the per-client recipes verified against the live CLIs:
/// - **claude** — `--add-dir` loads skills from `<dir>/.claude/skills`; the
///   context is appended to the system prompt (claude does NOT load `CLAUDE.md`
///   from an added dir); activity hooks come via `--settings`.
/// - **agy** — `--add-dir` auto-loads `<dir>/AGENTS.md` AND `<dir>/.agents/skills`.
/// - **codex** — no flag loads an out-of-tree instructions file, so the context
///   text is passed inline via `-c developer_instructions=…`; `--add-dir` grants
///   read access to the skill files referenced by the index.
fn injection_for(provider: &str, dir: &Path) -> SpawnInjection {
    let d = dir.to_string_lossy().into_owned();
    let ctx = dir.join(context_file_name(provider));
    match provider {
        "claude" => {
            let mut args = vec![format!("--add-dir={d}")];
            if ctx.is_file() {
                args.push(format!("--append-system-prompt-file={}", ctx.display()));
            }
            let settings = dir.join(SETTINGS_FILE);
            if settings.is_file() {
                args.push(format!("--settings={}", settings.display()));
            }
            SpawnInjection { args, env: Vec::new() }
        }
        "agy" => SpawnInjection { args: vec![format!("--add-dir={d}")], env: Vec::new() },
        "codex" => {
            let mut args = vec![format!("--add-dir={d}")];
            if let Ok(text) = fs::read_to_string(&ctx) {
                let text = text.trim();
                if !text.is_empty() {
                    args.push("-c".to_string());
                    args.push(format!("developer_instructions={text}"));
                }
            }
            SpawnInjection { args, env: Vec::new() }
        }
        _ => SpawnInjection::default(),
    }
}

/// Dry-run: describe exactly what [`provision`] would write for `provider`,
/// without spawning a session or touching disk.
pub fn preview(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
    provider: &str,
    ctx_root: &Path,
) -> ContextPreviewProvider {
    describe(plan(library, cfg, cwd, provider, ctx_root))
}

/// Resolve every artifact `provider` would materialize for `cfg` into its bundle
/// under `ctx_root`, without writing anything. The single source of truth shared
/// by `provision` and `preview`.
fn plan(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
    provider: &str,
    ctx_root: &Path,
) -> ProviderPlan {
    match provider {
        "claude" | "codex" | "agy" => plan_provider(library, cfg, cwd, provider, ctx_root),
        other => ProviderPlan {
            provider: other.to_string(),
            skipped: true,
            bundle: None,
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

/// Build the markdown context block (soul + optional skills index + context +
/// repo rules + memory). The skills section is controlled by `index`:
/// [`SkillIndex::None`] omits it (claude/agy auto-load the skill files), while
/// [`SkillIndex::Codex`] emits a compact name+description+path index (codex has
/// no first-class out-of-tree skills and reads each referenced `SKILL.md` on
/// demand). Full skill bodies are NEVER inlined here — that is what pushed codex
/// past its hard 150k-char instructions limit; an index scales as the library
/// grows.
fn build_block(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
    soul_name: &Option<String>,
    skills: &[LibrarySkill],
    index: SkillIndex,
) -> String {
    let mut sections: Vec<String> = Vec::new();

    if let Some(soul) = active_soul_body(library, soul_name) {
        sections.push(format!("## Soul\n\n{}", soul.trim_end()));
    }

    if let SkillIndex::Codex { skills_dir } = index {
        if !skills.is_empty() {
            let mut buf = String::from(
                "## Skills\n\nThe following skills are available. When one is relevant to your \
                 task, read its full instructions from the referenced file before you start, \
                 then follow them.",
            );
            for skill in skills {
                buf.push_str(&format!("\n\n### {}", skill.name));
                let desc = skill.description.trim();
                if !desc.is_empty() {
                    buf.push_str(&format!("\n\n{desc}"));
                }
                let path = skills_dir.join(&skill.name).join("SKILL.md");
                buf.push_str(&format!("\n\nFull instructions: `{}`", path.display()));
            }
            sections.push(buf);
        }
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

    // Aider-style repo map — LAST (lowest priority) so the line budget trims it
    // before any user-authored section, and bounded by its own line cap. Opt-in.
    if cfg.include_repo_map {
        let opts = crate::repomap::RepoMapOptions {
            max_lines: cfg.repo_map_max_lines.unwrap_or(100),
            ..Default::default()
        };
        if let Some(map) = crate::repomap::repo_map_cached(std::path::Path::new(cwd), &opts) {
            let map = map.trim();
            if !map.is_empty() {
                sections.push(format!("## Repo Map (most-referenced symbols)\n\n```\n{map}\n```"));
            }
        }
    }

    enforce_line_budget(sections.join("\n\n"))
}

/// Enforce [`MAX_CONTEXT_LINES`] on the assembled context block. When the block
/// is over budget, keep the head — soul → skills index → context → repo rules →
/// memory, in descending priority — and replace the overflowing tail with a
/// single visible marker, so the cap holds no matter how large memory or the
/// workspace context grows. Returns `block` unchanged when already within
/// budget.
fn enforce_line_budget(block: String) -> String {
    let total = block.lines().count();
    if total <= MAX_CONTEXT_LINES {
        return block;
    }
    // Reserve two lines (a blank separator + the marker) so the result is at
    // most MAX_CONTEXT_LINES lines.
    let kept: Vec<&str> = block.lines().take(MAX_CONTEXT_LINES - 2).collect();
    let dropped = total - kept.len();
    format!(
        "{}\n\n_[Otto trimmed {dropped} line(s) to keep injected context within {MAX_CONTEXT_LINES} \
         lines — tighten the soul, skills, repo rules, or memory. Your own repo CLAUDE.md/AGENTS.md \
         is unaffected.]_",
        kept.join("\n"),
    )
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

/// Build the on-disk skill artifacts for `skills`, targeting `skills_dir`. For
/// each active skill, prefer copying the full library dir when present
/// (multi-file skills with references/assets/scripts), else write `skill.body` as
/// a single `SKILL.md`.
fn build_skill_artifacts(
    library: &Library,
    skills: &[LibrarySkill],
    skills_dir: &Path,
) -> Vec<SkillArtifact> {
    let mut skill_artifacts = Vec::new();
    for skill in skills {
        let dest_dir = skills_dir.join(&skill.name);
        let lib_skill_dir = library.root.join("skills").join(&skill.name);
        if lib_skill_dir.is_dir() {
            skill_artifacts.push(SkillArtifact::CopyDir { lib_dir: lib_skill_dir, dest_dir });
        } else {
            skill_artifacts.push(SkillArtifact::Body { dest_dir, body: skill.body.clone() });
        }
    }
    skill_artifacts
}

/// Plan the materialization for one of the three supported providers into its
/// out-of-tree bundle under `ctx_root` (no writes). Nothing targets `cwd`.
///
/// The per-client layout differs (verified against each live CLI):
/// - **claude** — `CONTEXT.md` (appended to the system prompt) + `.claude/skills`
///   (first-class via `--add-dir`) + `settings.json` (activity hooks via
///   `--settings`).
/// - **agy** — `AGENTS.md` (auto-loaded via `--add-dir`) + `.agents/skills`.
/// - **codex** — `CONTEXT.md` (passed inline via `-c developer_instructions`,
///   carrying a skills *index*) + `skills/` (read on demand).
fn plan_provider(
    library: &Library,
    cfg: &WorkspaceContextConfig,
    cwd: &str,
    provider: &str,
    ctx_root: &Path,
) -> ProviderPlan {
    let skills = active_skills(library, cfg);
    let soul_name = active_soul_name(library, cfg);
    let bundle = bundle_dir(ctx_root, provider, cwd);
    let skills_dir = bundle.join(skills_subdir(provider));
    let skill_artifacts = build_skill_artifacts(library, &skills, &skills_dir);

    // codex gets a skills index into its bundle skills dir; claude/agy auto-load
    // the skill files, so their block omits the section.
    let index = if provider == "codex" {
        SkillIndex::Codex { skills_dir: &skills_dir }
    } else {
        SkillIndex::None
    };
    let block = build_block(library, cfg, cwd, &soul_name, &skills, index);
    let instruction = Some(InstructionPlan {
        path: bundle.join(context_file_name(provider)),
        merged: block,
    });

    // Only claude wires activity hooks (trail/task ingest), delivered out-of-tree
    // via `--settings <bundle>/settings.json`.
    let hooks = if provider == "claude" { plan_claude_hooks(&bundle) } else { None };

    ProviderPlan {
        provider: provider.to_string(),
        skipped: false,
        bundle: Some(bundle),
        skills,
        soul_name,
        skill_artifacts,
        skills_dir: Some(skills_dir),
        instruction,
        hooks,
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

/// Plan Otto's activity hooks as the bundle's `settings.json`, loaded at launch
/// via `--settings`. This is an Otto-owned file, so there is normally nothing to
/// preserve — but the reconcile logic is kept (idempotent, and harmless if a
/// stale file exists). Returns the path + JSON, or `None` if an existing file is
/// malformed and must not be clobbered. No writes occur here.
fn plan_claude_hooks(bundle: &Path) -> Option<HooksPlan> {
    let path = bundle.join(SETTINGS_FILE);

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

    /// (library dir, cwd dir, ctx-root dir, Library). The ctx-root stands in for
    /// `~/.otto/context` so tests never touch the real home.
    fn setup() -> (TempDir, TempDir, TempDir, Library) {
        let lib_dir = TempDir::new().unwrap();
        let cwd_dir = TempDir::new().unwrap();
        let root_dir = TempDir::new().unwrap();
        let library = Library::new(lib_dir.path());
        (lib_dir, cwd_dir, root_dir, library)
    }

    /// The bundle dir a provision for `(provider, cwd)` would target under `root`.
    fn bundle_of(root: &TempDir, provider: &str, cwd: &str) -> PathBuf {
        root.path().join(provider).join(enc_project(cwd))
    }

    /// Assert the working tree was never written: no Otto context files in `cwd`.
    fn assert_clean_cwd(cwd: &Path) {
        for p in ["CLAUDE.md", "AGENTS.md", "CONTEXT.md", ".claude", ".agents", "settings.json"] {
            assert!(!cwd.join(p).exists(), "working tree polluted with {p}");
        }
    }

    #[test]
    fn enc_replaces_non_alphanumeric() {
        assert_eq!(enc_project("/Users/x/my proj"), "-Users-x-my-proj");
        assert_eq!(enc_project("abc123"), "abc123");
    }

    #[test]
    fn provision_never_writes_into_the_working_tree() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("otto", "Persona.").unwrap();
        lib.put_skill("triage", "---\ndescription: d\n---\nBODY").unwrap();
        let cfg = WorkspaceContextConfig { soul: Some("otto".into()), ..Default::default() };

        for provider in ["claude", "codex", "agy"] {
            let (res, inj) = provision(&lib, &cfg, &cwd_path, provider, root.path());
            assert!(!res.skipped, "{provider} should not skip");
            // Nothing in the repo cwd, ever.
            assert_clean_cwd(cwd.path());
            // The bundle exists out-of-tree and the injection points at it.
            let bundle = bundle_of(&root, provider, &cwd_path);
            assert!(bundle.is_dir(), "{provider} bundle missing");
            assert!(
                inj.args.iter().any(|a| a == &format!("--add-dir={}", bundle.display())),
                "{provider} injection missing --add-dir: {:?}",
                inj.args
            );
        }
    }

    #[test]
    fn claude_injection_appends_context_and_settings() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("otto", "Be terse.").unwrap();
        lib.put_skill("triage", "---\ndescription: d\n---\nskill body A").unwrap();
        let cfg = WorkspaceContextConfig { soul: Some("otto".into()), ..Default::default() };

        let (_res, inj) = provision(&lib, &cfg, &cwd_path, "claude", root.path());
        let bundle = bundle_of(&root, "claude", &cwd_path);

        // Context file holds the block; skills live under .claude/skills; the
        // skill body is NOT inlined into the context file.
        let ctx = fs::read_to_string(bundle.join("CONTEXT.md")).unwrap();
        assert!(ctx.contains("## Soul"));
        assert!(ctx.contains("Be terse."));
        assert!(!ctx.contains("skill body A"));
        assert!(bundle.join(".claude/skills/triage/SKILL.md").is_file());
        assert!(bundle.join("settings.json").is_file());

        // Launch flags: --add-dir + --append-system-prompt-file + --settings.
        assert!(inj.args.iter().any(|a| a == &format!("--add-dir={}", bundle.display())));
        assert!(inj.args.iter().any(|a| a.starts_with("--append-system-prompt-file=")
            && a.ends_with("CONTEXT.md")));
        assert!(inj.args.iter().any(|a| a.starts_with("--settings=") && a.ends_with("settings.json")));
    }

    #[test]
    fn agy_uses_agents_md_and_agents_skills_dir() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("otto", "Gemini persona.").unwrap();
        lib.put_skill("triage", "---\ndescription: d\n---\nbody").unwrap();
        let cfg = WorkspaceContextConfig { soul: Some("otto".into()), ..Default::default() };

        let (_res, inj) = provision(&lib, &cfg, &cwd_path, "agy", root.path());
        let bundle = bundle_of(&root, "agy", &cwd_path);

        // agy auto-loads AGENTS.md from --add-dir and scans .agents/skills.
        assert!(bundle.join("AGENTS.md").is_file());
        assert!(fs::read_to_string(bundle.join("AGENTS.md")).unwrap().contains("Gemini persona."));
        assert!(bundle.join(".agents/skills/triage/SKILL.md").is_file());
        // Only --add-dir is needed (no system-prompt flag).
        assert_eq!(inj.args, vec![format!("--add-dir={}", bundle.display())]);
    }

    #[test]
    fn codex_indexes_skills_and_passes_developer_instructions() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("otto", "Persona text.").unwrap();
        lib.put_skill(
            "triage",
            "---\ndescription: Triage incoming tickets\n---\nSKILL BODY ALPHA",
        )
        .unwrap();

        let cfg = WorkspaceContextConfig { soul: Some("otto".into()), ..Default::default() };
        let (res, inj) = provision(&lib, &cfg, &cwd_path, "codex", root.path());
        assert!(!res.skipped);
        let bundle = bundle_of(&root, "codex", &cwd_path);

        // Context file: soul + a compact skills index pointing at the bundle
        // skills dir; the full body is NOT inlined.
        let ctx = fs::read_to_string(bundle.join("CONTEXT.md")).unwrap();
        assert!(ctx.contains("## Soul"));
        assert!(ctx.contains("Persona text."));
        assert!(ctx.contains("### triage"));
        assert!(ctx.contains("Triage incoming tickets"));
        let skill_md = bundle.join("skills/triage/SKILL.md");
        assert!(ctx.contains(&skill_md.display().to_string()), "index points at the skill file");
        assert!(!ctx.contains("SKILL BODY ALPHA"));

        // The full body is materialized as a file the agent reads on demand.
        assert_eq!(
            fs::read_to_string(&skill_md).unwrap(),
            "---\ndescription: Triage incoming tickets\n---\nSKILL BODY ALPHA"
        );
        assert_eq!(
            merge::read_manifest(&bundle.join("skills")),
            vec!["triage".to_string()]
        );

        // Injection: --add-dir + the context passed inline via developer_instructions.
        assert!(inj.args.iter().any(|a| a == &format!("--add-dir={}", bundle.display())));
        let di = inj.args.windows(2).find(|w| w[0] == "-c").map(|w| w[1].clone());
        let di = di.expect("a `-c developer_instructions=…` arg");
        assert!(di.starts_with("developer_instructions="));
        assert!(di.contains("## Soul"));
    }

    #[test]
    fn claude_hooks_are_idempotent_in_the_bundle() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();

        // Provision twice — the second run must not duplicate Otto's groups.
        provision(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude", root.path());
        provision(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude", root.path());

        let bundle = bundle_of(&root, "claude", &cwd_path);
        let doc: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(bundle.join("settings.json")).unwrap()).unwrap();

        let post = doc["hooks"]["PostToolUse"].as_array().unwrap();
        let otto_groups = post
            .iter()
            .filter(|g| {
                g["hooks"][0]["command"]
                    .as_str()
                    .is_some_and(|c| c.contains(OTTO_HOOK_SENTINEL))
            })
            .count();
        assert_eq!(otto_groups, 1, "exactly one Otto group (idempotent)");
        assert!(doc["hooks"]["UserPromptSubmit"].is_array());
        assert!(doc["hooks"]["Stop"].is_array());
    }

    #[test]
    fn deactivating_skill_removes_its_dir() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_skill("triage", "A").unwrap();
        lib.put_skill("router", "B").unwrap();
        let skills_dir = bundle_of(&root, "claude", &cwd_path).join(".claude/skills");

        // First run: both active.
        provision(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude", root.path());
        assert!(skills_dir.join("triage/SKILL.md").exists());
        assert!(skills_dir.join("router/SKILL.md").exists());

        // Second run: only triage active → router's dir is reconciled away.
        let cfg_one = WorkspaceContextConfig {
            skills: Some(vec!["triage".into()]),
            ..Default::default()
        };
        provision(&lib, &cfg_one, &cwd_path, "claude", root.path());
        assert!(skills_dir.join("triage/SKILL.md").exists());
        assert!(!skills_dir.join("router").exists());
        assert_eq!(merge::read_manifest(&skills_dir), vec!["triage".to_string()]);
    }

    #[test]
    fn never_touches_a_foreign_skill_dir_in_the_bundle() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_skill("triage", "A").unwrap();

        // A pre-existing skill dir not in any Otto manifest.
        let foreign = bundle_of(&root, "claude", &cwd_path).join(".claude/skills/user-skill");
        fs::create_dir_all(&foreign).unwrap();
        fs::write(foreign.join("SKILL.md"), "mine").unwrap();

        provision(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude", root.path());
        assert!(foreign.join("SKILL.md").exists(), "unmanaged skill survives reconcile");
    }

    #[test]
    fn unknown_provider_is_skipped() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        let (res, inj) = provision(&lib, &WorkspaceContextConfig::default(), &cwd_path, "shell", root.path());
        assert!(res.skipped);
        assert!(res.files_written.is_empty());
        assert_eq!(res.provider, "shell");
        assert!(inj.args.is_empty());
    }

    #[test]
    fn soul_falls_back_to_default() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("global", "Default persona.").unwrap();
        lib.set_default_soul("global").unwrap();

        let cfg = WorkspaceContextConfig { soul: None, ..Default::default() };
        provision(&lib, &cfg, &cwd_path, "codex", root.path());
        let ctx = fs::read_to_string(bundle_of(&root, "codex", &cwd_path).join("CONTEXT.md")).unwrap();
        assert!(ctx.contains("Default persona."));
    }

    #[test]
    fn enforce_line_budget_caps_oversized_blocks() {
        // A pure check of the enforcer: any oversized block is cut to the cap
        // with a visible marker, and the head (highest-priority content) survives.
        let big = (0..5000).map(|i| format!("line {i}")).collect::<Vec<_>>().join("\n");
        let capped = enforce_line_budget(big);
        let n = capped.lines().count();
        assert!(n <= MAX_CONTEXT_LINES, "block has {n} lines, over the {MAX_CONTEXT_LINES} cap");
        assert!(capped.contains("line 0"), "head is preserved");
        assert!(!capped.contains("line 4999"), "the tail is dropped");
        assert!(capped.contains("Otto trimmed"), "truncation marker present");
    }

    #[test]
    fn enforce_line_budget_leaves_within_budget_blocks_untouched() {
        let small = "## Soul\n\nBe terse.".to_string();
        assert_eq!(enforce_line_budget(small.clone()), small);
    }

    #[test]
    fn provisioned_bundle_never_exceeds_the_line_budget() {
        // Even with a giant workspace context, the file Otto injects into every
        // session stays within the cap — the guarantee the commit gate enforces.
        // The soul at the head always survives; the bulky tail is what gets cut.
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("otto", "Persona.").unwrap();
        let huge = (0..4000).map(|i| format!("rule {i}")).collect::<Vec<_>>().join("\n");
        let cfg = WorkspaceContextConfig {
            soul: Some("otto".into()),
            extra_context_md: huge,
            ..Default::default()
        };

        for provider in ["claude", "codex", "agy"] {
            provision(&lib, &cfg, &cwd_path, provider, root.path());
            let bundle = bundle_of(&root, provider, &cwd_path);
            let ctx = fs::read_to_string(bundle.join(context_file_name(provider))).unwrap();
            let n = ctx.lines().count();
            assert!(n <= MAX_CONTEXT_LINES, "{provider} injected {n} lines, over the cap");
            assert!(ctx.contains("## Soul"), "{provider} keeps the soul at the head");
            assert!(ctx.contains("Otto trimmed"), "{provider} carries the truncation marker");
        }
    }

    #[test]
    fn repo_map_section_injected_only_when_enabled() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("otto", "Persona.").unwrap();
        // A tiny code repo in the cwd: `shared` is referenced by two callers.
        fs::write(cwd.path().join("core.rs"), "pub fn shared() {}\n").unwrap();
        fs::write(cwd.path().join("a.rs"), "fn a() { shared(); }\n").unwrap();
        fs::write(cwd.path().join("b.rs"), "fn b() { shared(); }\n").unwrap();

        // Disabled (default) → no repo map.
        let off = WorkspaceContextConfig { soul: Some("otto".into()), ..Default::default() };
        provision(&lib, &off, &cwd_path, "claude", root.path());
        let ctx_off = fs::read_to_string(
            bundle_of(&root, "claude", &cwd_path).join(context_file_name("claude")),
        )
        .unwrap();
        assert!(!ctx_off.contains("Repo Map"), "repo map must be opt-in");

        // Enabled → the ranked map appears and names the shared symbol.
        let on = WorkspaceContextConfig {
            soul: Some("otto".into()),
            include_repo_map: true,
            ..Default::default()
        };
        provision(&lib, &on, &cwd_path, "claude", root.path());
        let ctx_on = fs::read_to_string(
            bundle_of(&root, "claude", &cwd_path).join(context_file_name("claude")),
        )
        .unwrap();
        assert!(ctx_on.contains("## Repo Map"), "repo map section present:\n{ctx_on}");
        assert!(ctx_on.contains("shared"), "repo map names the referenced symbol");
        assert!(ctx_on.lines().count() <= MAX_CONTEXT_LINES);
    }

    #[test]
    fn resume_injection_reads_the_persisted_bundle() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("otto", "Persona.").unwrap();
        let cfg = WorkspaceContextConfig { soul: Some("otto".into()), ..Default::default() };

        // Before any provision there is no bundle → empty injection.
        assert!(resume_injection(root.path(), &cwd_path, "claude").args.is_empty());

        // After provision, resume reconstructs the same launch flags from disk.
        let (_res, fresh) = provision(&lib, &cfg, &cwd_path, "claude", root.path());
        let resumed = resume_injection(root.path(), &cwd_path, "claude");
        assert_eq!(resumed.args, fresh.args);
    }

    // -- preview (dry-run) ----------------------------------------------------

    #[test]
    fn preview_writes_nothing() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_skill("triage", "A").unwrap();

        let p = preview(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude", root.path());
        assert!(!p.skipped);
        // Nothing on disk — not in the cwd, not in the bundle root.
        assert_clean_cwd(cwd.path());
        assert!(!bundle_of(&root, "claude", &cwd_path).exists());
        // But the plan is described.
        assert!(p.files.iter().any(|f| f.kind == "skill"));
        assert!(p.files.iter().any(|f| f.kind == "instructions"));
        assert!(p.files.iter().any(|f| f.kind == "hooks"));
    }

    #[test]
    fn preview_matches_what_provision_writes() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_soul("otto", "Persona.").unwrap();
        lib.put_skill("triage", "BODY").unwrap();
        let cfg = WorkspaceContextConfig { soul: Some("otto".into()), ..Default::default() };

        let p = preview(&lib, &cfg, &cwd_path, "claude", root.path());
        let (res, _inj) = provision(&lib, &cfg, &cwd_path, "claude", root.path());

        // The preview's instruction content equals the bytes written to the bundle.
        let ctx = fs::read_to_string(bundle_of(&root, "claude", &cwd_path).join("CONTEXT.md")).unwrap();
        assert_eq!(p.generated_instructions, ctx);
        assert_eq!(p.instructions_file_name.as_deref(), Some("CONTEXT.md"));

        // Every previewed file path is among the files provision wrote.
        for f in &p.files {
            assert!(
                res.files_written.contains(&f.path),
                "previewed file {} not written by provision",
                f.path
            );
        }
        assert_eq!(p.skills.len(), 1);
        assert_eq!(p.skills[0].name, "triage");
        assert_eq!(p.soul.as_deref(), Some("otto"));
    }

    #[test]
    fn preview_labels_hooks_enforced_and_rest_advisory() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        lib.put_skill("triage", "A").unwrap();

        let p = preview(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude", root.path());
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
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        let p = preview(&lib, &WorkspaceContextConfig::default(), &cwd_path, "shell", root.path());
        assert!(p.skipped);
        assert!(p.files.is_empty());
    }

    #[test]
    fn preview_enumerates_multi_file_skill() {
        let (_l, cwd, root, lib) = setup();
        let cwd_path = cwd.path().to_string_lossy().into_owned();
        // A multi-file skill dir in the library (SKILL.md + a reference asset).
        let skill_dir = lib.root.join("skills").join("multi");
        fs::create_dir_all(skill_dir.join("references")).unwrap();
        fs::write(skill_dir.join("SKILL.md"), "skill md").unwrap();
        fs::write(skill_dir.join("references").join("ref.md"), "ref body").unwrap();

        let p = preview(&lib, &WorkspaceContextConfig::default(), &cwd_path, "claude", root.path());
        let kinds: Vec<&str> = p.files.iter().map(|f| f.kind.as_str()).collect();
        assert!(kinds.contains(&"skill"), "SKILL.md described");
        assert!(kinds.contains(&"skill_asset"), "asset described");
    }
}
