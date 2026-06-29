// Proof Packs REST client. Thin wrappers over the daemon's /proof-* surface
// (see crates/otto-server/src/routes/proof.rs). Workspace-scoped list/create +
// summary; flat-by-id detail/patch/delete/assemble/waive + per-artifact ops.

import { api, authedBlobUrl, authedText } from './client';
import type {
  AddArtifactReq,
  ApiEvidenceReq,
  AssembleReq,
  AttachMediaReq,
  CiRefreshReq,
  CreateProofPackReq,
  CreateSnapshotReq,
  DbEvidenceReq,
  KafkaEvidenceReq,
  PrCheckReq,
  ProofPackDetail,
  ProofPackResp,
  ProofSnapshotMeta,
  ProofSnapshotResp,
  ProofSummaryResp,
  RepoProofConfig,
  RepoProofConfigResp,
  WaiveReq,
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

/** Waive a pack's gate with a human-supplied reason (recorded as the waiver). */
export function waiveProof(id: string, reason: string): Promise<ProofPackResp> {
  const body: WaiveReq = { reason };
  return api.post<ProofPackResp>(`/proof-packs/${id}/waive`, body);
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

// --- v2 --------------------------------------------------------------------

/** Create an immutable, content-hashed snapshot (+ frozen md/html report). */
export function createSnapshot(id: string, body: CreateSnapshotReq = {}): Promise<ProofSnapshotResp> {
  return api.post<ProofSnapshotResp>(`/proof-packs/${id}/snapshot`, body);
}

/** List a pack's snapshots (newest first). */
export function listSnapshots(id: string): Promise<ProofSnapshotMeta[]> {
  return api.get<ProofSnapshotMeta[]>(`/proof-packs/${id}/snapshots`);
}

/** Fetch a full snapshot (bundle + reports). */
export function getSnapshot(id: string): Promise<ProofSnapshotResp> {
  return api.get<ProofSnapshotResp>(`/proof-snapshots/${id}`);
}

/** Attach screenshot/video evidence (base64; ≤25 MiB). */
export function attachMedia(id: string, body: AttachMediaReq): Promise<ProofPackResp> {
  return api.post<ProofPackResp>(`/proof-packs/${id}/media`, body);
}

/** A revocable object URL for a media artifact's blob (auth'd fetch). */
export function artifactBlobUrl(artifactId: string): Promise<string> {
  return authedBlobUrl(`/proof-artifacts/${artifactId}/blob`);
}

/** Record an HTTP request/response as evidence. */
export function attachApiEvidence(id: string, body: ApiEvidenceReq): Promise<ProofPackResp> {
  return api.post<ProofPackResp>(`/proof-packs/${id}/evidence/api`, body);
}

/** Record a DB read result as evidence. */
export function attachDbEvidence(id: string, body: DbEvidenceReq): Promise<ProofPackResp> {
  return api.post<ProofPackResp>(`/proof-packs/${id}/evidence/db`, body);
}

/** Record a Kafka read result as evidence. */
export function attachKafkaEvidence(id: string, body: KafkaEvidenceReq): Promise<ProofPackResp> {
  return api.post<ProofPackResp>(`/proof-packs/${id}/evidence/kafka`, body);
}

/** Run the PR-description consistency check against the actual change. */
export function runPrCheck(id: string, body: PrCheckReq): Promise<ProofPackResp> {
  return api.post<ProofPackResp>(`/proof-packs/${id}/pr-check`, body);
}

/** Fetch live CI status for the pack's linked PR into a `ci` artifact. */
export function ciRefresh(id: string, body: CiRefreshReq = {}): Promise<ProofPackResp> {
  return api.post<ProofPackResp>(`/proof-packs/${id}/ci-refresh`, body);
}

/** Render a live Markdown or HTML report for a pack. */
export function proofReport(id: string, format: 'md' | 'html'): Promise<string> {
  return authedText(`/proof-packs/${id}/report?format=${format}`);
}

/** Read a repo's proof requirements. */
export function getRepoProofConfig(repoId: string): Promise<RepoProofConfigResp> {
  return api.get<RepoProofConfigResp>(`/repos/${repoId}/proof-config`);
}

/** Write a repo's proof requirements. */
export function setRepoProofConfig(
  repoId: string,
  cfg: RepoProofConfig,
): Promise<RepoProofConfigResp> {
  return api.put<RepoProofConfigResp>(`/repos/${repoId}/proof-config`, cfg);
}
