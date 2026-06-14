// Bridges native Tauri menu-bar events to in-app actions. The Rust shell emits
// `otto://menu` with the menu item id; the SPA owns all behavior.

import { ui } from './stores/ui.svelte';
import { ws } from './stores/workspace.svelte';
import { router } from './router.svelte';

export function handleMenu(id: string): void {
  switch (id) {
    case 'settings':
      router.go('settings/appearance');
      break;
    case 'new-session':
      ui.newSessionOpen = true;
      break;
    case 'new-workspace':
      ui.newWorkspaceOpen = true;
      break;
    case 'close-tab':
      ws.closeActiveTab();
      break;
    case 'toggle-rail':
      ui.toggleRail();
      break;
    case 'toggle-panel':
      ui.toggleRight();
      break;
    case 'zoom-in':
      ui.zoomIn();
      break;
    case 'zoom-out':
      ui.zoomOut();
      break;
    case 'zoom-reset':
      ui.zoomReset();
      break;
    case 'session-restart':
      if (ws.activeSessionId) void ws.restartSession(ws.activeSessionId);
      break;
    case 'session-kill':
      if (ws.activeSessionId) void ws.killSession(ws.activeSessionId);
      break;
    case 'walkthroughs':
      router.go('walkthroughs');
      break;
  }
}

/** Subscribe to native menu events (Tauri only). Returns an unlisten fn. */
export async function attachMenuBridge(): Promise<() => void> {
  if (!('__TAURI_INTERNALS__' in window)) return () => {};
  try {
    const { listen } = await import('@tauri-apps/api/event');
    return await listen<string>('otto://menu', (e) => handleMenu(e.payload));
  } catch {
    return () => {};
  }
}

/**
 * Window-close behavior (Tauri).
 *
 * Sessions are owned by the daemon (ottod), which runs under launchd with
 * KeepAlive and persists independently of the app window — exactly like loom's
 * tmux model. Closing the Otto window must therefore just DETACH the viewer and
 * leave agent sessions RUNNING on the daemon, so they can be reattached when the
 * app reopens. We deliberately do NOT terminate sessions on close.
 *
 * (An explicit "Quit & stop all sessions" still exists via the /app/kill-sessions
 * endpoint, e.g. from a menu item, for when the user really wants a clean stop.)
 */
export async function attachCloseHandler(): Promise<() => void> {
  // No-op: let the window close normally; the daemon keeps sessions alive.
  return () => {};
}
