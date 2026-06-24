// Control for the native in-app browser — one Tauri child webview PER TAB,
// overlaid on the Browser panel. No-ops on the plain web build, where the panel
// falls back to a single <iframe>.

const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;

export const nativeBrowserAvailable = isTauri;

export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

async function call<T = void>(cmd: string, args?: Record<string, unknown>): Promise<T | undefined> {
  if (!isTauri) return undefined;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    return await invoke<T>(cmd, args);
  } catch {
    return undefined;
  }
}

export const nativeBrowser = {
  /** Create-or-navigate tab `id`'s webview and position it at `r`. */
  open: (id: string, url: string, r: Rect) => call('browser_open', { id, url, ...r }),
  /** Keep tab `id`'s webview aligned with the panel rect. */
  bounds: (id: string, r: Rect) => call('browser_bounds', { id, ...r }),
  navigate: (id: string, url: string) => call('browser_navigate', { id, url }),
  reload: (id: string) => call('browser_reload', { id }),
  show: (id: string) => call('browser_show', { id }),
  hide: (id: string) => call('browser_hide', { id }),
  /** Hide every tab's webview (overlay open / panel not visible). */
  hideAll: () => call('browser_hide_all'),
  close: (id: string) => call('browser_close', { id }),
  /** Destroy every tab's webview (panel unmount). */
  closeAll: () => call('browser_close_all'),
  /** Toggle the web inspector (console / network / elements) for tab `id`. */
  devtools: (id: string) => call('browser_devtools', { id }),
  /**
   * Subscribe to new-tab requests — fired when a page inside any tab does
   * `window.open()` / `target=_blank` (the native popup is suppressed). Returns
   * an unlisten fn. No-op off Tauri.
   */
  onNewTab: async (cb: (url: string) => void): Promise<() => void> => {
    if (!isTauri) return () => {};
    try {
      const { listen } = await import('@tauri-apps/api/event');
      return await listen<string>('otto://browser-new-tab', (e) => cb(e.payload));
    } catch {
      return () => {};
    }
  },
  /**
   * Subscribe to per-tab navigation — fired on every committed navigation in a
   * tab (payload `[tabId, url]`), so the address bar tracks in-page link clicks.
   * Replaces polling `url()` (which panics on a not-yet-loaded webview). Returns
   * an unlisten fn. No-op off Tauri.
   */
  onUrlChange: async (cb: (id: string, url: string) => void): Promise<() => void> => {
    if (!isTauri) return () => {};
    try {
      const { listen } = await import('@tauri-apps/api/event');
      return await listen<[string, string]>('otto://browser-url', (e) =>
        cb(e.payload[0], e.payload[1]),
      );
    } catch {
      return () => {};
    }
  },
};
