// What the page's header shows for its panes (design.md §3, §7): on a phone
// the Back to the list a ListDetail pushed from ("‹ Threads"); elsewhere the
// "Show <pane>" button while the user has hidden a list pane. Each mounted
// ListDetail registers one entry; PageHeader reads the innermost (deepest) one
// that applies, so a list inside another list's content (Settings › Access
// groups) gets the Back first and the outer list's Back after it. Depth, not
// registration order: Svelte runs a child's $effect before its parent's, so a
// nested ListDetail mounted with the page claims first.
import { untrack, type Snippet } from 'svelte';
import { keyContext } from '../../keys';
import type { MenuItem } from '../../contextmenu.svelte';
import type { PaneHeader } from './context';
/** A page whose content draws its own PageHeader (Settings' sections) hosts
 *  that header at its top, across the list pane and the content (design.md
 *  §7.16): the content's PageHeader hands its bar to the nearest HEADER_HOST
 *  while it is `active`, with the host's crumbs first and its ⋯ rows last. */
export const HEADER_HOST = Symbol('header-host');
export interface HeaderHost {
  readonly active: boolean;
  readonly crumbs: { label: string; onclick: () => void }[];
  readonly more?: () => MenuItem[];
  claim(bar: Snippet): () => void;
}
export interface PaneBack {
  label: string;
  onclick: () => void;
  title?: string;
}
export interface PaneNavEntry {
  /** ListDetail nesting: 0 = the page's list, 1 = one in its content, … */
  depth: number;
  /** Is this pane's page on screen (an inner ListDetail inside an outer one's hidden content is not)? */
  active(): boolean;
  /** Phone: the list this content was pushed from (and the open item's title,
   *  when the page header is shared), or null while the list is the page. */
  back(): PaneBack | null;
  /** Desktop / tablet: the hidden pane's name and how to show it, or null. */
  show(): { label: string; onclick: () => void } | null;
  /** ⌘B: hide or show this list pane (null on a phone); the page's own list (the shallowest) wins. */
  toggle(): (() => void) | null;
  /** Phone, the list is the page: the pane's header for the page header. */
  listPage(): PaneHeader | null;
}

/** Phone, the list is the page: its ⋯ holds the pane's menu, then the page's
 *  own `more` rows (page-level actions such as Switch context stay reachable).
 *  Here, not in context.ts: PageHeader on every page needs it, and this
 *  module is in the shell already, so it adds no chunk of its own per page. */
export function listPageMenu(pane?: () => MenuItem[], page?: () => MenuItem[]): (() => MenuItem[]) | undefined {
  if (!pane || !page) return pane ?? page;
  return () => {
    const a = pane();
    const b = page();
    return a.length && b.length ? [...a, { separator: true }, ...b] : [...a, ...b];
  };
}

class PageNav {
  private entries: PaneNavEntry[] = $state.raw([]);
  private details: (() => void)[] = [];

  /** A Detail pane that can be toggled (⌥⌘B); the last mounted wins. */
  claimDetail(toggle: () => void): () => void {
    this.details = [...this.details, toggle];
    return () => {
      this.details = this.details.filter((t) => t !== toggle);
    };
  }
  get detailToggle(): (() => void) | null {
    return this.details.at(-1) ?? null;
  }

  /** Register from an $effect; returns the release. */
  claim(entry: PaneNavEntry): () => void {
    // untrack: the claiming $effect must not depend on the list it writes.
    this.entries = [...untrack(() => this.entries), entry];
    return () => {
      this.entries = untrack(() => this.entries).filter((e) => e !== entry);
    };
  }

  /** The deepest active entry that has a value (the later one on a tie). */
  private first<T>(pick: (e: PaneNavEntry) => T | null): T | null {
    let best: T | null = null;
    let depth = -1;
    for (const e of this.entries) {
      if (e.depth < depth || !e.active()) continue;
      const v = pick(e);
      if (v) [best, depth] = [v, e.depth];
    }
    return best;
  }

  get back(): PaneBack | null {
    return this.first((e) => e.back());
  }
  get show(): { label: string; onclick: () => void } | null {
    return this.first((e) => e.show());
  }
  /** Phone: the list pane that is the page right now (its header goes in PageHeader). */
  get listPage(): PaneHeader | null {
    return this.first((e) => e.listPage());
  }
  /** ⌘B: the page's own list pane (the shallowest that has one), not a list
   *  nested in a tool window or a section (design.md §7: ⌘B is the list pane). */
  get toggle(): (() => void) | null {
    let best: (() => void) | null = null;
    let depth = Infinity;
    for (const e of this.entries) {
      if (e.depth >= depth || !e.active()) continue;
      const t = e.toggle();
      if (t) [best, depth] = [t, e.depth];
    }
    return best;
  }
}

export const pageNav = new PageNav();
keyContext.listPaneToggle = () => pageNav.toggle;
keyContext.detailPaneToggle = () => pageNav.detailToggle;
