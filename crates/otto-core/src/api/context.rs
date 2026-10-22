//! Context provisioning (skills + souls + context library, per-workspace config).
//!
//! Split out of `api.rs` by area (re-exported from there, so every path is
//! still `otto_core::api::*`). Mirrored by `ui/src/lib/api/types/skills-context.ts`.
use super::*;

// ---------------------------------------------------------------------------
// Context provisioning (skills + souls + context library, per-workspace config)
// ---------------------------------------------------------------------------

/// A skill entry in the Otto library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySkill {
    pub name: String,
    pub category: String,
    pub version: u32,
    pub description: String,
    pub body: String,
}

/// A soul (persona) entry in the Otto library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibrarySoul {
    pub name: String,
    pub body: String,
}

/// A reusable context snippet in the Otto library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryContext {
    pub name: String,
    pub body: String,
}

/// `PUT /library/skills/{name}` (and souls/context — only `body` is sent;
/// `name` comes from the path).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpsertLibraryEntryReq {
    pub body: String,
}

/// One file inside a skill package (relative to the skill root), for the Skills
/// Lab file tree. `binary` is a best-effort sniff (NUL byte in the head) — the
/// editor shows text files inline and offers binary files view-only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFileEntry {
    /// Path relative to the skill dir, e.g. `SKILL.md` or `references/rubric.md`.
    pub path: String,
    pub size: u64,
    pub binary: bool,
}

/// `GET /library/skills/{name}/file?path=…` — one file's text content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillFileContentResp {
    pub path: String,
    pub content: String,
    pub binary: bool,
}

/// `PUT /library/skills/{name}/file` — create/overwrite one file in the library
/// copy of a skill.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteSkillFileReq {
    /// Path relative to the skill dir (validated: no absolute, no `..`).
    pub path: String,
    pub content: String,
    /// "New file": refuse (409) instead of overwriting when `path` already
    /// exists — a typed existing path used to truncate it (even `SKILL.md`).
    #[serde(default)]
    pub create_only: bool,
}

/// `POST /library/skills` — create a new (empty-ish) library skill. When `body`
/// is omitted a starter `SKILL.md` is synthesized from name/category/description.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateLibrarySkillReq {
    pub name: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub body: Option<String>,
}

/// `GET /library/bundled/{name}` — a bundled skill's full content (so the Skills
/// Lab can view a skill that is not installed into the library yet).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundledSkillContent {
    pub name: String,
    pub category: String,
    pub version: u32,
    pub description: String,
    /// The `SKILL.md` body.
    pub body: String,
    pub files: Vec<SkillFileEntry>,
}

/// One on-disk provider-global skill (`~/.claude|$CODEX_HOME|~/.gemini` + `/skills/<name>/`),
/// surfaced read-only in the Skills Lab so it can be viewed and reviewed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSkillInfo {
    /// "claude" | "codex" | "agy".
    pub provider: String,
    pub name: String,
    pub category: String,
    pub description: String,
}

/// `GET /library/provider-skills/{provider}/{name}` — a provider skill's content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSkillContent {
    pub provider: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub body: String,
    pub files: Vec<SkillFileEntry>,
}

/// `POST /workspaces/{id}/skill-reviews` — start a skills review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartSkillReviewReq {
    pub skill_name: String,
    /// "library" | "bundled". Defaults to "library".
    #[serde(default)]
    pub skill_source: String,
    /// Provider agents to run (e.g. `["claude","codex"]`). Empty ⇒ static-only.
    #[serde(default)]
    pub providers: Vec<String>,
    /// "static" (deterministic only) | "agents". Defaults to "agents" when
    /// providers are given, else "static".
    #[serde(default)]
    pub agent_mode: String,
    /// Extra reviewer instructions appended to every agent + summarizer prompt
    /// (e.g. "check recent commits — they fix the previous review round").
    #[serde(default)]
    pub instructions: String,
}

/// `POST /skill-reviews/{id}/apply` — send the findings to an agent that
/// applies the fixes to the real skill directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApplySkillFixReq {
    /// Provider to run the fixer with. Defaults to "claude".
    #[serde(default)]
    pub provider: String,
    /// Extra apply-time instructions (on top of the review's own instructions).
    #[serde(default)]
    pub instructions: String,
}

/// `PUT /library/default-soul`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalSoulReq {
    pub name: String,
}

/// `GET /library/default-soul`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GlobalSoulResp {
    pub name: Option<String>,
}

/// Per-workspace context config, stored under `Workspace.settings.context`.
/// `skills = None` ⇒ all library skills active; `soul = None` ⇒ global default.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceContextConfig {
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default)]
    pub soul: Option<String>,
    #[serde(default)]
    pub extra_context_md: String,
    /// Curated knowledge shared by every agent in this workspace. References
    /// remain text; materialization never reads their filesystem/URL targets.
    #[serde(default)]
    pub goal_md: String,
    #[serde(default)]
    pub memory_md: String,
    #[serde(default)]
    pub decisions_md: String,
    #[serde(default)]
    pub references: Vec<String>,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub context_version: i64,
    #[serde(default = "default_include_memory")]
    pub include_memory: bool,
    /// Machine-managed block of repo rules (from code review). Rendered from the
    /// `repo_rules` table by the server on every rule change and injected into
    /// agent instruction files at spawn. **Kept separate from `extra_context_md`
    /// (which the user owns) so a user context edit never clobbers it**, and
    /// preserved by the `PUT /workspaces/{id}/context` handler.
    #[serde(default)]
    pub repo_rules_md: String,
    /// Inject an Aider-style tree-sitter + PageRank **repo map** (a ranked list
    /// of the repo's most-referenced symbols) into the agent's context. Opt-in
    /// (default `false`): it parses the repo on the spawn path, so it stays off
    /// until a user enables it. Bounded + cached; always the lowest-priority
    /// context section (trimmed first under the line budget).
    #[serde(default)]
    pub include_repo_map: bool,
    /// Cap on the repo map's emitted lines (default 100 when `None`).
    #[serde(default)]
    pub repo_map_max_lines: Option<usize>,
}

pub(super) fn default_include_memory() -> bool {
    true
}

impl Default for WorkspaceContextConfig {
    fn default() -> Self {
        Self {
            skills: None,
            soul: None,
            extra_context_md: String::new(),
            goal_md: String::new(),
            memory_md: String::new(),
            decisions_md: String::new(),
            references: Vec::new(),
            artifacts: Vec::new(),
            context_version: 0,
            include_memory: true,
            repo_rules_md: String::new(),
            include_repo_map: false,
            repo_map_max_lines: None,
        }
    }
}

/// `PUT /workspaces/{id}/context`
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateWorkspaceContextReq {
    #[serde(
        default,
        deserialize_with = "de_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub skills: Option<Option<Vec<String>>>,
    #[serde(
        default,
        deserialize_with = "de_double_option",
        skip_serializing_if = "Option::is_none"
    )]
    pub soul: Option<Option<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_context_md: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_memory: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_repo_map: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_md: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_md: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decisions_md: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub references: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<String>>,
    /// Expected revision. Required when editing curated knowledge; legacy
    /// clients may omit it while editing only legacy fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_version: Option<i64>,
}

/// One provider's result from a materialize action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeProviderResult {
    pub provider: String,
    pub files_written: Vec<String>,
    pub skipped: bool,
}

/// `POST /workspaces/{id}/context/materialize`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MaterializeResp {
    pub provider_results: Vec<MaterializeProviderResult>,
}

/// How binding a planned artifact is on the agent.
///
/// - `Advisory` — instruction files (`AGENTS.md`/`CLAUDE.md`) and skills:
///   guidance the model reads and *may ignore*.
/// - `Enforced` — hooks / runtime settings the daemon imposes regardless of
///   what the model decides (e.g. activity-forwarding hooks).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextEnforcement {
    Advisory,
    Enforced,
}

/// A single artifact `materialize` would write, described without writing it.
/// Used by the dry-run context preview to show exactly what a spawn produces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPlanFile {
    /// Absolute destination path the file would be written to.
    pub path: String,
    /// What this file is: `instructions` (AGENTS.md/CLAUDE.md), `skill`,
    /// `skill_asset` (a non-SKILL.md file in a multi-file skill dir), `hooks`,
    /// or `manifest` (Otto's managed-skill manifest).
    pub kind: String,
    /// Advisory (model may ignore) vs enforced (runtime imposes).
    pub enforcement: ContextEnforcement,
    /// Size in bytes of the content that would be written.
    pub size: u64,
    /// First lines of the content (a short excerpt for the preview list).
    pub first_lines: String,
    /// Whether the full content was elided from `first_lines` (file is larger).
    pub truncated: bool,
}

/// A skill selected for a workspace, summarized for the preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPlanSkill {
    pub name: String,
    pub description: String,
    pub version: u32,
}

/// `POST /workspaces/{id}/context/preview` — exactly what a session spawn would
/// materialize for one provider, computed WITHOUT spawning or writing anything.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPreviewProvider {
    pub provider: String,
    /// True for providers that materialize nothing (shell/agy/…).
    pub skipped: bool,
    /// The skills that would be activated, resolved from the library.
    pub skills: Vec<ContextPlanSkill>,
    /// The soul (persona) name that would apply, if any.
    pub soul: Option<String>,
    /// Every file the spawn would write (paths, sizes, excerpts, enforcement).
    pub files: Vec<ContextPlanFile>,
    /// The generated instruction-file content (the bytes of the Otto region as
    /// merged into AGENTS.md/CLAUDE.md), for an exact preview of what the model
    /// will read. Empty when the provider writes no instruction file.
    pub generated_instructions: String,
    /// Name of the instruction file (`CLAUDE.md` or `AGENTS.md`), if any.
    pub instructions_file_name: Option<String>,
    /// The hooks/settings JSON the runtime would impose (enforced), if any.
    pub generated_hooks: Option<String>,
}

/// `POST /workspaces/{id}/context/preview`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextPreviewResp {
    pub providers: Vec<ContextPreviewProvider>,
}

/// `POST /workspaces/{id}/context/preview` body. All fields optional: when
/// present they override the workspace's stored context selection so the UI can
/// preview a not-yet-saved choice (the same inputs a session spawn would use).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextPreviewReq {
    /// Provider(s) to preview; `None` ⇒ both `claude` and `codex`.
    #[serde(default)]
    pub provider: Option<String>,
    /// Override the active skill allow-list. Double-`Option` so an absent key
    /// (`None`) inherits the stored config, while an explicit `null`
    /// (`Some(None)`) clears the allow-list to mean *all* library skills.
    #[serde(default, deserialize_with = "de_double_option")]
    pub skills: Option<Option<Vec<String>>>,
    /// Override the active soul. Double-`Option` so an absent key (`None`)
    /// inherits the stored config, while an explicit `null` (`Some(None)`)
    /// clears the soul to mean the global default.
    #[serde(default, deserialize_with = "de_double_option")]
    pub soul: Option<Option<String>>,
    /// Override the extra-context markdown (`None` ⇒ use stored config).
    #[serde(default)]
    pub extra_context_md: Option<String>,
    #[serde(default)]
    pub goal_md: Option<String>,
    #[serde(default)]
    pub memory_md: Option<String>,
    #[serde(default)]
    pub decisions_md: Option<String>,
    #[serde(default)]
    pub references: Option<Vec<String>>,
    #[serde(default)]
    pub artifacts: Option<Vec<String>>,
    /// Override the include-memory toggle (`None` ⇒ use stored config).
    #[serde(default)]
    pub include_memory: Option<bool>,
    /// Override the include-repo-map toggle (`None` ⇒ use stored config).
    #[serde(default)]
    pub include_repo_map: Option<bool>,
    /// Working directory the spawn would use (`None` ⇒ the workspace root). The
    /// New Session sheet lets the user pick a cwd other than the workspace root;
    /// passing it here makes the preview match what that spawn would write.
    #[serde(default)]
    pub cwd: Option<String>,
}
