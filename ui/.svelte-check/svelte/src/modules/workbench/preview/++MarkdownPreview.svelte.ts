///<reference types="svelte" />
;// Markdown preview: the vault reading-view renderer (marked GFM + sanitizer,
// highlighted fences) so a scratch note reads exactly like a vault note.
// Mermaid / D2 fences render to SVG through the same lazy bridges the vault
// uses; `![alt](asset:<id>)` images resolve to workbench assets.

import { onDestroy, untrack } from 'svelte';
import { renderNote } from '../../../lib/render/markdown';
import { renderMermaid } from '../../../lib/render/mermaid';
import { renderD2 } from '../../canvas/d2';
import { workbenchAssetUrl } from '../../../lib/api/workbench';
import { ui } from '../../../lib/stores/ui.svelte';
;

  interface Props {
    ws: string;
    content: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

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
          () => img.setAttribute('alt', `${img.alt || 'image'} (couldn’t load)`),
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
;
async () => {


 { const $$_div0 = svelteHTML.createElement("div", {  "class":`md`,});host = $$_div0; html; }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MarkdownPreview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MarkdownPreview__SvelteComponent_ = ReturnType<typeof MarkdownPreview__SvelteComponent_>;
/*Ωignore_endΩ*/export default MarkdownPreview__SvelteComponent_;