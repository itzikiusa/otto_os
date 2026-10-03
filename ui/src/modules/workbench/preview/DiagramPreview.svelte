<script lang="ts">
  // Mermaid / D2 source → SVG through the app's lazy bridges (the same ones
  // the vault and Canvas use: canvas/mermaid.ts — securityLevel 'strict' — and
  // canvas/d2.ts — the WASM worker). Latest-wins: a render superseded by a
  // newer edit is skipped/discarded by sequence token.
  import { untrack } from 'svelte';
  import { renderMermaid } from '../../canvas/mermaid';
  import { renderD2 } from '../../canvas/d2';
  import { ui } from '../../../lib/stores/ui.svelte';
  import PreviewError from './PreviewError.svelte';

  interface Props {
    kind: 'mermaid' | 'd2';
    content: string;
  }
  let { kind, content }: Props = $props();

  let svg = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);
  let attempt = $state(0);

  let seq = 0;
  let idSeq = 0;
  $effect(() => {
    void attempt;
    const src = content;
    const k = kind;
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
      const id = `wb-diag-${++idSeq}`;
      const isStale = () => mine !== seq;
      const run = k === 'd2' ? renderD2(id, src, { dark, isStale }) : renderMermaid(id, src, { dark, isStale });
      void run.then((r) => {
        if (mine !== seq || r.stale) return;
        loading = false;
        if (r.svg) {
          svg = r.svg;
          error = null;
        } else {
          error = r.error ?? 'Diagram error';
        }
      });
    });
  });

  /** Mermaid errors often carry "line N" — surface it in the header. */
  const errLine = $derived.by(() => {
    const m = error ? /line (\d+)/i.exec(error) : null;
    return m ? Number(m[1]) : undefined;
  });
</script>

<div class="diag">
  {#if error}
    <PreviewError
      title={kind === 'd2' ? 'D2 error' : 'Mermaid error'}
      message={error}
      line={errLine}
      onretry={() => attempt++}
    />
  {/if}
  {#if loading}
    <div class="state" aria-busy="true">Rendering {kind === 'd2' ? 'D2' : 'Mermaid'} diagram…</div>
  {:else if !content.trim()}
    <div class="state">Write some {kind === 'd2' ? 'D2' : 'Mermaid'} to see the diagram.</div>
  {/if}
  {#if svg}
    <!-- SVG from the mermaid (strict) / D2 bridges — the same path the vault uses. -->
    <div class="figure" class:dim={!!error}>{@html svg}</div>
  {/if}
</div>

<style>
  .diag {
    padding: 8px;
    min-block-size: 100%;
  }
  .state {
    padding: 12px;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .figure {
    display: flex;
    justify-content: center;
    padding: 12px;
    overflow: auto;
  }
  /* last good render stays visible (dimmed) while the source is broken */
  .figure.dim {
    opacity: 0.45;
  }
  .figure :global(svg) {
    max-inline-size: 100%;
    block-size: auto;
  }
</style>
