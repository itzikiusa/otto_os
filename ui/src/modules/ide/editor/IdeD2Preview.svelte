<script lang="ts">
  // A `.d2` file → SVG through Canvas' lazy D2 bridge (the WASM worker; output
  // sanitized there). The IDE preview pane's renderer for the one kind the
  // shared FilePreview cannot draw (lib/ never imports a module). Latest-wins
  // by sequence token; a broken source keeps the last good diagram, dimmed.
  import { untrack } from 'svelte';
  import PreviewNotice from '../../../lib/components/preview/PreviewNotice.svelte';
  import Skeleton from '../../../lib/components/Skeleton.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { ui } from '../../../lib/stores/ui.svelte';
  import { renderD2 } from '../../canvas/d2';

  interface Props {
    text: string;
  }
  let { text }: Props = $props();

  let svg = $state<string | null>(null);
  let error = $state<string | null>(null);
  let loading = $state(true);

  let seq = 0;
  let idSeq = 0;
  $effect(() => {
    const src = text;
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
      void renderD2(`ide-d2-${++idSeq}`, src, { dark, isStale: () => mine !== seq })
        .then((r) => {
          if (mine !== seq || r.stale) return;
          loading = false;
          if (r.svg) {
            svg = r.svg;
            error = null;
          } else error = r.error ?? 'Diagram error';
        })
        .catch((e: unknown) => {
          if (mine !== seq) return;
          loading = false;
          error = loadErrorText(e);
        });
    });
    return () => {
      seq++;
    };
  });
</script>

<div class="ide-d2" data-testid="ide-preview-d2">
  {#if error}
    <PreviewNotice title="D2 error" message={error} />
  {/if}
  {#if loading}
    <Skeleton rows={1} height={160} label="the diagram" />
  {:else if !text.trim()}
    <p class="ide-d2-empty">Empty diagram — write some D2 to see it here.</p>
  {/if}
  {#if svg}
    <!-- SVG from the D2 bridge (sanitized there) — the same path Canvas uses. -->
    <div class="ide-d2-figure" class:dim={!!error}>{@html svg}</div>
  {/if}
</div>

<style>
  .ide-d2 {
    flex: 1;
    min-block-size: 0;
    overflow: auto;
    padding: 8px;
    background: var(--bg);
  }
  .ide-d2-empty {
    margin: 0;
    padding: 12px;
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .ide-d2-figure {
    display: flex;
    justify-content: center;
    padding: 12px;
  }
  .ide-d2-figure.dim {
    opacity: var(--disabled-opacity);
  }
  .ide-d2-figure :global(svg) {
    max-inline-size: 100%;
    block-size: auto;
  }
</style>
