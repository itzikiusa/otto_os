// Run with Otto API client — thin typed wrappers over the generic `api` helper.
// Mirrors docs/contracts/api.md (the Run with Otto routes). The "one button"
// flow: a source item → an OttoRun driven through its stage machine into a
// reviewed, evidence-backed PR draft.

import { api } from './client';
import type {
  ApproveRunReq,
  LaunchRunReq,
  OttoRun,
  PrSummary,
  RunDetectResp,
  RunEvent,
} from './types';

export const runWithOttoApi = {
  /** All runs in a workspace (newest first). */
  list: (ws: string) => api.get<OttoRun[]>(`/workspaces/${ws}/runs`),
  /** Launch a run from a detected source or free seed text. */
  launch: (ws: string, body: LaunchRunReq) =>
    api.post<OttoRun>(`/workspaces/${ws}/runs`, body),
  /** Best-effort detect what `q` is (a Jira key, a URL, a finding/story/test id). */
  detect: (ws: string, q: string, signal?: AbortSignal) =>
    api.get<RunDetectResp>(`/workspaces/${ws}/runs/detect?q=${encodeURIComponent(q)}`, signal),
  /** One run by id. */
  get: (id: string) => api.get<OttoRun>(`/runs/${id}`),
  /** A run's stage timeline (chronological). */
  events: (id: string) => api.get<RunEvent[]>(`/runs/${id}/events`),
  /** Approve or reject a run that is awaiting approval. */
  approve: (id: string, body: ApproveRunReq) =>
    api.post<OttoRun>(`/runs/${id}/approve`, body),
  /** Cancel a non-terminal run. */
  cancel: (id: string) => api.post<OttoRun>(`/runs/${id}/cancel`, {}),
  /** Open the drafted PR for a run. */
  openPr: (id: string) => api.post<PrSummary>(`/runs/${id}/open-pr`, {}),
};
