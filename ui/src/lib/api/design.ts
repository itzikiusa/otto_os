// Design Hall API — the artifact graph (`/api/v1/design/*`).
// Mirrors docs/contracts/api.md § "Design Hall" + crates/otto-design/src/http.rs.
// Types live in ./types ("Design Hall" block); this module only adds the thin,
// typed request helpers the Design Hall UI (modules/design-hall) and the
// Product Design tab's graph strip share.

import { api, ApiError, baseUrl, getToken } from './client';
import type {
  CreateDesignArtifactReq,
  CreateDesignLinkReq,
  CreateDesignProjectReq,
  DesignArtifact,
  DesignArtifactDetail,
  DesignAssistReq,
  DesignAssistTurn,
  DesignCommitReq,
  DesignContentPutReq,
  DesignLink,
  DesignLinksResp,
  DesignProject,
  DesignSaveResult,
  DesignSearchHit,
  DesignSignal,
  DesignSignalKind,
  DesignSignalReq,
  DesignStatus,
  DesignStudio,
  DesignVersion,
  Id,
  Problem,
  UpdateDesignArtifactReq,
  UpdateDesignProjectReq,
} from './types';

const enc = encodeURIComponent;

/** `GET /design/artifacts` + `GET /design/search` shared filters. */
export interface DesignListFilter {
  workspace_id?: Id;
  project_id?: Id;
  studio?: DesignStudio;
  format?: string;
  status?: DesignStatus;
  story_id?: Id;
  include_children?: boolean;
  author_kind?: 'user' | 'agent' | 'system';
  since?: string;
  until?: string;
  include_archived?: boolean;
  limit?: number;
  offset?: number;
}

/** Build a `?a=b&…` query from a flat record, skipping empty values. */
export function qs(params: Record<string, string | number | boolean | undefined | null>): string {
  const parts: string[] = [];
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null || v === '' || v === false) continue;
    parts.push(`${enc(k)}=${enc(String(v))}`);
  }
  return parts.length ? `?${parts.join('&')}` : '';
}

// ── Projects ────────────────────────────────────────────────────────────────

export function listProjects(p: { workspace_id?: Id; include_archived?: boolean } = {}) {
  return api.get<DesignProject[]>(`/design/projects${qs({ ...p })}`);
}

export function createProject(body: CreateDesignProjectReq) {
  return api.post<DesignProject>('/design/projects', body);
}

export function getProject(id: Id) {
  return api.get<DesignProject>(`/design/projects/${enc(id)}`);
}

export function updateProject(id: Id, body: UpdateDesignProjectReq) {
  return api.patch<DesignProject>(`/design/projects/${enc(id)}`, body);
}

// ── Artifacts ───────────────────────────────────────────────────────────────

export function listArtifacts(f: DesignListFilter = {}) {
  return api.get<DesignArtifact[]>(`/design/artifacts${qs({ ...f })}`);
}

export function createArtifact(body: CreateDesignArtifactReq) {
  return api.post<DesignSaveResult>('/design/artifacts', body);
}

/** Detail; `content: true` inlines the text source (≤ 256 KiB) of the head (or `version`). */
export function getArtifact(id: Id, opts: { content?: boolean; version?: string } = {}) {
  return api.get<DesignArtifactDetail>(
    `/design/artifacts/${enc(id)}${qs({ content: opts.content, version: opts.version })}`,
  );
}

export function updateArtifact(id: Id, body: UpdateDesignArtifactReq) {
  return api.patch<DesignArtifact>(`/design/artifacts/${enc(id)}`, body);
}

/** Archives (every version kept). Hard delete is intentionally not exposed in the UI. */
export function archiveArtifact(id: Id) {
  return api.del<void>(`/design/artifacts/${enc(id)}`);
}

/** Save new content. `base_version` ≠ head → 409 (ApiError.status). */
export function putContent(id: Id, body: DesignContentPutReq) {
  return api.put<DesignSaveResult>(`/design/artifacts/${enc(id)}/content`, body);
}

export function listVersions(id: Id, p: { kind?: string; limit?: number; offset?: number } = {}) {
  return api.get<DesignVersion[]>(`/design/artifacts/${enc(id)}/versions${qs({ ...p })}`);
}

/** Named commit (`kind: named`). */
export function commitVersion(id: Id, body: DesignCommitReq) {
  return api.post<DesignSaveResult>(`/design/artifacts/${enc(id)}/versions`, body);
}

/** Human-only: moves `approved_version_id` (default: head) and sets status `approved`. */
export function approveArtifact(id: Id, versionId?: Id) {
  return api.post<DesignArtifact>(
    `/design/artifacts/${enc(id)}/approve`,
    versionId ? { version_id: versionId } : {},
  );
}

// ── Links ───────────────────────────────────────────────────────────────────

export function getLinks(id: Id, dir: 'out' | 'in' | 'both' = 'both') {
  return api.get<DesignLinksResp>(`/design/artifacts/${enc(id)}/links${qs({ dir })}`);
}

export function createLink(id: Id, body: CreateDesignLinkReq) {
  return api.post<DesignLink>(`/design/artifacts/${enc(id)}/links`, body);
}

export function deleteLink(id: Id, linkId: Id) {
  return api.del<void>(`/design/artifacts/${enc(id)}/links/${enc(linkId)}`);
}

// ── Search + signals ────────────────────────────────────────────────────────

export function search(q: string, f: DesignListFilter = {}, signal?: AbortSignal) {
  return api.get<DesignSearchHit[]>(`/design/search${qs({ q, ...f })}`, signal);
}

export function listSignals(
  p: { workspace_id?: Id; artifact_id?: Id; kind?: DesignSignalKind; since?: string; limit?: number } = {},
) {
  return api.get<DesignSignal[]>(`/design/signals${qs({ ...p })}`);
}

export function postSignal(body: DesignSignalReq) {
  return api.post<DesignSignal>('/design/signals', body);
}

/**
 * Fire-and-forget signal capture — the learning loop's data. A failed capture
 * must never break the user's action (it already happened), so errors are
 * swallowed here on purpose.
 */
export function captureSignal(body: DesignSignalReq): void {
  void postSignal(body).catch(() => {
    /* best effort: the action itself succeeded */
  });
}

// ── Raw content (text or binary) ────────────────────────────────────────────

export interface DesignContent {
  /** UTF-8 text when `asText`, else null. */
  text: string | null;
  /** Object URL of the bytes when not `asText` (caller revokes). */
  blobUrl: string | null;
  /** `X-Design-Version` — the version these bytes belong to. */
  versionId: string | null;
  seq: number | null;
}

/**
 * Fetch the raw bytes of the head (or a version: id, `v12` or `12`) with the
 * stored bearer token, reading the version headers the JSON routes don't carry.
 */
export async function fetchContent(id: Id, opts: { version?: string; asText: boolean }): Promise<DesignContent> {
  const path = opts.version
    ? `/design/artifacts/${enc(id)}/versions/${enc(opts.version)}/content`
    : `/design/artifacts/${enc(id)}/content`;
  const token = getToken();
  const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  const resp = await fetch(`${baseUrl()}/api/v1${path}`, { headers });
  if (!resp.ok) {
    let problem: Problem = { code: 'internal', message: resp.statusText };
    try {
      problem = await resp.json();
    } catch {
      /* non-JSON error body */
    }
    throw new ApiError(resp.status, problem);
  }
  const versionId = resp.headers.get('X-Design-Version');
  const seqRaw = resp.headers.get('X-Design-Seq');
  const seq = seqRaw != null && seqRaw !== '' ? Number(seqRaw) : null;
  if (opts.asText) return { text: await resp.text(), blobUrl: null, versionId, seq };
  const blob = await resp.blob();
  return { text: null, blobUrl: URL.createObjectURL(blob), versionId, seq };
}

/** Authed object URL of the PNG thumbnail (caller revokes). */
export async function thumbnailUrl(id: Id): Promise<string> {
  const token = getToken();
  const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  const resp = await fetch(`${baseUrl()}/api/v1/design/artifacts/${enc(id)}/thumbnail`, { headers });
  if (!resp.ok) throw new ApiError(resp.status, { code: 'not_found', message: resp.statusText });
  return URL.createObjectURL(await resp.blob());
}

// ── Agent assist + rendered thumbnails ──────────────────────────────────────

/**
 * One design-assist agent turn (`POST …/assist`, 202 once the session is live).
 * The result lands as a new `agent` version; progress arrives over WS
 * (`design_assist_updated`, `design_artifact_updated {change:"live"}`).
 */
export function assistArtifact(id: Id, body: DesignAssistReq) {
  return api.post<DesignAssistTurn>(`/design/artifacts/${enc(id)}/assist`, body);
}

/** Recent assist turns of an artifact (in memory, newest first; empty after a restart). */
export function listAssistTurns(id: Id) {
  return api.get<DesignAssistTurn[]>(`/design/artifacts/${enc(id)}/assist`);
}

/**
 * Store a UI-rendered thumbnail (`PUT …/thumbnail`, raw PNG/WebP ≤ 2 MB). A
 * thumbnail is a cache, not an edit: no version is created.
 */
export async function putThumbnail(id: Id, image: Blob): Promise<DesignArtifact> {
  const token = getToken();
  const headers: Record<string, string> = { 'Content-Type': image.type || 'image/png' };
  if (token) headers.Authorization = `Bearer ${token}`;
  const resp = await fetch(`${baseUrl()}/api/v1/design/artifacts/${enc(id)}/thumbnail`, { method: 'PUT', headers, body: image });
  if (!resp.ok) {
    let problem: Problem = { code: 'internal', message: resp.statusText };
    try {
      problem = await resp.json();
    } catch {
      /* non-JSON error body */
    }
    throw new ApiError(resp.status, problem);
  }
  return (await resp.json()) as DesignArtifact;
}
