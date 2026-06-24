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
  import { ws } from '../../lib/stores/workspace.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { openExternal as openExternalUrl } from '../../lib/external';
  import { nativeBrowser, nativeBrowserAvailable } from '../../lib/nativeBrowser';
  import { api, baseUrl, getToken } from '../../lib/api/client';
  import { toasts } from '../../lib/toast.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import type { AttachedIssue } from '../../lib/api/types';

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
    const showActive = useNative && !!tab && !!tab.url && !overlay;
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
    if (!nativeBrowserAvailable || !useNative || !activeTab?.url || !hostEl) return;
    const id = activeId;
    const _z = ui.zoom; // re-align immediately when the page zoom changes
    const sync = (): void => {
      const r = hostRect();
      if (r) void nativeBrowser.bounds(id, r);
    };
    const ro = new ResizeObserver(sync);
    ro.observe(hostEl);
    window.addEventListener('resize', sync);
    const iv = setInterval(sync, 400); // catches position drift (panel drag, layout)
    sync();
    return () => {
      ro.disconnect();
      window.removeEventListener('resize', sync);
      clearInterval(iv);
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
        const { desc, x, y, url } = ev.data as {
          desc: string;
          x: number;
          y: number;
          url: string;
        };

        const maxX = Math.max(0, (frame?.clientWidth ?? 400) - 320);
        const maxY = Math.max(0, (frame?.clientHeight ?? 600) - 200);

        popoverComment = '';
        popover = {
          open: true,
          x: Math.min(Math.max(0, x), maxX),
          y: Math.min(Math.max(0, y), maxY),
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

  // ── Global Esc handler ────────────────────────────────────────────────────
  $effect(() => {
    function onKeydown(e: KeyboardEvent): void {
      if (e.key === 'Escape') {
        if (popover.open) popover = { ...popover, open: false };
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

<div class="browser">
  <!-- ── Tab strip ─────────────────────────────────────────────────────────── -->
  <div class="tabstrip" role="tablist">
    {#each tabs as t (t.id)}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div
        class="btab"
        class:active={t.id === activeId}
        role="tab"
        tabindex="0"
        aria-selected={t.id === activeId}
        title={t.url || 'New tab'}
        onclick={() => setActiveTab(t.id)}
        onkeydown={(e) => e.key === 'Enter' && setActiveTab(t.id)}
        onauxclick={(e) => {
          if (e.button === 1) {
            e.preventDefault();
            closeTab(t.id);
          }
        }}
      >
        <span class="btab-title">{t.title}</span>
        <button
          class="btab-close"
          title="Close tab"
          aria-label="Close tab"
          onclick={(e) => {
            e.stopPropagation();
            closeTab(t.id);
          }}
        >
          <Icon name="x" size={9} />
        </button>
      </div>
    {/each}
    <button class="btab-new" title="New tab" aria-label="New tab" onclick={() => newTab('')}>
      <Icon name="plus" size={12} />
    </button>
  </div>

  <!-- ── Toolbar ──────────────────────────────────────────────────────────── -->
  <div class="toolbar">
    <button class="tb-btn" title="Reload" aria-label="Reload" disabled={!current} onclick={reload}>
      <Icon name="refresh" size={13} />
    </button>
    <button class="tb-btn" title="Start page" aria-label="Start page" disabled={!current} onclick={home}>
      <Icon name="info" size={13} />
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
    <button class="tb-btn" title="Go" aria-label="Go" disabled={!urlInput.trim()} onclick={() => load(urlInput)}>
      <Icon name="chevronRight" size={13} />
    </button>

    <!-- Take-over toggle -->
    <button
      class="tb-btn takeover-btn"
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
        class="tb-btn"
        title="DevTools — console, network, elements"
        aria-label="DevTools"
        disabled={!current}
        onclick={toggleDevtools}
      >
        <Icon name="terminal" size={13} />
      </button>
    {/if}

    <button
      class="tb-btn"
      title="Open in system browser"
      aria-label="Open externally"
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
          <button class="btn-ghost" onclick={() => (popover = { ...popover, open: false })}>
            Cancel
          </button>
          <button class="btn-accent" disabled={!popoverComment.trim()} onclick={addAnnotation}>
            Add
          </button>
        </div>
      </div>
    {/if}

    <!-- Annotation badge + "Send to agent" -->
    {#if annotations.length > 0}
      <div class="annot-badge">
        <span class="annot-count">{annotations.length} marked</span>
        <button class="btn-accent btn-small" onclick={sendToAgent}>
          Send {annotations.length} to agent
        </button>
        <button
          class="btn-ghost btn-small"
          title="Clear all annotations"
          onclick={() => (annotations = [])}
        >
          Clear
        </button>
      </div>
    {/if}

    <div class="frame-foot">
      <span class="dim ellipsis">{current}</span>
      <button class="link" onclick={() => openExternal(current)}>Open externally ↗</button>
    </div>
  {:else}
    <div class="start">
      <p class="hint">
        {#if nativeBrowserAvailable}
          Enter a URL above to browse any site here — including ones that block
          embedding (Google, Jira, GitHub) and local dev servers. Links that open
          in a new tab open here as a new tab. Use ↗ to open in your system browser.
        {:else}
          Enter a URL above to browse it here. Sites that block embedding (Jira,
          Google, GitHub) open in your system browser with ↗.
        {/if}
      </p>

      {#if attachedIssue}
        <section class="section">
          <div class="section-title">Attached Issue</div>
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
  .tabstrip {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 6px 0 6px;
    overflow-x: auto;
    scrollbar-width: none;
    flex-shrink: 0;
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
    padding: 0 4px 0 10px;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: var(--radius-s) var(--radius-s) 0 0;
    color: var(--text-dim);
    font-size: 12px;
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
  .btab-title {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .btab-close {
    display: grid;
    place-items: center;
    width: 16px;
    height: 16px;
    flex-shrink: 0;
    border: none;
    border-radius: 3px;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    opacity: 0;
    transition: opacity 120ms ease-out, background 120ms ease-out;
  }
  .btab:hover .btab-close,
  .btab.active .btab-close {
    opacity: 1;
  }
  .btab-close:hover {
    background: color-mix(in srgb, var(--text-dim) 22%, transparent);
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
    font-size: 12px;
  }
  .tb-btn {
    width: 28px;
    height: 28px;
    flex-shrink: 0;
    display: grid;
    place-items: center;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    cursor: pointer;
  }
  .tb-btn:hover:not(:disabled) {
    border-color: var(--accent);
    color: var(--accent);
  }
  .tb-btn:disabled {
    opacity: 0.35;
    cursor: default;
  }

  /* Take-over toggle highlighted state */
  .takeover-btn.takeover-active {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
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
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .link {
    flex-shrink: 0;
    border: none;
    background: transparent;
    color: var(--accent);
    font-size: 11px;
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
    font-size: 11.5px;
    color: var(--text-dim);
    line-height: 1.5;
  }
  .section {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .section-title {
    font-size: 10px;
    font-weight: 700;
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
    font-size: 11px;
    font-weight: 700;
    color: var(--accent);
  }
  .ql-label {
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    color: var(--text);
  }

  /* ── Comment popover ───────────────────────────────────────────────────── */
  .popover {
    position: absolute;
    z-index: 200;
    width: 300px;
    background: var(--surface, #1e1e2e);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s, 6px);
    box-shadow: 0 8px 24px rgba(0, 0, 0, 0.45);
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .popover-desc {
    font-family: var(--font-mono, monospace);
    font-size: 10.5px;
    color: var(--accent);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    background: color-mix(in srgb, var(--accent) 10%, transparent);
    padding: 4px 6px;
    border-radius: 3px;
    border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .popover-textarea {
    width: 100%;
    resize: vertical;
    font-size: 12px;
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
    z-index: 150;
    display: flex;
    align-items: center;
    gap: 6px;
    background: var(--surface, #1e1e2e);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s, 6px);
    padding: 5px 8px;
    box-shadow: 0 4px 14px rgba(0, 0, 0, 0.4);
  }
  .annot-count {
    font-size: 11px;
    color: var(--text-dim);
  }

  /* ── Generic button helpers ────────────────────────────────────────────── */
  .btn-accent {
    background: var(--accent);
    color: #fff;
    border: none;
    border-radius: var(--radius-s, 4px);
    cursor: pointer;
    font-size: 12px;
    padding: 5px 10px;
    font-weight: 600;
  }
  .btn-accent:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .btn-ghost {
    background: transparent;
    color: var(--text-dim);
    border: 1px solid var(--border);
    border-radius: var(--radius-s, 4px);
    cursor: pointer;
    font-size: 12px;
    padding: 5px 10px;
  }
  .btn-ghost:hover {
    color: var(--text);
    border-color: var(--text-dim);
  }
  .btn-small {
    font-size: 11px;
    padding: 3px 8px;
  }
</style>
