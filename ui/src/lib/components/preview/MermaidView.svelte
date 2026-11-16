<script lang="ts">
  // A `.mmd` / `.mermaid` file → SVG through the app's lazy Mermaid bridge
  // (lib/render/mermaid.ts — securityLevel 'strict', the same one the vault,
  // Canvas and Workbench use). Latest-wins by sequence token; while the source
  // is broken the last good diagram stays visible, dimmed, under the error.
  import { untrack } from 'svelte';
  import { renderMermaid } from '../../render/mermaid';
  import { ui } from '../../stores/ui.svelte';
  import Skeleton from '../Skeleton.svelte';
  import PreviewNotice from './PreviewNotice.svelte';

  interface Props {
    content: string;
  }
  let { content }: Props = $props();

  let svg = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);

  let seq = 0;
  let idSeq = 0;
  $effect(() => {
    const src = content;
    const dark = ui.resolvedScheme === 'dark';
    const mine = ++seq;
    untrack(() => {
      if (!src.trim()) {
        svg = null;
        error = null;
        loading = false;
        return;
      }
      loading = svg === null;
      void renderMermaid(`fp-mmd-${++idSeq}`, src, { dark, isStale: () => mine !== seq }).then((r) => {
        if (mine !== seq || r.stale) return;
        loading = false;
        if (r.svg) {
          svg = r.svg;
          error = null;
        } else error = r.error ?? 'Diagram error';
      });
    });
  });

  /** Mermaid errors often carry "line N" — surface it in the header. */
  const errLine = $derived.by(() => {
    const m = error ? /line (\d+)/i.exec(error) : null;
    return m ? Number(m[1]) : undefined;
  });
</script>

<div class="fp-mmd">
  {#if error}
    <PreviewNotice title="Mermaid error" message={error} line={errLine} />
  {/if}
  {#if loading}
    <Skeleton rows={1} height={160} label="the diagram" />
  {:else if !content.trim()}
    <p class="fp-mmd-empty">Empty diagram — write some Mermaid to see it here.</p>
  {/if}
  {#if svg}
    <!-- SVG from the strict Mermaid bridge — the same path the vault uses. -->
    <div class="fp-mmd-figure" class:dim={!!error}>{@html svg}</div>
  {/if}
</div>

<style>
  .fp-mmd {
    padding: 8px;
  }
  .fp-mmd-empty {
    margin: 0;
    padding: 12px;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .fp-mmd-figure {
    display: flex;
    justify-content: center;
    padding: 12px;
    overflow: auto;
  }
  .fp-mmd-figure.dim {
    opacity: var(--disabled-opacity);
  }
  .fp-mmd-figure :global(svg) {
    max-inline-size: 100%;
    block-size: auto;
  }
</style>
