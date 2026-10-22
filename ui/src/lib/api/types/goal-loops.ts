// Contract mirror: goal-loops. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Events (WS /ws/events)
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Goal Loops — mirror of crates/otto-core domain + api DTOs.
// ---------------------------------------------------------------------------

export type GoalLoopStatus =
  | 'draft'
  | 'running'
  | 'paused'
  | 'blocked'
  | 'succeeded'
  | 'exhausted'
  | 'failed'
  | 'stopped';

export type GoalLoopPhase =
  | 'planning'
  | 'executing'
  | 'evaluating'
  | 'digesting'
  | 'waiting'
  | 'done';

export interface AcceptanceCriterion {
  id: string;
  text: string;
  verify: string;
  verify_kind: 'command' | 'agent' | 'human' | 'manual';
  verify_cmd?: string | null;
}

export interface GoalLoopDefinition {
  title: string;
  summary: string;
  objectives: string[];
  acceptance_criteria: AcceptanceCriterion[];
  constraints: string[];
  out_of_scope: string[];
  success_signal: string;
}

export interface GoalLoopLimits {
  max_iterations: number;
  max_runtime_secs: number;
  /** 1..=86400 (create/update answer 400 outside it). */
  per_phase_timeout_secs: number;
  max_cost_usd?: number | null;
  max_attempts_per_executor: number;
}

export interface GoalLoopAgentCfg {
  name: string;
  provider: string;
  model: string;
  prompt_extra: string;
}

export interface GoalLoopRoleCfg {
  provider: string;
  model: string;
  prompt: string;
}

export interface GoalLoopConfig {
  allow_commits?: boolean;
  mode?: 'build' | 'research';
  source_links?: string[];
  skills?: string[];
  require_review?: boolean;
  executors: GoalLoopAgentCfg[];
  planner: GoalLoopRoleCfg;
  evaluator: GoalLoopRoleCfg;
  digester: GoalLoopRoleCfg;
  definer: GoalLoopRoleCfg;
}

export interface LoopAgentState {
  name: string;
  provider: string;
  model: string;
  status: 'pending' | 'running' | 'waiting' | 'done' | 'error';
  note: string;
  session_id?: string | null;
  output_summary?: string | null;
}

export interface EvalCriterion {
  id: string;
  met: boolean;
  evidence: string;
}

export interface GoalLoopEvaluation {
  progress_pct: number;
  verdict: 'achieved' | 'continue' | 'blocked';
  criteria: EvalCriterion[];
  feedback: string;
  rationale: string;
}

export interface GoalHumanVerification {
  criterion_id: string;
  criterion_revision: string;
  verified_by: string;
  evidence: string;
  verified_at: string;
}
export interface GoalQuestion {
  id: string;
  question: string;
  answer: string | null;
  answered_by: string | null;
  answered_at: string | null;
}
export interface GoalLoopLedger {
  resume_evaluation_iteration?: number | null;
  verifications: GoalHumanVerification[];
  questions: GoalQuestion[];
  next_action: string;
  last_failure_signature: string;
  repeated_failures: number;
  review_summary: string;
  review_passed: boolean;
}

export interface GoalLoop {
  id: Id;
  workspace_id: Id;
  name: string;
  repo_path: string;
  definition: GoalLoopDefinition;
  limits: GoalLoopLimits;
  config: GoalLoopConfig;
  status: GoalLoopStatus;
  phase: GoalLoopPhase;
  iterations_started: number;
  current_iteration: number;
  progress_pct: number;
  context_digest: string;
  ledger?: GoalLoopLedger;
  branch?: string | null;
  worktree_path?: string | null;
  base_commit?: string | null;
  summary?: string | null;
  error?: string | null;
  run_started_at?: string | null;
  elapsed_secs: number;
  cost_usd: number;
  created_by: Id;
  created_at: string;
  updated_at: string;
  finished_at?: string | null;
}

export interface GoalLoopIteration {
  id: Id;
  loop_id: Id;
  workspace_id: Id;
  idx: number;
  status: string;
  plan: string;
  agents: LoopAgentState[];
  evaluation?: GoalLoopEvaluation | null;
  context_in: string;
  context_out: string;
  tokens_input: number;
  tokens_output: number;
  cost_usd: number;
  started_at: string;
  finished_at?: string | null;
}

export interface GoalLoopDetail {
  loop: GoalLoop;
  /** With `?summary=true`: `plan`/`context_in`/`context_out` are `""` on all
   *  but the newest iteration — `GET /goal-loops/{id}/iterations/{idx}`. */
  iterations: GoalLoopIteration[];
}

export interface DefineGoalReq {
  provider?: string;
  model?: string;
  mode?: 'build' | 'research';
  seed: string;
  repo_path: string;
  context?: string | null;
  feedback?: string | null;
}

export interface GoalLoopDraft {
  definition: GoalLoopDefinition;
  suggested_limits: GoalLoopLimits;
  suggested_config: GoalLoopConfig;
}

export interface CreateGoalLoopReq {
  name: string;
  repo_path: string;
  definition: GoalLoopDefinition;
  limits: GoalLoopLimits;
  config: GoalLoopConfig;
  autostart: boolean;
}

/** Validated and applied atomically; a rejected patch leaves all fields unchanged. */
export interface UpdateGoalLoopReq {
  name?: string;
  limits?: GoalLoopLimits;
  config?: GoalLoopConfig;
}
