// The side pane's header chrome (embedded document only — see lib/sidePane.ts).
//
// The side pane has no bar of its own: its controls (Swap / Open in main pane
// / Close) sit at the trailing end of the page's OWN top row — the PageHeader,
// or the Agents TabBar — like an Xcode editor's jump bar, so the two panes'
// toolbars line up and nothing says "Connections" twice. Every PageHeader /
// TabBar registers here; the first one in document order hosts the controls.
// The host also says whether this pane sits under the window's traffic
// lights (it is the leading pane and the sidebar is collapsed).

import { untrack } from 'svelte';
import { isEmbedded } from '../desktop';

class EmbedChrome {
  /** Pad the top row past the traffic lights (host-reported). */
  padTraffic = $state(false);
  private headers: HTMLElement[] = $state([]);

  /** A top-row candidate mounted (no-op outside the side pane). Callers
   *  claim from an $effect: the read of `headers` must not become that
   *  effect's dependency (read + write = an update loop). */
  claim(el: HTMLElement): () => void {
    if (!isEmbedded) return () => {};
    this.headers = [...untrack(() => this.headers), el];
    return () => {
      this.headers = untrack(() => this.headers).filter((h) => h !== el);
    };
  }

  /** The row that hosts the pane controls: the first registered one in
   *  document order (the page's top row, never a nested header). */
  get owner(): HTMLElement | null {
    let best: HTMLElement | null = null;
    for (const el of this.headers) {
      if (!el.isConnected) continue;
      if (!best || best.compareDocumentPosition(el) & Node.DOCUMENT_POSITION_PRECEDING) best = el;
    }
    return best;
  }
}

export const embedChrome = new EmbedChrome();
