// Editor chrome in the stacked (phone / narrow) IDE layout.
// - The sub-bar row (breadcrumb + tool icons) is hidden there by default so the code gets
//   that row back; the tab strip's ▾ menu shows it again ("Show editor toolbar"). A
//   notice the person must see (a refused save, the room or language server reconnecting)
//   still shows the row.
// - CodeMirror's compact mode (soft wrap, a narrow breakpoint lane) follows the layout.

import { setIdeCompactLayout } from '../../../lib/components/cm-ide-compact';
import { ideLayout } from '../shell/ideLayout.svelte';

class IdeEditorChrome {
  /** The person asked for the sub-bar in the stacked layout. */
  subbarShown = $state(false);
}

export const ideEditorChrome = new IdeEditorChrome();

let following = false;

/** Keep CodeMirror's compact mode in step with the IDE layout (idempotent). */
export function followIdeCompactLayout(): void {
  if (following) return;
  following = true;
  $effect.root(() => {
    $effect(() => {
      setIdeCompactLayout(ideLayout.stack);
    });
  });
}
