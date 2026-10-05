<script lang="ts">
  // Image preview: a workbench asset (content = asset id) fetched with auth
  // into an object URL, or an SVG document shown through a data-URL `<img>`
  // (an <img> never runs scripts). Fit-to-pane by default; toggle to 1:1.
  import { onDestroy } from 'svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import { workbenchAssetUrl } from '../../../lib/api/workbench';
  import { svgDataUrl } from './kinds';
  import PreviewError from './PreviewError.svelte';

  interface Props {
    ws: string;
    /** Asset id (`image`), or SVG markup when `svg` is set. */
    content: string;
    name: string;
    svg?: boolean;
  }
  let { ws, content, name, svg = false }: Props = $props();

  let url = $state<string | null>(null);
  let owned: string | null = null; // object URL we must revoke
  let error = $state<string | null>(null);
  let loading = $state(false);
  let fit = $state(true);
  let dims = $state<{ w: number; h: number } | null>(null);
  let attempt = $state(0);

  function release(): void {
    if (owned) URL.revokeObjectURL(owned);
    owned = null;
  }
  onDestroy(release);

  let seq = 0;
  $effect(() => {
    void attempt;
    const id = content.trim();
    const isSvg = svg;
    const workspace = ws;
    const mine = ++seq;
    release();
    dims = null;
    error = null;
    if (isSvg) {
      loading = false;
      url = id ? svgDataUrl(content) : null;
      if (!id) error = 'Empty SVG document';
      return;
    }
    if (!id) {
      url = null;
      loading = false;
      error = 'This image file has no stored asset.';
      return;
    }
    loading = true;
    url = null;
    workbenchAssetUrl(workspace, id).then(
      (u) => {
        if (mine !== seq) return URL.revokeObjectURL(u);
        owned = u;
        url = u;
        loading = false;
      },
      (e: unknown) => {
        if (mine !== seq) return;
        loading = false;
        error = e instanceof Error ? e.message : String(e);
      },
    );
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
  {#if loading}
    <div class="state"><Skeleton rows={1} height={200} label="the image" /></div>
  {:else if error}
    <PreviewError title="Image unavailable" message={error} onretry={svg ? undefined : () => attempt++} />
  {:else if url}
    <div class="stage" class:fit>
      <img src={url} alt={name} {onload} onerror={() => (error = 'The image could not be decoded.')} />
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
  .state {
    padding: 16px;
    color: var(--text-dim);
    font-size: var(--fs-m);
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
