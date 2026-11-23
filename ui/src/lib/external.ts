// Opening external URLs in the system browser.
//
// In a Tauri WKWebView a plain `<a target="_blank">` does NOT open the OS
// browser — the webview tries to open a (blocked) in-app window, so the click
// appears to do nothing. We route external links through the opener plugin's
// `open_url` command instead. On the plain web build we fall back to window.open.

import { isEmbedded } from './desktop';
import { postToHost } from './embedGuest';

// eslint-disable-next-line no-control-regex
const CTRL_RE = /[\u0000-\u001f\u007f]/;

/** Return trimmed `http(s)://` URL if safe, else `undefined`. Rejects control
 *  characters and non-HTTP(S) schemes (`javascript:`, `file:`, `smb:`, …). */
export function safeHttpUrl(raw: string | null | undefined): string | undefined {
  const href = raw?.trim();
  if (!href || CTRL_RE.test(href)) return undefined;
  return /^https?:\/\//i.test(href) ? href : undefined;
}

/** Return trimmed URL if allowed for `openExternal` (`http:`, `https:`, or
 *  `mailto:`, with no control chars), else `undefined`. */
export function safeExternalUrl(raw: string | null | undefined): string | undefined {
  const href = raw?.trim();
  if (!href || CTRL_RE.test(href)) return undefined;
  return /^(https?:\/\/|mailto:)/i.test(href) ? href : undefined;
}

/** True for absolute http(s) URLs that should be handed to the OS browser. */
export function isExternalUrl(href: string | null | undefined): boolean {
  return safeHttpUrl(href) !== undefined;
}

/** Open `url` in the system browser (Tauri: opener `open_url`; web: window.open).
 *  Refuses any URL outside the `http(s):` / `mailto:` allowlist. */
export async function openExternal(url: string | null | undefined): Promise<void> {
  const href = safeExternalUrl(url);
  if (!href) return;

  // The side-by-side pane is an iframe without the Tauri bridge (it is
  // injected into the main frame only): the host window opens the link.
  if (isEmbedded && isExternalUrl(href)) {
    postToHost({ type: 'open-external', url: href });
    return;
  }

  if ('__TAURI_INTERNALS__' in window) {
    const { invoke } = await import('@tauri-apps/api/core');
    try {
      await invoke('plugin:opener|open_url', { url: href });
      return;
    } catch {
      /* fall through to window.open */
    }
  }

  // Not a synthetic `<a target=_blank>` click: App.svelte's capture-phase
  // link handler routes such clicks back here, and WebKit recursed until the
  // stack overflowed (the web build's every external link).
  window.open(href, '_blank', 'noopener,noreferrer');
}
