// Contract mirror: skills-context. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Agent self-improvement
// ---------------------------------------------------------------------------

export type Autonomy = 'tiered' | 'propose' | 'auto';
/** What kicked off an improvement run: the schedule, a person, or the in-loop evolver ('live', after a watched interaction concludes). */
export type ImprovementTrigger = 'scheduled' | 'manual' | 'live';
export type ImprovementRunStatus = 'running' | 'done' | 'skipped' | 'failed';
export type ImprovementTarget = 'skill' | 'memory';
export type ImprovementEditKind = 'add' | 'modify' | 'remove';
export type ImprovementRisk = 'low' | 'structural';
export type ImprovementEditStatus =
  | 'pending'
  | 'applied'
  | 'rejected'
  | 'rolled_back'
  | 'conflict';

export interface SelfImprovementConfig {
  enabled: boolean;
  cadence_minutes: number;
  lookback_hours: number;
  skill_allowlist: string[];
  autonomy: Autonomy;
  /** Agent CLIs to run the analysis on — one set of suggestions per provider. */
  providers: string[];
  live_evolve: boolean;
  last_run_at: string | null;
  next_run_at: string | null;
}

export interface UpdateSelfImprovementReq {
  enabled: boolean;
  cadence_minutes: number;
  lookback_hours: number;
  skill_allowlist: string[];
  autonomy: Autonomy;
  providers: string[];
  live_evolve: boolean;
}

export interface ImprovementRun {
  id: string;
  workspace_id: string;
  trigger: ImprovementTrigger;
  status: ImprovementRunStatus;
  summary: string;
  sessions_reviewed: number;
  applied: number;
  pending: number;
  error: string | null;
  started_at: string;
  finished_at: string | null;
}

export interface ImprovementEdit {
  id: string;
  run_id: string;
  workspace_id: string;
  target: ImprovementTarget;
  target_ref: string;
  target_path: string;
  kind: ImprovementEditKind;
  risk: ImprovementRisk;
  status: ImprovementEditStatus;
  rationale: string;
  evidence: string[];
  before_content: string | null;
  after_content: string;
  applied_at: string | null;
  actor: string | null;
  created_at: string;
}

export interface RunNowResp {
  run_id: string;
}

// ---------------------------------------------------------------------------
// Context provisioning (library + per-workspace context/soul)
// ---------------------------------------------------------------------------

export interface LibrarySkill {
  name: string;
  description: string;
  body: string;
  /** Present on `/library/skills` responses (frontmatter `category:`). */
  category?: string;
  /** Present on `/library/skills` responses (frontmatter `version:`). */
  version?: number;
}

// --- Skills Lab: multi-file skill editor + skill review --------------------

/** One file inside a skill package, relative to its root (Skills Lab tree). */
export interface SkillFileEntry {
  path: string;
  size: number;
  binary: boolean;
}

export interface SkillFileContentResp {
  path: string;
  content: string;
  binary: boolean;
}

export interface WriteSkillFileReq {
  path: string;
  content: string;
  /** "New file": 409 instead of overwriting an existing `path`. */
  create_only?: boolean;
}

export interface CreateLibrarySkillReq {
  name: string;
  category?: string;
  description?: string;
  body?: string;
}

/** One on-disk provider-global skill (~/.claude, $CODEX_HOME, ~/.gemini for agy), read-only. */
export interface ProviderSkillInfo {
  provider: string;
  name: string;
  category: string;
  description: string;
}

/** One entry of `GET /sessions/{id}/slash-commands` — what the chat composer completes after `/`. */
export interface SlashCommand {
  /** Without the leading slash (`compact`, `commit`, `frontend:component`). */
  name: string;
  description: string;
  /** `builtin` = the provider CLI's own; `user` = `~/.claude/{commands,skills}`; `project` = the session cwd's `.claude/…`. */
  source: 'builtin' | 'user' | 'project';
}

/**
 * `GET /sessions/{id}/screen` — the session's CURRENT terminal screen as plain
 * text (no scrollback), for the Home "Otto School" monitors. Not live ⇒
 * `{live: false, cols: 0, rows: 0, lines: []}` (the read never spawns/resumes).
 */
export interface SessionScreen {
  /** The session has a live PTY right now. */
  live: boolean;
  /** Emulator size; `0` when not live. */
  cols: number;
  rows: number;
  /** Plain rows top→bottom: trailing blank rows dropped, each ≤ 240 chars, the LAST ≤ 60 rows. */
  lines: string[];
}

/** A provider skill's full content (body + file list). */
export interface ProviderSkillContent {
  provider: string;
  name: string;
  category: string;
  description: string;
  body: string;
  files: SkillFileEntry[];
}

/** One bundled skill's full content (view without installing). */
export interface BundledSkillContent {
  name: string;
  category: string;
  version: number;
  description: string;
  body: string;
  files: SkillFileEntry[];
}

/** One row from `GET /library/bundled` (catalog + drift). */
export interface BundledSkillView {
  name: string;
  category: string;
  version: number;
  description: string;
  installed_version: number | null;
  /** "not_installed" | "up_to_date" | "update_available" | "ahead". */
  state: string;
  update_available: boolean;
}

export interface StartSkillReviewReq {
  skill_name: string;
  /** "library" | "bundled". */
  skill_source: string;
  providers: string[];
  /** "static" | "agents". */
  agent_mode: string;
  /** Extra reviewer instructions appended to every agent + summarizer prompt. */
  instructions?: string;
}

export interface ApplySkillFixReq {
  /** Provider to run the fixer with. Defaults to "claude". */
  provider?: string;
  /** Extra apply-time instructions (on top of the review's own instructions). */
  instructions?: string;
}

export interface SkillFinding {
  /** "Critical" | "High" | "Medium" | "Low". */
  severity: string;
  code: string;
  title: string;
  evidence: string;
  why: string;
  fix: string;
}

export interface SkillScoreRow {
  area: string;
  score: number;
  notes: string;
}

export interface SkillStaticReport {
  /** "Ready" | "Ready with fixes" | "Do not publish". */
  verdict: string;
  average_score: number;
  scorecard: SkillScoreRow[];
  findings: SkillFinding[];
}

export interface SkillReviewAgent {
  name: string;
  provider: string;
  model: string;
  /** "pending" | "running" | "waiting" | "done" | "error". */
  status: string;
  note: string;
  session_id?: string | null;
  findings: SkillFinding[];
}

export interface SkillReviewSummary {
  verdict: string;
  average_score: number;
  scorecard: SkillScoreRow[];
  findings: SkillFinding[];
  patch_plan: string[];
}

export interface SkillReview {
  id: Id;
  workspace_id: Id;
  skill_name: string;
  /** "library" | "bundled". */
  skill_source: string;
  /** "running" | "done" | "error" | "cancelled". */
  status: string;
  /** "static" | "agents". */
  agent_mode: string;
  /** Extra reviewer instructions this review ran with ("" when none). */
  instructions: string;
  agents: SkillReviewAgent[];
  /** The apply-fixes agent (present once "Apply fixes" has been triggered). */
  fix_agent?: SkillReviewAgent | null;
  static_report: SkillStaticReport | null;
  summary: SkillReviewSummary | null;
  error: string | null;
  created_at: string;
  updated_at: string;
}

export interface LibrarySoul {
  name: string;
  body: string;
}

export interface LibraryContext {
  name: string;
  body: string;
}

export interface UpsertLibraryEntryReq {
  body: string;
}

// --- Bundled skills library (Settings → Skills) ----------------------------

/** Install state of a bundled skill vs. the installed library copy. */
export type BundledSkillState =
  | 'not_installed'
  | 'up_to_date'
  | 'update_available'
  | 'ahead';

/** A skill shipped with Otto, with its state relative to the installed copy. */
export interface BundledSkill {
  name: string;
  category: string;
  version: number;
  description: string;
  /** Version of the currently-installed copy, or null if not installed. */
  installed_version: number | null;
  state: BundledSkillState;
  /**
   * True iff the bundle is strictly newer than the installed copy
   * (`bundled > installed`) — the `update_available` state. Lets the UI show an
   * "Update" button. A hand-edited copy that is `ahead` is NOT an update.
   */
  update_available: boolean;
}

/** Result of installing a single bundled skill. */
export interface InstallBundledResp {
  name: string;
  installed: boolean;
  backed_up: boolean;
  /** Path of the backup taken before overwriting, when backed_up is true. */
  backup_path: string | null;
  /** Provider skill paths (`~/.claude/skills/<name>`, …) left untouched because a user-owned skill of that name lives there. Omitted when empty. */
  user_owned?: string[];
}

/** Result of installing every bundled skill (optionally a single category). */
export interface InstallAllBundledResp {
  installed: string[];
  backed_up: string[];
  /**
   * Library copies already at the bundled version, or ahead of it (local edits)
   * without `force`. Those Library files stay untouched. Provider mirrors for a
   * skipped skill are still reconciled.
   */
  skipped: string[];
  /** A Library install or a provider-mirror reconcile failed; the rest of the batch still ran. */
  failed: { name: string; error: string }[];
  /** `<provider skills dir>/<name>` paths left untouched: a user-owned skill of that name already lives there. */
  user_owned: string[];
}

export interface GlobalSoulReq {
  name: string;
}

export interface GlobalSoulResp {
  name: string | null;
}

export interface WorkspaceContextConfig {
  repo_rules_md?: string; // machine-managed review rules
  goal_md: string;
  memory_md: string;
  decisions_md: string;
  references: string[];
  artifacts: string[];
  context_version: number;
  skills: string[] | null; // null = all library skills
  soul: string | null; // null = global default
  extra_context_md: string;
  include_memory: boolean;
  include_repo_map?: boolean; // opt-in tree-sitter repo map
  repo_map_max_lines?: number | null;
}

export interface UpdateWorkspaceContextReq {
  skills?: string[] | null;
  soul?: string | null;
  extra_context_md?: string;
  include_memory?: boolean;
  include_repo_map?: boolean;
  goal_md?: string;
  memory_md?: string;
  decisions_md?: string;
  references?: string[];
  artifacts?: string[];
  /** Required for shared knowledge edits; stale revisions return 409. */
  context_version?: number;
}

export interface MaterializeProviderResult {
  provider: string;
  files_written: string[];
  skipped: boolean;
}

export interface MaterializeResp {
  provider_results: MaterializeProviderResult[];
}

// --- Context preview (dry-run before spawn) --------------------------------

/**
 * How binding a planned artifact is on the agent:
 * - `advisory` — instruction files (AGENTS.md/CLAUDE.md) and skills: guidance
 *   the model reads and *may ignore*.
 * - `enforced` — hooks / runtime settings the daemon imposes regardless of what
 *   the model decides (e.g. activity-forwarding hooks).
 */
export type ContextEnforcement = 'advisory' | 'enforced';

/** A single artifact a spawn would write, described without writing it. */
export interface ContextPlanFile {
  /** Absolute destination path the file would be written to. */
  path: string;
  /** `instructions`, `skill`, `skill_asset`, `hooks`, or `manifest`. */
  kind: string;
  enforcement: ContextEnforcement;
  /** Size in bytes of the content that would be written. */
  size: number;
  /** First lines of the content (a short excerpt for the preview list). */
  first_lines: string;
  /** Whether content was elided from first_lines (file is larger). */
  truncated: boolean;
}

/** A skill that would be activated, summarized for the preview. */
export interface ContextPlanSkill {
  name: string;
  description: string;
  version: number;
}

/** What a session spawn would materialize for one provider (dry-run). */
export interface ContextPreviewProvider {
  provider: string;
  /** True for providers that materialize nothing (shell/agy/…). */
  skipped: boolean;
  skills: ContextPlanSkill[];
  /** The soul (persona) name that would apply, if any. */
  soul: string | null;
  files: ContextPlanFile[];
  /** The exact instruction-file bytes the model will read (Otto region merged
   * into AGENTS.md/CLAUDE.md). Empty when the provider writes no instructions. */
  generated_instructions: string;
  /** Name of the instruction file (`CLAUDE.md` or `AGENTS.md`), if any. */
  instructions_file_name: string | null;
  /** The hooks/settings JSON the runtime would impose (enforced), if any. */
  generated_hooks: string | null;
}

/** `POST /workspaces/{id}/context/preview` */
export interface ContextPreviewResp {
  providers: ContextPreviewProvider[];
}

/**
 * `POST /workspaces/{id}/context/preview` body. All fields optional: when
 * present they override the workspace's stored context selection so the UI can
 * preview a not-yet-saved choice (the same inputs a session spawn would use).
 */
export interface ContextPreviewReq {
  goal_md?: string;
  memory_md?: string;
  decisions_md?: string;
  references?: string[];
  artifacts?: string[];
  /** Provider to preview; omit for both `claude` and `codex`. */
  provider?: string;
  /**
   * Override the active skill allow-list. Omit the key to inherit the stored
   * config; send explicit `null` to override to *all* library skills; send a
   * list to use exactly those.
   */
  skills?: string[] | null;
  /**
   * Override the active soul. Omit the key to inherit the stored config; send
   * explicit `null` to override to the global default; send a name to use it.
   */
  soul?: string | null;
  /** Override the extra-context markdown (omit ⇒ use stored config). */
  extra_context_md?: string;
  /** Override the include-memory toggle (omit ⇒ use stored config). */
  include_memory?: boolean;
  /** Override the include-repo-map toggle (omit ⇒ use stored config). */
  include_repo_map?: boolean;
  /** Working directory the spawn would use (omit ⇒ the workspace root). */
  cwd?: string;
}
