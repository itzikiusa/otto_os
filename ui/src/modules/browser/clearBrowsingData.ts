// "Clear browsing data" for the desktop app's in-app browser (Tauri
// `browser_clear_data`). The tabs' web views keep their own WebKit data store
// (macOS 14+), so this never touches Otto's own window; the command refuses on
// older macOS, where there is no separate store.

/** What the confirm says is cleared, and for whom. */
export const CLEAR_DATA_TEXT =
  'This deletes the cookies, cache and site storage (local storage, IndexedDB, service workers) of every site opened in Otto’s browser on this Mac — the live tabs of every workspace share them. You will be signed out of those sites. Otto’s own sign-in and settings are not affected, nor is Otto’s Chromium renderer.';

/** Run the desktop command; rejects with its message (e.g. macOS before 14). */
export async function clearBrowsingData(): Promise<void> {
  const { invoke } = await import('@tauri-apps/api/core');
  await invoke('browser_clear_data');
}
