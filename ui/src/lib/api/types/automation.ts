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
