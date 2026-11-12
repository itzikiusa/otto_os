// "Ask about this page" on a LIVE tab: the agent can't fetch a logged-in page
// itself, so the ask carries what the tab shows. Read from the desktop app's
// own webview (`browser_eval`); the daemon caps it again and fences it as
// untrusted page content (`build_ask_block`).

import { nativeBrowser, nativeBrowserAvailable } from '../../lib/nativeBrowser';
import { browserLive } from '../../lib/stores/browserLive.svelte';
import type { BrowserTab } from '../../lib/api/types';

/** Same cap as the daemon's `SUMMARIZE_MAX_CHARS`. */
export const LIVE_TEXT_MAX = 30_000;

/** The page's visible text, capped in the page so a huge DOM never crosses IPC whole. */
export const LIVE_TEXT_JS = `(function(){var b=document.body;return b?String(b.innerText||'').slice(0,${LIVE_TEXT_MAX}):'';})()`;

/** `browser_eval` resolves with the value raw or JSON-encoded — take either. */
export function decodeEvalText(raw: unknown): string {
  const s = typeof raw === 'string' ? raw : '';
  try {
    const v: unknown = JSON.parse(s);
    if (typeof v === 'string') return v;
  } catch {
    /* not JSON: the raw text itself */
  }
  return s;
}

/** The active live tab's visible text in the desktop webview, or undefined
 *  (a reader tab, the streamed renderer, off the desktop app, or a page that
 *  didn't answer). */
export async function livePageText(tab: BrowserTab | null | undefined): Promise<string | undefined> {
  if (!tab || tab.mode !== 'live' || !nativeBrowserAvailable || browserLive.renderer !== 'native') return undefined;
  const text = decodeEvalText(await nativeBrowser.eval(tab.id, LIVE_TEXT_JS)).trim();
  return text ? text.slice(0, LIVE_TEXT_MAX) : undefined;
}
