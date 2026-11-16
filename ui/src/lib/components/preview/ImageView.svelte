<script lang="ts">
  // An image file shown as an image, never as a "binary file" editor: its pixel
  // size and byte size, zoom to fit (default) or 100%. `src` is a blob: / data:
  // URL (an <img> never runs an SVG's script) or a plain URL.
  import { formatBytes } from '../../metric-format';
  import PreviewNotice from './PreviewNotice.svelte';

  interface Props {
    src: string;
    name: string;
  }
  let { src, name }: Props = $props();

  let fit = $state(true);
  let dims = $state<{ w: number; h: number } | null>(null);
  let bytes = $state<number | null>(null);
  let broken = $state(false);

  $effect(() => {
    const u = src;
    dims = null;
    bytes = null;
    broken = false;
    // Local URLs: read the size back from the bytes already in memory.
    if (!u.startsWith('blob:') && !u.startsWith('data:')) return;
    let live = true;
    void fetch(u)
      .then((r) => r.blob())
      .then((b) => {
        if (live) bytes = b.size;
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  });

  const meta = $derived(
    [dims ? `${dims.w} × ${dims.h} px` : '', bytes !== null ? formatBytes(bytes) : ''].filter(Boolean).join(' · '),
  );
</script>

<div class="fp-img" data-testid="file-preview-image">
  <div class="fp-img-bar">
    <span class="fp-img-meta" data-testid="file-preview-image-meta">{meta}</span>
    <div class="segmented" role="group" aria-label="Image zoom">
      <button type="button" aria-pressed={fit} title="Zoom to fit the pane" onclick={() => (fit = true)}>Fit</button>
      <button type="button" aria-pressed={!fit} title="Actual size" onclick={() => (fit = false)}>100%</button>
    </div>
  </div>
  {#if broken}
    <PreviewNotice title="Image unavailable" message="The image couldn’t be decoded." />
  {:else}
    <div class="fp-img-stage" class:fit>
      <img
        {src}
        alt={name}
        onload={(e) => {
          const img = e.currentTarget as HTMLImageElement;
          dims = { w: img.naturalWidth, h: img.naturalHeight };
        }}
        onerror={() => (broken = true)}
      />
    </div>
  {/if}
</div>

<style>
  .fp-img {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
  }
  .fp-img-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    border-block-end: 1px solid var(--border);
    flex-shrink: 0;
  }
  .fp-img-meta {
    color: var(--text-dim);
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .fp-img-stage {
    flex: 1;
    min-block-size: 0;
    overflow: auto;
    padding: 12px;
    /* neutral backdrop so transparent PNGs stay visible */
    background: var(--surface-2);
  }
  .fp-img-stage.fit {
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .fp-img-stage.fit img {
    max-inline-size: 100%;
    max-block-size: 100%;
    object-fit: contain;
  }
  .fp-img-stage img {
    display: block;
  }
</style>
