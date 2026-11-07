<script lang="ts">
  import Modal from '../../../lib/components/Modal.svelte';
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
  <Modal title="Tool image" width={1000} onclose={() => (selected = null)}>
    <img class="enlarged" src={selected} alt="Enlarged tool result" data-testid="runtime-tool-image-enlarged" />
  </Modal>
{/if}

<style>
  .tool-images { display: flex; flex-wrap: wrap; gap: 8px; min-inline-size: 0; }
  .thumbnail { display: flex; align-items: center; justify-content: center; max-inline-size: 100%; inline-size: 160px; block-size: 112px; padding: 4px; border: 1px solid var(--border); border-radius: var(--radius-m); background: var(--surface-2); cursor: zoom-in; }
  .thumbnail:hover { border-color: var(--accent-text); }
  .thumbnail img { max-inline-size: 100%; max-block-size: 100%; object-fit: contain; }
  .enlarged { display: block; inline-size: 100%; max-block-size: 75dvh; object-fit: contain; }
  .image-error { margin: 0; color: var(--text-dim); font-size: var(--fs-xs); }
</style>
