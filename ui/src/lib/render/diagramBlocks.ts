// Post-render pass for Markdown diagram fences: `renderNote` (./markdown.ts)
// emits each ```mermaid / ```d2 fence as `div.diagram-block` holding its source
// as text; this swaps every not-yet-rendered block under `root` for the SVG
// from the lazy bridges (mermaid here, securityLevel 'strict'; D2 only when the
// host hands its WASM bridge in — it lives with Canvas). A failed render keeps
// the source visible under an inline error. Used by the Workbench Markdown
// preview and the shared FilePreview (IDE, Threads).

import { renderMermaid } from './mermaid';

export type DiagramRenderer = (
  id: string,
  src: string,
  opts: { dark?: boolean; isStale?: () => boolean },
) => Promise<{ svg?: string; error?: string; stale?: boolean }>;

let seq = 0;

export function hydrateDiagramBlocks(
  root: ParentNode,
  opts: { dark: boolean; idPrefix: string; renderD2?: DiagramRenderer },
): void {
  for (const block of Array.from(root.querySelectorAll('div.diagram-block:not([data-rendered])'))) {
    const kind = block.getAttribute('data-diagram');
    // Without a D2 bridge the fence stays as readable source.
    if (kind === 'd2' && !opts.renderD2) continue;
    block.setAttribute('data-rendered', '1');
    const src = block.querySelector('pre.diagram-src')?.textContent ?? '';
    const id = `${opts.idPrefix}-${++seq}`;
    const isStale = () => !block.isConnected;
    const run =
      kind === 'd2' && opts.renderD2
        ? opts.renderD2(id, src, { dark: opts.dark, isStale })
        : renderMermaid(id, src, { dark: opts.dark, isStale });
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
}
