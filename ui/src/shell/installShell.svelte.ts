// The shell's one-shot global installers (architecture-2-05), installed once
// from App.svelte's onMount instead of as loose `$effect`s in the app root —
// where one stray reactive read used to re-run boot code (c14b1d5e). Returns
// one teardown for all of them (sign-out unmounts the shell).

import { badgeWriter, tauriBadgeSink } from '../lib/dockBadge';
import { isEmbedded } from '../lib/desktop';
import { isTauri, ui } from '../lib/stores/ui.svelte';
import { auth } from '../lib/stores/auth.svelte';
import { availableModules } from '../lib/sidebar';
import { untrack } from 'svelte';
import { router } from '../lib/router.svelte';
import { lsGet, lsSet } from '../lib/storage';
import { LIGHT_HOME, lightMode } from '../lib/lightMode.svelte';
import { checkOpenTabs } from './openTabsCheck';
import { LS_WS_RECENT, noteRecent, parseRecent } from './workspaceSwitch';
import { ws } from '../lib/stores/workspace.svelte';
import { installPaletteSources, type PaletteDeps } from './paletteSources.svelte';
import { registerBrowserView } from '../lib/browserPort';
import { registerRuntimeThreadView } from '../lib/runtimeThreadPort';

// Hosts that embed the Browser view (Threads' inspector) load it through the port.
registerBrowserView(() => import('../modules/browser/BrowserView.svelte'));
// Hosts that embed one Threads conversation (a personal agent's chat).
registerRuntimeThreadView(() => import('../modules/threads/runtime/EmbeddedRuntimeThread.svelte'));

export function installShell(deps: PaletteDeps): () => void {
  const stopPalette = installPaletteSources(deps);
  const stopBadge = $effect.root(() => {
    // Dock badge = sessions waiting on you (A1). Once per window; the shell
    // command itself only honours the main window. Unmount (sign-out) → 0.
    if (!isTauri || isEmbedded) return;
    const badge = badgeWriter(tauriBadgeSink);
    $effect(() => badge.set(ws.needsYouCount));
    return () => badge.flush(0);
  });
  const stopKnown = $effect.root(() => {
    // A sidebar module counts as "known" only once this user can open it, so
    // one granted later is still placed and surfaced like a new one.
    $effect(() => {
      if (auth.phase !== 'ready' || !auth.me || auth.capabilitiesError) return;
      // Unscoped: the phone's light version must not make the other modules
      // look "new" (and re-surface ones the person hid) when it is switched off.
      const ids = availableModules((f) => auth.can(f, 'view'), [], { unscoped: true }).map((m) => m.id);
      untrack(() => ui.noteSidebarAvailable(ids));
    });
  });
  // The phone's light version lands on Threads (lib/lightMode.svelte.ts): the
  // bare route (opening the app, a home-screen launch) would be Agents.
  const stopLight = $effect.root(() => {
    $effect(() => {
      if (lightMode.active && router.parts.length === 0) untrack(() => router.replace(LIGHT_HOME));
    });
  });
  // Per workspace: the modules' resume points (router) and the switcher's
  // recently used list (workspaceSwitch.ts).
  const stopRecent = $effect.root(() => {
    $effect(() => {
      if (!ws.listSettled) return;
      const id = ws.currentId;
      untrack(() => {
        router.setResumeScope(id);
        if (id) lsSet(LS_WS_RECENT, JSON.stringify(noteRecent(parseRecent(lsGet(LS_WS_RECENT)), id)));
      });
    });
  });
  // Tabs whose session is gone close when the window comes back (tabSweep.ts).
  const onReturn = (): void => {
    if (document.visibilityState === 'visible') checkOpenTabs();
  };
  window.addEventListener('focus', onReturn);
  document.addEventListener('visibilitychange', onReturn);
  return () => {
    window.removeEventListener('focus', onReturn);
    document.removeEventListener('visibilitychange', onReturn);
    stopRecent();
    stopLight();
    stopKnown();
    stopBadge();
    stopPalette();
  };
}
