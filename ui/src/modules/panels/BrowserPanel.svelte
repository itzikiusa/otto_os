<script lang="ts">
  // Browser tab: a real inline browser. The app CSP is null, so an <iframe>
  // can load external pages directly in the panel. Some sites (Jira, Google,
  // GitHub) send X-Frame-Options/frame-ancestors and refuse to be framed — for
  // those we offer "Open externally". A start page lists the attached issue and
  // quick links until you navigate somewhere.
  //
  // "Take over" mode: reloads the page via the daemon's proxy endpoint so a
  // picker script is injected. Clicking elements captures a CSS-selector
  // description; the user adds a comment; all comments are sent as one message
  // to the active agent session's input.
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

  let urlInput = $state('');
  let current = $state(''); // the logical URL currently loaded
  let reloadTick = $state(0); // bump to force the iframe to reload

  // ── Take-over state ────────────────────────────────────────────────────────
  let takeover = $state(false);
  type Annotation = { desc: string; comment: string; url: string };
  let annotations = $state<Annotation[]>([]);

  type Popover = { open: boolean; x: number; y: number; desc: string; url: string };
  let popover = $state<Popover>({ open: false, x: 0, y: 0, desc: '', url: '' });
  let popoverComment = $state('');

  // The iframe DOM node — bound below with bind:this
  let frame = $state<HTMLIFrameElement | null>(null);

  // ── Native browser (Tauri child webview) ───────────────────────────────────
  // A real webview ignores X-Frame-Options and loads http://localhost, so it
  // behaves like an actual browser. Used for normal browsing; take-over still
  // uses the iframe+proxy path so the element picker keeps working.
  let hostEl = $state<HTMLDivElement | null>(null);
  let urlFocused = $state(false);
  const useNative = $derived(nativeBrowserAvailable && !takeover);

  function hostRect(): { x: number; y: number; width: number; height: number } | null {
    if (!hostEl) return null;
    const r = hostEl.getBoundingClientRect();
    if (r.width < 1 || r.height < 1) return null;
    return { x: r.left, y: r.top, width: r.width, height: r.height };
  }

  let lastOpenedUrl: string | null = null;

  // Drive the native webview. It's shown only when browsing natively (not
  // take-over / start page) AND no SPA overlay is open — a native webview always
  // paints above the HTML, so it must hide for palette / modals / context menus.
  // Same-URL re-shows don't re-navigate (no reload when an overlay closes).
  $effect(() => {
    if (!nativeBrowserAvailable) return;
    const url = current; // reactive deps
    const overlay = ui.overlayOpen || ctxMenu.open;
    const shouldShow = useNative && !!url && !overlay;
    if (!shouldShow) {
      void nativeBrowser.hide();
      return;
    }
    const r = hostRect();
    if (!r) return;
    if (url !== lastOpenedUrl) {
      lastOpenedUrl = url;
      void nativeBrowser.open(url, r); // navigates + shows
    } else {
      void nativeBrowser.bounds(r);
      void nativeBrowser.show();
    }
  });

  // Keep the webview aligned with the panel as it resizes / the window changes.
  $effect(() => {
    if (!nativeBrowserAvailable || !useNative || !current || !hostEl) return;
    const sync = (): void => {
      const r = hostRect();
      if (r) void nativeBrowser.bounds(r);
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

  // Reflect link navigations inside the webview into the address bar.
  $effect(() => {
    if (!nativeBrowserAvailable || !useNative || !current) return;
    const iv = setInterval(async () => {
      const u = await nativeBrowser.currentUrl();
      if (u && !urlFocused && u !== urlInput) urlInput = u;
    }, 1200);
    return () => clearInterval(iv);
  });

  // Tear the webview down when the panel/tab unmounts.
  $effect(() => () => {
    if (nativeBrowserAvailable) void nativeBrowser.close();
  });

  // ── Derived iframe src ─────────────────────────────────────────────────────
  // When take-over is active, route through the daemon proxy so the picker
  // script is injected. Otherwise load the URL directly.
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

  function load(u: string): void {
    const href = normalize(u);
    if (!href) return;
    urlInput = href;
    current = href;
    reloadTick++;
  }

  function reload(): void {
    if (!current) return;
    if (useNative) void nativeBrowser.reload();
    else reloadTick++;
  }

  function home(): void {
    current = '';
    takeover = false;
    annotations = [];
    popover = { open: false, x: 0, y: 0, desc: '', url: '' };
    lastOpenedUrl = null;
    if (nativeBrowserAvailable) void nativeBrowser.hide();
  }

  function openExternal(u: string): void {
    // Route through the shell `open` command (Tauri); plain anchors with
    // target=_blank don't reach the system browser inside the webview.
    void openExternalUrl(normalize(u));
  }

  function onEnter(e: KeyboardEvent): void {
    if (e.key === 'Enter') load(urlInput);
  }

  // ── Take-over toggle ───────────────────────────────────────────────────────
  function toggleTakeover(): void {
    if (takeover) {
      releaseTakeover();
    } else {
      takeover = true;
      // frameSrc is now the proxy URL; the iframe will reload.
      // After load we postMessage in the onload handler below.
    }
  }

  function releaseTakeover(): void {
    takeover = false;
    popover = { open: false, x: 0, y: 0, desc: '', url: '' };
    // frameSrc reverts to direct URL; browser reloads the original.
  }

  // Tell the iframe's injected picker to enable/disable the crosshair.
  function syncTakeoverMessage(): void {
    frame?.contentWindow?.postMessage({ type: 'otto-takeover', enabled: takeover }, '*');
  }

  // Called on iframe's onload event.
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

        // Clamp the popover within the browser panel container.
        // The iframe fills the parent, so x/y are relative to the iframe
        // which is positioned inside .browser. We keep the popover inside
        // the panel by clamping to reasonable bounds.
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

  // Whenever takeover changes, re-sync the message (for the case where the
  // frame was already loaded and we just toggled the mode).
  $effect(() => {
    // Access takeover so the effect re-runs when it changes.
    const _t = takeover;
    syncTakeoverMessage();
  });

  // ── Global Esc handler ────────────────────────────────────────────────────
  $effect(() => {
    function onKeydown(e: KeyboardEvent): void {
      if (e.key === 'Escape') {
        if (popover.open) {
          popover = { ...popover, open: false };
        } else if (takeover) {
          releaseTakeover();
        }
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
      {#if takeover}
        <Icon name="cursor" size={13} />
      {:else}
        <Icon name="cursor" size={13} />
      {/if}
    </button>

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
      <!-- A native child webview is positioned over this host element (it loads
           any site — google, localhost — unlike an iframe). -->
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
        <button
          class="btn-accent btn-small"
          onclick={sendToAgent}
        >
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
          embedding (Google, Jira, GitHub) and local dev servers. Use ↗ to open
          in your system browser.
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
  .toolbar {
    display: flex;
    align-items: center;
    gap: 5px;
    padding: 8px 8px;
    border-bottom: 1px solid var(--border);
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
