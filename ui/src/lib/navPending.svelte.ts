// Cold page-switch feedback (perf F7). The router holds `parts` (and with it
// the highlighted sidebar item) until the next page's chunk is in — instant
// when it's prefetched, but on a phone / remote daemon (no idle prefetch) a
// tap could do nothing visible for a second or two. After PENDING_MS the
// tapped item turns `aria-busy`; after SLOW_MS a thin bar runs along the top
// of the content column. A warm switch commits well inside either delay, so
// it never flickers.

import { router } from './router.svelte';
import { activeNavId } from './sidebar';

/** Delay before the tapped sidebar item shows as pending. */
export const PENDING_MS = 120;
/** Delay before the content column's progress bar appears. */
export const SLOW_MS = 300;

class NavPending {
  /** Sidebar id (`data-nav-id`) of the route being loaded, once PENDING_MS passed. */
  id: string | null = $state(null);
  /** The load has taken longer than SLOW_MS. */
  slow = $state(false);
}

export const navPending = new NavPending();

if (typeof window !== 'undefined') {
  $effect.root(() => {
    $effect(() => {
      const target = router.pendingTarget;
      navPending.id = null;
      navPending.slow = false;
      if (!target) return;
      const id = activeNavId(target);
      const t1 = setTimeout(() => (navPending.id = id), PENDING_MS);
      const t2 = setTimeout(() => (navPending.slow = true), SLOW_MS);
      return () => {
        clearTimeout(t1);
        clearTimeout(t2);
      };
    });
  });
}
