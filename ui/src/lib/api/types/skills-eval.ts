// Contract mirror: skills-eval. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Skills Evaluator
// ---------------------------------------------------------------------------

export type SkillEvalStatus = 'running' | 'done' | 'error' | 'cancelled';
export type EvalAgentStatus = 'pending' | 'running' | 'waiting' | 'done' | 'error';
export type EvalIterStatus =
  | 'pending'
  | 'implementing'
  | 'validating'
  | 'improving'
  | 'done'
  | 'error';

/** One problem a validation found, with the concrete suggested fix. */
export interface EvalFinding {
  /** 'info' | 'warn' | 'fail' */
  severity: string;
  issue: string;
  suggestion: string;
  location?: string | null;
}

/** Live state of one validation agent (validation × provider) in an iteration.
 * Every requested pass must produce a valid verdict; incomplete passes are error/0.
 */
export interface EvalValidationState {
  validation: string;
  name: string;
  provider: string;
  model: string;
  status: EvalAgentStatus;
  note: string;
  passed: boolean;
  score: number;
  session_id?: string | null;
  findings: EvalFinding[];
}

/** One scored signal (tests, lint, review) on a 0–100 scale. */
export interface SignalScore {
  ran: boolean;
  score: number;
  detail: string;
}

/** The diff-quality signal: parsimony + risk of the produced change. */
export interface DiffScore {
  ran: boolean;
  files_changed: number;
  additions: number;
  deletions: number;
  risky: number;
  score: number;
  detail: string;
}

/** The human-rating signal. */
export interface HumanScore {
  rating?: number | null;
  note: string;
  rater: string;
  score: number;
}

/** Relative weights for the composite (renormalized over the signals that ran). */
export interface ScoreWeights {
  tests: number;
  lint: number;
  diff: number;
  review: number;
  human: number;
}

/** The full multi-signal score for one iteration's produced code. */
export interface EvalScore {
  tests: SignalScore;
  lint: SignalScore;
  diff: DiffScore;
  review: SignalScore;
  human: HumanScore;
  /** Original signal weights, including intentional zeros; ratings preserve them. */
  weights: ScoreWeights;
  /** Weighted mean over the signals that ran, 0–100. */
  composite: number;
  /** Proof status: missing|partial|passed|failed|waived; pending while a saved rating awaits evidence publication (not promotable). */
  proof_status: string;
  /** Proof done-contract score 0–100. */
  done_score: number;
}

/** One iteration (round) of a skill evaluation. */
export interface EvalIteration {
  id: Id;
  eval_id: Id;
  iter: number;
  base_iter?: number | null;
  skill_name: string;
  skill_before: string;
  skill_after?: string | null;
  impl_provider: string;
  impl_session_id?: string | null;
  impl_summary: string;
  worktree_path?: string | null;
  status: EvalIterStatus;
  note: string;
  /** Validator mean for generated iterations, including failed validators as zero. */
  score: number;
  agents: EvalValidationState[];
  improvement_summary: string;
  skill_diff: string;
  /** Multi-signal score (null until the scoring pipeline has run). */
  scoring?: EvalScore | null;
  /** The Proof Pack assembled for this iteration's evidence. */
  proof_pack_id?: string | null;
  /** Human quality rating 0–5. */
  human_rating?: number | null;
  human_note?: string;
  human_rater?: string;
  created_at: string;
}

/** A complete skill-evaluation run. */
export interface SkillEval {
  id: Id;
  workspace_id: Id;
  source_skill: string;
  task: string;
  impl_cli: string;
  target_iterations: number;
  status: SkillEvalStatus;
  error?: string | null;
  summary: string;
  best_iteration?: number | null;
  best_score?: number | null;
  iterations: EvalIteration[];
  /** The original StartSkillEvalReq JSON (for per-validation retry + display). */
  config?: unknown;
  /** 'generate' | 'score_only' */
  mode?: string;
  golden_task_id?: string | null;
  matrix_id?: string | null;
  dim_provider?: string | null;
  dim_skill?: string | null;
  dim_prompt?: string | null;
  /** Best iteration's composite score. */
  composite_score?: number | null;
  promoted?: boolean;
  promoted_at?: string | null;
  promoted_by?: string | null;
  created_at: string;
}

/** A run's list row — `GET /workspaces/{id}/skill-evaluations?summary=1`.
 *  The headline columns of {@link SkillEval} (same names) without
 *  `iterations` / `config`, plus the iteration count. */
export interface SkillEvalSummary {
  id: Id;
  workspace_id: Id;
  source_skill: string;
  task: string;
  impl_cli: string;
  target_iterations: number;
  status: SkillEvalStatus;
  error?: string | null;
  summary: string;
  best_iteration?: number | null;
  best_score?: number | null;
  mode?: string;
  golden_task_id?: string | null;
  matrix_id?: string | null;
  dim_provider?: string | null;
  dim_skill?: string | null;
  dim_prompt?: string | null;
  composite_score?: number | null;
  promoted?: boolean;
  iteration_count: number;
  created_at: string;
}

/** One keyset page of run summaries, newest first. */
export interface SkillEvalSummaryPage {
  items: SkillEvalSummary[];
  /** Pass back as `cursor` for the next (older) page; null on the last. */
  next_cursor: string | null;
}

/** A reusable, per-repo evaluation task (golden corpus + regression cases). */
export interface GoldenTask {
  id: Id;
  workspace_id: Id;
  repo_key: string;
  name: string;
  prompt: string;
  skill: string;
  test_cmd: string;
  lint_cmd: string;
  build_cmd: string;
  rubric: string;
  tags: string[];
  /** 'manual' | 'regression' */
  origin: string;
  source_eval_id?: string | null;
  source_iter_id?: string | null;
  enabled: boolean;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export interface GoldenTaskReq {
  name: string;
  prompt: string;
  skill?: string;
  test_cmd?: string;
  lint_cmd?: string;
  build_cmd?: string;
  rubric?: string;
  tags?: string[];
  repo_key?: string | null;
  enabled?: boolean;
}

/** Where a score-only run (or matrix cell) finds the code to score. */
export interface EvalTarget {
  /** 'working' | 'branch' | 'path' */
  kind: string;
  git_ref?: string | null;
  path?: string | null;
}

export interface RunGoldenReq {
  /** 'generate' | 'score_only' (default) */
  mode?: string;
  provider?: string | null;
  target?: EvalTarget | null;
}

export interface RateIterationReq {
  /** 0–5 */
  rating: number;
  note?: string;
}

export interface RegressionReq {
  name?: string | null;
}

/** Whether a run's winning skill may be promoted, with the reasons if not. */
export interface PromoteGate {
  allowed: boolean;
  score: number;
  threshold: number;
  proof_status: string;
  require_proof: boolean;
  score_ok: boolean;
  proof_ok: boolean;
  reasons: string[];
}

/** One prompt column-input of a matrix. */
export interface MatrixPrompt {
  label: string;
  task: string;
  golden_task_id?: string | null;
}

/** One executed cell of a matrix (a single skill_eval run). */
export interface MatrixCell {
  eval_id: Id;
  provider: string;
  skill: string;
  prompt: string;
  status: string;
  composite_score?: number | null;
  proof_status: string;
  best_iteration?: number | null;
}

/** A provider × skill × prompt comparison run. */
export interface EvalMatrix {
  id: Id;
  workspace_id: Id;
  name: string;
  status: string;
  repo_key: string;
  mode: string;
  providers: string[];
  skills: string[];
  prompts: MatrixPrompt[];
  cells: MatrixCell[];
  created_at: string;
}

/** At most 64 cells / 256 expanded steps; dimensions are non-empty and unique. */
export interface StartMatrixReq {
  name: string;
  mode?: string;
  providers: string[];
  skills: SkillSourceReq[];
  prompts: MatrixPrompt[];
  target?: EvalTarget | null;
  test_cmd?: string | null;
  lint_cmd?: string | null;
  base_ref?: string | null;
  weights?: ScoreWeights | null;
  validations?: SkillEvalValidationCfg[];
  iterations?: number;
}

/** One configurable validation dimension. */
export interface SkillEvalValidationCfg {
  name: string;
  criteria: string;
  /** Unique non-empty CLIs (one agent each); at most 16 expanded validators per run. */
  providers: string[];
  model: string;
}

export interface SkillEvalImproverCfg {
  provider: string;
  model: string;
}

export interface SkillEvalConfig {
  validations: SkillEvalValidationCfg[];
  improver: SkillEvalImproverCfg;
  iterations: number;
  /** Validation passes to average (1–3) — reduces grader noise. */
  validator_passes: number;
  /** Default composite-score weights. */
  weights?: ScoreWeights;
  /** Minimum composite score (0–100) required to promote. */
  promote_min_score?: number;
  /** Whether promotion also requires the iteration's proof pack to pass. */
  require_proof_pass?: boolean;
  default_test_cmd?: string;
  default_lint_cmd?: string;
}

/** Where the skill under test comes from. */
export interface SkillSourceReq {
  /** 'library' | 'path' | 'provider' */
  kind: string;
  reference: string;
  provider?: string | null;
}

/** Admission allows at most 10 iterations; golden task must belong to this workspace. */
export interface StartSkillEvalReq {
  source: SkillSourceReq;
  task: string;
  impl_cli: string;
  validations: SkillEvalValidationCfg[];
  iterations: number;
  improver?: SkillEvalImproverCfg | null;
  base_ref?: string | null;
  /** Validation passes to average (1–3). */
  validator_passes?: number;
  /** 'generate' (default) | 'score_only' */
  mode?: string;
  golden_task_id?: string | null;
  target?: EvalTarget | null;
  test_cmd?: string | null;
  lint_cmd?: string | null;
  weights?: ScoreWeights | null;
}

/** Promotion can fail after the library write: the error identifies the written
 * skill and asks for a retry, or library inspection if the evaluation was deleted. */
export interface PromoteSkillReq {
  iteration_id: Id;
  /** 'tested' = the skill that iteration ran with; 'improved' = its edited version. */
  source: 'tested' | 'improved';
  name: string;
  /** Bypass the score+proof gate (root human only; audited + waives proof before writing the library). */
  force?: boolean;
}

export interface ImplDiffResp {
  diff: string;
  truncated: boolean;
}

export interface SkillSourceInfo {
  /** 'library' | 'provider' */
  kind: string;
  name: string;
  description: string;
  provider?: string | null;
}

export interface SkillSourcesResp {
  sources: SkillSourceInfo[];
}
