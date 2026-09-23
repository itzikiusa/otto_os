// Site Studio — the daemon calls the studio needs beyond the shared design API
// (ui/src/lib/api/design.ts): the static export / local preview publish, the
// publish history, and resolving what a site references (the brand kit that
// themes it, the 3D artifacts it embeds, the images it shows).
// Contract: docs/contracts/api.md § "Site Studio".

import { api, ApiError, authedText, baseUrl, getToken } from '../../../lib/api/client';
import * as design from '../../../lib/api/design';
import type {
  DesignArtifact,
  DesignPublish,
  DesignSiteExportReq,
  DesignSiteLocalResp,
  Problem,
} from '../../../lib/api/types';
import { buildTheme, type Theme } from './engine/theme';
import type { EmbedInfo } from './engine/render';

const enc = encodeURIComponent;

/** `POST …/export {target:"zip"}` → the archive as a Blob + its file name. */
export async function exportZip(id: string, version?: string): Promise<{ blob: Blob; filename: string; publishId: string | null; seq: number | null }> {
  const token = getToken();
  const headers: Record<string, string> = { 'Content-Type': 'application/json' };
  if (token) headers.Authorization = `Bearer ${token}`;
  const body: DesignSiteExportReq = { target: 'zip', ...(version ? { version } : {}) };
  const resp = await fetch(`${baseUrl()}/api/v1/design/artifacts/${enc(id)}/export`, { method: 'POST', headers, body: JSON.stringify(body) });
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
  const m = /filename="([^"]+)"/.exec(cd);
  const seqRaw = resp.headers.get('X-Design-Seq');
  return {
    blob: await resp.blob(),
    filename: m?.[1] ?? 'site.zip',
    publishId: resp.headers.get('X-Design-Publish'),
    seq: seqRaw ? Number(seqRaw) : null,
  };
}

/** `POST …/export {target:"local"}` — records a publish and returns the loopback preview. */
export function publishLocal(id: string, version?: string): Promise<DesignSiteLocalResp> {
  const body: DesignSiteExportReq = { target: 'local', ...(version ? { version } : {}) };
  return api.post<DesignSiteLocalResp>(`/design/artifacts/${enc(id)}/export`, body);
}

export function listPublishes(id: string): Promise<DesignPublish[]> {
  return api.get<DesignPublish[]>(`/design/artifacts/${enc(id)}/publishes`);
}

/** The server-rendered page (`GET …/preview[/<page>]`) as HTML text. */
export function previewHtml(id: string, opts: { page?: string; publish?: string; version?: string } = {}): Promise<string> {
  const q = design.qs({ publish: opts.publish, version: opts.version });
  return authedText(`/design/artifacts/${enc(id)}/preview${opts.page ? `/${enc(opts.page)}` : ''}${q}`);
}

// ── References ───────────────────────────────────────────────────────────────

/** `otto://design/<id>[@approved|@latest|@vN][#node]` → parts. */
export function parseRef(uri: string): { id: string; sel: 'approved' | 'latest' | 'default' | number } | null {
  const m = /^otto:\/\/design\/([A-Za-z0-9_-]{1,64})(?:@(approved|latest|v([1-9][0-9]*)))?(?:#[A-Za-z0-9_:.-]{1,128})?$/.exec(uri.trim());
  if (!m) return null;
  return { id: m[1], sel: m[3] ? Number(m[3]) : ((m[2] as 'approved' | 'latest' | undefined) ?? 'default') };
}

export function policyOf(sel: 'approved' | 'latest' | 'default' | number): string {
  return typeof sel === 'number' ? 'pinned' : sel === 'latest' ? 'follow_latest' : 'follow_approved';
}

export interface BrandResolution {
  theme: Theme;
  kit: DesignArtifact | null;
  seq: number | null;
  /** Why no kit applies (none linked / failed to load). */
  note: string | null;
}

/**
 * The brand kit that themes a site: the project's `brand_kit_id`, else the
 * document's `brand` reference, else a `uses_tokens` link — at its approved
 * version (else head). No kit → the default palette.
 */
export async function resolveBrand(opts: { projectKitId: string | null; docBrand: string | undefined; artifactId: string }): Promise<BrandResolution> {
  let kitId = opts.projectKitId;
  let pinned: number | null = null;
  if (!kitId && opts.docBrand) {
    const r = parseRef(opts.docBrand);
    if (r) {
      kitId = r.id;
      if (typeof r.sel === 'number') pinned = r.sel;
    }
  }
  if (!kitId) {
    try {
      const l = await design.getLinks(opts.artifactId, 'out');
      kitId = l.links.find((x) => x.rel === 'uses_tokens' && x.dst_kind === 'artifact' && !x.broken)?.dst_id ?? null;
    } catch {
      /* no links → defaults */
    }
  }
  if (!kitId) return { theme: buildTheme(null), kit: null, seq: null, note: 'No brand kit linked — using the default palette.' };
  try {
    const head = await design.getArtifact(kitId);
    const version = pinned != null ? `v${pinned}` : (head.artifact.approved_version_id ?? head.artifact.head_version_id ?? undefined);
    const d = await design.getArtifact(kitId, { content: true, version });
    let parsed: unknown = null;
    try {
      parsed = d.content ? JSON.parse(d.content) : null;
    } catch {
      parsed = null;
    }
    const seq = pinned ?? (head.approved?.seq ?? head.head?.seq ?? null);
    return { theme: buildTheme(parsed, d.artifact.title), kit: d.artifact, seq, note: parsed ? null : 'The brand kit isn’t valid JSON — using the default palette.' };
  } catch (e) {
    return { theme: buildTheme(null), kit: null, seq: null, note: `Couldn’t load the brand kit (${e instanceof Error ? e.message : String(e)}).` };
  }
}

/** Resolve one 3D reference for the canvas (title, version, policy, poster). */
export async function resolveEmbed(uri: string): Promise<EmbedInfo> {
  const r = parseRef(uri);
  if (!r) return { title: 'Invalid reference', seq: null, policy: 'follow_approved', poster: null, broken: true };
  try {
    const d = await design.getArtifact(r.id);
    const seq = typeof r.sel === 'number' ? r.sel : r.sel === 'latest' ? (d.head?.seq ?? null) : (d.approved?.seq ?? d.head?.seq ?? null);
    let poster: string | null = null;
    if (d.artifact.thumb_blob) {
      try {
        poster = await design.thumbnailUrl(r.id);
      } catch {
        poster = null;
      }
    }
    return { title: d.artifact.title, seq, policy: policyOf(r.sel), poster };
  } catch {
    return { title: 'Missing 3D artifact', seq: null, policy: policyOf(r.sel), poster: null, broken: true };
  }
}

/** An `otto://design/…` image → an object URL of the version the reference asks for. */
export async function resolveImage(uri: string): Promise<string | null> {
  const r = parseRef(uri);
  if (!r) return null;
  try {
    let version: string | undefined;
    if (typeof r.sel === 'number') version = `v${r.sel}`;
    else if (r.sel !== 'latest') {
      const d = await design.getArtifact(r.id);
      version = d.artifact.approved_version_id ?? undefined;
    }
    const c = await design.fetchContent(r.id, { version, asText: false });
    return c.blobUrl;
  } catch {
    return null;
  }
}

/** Other sites in the workspace ("From your library"). */
export async function librarySites(workspaceId: string, exclude: string): Promise<DesignArtifact[]> {
  const list = await design.listArtifacts({ workspace_id: workspaceId, format: 'otto-site', limit: 24 });
  return list.filter((a) => a.id !== exclude);
}

/** A site's document at its approved (else head) version. */
export async function siteContent(a: DesignArtifact): Promise<{ text: string; seq: number } | null> {
  const version = a.approved_version_id ?? a.head_version_id ?? undefined;
  if (!version) return null;
  const d = await design.getArtifact(a.id, { content: true, version });
  if (d.content == null || d.content_truncated) return null;
  const seq = (d.content_version_id === d.approved?.id ? d.approved?.seq : d.head?.seq) ?? a.head_seq ?? 1;
  return { text: d.content, seq };
}
