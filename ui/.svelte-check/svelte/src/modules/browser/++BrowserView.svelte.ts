///<reference types="svelte" />
;
import { announceShown } from '../../lib/announce.svelte';
// Browser module page: tab strip + URL bar + reader-mode page. Mirrors the
// shape of VaultPage/LoopsPage (a thin view over a $state store).
import { untrack } from 'svelte';
import { toastError } from '../../lib/toastError';
import { loadErrorText } from '../../lib/loadError';
import { pollWhileVisible } from '../../lib/poll';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { browser } from '../../lib/stores/browser.svelte';
import { vault } from '../vault/vault.svelte';
import { toasts } from '../../lib/toast.svelte';
import { isAbortError } from '../../lib/api/client';
import { confirmer } from '../../lib/confirm.svelte';
import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
import { registry } from '../../lib/commands.svelte';
import { nativeBrowser, nativeBrowserAvailable, type Rect } from '../../lib/nativeBrowser';
import * as browserApi from '../../lib/api/browser';
import type { BrowserCredential } from '../../lib/api/types';
import Icon from '../../lib/components/Icon.svelte';
import TabStrip from './TabStrip.svelte';
import Modal from '../../lib/components/Modal.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import ReaderView from './ReaderView.svelte';
import NotesRail from './NotesRail.svelte';
import AgentDock from './AgentDock.svelte';
import AskBar from './AskBar.svelte';
import RemoteLiveView from './live/RemoteLiveView.svelte';
import type { LiveViewState } from './live/protocol';
import LiveEngineSetup from './live/LiveEngineSetup.svelte';
import { browserLive } from '../../lib/stores/browserLive.svelte';
import * as liveApi from '../../lib/api/browserLive';
// Raw source text of the picker overlay — `eval`'d into the live tab's own
// JS context via `browser_eval`, never bundled/imported as a module (see
// overlay.js's header for why).
import overlaySrc from './overlay.js?raw';
import { buildFillScript, fillAllowedForUrl, fillResult, fillTargetOk, hostOf, matchDomain } from './autofill';
;

  interface Props {
    /** Hosted inside the agent-mode right panel (the "v2" Browser tab): no
     *  embedded agent dock — the session is the main pane — just the ask bar
     *  targeting `targetSessionId`. */
    embedded?: boolean;
    /** The session the embedded ask bar submits to (the active agent session). */
    targetSessionId?: string | null;
  };function $$render() {

  
  
  

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  let showCredentials = $state(false);
  let credentialsPanel = $state<Promise<typeof import('./CredentialsPanel.svelte')> | null>(null);
  function openCredentials(): void {
    credentialsPanel = import('./CredentialsPanel.svelte');
    showCredentials = true;
  }
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { embedded = false, targetSessionId = null }: Props = $props();

  let urlInput = $state('');
  let vaultSaving = $state(false);
  let urlFocused = $state(false);

  // (Re)load the tab list when the workspace changes.
  $effect(() => {
    const id = ws.currentId;
    if (id) void browser.loadTabs(id);
  });

  // Keep the URL bar in sync with the active tab — unless the field is
  // focused (typing a new URL) or a live tab's in-page navigation is
  // updating it, handled by the onUrlChange listener below instead.
  $effect(() => {
    if (!urlFocused) urlInput = browser.activeTab?.url ?? urlInput;
  });

  // ── Live tab: two renderers (browserLive.renderer) ─────────────────────
  //   native — the desktop app's Tauri child webview overlaid on `liveHostEl`
  //            (only inside Otto.app; the default there);
  //   remote — the daemon-owned Chromium streamed into RemoteLiveView (any
  //            device: remote web session, PWA, phone — and agent-drivable).
  // Only a daemon that predates live streaming leaves a remote live tab on
  // the old honest fallback (Reader + "Open in new tab").
  let liveHostEl = $state<HTMLDivElement | null>(null);
  const activeLive = $derived(
    nativeBrowserAvailable && browserLive.renderer === 'native' && browser.activeTab?.mode === 'live'
      ? browser.activeTab
      : null,
  );
  const activeRemote = $derived(
    browserLive.renderer === 'remote' && browserLive.supported !== false && browser.activeTab?.mode === 'live'
      ? browser.activeTab
      : null,
  );

  // Learn whether the daemon engine exists (and is installed) once a live
  // tab needs it. Never installs anything — that's an explicit click.
  $effect(() => {
    if (!activeRemote) return;
    if (browserLive.status || browserLive.loading || browserLive.supported === false) return;
    void browserLive.load();
  });

  // The daemon turned out to predate live streaming: the active live tab
  // needs its reader page after all (the store skipped it while unknown).
  $effect(() => {
    const tab = browser.activeTab;
    if (browserLive.supported !== false || !tab || tab.mode !== 'live' || activeLive) return;
    if (!browser.page && !browser.loadingPage && !browser.pageError) void browser.loadPage(tab.url);
  });

  // ── Remote live view state (nav buttons, ⌘L) ───────────────────────────
  // Element picking has no remote counterpart in the live protocol yet, so
  // the target button is simply absent for a remote tab (no dead control).
  let remoteView = $state<ReturnType<typeof RemoteLiveView> | null>(null);
  let remoteState: LiveViewState | null = $state(null);
  let urlEl = $state<HTMLInputElement | null>(null);
  // A primitive, so a nav that replaces the tab object (trackLiveNav) doesn't
  // look like a tab switch.
  const activeRemoteId = $derived(activeRemote?.id ?? null);
  $effect(() => {
    // The reported state belongs to one tab's session.
    const _id = activeRemoteId;
    remoteState = null;
  });

  function onRemoteNav(url: string, title: string): void {
    const tab = activeRemote;
    if (!tab) return;
    browser.trackLiveNav(tab.id, url, title);
    if (!urlFocused) urlInput = url;
  }

  function focusUrl(): void {
    urlEl?.focus();
    urlEl?.select();
  }

  function chooseRenderer(e: MouseEvent): void {
    const cur = browserLive.renderer;
    ctxMenu.show(e, [
      {
        label: "This Mac’s web view",
        icon: cur === 'native' ? 'check' : 'globe',
        action: () => browserLive.setPref('native'),
      },
      {
        label: "Otto’s Chromium (streams anywhere, agents can drive it)",
        icon: cur === 'remote' ? 'check' : 'compass',
        action: () => browserLive.setPref('remote'),
      },
    ]);
  }
  // Per-tab last-navigated URL, so the driver effect only calls `open` (which
  // re-navigates) when the URL actually changed — switching tabs just
  // shows/hides, preserving each tab's scroll/form state.
  const openedUrl: Record<string, string> = {};

  function hostRect(): Rect | null {
    if (!liveHostEl) return null;
    const r = liveHostEl.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return null;
    // The native WKWebView page-zoom magnifies the whole SPA from the
    // window's top-left without reflowing, so getBoundingClientRect() (CSS
    // px) must be scaled by the zoom factor to land in window-logical points,
    // which is what the child webview is positioned in.
    const z = ui.zoom || 1;
    return { x: r.left * z, y: r.top * z, width: r.width * z, height: r.height * z };
  }

  // Show the active live tab's webview over the host rect; hide every other
  // live tab this view has opened. A native webview always paints above the
  // HTML, so it must also hide when an SPA overlay is open.
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    const tabs = browser.tabs; // reactive dep
    const active = activeLive; // reactive dep
    const overlay = ui.overlayOpen || ctxMenu.open;
    for (const t of tabs) {
      if (t.mode === 'live' && (!active || t.id !== active.id || overlay)) {
        void nativeBrowser.hide(t.id);
      }
    }
    if (!active || overlay) return;
    const r = hostRect();
    if (!r) return;
    if (openedUrl[active.id] !== active.url) {
      openedUrl[active.id] = active.url;
      // create-or-navigate + show, then arm the picker overlay on the fresh
      // page (a same-tab URL change reloads the DOM, dropping any overlay a
      // previous injection installed).
      void nativeBrowser.open(active.id, active.url, r).then(() => injectOverlay(active.id));
    } else {
      void nativeBrowser.bounds(active.id, r);
      void nativeBrowser.show(active.id);
    }
  });

  // Keep the active live tab's webview aligned with the pane as it resizes.
  $effect(() => {
    if (!nativeBrowserAvailable || !activeLive || !liveHostEl) return;
    const id = activeLive.id;
    const _z = ui.zoom; // re-align immediately when the page zoom changes
    const sync = (): void => {
      const r = hostRect();
      if (r) void nativeBrowser.bounds(id, r);
    };
    const ro = new ResizeObserver(sync);
    ro.observe(liveHostEl);
    window.addEventListener('resize', sync);
    sync();
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', sync);
    };
  });

  // ── Live tab: element picker overlay ────────────────────────────────────
  // The overlay can't call back into the app over IPC (child webviews are
  // denied it — see Task 9's report), so the app polls it instead: every tick
  // it pushes the current URL's marks in (for re-highlighting) and pulls out
  // whatever the overlay queued since the last poll (marks made by clicking
  // in pick mode). See overlay.js's header for the full protocol.
  let pickMode = $state(false);

  async function injectOverlay(id: string): Promise<void> {
    if (!nativeBrowserAvailable) return;
    await nativeBrowser.eval(id, overlaySrc);
    // Picking was already armed before this (re-)injection (e.g. an in-page
    // nav reloaded the DOM mid-pick) — restore it on the fresh page.
    if (pickMode && id === activeLive?.id) {
      void nativeBrowser.eval(id, 'window.__ottoOverlay && window.__ottoOverlay.setPicking(true)');
    }
  }

  function togglePick(): void {
    const active = activeLive;
    if (!active) return;
    pickMode = !pickMode;
    void nativeBrowser.eval(
      active.id,
      `window.__ottoOverlay && window.__ottoOverlay.setPicking(${pickMode})`,
    );
  }

  // Pick mode is per-tab-load, not per-tab — reset it when the active LIVE
  // TAB actually changes (not on every `activeLive` object identity change,
  // which happens on every annotation-list update too and would otherwise
  // flip picking back off right after a mark round-trips).
  let lastLiveId: string | null = null;
  $effect(() => {
    const id = activeLive?.id ?? null;
    if (id !== lastLiveId) {
      lastLiveId = id;
      pickMode = false;
    }
  });

  // Backpressure: a `browser_eval` round-trip can outlive a single 700ms
  // tick (a slow/busy page, or `createAnnotation` calls piling up for a
  // multi-mark drain) — skip starting a new poll while one's still in
  // flight rather than letting them queue up and land out of order.
  let pollInFlight = false;

  async function pollOverlay(id: string, url: string): Promise<void> {
    if (pollInFlight) return;
    pollInFlight = true;
    try {
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
    } finally {
      pollInFlight = false;
    }
  }

  // Poll while a live tab is active — even outside pick mode, so marks made
  // just before switching tabs still drain, and existing marks stay
  // highlighted across scroll/DOM changes.
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    const active = activeLive;
    if (!active) return;
    const id = active.id;
    const url = active.url;
    // Shared poll chain: never overlaps a stalled `browser_eval`, and pauses
    // while the Otto window is hidden (SB-14) — one catch-up tick on return.
    const p = pollWhileVisible(() => untrack(() => pollOverlay(id, url)), { ms: 700, floorMs: 500, maxBackoff: 4 });
    return () => p.stop();
  });

  // ── Live tab: credential autofill (user-triggered, key icon) ───────────
  // Distinct from `browser_login` (the governed AGENT tool, `crates/otto-
  // server/src/routes/browser.rs`'s `/browser/login` route) — that drives an
  // off-screen CDP session server-side and never touches a visible tab. This
  // fills the credential straight into the tab the user is looking at, via
  // `browser_eval`, and NEVER submits the form — the user reviews and
  // submits it themselves. The key icon only appears when ALL of: a live
  // tab is active, its host matches a stored credential's domain, the
  // CURRENT page actually has a password field right now (not just "this
  // domain has one somewhere"), and the page isn't plain `http:` on a
  // non-loopback host (credentials over unencrypted transport is the one
  // case this refuses outright, loopback dev servers excepted).
  let credentials: BrowserCredential[] = $state([]);
  $effect(() => {
    const id = ws.currentId;
    if (!id) return;
    void browserApi
      .listCredentials(id)
      .then((c) => (credentials = c))
      .catch(() => {
        // No Editor role, or the call failed — the key icon simply never
        // appears; autofill is a convenience, not something to surface an
        // error toast for on every workspace switch.
        credentials = [];
      });
  });

  const matchedCredential = $derived.by(() => {
    const tab = activeLive;
    if (!tab) return null;
    const host = hostOf(tab.url);
    if (!host || !fillAllowedForUrl(tab.url)) return null;
    return credentials.find((c) => matchDomain(host, c.domain)) ?? null;
  });

  // Whether the CURRENT page (not just the domain) actually has a password
  // field right now — polled only while a domain match exists, so a normal
  // browsing session with no matching credential never pays for the extra
  // `browser_eval` round-trip.
  let hasLoginForm = $state(false);
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    const cred = matchedCredential;
    const active = activeLive;
    if (!cred || !active) {
      hasLoginForm = false;
      return;
    }
    const id = active.id;
    let stopped = false;
    const check = async (): Promise<void> => {
      const raw = await nativeBrowser.eval(
        id,
        "window.__ottoOverlay && window.__ottoOverlay.hasLoginForm ? window.__ottoOverlay.hasLoginForm() : false",
      );
      if (!stopped) hasLoginForm = raw === 'true';
    };
    // In-flight guard + hidden pause (SB-14): a stalled `browser_eval` used to
    // stack one more call every second.
    const p = pollWhileVisible(() => untrack(() => check()), { ms: 1000, maxBackoff: 4 });
    return () => {
      stopped = true;
      p.stop();
    };
  });

  const canAutofill = $derived(!!activeLive && !!matchedCredential && hasLoginForm);
  let filling = $state(false);

  async function autofill(): Promise<void> {
    const tab = activeLive;
    const cred = matchedCredential;
    if (!tab || !cred || filling) return;
    const ok = await confirmer.ask(
      `Fill the saved credentials for "${cred.username}" on ${cred.domain} into this page? Nothing is submitted automatically — review it before you sign in.`,
      { title: 'Autofill credentials', confirmLabel: 'Fill', danger: false },
    );
    if (!ok) return;
    filling = true;
    try {
      // Reveal is Edit-gated and returns the plaintext password ONLY to this
      // call — it's never stored beyond the local scope below, never logged
      // (the eval string below is never passed to `toasts`/`console`), and
      // it never re-enters this component's own $state.
      const { password } = await browserApi.revealCredential(cred.id);
      // Re-check AFTER both awaits (S18-04): the confirmation and the reveal
      // can take seconds, and the tab may have redirected to another origin
      // meanwhile. The script below re-checks the page's own origin too.
      if (activeLive?.id !== tab.id || !fillTargetOk(activeLive.url, cred.domain)) {
        toasts.warn('Not filled', `This tab left ${cred.domain} — nothing was filled.`);
        return;
      }
      const js = buildFillScript(cred.domain, cred.username, password);
      const result = await nativeBrowser.eval(tab.id, js);
      const outcome = fillResult(result);
      if (outcome === 'origin-changed') {
        toasts.warn('Not filled', `This tab left ${cred.domain} — nothing was filled.`);
      } else if (outcome === 'no-password-field') {
        toasts.warn('Nothing to fill', 'The page no longer has a password field.');
      } else {
        toasts.success('Autofilled', `${cred.username} · ${cred.domain} — review before submitting`);
      }
    } catch (e) {
      toastError('Couldn’t fill the login', e);
    } finally {
      filling = false;
    }
  }

  // Reflect a live tab's in-page navigations into the store + address bar.
  // Event-driven (wry on_navigation) — never polls url().
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    let unlisten = (): void => {};
    let disposed = false;
    void nativeBrowser
      .onUrlChange((id, url) => {
        openedUrl[id] = url; // keep the driver effect from re-navigating (loop)
        browser.trackLiveNav(id, url);
        if (!urlFocused && id === browser.activeId) urlInput = url;
        void injectOverlay(id); // in-page nav reloaded the DOM — re-arm the picker
      })
      .then((un) => (disposed ? un() : (unlisten = un)));
    return () => {
      disposed = true;
      unlisten();
    };
  });

  // A live tab asked to open a new tab (window.open / target=_blank) — open a
  // real in-app live tab for it and focus it.
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    let unlisten = (): void => {};
    let disposed = false;
    void nativeBrowser
      .onNewTab((url) => void browser.openLiveTab(url))
      .then((un) => (disposed ? un() : (unlisten = un)));
    return () => {
      disposed = true;
      unlisten();
    };
  });

  // Destroy the native webview for any live tab this view previously knew
  // about that's no longer in the tab list (closed via TabStrip's ✕). Never
  // closeAll()/hideAll() here — this component may share the window with
  // another native-browser host (e.g. the right-panel Browser tab), and those
  // commands aren't scoped past the window, so they'd tear down its tabs too.
  let knownLiveIds = new Set<string>();
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    const current = new Set(browser.tabs.filter((t) => t.mode === 'live').map((t) => t.id));
    for (const id of knownLiveIds) {
      if (!current.has(id)) {
        void nativeBrowser.close(id);
        delete openedUrl[id];
      }
    }
    knownLiveIds = current;
    liveMru = liveMru.filter((id) => current.has(id));
  });

  // Cap live native webviews (perf SB-14): every live tab kept a hidden
  // WKWebView — its own WebContent process, JS timers and media — for as long
  // as the tab existed. Keep the MAX_LIVE_WEBVIEWS most recently shown; close
  // older hidden ones (only those this view opened). Re-activating one re-opens
  // it at the tab's current URL (`openedUrl` is cleared, so the show effect
  // calls `open`) — in-page state (scroll, form input) is lost, like Safari's
  // tab discarding.
  const MAX_LIVE_WEBVIEWS = 4;
  let liveMru: string[] = [];
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    const id = activeLive?.id;
    if (!id) return;
    untrack(() => {
      liveMru = [id, ...liveMru.filter((x) => x !== id)];
      while (liveMru.length > MAX_LIVE_WEBVIEWS) {
        const old = liveMru.pop();
        if (old && old !== id && openedUrl[old] !== undefined) {
          void nativeBrowser.close(old);
          delete openedUrl[old];
        }
      }
    });
  });

  // Hide (not close — this view doesn't own destroying a tab's session state)
  // every live tab's webview when the module unmounts, so navigating away
  // from Browser doesn't leave one floating over whatever's shown next.
  $effect(() => () => {
    if (!nativeBrowserAvailable) return;
    for (const t of browser.tabs) {
      if (t.mode === 'live') void nativeBrowser.hide(t.id);
    }
  });

  function normalize(raw: string): string {
    const t = raw.trim();
    if (!t) return t;
    return /^[a-z][a-z0-9+.-]*:\/\//i.test(t) ? t : `https://${t}`;
  }

  async function go(): Promise<void> {
    const url = normalize(urlInput);
    if (!url) return;
    urlInput = url;
    try {
      await browser.navigate(url);
      browser.summary = '';
    } catch (e) {
      toastError('Couldn’t load page', e);
    }
  }

  function onkeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') void go();
  }

  function newTab(): void {
    urlInput = '';
    browser.deselect();
  }

  // A live tab with no renderer here (an older daemon without live
  // streaming, off the desktop app) shows Reader instead — say so rather than
  // highlighting a mode that isn't running.
  const liveUnavailable = $derived(browser.activeTab?.mode === 'live' && !activeLive && !activeRemote);
  const liveCapable = $derived(nativeBrowserAvailable || browserLive.supported !== false);

  /** Open the page in a real tab of the viewer's own browser — the useful
   *  "live" view in a remote session (sites refuse to be framed). */
  function openInOwnBrowser(): void {
    const url = browser.activeTab?.url;
    if (url) window.open(url, '_blank', 'noopener,noreferrer');
  }

  function toggleMode(mode: 'reader' | 'live'): void {
    const tab = browser.activeTab;
    if (!tab) return;
    if (mode === 'live' && !liveCapable) {
      openInOwnBrowser();
      return;
    }
    // Leaving live on the daemon engine frees its Chromium session (the
    // daemon would idle-reap it anyway; this just doesn't make it wait).
    if (mode === 'reader' && activeRemote?.id === tab.id) {
      void liveApi.closeLive(tab.id).catch(() => {
        /* no session / already gone: nothing to free */
      });
    }
    void browser.setMode(tab.id, mode);
  }

  async function doSummarize(): Promise<void> {
    const url = browser.activeTab?.url;
    if (!url) return;
    try {
      await browser.runSummarize(url);
    } catch (e) {
      if (isAbortError(e)) toasts.info('Summary stopped');
      else toastError('Couldn’t summarize the page', e);
    }
  }

  async function saveToVault(vaultId: number): Promise<void> {
    const url = browser.activeTab?.url;
    if (!url || vaultSaving) return;
    vaultSaving = true;
    try {
      // Always pass a summary — the prior /summarize output when present,
      // else a slice of the already-fetched page markdown. Either way this
      // keeps the daemon on the caller-supplied-summary path (no second page
      // fetch); omitting it would make vault-save re-fetch the URL server-side.
      const derived = browser.summary || browser.page?.markdown?.slice(0, 4000) || '';
      const resp = await browser.vaultSave(url, vaultId, derived);
      toasts.success('Saved to vault', resp.note_path);
    } catch (e) {
      toastError('Couldn’t save to the vault', e);
    } finally {
      vaultSaving = false;
    }
  }

  async function doVaultSave(e: MouseEvent): Promise<void> {
    if (!browser.activeTab?.url) return;
    if (vault.vaults.length === 0) await vault.load();
    if (vault.vaults.length === 0) {
      toasts.warn('No vault registered', 'Open the Vault module and register one first.');
      return;
    }
    if (vault.vaults.length === 1) {
      await saveToVault(vault.vaults[0].id);
      return;
    }
    const items: MenuItem[] = vault.vaults.map((v) => ({
      label: v.name,
      icon: 'note',
      action: () => void saveToVault(v.id),
    }));
    ctxMenu.show(e, items);
  }

  // ⌘K: the page's verbs (the module page only — the docked panel copy of this
  // view would register the same commands twice).
  $effect(() => {
    if (embedded) return;
    const tab = browser.activeTab;
    const hasUrl = !!tab?.url;
    return registry.register('browser', [
      { id: 'browser.newTab', title: 'New browser tab', group: 'Browser', keywords: 'open url page', run: () => { newTab(); requestAnimationFrame(focusUrl); } },
      { id: 'browser.address', title: 'Go to an address…', group: 'Browser', keywords: 'url location bar navigate', run: focusUrl },
      { id: 'browser.summarize', title: 'Summarize this page', group: 'Browser', keywords: 'tl;dr digest ai', disabled: !hasUrl, run: () => void doSummarize() },
      { id: 'browser.reader', title: 'Use Reader view', group: 'Browser', keywords: 'markdown text mode', disabled: !tab || tab.mode === 'reader', run: () => toggleMode('reader') },
      { id: 'browser.live', title: 'Use Live view', group: 'Browser', keywords: 'chromium interactive mode', disabled: !tab || tab.mode === 'live', run: () => toggleMode('live') },
      { id: 'browser.external', title: 'Open this page in your browser', group: 'Browser', keywords: 'safari chrome external system', disabled: !hasUrl, run: openInOwnBrowser },
    ]);
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`browser`,});
  if(embedded){
     { const $$_pirtSbaT1C = __sveltets_2_ensureComponent(TabStrip); new $$_pirtSbaT1C({ target: __sveltets_2_any(), props: {  "onnew":newTab,}});}
  }else{
    
     { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: { "title":`Browser`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Site credentials`,"title":`Site credentials`,"onclick":openCredentials,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":14,}});} }
      };return __sveltets_2_any(0)},tabs:() => { async ()/*Ωignore_positionΩ*/ => {
          { const $$_pirtSbaT0C = __sveltets_2_ensureComponent(TabStrip); new $$_pirtSbaT0C({ target: __sveltets_2_any(), props: {  "onnew":newTab,"inbar":true,}});}
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions, tabs} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
      
      
     PageHeader}
  }

   { svelteHTML.createElement("div", { "class":`urlbar`,});
    if(activeRemote && browserLive.ready){
       { svelteHTML.createElement("button", {           "class":`icon-btn tool`,"onclick":() => remoteView?.back(),"disabled":remoteState?.status !== 'live' || !remoteState?.session?.can_go_back,"aria-label":`Back`,"title":`Back`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":14,}});}
       }
       { svelteHTML.createElement("button", {           "class":`icon-btn tool`,"onclick":() => remoteView?.forward(),"disabled":remoteState?.status !== 'live' || !remoteState?.session?.can_go_forward,"aria-label":`Forward`,"title":`Forward`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":14,}});}
       }
       { svelteHTML.createElement("button", {           "class":`icon-btn tool`,"onclick":() => remoteView?.reload(),"disabled":remoteState?.status !== 'live',"aria-label":`Reload page`,"title":`Reload page (⌘R)`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":14,}});}
       }
    }
     { const $$_input2 = svelteHTML.createElement("input", {                          "type":`text`,"dir":`ltr`,"placeholder":`e.g. https://example.com`,"aria-label":`Address`,"enterkeyhint":`go`,"inputmode":`url`,"autocapitalize":`off`,"spellcheck":`false`,"bind:value":urlInput,"onkeydown":onkeydown,"onfocus":() => (urlFocused = true),"onblur":() => (urlFocused = false),});urlEl = $$_input2;/*Ωignore_startΩ*/() => urlInput = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("button", {         "class":`icon-btn tool go-btn`,"onclick":go,"title":`Go`,"aria-label":`Go`,"disabled":!urlInput.trim(),});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":14,}});}
     }
    if(browser.activeTab){
       { svelteHTML.createElement("div", {     "class":`mode-toggle`,"role":`group`,"aria-label":`Tab mode`,});
         { svelteHTML.createElement("button", {            "class":`seg`,"onclick":() => toggleMode('reader'),"aria-label":`Reader mode`,"aria-pressed":browser.activeTab.mode === 'reader',"title":`Reader mode — fetched and rendered as clean markdown`,});browser.activeTab.mode === 'reader';
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":13,}});}
         }
         { svelteHTML.createElement("button", {            "class":`seg`,"onclick":() => toggleMode('live'),"aria-label":liveCapable ? 'Live mode' : 'Open in a new browser tab',"aria-pressed":liveCapable && browser.activeTab.mode === 'live',"title":liveCapable
            ? 'Live mode — a real browser'
            : 'Open this page in a new browser tab. This Otto daemon has no live browser.',});liveCapable && browser.activeTab.mode === 'live';
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":13,}});}
         }
       }
      if(nativeBrowserAvailable && browser.activeTab.mode === 'live'){
         { svelteHTML.createElement("button", {         "class":`icon-btn tool`,"onclick":chooseRenderer,"aria-label":`Live engine`,"title":browserLive.renderer === 'native'
            ? "Live engine: this Mac’s web view"
            : "Live engine: Otto’s Chromium",});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":14,}});}
         }
      }
    }
    if(activeLive){
       { svelteHTML.createElement("button", {            "class":`icon-btn tool`,"onclick":togglePick,"title":pickMode ? 'Picking… click an element in the page' : 'Pick an element to mark',"aria-label":`Pick an element to mark`,"aria-pressed":pickMode,});pickMode;
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`target`,"size":14,}});}
       }
    }
    if(canAutofill){
       { svelteHTML.createElement("button", {           "class":`icon-btn tool`,"onclick":autofill,"disabled":filling,"aria-label":`Autofill saved credentials`,"title":`Autofill ${matchedCredential?.username ?? ''} for ${matchedCredential?.domain ?? ''}`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`key`,"size":14,}});}
       }
    }
    
    if(browser.activeTab){
       { svelteHTML.createElement("button", {           "class":`icon-btn tool`,"onclick":doSummarize,"disabled":browser.summarizing,"aria-label":`Summarize this page`,"title":browser.summarizing ? 'Summarizing…' : 'Summarize this page',});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":14,}});}
       }
       { svelteHTML.createElement("button", {           "class":`icon-btn tool`,"onclick":doVaultSave,"disabled":vaultSaving,"aria-label":`Save to vault`,"title":vaultSaving ? 'Saving to vault…' : 'Save to vault',});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`book`,"size":14,}});}
       }
    }
   }

  if(browser.summarizing && !browser.summary){
     {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),('Summarizing this page…')));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`summary`,});
       { svelteHTML.createElement("div", { "class":`summary-head`,}); { svelteHTML.createElement("span", {});     } }
       { svelteHTML.createElement("p", { "class":`dim-line`,});   }
       { svelteHTML.createElement("div", {}); { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => browser.stopSummarize(),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}  } }
     }}
  } else if (browser.summary){
     { svelteHTML.createElement("div", { "class":`summary`,});
       { svelteHTML.createElement("div", { "class":`summary-head`,});
         { svelteHTML.createElement("span", {});     }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (browser.summary = ''),"aria-label":`Close summary`,"title":`Close summary`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
       }
       { svelteHTML.createElement("p", { "class":`dim-line`,});if(browser.summaryAt){ { svelteHTML.createElement("time", { "datetime":browser.summaryAt,});new Date(browser.summaryAt).toLocaleTimeString(); } }  browser.summaryEngine || 'the browser';if(browser.summaryDegraded){        } }
       { svelteHTML.createElement("p", {});browser.summary; }
     }
  }

   { svelteHTML.createElement("div", { "class":`body`,});
    if(browser.tabsError || (browser.loadingTabs && !browser.tabs.length)){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {           "what":`browser tabs`,"variant":`page`,"loading":browser.loadingTabs,"error":browser.tabsError,"empty":true,"onretry":() => ws.currentId && void browser.loadTabs(ws.currentId),}});}
    } else if (activeLive){
      
       { const $$_div2 = svelteHTML.createElement("div", {  "class":`live-host`,});liveHostEl = $$_div2; }
    } else if (activeRemote && browserLive.ready){
       { const $$_weiVeviLetomeR2C = __sveltets_2_ensureComponent(RemoteLiveView); const $$_weiVeviLetomeR2 = new $$_weiVeviLetomeR2C({ target: __sveltets_2_any(), props: {              "tab":activeRemote,"onnav":onRemoteNav,"onstate":(st) => (remoteState = st),"onfocusurl":focusUrl,"onnewtab":(url) => void browser.openLiveTab(url),"onreader":() => toggleMode('reader'),}});remoteView = $$_weiVeviLetomeR2;}
    } else if (activeRemote){
       { const $$_puteSenignEeviL2C = __sveltets_2_ensureComponent(LiveEngineSetup); new $$_puteSenignEeviL2C({ target: __sveltets_2_any(), props: {  "onreader":() => toggleMode('reader'),}});}
    }else{
      if(liveUnavailable){
         { svelteHTML.createElement("div", { "class":`live-note`,});
           { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`globe`,"size":13,}});}
           { svelteHTML.createElement("span", {});                   }
           { svelteHTML.createElement("span", { "class":`grow`,}); }
           { svelteHTML.createElement("button", {   "class":`btn`,"onclick":openInOwnBrowser,});    }
           { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":() => browser.activeTab && void browser.setMode(browser.activeTab.id, 'reader'),});   }
         }
      }
       { const $$_weiVredaeR2C = __sveltets_2_ensureComponent(ReaderView); new $$_weiVredaeR2C({ target: __sveltets_2_any(), props: {           "page":browser.page,"loading":browser.loadingPage,"error":browser.pageError,"onopenurl":focusUrl,"onretry":() => {
          const url = browser.activeTab?.url;
          if (url) void browser.loadPage(url, { fresh: true });
        },}});}
      if(browser.page){
         { const $$_liaRsetoN2C = __sveltets_2_ensureComponent(NotesRail); new $$_liaRsetoN2C({ target: __sveltets_2_any(), props: {  "annotations":browser.annotations,}});}
      }
    }
   }

  if(embedded){
     { const $$_raBksA1C = __sveltets_2_ensureComponent(AskBar); new $$_raBksA1C({ target: __sveltets_2_any(), props: {    "sessionId":targetSessionId,"unboundHint":`Open an agent session to ask about this page.`,}});}
  }else{
     { const $$_kcoDtnegA1C = __sveltets_2_ensureComponent(AgentDock); new $$_kcoDtnegA1C({ target: __sveltets_2_any(), props: {}});}
  }
 }

if(showCredentials && ws.currentId){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":`Site credentials`,"width":640,"onclose":() => { showCredentials = false; const id = ws.currentId; if (id) void browserApi.listCredentials(id).then(c => { if (ws.currentId === id) credentials = c; }).catch(() => {}); },}});
        { 
        { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {  "what":`site credentials`,"loading":true,}});}
    try { const $$_value = await (credentialsPanel);{ const panel = $$_value; 
      if(panel){ { const $$_tluafed_lenap1C = __sveltets_2_ensureComponent(panel.default); new $$_tluafed_lenap1C({ target: __sveltets_2_any(), props: {  "workspaceId":ws.currentId,}});}}
    }} catch($$_e) { const error = __sveltets_2_any();
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`site credentials`,"error":loadErrorText(error),"onretry":openCredentials,}});}
    }}
   Modal}
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const BrowserView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type BrowserView__SvelteComponent_ = ReturnType<typeof BrowserView__SvelteComponent_>;
/*Ωignore_endΩ*/export default BrowserView__SvelteComponent_;