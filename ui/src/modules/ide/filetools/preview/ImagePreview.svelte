<script lang="ts">
  // SVG preview: the document shown through a data-URL `<img>` (an <img>
  // never runs scripts). Fit-to-pane by default; toggle to 1:1.
  import { svgDataUrl } from './kinds';
  import PreviewError from './PreviewError.svelte';

  interface Props {
    /** SVG markup. */
    content: string;
    name: string;
  }
  let { content, name }: Props = $props();

  let fit = $state(true);
  let dims = $state<{ w: number; h: number } | null>(null);
  let decodeError = $state<string | null>(null);
  const url = $derived(content.trim() ? svgDataUrl(content) : null);
  const error = $derived(url ? decodeError : 'Empty SVG document');
  $effect(() => {
    void url;
    dims = null;
    decodeError = null;
  });

  function onload(e: Event): void {
    const img = e.currentTarget as HTMLImageElement;
    dims = { w: img.naturalWidth, h: img.naturalHeight };
  }
</script>

<div class="img-wrap">
  <div class="bar">
    <span class="meta">{dims ? `${dims.w} × ${dims.h} px` : ''}</span>
    <button
      type="button"
      class="btn small ghost"
      aria-pressed={!fit}
      title={fit ? 'Show at actual size' : 'Fit to pane'}
      onclick={() => (fit = !fit)}
      disabled={!url}
    >
      {fit ? '1:1' : 'Fit'}
    </button>
  </div>
  {#if error}
    <PreviewError title="Image unavailable" message={error} />
  {:else if url}
    <div class="stage" class:fit>
      <img src={url} alt={name} {onload} onerror={() => (decodeError = 'The image couldn’t be decoded.')} />
    </div>
  {/if}
</div>

<style>
  .img-wrap {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
  }
  .meta {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .stage {
    flex: 1;
    min-block-size: 0;
    overflow: auto;
    padding: 12px;
    /* checkerboard-free neutral backdrop so transparent PNGs stay visible */
    background: var(--surface-2);
  }
  .stage.fit {
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .stage.fit img {
    max-inline-size: 100%;
    max-block-size: 100%;
    object-fit: contain;
  }
  .stage img {
    display: block;
  }
</style>
