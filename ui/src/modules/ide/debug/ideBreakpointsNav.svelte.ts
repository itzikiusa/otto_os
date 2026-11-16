// "Breakpoints (n)" in the debugger's ⋯ opens THE breakpoint list — the
// project tree's Breakpoints tab (I2-11) — instead of a second list in a modal.
//
// `ideBreakpointsNav.seq` is the request (bumped on every open): the page
// shows a hidden explorer for it and the tree switches tab. The tab is also
// activated through its own button once it is on screen (it takes focus); on
// the stacked (phone) layout the Files pane is brought up first.

import { tick } from 'svelte';
import { idePreferences } from '../../../lib/idePreferences.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { ideKeymapPresetOf, ideShortcutLabel } from '../shell/ideShortcuts';
import { ideLayout } from '../shell/ideLayout.svelte';

class IdeBreakpointsNav {
  seq = $state(0);
}

export const ideBreakpointsNav = new IdeBreakpointsNav();

const TAB = '[data-testid="ide-tree-tab-breakpoints"]';

export async function openBreakpointsList(): Promise<boolean> {
  ideBreakpointsNav.seq += 1;
  if (ideLayout.stack) ideLayout.pane = 'files';
  await tick();
  // The page shows a hidden explorer for this request and the tree mounts lazily:
  // wait a few frames for its tab before giving up.
  let tab: HTMLElement | null = null;
  for (let i = 0; i < 30 && !tab; i += 1) {
    await new Promise<void>((r) => (typeof requestAnimationFrame === 'function' ? requestAnimationFrame(() => r()) : r()));
    tab = typeof document !== 'undefined' ? document.querySelector<HTMLElement>(TAB) : null;
    if (typeof requestAnimationFrame !== 'function') break;
  }
  if (tab) {
    tab.click();
    tab.focus();
    return true;
  }
  const key = ideShortcutLabel('toggleExplorer', ideKeymapPresetOf(idePreferences.current.keymapPreset));
  toasts.info('Breakpoints are in the project explorer', `Show the explorer${key ? ` (${key})` : ''}, then open its Breakpoints tab.`);
  return false;
}
