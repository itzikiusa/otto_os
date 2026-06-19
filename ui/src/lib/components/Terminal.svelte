<script lang="ts">
  // xterm.js terminal bound to WS /ws/term/{id} per docs/contracts/ws.md.
  // Binary frames → term.write; JSON control frames for status/exit/scrollback.
  import { untrack } from 'svelte';
  import { Terminal } from '@xterm/xterm';
  // ILinkProvider isn't exported from the ambient module, so derive its shape
  // from registerLinkProvider's parameter (kept in lockstep with the version).
  type LinkProvider = Parameters<Terminal['registerLinkProvider']>[0];
  import { FitAddon } from '@xterm/addon-fit';
  import { SearchAddon } from '@xterm/addon-search';
  import { WebglAddon } from '@xterm/addon-webgl';
  import '@xterm/xterm/css/xterm.css';
  import { wsUrl } from '../api/client';
  import type { SessionStatus } from '../api/types';
  import { textToBase64, base64ToBytes, bytesToBase64 } from '../b64';
  import { terminalTheme } from '../termtheme';
  import { ui } from '../stores/ui.svelte';
  import { ws } from '../stores/workspace.svelte';
  import { openFile } from '../stores/openfile.svelte';
  import { openExternal } from '../external';
  import { keyContext } from '../keys';

  interface Props {
    sessionId: string;
    readOnly?: boolean;
    /** When true a "Resume" button is shown on the exited overlay; clicking it
     *  reconnects the WS — the daemon's ensure_live will resume the session. */
    resumable?: boolean;
    /** When true, force the xterm palette and host background to dark regardless
     *  of the app's current light/dark scheme. Use for embedded agent CLIs
     *  (claude, codex) that render their own dark TUI canvas. */
    forceDark?: boolean;
    onstatus?: (status: SessionStatus) => void;
  }
  let { sessionId, readOnly = false, resumable = false, forceDark = false, onstatus }: Props = $props();

  const effScheme = $derived(forceDark ? 'dark' : ui.resolvedScheme);

  let container: HTMLDivElement;
  let term: Terminal | null = null;
  let fit: FitAddon | null = null;
  let search: SearchAddon | null = null;
  let sock: WebSocket | null = null;

  let connected = $state(false);
  let exitCode: number | null = $state(null);
  let disconnected = $state(false);
  let reconnecting = $state(false);

  // Auto-reconnect: the WS drops on daemon restarts / transient blips. Instead
  // of stranding a "disconnected" badge, retry with capped exponential backoff
  // (the server's ensure_live resumes the session on re-attach).
  let reconnectAttempts = 0;
  let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  let closedByUs = false;

  function scheduleReconnect(): void {
    if (closedByUs || exitCode !== null || reconnectTimer) return;
    reconnecting = true;
    const delay = Math.min(500 * 2 ** reconnectAttempts, 5000);
    reconnectAttempts++;
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      connect();
    }, delay);
  }

  // find bar
  let findOpen = $state(false);
  let findQuery = $state('');
  let findInput: HTMLInputElement | null = $state(null);

  function sendJson(obj: unknown): void {
    if (sock && sock.readyState === WebSocket.OPEN) sock.send(JSON.stringify(obj));
  }

  function connect(): void {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    closedByUs = false;
    disconnected = false;
    sock = new WebSocket(wsUrl(`/ws/term/${sessionId}`));
    sock.binaryType = 'arraybuffer';

    sock.onopen = () => {
      connected = true;
      reconnecting = false;
      reconnectAttempts = 0;
      // Sync the PTY to our size first, then request the current-screen
      // snapshot (the server reproduces the live screen in one coherent frame
      // — input box included — so there's no replay flicker or clipped TUI).
      sendResize(true);
      sendJson({ type: 'scrollback', lines: 2000 });
    };

    sock.onmessage = (ev: MessageEvent) => {
      if (ev.data instanceof ArrayBuffer) {
        term?.write(new Uint8Array(ev.data));
        return;
      }
      if (typeof ev.data !== 'string') return;
      try {
        const msg = JSON.parse(ev.data);
        switch (msg.type) {
          case 'scrollback':
            if (msg.data) term?.write(base64ToBytes(msg.data));
            break;
          case 'status':
            onstatus?.(msg.status as SessionStatus);
            break;
          case 'exit':
            exitCode = msg.code ?? 0;
            break;
          case 'error':
            term?.write(`\r\n\x1b[31m[otto] ${msg.code}: ${msg.message ?? ''}\x1b[0m\r\n`);
            break;
        }
      } catch {
        /* ignore malformed control frame */
      }
    };

    sock.onclose = () => {
      connected = false;
      if (exitCode === null && !closedByUs) {
        disconnected = true;
        scheduleReconnect();
      }
    };
  }

  // Only push a resize when the dimensions actually changed — sending a
  // SIGWINCH on every fit() call makes claude/codex repaint and flicker.
  let lastCols = 0;
  let lastRows = 0;

  function sendResize(force = false): void {
    if (!term) return;
    const { cols, rows } = term;
    if (!force && cols === lastCols && rows === lastRows) return;
    lastCols = cols;
    lastRows = rows;
    sendJson({ type: 'resize', cols, rows });
  }

  // Guarded fit: only run when the container is actually visible and has real
  // dimensions. On first open the pane lives inside a CSS grid that isn't laid
  // out during the synchronous mount tick, so clientWidth/clientHeight are 0 —
  // fitting then computes a garbage grid (wrong cols/rows) which the PTY snaps
  // its scrollback to → garbled wrapping / broken scroll until the next re-fit.
  // Skipping the fit when 0×0 (or when proposeDimensions() can't measure)
  // guarantees we only ever push a correct grid to xterm and the backend.
  function safeFit(): boolean {
    if (!term || !fit || !container) return false;
    if (container.clientWidth < 1 || container.clientHeight < 1) return false;
    let dims: { cols: number; rows: number } | undefined;
    try {
      dims = fit.proposeDimensions();
    } catch {
      return false; // container not laid out / detached
    }
    if (!dims || !Number.isFinite(dims.cols) || !Number.isFinite(dims.rows)) return false;
    if (dims.cols < 1 || dims.rows < 1) return false;
    try {
      fit.fit();
    } catch {
      return false;
    }
    return true;
  }


  function openFind(): void {
    findOpen = true;
    queueMicrotask(() => findInput?.focus());
  }

  function closeFind(): void {
    findOpen = false;
    search?.clearDecorations();
    term?.focus();
  }

  const searchDecorations = {
    matchOverviewRuler: '#febc2e',
    activeMatchColorOverviewRuler: '#ff9f0a',
  };

  function findNext(back = false): void {
    if (!search || findQuery === '') return;
    if (back) search.findPrevious(findQuery, { decorations: searchDecorations });
    else search.findNext(findQuery, { decorations: searchDecorations });
  }

  // ── Clickable links (URLs + `file:line(:col)`) ─────────────────────────────
  // Implemented with xterm's built-in registerLinkProvider (no web-links addon
  // dependency). For each buffer line we scan the text and surface ranges for:
  //   • http(s) URLs            → open in the system browser (openExternal)
  //   • path:line(:col) refs    → open the file in the Files panel at the line
  // Detection is conservative to avoid turning ordinary prose into dead links.

  interface LinkHit {
    /** 0-based [start, end) character offsets within the line string. */
    start: number;
    end: number;
    kind: 'url' | 'file';
    text: string;
    /** file refs only */
    path?: string;
    line?: number;
    col?: number;
  }

  // URLs: stop at whitespace and characters that are almost never *inside* a URL
  // but commonly trail one in prose. Trailing punctuation is trimmed afterwards.
  const URL_RE = /\bhttps?:\/\/[^\s<>"'`(){}\[\]]+/gi;
  // file:line(:col) — require a file extension (1–8 alnum chars) right before the
  // `:line` so we don't match clock times (12:34) or `host:port`. The path part
  // allows dirs, `.`, `~`, `@`, `+`, `-`, `_`.
  const FILE_RE = /(?<![\w./@~+-])((?:[\w.@~+-]+\/)*[\w.@~+-]+\.[A-Za-z0-9]{1,8}):(\d+)(?::(\d+))?\b/g;

  /** Trim trailing punctuation that's likely sentence/markup, not part of the URL. */
  function trimUrl(raw: string): string {
    let s = raw;
    // Drop a trailing unbalanced closing paren/bracket and common terminators.
    while (s.length > 1 && /[.,;:!?'"]$/.test(s)) s = s.slice(0, -1);
    if (s.endsWith(')') && !s.includes('(')) s = s.slice(0, -1);
    if (s.endsWith(']') && !s.includes('[')) s = s.slice(0, -1);
    return s;
  }

  /** Scan one rendered buffer-line string for URL + file:line hits. */
  function scanLine(text: string): LinkHit[] {
    const hits: LinkHit[] = [];

    URL_RE.lastIndex = 0;
    for (let m = URL_RE.exec(text); m; m = URL_RE.exec(text)) {
      const trimmed = trimUrl(m[0]);
      if (trimmed.length < 'http://a'.length) continue;
      hits.push({ start: m.index, end: m.index + trimmed.length, kind: 'url', text: trimmed });
    }

    FILE_RE.lastIndex = 0;
    for (let m = FILE_RE.exec(text); m; m = FILE_RE.exec(text)) {
      const start = m.index;
      const end = m.index + m[0].length;
      // Skip file refs that sit inside a URL we already matched (e.g. a port).
      if (hits.some((h) => h.kind === 'url' && start < h.end && end > h.start)) continue;
      const line = Number(m[2]);
      const col = m[3] ? Number(m[3]) : undefined;
      if (!Number.isFinite(line) || line < 1) continue;
      hits.push({ start, end, kind: 'file', text: m[0], path: m[1], line, col });
    }
    return hits;
  }

  /** Resolve a (possibly relative) file path against the session cwd. */
  function resolvePath(p: string): string {
    if (p.startsWith('/') || p.startsWith('~')) return p;
    const cwd = ws.sessions.find((s) => s.id === sessionId)?.cwd;
    if (!cwd) return p;
    return `${cwd.replace(/\/$/, '')}/${p}`;
  }

  /** Build the xterm link provider that surfaces every URL + file:line hit on a line. */
  function makeLinkProvider(): LinkProvider {
    return {
      provideLinks(bufferLineNumber, callback) {
        const buf = term?.buffer.active;
        const line = buf?.getLine(bufferLineNumber - 1);
        if (!line) {
          callback(undefined);
          return;
        }
        const text = line.translateToString(true);
        const cols = term?.cols ?? text.length;
        const hits = scanLine(text);
        if (hits.length === 0) {
          callback(undefined);
          return;
        }
        const links = hits.map((h) => {
          // xterm buffer coords are 1-based; clamp the end to the row width.
          const startX = h.start + 1;
          const endX = Math.min(h.end, cols) + 1;
          return {
            text: h.text,
            range: {
              start: { x: startX, y: bufferLineNumber },
              end: { x: endX, y: bufferLineNumber },
            },
            decorations: { pointerCursor: true, underline: true },
            activate: (event: MouseEvent) => {
              event.preventDefault();
              if (h.kind === 'url') {
                void openExternal(h.text);
              } else if (h.path) {
                openFile.open(resolvePath(h.path), h.line, h.col);
              }
            },
          };
        });
        callback(links);
      },
    };
  }

  $effect(() => {
    // Tracked read: toggling the experimental RTL mode re-runs this effect so the
    // terminal is rebuilt with the correct renderer (WebGL vs DOM — see below).
    const rtl = ui.rtlBidi;
    term = new Terminal({
      fontFamily: untrack(() => ui.termFontStack),
      fontSize: untrack(() => ui.termFontSize),
      cursorBlink: true,
      allowProposedApi: true,
      // Keep fallback-font glyphs (e.g. Hebrew from Cousine) inside their grid
      // cell when their advance width differs from the primary font's cell.
      rescaleOverlappingGlyphs: true,
      scrollback: 10_000,
      theme: untrack(() => terminalTheme(ui.theme, untrack(() => effScheme))),
      macOptionIsMeta: true,
    });
    fit = new FitAddon();
    search = new SearchAddon();
    term.loadAddon(fit);
    term.loadAddon(search);
    term.open(container);
    // Experimental RTL: skip the WebGL renderer. WebGL/canvas draws cells in raw
    // logical order with no bidi (Hebrew comes out reversed). xterm's DOM
    // renderer instead emits per-run spans that the `.rtl-bidi` CSS rules re-flow
    // into a single bidi paragraph per line, so the browser applies the full
    // Unicode Bidi Algorithm — Hebrew reads right-to-left with English embedded
    // LTR. (Trade-off: no GPU renderer, and the grid no longer aligns to exact
    // columns — fine for prose, imperfect for TUI tables/boxes.)
    if (!rtl) {
      try {
        term.loadAddon(new WebglAddon());
      } catch {
        // WebGL unavailable — xterm falls back to its default renderer
      }
    }

    // Clickable links — works identically with WebGL on or off (the link layer
    // is a DOM overlay above the renderer). Disposed on teardown below.
    const linkProvider = term.registerLinkProvider(makeLinkProvider());
    // NOTE: do NOT fit() here. The container has no real size yet on first open
    // (grid/flex layout isn't resolved this tick). The ResizeObserver below
    // fires once the pane gets its real box and performs the first valid fit,
    // and connect() is deferred until then so the PTY is sized correctly.

    // Shift+Enter must insert a newline in the agent's composer, not submit.
    // xterm emits plain `\r` for Enter regardless of Shift, and `\r` is what
    // claude/codex read as "submit". Intercept Shift+Enter and send `\x1b\r`
    // (ESC+CR) instead — the same sequence Option/Meta+Enter produces (this
    // terminal sets macOptionIsMeta), which these TUIs treat as a newline.
    // Plain Enter is left untouched, so it still submits.
    term.attachCustomKeyEventHandler((e) => {
      if (
        e.type === 'keydown' &&
        e.key === 'Enter' &&
        e.shiftKey &&
        !e.ctrlKey &&
        !e.metaKey &&
        !e.altKey
      ) {
        // preventDefault is essential: returning false alone stops xterm's
        // `\r`, but the browser's default Enter-in-textarea would still insert
        // a `\n` that xterm forwards — and claude reads that `\n` as submit.
        // Prevent the default so ONLY our newline sequence is sent.
        e.preventDefault();
        e.stopPropagation();
        if (!readOnly) sendJson({ type: 'input', data: textToBase64('\x1b\r') });
        return false; // suppress xterm's default `\r` (which would submit)
      }
      return true;
    });

    term.onData((data) => {
      if (readOnly) return;
      sendJson({ type: 'input', data: textToBase64(data) });
    });
    term.onBinary((data) => {
      if (readOnly) return;
      // raw binary path (e.g. some IME flows) — bytes are latin1 in a string
      const bytes = new Uint8Array(data.length);
      for (let i = 0; i < data.length; i++) bytes[i] = data.charCodeAt(i) & 0xff;
      sendJson({ type: 'input', data: bytesToBase64(bytes) });
    });

    const textarea = term.textarea;
    const onFocus = () => {
      keyContext.terminalFocused = true;
      keyContext.openFind = openFind;
    };
    const onBlur = () => {
      keyContext.terminalFocused = false;
      if (keyContext.openFind === openFind) keyContext.openFind = null;
    };
    textarea?.addEventListener('focus', onFocus);
    textarea?.addEventListener('blur', onBlur);

    // The WS is connected lazily on the first *valid* fit so the very first
    // sendResize(true) in sock.onopen ships a correct grid (covers first open).
    let didFirstFit = false;
    let refitTimer: ReturnType<typeof setTimeout> | null = null;
    const refit = () => {
      const ok = safeFit();
      if (!ok) return; // 0×0 / not laid out / detached — try again on next RO tick
      sendResize();
      if (!didFirstFit) {
        didFirstFit = true;
        connect();
      }
    };
    // Debounce: a single layout change fires the observer many times; coalesce
    // them so we fit + resize once things settle (prevents SIGWINCH flicker).
    // The observer ALSO drives the initial fit: it fires as soon as the pane is
    // assigned a real (non-zero) box — including when we navigate back to a
    // workspace and the terminal becomes visible/active again.
    const ro = new ResizeObserver(() => {
      if (refitTimer) clearTimeout(refitTimer);
      refitTimer = setTimeout(refit, didFirstFit ? 90 : 0);
    });
    ro.observe(container);
    // Belt-and-suspenders for environments where the box is already sized at
    // mount (e.g. workspace switch back): try a fit after layout settles. If
    // the container still has no size, safeFit() no-ops and the RO handles it.
    requestAnimationFrame(() => requestAnimationFrame(refit));

    return () => {
      ro.disconnect();
      linkProvider.dispose();
      if (refitTimer) clearTimeout(refitTimer);
      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      closedByUs = true;
      textarea?.removeEventListener('focus', onFocus);
      textarea?.removeEventListener('blur', onBlur);
      onBlur();
      sock?.close();
      sock = null;
      term?.dispose();
      term = null;
    };
  });

  // Recover immediately (skip backoff) when the network or app window comes
  // back, if we're sitting disconnected.
  $effect(() => {
    const retryNow = (): void => {
      if (closedByUs || exitCode !== null || connected) return;
      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      reconnectAttempts = 0;
      connect();
    };
    const onVis = (): void => {
      if (document.visibilityState === 'visible') retryNow();
    };
    window.addEventListener('online', retryNow);
    document.addEventListener('visibilitychange', onVis);
    return () => {
      window.removeEventListener('online', retryNow);
      document.removeEventListener('visibilitychange', onVis);
    };
  });

  // react to terminal font-size zoom
  $effect(() => {
    const size = ui.termFontSize;
    if (term && term.options.fontSize !== size) {
      term.options.fontSize = size;
      if (safeFit()) sendResize();
    }
  });

  // react to terminal font-family choice (live, no rebuild needed)
  $effect(() => {
    const family = ui.termFontStack;
    if (term && term.options.fontFamily !== family) {
      term.options.fontFamily = family;
      if (safeFit()) sendResize();
    }
  });

  // react to theme + light/dark scheme switches (respects forceDark override)
  $effect(() => {
    const theme = terminalTheme(ui.theme, effScheme);
    if (term) term.options.theme = theme;
  });

  // Apply programmatic input injected into this session (e.g. DB rows → a running
  // agent), wrapped in bracketed paste so multi-line content isn't auto-submitted.
  let lastInjN = 0;
  $effect(() => {
    const inj = ws.injections[sessionId];
    if (!inj || readOnly || inj.n <= lastInjN) return;
    lastInjN = inj.n;
    sendJson({ type: 'input', data: textToBase64(`\x1b[200~${inj.text}\x1b[201~`) });
  });

  export function focus(): void {
    term?.focus();
  }
</script>

<div class="term-wrap" class:force-dark-wrap={forceDark}>
  {#if findOpen}
    <div class="find-bar">
      <input
        bind:this={findInput}
        bind:value={findQuery}
        placeholder="Find in terminal"
        onkeydown={(e) => {
          if (e.key === 'Enter') findNext(e.shiftKey);
          if (e.key === 'Escape') closeFind();
        }}
      />
      <button class="icon-btn" onclick={() => findNext(true)} title="Previous (⇧↵)">↑</button>
      <button class="icon-btn" onclick={() => findNext(false)} title="Next (↵)">↓</button>
      <button class="icon-btn" onclick={closeFind} title="Close (esc)">✕</button>
    </div>
  {/if}

  <div
    class="term-host"
    class:force-dark={forceDark}
    class:rtl-bidi={ui.rtlBidi}
    bind:this={container}
  ></div>

  {#if exitCode !== null}
    <div class="term-overlay">
      <span class="badge {exitCode === 0 ? 'ok' : 'bad'}">exited ({exitCode})</span>
      {#if resumable && !readOnly}
        <button class="btn" onclick={() => { exitCode = null; connect(); }}>Resume</button>
      {/if}
    </div>
  {:else if reconnecting}
    <div class="term-overlay dim">
      <span class="badge">reconnecting…</span>
      <button class="btn" onclick={() => { reconnectAttempts = 0; connect(); }}>Now</button>
    </div>
  {:else if disconnected}
    <div class="term-overlay">
      <span class="badge bad">disconnected</span>
      <button class="btn" onclick={connect}>Reconnect</button>
    </div>
  {:else if !connected}
    <div class="term-overlay dim"><span class="badge">connecting…</span></div>
  {/if}

  {#if readOnly}
    <div class="ro-chip" title="Viewer role — input disabled">read-only</div>
  {/if}
</div>

<style>
  .term-wrap {
    position: relative;
    width: 100%;
    height: 100%;
    background: var(--term-bg);
    overflow: hidden;
  }
  /* Force dark: override the host wrapper and xterm host bg so the entire
     embedded terminal reads as one dark widget regardless of app scheme. */
  .term-wrap.force-dark-wrap {
    background: #131318;
  }
  .term-host {
    position: absolute;
    inset: 6px 0 4px 8px;
  }
  .term-host.force-dark {
    background: #131318;
  }
  /* Experimental RTL (ui.rtlBidi, DOM renderer only). xterm renders each run as
     a fixed-width `inline-block` span, which is atomic to the bidi algorithm —
     so words stay left-to-right. Forcing the spans back to inline flow makes the
     whole row one bidi paragraph; `unicode-bidi: plaintext` then gives each line
     a per-line base direction (RTL when it starts with Hebrew). The browser then
     lays the line out exactly like native bidi: Hebrew right-to-left with English
     embedded left-to-right. Cost: the monospace grid no longer aligns to exact
     columns (fine for prose, imperfect for TUI tables/boxes). */
  .term-host.rtl-bidi :global(.xterm-rows > div) {
    unicode-bidi: plaintext;
  }
  .term-host.rtl-bidi :global(.xterm-rows > div span) {
    display: inline !important;
    unicode-bidi: normal !important;
    width: auto !important;
    letter-spacing: 0 !important;
  }
  .find-bar {
    position: absolute;
    top: 8px;
    right: 16px;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 6px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
  }
  .find-bar input {
    width: 180px;
    border: none;
    background: transparent;
    font-size: 12px;
    color: var(--text);
    outline: none;
  }
  /* Small unobtrusive chip in the top-right — never covers the input line. */
  .term-overlay {
    position: absolute;
    top: 6px;
    right: 8px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px 2px 8px;
    z-index: 6;
    background: color-mix(in srgb, var(--surface) 88%, transparent);
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: 10px;
    opacity: 0.9;
  }
  .term-overlay.dim {
    opacity: 0.7;
  }
  .term-overlay .badge {
    font-size: 10px;
    padding: 0;
    background: none;
    border: none;
  }
  .term-overlay .btn {
    padding: 1px 8px;
    font-size: 10px;
  }
  .badge {
    font-size: 11px;
    padding: 3px 8px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    border: 1px solid var(--border);
  }
  .badge.ok {
    color: var(--status-working);
  }
  .badge.bad {
    color: var(--status-exited);
  }
  .ro-chip {
    position: absolute;
    top: 8px;
    right: 8px;
    z-index: 4;
    font-size: 10px;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
    background: var(--surface-2);
    border: 1px solid var(--border);
    padding: 2px 7px;
    border-radius: 999px;
    opacity: 0.85;
  }
</style>
