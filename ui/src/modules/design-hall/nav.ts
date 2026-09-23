// Design Hall navigation helpers — the few places the Hall hands off to other
// modules (Product story → Design tab, Canvas, a session).

import { router } from '../../lib/router.svelte';
import { product } from '../../lib/stores/product.svelte';
import { canvas } from '../../lib/stores/canvas.svelte';
import type { DesignArtifact } from '../../lib/api/types';

export function openArtifact(id: string): void {
  router.go(`design/a/${encodeURIComponent(id)}`);
}

/** Open a product story on its Design tab (the arena + the graph strip). */
export function openStoryInProduct(storyId: string): void {
  product.view = 'stories';
  product.tab = 'mockups';
  void product.select(storyId).catch(() => {});
  router.go('product');
}

/** Where an imported artifact's original lives, if it can be opened. */
export function originOf(a: DesignArtifact): { label: string; open: () => void } | null {
  if (a.source_kind === 'canvas_scene' && a.source_id) {
    const sceneId = a.source_id;
    return {
      label: 'Open in Canvas',
      open: () => {
        canvas.pendingOpenId = sceneId;
        router.go('canvas');
      },
    };
  }
  if (a.source_kind === 'product_attachment') {
    const imported = (a.meta?.imported_from ?? {}) as Record<string, unknown>;
    const storyId = typeof imported.story_id === 'string' ? imported.story_id : null;
    if (storyId) return { label: 'Open in Product', open: () => openStoryInProduct(storyId) };
  }
  return null;
}
