<script lang="ts">
  // Browser module page: tab strip + URL bar + reader page (a thin view over a $state store).

  import { untrack } from 'svelte';
  import { toastError } from '../../lib/toastError';
  import { loadErrorText } from '../../lib/loadError';
  import { pollWhileVisible } from '../../lib/poll';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { browser } from '../../lib/stores/browser.svelte';
  import { vault } from '../vault/vault.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { isAbortError } from '../../lib/api/client';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { registry } from '../../lib/commands.svelte';
  import { nativeBrowser, nativeBrowserAvailable, type Rect } from '../../lib/nativeBrowser';
  import * as browserApi from '../../lib/api/browser';
  import type { BrowserCredential, BrowserInspectedElement, SimDevice } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import TabStrip from './TabStrip.svelte';
  import Modal from '../../lib/components/Modal.svelte';
  let showCredentials = $state(false);
  let credentialsPanel = $state<Promise<typeof import('./CredentialsPanel.svelte')> | null>(null);
  function openCredentials(): void {
    credentialsPanel = import('./CredentialsPanel.svelte');
    showCredentials = true;
  }
  let showSimulators = $state(false);
  let simulatorsPanel = $state<Promise<typeof import('./simulators/SimulatorPanel.svelte')> | null>(null);
  function openSimulators(): void {
    simulatorsPanel = import('./simulators/SimulatorPanel.svelte');
    showSimulators = true;
  }
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import BrowserMenu from './BrowserMenu.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import BrowserSummary from './BrowserSummary.svelte';
  import ReaderView from './ReaderView.svelte';
  import NotesRail from './NotesRail.svelte';
  import AgentDock from './AgentDock.svelte';
  import AskBar from './AskBar.svelte';
  import BrowserUrlBar from './BrowserUrlBar.svelte';
  import type LiveBrowserStageView from './live/LiveBrowserStage.svelte';
  import type { LiveViewState } from './live/protocol';
  import LiveEngineSetup from './live/LiveEngineSetup.svelte';
  import SimulatorView from './simulators/SimulatorView.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';
  // The live (remote engine) stage loads with the first live tab.
  const LiveBrowserStage = lazyComponent(() => import('./live/LiveBrowserStage.svelte'));
  import {
    DEFAULT_EMULATION_STATE,
    stateForPreset,
    type DeviceEmulationState,
  } from './live/devicePresets';
  import { consumeBrowserDeepLink } from './deepLink';
  import { browserBookmarks } from './bookmarks.svelte';
  import { browserLive } from '../../lib/stores/browserLive.svelte';
  import * as liveApi from '../../lib/api/browserLive';
  import { fillAllowedForUrl, hostOf, matchDomain } from './autofill';
  import { drainOverlayTick, injectLiveOverlay, runLiveAutofill } from './autofillRunner';
  import { liveWebviews, nativeEventHub, NATIVE_NAV_JS, type LiveNav } from './liveWebviews';

  interface Props {
    /** Hosted inside the agent-mode right panel (the "v2" Browser tab): no
     *  embedded agent dock — the session is the main pane — just the ask bar
     *  targeting `targetSessionId`. */
    embedded?: boolean;
    /** The session the embedded ask bar submits to (the active agent session). */
    targetSessionId?: string | null;
  }
  let { embedded = false, targetSessionId = null }: Props = $props();

  let urlInput = $state('');
  let vaultSaving = $state(false);
  let urlFocused = $state(false);
  let emulation = $state<DeviceEmulationState>({ ...DEFAULT_EMULATION_STATE });
  let inspectorOpen = $state(false);
  let mirroredSim = $state<SimDevice | null>(null);
  let simSplit = $state(true);

  // (Re)load the tab list & workspace bookmarks when the workspace changes.
  $effect(() => {
    const id = ws.currentId;
    if (id) {
      void browser.loadTabs(id);
      browserBookmarks.load(id);
    }
  });

  // Single unified handler for `#/browser?...` deep-links (Git PR previews,
  // IDE dev-server chips, device presets, and simulator handoffs).
  $effect(() => {
    if (embedded || !ws.currentId) return;
    void router.parts;
    const run = () =>
      void consumeBrowserDeepLink({
        setUrlInput: (u) => (urlInput = u),
        setEmulation: (emu) => (emulation = emu),
        onSimulator: (dev, split) => { mirroredSim = dev; simSplit = split; },
        onSimulatorMissing: openSimulators,
        liveCapable: untrack(() => liveCapable),
      });
    run();
    window.addEventListener('hashchange', run);
    return () => window.removeEventListener('hashchange', run);
  });

  // Keep the URL bar in sync with the active tab — unless the field is
  // focused (typing a new URL) or a live tab's in-page navigation is
  // updating it, handled by the onUrlChange listener below instead.
  $effect(() => {
    if (!urlFocused) urlInput = browser.activeTab?.url ?? urlInput;
  });

  // Live tab renderers: native (desktop Tauri child webview) or remote
  // (daemon-owned Chromium streamed into RemoteLiveView).
  let liveHostEl = $state<HTMLDivElement | null>(null);
  const fullSimActive = $derived(!!mirroredSim && !simSplit);
  const activeLive = $derived(
    !fullSimActive && nativeBrowserAvailable && browserLive.renderer === 'native' && browser.activeTab?.mode === 'live'
      ? browser.activeTab
      : null,
  );
  const activeRemote = $derived(
    !fullSimActive && browserLive.renderer === 'remote' && browserLive.supported !== false && browser.activeTab?.mode === 'live'
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

  // Live tabs skip `loadPage()`, so load the active live URL's marks here on
  // tab selection, mode switch, and in-page navigation (both native & remote).
  $effect(() => {
    const liveTab = activeLive ?? activeRemote;
    const _ws = ws.currentId;
    const _id = liveTab?.id;
    const url = liveTab?.url;
    if (!_ws || !_id || !url) return;
    void browser.loadAnnotations(url);
  });

  // Marks rail visibility: auto-opens when the page has marks (or user clicks
  // the Marks toggle), and stays collapsed when empty so narrow embedded panels
  // and Reader view don't lose 260px to an empty rail.
  let marksRailOverride = $state<boolean | null>(null);
  let lastMarksTabId = $state<string | null>(null);
  $effect(() => {
    const id = browser.activeId;
    if (id !== lastMarksTabId) {
      lastMarksTabId = id;
      marksRailOverride = null;
    }
  });
  const canShowMarksRail = $derived(
    !!activeLive || (!!activeRemote && browserLive.ready) || !!browser.page,
  );
  const showMarksRail = $derived(
    canShowMarksRail && (marksRailOverride ?? browser.annotations.length > 0),
  );
  function closeMarksRail(): void {
    marksRailOverride = false;
    urlBar?.focusMarksToggle();
  }

  // ── Remote live view state (nav buttons, ⌘L) ───────────────────────────
  // Element picking has no remote counterpart in the live protocol yet, so
  // the target button is simply absent for a remote tab (no dead control).
  let remoteView = $state<ReturnType<typeof LiveBrowserStageView> | null>(null);
  let readerView = $state<ReturnType<typeof ReaderView> | null>(null);
  let remoteState: LiveViewState | null = $state(null);
  let urlBar = $state<ReturnType<typeof BrowserUrlBar> | null>(null);
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
    urlBar?.focusUrl();
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
    // `liveWebviews` outlives this view, so a re-mount (or a tab switch)
    // only shows the webview, keeping its scroll / form / app state.
    if (liveWebviews.action(active.id, active.url) === 'navigate') {
      liveWebviews.set(active.id, active.url);
      // create-or-navigate + show, then arm the picker overlay on the fresh
      // page (a same-tab URL change reloads the DOM, dropping any overlay a
      // previous injection installed).
      void nativeBrowser.open(active.id, active.url, r).then(() => injectOverlay(active.id));
    } else {
      // Places + shows it (re-creates one another host closed): the shell
      // never re-navigates a webview to the URL it already shows.
      void nativeBrowser.open(active.id, active.url, r);
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

  let pickMode = $state(false);
  const injectOverlay = (id: string) => injectLiveOverlay(id, pickMode && id === activeLive?.id);

  function togglePick(): void {
    const active = activeLive;
    if (!active) return;
    pickMode = !pickMode;
    void nativeBrowser.eval(active.id, `window.__ottoOverlay && window.__ottoOverlay.setPicking(${pickMode})`);
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

  let pollInFlight = false;
  async function pollOverlay(id: string, url: string): Promise<void> {
    if (pollInFlight) return;
    pollInFlight = true;
    try {
      await drainOverlayTick(id, url);
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
    if (!id) {
      credentials = [];
      return;
    }
    let aborted = false;
    void browserApi
      .listCredentials(id)
      .then((c) => {
        if (!aborted && ws.currentId === id) credentials = c;
      })
      .catch(() => {
        if (!aborted && ws.currentId === id) credentials = [];
      });
    return () => {
      aborted = true;
    };
  });

  const matchedCredentials = $derived.by(() => {
    const tab = activeLive;
    if (!tab) return [] as BrowserCredential[];
    const host = hostOf(tab.url);
    if (!host || !fillAllowedForUrl(tab.url)) return [] as BrowserCredential[];
    return credentials.filter((c) => matchDomain(host, c.domain));
  });
  const matchedCredential = $derived(matchedCredentials[0] ?? null);

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

  const canAutofill = $derived(!!activeLive && matchedCredentials.length > 0 && hasLoginForm);
  let filling = $state(false);

  function onAutofillClick(e: MouseEvent): void {
    if (matchedCredentials.length === 0 || filling) return;
    if (matchedCredentials.length === 1) {
      void autofill(matchedCredentials[0]);
      return;
    }
    ctxMenu.show(
      e,
      matchedCredentials.map((c) => ({
        label: `${c.username} (${c.domain})`,
        icon: 'key',
        action: () => void autofill(c),
      })),
    );
  }

  async function autofill(cred: BrowserCredential): Promise<void> {
    if (filling) return;
    filling = true;
    try {
      await runLiveAutofill(cred, () => activeLive);
    } finally {
      filling = false;
    }
  }

  // Reflect live-tab in-page navigations and `window.open()` through a single
  // ref-counted native event hub so multiple mounted BrowserView instances
  // (e.g. Browser page + embedded panels / split ThreadInspector) never open
  // duplicate tabs or double-inject the overlay.
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    return nativeEventHub.attach(nativeBrowser, {
      onUrlChange(id, url) {
        liveWebviews.set(id, url); // keep the driver effect from re-navigating (loop)
        browser.trackLiveNav(id, url);
        void injectOverlay(id); // in-page nav reloaded the DOM — re-arm the picker
      },
      onLocalUrlChange(id, url) {
        if (!urlFocused && id === browser.activeId) urlInput = url;
      },
      onNewTab(url) {
        void browser.openLiveTab(url);
      },
    });
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
        liveWebviews.forget(id);
      }
    }
    knownLiveIds = current;
  });

  // Cap live native webviews (perf SB-14): every live tab kept a hidden
  // WKWebView — its own WebContent process, JS timers and media — for as long
  // as the tab existed. Keep the MAX_LIVE_WEBVIEWS most recently shown; close
  // older hidden ones. Re-activating one re-opens it at the tab's current URL
  // — in-page state (scroll, form input) is lost, like Safari's tab discarding.
  const MAX_LIVE_WEBVIEWS = 4;
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    const id = activeLive?.id;
    if (!id) return;
    untrack(() => liveWebviews.touch(id, MAX_LIVE_WEBVIEWS).forEach((old) => void nativeBrowser.close(old)));
  });

  // Back / Forward / Reload / Stop for whichever engine renders the live tab.
  function liveNav(nav: LiveNav): void {
    if (activeRemote) {
      remoteView?.[nav]();
    } else if (activeLive) {
      if (nav === 'reload') void nativeBrowser.reload(activeLive.id);
      else void nativeBrowser.eval(activeLive.id, NATIVE_NAV_JS[nav]);
    }
  }
  const remoteDown = $derived.by(() => !!activeRemote && remoteState?.status !== 'live');

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

  function toggleDevicePreview(): void {
    if (!emulation.enabled) {
      if (nativeBrowserAvailable && browserLive.renderer === 'native') {
        browserLive.setPref('remote');
      }
      if (browser.activeTab && browser.activeTab.mode !== 'live') {
        void browser.setMode(browser.activeTab.id, 'live');
      }
      emulation = stateForPreset('iphone-16-pro', { colorScheme: emulation.colorScheme });
    } else {
      emulation = { ...DEFAULT_EMULATION_STATE };
    }
  }

  function toggleInspector(): void {
    if (!inspectorOpen && nativeBrowserAvailable && browserLive.renderer === 'native') {
      browserLive.setPref('remote');
    }
    inspectorOpen = !inspectorOpen;
  }

  async function markInspectedElement(el: BrowserInspectedElement): Promise<void> {
    const url = browser.activeTab?.url;
    if (!url) return;
    try {
      await browser.createAnnotation({
        url,
        selector: el.selector,
        excerpt: el.outer_html.slice(0, 2000),
        text: el.text.slice(0, 2000),
        comment: '',
      });
      toasts.success('Marked element', el.selector);
    } catch (e) {
      toastError('Couldn’t save mark', e);
    }
  }

  function mirrorSimulator(d: SimDevice, split: boolean): void {
    mirroredSim = d;
    simSplit = split && !!browser.activeTab;
    showSimulators = false;
  }

  const isBookmarked = $derived(browserBookmarks.has(browser.activeTab?.url));

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
      { id: 'browser.devicePreview', title: emulation.enabled ? 'Exit device preview' : 'Toggle device preview (mobile/tablet)', group: 'Browser', keywords: 'responsive iphone android ipad viewport emulate', disabled: !tab, run: toggleDevicePreview },
      { id: 'browser.inspector', title: inspectorOpen ? 'Close page inspector' : 'Open page inspector (Console & Network)', group: 'Browser', keywords: 'devtools console network element inspect', disabled: !activeRemote, run: toggleInspector },
      { id: 'browser.simulators', title: 'iOS Simulators & Android Emulators…', group: 'Browser', keywords: 'simulator emulator xcode ios android avd mobile', run: openSimulators },
      { id: 'browser.screenshot', title: 'Capture live page screenshot (PNG)', group: 'Browser', keywords: 'screenshot capture image png', disabled: !(activeRemote && browserLive.ready), run: () => void remoteView?.captureScreenshot('viewport') },
      { id: 'browser.bookmark', title: isBookmarked ? 'Remove page bookmark' : 'Bookmark this page', group: 'Browser', keywords: 'favorite pin url save', disabled: !hasUrl, run: () => { if (tab?.url) { const added = browserBookmarks.toggle(tab.url, tab.title); toasts.info(added ? 'Bookmarked' : 'Removed bookmark', tab.title || tab.url); } } },
      { id: 'browser.external', title: 'Open this page in your browser', group: 'Browser', keywords: 'safari chrome external system', disabled: !hasUrl, run: openInOwnBrowser },
    ]);
  });
</script>

<div class="browser">
  {#if embedded}
    <TabStrip onnew={newTab} />
  {:else}
    <!-- Module page: the page tabs ride inline in the unified header bar. -->
    <PageHeader title="Browser">
      {#snippet actions()}
        <BrowserMenu
          oncredentials={openCredentials}
          onsimulators={openSimulators}
          simActive={!!mirroredSim}
        />
      {/snippet}
      {#snippet tabs()}
        <TabStrip onnew={newTab} inbar />
      {/snippet}
    </PageHeader>
  {/if}

  <BrowserUrlBar
    bind:this={urlBar}
    {urlInput}
    live={{
      activeLive,
      activeRemote,
      remoteState,
      remoteDown,
      liveCapable,
      pickMode,
      canAutofill,
      matchedCredential,
      matchedCredentialsCount: matchedCredentials.length,
      filling,
    }}
    tools={{
      vaultSaving,
      emulationEnabled: emulation.enabled,
      inspectorOpen,
      mirroredSim,
      canShowMarksRail,
      showMarksRail,
    }}
    actions={{
      onUrlInput: (v) => (urlInput = v),
      onUrlFocus: (f) => (urlFocused = f),
      go: () => void go(),
      liveNav,
      toggleMode,
      chooseRenderer,
      setGuard: (on) => remoteView?.setGuard(on) ?? false,
      togglePick,
      autofill: onAutofillClick,
      toggleDevice: toggleDevicePreview,
      toggleInspector,
      screenshot: () => void remoteView?.captureScreenshot('viewport'),
      simulators: openSimulators,
      summarize: () => void doSummarize(),
      vaultSave: (e) => void doVaultSave(e),
      toggleMarksRail: () => (marksRailOverride = !showMarksRail),
    }}
  />

  <BrowserSummary />

  <div class="body" class:sim-split={!!mirroredSim && simSplit}>
    {#if !fullSimActive}
      <div class="web-pane">
        {#if browser.tabsError || (browser.loadingTabs && !browser.tabs.length)}
          <LoadState what="browser tabs" variant="page" loading={browser.loadingTabs} error={browser.tabsError} empty
            onretry={() => ws.currentId && void browser.loadTabs(ws.currentId)} />
        {:else if activeLive}
          <!-- The native child webview paints ABOVE this div's rect — the div
               itself just holds the geometry the ResizeObserver above tracks. -->
          <div class="live-host" bind:this={liveHostEl}></div>
        {:else if activeRemote && browserLive.ready && !LiveBrowserStage.component}
          <LoadState what="the live browser" variant="panel" empty loading={!LiveBrowserStage.error}
            error={LiveBrowserStage.error} onretry={() => LiveBrowserStage.retry()} />
        {:else if activeRemote && browserLive.ready}
          {@const Stage = LiveBrowserStage.component}
          <Stage
            bind:this={remoteView}
            tab={activeRemote}
            {emulation}
            onemulationchange={(next) => (emulation = next)}
            {inspectorOpen}
            oninspectorchange={(open) => (inspectorOpen = open)}
            onmark={(el) => void markInspectedElement(el)}
            onnav={onRemoteNav}
            onstate={(st) => (remoteState = st)}
            onfocusurl={focusUrl}
            onnewtab={(url) => void browser.openLiveTab(url)}
            onreader={() => toggleMode('reader')}
          />
        {:else if activeRemote}
          <LiveEngineSetup onreader={() => toggleMode('reader')} />
        {:else}
          {#if liveUnavailable}
            <div class="live-note">
              <Icon name="globe" size={13} />
              <span>This tab is in live mode, but this Otto daemon has no live browser. You’re seeing Reader view here.</span>
              <span class="grow"></span>
              <button class="btn" onclick={openInOwnBrowser}>Open in new tab</button>
              <button class="btn ghost" onclick={() => browser.activeTab && void browser.setMode(browser.activeTab.id, 'reader')}>Switch to Reader</button>
            </div>
          {/if}
          <ReaderView
            bind:this={readerView}
            page={browser.page}
            loading={browser.loadingPage}
            error={browser.pageError}
            onopenurl={focusUrl}
            onnavigate={(url) => { urlInput = url; void go(); }}
            onretry={() => {
              const url = browser.activeTab?.url;
              if (url) void browser.loadPage(url, { fresh: true });
            }}
          />
        {/if}
      </div>
    {/if}

    {#if mirroredSim}
      <SimulatorView
        device={mirroredSim}
        currentUrl={browser.activeTab?.url ?? ''}
        split={simSplit}
        ontogglesplit={() => (simSplit = !simSplit)}
        ondevicechange={(d) => (mirroredSim = d)}
        onclose={() => (mirroredSim = null)}
      />
    {/if}
    {#if showMarksRail}
      <div class="notes-wrap mobile-open">
        <NotesRail
          annotations={browser.annotations}
          liveMode={!!activeLive || !!activeRemote}
          onclose={closeMarksRail}
          onfocusmark={!activeLive && !activeRemote && browser.page ? (sel) => readerView?.scrollToMark(sel) : undefined}
        />
      </div>
    {/if}
  </div>

  {#if embedded}
    <AskBar sessionId={targetSessionId} unboundHint="Open an agent session to ask about this page." />
  {:else}
    <AgentDock />
  {/if}
</div>

{#if showCredentials && ws.currentId}
  <Modal title="Site credentials" width={640} onclose={() => { showCredentials = false; const id = ws.currentId; if (id) void browserApi.listCredentials(id).then(c => { if (ws.currentId === id) credentials = c; }).catch(() => {}); }}>
    {#await credentialsPanel}
      <LoadState what="site credentials" loading />
    {:then panel}
      {#if panel}<panel.default workspaceId={ws.currentId} />{/if}
    {:catch error}
      <LoadState what="site credentials" error={loadErrorText(error)} onretry={openCredentials} />
    {/await}
  </Modal>
{/if}

{#if showSimulators}
  <Modal title="iOS Simulators & Android Emulators" width={680} onclose={() => (showSimulators = false)}>
    {#await simulatorsPanel}
      <LoadState what="simulators" loading />
    {:then panel}
      {#if panel}
        <panel.default
          currentUrl={browser.activeTab?.url ?? ''}
          onmirror={mirrorSimulator}
        />
      {/if}
    {:catch error}
      <LoadState what="simulators" error={loadErrorText(error)} onretry={openSimulators} />
    {/await}
  </Modal>
{/if}

<style>
  .browser {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    min-width: 0;
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
    min-width: 0;
    position: relative;
  }
  .web-pane {
    flex: 1;
    display: flex;
    min-height: 0;
    min-width: 0;
    position: relative;
  }
  .notes-wrap {
    display: flex;
    min-height: 0;
    flex-shrink: 0;
  }
  @media (max-width: 640px) {
    .body.sim-split {
      flex-direction: column;
    }
  }
  @media (max-width: 640px), (max-height: 500px) {
    .web-pane {
      flex-direction: column;
    }
    .notes-wrap:not(.mobile-open) {
      display: none;
    }
  }
  @container (max-width: 640px) {
    .body {
      flex-direction: column;
    }
  }
  .live-note {
    position: absolute;
    inset-inline: 12px;
    inset-block-start: 8px;
    z-index: 2;
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 8px;
    padding-block: 6px; padding-inline: 10px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--info-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .live-note .grow { flex: 1; }
  .live-host {
    flex: 1;
    min-width: 0;
    min-height: 0;
    /* Empty on purpose — the Tauri child webview paints natively above this
       rect; ResizeObserver above keeps its bounds synced to this div. */
  }
</style>
