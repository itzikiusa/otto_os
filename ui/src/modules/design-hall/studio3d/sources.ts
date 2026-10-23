// Where the 3D Studio (and the embed runtime) get their bytes from, with the
// user's bearer token and never a raw URL from the document:
//
//   • models — a gltf object's `src: otto://design/<id>[@approved|@latest|@vN]`
//     (a GLB/glTF Design Hall artifact) or a legacy `attachment_id` (a Design
//     Hall artifact id, else a Product attachment — the extractor resolves the
//     same way);
//   • the brand kit — the scene's `brand` URI, else the project's chosen kit
//     (`brand_kit_id`), else the first `otto-brand` kit of its project, else of
//     its workspace (Brand Kit v1 owns the kit editor;
//     `token:color.<name>` resolution is `scene3d/tokens.ts`).
import { authedBlobUrl } from '../../../lib/api/client';
import { fetchContent, getArtifact, listArtifacts } from '../../../lib/api/design';
import type { DesignArtifact } from '../../../lib/api/types';
import { parseOttoUri, type OttoUri } from '../model';

/** Which version a URI selector means for `a`: an id, a `vN`, or null for the head. */
export function versionFor(a: DesignArtifact, sel: OttoUri['selector']): string | null {
  switch (sel.kind) {
    case 'version':
      return `v${sel.seq}`;
    case 'latest':
      return null;
    case 'approved':
    case 'default':
    default:
      // Render links follow the approved version; drafts never leak into consumers.
      return a.approved_version_id ?? null;
  }
}

/** Model blobs kept warm when no scene holds them (most recently used). */
const MODEL_CACHE_MAX = 12;

interface ModelEntry {
  url: Promise<string>;
  /** Live scenes holding this blob (never revoked while > 0). */
  holders: number;
  /** Dropped from the cache while held: revoke on the last release. */
  orphan: boolean;
}

/**
 * Model blob URLs by resolved version (`<artifact>@<version id>`, so
 * `@latest` and `@approved` follow new versions and a version is fetched
 * once whichever selector named it). A small LRU: an entry no live scene
 * holds is revoked once more than {@link MODEL_CACHE_MAX} are cached, and
 * {@link releaseModelCache} (Design Hall teardown) revokes them all — a GLB
 * pins its bytes in the WebContent process until its URL is revoked.
 */
const modelCache = new Map<string, ModelEntry>();

function revoke(e: ModelEntry): void {
  void e.url.then((u) => {
    if (u.startsWith('blob:')) URL.revokeObjectURL(u);
  }, () => {});
}

function trimModelCache(): void {
  for (const [key, e] of modelCache) {
    if (modelCache.size <= MODEL_CACHE_MAX) return;
    if (e.holders > 0) continue;
    modelCache.delete(key);
    revoke(e);
  }
}

/** The cache key a reference resolves to, and how to load its bytes. */
async function resolveKey(ref: string): Promise<{ key: string; load: () => Promise<string> }> {
  const uri = parseOttoUri(ref);
  if (!uri) return { key: `legacy:${ref}`, load: () => loadLegacy(ref) };
  const a = (await getArtifact(uri.artifactId)).artifact;
  const version = versionFor(a, uri.selector) ?? a.head_version_id ?? 'head';
  return { key: `${a.id}@${version}`, load: () => loadModel(a.id, version === 'head' ? null : version) };
}

async function acquire(ref: string): Promise<{ key: string; entry: ModelEntry }> {
  const { key, load } = await resolveKey(ref);
  let entry = modelCache.get(key);
  if (entry) modelCache.delete(key); // re-inserted below: most recently used
  else {
    entry = { url: load(), holders: 0, orphan: false };
    const mine = entry;
    entry.url.catch(() => {
      if (modelCache.get(key) === mine) modelCache.delete(key);
    });
  }
  entry.holders += 1;
  modelCache.set(key, entry);
  trimModelCache();
  return { key, entry };
}

function releaseEntry(key: string, entry: ModelEntry): void {
  entry.holders = Math.max(0, entry.holders - 1);
  if (entry.holders > 0) return;
  if (entry.orphan) revoke(entry);
  else if (modelCache.get(key) === entry) trimModelCache();
}

/**
 * A scene's hold on its model blobs: `resolve(ref)` gives a blob URL for a
 * gltf reference that stays valid until `release()` (the scene's teardown —
 * `embed.destroy()`, ArtifactStage and Studio3D unmount).
 */
export function modelScope(): { resolve: (ref: string) => Promise<string>; release: () => void } {
  const held: { key: string; entry: ModelEntry }[] = [];
  let released = false;
  return {
    async resolve(ref) {
      const h = await acquire(ref);
      if (released) {
        // Resolved after the scene went away: don't pin it.
        releaseEntry(h.key, h.entry);
        throw new Error('The 3D view was closed');
      }
      held.push(h);
      return h.entry.url;
    },
    release() {
      released = true;
      for (const h of held.splice(0)) releaseEntry(h.key, h.entry);
    },
  };
}

/** Design Hall teardown: revoke every cached model blob no scene holds, and
 *  revoke the held ones as their scenes release them. */
export function releaseModelCache(): void {
  for (const e of modelCache.values()) {
    if (e.holders > 0) e.orphan = true;
    else revoke(e);
  }
  modelCache.clear();
}

/** Cached entries (tests). */
export function modelCacheSize(): number {
  return modelCache.size;
}

async function loadModel(id: string, version: string | null): Promise<string> {
  const c = await fetchContent(id, { version: version ?? undefined, asText: false });
  if (!c.blobUrl) throw new Error('The model has no content');
  return c.blobUrl;
}

async function loadLegacy(ref: string): Promise<string> {
  // Legacy bare id: a Design Hall artifact first, else a Product attachment.
  try {
    const c = await fetchContent(ref, { asText: false });
    if (c.blobUrl) return c.blobUrl;
  } catch {
    /* not a design artifact — fall through */
  }
  return authedBlobUrl(`/product/attachments/${encodeURIComponent(ref)}`);
}

export interface BrandKit {
  artifact: DesignArtifact;
  /** The parsed `otto-brand` document. */
  doc: unknown;
  /** What a scene stores in `brand` to follow this kit (a uses_tokens link). */
  uri: string;
  /** "Acme Brand Kit v4". */
  label: string;
}

/**
 * The kit a scene's tokens resolve against: its `brand` URI when set, else the
 * project's (then the workspace's) first `otto-brand` kit — approved first.
 * Null when there is none (tokens then render with a neutral fallback).
 */
export async function loadBrandKit(
  sceneBrand: string | undefined,
  artifact: DesignArtifact,
  /** The project's chosen kit (`DesignProject.brand_kit_id`), when known. */
  projectKitId?: string | null,
): Promise<BrandKit | null> {
  let kit: DesignArtifact | null = null;
  let version: string | null = null;
  const uri = sceneBrand ? parseOttoUri(sceneBrand) : null;
  if (uri) {
    kit = (await getArtifact(uri.artifactId)).artifact;
    version = versionFor(kit, uri.selector);
  } else {
    const pick = (xs: DesignArtifact[]) => xs.find((a) => a.approved_version_id) ?? xs[0] ?? null;
    if (projectKitId) kit = await getArtifact(projectKitId).then((d) => d.artifact, () => null);
    if (!kit && artifact.project_id) kit = pick(await listArtifacts({ project_id: artifact.project_id, format: 'otto-brand', limit: 10 }));
    if (!kit) kit = pick(await listArtifacts({ workspace_id: artifact.workspace_id, format: 'otto-brand', limit: 10 }));
    if (!kit) return null;
    version = kit.approved_version_id;
  }
  if (!kit.head_version_id) return null;
  const c = await fetchContent(kit.id, { version: version ?? undefined, asText: true });
  let doc: unknown = null;
  try {
    doc = JSON.parse(c.text ?? '');
  } catch {
    return null;
  }
  const seq = c.seq ?? kit.head_seq;
  return {
    artifact: kit,
    doc,
    uri: sceneBrand ?? `otto://design/${kit.id}@${kit.approved_version_id ? 'approved' : 'latest'}`,
    label: seq ? `${kit.title} v${seq}` : kit.title,
  };
}
