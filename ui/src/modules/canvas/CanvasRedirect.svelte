<script lang="ts">
  // `#/canvas` — the Canvas page folded into Design Hall: every scene is
  // mirrored there live and opens as a Whiteboard design. This page only
  // forwards: a scene someone asked to open (`canvas.pendingOpenId`, set by
  // callers that still use the old hand-off, or `#/canvas/<scene id>`) goes to
  // its Hall mirror; anything else lands on the Whiteboard studio. The old
  // editor (CanvasPage.svelte, SceneList, ConversationPanel, the canvas store)
  // is kept, unmounted — pointing the `canvas` loader in shell/pages.svelte.ts
  // back at CanvasPage.svelte undoes this.
  import { onMount } from 'svelte';
  import { router } from '../../lib/router.svelte';
  import { canvas } from '../../lib/stores/canvas.svelte';
  import { openSceneInHall } from '../design-hall/nav'; // ui-guards: allow — the Canvas route forwards into Design Hall
  import Skeleton from '../../lib/components/Skeleton.svelte';

  onMount(() => {
    const sceneId = canvas.pendingOpenId ?? router.parts[1] ?? null;
    canvas.pendingOpenId = null;
    if (sceneId) void openSceneInHall(sceneId, { replace: true });
    else router.replace('design/studio/whiteboard');
  });
</script>

<div class="fwd" aria-busy="true" aria-label="Opening Design Hall">
  <Skeleton rows={2} height={160} />
</div>

<style>
  .fwd {
    padding: 16px;
  }
</style>
