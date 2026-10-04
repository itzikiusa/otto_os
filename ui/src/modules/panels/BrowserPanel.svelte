<script lang="ts">
  // Browser tab: a real inline browser with TABS. Each tab is its own native
  // child webview (Tauri), so switching tabs is instant and preserves the page's
  // scroll/form/login state. A `window.open()` / `target=_blank` inside a tab is
  // intercepted natively (no OS popup) and surfaced as `otto://browser-new-tab`,
  // which opens a real in-app tab here and focuses it.
  //
  // On the plain web build (no native webview) each tab falls back to a single
  // <iframe>; sites that send X-Frame-Options refuse to frame — use "Open
  // externally" for those.
  //
  // "Take over" mode reloads the active tab via the daemon's proxy endpoint so a
  // picker script is injected; clicking elements captures a CSS-selector
  // description, the user comments, and all comments are sent to the active agent.
  import { tick } from 'svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { openExternal as openExternalUrl } from '../../lib/external';
  import { nativeBrowser, nativeBrowserAvailable } from '../../lib/nativeBrowser';
  import { api, baseUrl, getToken } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { AttachedIssue } from '../../lib/api/types';

  // `active` = this panel is the one on screen. The right panel keeps the
  // browser MOUNTED while another tab (or the collapsed strip) is showing, so
  // its tabs, pages and take-over annotations survive a tab switch; a native
  // webview always paints above the HTML, so a hidden panel must hide them.
  let { active = true }: { active?: boolean } = $props();

  const session = $derived(ws.activeSession);
  const attachedIssue = $derived(
    (session?.meta?.issue as AttachedIssue | undefined) ?? null,
  );

  // ── Tabs ────────────────────────────────────────────────────────────────────
  type Tab = { id: string; url: string; title: string };
  let tabSeq = 0;
  let tabs = $state<Tab[]>([{ id: 't0', url: '', title: 'New tab' }]);
  let activeId = $state('t0');
  // Per-tab last-navigated URL. The native webview is told to navigate ONLY when
  // a tab's url actually changes; switching tabs just show/hides, so each tab's
  // page stays live (state preserved). NOT reactive — bookkeeping only.
  const openedUrl: Record<string, string> = {};

  const activeTab = $derived(tabs.find((t) => t.id === activeId) ?? null);

  // The tab list scrolls horizontally with its scrollbar hidden — keep the
  // active tab (e.g. one a page just opened) scrolled into view.
  let stripEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const id = activeId;
    const el = stripEl?.querySelector<HTMLElement>(`[data-tab-id="${id}"]`);
    el?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  });
  const current = $derived(activeTab?.url ?? ''); // active tab's loaded URL

  let urlInput = $state('');
  let reloadTick = $state(0); // bump to force the iframe (web build / take-over) to reload

  // ── Take-over state ────────────────────────────────────────────────────────
  let takeover = $state(false);
  type Annotation = { desc: string; comment: string; url: string };
  let annotations = $state<Annotation[]>([]);

  type Popover = { open: boolean; x: number; y: number; desc: string; url: string };
  let popover = $state<Popover>({ open: false, x: 0, y: 0, desc: '', url: '' });
  let popoverComment = $state('');
  let browserEl = $state<HTMLDivElement | null>(null);
  let popEl = $state<HTMLDivElement | null>(null);
  let takeoverBtnEl = $state<HTMLButtonElement | null>(null);

  // The iframe DOM node — bound below with bind:this (web build / take-over only)
  let frame = $state<HTMLIFrameElement | null>(null);

  // ── Native browser (Tauri child webview, one per tab) ───────────────────────
  let hostEl = $state<HTMLDivElement | null>(null);
  let urlFocused = $state(false);
  const useNative = $derived(nativeBrowserAvailable && !takeover);

  function hostRect(): { x: number; y: number; width: number; height: number } | null {
    if (!hostEl) return null;
    const r = hostEl.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return null;
    // The native WKWebView page-zoom magnifies the whole SPA from the window's
    // top-left WITHOUT reflowing, so getBoundingClientRect() (CSS px) maps to
    // window-logical points by × the zoom factor. The child webview is positioned
    // in window-logical points, so scale the rect — otherwise it's mis-aligned
    // (too small → desktop shows through; too big → spills over) at zoom ≠ 1.
    const z = ui.zoom || 1;
    return { x: r.left * z, y: r.top * z, width: r.width * z, height: r.height * z };
  }

  // ── Tab helpers ─────────────────────────────────────────────────────────────
  function makeTitle(url: string): string {
    if (!url) return 'New tab';
    try {
      const u = new URL(url);
      const tail = u.pathname.replace(/\/+$/, '').split('/').filter(Boolean).pop();
      return tail || u.hostname;
    } catch {
      return url;
    }
  }

  function newTab(url = ''): string {
    const id = `t${++tabSeq}`;
    tabs = [...tabs, { id, url, title: makeTitle(url) }];
    activeId = id;
    urlInput = url;
    takeover = false;
    return id;
  }

  function setActiveTab(id: string): void {
    if (id === activeId) return;
    activeId = id;
    const t = tabs.find((x) => x.id === id);
    urlInput = t?.url ?? '';
    takeover = false;
    popover = { open: false, x: 0, y: 0, desc: '', url: '' };
  }

  function closeTab(id: string): void {
    const idx = tabs.findIndex((t) => t.id === id);
    if (idx < 0) return;
    if (nativeBrowserAvailable) void nativeBrowser.close(id);
    delete openedUrl[id];
    const remaining = tabs.filter((t) => t.id !== id);
    if (remaining.length === 0) {
      // Never leave zero tabs — replace with a fresh blank one.
      tabs = [];
      newTab('');
      return;
    }
    tabs = remaining;
    if (activeId === id) {
      const next = remaining[Math.min(idx, remaining.length - 1)];
      activeId = next.id;
      urlInput = next.url;
      takeover = false;
    }
  }

  // ── Native webview drivers ──────────────────────────────────────────────────
  // Show the active tab's webview over the host rect; hide every other tab.
  // A native webview always paints above the HTML, so it must also hide for SPA
  // overlays (palette / modals / context menus) and take-over / start pages.
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    const list = tabs; // reactive dep
    const tab = activeTab; // reactive dep
    const overlay = ui.overlayOpen || ctxMenu.open;
    const showActive = active && useNative && !!tab && !!tab.url && !overlay;
    for (const t of list) {
      if (!showActive || !tab || t.id !== tab.id) void nativeBrowser.hide(t.id);
    }
    if (!showActive || !tab) return;
    const r = hostRect();
    if (!r) return;
    if (openedUrl[tab.id] !== tab.url) {
      openedUrl[tab.id] = tab.url;
      void nativeBrowser.open(tab.id, tab.url, r); // create-or-navigate + show
    } else {
      void nativeBrowser.bounds(tab.id, r);
      void nativeBrowser.show(tab.id);
    }
  });

  // Keep the active tab's webview aligned with the panel as it resizes / moves.
  $effect(() => {
    if (!active || !nativeBrowserAvailable || !useNative || !activeTab?.url || !hostEl) return;
    const id = activeId;
    const _z = ui.zoom; // re-align immediately when the page zoom changes
    // Event-driven (perf P14): the host's own size (panel drag, tab bar), the
    // app's size (anything reflowing the shell), window resize and the end of
    // a layout transition — each coalesced to one rect read per frame, and the
    // bounds IPC only sent when the rect actually moved. This replaced a 400 ms
    // forever-interval that re-read layout and sent an IPC every tick; a 2 s
    // visible-only check remains for pure position drift nothing observes.
    let last = '';
    let frame = 0;
    const apply = (): void => {
      frame = 0;
      const r = hostRect();
      if (!r) return;
      const key = `${r.x},${r.y},${r.width},${r.height}`;
      if (key === last) return;
      last = key;
      void nativeBrowser.bounds(id, r);
    };
    const sync = (): void => {
      if (!frame) frame = requestAnimationFrame(apply);
    };
    const ro = new ResizeObserver(sync);
    ro.observe(hostEl);
    ro.observe(document.body);
    window.addEventListener('resize', sync);
    window.addEventListener('transitionend', sync, true);
    const iv = setInterval(() => {
      if (document.visibilityState === 'visible') apply();
    }, 2000);
    apply();
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', sync);
      window.removeEventListener('transitionend', sync, true);
      clearInterval(iv);
      if (frame) cancelAnimationFrame(frame);
    };
  });

  // Reflect a tab's in-page navigations into its stored URL + the address bar.
  // Event-driven (wry on_navigation) — never polls url(), which panics on a
  // webview that hasn't committed a load yet (and poisons a shared lock).
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    let unlisten = (): void => {};
    let disposed = false;
    void nativeBrowser
      .onUrlChange((id, url) => {
        // Keep openedUrl in sync so the driver effect doesn't re-navigate (loop).
        openedUrl[id] = url;
        const t = tabs.find((x) => x.id === id);
        if (t && t.url !== url) {
          tabs = tabs.map((x) => (x.id === id ? { ...x, url, title: makeTitle(url) } : x));
        }
        if (!urlFocused && id === activeId && url !== urlInput) urlInput = url;
      })
      .then((un) => (disposed ? un() : (unlisten = un)));
    return () => {
      disposed = true;
      unlisten();
    };
  });

  // A page asked to open a new tab (window.open / target=_blank) — open + focus it.
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    let unlisten = (): void => {};
    let disposed = false;
    void nativeBrowser
      .onNewTab((url) => newTab(url))
      .then((un) => {
        if (disposed) un();
        else unlisten = un;
      });
    return () => {
      disposed = true;
      unlisten();
    };
  });

  // Tear every tab's webview down when the panel/tab unmounts.
  $effect(() => () => {
    if (nativeBrowserAvailable) void nativeBrowser.closeAll();
  });

  // ── Derived iframe src (web build / take-over) ──────────────────────────────
  const frameSrc = $derived(
    current
      ? takeover
        ? `${baseUrl()}/browser/proxy?url=${encodeURIComponent(current)}&token=${encodeURIComponent(getToken() ?? '')}`
        : current
      : '',
  );

  // ── Helpers ────────────────────────────────────────────────────────────────
  function normalize(u: string): string {
    const t = u.trim();
    if (!t) return '';
    return /^[a-z]+:\/\//i.test(t) ? t : `https://${t}`;
  }

  // Load a URL into the ACTIVE tab.
  function load(u: string): void {
    const href = normalize(u);
    if (!href) return;
    if (!activeTab) {
      newTab(href);
      reloadTick++;
      return;
    }
    urlInput = href;
    tabs = tabs.map((t) =>
      t.id === activeId ? { ...t, url: href, title: makeTitle(href) } : t,
    );
    reloadTick++;
  }

  function reload(): void {
    if (!current) return;
    if (useNative) void nativeBrowser.reload(activeId);
    else reloadTick++;
  }

  // Reset the ACTIVE tab to its start page.
  function home(): void {
    takeover = false;
    annotations = [];
    popover = { open: false, x: 0, y: 0, desc: '', url: '' };
    if (!activeTab) return;
    delete openedUrl[activeId];
    tabs = tabs.map((t) => (t.id === activeId ? { ...t, url: '', title: 'New tab' } : t));
    urlInput = '';
    if (nativeBrowserAvailable) void nativeBrowser.hide(activeId);
  }

  function openExternal(u: string): void {
    // Route through the shell `open` command (Tauri); plain anchors with
    // target=_blank don't reach the system browser inside the webview.
    void openExternalUrl(normalize(u));
  }

  // Toggle the web inspector (console / network / elements) for the active tab.
  function toggleDevtools(): void {
    if (nativeBrowserAvailable && activeTab?.url) void nativeBrowser.devtools(activeId);
  }

  function onEnter(e: KeyboardEvent): void {
    if (e.key === 'Enter') load(urlInput);
  }

  // ── Take-over toggle ───────────────────────────────────────────────────────
  function toggleTakeover(): void {
    if (takeover) releaseTakeover();
    else takeover = true;
  }

  function releaseTakeover(): void {
    takeover = false;
    popover = { open: false, x: 0, y: 0, desc: '', url: '' };
  }

  // Tell the iframe's injected picker to enable/disable the crosshair.
  function syncTakeoverMessage(): void {
    frame?.contentWindow?.postMessage({ type: 'otto-takeover', enabled: takeover }, '*');
  }

  function onFrameLoad(): void {
    syncTakeoverMessage();
  }

  // ── postMessage listener for picker events from the proxy iframe ──────────
  $effect(() => {
    function onMessage(ev: MessageEvent): void {
      if (!ev.data || typeof ev.data !== 'object') return;

      if (ev.data.type === 'otto-element' && takeover) {
        // Only the take-over frame's own picker may open the comment popover.
        if (!frame || ev.source !== frame.contentWindow) return;
        const { desc, x, y, url } = ev.data as {
          desc: string;
          x: number;
          y: number;
          url: string;
        };

        // x/y are the click's clientX/Y inside the iframe; the popover is
        // positioned in .browser, where the frame starts below the tab strip
        // and toolbar — offset by the frame's position, then clamp to it.
        const left = frame.offsetLeft;
        const top = frame.offsetTop;
        const maxX = left + Math.max(0, frame.clientWidth - 320);
        const maxY = top + Math.max(0, frame.clientHeight - 200);

        popoverComment = '';
        popover = {
          open: true,
          x: Math.min(Math.max(left, left + x), maxX),
          y: Math.min(Math.max(top, top + y), maxY),
          desc,
          url,
        };
      }
    }

    window.addEventListener('message', onMessage);
    return () => window.removeEventListener('message', onMessage);
  });

  // Re-sync the take-over message whenever the mode changes.
  $effect(() => {
    const _t = takeover;
    syncTakeoverMessage();
  });

  // Clamp the popover into the panel using its REAL size (the estimate above
  // can't know how tall the description + textarea render) — a popover near the
  // bottom/inline-end edge would otherwise hang off the panel.
  $effect(() => {
    if (!popover.open || !popEl || !browserEl) return;
    const pad = 8;
    const maxX = Math.max(pad, browserEl.clientWidth - popEl.offsetWidth - pad);
    const maxY = Math.max(pad, browserEl.clientHeight - popEl.offsetHeight - pad);
    const x = Math.min(Math.max(pad, popover.x), maxX);
    const y = Math.min(Math.max(pad, popover.y), maxY);
    if (x !== popover.x || y !== popover.y) popover = { ...popover, x, y };
  });

  // Closing the popover with Esc hands focus back to the take-over toggle (the
  // control that opened this mode) instead of dropping it on <body>.
  function closePopover(): void {
    popover = { ...popover, open: false };
    void tick().then(() => takeoverBtnEl?.focus());
  }

  // ── Global Esc handler ────────────────────────────────────────────────────
  $effect(() => {
    function onKeydown(e: KeyboardEvent): void {
      if (e.key === 'Escape') {
        if (popover.open) closePopover();
        else if (takeover) releaseTakeover();
      }
    }
    window.addEventListener('keydown', onKeydown);
    return () => window.removeEventListener('keydown', onKeydown);
  });

  // ── Popover: "Add" button ─────────────────────────────────────────────────
  function addAnnotation(): void {
    if (!popover.open) return;
    annotations = [
      ...annotations,
      { desc: popover.desc, comment: popoverComment, url: popover.url },
    ];
    popover = { ...popover, open: false };
    popoverComment = '';
  }

  function popoverKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) addAnnotation();
  }

  // ── Send to agent ─────────────────────────────────────────────────────────
  async function sendToAgent(): Promise<void> {
    if (!ws.activeSessionId || ws.activeSession?.kind !== 'agent') {
      toasts.error('No agent session', 'Open an agent session to receive the feedback.');
      return;
    }

    const n = annotations.length;
    const urlRef = annotations[0]?.url ?? current;
    const lines = annotations
      .map((a, i) => `${i + 1}. ${a.desc} — ${a.comment}`)
      .join('\n');

    const text =
      `User left ${n} comment(s) while reviewing ${urlRef}:\n\n` +
      `${lines}\n\n` +
      `Please inspect each and propose fixes.`;

    try {
      await api.post(`/sessions/${ws.activeSessionId}/input`, { text, submit: true });
      annotations = [];
      toasts.success('Sent to agent', `${n} comment(s)`);
    } catch {
      toasts.error('Failed to send', 'Could not inject message into the agent session.');
    }
  }
</script>

<div class="browser" bind:this={browserEl}>
  <!-- ── Tab strip ─────────────────────────────────────────────────────────── -->
  <!-- The "+" sits OUTSIDE the scrolling tab list so it can never be scrolled
       or clipped out of reach when there are many tabs. -->
  <div class="tabstrip-row">
    <div class="tabstrip" role="tablist" aria-label="Browser tabs" bind:this={stripEl}>
      {#each tabs as t (t.id)}
        <!-- The tab and its close are TWO real buttons (no control nested in a
             role=tab); ←/→/Home/End move between tabs (roving tabindex). -->
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="btab"
          class:active={t.id === activeId}
          role="presentation"
          data-tab-id={t.id}
          onauxclick={(e) => {
            if (e.button === 1) {
              e.preventDefault();
              closeTab(t.id);
            }
          }}
        >
          <button
            class="btab-main"
            role="tab"
            aria-selected={t.id === activeId}
            tabindex={t.id === activeId ? 0 : -1}
            title={t.url || 'New tab'}
            onclick={() => setActiveTab(t.id)}
            onkeydown={onTabKey}
          >
            <span class="btab-title">{t.title}</span>
          </button>
          <button
            class="btab-close"
            title="Close tab"
            aria-label="Close {t.title || 'tab'}"
            onclick={(e) => {
              e.stopPropagation();
              closeTab(t.id);
            }}
          >
            <Icon name="x" size={9} />
          </button>
        </div>
      {/each}
    </div>
    <button class="btab-new" title="New tab" aria-label="New tab" onclick={() => newTab('')}>
      <Icon name="plus" size={12} />
    </button>
  </div>

  <!-- ── Toolbar ──────────────────────────────────────────────────────────── -->
  <div class="toolbar">
    <button class="icon-btn" title="Reload" aria-label="Reload" disabled={!current} onclick={reload}>
      <Icon name="refresh" size={13} />
    </button>
    <button class="icon-btn" title="Start page" aria-label="Start page" disabled={!current} onclick={home}>
      <Icon name="home" size={13} />
    </button>
    <input
      class="input url-input"
      bind:value={urlInput}
      placeholder="Search or enter URL…"
      spellcheck="false"
      autocomplete="off"
      onfocus={() => (urlFocused = true)}
      onblur={() => (urlFocused = false)}
      onkeydown={onEnter}
    />
    <button class="icon-btn" title="Go" aria-label="Go" disabled={!urlInput.trim()} onclick={() => load(urlInput)}>
      <Icon name="chevronRight" size={13} />
    </button>

    <!-- Take-over toggle -->
    <button
      class="icon-btn takeover-btn"
      bind:this={takeoverBtnEl}
      class:takeover-active={takeover}
      title={takeover
        ? 'Release (Esc)'
        : 'Take over — click elements to comment & send to the agent'}
      aria-label={takeover ? 'Release take-over' : 'Take over'}
      aria-pressed={takeover}
      disabled={!current}
      onclick={toggleTakeover}
    >
      <Icon name="cursor" size={13} />
    </button>

    {#if nativeBrowserAvailable}
      <button
        class="icon-btn"
        title="DevTools — console, network, elements"
        aria-label="DevTools"
        disabled={!current}
        onclick={toggleDevtools}
      >
        <Icon name="terminal" size={13} />
      </button>
    {/if}

    <button
      class="icon-btn"
      title="Open in system browser"
      aria-label="Open in system browser"
      disabled={!(current || urlInput.trim())}
      onclick={() => openExternal(current || urlInput)}
    >
      <Icon name="external" size={12} />
    </button>
  </div>

  {#if current}
    {#if useNative}
      <!-- A native child webview (the active tab) is positioned over this host. -->
      <div class="frame native-host" bind:this={hostEl}></div>
    {:else}
      <!-- key forces a full iframe reload when frameSrc changes (proxy ↔ direct) -->
      {#key frameSrc + '#' + reloadTick}
        <iframe
          bind:this={frame}
          class="frame"
          class:takeover-cursor={takeover}
          src={frameSrc}
          title="Browser"
          referrerpolicy="no-referrer"
          onload={onFrameLoad}
        ></iframe>
      {/key}
    {/if}

    <!-- Comment popover (absolutely positioned within .browser) -->
    {#if popover.open}
      <div
        class="popover"
        bind:this={popEl}
        style="left:{popover.x}px; top:{popover.y}px;"
        role="dialog"
        aria-label="Add comment"
        tabindex="-1"
        onkeydown={popoverKeydown}
      >
        <div class="popover-desc" title={popover.desc}>{popover.desc}</div>
        <!-- svelte-ignore a11y_autofocus -->
        <textarea
          class="input popover-textarea"
          bind:value={popoverComment}
          placeholder="Your comment…"
          rows={3}
          onkeydown={popoverKeydown}
          autofocus
        ></textarea>
        <div class="popover-actions">
          <button class="btn" onclick={closePopover}>
            Cancel
          </button>
          <button class="btn primary" disabled={!popoverComment.trim()} onclick={addAnnotation}>
            Add comment
          </button>
        </div>
      </div>
    {/if}

    <!-- Annotation badge + "Send to agent" -->
    {#if annotations.length > 0}
      <div class="annot-badge">
        <span class="annot-count">{annotations.length} marked</span>
        <button class="btn primary small" onclick={sendToAgent}>
          Send {annotations.length} to agent
        </button>
        <button
          class="btn small"
          title="Clear all annotations"
          onclick={() => (annotations = [])}
        >
          Clear
        </button>
      </div>
    {/if}

    <div class="frame-foot">
      <span class="dim ellipsis" title={current}>{current}</span>
      <button class="link" onclick={() => openExternal(current)}>
        <Icon name="external" size={11} /> Open externally
      </button>
    </div>
  {:else}
    <div class="start">
      <p class="hint">
        {#if nativeBrowserAvailable}
          Enter a URL above to browse any site here — including ones that block
          embedding (Google, Jira, GitHub) and local dev servers. Links that open
          in a new tab open here as a new tab. Use Open in system browser (the
          arrow button in the toolbar) for anything that should leave Otto.
        {:else}
          Enter a URL above to browse it here. Sites that block embedding (Jira,
          Google, GitHub) won't load here — open them with Open in system
          browser (the arrow button in the toolbar).
        {/if}
      </p>

      {#if attachedIssue}
        <section class="section">
          <div class="section-title">Attached issue</div>
          <button class="quick-link" onclick={() => openExternal(attachedIssue.url)}>
            <Icon name="ticket" size={13} />
            <div class="ql-text">
              <span class="ql-key">{attachedIssue.key}</span>
              <span class="ql-label">{attachedIssue.summary}</span>
            </div>
            <Icon name="external" size={12} />
          </button>
        </section>
      {/if}

      <section class="section">
        <div class="section-title">Quick links</div>
        <button
          class="quick-link"
          onclick={() => openExternal('https://id.atlassian.com/manage-profile/security/api-tokens')}
        >
          <Icon name="key" size={13} />
          <span class="ql-label">Atlassian API tokens</span>
          <Icon name="external" size={12} />
        </button>
      </section>
    </div>
  {/if}
</div>

<style>
  .browser {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    position: relative; /* anchor for the popover */
  }

  /* ── Tab strip ───────────────────────────────────────────────────────────── */
  .tabstrip-row {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 6px 0 6px;
    flex-shrink: 0;
    min-width: 0;
  }
  .tabstrip {
    display: flex;
    align-items: center;
    gap: 2px;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tabstrip::-webkit-scrollbar {
    display: none;
  }
  .btab {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 160px;
    height: 26px;
    padding-block: 0;
    padding-inline: 10px 4px;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: var(--radius-s) var(--radius-s) 0 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
    cursor: pointer;
    white-space: nowrap;
    transition: background 120ms ease-out, color 120ms ease-out;
  }
  .btab:hover {
    background: var(--surface-2);
  }
  .btab.active {
    background: var(--surface);
    border-color: var(--border);
    color: var(--text);
  }
  .btab-main {
    flex: 1 1 auto;
    display: flex;
    align-items: center;
    align-self: stretch;
    min-width: 0;
    padding: 0;
    border: none;
    background: transparent;
    color: inherit;
    font: inherit;
    cursor: pointer;
  }
  .btab-title {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .btab-close {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
    transition: opacity 120ms ease-out, background 120ms ease-out;
  }
  .btab:hover .btab-close,
  .btab:focus-within .btab-close,
  .btab.active .btab-close {
    opacity: 1;
  }
  /* Touch has no hover: the close control must always be visible. */
  @media (hover: none) {
    .btab-close {
      opacity: 1;
    }
  }
  .btab-close:hover {
    background: var(--hover);
    color: var(--text);
  }
  .btab-new {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    flex-shrink: 0;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
  }
  .btab-new:hover {
    background: var(--surface-2);
    color: var(--text);
  }

  .toolbar {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 8px 8px;
    border-bottom: 1px solid var(--border);
    border-top: 1px solid var(--border);
    flex-shrink: 0;
  }
  .url-input {
    flex: 1;
    min-width: 0;
    height: 28px;
    font-size: var(--fs-s);
  }
  /* Take-over toggle highlighted state */
  .takeover-btn.takeover-active {
    background: var(--accent-soft);
    color: var(--accent-text);
  }

  .frame {
    flex: 1;
    min-height: 0;
    width: 100%;
    border: none;
    background: #fff;
  }
  /* Crosshair cursor hint while take-over is on */
  .frame.takeover-cursor {
    cursor: crosshair;
  }

  .frame-foot {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 5px 10px;
    border-top: 1px solid var(--border);
    flex-shrink: 0;
  }
  .frame-foot .dim {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .link {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    border: none;
    background: transparent;
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .ellipsis {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .start {
    padding: 12px 10px;
    display: flex;
    flex-direction: column;
    gap: 16px;
    overflow-y: auto;
  }
  .hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .section-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.07em;
    color: var(--text-dim);
    padding-bottom: 4px;
    border-bottom: 1px solid var(--border);
  }
  .quick-link {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    cursor: pointer;
    text-align: start;
    transition: background 120ms ease-out;
    color: var(--text);
    width: 100%;
  }
  .quick-link:hover {
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    border-color: color-mix(in srgb, var(--accent) 35%, transparent);
  }
  .ql-text {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
  }
  .ql-key {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--accent-text);
  }
  .ql-label {
    font-size: var(--fs-s);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
  }

  /* ── Comment popover ───────────────────────────────────────────────────── */
  .popover {
    position: absolute;
    z-index: var(--z-sticky);
    width: 300px;
    max-width: calc(100% - 16px);
    max-height: calc(100% - 16px);
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    box-shadow: var(--glass-shadow);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .popover-desc {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--accent-text);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    padding: 4px 6px;
    border-radius: var(--radius-s);
    border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .popover-textarea {
    width: 100%;
    resize: vertical;
    font-size: var(--fs-s);
    min-height: 64px;
    box-sizing: border-box;
  }
  .popover-actions {
    display: flex;
    justify-content: flex-end;
    gap: 6px;
  }

  /* ── Annotation badge ──────────────────────────────────────────────────── */
  .annot-badge {
    position: absolute;
    bottom: 34px; /* just above frame-foot */
    inset-inline-end: 10px;
    z-index: var(--z-sticky);
    display: flex;
    align-items: center;
    gap: 6px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 5px 8px;
    box-shadow: var(--glass-shadow);
  }
  .annot-count {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
</style>
