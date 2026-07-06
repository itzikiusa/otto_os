// Skills Lab — multi-agent skill review API. Mirrors the code-review client:
// start a review, poll/refresh the run, open each agent's embedded terminal,
// retry one agent, cancel/delete.

import { api } from './client';
import type { ApplySkillFixReq, SkillReview, StartSkillReviewReq } from './types';

export const skillReviewApi = {
  /** Start a review; returns the freshly-created run (status "running"). */
  start: (wsId: string, body: StartSkillReviewReq) =>
    api.post<SkillReview>(`/workspaces/${wsId}/skill-reviews`, body),

  /** All reviews for a workspace, newest first. */
  list: (wsId: string) => api.get<SkillReview[]>(`/workspaces/${wsId}/skill-reviews`),

  /** One review (static report + live agents + summary). Poll/refresh on bus tick. */
  get: (id: string) => api.get<SkillReview>(`/skill-reviews/${id}`),

  /** Stop an in-flight review (archives its agent sessions). */
  cancel: (id: string) => api.post<SkillReview>(`/skill-reviews/${id}/cancel`),

  /** Delete a review (also archives sessions). */
  remove: (id: string) => api.del<void>(`/skill-reviews/${id}`),

  /** Re-run one reviewer agent by its index. */
  retryAgent: (id: string, index: number) =>
    api.post<SkillReview>(`/skill-reviews/${id}/agents/${index}/retry`),

  /** Send the findings to a fixer agent that applies them to the real skill dir. */
  apply: (id: string, body: ApplySkillFixReq) =>
    api.post<SkillReview>(`/skill-reviews/${id}/apply`, body),
};
