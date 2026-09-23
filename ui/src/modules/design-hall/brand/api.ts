// Brand Kit API — the two brand routes plus the learned-rules read the editor
// shows as "Rule from team" chips. Everything else (load, save, approve,
// versions) is the ordinary Design Hall client (`lib/api/design`).
// Contract: docs/contracts/api.md § Brand Kit.

import { api, ApiError, baseUrl, getToken } from '../../../lib/api/client';
import type {
  BrandDoc,
  BrandExportFormat,
  BrandImpactResp,
  DesignLearnedResp,
  Id,
  Problem,
} from '../../../lib/api/types';

const enc = encodeURIComponent;

/**
 * Which artifacts use this kit, and what `content` (the proposed kit) would
 * change for them. Omit `content` for a plain "used in" listing.
 */
export function brandImpact(id: Id, content?: BrandDoc, signal?: AbortSignal): Promise<BrandImpactResp> {
  const body = content ? { content } : {};
  return api.post<BrandImpactResp>(`/design/artifacts/${enc(id)}/brand/impact`, body, signal);
}

export interface BrandExport {
  text: string;
  fileName: string;
  mime: string;
}

/** The kit exported as CSS vars, a Tailwind v4 `@theme`, or DTCG JSON. */
export async function brandExport(id: Id, format: BrandExportFormat, version?: string): Promise<BrandExport> {
  const q = `?format=${enc(format)}${version ? `&version=${enc(version)}` : ''}`;
  const token = getToken();
  const headers: Record<string, string> = token ? { Authorization: `Bearer ${token}` } : {};
  const resp = await fetch(`${baseUrl()}/api/v1/design/artifacts/${enc(id)}/brand/export${q}`, { headers });
  if (!resp.ok) {
    let problem: Problem = { code: 'internal', message: resp.statusText };
    try {
      problem = await resp.json();
    } catch {
      /* non-JSON error body */
    }
    throw new ApiError(resp.status, problem);
  }
  const cd = resp.headers.get('Content-Disposition') ?? '';
  const fileName = /filename="([^"]+)"/.exec(cd)?.[1] ?? `brand.${format === 'dtcg' ? 'json' : 'css'}`;
  return { text: await resp.text(), fileName, mime: resp.headers.get('Content-Type') ?? 'text/plain' };
}

/** The workspace's learned design rules (active + pending). */
export function getLearned(workspaceId: Id): Promise<DesignLearnedResp> {
  return api.get<DesignLearnedResp>(`/design/learned?workspace_id=${enc(workspaceId)}`);
}
