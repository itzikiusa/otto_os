// Skills Evaluator API helpers: discover skill sources, configure defaults,
// start runs, and poll a run's iterations/findings.

import { api } from './client';
import type {
  ImplDiffResp,
  LibrarySkill,
  PromoteSkillReq,
  SkillEval,
  SkillEvalConfig,
  SkillSourcesResp,
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
};
