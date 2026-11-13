<script lang="ts">
  // Images a tool returned (screenshots, renders): readable previews with the
  // aspect kept; a click opens the shared Lightbox (fit / 1:1).
  import Lightbox from '../../../lib/components/Lightbox.svelte';
  import type { ToolOutputImage } from '../../../lib/api/types';
  import { toolImageSources } from './toolImages';

  let { images = [] }: { images?: ToolOutputImage[] } = $props();
  const sources = $derived(toolImageSources(images));
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
