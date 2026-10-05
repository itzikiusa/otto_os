<script lang="ts">
  // Markdown preview: the vault reading-view renderer (marked GFM + sanitizer,
  // highlighted fences) so a scratch note reads exactly like a vault note.
  // Mermaid / D2 fences render to SVG through the same lazy bridges the vault
  // uses; `![alt](asset:<id>)` images resolve to workbench assets.
  import { onDestroy, untrack } from 'svelte';
  import { renderNote } from '../../vault/mdRender';
  import { renderMermaid } from '../../canvas/mermaid';
  import { renderD2 } from '../../canvas/d2';
  import { workbenchAssetUrl } from '../../../lib/api/workbench';
  import { ui } from '../../../lib/stores/ui.svelte';

  interface Props {
    ws: string;
    content: string;
  }
  let { ws, content }: Props = $props();

  let host: HTMLDivElement | undefined = $state();
  const html = $derived(
    renderNote(content, { resolve: () => null, assetUrl: () => null, lazyAssets: true }),
  );

  // Object URLs minted for asset images; revoked on re-render and unmount.
  let blobUrls: string[] = [];
  function revokeAll(): void {
    for (const u of blobUrls) URL.revokeObjectURL(u);
    blobUrls = [];
  }
  onDestroy(revokeAll);

  let seq = 0;
  $effect(() => {
    void html;
    const el = host;
    if (!el) return;
    const dark = ui.resolvedScheme === 'dark';
    untrack(() => {
      revokeAll();
      // Images: only `asset:<id>` refs resolve; anything else is left alt-only.
      for (const img of Array.from(el.querySelectorAll<HTMLImageElement>('img[data-asset]'))) {
        const ref = img.getAttribute('data-asset') ?? '';
        const m = /^asset:([A-Za-z0-9_-]+)$/.exec(ref);
        if (!m) continue;
        void workbenchAssetUrl(ws, m[1]).then(
          (url) => {
            if (!img.isConnected) return URL.revokeObjectURL(url);
            blobUrls.push(url);
            img.src = url;
          },
          () => img.setAttribute('alt', `${img.alt || 'image'} (failed to load)`),
        );
      }
      for (const block of Array.from(el.querySelectorAll('div.diagram-block:not([data-rendered])'))) {
        block.setAttribute('data-rendered', '1');
        const kind = block.getAttribute('data-diagram');
        const src = block.querySelector('pre.diagram-src')?.textContent ?? '';
        const id = `wb-md-diag-${++seq}`;
        const isStale = () => !block.isConnected;
        const run =
          kind === 'd2' ? renderD2(id, src, { dark, isStale }) : renderMermaid(id, src, { dark, isStale });
        void run.then(({ svg, error }) => {
          if (!block.isConnected) return;
          if (svg) {
            block.innerHTML = svg;
            block.classList.add('diagram-ok');
          } else if (error) {
            const err = document.createElement('div');
            err.className = 'diagram-error';
            err.textContent = `Diagram error: ${error}`;
            block.prepend(err);
          }
        });
      }
    });
  });
</script>

<!-- html is sanitised by renderNote (allowlist sanitizer). -->
<div class="md" bind:this={host}>{@html html}</div>

<style>
  .md {
    padding: 16px 20px;
    font-size: var(--fs-m);
    line-height: 1.6;
    color: var(--text);
    max-inline-size: 860px;
  }
  .md :global(h1) { font-size: var(--fs-2xl); margin: 0.6em 0 0.4em; }
  .md :global(h2) { font-size: var(--fs-xl); margin: 0.8em 0 0.4em; }
  .md :global(h3) { font-size: var(--fs-l); margin: 0.8em 0 0.3em; }
  .md :global(p) { margin: 0.5em 0; }
  .md :global(a) { color: var(--accent-text); }
  .md :global(code) {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    background: var(--surface-2);
    padding: 1px 4px;
    border-radius: var(--radius-s);
  }
  .md :global(pre) {
    background: var(--surface-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 10px 12px;
    overflow-x: auto;
  }
  .md :global(pre code) { background: none; padding: 0; }
  .md :global(blockquote) {
    margin: 0.6em 0;
    padding-inline-start: 12px;
    border-inline-start: 3px solid var(--border-strong);
    color: var(--text-dim);
  }
  .md :global(table) { border-collapse: collapse; display: block; overflow-x: auto; }
  .md :global(th), .md :global(td) {
    border: 1px solid var(--border);
    padding: 4px 8px;
    text-align: start;
  }
  .md :global(th) { background: var(--surface-2); }
  .md :global(img) { max-inline-size: 100%; height: auto; }
  .md :global(div.diagram-block) {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 14px;
    margin: 10px 0;
    background: var(--surface-2);
    overflow-x: auto;
  }
  .md :global(div.diagram-block.diagram-ok) { display: flex; justify-content: center; }
  .md :global(div.diagram-block svg) { max-width: 100%; height: auto; }
  .md :global(div.diagram-error) {
    color: var(--danger);
    font-size: var(--fs-s);
    margin-bottom: 8px;
  }
  .md :global(pre.diagram-src) { margin: 0; font-size: var(--fs-s); }
  .md :global(.unresolved) { color: var(--text-dim); }
</style>
