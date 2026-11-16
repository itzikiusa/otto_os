<script lang="ts">
  // The images / PDFs waiting in a runtime composer, each removable before
  // the message is sent: an image shows as a readable preview (aspect kept,
  // never cropped) that opens full size on click, a PDF as its name.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';
  import Lightbox from '../../../lib/components/Lightbox.svelte';

  let { images, onremove }: { images: ComposerFile[]; onremove: (index: number) => void } = $props();
  let opened = $state<ComposerFile | null>(null);
</script>

<div class="rt-images" data-testid="runtime-images">
  {#each images as img, i (i)}
    {#if img.mime.startsWith('image/')}
      <span class="rt-thumb">
        <button type="button" class="rt-thumb-open" aria-label="Open {img.name}" title={img.name} onclick={() => (opened = img)}>
          <img src="data:{img.mime};base64,{img.data_base64}" alt={img.name} />
        </button>
        <button type="button" class="rt-thumb-x" aria-label="Remove {img.name}" title="Remove" onclick={() => onremove(i)}>
          <Icon name="x" size={12} />
        </button>
      </span>
    {:else}
      <span class="rt-image">
        <Icon name="file" size={12} /><span class="ellipsis" dir="auto">{img.name}</span>
        <button type="button" class="icon-btn" aria-label="Remove {img.name}" title="Remove" onclick={() => onremove(i)}>
          <Icon name="x" size={12} />
        </button>
      </span>
    {/if}
  {/each}
</div>

{#if opened}
  <Lightbox src="data:{opened.mime};base64,{opened.data_base64}" alt={opened.name} testid="runtime-image-full" onclose={() => (opened = null)} />
{/if}

<style>
  .rt-images {
    display: flex;
    flex-wrap: wrap;
    align-items: flex-end;
    gap: 8px;
  }
  .rt-thumb {
    position: relative;
    max-inline-size: 100%;
  }
  /* Readable, uncropped: 160px tall, width from the aspect (64–240px). */
  .rt-thumb-open {
    display: grid;
    place-items: center;
    min-inline-size: 64px;
    min-block-size: 64px;
    max-inline-size: 100%;
    padding: 0;
    overflow: hidden;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    cursor: zoom-in;
  }
  .rt-thumb-open:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .rt-thumb img {
    display: block;
    block-size: 160px;
    inline-size: auto;
    min-inline-size: 64px;
    max-inline-size: min(240px, 100%);
    object-fit: contain;
  }
  .rt-thumb-x {
    position: absolute;
    inset-block-start: 4px;
    inset-inline-end: 4px;
    display: grid;
    place-items: center;
    inline-size: 22px;
    block-size: 22px;
    padding: 0;
    border: 1px solid var(--border);
    border-radius: 50%;
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
  }
  .rt-image {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    max-inline-size: 220px;
    padding-inline: 8px 2px;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
</style>
