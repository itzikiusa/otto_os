// Workbench API — per-user scratch files with full, append-only edit history.
// Mirrors docs/contracts/api.md "Workbench" + crates/otto-server/src/routes/workbench.rs.

import { api, authedBlobUrl, postBlob } from './client';
import type {
  WorkbenchAsset,
  WorkbenchCreateReq,
  WorkbenchDiff,
  WorkbenchDoc,
  WorkbenchDocFull,
  WorkbenchRevision,
  WorkbenchRevisionDetail,
  WorkbenchUpdateReq,
} from './types';

const base = (ws: string) => `/workspaces/${encodeURIComponent(ws)}/workbench`;
const doc = (ws: string, id: string) => `${base(ws)}/docs/${encodeURIComponent(id)}`;

/** Live docs (default) or the trash (`trash: true`). */
export function listWorkbenchDocs(ws: string, opts: { trash?: boolean } = {}) {
  return api.get<WorkbenchDoc[]>(`${base(ws)}/docs${opts.trash ? '?trash=true' : ''}`);
}

export function createWorkbenchDoc(ws: string, body: WorkbenchCreateReq) {
  return api.post<WorkbenchDocFull>(`${base(ws)}/docs`, body);
}

export function getWorkbenchDoc(ws: string, id: string) {
  return api.get<WorkbenchDocFull>(doc(ws, id));
}

/** Autosave / rename / pin / language override. Returns the new metadata. */
export function updateWorkbenchDoc(ws: string, id: string, body: WorkbenchUpdateReq) {
  return api.patch<WorkbenchDoc>(doc(ws, id), body);
}

/** Move to the trash (soft delete — history kept, restorable). */
export function trashWorkbenchDoc(ws: string, id: string) {
  return api.del<WorkbenchDoc>(doc(ws, id));
}

/** Permanently delete a TRASHED doc and its whole history (irreversible). */
export function purgeWorkbenchDoc(ws: string, id: string) {
  return api.del<void>(`${doc(ws, id)}?permanent=true`);
}

export function restoreWorkbenchDoc(ws: string, id: string) {
  return api.post<WorkbenchDoc>(`${doc(ws, id)}/restore`, {});
}

/** Newest-first metadata page; before_seq is exclusive. History is retained. */
export function listWorkbenchRevisions(ws: string, id: string, opts: { limit?: number; before_seq?: number } = {}) {
  const query = new URLSearchParams();
  if (opts.limit !== undefined) query.set('limit', String(opts.limit));
  if (opts.before_seq !== undefined) query.set('before_seq', String(opts.before_seq));
  const suffix = query.size ? `?${query}` : '';
  return api.get<WorkbenchRevision[]>(`${doc(ws, id)}/revisions${suffix}`);
}

export function getWorkbenchRevision(ws: string, id: string, seq: number) {
  return api.get<WorkbenchRevisionDetail>(`${doc(ws, id)}/revisions/${seq}`);
}

/** Restore an old revision as the current content (adds a `restore` revision;
 *  nothing is overwritten). */
export function restoreWorkbenchRevision(ws: string, id: string, seq: number) {
  return api.post<WorkbenchDocFull>(`${doc(ws, id)}/revisions/${seq}/restore`, {});
}

/** Line diff `from` → `to` (`to` omitted = the current content). */
export function diffWorkbenchRevisions(ws: string, id: string, from: number, to?: number) {
  const q = `from=${from}${to === undefined ? '' : `&to=${to}`}`;
  return api.get<WorkbenchDiff>(`${doc(ws, id)}/diff?${q}`);
}

/** Upload an image (raw bytes). */
export function uploadWorkbenchAsset(ws: string, blob: Blob, contentType: string) {
  return postBlob<WorkbenchAsset>(`${base(ws)}/assets`, blob, contentType);
}

/** Authenticated object URL for an asset (revoke with URL.revokeObjectURL). */
export function workbenchAssetUrl(ws: string, assetId: string) {
  return authedBlobUrl(`${base(ws)}/assets/${encodeURIComponent(assetId)}`);
}
