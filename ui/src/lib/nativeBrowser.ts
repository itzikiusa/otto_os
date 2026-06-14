// Control for the native in-app browser (a Tauri child webview that overlays
// the Browser panel). No-ops on the plain web build, where the panel falls back
// to an <iframe>.

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
  /** Create-or-navigate the browser webview and position it at `r`. */
  open: (url: string, r: Rect) => call('browser_open', { url, ...r }),
  /** Keep the webview aligned with the panel rect. */
  bounds: (r: Rect) => call('browser_bounds', { ...r }),
  navigate: (url: string) => call('browser_navigate', { url }),
  reload: () => call('browser_reload'),
  show: () => call('browser_show'),
  hide: () => call('browser_hide'),
  close: () => call('browser_close'),
  currentUrl: () => call<string | null>('browser_current_url'),
};
