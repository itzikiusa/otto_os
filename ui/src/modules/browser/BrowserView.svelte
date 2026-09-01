<script lang="ts">
  // Browser module page: tab strip + URL bar + reader-mode page. Mirrors the
  // shape of VaultPage/LoopsPage (a thin view over a $state store).

  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { browser } from '../../lib/stores/browser.svelte';
  import { vault } from '../vault/vault.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { nativeBrowser, nativeBrowserAvailable, type Rect } from '../../lib/nativeBrowser';
  import Icon from '../../lib/components/Icon.svelte';
  import TabStrip from './TabStrip.svelte';
  import ReaderView from './ReaderView.svelte';
  import NotesRail from './NotesRail.svelte';

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

  // ── Live tab: native (Tauri) child webview overlaid on `liveHostEl` ────────
  // Off Tauri (remote/PWA), a tab flagged mode:"live" simply falls back to
  // reader — `nativeBrowserAvailable` is false there so none of this fires.
  let liveHostEl = $state<HTMLDivElement | null>(null);
  const activeLive = $derived(
    nativeBrowserAvailable && browser.activeTab?.mode === 'live' ? browser.activeTab : null,
  );
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
      void nativeBrowser.open(active.id, active.url, r); // create-or-navigate + show
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

  function toggleMode(mode: 'reader' | 'live'): void {
    const tab = browser.activeTab;
    if (tab) void browser.setMode(tab.id, mode);
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
  <TabStrip onnew={newTab} />

  <div class="urlbar">
    <input
      type="text"
      placeholder="Enter URL"
      bind:value={urlInput}
      onkeydown={onkeydown}
      onfocus={() => (urlFocused = true)}
      onblur={() => (urlFocused = false)}
    />
    <button class="btn" onclick={go} title="Go">
      <Icon name="external" size={14} />
    </button>
    {#if browser.activeTab}
      <div class="mode-toggle" role="group" aria-label="Tab mode">
        <button
          class="seg"
          class:active={browser.activeTab.mode === 'reader'}
          onclick={() => toggleMode('reader')}
          title="Reader mode — fetched and rendered as clean markdown"
        >
          <Icon name="file" size={13} />
        </button>
        <button
          class="seg"
          class:active={browser.activeTab.mode === 'live'}
          onclick={() => toggleMode('live')}
          title={nativeBrowserAvailable
            ? 'Live mode — a real embedded browser'
            : 'Live mode needs the Otto desktop app — falls back to reader here'}
        >
          <Icon name="globe" size={13} />
        </button>
      </div>
    {/if}
    <button
      class="btn"
      onclick={doSummarize}
      disabled={!browser.activeTab || summarizing}
      title="Summarize"
    >
      <Icon name="zap" size={14} />
    </button>
    <button
      class="btn"
      onclick={doVaultSave}
      disabled={!browser.activeTab || vaultSaving}
      title="Save to vault"
    >
      <Icon name="folder" size={14} />
    </button>
  </div>

  {#if summary}
    <div class="summary">
      <div class="summary-head">
        <span>Summary</span>
        <button class="close" onclick={() => (summary = '')}><Icon name="x" size={12} /></button>
      </div>
      <p>{summary}</p>
    </div>
  {/if}

  <div class="body">
    {#if activeLive}
      <!-- The native child webview paints ABOVE this div's rect — the div
           itself just holds the geometry the ResizeObserver above tracks. -->
      <div class="live-host" bind:this={liveHostEl}></div>
    {:else}
      <ReaderView page={browser.page} loading={browser.loadingPage} error={browser.pageError} />
      {#if browser.page}
        <NotesRail annotations={browser.annotations} />
      {/if}
    {/if}
  </div>
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
    border-left: 1px solid var(--border);
  }
  .seg:hover {
    color: var(--text);
  }
  .seg.active {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
  }
  .urlbar {
    display: flex;
    align-items: center;
    gap: 0.4rem;
    padding: 0.5rem 0.75rem;
    border-bottom: 1px solid var(--border);
  }
  .urlbar input {
    flex: 1;
    background: var(--bg);
    color: var(--text);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 0.4rem 0.6rem;
    font: inherit;
    font-size: 0.85rem;
  }
  .btn {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 30px;
    height: 30px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    cursor: pointer;
  }
  .btn:hover:not(:disabled) {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }
  .summary {
    margin: 0.6rem 0.75rem 0;
    padding: 0.6rem 0.75rem;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    font-size: 0.85rem;
  }
  .summary-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    color: var(--text-dim);
    font-size: 0.75rem;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    margin-bottom: 0.35rem;
  }
  .summary p {
    margin: 0;
    color: var(--text);
    line-height: 1.5;
  }
  .close {
    display: flex;
    background: transparent;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
  }
</style>
