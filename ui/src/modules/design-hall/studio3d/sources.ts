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

const modelCache = new Map<string, Promise<string>>();

/**
 * Blob URL for a gltf reference. Cached per reference for the session (a
 * model's bytes are immutable per version; `@latest` re-resolves on reload).
 */
export function resolveModelRef(ref: string): Promise<string> {
  let p = modelCache.get(ref);
  if (!p) {
    p = loadModel(ref);
    modelCache.set(ref, p);
    p.catch(() => modelCache.delete(ref));
  }
  return p;
}

async function loadModel(ref: string): Promise<string> {
  const uri = parseOttoUri(ref);
  if (uri) {
    const d = await getArtifact(uri.artifactId);
    const c = await fetchContent(uri.artifactId, { version: versionFor(d.artifact, uri.selector) ?? undefined, asText: false });
    if (!c.blobUrl) throw new Error('The model has no content');
    return c.blobUrl;
  }
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
