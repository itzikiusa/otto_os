// Skills Evaluator API helpers: discover skill sources, configure defaults,
// start runs, and poll a run's iterations/findings.

import { api } from './client';
import type {
  EvalMatrix,
  EvalScore,
  GoldenTask,
  GoldenTaskReq,
  ImplDiffResp,
  LibrarySkill,
  PromoteGate,
  PromoteSkillReq,
  RateIterationReq,
  RegressionReq,
  RunGoldenReq,
  SkillEval,
  SkillEvalConfig,
  SkillSourcesResp,
  StartMatrixReq,
  StartSkillEvalReq,
} from './types';

export const skillsEvalApi = {
  /** Skills the user can pick from (Otto library + provider on-disk skills). */
  listSources: (wsId: string) =>
    api.get<SkillSourcesResp>(`/workspaces/${wsId}/skill-sources`),

  /** Default validations / improver / iterations for the start form. */
  getConfig: () => api.get<SkillEvalConfig>('/settings/skill-eval'),
  putConfig: (body: SkillEvalConfig) =>
    api.put<SkillEvalConfig>('/settings/skill-eval', body),

  /** Start a run; returns the freshly-created run (status "running"). */
  start: (wsId: string, body: StartSkillEvalReq) =>
    api.post<SkillEval>(`/workspaces/${wsId}/skill-evaluations`, body),

  /** All runs for a workspace, newest first. */
  list: (wsId: string) =>
    api.get<SkillEval[]>(`/workspaces/${wsId}/skill-evaluations`),

  /** A single run with all its iterations (poll while running). */
  get: (evalId: string) => api.get<SkillEval>(`/skill-evaluations/${evalId}`),

  /** Stop an in-flight run (kills its agent sessions). */
  cancel: (evalId: string) => api.post<SkillEval>(`/skill-evaluations/${evalId}/cancel`),

  /** Delete a run: archives sessions, removes worktrees, drops the rows. */
  remove: (evalId: string) => api.del<void>(`/skill-evaluations/${evalId}`),

  /** Save an iteration's skill back to the Otto library. */
  promote: (evalId: string, body: PromoteSkillReq) =>
    api.post<LibrarySkill>(`/skill-evaluations/${evalId}/promote`, body),

  /** The code the implementation agent produced in an iteration's worktree. */
  implDiff: (evalId: string, iterId: string) =>
    api.get<ImplDiffResp>(`/skill-evaluations/${evalId}/iterations/${iterId}/diff`),

  /** Re-run a single validation agent within an iteration. */
  retryValidation: (evalId: string, iterId: string, index: number) =>
    api.post<SkillEval>(
      `/skill-evaluations/${evalId}/iterations/${iterId}/agents/${index}/retry`,
    ),

  // --- Eval lab: scoring, proof, rating, gate -----------------------------

  /** An iteration's multi-signal score. */
  iterScore: (evalId: string, iterId: string) =>
    api.get<EvalScore>(`/skill-evaluations/${evalId}/iterations/${iterId}/score`),

  /** An iteration's assembled proof pack (header + evidence artifacts). */
  iterProofPack: (evalId: string, iterId: string) =>
    api.get<{
      exists: boolean;
      id?: string;
      status?: string;
      risk_score?: number;
      done_score?: number;
      badges?: string[];
      contract?: unknown;
      artifacts?: Array<{
        kind: string;
        title: string;
        status: string;
        preview: string;
        truncated: boolean;
        metadata: unknown;
      }>;
    }>(`/skill-evaluations/${evalId}/iterations/${iterId}/proof-pack`),

  /** Record a human rating (0–5) and re-derive the iteration's score. */
  rate: (evalId: string, iterId: string, body: RateIterationReq) =>
    api.post<SkillEval>(
      `/skill-evaluations/${evalId}/iterations/${iterId}/rate`,
      body,
    ),

  /** Whether (and why) the run's best (or a named) iteration may be promoted. */
  promoteGate: (evalId: string, iterationId?: string) =>
    api.get<PromoteGate>(
      `/skill-evaluations/${evalId}/promote-gate${iterationId ? `?iteration_id=${encodeURIComponent(iterationId)}` : ''}`,
    ),

  /** Capture a (failed) iteration as a regression golden task. */
  regression: (evalId: string, iterId: string, body: RegressionReq = {}) =>
    api.post<GoldenTask>(
      `/skill-evaluations/${evalId}/iterations/${iterId}/regression`,
      body,
    ),

  // --- Golden tasks -------------------------------------------------------

  listGolden: (wsId: string, repoKey?: string) =>
    api.get<GoldenTask[]>(
      `/workspaces/${wsId}/golden-tasks${repoKey ? `?repo_key=${encodeURIComponent(repoKey)}` : ''}`,
    ),
  createGolden: (wsId: string, body: GoldenTaskReq) =>
    api.post<GoldenTask>(`/workspaces/${wsId}/golden-tasks`, body),
  getGolden: (id: string) => api.get<GoldenTask>(`/golden-tasks/${id}`),
  updateGolden: (id: string, body: GoldenTaskReq) =>
    api.put<GoldenTask>(`/golden-tasks/${id}`, body),
  deleteGolden: (id: string) => api.del<void>(`/golden-tasks/${id}`),
  runGolden: (id: string, body: RunGoldenReq = {}) =>
    api.post<SkillEval>(`/golden-tasks/${id}/run`, body),

  // --- Matrices -----------------------------------------------------------

  listMatrices: (wsId: string) =>
    api.get<EvalMatrix[]>(`/workspaces/${wsId}/eval-matrices`),
  createMatrix: (wsId: string, body: StartMatrixReq) =>
    api.post<EvalMatrix>(`/workspaces/${wsId}/eval-matrices`, body),
  getMatrix: (id: string) => api.get<EvalMatrix>(`/eval-matrices/${id}`),
  cancelMatrix: (id: string) =>
    api.post<EvalMatrix>(`/eval-matrices/${id}/cancel`),
};
