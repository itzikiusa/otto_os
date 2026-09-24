// "Open in New Window" rows for ctxMenu (sessions, DB tabs, Design Hall
// artifacts). Desktop app only: outside the Tauri shell these return no rows,
// so web / phone menus never offer a window they can't open.

import type { MenuItem } from './contextmenu.svelte';
import { isTauri, openPopout } from './desktop';
import { toasts } from './toast.svelte';

/** `route` without the leading `#/` (e.g. `agents/<id>`, `database/<id>`). */
export function popoutItems(route: string, title?: string): MenuItem[] {
  if (!isTauri) return [];
  return [
    {
      label: 'Open in New Window',
      icon: 'external',
      action: () =>
        void openPopout(route, title).catch((e: unknown) =>
          toasts.error('Could not open window', e instanceof Error ? e.message : String(e)),
        ),
    },
  ];
}

/** True when pop-out windows exist (callers that build their own menu). */
export const canPopout = isTauri;
