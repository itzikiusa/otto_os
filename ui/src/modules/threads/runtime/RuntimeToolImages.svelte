<script lang="ts">
  // Images a tool returned (screenshots, renders): readable previews with the
  // aspect kept; a click opens the shared Lightbox (fit / 1:1). Images a tool
  // left on disk (Codex `imageView` / `imageGeneration`, `image_files`) load
  // from the daemon's read-only route for this thread.
  import { getContext, onDestroy } from 'svelte';
  import Lightbox from '../../../lib/components/Lightbox.svelte';
  import { authedBlobUrl } from '../../../lib/api/client';
  import type { ToolOutputImage } from '../../../lib/api/types';
  import { PATH_LINK_CTX } from '../../../lib/pathLinks/open';
  import { toolImageSources } from './toolImages';

  let { images = [], files = [], itemId = '' }: { images?: ToolOutputImage[]; files?: string[]; itemId?: string } = $props();
  const target = getContext<(() => { sessionId?: string }) | undefined>(PATH_LINK_CTX);
  let fileUrls = $state<string[]>([]);
  $effect(() => {
    const sid = target?.().sessionId;
    const n = files.length;
    if (!sid || !itemId || n === 0) return;
    let alive = true;
    void Promise.all(
      Array.from({ length: n }, (_, i) =>
        authedBlobUrl(`/runtime-sessions/${encodeURIComponent(sid)}/items/${encodeURIComponent(itemId)}/images/${i}`).catch(() => null),
      ),
    ).then((urls) => {
      const got = urls.filter((u): u is string => !!u);
      if (alive) fileUrls = got;
      else for (const u of got) URL.revokeObjectURL(u);
    });
    return () => {
      alive = false;
    };
  });
  onDestroy(() => {
    for (const u of fileUrls) URL.revokeObjectURL(u);
  });
  const sources = $derived([...toolImageSources(images), ...fileUrls]);
  let selected = $state<string | null>(null);
  let failed = $state<string[]>([]);
</script>

{#if sources.length}
  <div class="tool-images" data-testid="runtime-tool-images">
    {#each sources as src, i}
      {#if failed.includes(src)}
        <p class="image-error">Image {i + 1} could not be displayed.</p>
      {:else}
        <button type="button" class="thumbnail" aria-label="Enlarge tool image {i + 1}" title="Enlarge tool image {i + 1}" onclick={() => (selected = src)}>
          <img {src} alt="Tool result {i + 1}" loading="lazy" onerror={() => (failed = [...failed, src])} />
        </button>
      {/if}
    {/each}
  </div>
{/if}

{#if selected}
  <Lightbox src={selected} alt="Tool image" testid="runtime-tool-image-enlarged" onclose={() => (selected = null)} />
{/if}

<style>
  .tool-images { display: flex; flex-wrap: wrap; gap: 8px; min-inline-size: 0; }
  .thumbnail { display: grid; place-items: center; min-inline-size: 64px; min-block-size: 64px; max-inline-size: 100%; padding: 0; overflow: hidden; border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface-2); cursor: zoom-in; }
  .thumbnail:hover { border-color: var(--accent-text); }
  .thumbnail:focus-visible { outline: 2px solid var(--accent-text); outline-offset: 2px; }
  /* A fixed readable height, width from the aspect (capped), never cropped. */
  .thumbnail img { display: block; block-size: 160px; inline-size: auto; min-inline-size: 64px; max-inline-size: min(240px, 100%); object-fit: contain; }
  .image-error { margin: 0; color: var(--text-dim); font-size: var(--fs-xs); }
</style>
