// Proof Packs REST client. Thin wrappers over the daemon's /proof-* surface
// (see crates/otto-server/src/routes/proof.rs). Workspace-scoped list/create +
// summary; flat-by-id detail/patch/delete/assemble/waive + per-artifact ops.

import { api } from './client';
import type {
  AddArtifactReq,
  AssembleReq,
  CreateProofPackReq,
  ProofPackDetail,
  ProofPackResp,
  ProofSummaryResp,
} from './types';

/** Filters for the workspace pack list (all optional). */
export interface ProofPackFilter {
  status?: string;
  work_item_kind?: string;
  work_item_id?: string;
}

/** List a workspace's proof packs, optionally filtered. */
export function listProofPacks(wsId: string, q?: ProofPackFilter): Promise<ProofPackResp[]> {
  const params = new URLSearchParams();
  if (q?.status) params.set('status', q.status);
  if (q?.work_item_kind) params.set('work_item_kind', q.work_item_kind);
  if (q?.work_item_id) params.set('work_item_id', q.work_item_id);
  const qs = params.toString();
  return api.get<ProofPackResp[]>(`/workspaces/${wsId}/proof-packs${qs ? `?${qs}` : ''}`);
}

/** Cheap per-work-item badge/status roll-up for the whole workspace. */
export function proofSummary(wsId: string): Promise<ProofSummaryResp> {
  return api.get<ProofSummaryResp>(`/workspaces/${wsId}/proof-summary`);
}

/** Create (or reuse, by work item) a proof pack. */
export function createProofPack(wsId: string, body: CreateProofPackReq): Promise<ProofPackResp> {
  return api.post<ProofPackResp>(`/workspaces/${wsId}/proof-packs`, body);
}

/** Full detail of one pack: badges, artifacts (with previews), children. */
export function getProofPack(id: string): Promise<ProofPackDetail> {
  return api.get<ProofPackDetail>(`/proof-packs/${id}`);
}

/** Update a pack's title/summary. */
export function patchProofPack(
  id: string,
  body: { title?: string; summary?: string },
): Promise<ProofPackResp> {
  return api.patch<ProofPackResp>(`/proof-packs/${id}`, body);
}

/** Delete a pack (and its artifacts). */
export function deleteProofPack(id: string): Promise<void> {
  return api.del<void>(`/proof-packs/${id}`);
}

/** Append an artifact (inline content or external URL) then recompute. */
export function addArtifact(id: string, body: AddArtifactReq): Promise<ProofPackResp> {
  return api.post<ProofPackResp>(`/proof-packs/${id}/artifacts`, body);
}

/** Re-run auto-assembly (diff + commands) from a working dir, then recompute. */
export function assembleProof(id: string, body: AssembleReq): Promise<ProofPackResp> {
  return api.post<ProofPackResp>(`/proof-packs/${id}/assemble`, body);
}

/** Waive a pack's gate with a human-supplied reason. */
export function waiveProof(id: string, reason: string): Promise<ProofPackResp> {
  return api.post<ProofPackResp>(`/proof-packs/${id}/waive`, { reason });
}

/** Delete a single artifact (recomputes the owning pack server-side). */
export function deleteArtifact(id: string): Promise<void> {
  return api.del<void>(`/proof-artifacts/${id}`);
}

/** Full (uncapped) content of one artifact. */
export function artifactContent(id: string): Promise<{
  content: string | null;
  ref_kind: string;
  kind: string;
  status: string;
  metadata: unknown;
}> {
  return api.get(`/proof-artifacts/${id}/content`);
}
