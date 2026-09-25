<script lang="ts">
  // Browser module page: tab strip + URL bar + reader-mode page. Mirrors the
  // shape of VaultPage/LoopsPage (a thin view over a $state store).

  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { browser } from '../../lib/stores/browser.svelte';
  import { vault } from '../vault/vault.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { nativeBrowser, nativeBrowserAvailable, type Rect } from '../../lib/nativeBrowser';
  import * as browserApi from '../../lib/api/browser';
  import type { BrowserCredential } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import TabStrip from './TabStrip.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
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
  let summarizing = $state(false);
  let summary = $state('');
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
        label: "This Mac's web view",
        icon: cur === 'native' ? 'check' : 'globe',
        action: () => browserLive.setPref('native'),
      },
      {
        label: "Otto's Chromium (streams anywhere, agents can drive it)",
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
          toasts.error('Failed to save mark', e instanceof Error ? e.message : undefined);
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
    const timer = setInterval(() => void pollOverlay(id, url), 700);
    return () => clearInterval(timer);
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

  function hostOf(url: string): string | null {
    try {
      return new URL(url).hostname || null;
    } catch {
      return null;
    }
  }

  /** Mirrors `otto_state::browser_credentials::match_domain` (exact host, or
   *  any subdomain of a stored domain). Client-side only — a mismatch here
   *  just hides the key icon; the server independently re-derives its own
   *  match for the agent-facing `/browser/login` route, so this is never a
   *  security boundary, only a UX one. */
  function matchDomain(host: string, domain: string): boolean {
    const h = host.trim().replace(/\.$/, '').toLowerCase();
    const d = domain.trim().replace(/\.$/, '').toLowerCase();
    if (!d) return false;
    return h === d || h.endsWith(`.${d}`);
  }

  function fillAllowedForUrl(url: string): boolean {
    let u: URL;
    try {
      u = new URL(url);
    } catch {
      return false;
    }
    if (u.protocol === 'https:') return true;
    if (u.protocol === 'http:') {
      return u.hostname === 'localhost' || u.hostname === '127.0.0.1' || u.hostname === '::1';
    }
    return false;
  }

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
    let cancelled = false;
    const check = async (): Promise<void> => {
      const raw = await nativeBrowser.eval(
        id,
        "window.__ottoOverlay && window.__ottoOverlay.hasLoginForm ? window.__ottoOverlay.hasLoginForm() : false",
      );
      if (!cancelled) hasLoginForm = raw === 'true';
    };
    void check();
    const timer = setInterval(() => void check(), 1000);
    return () => {
      cancelled = true;
      clearInterval(timer);
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
      const userJs = JSON.stringify(cred.username);
      const passJs = JSON.stringify(password);
      const js =
        '(function(){' +
        'var pwd = document.querySelector(\'input[type="password"]\');' +
        "if (!pwd) return 'no-password-field';" +
        'var user = document.querySelector(\'input[type="email"]\') || ' +
        'document.querySelector(\'input[autocomplete="username"]\') || ' +
        'document.querySelector(\'input[type="text"]\');' +
        'if (user) {' +
        'user.focus();' +
        `user.value = ${userJs};` +
        "user.dispatchEvent(new Event('input', {bubbles: true}));" +
        "user.dispatchEvent(new Event('change', {bubbles: true}));" +
        '}' +
        'pwd.focus();' +
        `pwd.value = ${passJs};` +
        "pwd.dispatchEvent(new Event('input', {bubbles: true}));" +
        "pwd.dispatchEvent(new Event('change', {bubbles: true}));" +
        "return 'filled';" +
        '})()';
      const result = await nativeBrowser.eval(tab.id, js);
      if (result === "'no-password-field'" || result === 'no-password-field') {
        toasts.warn('Nothing to fill', 'The page no longer has a password field.');
      } else {
        toasts.success('Autofilled', `${cred.username} · ${cred.domain} — review before submitting`);
      }
    } catch (e) {
      toasts.error('Autofill failed', e instanceof Error ? e.message : undefined);
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
      summary = '';
    } catch (e) {
      toasts.error('Failed to load page', e instanceof Error ? e.message : undefined);
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
    summarizing = true;
    try {
      const resp = await browser.summarize(url);
      summary = resp.summary;
    } catch (e) {
      toasts.error('Summarize failed', e instanceof Error ? e.message : undefined);
    } finally {
      summarizing = false;
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
      const derived = summary || browser.page?.markdown?.slice(0, 4000) || '';
      const resp = await browser.vaultSave(url, vaultId, derived);
      toasts.success('Saved to vault', resp.note_path);
    } catch (e) {
      toasts.error('Vault save failed', e instanceof Error ? e.message : undefined);
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
</script>

<div class="browser">
  {#if embedded}
    <TabStrip onnew={newTab} />
  {:else}
    <!-- Module page: the page tabs ride inline in the unified header bar. -->
    <PageHeader title="Browser">
      {#snippet tabs()}
        <TabStrip onnew={newTab} inbar />
      {/snippet}
    </PageHeader>
  {/if}

  <div class="urlbar">
    {#if activeRemote && browserLive.ready}
      <button
        class="icon-btn tool"
        onclick={() => remoteView?.back()}
        disabled={remoteState?.status !== 'live' || !remoteState?.session?.can_go_back}
        aria-label="Back"
        title="Back"
      >
        <Icon name="chevronLeft" size={14} />
      </button>
      <button
        class="icon-btn tool"
        onclick={() => remoteView?.forward()}
        disabled={remoteState?.status !== 'live' || !remoteState?.session?.can_go_forward}
        aria-label="Forward"
        title="Forward"
      >
        <Icon name="chevronRight" size={14} />
      </button>
      <button
        class="icon-btn tool"
        onclick={() => remoteView?.reload()}
        disabled={remoteState?.status !== 'live'}
        aria-label="Reload page"
        title="Reload page (⌘R)"
      >
        <Icon name="refresh" size={14} />
      </button>
    {/if}
    <input
      type="text"
      placeholder="Enter URL"
      aria-label="Address"
      bind:this={urlEl}
      bind:value={urlInput}
      onkeydown={onkeydown}
      onfocus={() => (urlFocused = true)}
      onblur={() => (urlFocused = false)}
    />
    <button class="icon-btn tool" onclick={go} title="Go" aria-label="Go">
      <Icon name="external" size={14} />
    </button>
    {#if browser.activeTab}
      <div class="mode-toggle" role="group" aria-label="Tab mode">
        <button
          class="seg"
          class:active={browser.activeTab.mode === 'reader'}
          onclick={() => toggleMode('reader')}
          aria-label="Reader mode"
          aria-pressed={browser.activeTab.mode === 'reader'}
          title="Reader mode — fetched and rendered as clean markdown"
        >
          <Icon name="file" size={13} />
        </button>
        <button
          class="seg"
          class:active={liveCapable && browser.activeTab.mode === 'live'}
          onclick={() => toggleMode('live')}
          aria-label={liveCapable ? 'Live mode' : 'Open in a new browser tab'}
          aria-pressed={liveCapable && browser.activeTab.mode === 'live'}
          title={liveCapable
            ? 'Live mode — a real browser'
            : 'Open this page in a new browser tab. This Otto daemon has no live browser.'}
        >
          <Icon name="globe" size={13} />
        </button>
      </div>
      {#if nativeBrowserAvailable && browser.activeTab.mode === 'live'}
        <button
          class="icon-btn tool"
          onclick={chooseRenderer}
          aria-label="Live engine"
          title={browserLive.renderer === 'native'
            ? "Live engine: this Mac's web view"
            : "Live engine: Otto's Chromium"}
        >
          <Icon name="chevronDown" size={14} />
        </button>
      {/if}
    {/if}
    {#if activeLive}
      <button
        class="icon-btn tool"
        class:active={pickMode}
        onclick={togglePick}
        title={pickMode ? 'Picking… click an element in the page' : 'Pick an element to mark'}
        aria-label="Pick an element to mark"
        aria-pressed={pickMode}
      >
        <Icon name="target" size={14} />
      </button>
    {/if}
    {#if canAutofill}
      <button
        class="icon-btn tool"
        onclick={autofill}
        disabled={filling}
        aria-label="Autofill saved credentials"
        title={`Autofill ${matchedCredential?.username ?? ''} for ${matchedCredential?.domain ?? ''}`}
      >
        <Icon name="key" size={14} />
      </button>
    {/if}
    <button
      class="icon-btn tool"
      onclick={doSummarize}
      disabled={!browser.activeTab || summarizing}
      aria-label="Summarize"
      title="Summarize"
    >
      <Icon name="zap" size={14} />
    </button>
    <button
      class="icon-btn tool"
      onclick={doVaultSave}
      disabled={!browser.activeTab || vaultSaving}
      aria-label="Save to vault"
      title="Save to vault"
    >
      <Icon name="folder" size={14} />
    </button>
  </div>

  {#if summary}
    <div class="summary">
      <div class="summary-head">
        <span>Summary</span>
        <button class="icon-btn" onclick={() => (summary = '')} aria-label="Close summary" title="Close summary"><Icon name="x" size={12} /></button>
      </div>
      <p>{summary}</p>
    </div>
  {/if}

  <div class="body">
    {#if activeLive}
      <!-- The native child webview paints ABOVE this div's rect — the div
           itself just holds the geometry the ResizeObserver above tracks. -->
      <div class="live-host" bind:this={liveHostEl}></div>
    {:else if activeRemote && browserLive.ready}
      <RemoteLiveView
        bind:this={remoteView}
        tab={activeRemote}
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
        <div class="live-note" role="status">
          <Icon name="globe" size={13} />
          <span>This tab is in live mode, but this Otto daemon has no live browser. You're seeing Reader view here.</span>
          <span class="grow"></span>
          <button class="btn" onclick={openInOwnBrowser}>Open in new tab</button>
          <button class="btn ghost" onclick={() => browser.activeTab && void browser.setMode(browser.activeTab.id, 'reader')}>Switch to Reader</button>
        </div>
      {/if}
      <ReaderView
        page={browser.page}
        loading={browser.loadingPage}
        error={browser.pageError}
        onretry={() => {
          const url = browser.activeTab?.url;
          if (url) void browser.loadPage(url);
        }}
      />
      {#if browser.page}
        <NotesRail annotations={browser.annotations} />
      {/if}
    {/if}
  </div>

  {#if embedded}
    <AskBar sessionId={targetSessionId} unboundHint="Open an agent session to ask about this page." />
  {:else}
    <AgentDock />
  {/if}
</div>

<style>
  .browser {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .body {
    flex: 1;
    display: flex;
    min-height: 0;
    position: relative;
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
    padding: 6px 8px 6px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--info-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .live-note .grow {
    flex: 1;
  }
  .live-host {
    flex: 1;
    min-height: 0;
    /* Empty on purpose — the Tauri child webview paints natively above this
       rect; ResizeObserver above keeps its bounds synced to this div. */
  }
  .mode-toggle {
    display: flex;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    overflow: hidden;
  }
  .seg {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 30px;
    border: none;
    background: var(--surface);
    color: var(--text-dim);
    cursor: pointer;
  }
  .seg + .seg {
    border-inline-start: 1px solid var(--border);
  }
  .seg:hover {
    color: var(--text);
  }
  .seg.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent-text);
  }
  .urlbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    min-width: 0;
  }
  .urlbar input {
    flex: 1;
    min-width: 0;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px 10px;
    font: inherit;
    font-size: var(--fs-m);
  }
  /* Urlbar tools: the shared .icon-btn, framed like the address field. */
  .icon-btn.tool {
    flex: none;
    width: 30px;
    height: 30px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
  }
  .icon-btn.tool:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .icon-btn.tool.active {
    background: var(--accent-soft);
    color: var(--accent-text);
    border-color: var(--accent);
  }
  @media (max-width: 640px) {
    .urlbar {
      flex-wrap: wrap;
    }
    .urlbar input {
      flex-basis: 100%;
      order: -1;
    }
    .icon-btn.tool {
      width: 36px;
      height: 36px;
    }
  }
  .summary {
    margin: 10px 12px 0;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: var(--fs-m);
  }
  .summary-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.06em;
    margin-bottom: 6px;
  }
  .summary p {
    /* A long summary scrolls in place instead of shoving the page off-screen. */
    max-height: 30vh;
    overflow-y: auto;
    margin: 0;
    color: var(--text);
    line-height: 1.5;
  }
</style>
