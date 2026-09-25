// The side pane's half of the side-by-side protocol (lib/sidePane.ts): runs
// only in the embedded document (`?embed=1`, an iframe in the main window).
// It tells the host where this pane is, hands the host the navigations and
// keys that belong to the window, and takes the host's navigate / workspace /
// menu / command requests. The host's half is stores/sidePane.svelte.ts.

import { isEmbedded } from './desktop';
import { router } from './router.svelte';
import { embedChrome } from './stores/embedChrome.svelte';
import {
  SIDE_NS,
  paneKey,
  readHostMsg,
  targetOrigin,
  type GuestMsg,
  type HostMsg,
} from './sidePane';

type Payload<T> = T extends unknown ? Omit<T, 'ns'> : never;

/** Post to the host window (a no-op outside the side pane). */
export function postToHost(msg: Payload<GuestMsg>): void {
  if (!isEmbedded) return;
  try {
    window.parent.postMessage({ ns: SIDE_NS, ...msg }, targetOrigin(window.location.origin));
  } catch {
    /* host gone (window closing) */
  }
}

/** The host's primary module key, as last reported (fallback for reads of
 *  the host's location failing). */
let hostPrimary: string | null = null;

/** Which module the MAIN pane shows right now — read straight from the host's
 *  location (same origin, always current), else its last report. */
export function hostPrimaryKey(): string | null {
  try {
    return paneKey(window.parent.location.hash);
  } catch {
    return hostPrimary;
  }
}

export interface GuestHooks {
  /** A native menu item the host forwarded because this pane has focus. */
  runMenu: (id: string) => void;
  /** The host switched workspace. */
  selectWorkspace: (id: string) => void;
  /** Run one of this pane's ⌘K commands (the host mirrors them). */
  runCommand: (id: string) => void;
}

/**
 * Wire the side pane up: a router delegate that hands the main pane's module
 * back to the host (so it never opens twice), the host message listener and a
 * focus reporter (the host draws the focused pane's bar as active). Returns
 * the teardown.
 */
export function startGuest(hooks: GuestHooks): () => void {
  if (!isEmbedded) return () => {};
  router.setDelegate({
    claims: (route) => {
      const primary = hostPrimaryKey();
      return primary !== null && paneKey(route) === primary;
    },
    deliver: (route) => postToHost({ type: 'open-in-main', route }),
  });

  const onMessage = (e: MessageEvent): void => {
    if (e.source !== window.parent) return;
    const msg: HostMsg | null = readHostMsg(e.data);
    if (!msg) return;
    switch (msg.type) {
      case 'navigate':
        router.go(msg.route);
        break;
      case 'host':
        hostPrimary = msg.primary;
        embedChrome.padTraffic = msg.padTraffic;
        break;
      case 'workspace':
        hooks.selectWorkspace(msg.id);
        break;
      case 'run-command':
        hooks.runCommand(msg.id);
        break;
      case 'menu':
        hooks.runMenu(msg.id);
        break;
    }
  };
  const onFocus = (): void => postToHost({ type: 'focus' });
  window.addEventListener('message', onMessage);
  window.addEventListener('focus', onFocus);
  // A click on a non-focusable surface still moves focus into this document;
  // report it on pointerdown too so the active-pane bar follows the click.
  window.addEventListener('pointerdown', onFocus, true);
  return () => {
    router.setDelegate(null);
    window.removeEventListener('message', onMessage);
    window.removeEventListener('focus', onFocus);
    window.removeEventListener('pointerdown', onFocus, true);
  };
}
