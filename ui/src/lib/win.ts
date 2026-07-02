// Per-window identity. Secondary Tauri windows get `window.__OTTO_WIN__ = '<label>'`
// injected via initialization_script; browser/E2E contexts can pass `?win=<id>`.
// The main window (and plain web) resolves to 'main' and keeps the legacy
// unprefixed localStorage keys — zero migration.
const fromQuery =
  typeof window !== 'undefined' ? new URLSearchParams(window.location.search).get('win') : null;
const fromTauri =
  typeof window !== 'undefined' ? (window as { __OTTO_WIN__?: string }).__OTTO_WIN__ : undefined;
export const windowId: string = fromQuery || fromTauri || 'main';

/** Namespace a localStorage key by window: pass-through for 'main'. */
export function winKey(key: string): string {
  return windowId === 'main' ? key : `otto_win_${windowId}::${key}`;
}

/** Prune localStorage groups of windows no longer in the shell's registry.
 *  Main-window + Tauri only; a browser context has no registry → no-op. */
export async function gcWindowKeys(): Promise<void> {
  if (windowId !== 'main' || !('__TAURI_INTERNALS__' in window)) return;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    const live = new Set(await invoke<string[]>('windows_registry'));
    for (const k of Object.keys(localStorage)) {
      const m = /^otto_win_([^:]+)::/.exec(k);
      if (m && !live.has(m[1])) localStorage.removeItem(k);
    }
  } catch {
    /* registry unavailable — never block boot on GC */
  }
}
