// On a phone a Git list pane IS the page: its header takes PageHeader's one
// row (design.md §7.1), so the page's own title (the repository switcher) and
// its view switch (History · Pull requests · Review) are not drawn there.
// GitPage hands them over here and every Git list pane appends them to its ⋯.
import type { MenuItem } from '../../lib/contextmenu.svelte';
import { viewport } from '../../lib/stores/viewport.svelte';

let source = $state.raw<(() => MenuItem[]) | null>(null);

export const gitPhoneMenu = {
  /** GitPage: what a phone list page's ⋯ adds (returns the release). */
  set(fn: () => MenuItem[]): () => void {
    source = fn;
    return () => {
      if (source === fn) source = null;
    };
  },
  /** A Git list pane's ⋯ tail: the views and repositories on a phone, nothing elsewhere. */
  items(): MenuItem[] {
    const s = source;
    return viewport.navDrawer && s ? [{ separator: true }, ...s()] : [];
  },
};
