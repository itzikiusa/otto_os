// Contract mirror: automation. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Scheduled Tasks (mirror of otto_core::domain::ScheduledTask* — keep in lockstep)
// ---------------------------------------------------------------------------

/** A scheduled task: a recurring agent job with a cadence + delivery. */
export interface ScheduledTask {
  id: Id;
  workspace_id: Id;
  name: string;
  /** `agent_prompt` (run an agent) or `workflow` (hand off to a workflow). */
  kind: string;
  prompt: string;
  skill?: string | null;
  /** `claude` | `codex` | `agy` | `shell` | a custom provider slug. */
  provider: string;
  model: string;
  /** Working dir; `''` => a per-task scratch dir. NOT a security boundary. */
  cwd: string;
  /** `{cadence:'interval'|'daily'|'weekly'|'cron', every_min?, at?, weekday?, expr?}`. */
  schedule: Record<string, unknown>;
  /** `{type:'none'|'slack'|'telegram'|'email'|'webhook', ...}`. */
  destination: Record<string, unknown>;
  enabled: boolean;
  // v2
  /** IANA timezone the daily/weekly/cron times are interpreted in (default UTC). */
  timezone: string;
  /** For `kind:'workflow'`: the workflow this task launches. */
  workflow_id?: string | null;
  /** `none` | `worktree` (run in a fresh isolated git worktree). */
  sandbox: string;
  /** Extra agent attempts on failure (total attempts = 1 + max_retries). */
  max_retries: number;
  /** Deliver only when the report meaningfully changes from the last ok run. */
  notify_on_change: boolean;
  /** Build a proof pack (report + run metadata) for each run. */
  attach_proof: boolean;
  last_run_at?: string | null;
  last_status?: string | null;
  next_run_at?: string | null;
  /** When the schedule was last (re)armed — created, resumed, or given a new
   *  cadence/timezone. The scheduler never fires an occurrence from before it
   *  (a resumed task doesn't catch up what it missed). Null on older rows. */
  armed_at?: string | null;
  created_by?: string | null;
  created_at: string;
  updated_at: string;
}

/** One execution of a scheduled task. */
export interface ScheduledTaskRun {
  id: Id;
  task_id: Id;
  workspace_id: Id;
  /** `canceled`: stopped from Otto (`POST …/runs/{run_id}/cancel`).
   *  `skipped`: a workflow task whose workflow was still busy with an earlier run. */
  status: 'running' | 'ok' | 'error' | 'canceled' | 'skipped';
  trigger: 'schedule' | 'manual';
  started_at: string;
  finished_at?: string | null;
  summary: string;
  report_path?: string | null;
  report_rel?: string | null;
  delivered: boolean;
  delivery_error?: string | null;
  error?: string | null;
  /** The visible agent session the run drove (Open it from the run row). */
  session_id?: string | null;
  // v2
  report_hash?: string | null;
  proof_pack_id?: string | null;
  attempts: number;
  /** Delivery was suppressed because the report didn't meaningfully change. */
  skipped_delivery: boolean;
  workflow_run_id?: string | null;
  created_at: string;
}

/** `POST /scheduled-tasks/preview` response (#137a): the next fires of an
 *  unsaved schedule, RFC 3339 UTC — empty when it has none (a spent `once`). */
export interface ScheduledTaskPreview {
  next_fire_times: string[];
}

/** A built-in template the create form can pre-fill from. */
export interface ScheduledTaskPreset {
  id: string;
  name: string;
  description: string;
  kind: string;
  prompt: string;
  schedule: Record<string, unknown>;
  suggested_destination: Record<string, unknown>;
  skill?: string | null;
}

// ---------------------------------------------------------------------------
// Run with Otto — the "one button" flow: a source item driven through a stage
// machine (resolve source → context → worktree → agent/goal-loop → proof → AI
// review → human approval → PR draft). Mirrors otto-core / otto_state::otto_runs.
// ---------------------------------------------------------------------------

/** Where a run is in its stage machine. The `*_*` mid-states are the live
 *  stages; the trailing ones are terminal. */
export type RunStatus =
  | 'queued'
  | 'resolving_source'
  | 'building_context'
  | 'provisioning'
  | 'executing'
  | 'proving'
  | 'reviewing'
  | 'awaiting_approval'
  | 'drafting_pr'
  | 'completed'
  | 'failed'
  | 'rejected'
  | 'cancelled';

/** The kind of source an a run was seeded from. */
export type SourceKind =
  | 'jira'
  | 'confluence'
  | 'github_pr'
  | 'github_issue'
  | 'channel'
  | 'product_story'
  | 'finding'
  | 'test'
  | 'scheduled_report';

/** Single agent turn vs an iterative goal loop. */
export type RunMode = 'single_agent' | 'goal_loop';

/** Where the run was launched from. */
export type RunOrigin = 'slack' | 'telegram' | 'webhook' | 'ui' | 'mcp' | 'api';

/** A Run with Otto run: a source item driven end-to-end into a reviewed,
 *  evidence-backed PR draft. */
export interface OttoRun {
  id: Id;
  workspace_id: Id;
  title: string;
  source_kind: SourceKind;
  source_ref: string;
  source_url?: string;
  goal: string;
  mode: RunMode;
  provider: string;
  /** Model override handed to the executing agent ("" = provider default). */
  model: string;
  repo_id?: string;
  repo_path?: string;
  base_branch?: string;
  branch?: string;
  worktree_path?: string;
  base_commit?: string;
  status: RunStatus;
  error?: string;
  origin_kind: RunOrigin;
  origin_chat?: string;
  origin_thread?: string;
  origin_user?: string;
  callback_url?: string;
  goal_loop_id?: string;
  review_id?: string;
  proof_pack_id?: string;
  proof_status?: string;
  risk_score?: number;
  findings_total: number;
  findings_blocking: number;
  pr_draft_json?: string;
  pr_url?: string;
  auto_open_pr: boolean;
  approval_decision?: string;
  approved_by?: string;
  approved_at?: string;
  result_summary?: string;
  context_summary?: string;
  created_by: string;
  created_at: string;
  updated_at: string;
}

/** One entry in a run's stage timeline (a stage start/finish/note). */
export interface RunEvent {
  id: Id;
  run_id: Id;
  workspace_id: Id;
  kind: string;
  status?: string;
  message: string;
  detail?: unknown;
  created_at: string;
}

/** Launch a run — either from a detected source (`source_kind`/`source_ref` or
 *  `url`) or free `seed_text` (→ a channel run). */
export interface LaunchRunReq {
  source_kind?: SourceKind;
  source_ref?: string;
  url?: string;
  seed_text?: string;
  mode?: RunMode;
  provider?: string;
  /** Model override (e.g. "opus"); blank/absent = the provider's default. */
  model?: string;
  repo_id?: string;
  auto_open_pr?: boolean;
  title?: string;
}

/** Human approval / rejection of a run awaiting approval. */
export interface ApproveRunReq {
  decision: 'approve' | 'reject';
  note?: string;
}

/** The shape returned by `GET /runs/:id/detect` — a best-effort source guess. */
export interface RunDetectResp {
  detected?: {
    source_kind: SourceKind;
    source_ref: string;
    url: string;
  };
}
