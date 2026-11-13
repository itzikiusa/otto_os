// Design Hall navigation helpers — the few places the Hall hands off to other
// modules (Product story → Design tab, a session) and the one door other
// modules use to open a Canvas scene, which now lives in the Hall as a mirror.

import { router } from '../../lib/router.svelte';
import { product } from '../../lib/stores/product.svelte';
import { listArtifacts } from '../../lib/api/design';
import type { DesignArtifact } from '../../lib/api/types';
import { opensInHall, syncErrorOf, type SceneHallCopy } from './mirrorSync';

export function openArtifact(id: string): void {
  router.go(`design/a/${encodeURIComponent(id)}`);
}

/** Open a product story on its Design tab (the story's designs grid). */
export function openStoryInProduct(storyId: string): void {
  product.view = 'stories';
  product.tab = 'mockups';
  void product.select(storyId).catch(() => {});
  router.go('product');
}

/** Where the live sync lands a scene: polled this often, this many times
 *  (the sync runs ~1.5 s after the last change, so ~3 s covers a fresh one). */
const MIRROR_POLL_MS = 750;
const MIRROR_POLLS = 4;

/** The Hall artifact that mirrors a Canvas scene, or null when there is none
 *  yet. A 403/404 is a definite "no" (no Design Hall access / old daemon). */
async function sceneMirror(sceneId: string): Promise<SceneHallCopy | null | 'unavailable'> {
  try {
    const rows = await listArtifacts({ source_kind: 'canvas_scene', source_id: sceneId, include_archived: true, limit: 1 });
    const a = rows[0];
    return a ? { id: a.id, adopted: a.meta?.adopted === true, behind: syncErrorOf(a)?.reason ?? null } : null;
  } catch (e) {
    const status = (e as { status?: number }).status;
    return status === 403 || status === 404 || status === 405 ? 'unavailable' : null;
  }
}

/**
 * A Canvas scene's Hall copy (`source_kind=canvas_scene&source_id=<id>`). A
 * scene made a moment ago may not be mirrored yet, so this waits briefly for
 * the live sync (`polls` lookups). Null: still no copy — the daemon could not
 * make one (too large with its images, an image missing) or is slow.
 */
export async function findSceneHallCopy(sceneId: string, polls = MIRROR_POLLS): Promise<SceneHallCopy | null | 'unavailable'> {
  let hit: SceneHallCopy | null | 'unavailable' = null;
  for (let i = 0; i < polls; i++) {
    hit = await sceneMirror(sceneId);
    if (hit) return hit;
    if (i < polls - 1) await new Promise((r) => setTimeout(r, MIRROR_POLL_MS));
  }
  return hit;
}

/** The old Canvas editor on one board (`#/canvas/<id>/editor`, CanvasRedirect). */
export function canvasEditorRoute(sceneId: string): string {
  return `canvas/${encodeURIComponent(sceneId)}/editor`;
}

/**
 * Open a Canvas scene where it lives now: its Design Hall copy — unless there
 * is none (it never reached the Hall) or the copy is a mirror that is behind
 * the board (`meta.sync_error`); then the board opens in the kept Canvas
 * editor, which says why. Never a dead end. `replace` swaps the current
 * history entry.
 */
export async function openSceneInHall(sceneId: string, opts: { replace?: boolean } = {}): Promise<void> {
  const go = (path: string) => (opts.replace ? router.replace(path) : router.go(path));
  const copy = await findSceneHallCopy(sceneId);
  go(opensInHall(copy) ? `design/a/${encodeURIComponent(copy.id)}` : canvasEditorRoute(sceneId));
}

/** Where an imported artifact's original lives, if it can be opened. A Canvas
 *  scene's mirror IS where it opens, so it only has an origin when the copy is
 *  behind the board (`meta.sync_error`): then the board opens in the Canvas
 *  editor. A story attachment opens on its story. */
export function originOf(a: DesignArtifact): { label: string; open: () => void } | null {
  if (a.source_kind === 'canvas_scene' && a.source_id && syncErrorOf(a)) {
    const sceneId = a.source_id;
    return { label: 'Open the board', open: () => router.go(canvasEditorRoute(sceneId)) };
  }
  if (a.source_kind === 'product_attachment') {
    const imported = (a.meta?.imported_from ?? {}) as Record<string, unknown>;
    const storyId = typeof imported.story_id === 'string' ? imported.story_id : null;
    if (storyId) return { label: 'Open in Product', open: () => openStoryInProduct(storyId) };
  }
  return null;
}
