// Browser live-tab autofill & overlay-drain runners extracted from BrowserView
// so the main view stays below its LOC ratchet and cleanly separated.
import * as browserApi from '../../lib/api/browser';
import type { BrowserCredential, BrowserTab } from '../../lib/api/types';
import { confirmer } from '../../lib/confirm.svelte';
import { nativeBrowser, nativeBrowserAvailable } from '../../lib/nativeBrowser';
import { browser } from '../../lib/stores/browser.svelte';
import { toasts } from '../../lib/toast.svelte';
import { toastError } from '../../lib/toastError';
import { buildFillScript, fillResult, fillTargetOk } from './autofill';
import overlaySrc from './overlay.js?raw';

export async function injectLiveOverlay(id: string, restorePicking: boolean): Promise<void> {
  if (!nativeBrowserAvailable) return;
  await nativeBrowser.eval(id, overlaySrc);
  if (restorePicking) {
    void nativeBrowser.eval(id, 'window.__ottoOverlay && window.__ottoOverlay.setPicking(true)');
  }
}

export async function runLiveAutofill(
  cred: BrowserCredential,
  getActiveLive: () => BrowserTab | null,
): Promise<void> {
  const tab = getActiveLive();
  if (!tab || !cred) return;
  const ok = await confirmer.ask(
    `Fill the saved credentials for "${cred.username}" on ${cred.domain} into this page? Nothing is submitted automatically — review it before you sign in.`,
    { title: 'Autofill credentials', confirmLabel: 'Fill', danger: false },
  );
  if (!ok) return;
  try {
    // Reveal is Edit-gated and returns the plaintext password ONLY to this
    // call — never stored beyond this local scope or logged.
    const { password } = await browserApi.revealCredential(cred.id);
    // Re-check AFTER both awaits (S18-04): the tab may have redirected meanwhile.
    const now = getActiveLive();
    if (now?.id !== tab.id || !fillTargetOk(now.url, cred.domain)) {
      toasts.warn('Not filled', `This tab left ${cred.domain} — nothing was filled.`);
      return;
    }
    const js = buildFillScript(cred.domain, cred.username, password);
    const outcome = fillResult(await nativeBrowser.eval(tab.id, js));
    if (outcome === 'origin-changed') {
      toasts.warn('Not filled', `This tab left ${cred.domain} — nothing was filled.`);
    } else if (outcome === 'no-password-field') {
      toasts.warn('Nothing to fill', 'The page no longer has a password field.');
    } else {
      toasts.success('Autofilled', `${cred.username} · ${cred.domain} — review before submitting`);
    }
  } catch (e) {
    toastError('Couldn’t fill the login', e);
  }
}

export async function drainOverlayTick(id: string, url: string): Promise<void> {
  const highlights = browser.annotations
    .filter((a) => a.url === url)
    .map((a) => ({ selector: a.selector, color: a.color || 'yellow' }));
  const arg = JSON.stringify(JSON.stringify(highlights));
  const raw = await nativeBrowser.eval(id, `window.__ottoOverlay && window.__ottoOverlay.tick(${arg})`);
  if (!raw) return;
  let drained: Array<{ selector: string; outerHtml: string; text: string }>;
  try {
    const parsed = JSON.parse(raw);
    if (!Array.isArray(parsed)) return;
    drained = parsed;
  } catch {
    return;
  }
  for (const m of drained) {
    try {
      await browser.createAnnotation({
        url,
        selector: m.selector,
        excerpt: (m.outerHtml || '').slice(0, 2000),
        text: (m.text || '').slice(0, 2000),
        comment: '',
      });
    } catch (e) {
      toastError('Couldn’t save mark', e);
    }
  }
}
