// Smart Create productions client (`/api/v1/design/productions*`), mirrors
// the Smart-mode productions types in ./types/media.ts. Progress is read by polling `getProduction`.

import { api, ApiError, baseUrl, getToken } from './client';
import type { CreateProductionReq, Creations, Id, PatchProductionReq, Production, ProductionSummary, SteerProductionReq } from './types';

const enc = encodeURIComponent;
const base = (id: Id) => `/design/productions/${enc(id)}`;

export const createProduction = (body: CreateProductionReq) => api.post<Production>('/design/productions', body);
export const getProduction = (id: Id) => api.get<Production>(base(id));
export const listProductions = (workspaceId: Id) =>
  api.get<ProductionSummary[]>(`/design/productions?workspace_id=${enc(workspaceId)}`);
/** The Create shelf: productions, loose models and clips across every workspace the caller can see. */
export const listCreations = () => api.get<Creations>('/design/creations');
export const updateProduction = (id: Id, body: PatchProductionReq) => api.patch<Production>(base(id), body);
export const startProduction = (id: Id) => api.post<Production>(`${base(id)}/start`, {});
export const cancelProduction = (id: Id) => api.post<Production>(`${base(id)}/cancel`, {});
export const steerProduction = (id: Id, body: SteerProductionReq) => api.post<Production>(`${base(id)}/steer`, body);
/** "Not this one": a piece whose 3D shape is not built yet is drawn again (another seed) at its turn. */
export const redrawProductionItem = (id: Id, itemId: string) =>
  api.post<Production>(`${base(id)}/items/${encodeURIComponent(itemId)}/redraw`, {});
/** Authed object URL of an item's picture (`has_preview`; `rev` = its `preview_rev`, a cache key). Caller revokes. */
export async function productionPreviewUrl(id: Id, itemId: string, rev = 0): Promise<string> {
  const token = getToken();
  const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  const resp = await fetch(`${baseUrl()}/api/v1${base(id)}/items/${enc(itemId)}/preview?rev=${rev}`, { headers });
  if (!resp.ok) throw new ApiError(resp.status, { code: 'not_found', message: resp.statusText });
  return URL.createObjectURL(await resp.blob());
}
export const revertProduction = (id: Id, revisionId: Id) =>
  api.post<Production>(`${base(id)}/revert`, { revision_id: revisionId });
