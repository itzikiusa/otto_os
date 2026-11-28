// On a phone a Git list pane IS the page: its header takes PageHeader's one
// row (design.md §7.1), so the page's own title — the repository switcher —
// is not drawn there. GitPage hands it over here and every Git list pane
// appends it to its ⋯.
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
  /** A Git list pane's ⋯ tail: the repositories on a phone, nothing elsewhere. */
  items(): MenuItem[] {
    const s = source;
    return viewport.navDrawer && s ? [{ separator: true }, ...s()] : [];
  },
};
