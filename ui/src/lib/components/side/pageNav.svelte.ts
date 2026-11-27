// What the page's header shows for its panes (design.md §3, §7): on a phone
// the Back to the list a ListDetail pushed from ("‹ Threads"); elsewhere the
// "Show <pane>" button while the user has hidden a list pane. Each mounted
// ListDetail registers one entry; PageHeader reads the innermost one that
// applies, so a list inside another list's content (Settings › Access
// groups) gets the Back first and the outer list's Back after it.
import { keyContext } from '../../keys';
export interface PaneNavEntry {
  /** Is this pane's page on screen (an inner ListDetail inside an outer one's hidden content is not)? */
  active(): boolean;
  /** Phone: the list this content was pushed from, or null while the list is the page. */
  back(): { label: string; onclick: () => void } | null;
  /** Desktop / tablet: the hidden pane's name and how to show it, or null. */
  show(): { label: string; onclick: () => void } | null;
  /** ⌘B: hide or show this page's list pane (null on a phone). */
  toggle(): (() => void) | null;
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
    this.entries = [...this.entries, entry];
    return () => {
      this.entries = this.entries.filter((e) => e !== entry);
    };
  }

  private first<T>(pick: (e: PaneNavEntry) => T | null): T | null {
    for (let i = this.entries.length - 1; i >= 0; i--) {
      const e = this.entries[i];
      if (!e.active()) continue;
      const v = pick(e);
      if (v) return v;
    }
    return null;
  }

  get back(): { label: string; onclick: () => void } | null {
    return this.first((e) => e.back());
  }
  get show(): { label: string; onclick: () => void } | null {
    return this.first((e) => e.show());
  }
  /** The innermost visible list pane's ⌘B, if any. */
  get toggle(): (() => void) | null {
    return this.first((e) => e.toggle());
  }
}

export const pageNav = new PageNav();
keyContext.listPaneToggle = () => pageNav.toggle;
keyContext.detailPaneToggle = () => pageNav.detailToggle;
