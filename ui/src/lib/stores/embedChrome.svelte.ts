// The side pane's header chrome (embedded document only — see lib/sidePane.ts).
//
// The side pane has no bar of its own: its controls (Swap / Open in main pane
// / Close) sit at the trailing end of the page's OWN top row — the PageHeader,
// or the Agents TabBar — like an Xcode editor's jump bar, so the two panes'
// toolbars line up and nothing says "Connections" twice. Every PageHeader /
// TabBar registers here; the first one in document order hosts the controls.
// The host also says whether this pane sits under the window's traffic
// lights (it is the leading pane and the sidebar is collapsed).

import { isEmbedded, isTauri } from '../desktop';

class EmbedChrome {
  /** Pad the top row past the traffic lights (host-reported). */
  padTraffic = $state(false);
  // Authoritative list is plain; `headers` only publishes it. Callers claim
  // from an $effect and release from its teardown, where Svelte answers a
  // `$state` read with the PRE-flush value — so rebuilding from `headers`
  // there dropped a header claimed in the same flush (a page swap). Never
  // read `headers` from claim/release (commands.svelte.ts has the repro).
  private list: HTMLElement[] = [];
  private headers: HTMLElement[] = $state.raw([]);

  /** A top-row candidate mounted (no-op outside the side pane). */
  claim(el: HTMLElement): () => void {
    if (!isEmbedded && !isTauri) return () => {};
    this.list = [...this.list, el];
    this.headers = this.list;
    return () => {
      this.list = this.list.filter((h) => h !== el);
      this.headers = this.list;
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
