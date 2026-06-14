// Opening external URLs in the system browser.
//
// In a Tauri WKWebView a plain `<a target="_blank">` does NOT open the OS
// browser — the webview tries to open a (blocked) in-app window, so the click
// appears to do nothing. We route external links through the shell plugin's
// `open` command instead. On the plain web build we fall back to an anchor.

/** True for absolute http(s) URLs that should be handed to the OS browser. */
export function isExternalUrl(href: string | null | undefined): boolean {
  return !!href && /^https?:\/\//i.test(href.trim());
}

/** Open `url` in the system browser (Tauri: shell `open`; web: anchor). */
export async function openExternal(url: string | null | undefined): Promise<void> {
  const href = url?.trim();
  if (!href) return;

  if ('__TAURI_INTERNALS__' in window) {
    const { invoke } = await import('@tauri-apps/api/core');
    // Prefer the dedicated opener plugin (the Tauri 2 way to open a URL in the
    // OS browser); fall back to the shell plugin's `open`, then to an anchor.
    try {
      await invoke('plugin:opener|open_url', { url: href });
      return;
    } catch {
      /* try shell next */
    }
    try {
      await invoke('plugin:shell|open', { path: href });
      return;
    } catch {
      /* fall through to the anchor approach */
    }
  }

  const a = document.createElement('a');
  a.href = href;
  a.target = '_blank';
  a.rel = 'noopener noreferrer';
  document.body.appendChild(a);
  a.click();
  a.remove();
}
