<script lang="ts">
  // The images / PDFs waiting in a runtime composer, each removable before
  // the message is sent: an image shows as a preview, a PDF as its name.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';

  let { images, onremove }: { images: ComposerFile[]; onremove: (index: number) => void } = $props();
</script>

<div class="rt-images" data-testid="runtime-images">
  {#each images as img, i (i)}
    {#if img.mime.startsWith('image/')}
      <span class="rt-thumb" title={img.name}>
        <img src="data:{img.mime};base64,{img.data_base64}" alt={img.name} />
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

<style>
  .rt-images {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .rt-thumb {
    position: relative;
    inline-size: 56px;
    block-size: 56px;
    border-radius: var(--radius-s);
    overflow: hidden;
    background: var(--surface-2);
    border: 1px solid var(--border);
  }
  .rt-thumb img {
    inline-size: 100%;
    block-size: 100%;
    object-fit: cover;
    display: block;
  }
  .rt-thumb-x {
    position: absolute;
    inset-block-start: 2px;
    inset-inline-end: 2px;
    display: grid;
    place-items: center;
    inline-size: 20px;
    block-size: 20px;
    padding: 0;
    border: none;
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
