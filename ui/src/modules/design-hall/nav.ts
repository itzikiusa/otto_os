// Design Hall navigation helpers — the few places the Hall hands off to other
// modules (Product story → Design tab, a session) and the one door other
// modules use to open a Canvas scene, which now lives in the Hall as a mirror.

import { router } from '../../lib/router.svelte';
import { product } from '../../lib/stores/product.svelte';
import { toasts } from '../../lib/toast.svelte';
import { listArtifacts } from '../../lib/api/design';
import type { DesignArtifact } from '../../lib/api/types';

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
async function sceneMirror(sceneId: string): Promise<{ id: string } | null | 'unavailable'> {
  try {
    const rows = await listArtifacts({ source_kind: 'canvas_scene', source_id: sceneId, include_archived: true, limit: 1 });
    return rows[0] ? { id: rows[0].id } : null;
  } catch (e) {
    const status = (e as { status?: number }).status;
    return status === 403 || status === 404 || status === 405 ? 'unavailable' : null;
  }
}

/**
 * Open a Canvas scene where it lives now: its Design Hall mirror
 * (`source_kind=canvas_scene&source_id=<id>`). A scene made a moment ago may
 * not be mirrored yet, so this waits briefly for the live sync, then falls
 * back to the Whiteboard studio with a note — never a dead end. `replace`
 * swaps the current history entry (the `#/canvas` redirect uses it).
 */
export async function openSceneInHall(sceneId: string, opts: { replace?: boolean } = {}): Promise<void> {
  const go = (path: string) => (opts.replace ? router.replace(path) : router.go(path));
  for (let i = 0; i < MIRROR_POLLS; i++) {
    const hit = await sceneMirror(sceneId);
    if (hit === 'unavailable') break;
    if (hit) {
      go(`design/a/${encodeURIComponent(hit.id)}`);
      return;
    }
    if (i < MIRROR_POLLS - 1) await new Promise((r) => setTimeout(r, MIRROR_POLL_MS));
  }
  toasts.info('Opened the Whiteboard studio', 'That board hasn’t reached Design Hall yet. It appears here within a few seconds.');
  go('design/studio/whiteboard');
}

/** Where an imported artifact's original lives, if it can be opened. A Canvas
 *  scene has no other home any more (the Canvas page folded into the Hall:
 *  its mirror IS where it opens), so only a story attachment has an origin. */
export function originOf(a: DesignArtifact): { label: string; open: () => void } | null {
  if (a.source_kind === 'product_attachment') {
    const imported = (a.meta?.imported_from ?? {}) as Record<string, unknown>;
    const storyId = typeof imported.story_id === 'string' ? imported.story_id : null;
    if (storyId) return { label: 'Open in Product', open: () => openStoryInProduct(storyId) };
  }
  return null;
}
