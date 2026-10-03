<script lang="ts">
  // The vault's page-preview popover (state in previewStore.svelte.ts). Clamped
  // into the viewport: hangs under the anchor, flips above it only when that
  // fits, otherwise pins to the bottom edge; height capped + scrollable.
  import { linkPreview } from './previewStore.svelte';

  let el = $state<HTMLElement | undefined>();
  let h = $state(120);
  $effect(() => {
    void linkPreview.current?.state;
    if (el) h = el.offsetHeight;
  });
  const style = $derived.by(() => {
    const p = linkPreview.current;
    if (!p) return '';
    const vw = window.innerWidth, vh = window.innerHeight;
    const w = Math.min(320, vw - 16);
    const left = Math.max(8, Math.min(p.x, vw - w - 8));
    const maxH = Math.max(80, Math.min(360, vh - 16));
    const height = Math.min(h, maxH);
    let top = p.y;
    if (top + height > vh - 8) top = p.top - 6 - height >= 8 ? p.top - 6 - height : Math.max(8, vh - height - 8);
    return `left:${left}px;top:${top}px;width:${w}px;max-height:${maxH}px`;
  });
</script>

{#if linkPreview.current}
  {@const p = linkPreview.current}
  <div class="lp" role="tooltip" bind:this={el} {style} data-testid="vault-link-preview">
    <div class="lp-title">{p.title}{#if p.type}<span class="lp-type">{p.type}</span>{/if}</div>
    <div class="lp-path">{p.path}</div>
    {#if p.state === 'loading'}<p class="lp-dim" role="status">Loading preview…</p>
    {:else if p.state === 'error'}<p class="lp-dim">Preview unavailable</p>
    {:else if p.text}<p>{p.text}</p>
    {:else}<p class="lp-dim">Empty note</p>{/if}
  </div>
{/if}

<style>
  .lp {
    position: fixed;
    z-index: var(--z-popover);
    overflow-y: auto;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    box-shadow: var(--shadow);
    font-size: var(--fs-s);
    pointer-events: none;
  }
  .lp-title { display: flex; gap: 6px; align-items: center; font-weight: 600; overflow-wrap: anywhere; }
  .lp-type {
    font-size: var(--fs-xs);
    font-weight: 400;
    padding: 1px 7px;
    border: 1px solid var(--border);
    border-radius: 999px;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .lp-path { font-size: var(--fs-xs); color: var(--text-dim); margin-block: 2px 6px; overflow-wrap: anywhere; }
  p { margin: 0; overflow-wrap: anywhere; }
  .lp-dim { color: var(--text-dim); }
  @media (prefers-reduced-motion: no-preference) {
    .lp { animation: lp-in 120ms ease-out; }
  }
  @keyframes lp-in { from { opacity: 0; } to { opacity: 1; } }
</style>
