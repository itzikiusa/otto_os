// One sidebar, like T3 (patterns.md §10): while a page with its own list
// column is open (Threads, Agents) Otto's navigator folds to its icon rail —
// not persisted, the shell's own preference is untouched — and comes back on
// leaving. Expanding the rail there is remembered as "don't fold" (`pref`),
// folding it again as "fold". A phone has no rail: nothing happens.
// Call from `onMount`; returns the cleanup.
import { untrack } from 'svelte';
import { ui } from './stores/ui.svelte';
import { viewport } from './stores/viewport.svelte';

export interface FoldPref {
  /** Fold the navigator while the page is open. */
  get(): boolean;
  set(on: boolean): void;
}

export function foldNavWhileMounted(pref: FoldPref): () => void {
  if (viewport.isPhone) return () => {};
  let folded = false;
  if (pref.get() && ui.railExpanded) {
    ui.railExpanded = false;
    folded = true;
  }
  const stop = $effect.root(() => {
    $effect(() => {
      const expanded = ui.railExpanded;
      untrack(() => {
        if (expanded && folded) {
          folded = false;
          pref.set(false);
        } else if (!expanded && !folded && !pref.get()) {
          pref.set(true);
        }
      });
    });
  });
  return () => {
    stop();
    if (folded && !ui.railExpanded) ui.railExpanded = true;
  };
}
