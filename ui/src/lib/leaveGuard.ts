// The one "Discard unsaved changes?" leave-guard for editors (router.guard).
//
// Usage — in a component, hand the unregister function back as the effect's
// cleanup so the guard lives exactly as long as the editor:
//
//   $effect(() => guardUnsaved(() => dirty));
//
// Friction fits the risk: nothing is asked when the editor is clean, and the
// confirm names the loss ("Discard") with "Keep editing" as the safe default.

import { router } from './router.svelte';
import { confirmer } from './confirm.svelte';

export interface GuardUnsavedOpts {
  /** What is unsaved, for the message ("the brand kit", "this skill"). */
  what?: string;
  /** Routes (without `#/`) this editor may move to WITHOUT asking — e.g. its
   *  own sub-routes, when they keep the draft mounted. */
  allow?: (to: string) => boolean;
}

/** Ask "Discard unsaved changes?" before leaving while `isDirty()` holds.
 *  Returns the unregister function. */
export function guardUnsaved(isDirty: () => boolean, opts: GuardUnsavedOpts = {}): () => void {
  return router.guard(async (to) => {
    if (!isDirty()) return true;
    if (opts.allow?.(to)) return true;
    return confirmer.ask(
      opts.what
        ? `You have unsaved changes to ${opts.what}. Leaving now discards them.`
        : 'You have unsaved changes. Leaving now discards them.',
      { title: 'Discard unsaved changes?', confirmLabel: 'Discard', cancelLabel: 'Keep editing' },
    );
  });
}
