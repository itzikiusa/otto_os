<script lang="ts">
  // The images / PDFs waiting in a runtime composer, each removable before
  // the message is sent.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { ComposerFile } from '../../../lib/stores/runtimeDrafts.svelte';

  let { images, onremove }: { images: ComposerFile[]; onremove: (index: number) => void } = $props();
</script>

<div class="rt-images" data-testid="runtime-images">
  {#each images as img, i (i)}
    <span class="rt-image">
      <Icon name={img.mime.startsWith('image/') ? 'image' : 'file'} size={12} /><span class="ellipsis" dir="auto">{img.name}</span>
      <button type="button" class="icon-btn" aria-label="Remove {img.name}" title="Remove" onclick={() => onremove(i)}>
        <Icon name="x" size={12} />
      </button>
    </span>
  {/each}
</div>

<style>
  .rt-images {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
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
