// Vault v3 API — file-backed markdown vaults (the docs home).
// Mirrors docs/contracts/api.md "Vault v3" + crates/otto-vault/src/http.rs.

import { api } from './client';
import type {
  OkfReport,
  Vault,
  VaultBacklink,
  VaultDirListing,
  VaultDocsRun,
  VaultGraphPayload,
  VaultNote,
  VaultNoteMeta,
  VaultRenameResult,
  VaultSearchHit,
  VaultSearchReq,
  VaultStatus,
  VaultSwitchHit,
  VaultTagCount,
} from './types';

const base = (ws: string) => `/workspaces/${ws}/vault/vaults`;
const enc = encodeURIComponent;

export function listVaults(ws: string) {
  return api.get<Vault[]>(base(ws));
}

export function createVault(ws: string, body: { name: string; root_path?: string; okf?: boolean }) {
  return api.post<Vault>(base(ws), body);
}

export function patchVault(ws: string, id: number, body: { name?: string; okf?: boolean }) {
  return api.patch<Vault>(`${base(ws)}/${id}`, body);
}

export function deleteVault(ws: string, id: number) {
  return api.del<void>(`${base(ws)}/${id}`);
}

export function rescanVault(ws: string, id: number) {
  return api.post<VaultStatus>(`${base(ws)}/${id}/rescan`, {});
}

export function vaultStatus(ws: string, id: number) {
  return api.get<VaultStatus>(`${base(ws)}/${id}/status`);
}

export function vaultDir(ws: string, id: number, path: string) {
  return api.get<VaultDirListing>(`${base(ws)}/${id}/dir?path=${enc(path)}`);
}

export function vaultNote(ws: string, id: number, path: string) {
  return api.get<VaultNote>(`${base(ws)}/${id}/note?path=${enc(path)}`);
}

export function writeVaultNote(
  ws: string,
  id: number,
  body: { path: string; content: string; if_hash?: string },
) {
  return api.put<VaultNoteMeta>(`${base(ws)}/${id}/note`, body);
}

export function deleteVaultNote(ws: string, id: number, path: string) {
  return api.del<void>(`${base(ws)}/${id}/note?path=${enc(path)}`);
}

export function renameVaultPath(ws: string, id: number, from: string, to: string) {
  return api.post<VaultRenameResult>(`${base(ws)}/${id}/rename`, { from, to });
}

export function createVaultFolder(ws: string, id: number, path: string) {
  return api.post<void>(`${base(ws)}/${id}/folder`, { path });
}

export function vaultBacklinks(ws: string, id: number, path: string) {
  return api.get<VaultBacklink[]>(`${base(ws)}/${id}/backlinks?path=${enc(path)}`);
}

export function vaultSearch(ws: string, id: number, req: VaultSearchReq) {
  return api.post<VaultSearchHit[]>(`${base(ws)}/${id}/search`, req);
}

export function vaultSwitcher(ws: string, id: number, q: string) {
  return api.get<VaultSwitchHit[]>(`${base(ws)}/${id}/switcher?q=${enc(q)}`);
}

export function vaultTags(ws: string, id: number) {
  return api.get<VaultTagCount[]>(`${base(ws)}/${id}/tags`);
}

export interface VaultGraphQuery {
  mode?: 'full' | 'local';
  path?: string;
  depth?: number;
  tags?: boolean;
  orphans?: boolean;
  reserved?: boolean;
  ghosts?: boolean;
  edge_budget?: number;
  group_by?: 'folder' | 'type';
}

export function vaultGraph(ws: string, id: number, q: VaultGraphQuery = {}) {
  const params = new URLSearchParams();
  for (const [k, v] of Object.entries(q)) {
    if (v !== undefined && v !== null) params.set(k, String(v));
  }
  const qs = params.toString();
  return api.get<VaultGraphPayload>(`${base(ws)}/${id}/graph${qs ? `?${qs}` : ''}`);
}

export function okfValidate(ws: string, id: number) {
  return api.post<OkfReport>(`${base(ws)}/${id}/okf/validate`, {});
}

export function okfIndexes(ws: string, id: number) {
  return api.post<{ written: number }>(`${base(ws)}/${id}/okf/indexes`, {});
}

/** URL of an attachment (needs the bearer — use with authedBlobUrl). */
export function assetPath(ws: string, id: number, path: string) {
  return `${base(ws)}/${id}/asset?path=${enc(path)}`;
}

// -- Docs agents — multi-writer documentation runs + per-note refine ---------

export interface VaultDocsRunReq {
  prompt: string;
  target_dir?: string; // vault-relative folder ("" = root)
  agents: { provider: string; model?: string }[]; // 1..=4
  summarizer?: { provider?: string; model?: string };
}

export function runDocsAgents(ws: string, id: number, body: VaultDocsRunReq) {
  return api.post<VaultDocsRun>(`${base(ws)}/${id}/docs-agents/run`, body);
}

/** NOT ws-scoped — the run carries its ws and is checked server-side. */
export function docsRun(runId: string) {
  return api.get<VaultDocsRun>(`/vault/docs-agents/runs/${enc(runId)}`);
}

/** The vault's persisted runs (docs + refine), newest-first — live runs carry
 *  their fresher in-memory snapshot. Survives daemon restarts (history). */
export function listDocsRuns(ws: string, id: number, limit = 50) {
  return api.get<VaultDocsRun[]>(`${base(ws)}/${id}/docs-agents/runs?limit=${limit}`);
}

export function cancelDocsRun(runId: string) {
  return api.post<void>(`/vault/docs-agents/runs/${enc(runId)}/cancel`, {});
}

/** LONG request — resolves when the refine turn completes. */
export function refineNote(
  ws: string,
  id: number,
  body: { path: string; prompt: string; provider?: string; model?: string },
) {
  return api.post<{ session_id: string; reply: string }>(
    `${base(ws)}/${id}/docs-agents/refine`,
    body,
  );
}

/** Poll right after POSTing refine to attach the live shell. */
export function refineSession(ws: string, id: number, path: string) {
  return api.get<{ session_id: string | null; running: boolean }>(
    `${base(ws)}/${id}/docs-agents/refine-session?path=${enc(path)}`,
  );
}
