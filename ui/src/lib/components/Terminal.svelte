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
  import { wsUrl, WS_BEARER_SUBPROTOCOL } from '../api/client';
  import type { SessionStatus, TermSearchMatch, WsSearchResultFrame } from '../api/types';
  import { textToBase64, base64ToBytes, bytesToBase64 } from '../b64';
  import { terminalTheme } from '../termtheme';
  import { ui } from '../stores/ui.svelte';
  import { viewport } from '../stores/viewport.svelte';
  import { ws } from '../stores/workspace.svelte';
  import { openFile } from '../stores/openfile.svelte';
  import { openExternal, isExternalUrl } from '../external';
  import { keyContext } from '../keys';
  import TermKeysBar from './TermKeysBar.svelte';

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
    /** When provided, the WS is opened with `Authorization` via the
     *  `otto-bearer` Sec-WebSocket-Protocol subprotocol carrying this token
     *  instead of the stored owner login token. Used by the guest share view
     *  (SharePage) so the scoped share token never touches localStorage.
     *  Default = undefined → falls back to today's wsUrl() behaviour. */
    shareToken?: string;
    onstatus?: (status: SessionStatus) => void;
    /** Called when the server returns a ring-buffer search result frame.
     *  The parent can surface results in a search-result panel. */
    onsearchresult?: (frame: WsSearchResultFrame) => void;
  }
  let { sessionId, readOnly = false, resumable = false, forceDark = false, shareToken, onstatus, onsearchresult }: Props = $props();

  const effScheme = $derived(forceDark ? 'dark' : ui.resolvedScheme);

  // Effective terminal font size. On phone we apply a comfortable readability
  // floor (PHONE_MIN_FONT): a 13px monospace grid is legible on a desktop
  // monitor but cramped on a high-DPI handset held at arm's length, which is a
  // big part of why the mobile terminal felt unusable. The user's zoom still
  // wins when they zoom LARGER — we only raise the floor, never cap. Desktop is
  // unchanged (the floor never applies), so `ui.termFontSize` passes through 1:1.
  const PHONE_MIN_FONT = 15;
  const effFontSize = $derived(
    viewport.isPhone ? Math.max(ui.termFontSize, PHONE_MIN_FONT) : ui.termFontSize,
  );

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
  // closedByUs = true while we intentionally close the WS — suppresses the
  // auto-reconnect that sock.onclose would otherwise trigger. Set to true
  // both on component teardown AND on a deliberate session-switch close, then
  // cleared again inside connect() so a normal reconnect after a blip still works.
  let closedByUs = false;
  // Set to true by Effect 1 once the initial WS connect is delegated to
  // the first-fit callback. Effect 2 (session-switch) skips its first run
  // until this is true, because Effect 1's RAF/fit chain handles the initial
  // connect; Effect 2 only needs to act on subsequent sessionId changes.
  let termDidInit = false;

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

  // ── Phone-only touch/keyboard state (Tasks 5.1–5.3) ────────────────────────
  // All of these are only read when viewport.isPhone; desktop code paths are
  // entirely unaffected — no gating changes are needed in the existing handlers.

  /** Whether the on-screen key accessory bar is shown on phone. */
  let keybarVisible = $state(false);

  // Touch-scroll state (Task 5.3): track single-finger pointer drags and convert
  // them to xterm line-scroll deltas. Only active on phone; desktop mouse-wheel
  // scroll uses xterm's own built-in handler (untouched here).
  let touchScrolling = $state(false);
  let touchScrollStartY = 0;
  let touchScrollAccum = 0;     // accumulated px before rounding to lines
  /** Pixels of dragged distance per terminal line (approximate; refined at runtime). */
  const PX_PER_LINE = 20;

  // ── Find / ring-search bar ──────────────────────────────────────────────────
  // Two complementary search modes share the single find bar:
  //
  //   1. LOCAL  — xterm's SearchAddon searches the emulator's visible buffer
  //      (term rows) and highlights matches in the viewport with decorations.
  //      Active immediately as the user types.
  //
  //   2. SERVER — sends a `{"type":"search","query":"…"}` WebSocket frame; the
  //      daemon greps the persistent ring-buffer (survives reconnects) and replies
  //      with a `search_result` frame containing up to 200 matching `{line, text}`
  //      pairs. These are listed below the input and the user can navigate them.
  //      Because the ring buffer stores raw bytes the matches are ANSI-stripped
  //      text; xterm.scrollToLine jumps the viewport to each hit.
  //
  // The two modes run in parallel — the local search updates on every keystroke
  // while the server result arrives asynchronously a frame later.

  let findOpen = $state(false);
  let findQuery = $state('');
  let findInput: HTMLInputElement | null = $state(null);

  /** Server-side ring-buffer matches for the current query. */
  let serverMatches = $state<TermSearchMatch[]>([]);
  /** Index of the currently highlighted server match (−1 = none). */
  let serverMatchIdx = $state(-1);
  /** True while the server search round-trip is in flight. */
  let serverSearchPending = $state(false);
  /** Debounce timer id for server searches. */
  let serverSearchTimer: ReturnType<typeof setTimeout> | null = null;

  /** Send the current query to the daemon ring-buffer search. */
  function sendServerSearch(query: string): void {
    if (!query) {
      serverMatches = [];
      serverMatchIdx = -1;
      serverSearchPending = false;
      return;
    }
    serverSearchPending = true;
    sendJson({ type: 'search', query });
  }

  /** Debounced wrapper so we don't flood the WS on every keystroke. */
  function scheduleServerSearch(query: string): void {
    if (serverSearchTimer !== null) clearTimeout(serverSearchTimer);
    serverSearchTimer = setTimeout(() => {
      serverSearchTimer = null;
      sendServerSearch(query);
    }, 300);
  }

  /** Jump the xterm viewport to the server match at `idx`. */
  function goToServerMatch(idx: number): void {
    if (serverMatches.length === 0) return;
    const clamped = ((idx % serverMatches.length) + serverMatches.length) % serverMatches.length;
    serverMatchIdx = clamped;
    const lineIdx = serverMatches[clamped].line;
    // xterm's scrollToLine(n) scrolls to line n in the buffer (0-based from top
    // of the scrollback). The ring buffer line indices are oldest → newest, which
    // matches the xterm buffer order when the scrollback is full.
    term?.scrollToLine(lineIdx);
  }

  function sendJson(obj: unknown): void {
    if (sock && sock.readyState === WebSocket.OPEN) sock.send(JSON.stringify(obj));
  }

  // ── sendSeqToTerm (Task 5.2) ─────────────────────────────────────────────
  // Shared send path for TermKeysBar — identical to what term.onData uses.
  // readOnly is enforced here so viewer shares can't type via the accessory bar.
  function sendSeqToTerm(seq: string): void {
    if (readOnly) return;
    sendJson({ type: 'input', data: textToBase64(seq) });
  }

  // ── Touch scroll handlers (Task 5.3) ─────────────────────────────────────
  // These are only wired to the container on phone (see the template below).
  // Desktop mouse-wheel scroll goes through xterm's own built-in — untouched.

  function onTouchPointerDown(e: PointerEvent): void {
    if (!viewport.isPhone || e.pointerType === 'mouse') return;
    // Only handle single-finger primary pointer
    if (!e.isPrimary) return;
    // Focus the terminal on tap. iOS/Android only raise the soft keyboard when
    // .focus() runs synchronously inside a user-gesture handler — so a tap on the
    // canvas must focus here (we preventDefault below, which would otherwise stop
    // xterm's own focus). Without this, the phone keyboard never appears and the
    // user can't type. readOnly viewers stay unfocused (no input anyway).
    if (!readOnly) term?.focus();
    touchScrolling = true;
    touchScrollStartY = e.clientY;
    touchScrollAccum = 0;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    // Prevent xterm from receiving this as a text-selection drag
    e.preventDefault();
  }

  function onTouchPointerMove(e: PointerEvent): void {
    if (!touchScrolling || !viewport.isPhone) return;
    if (!e.isPrimary) return;
    const dy = touchScrollStartY - e.clientY; // positive = scrolled up (toward older output)
    touchScrollAccum += dy;
    touchScrollStartY = e.clientY;

    // Convert accumulated pixels to whole lines and flush
    const lines = Math.trunc(touchScrollAccum / PX_PER_LINE);
    if (lines !== 0) {
      touchScrollAccum -= lines * PX_PER_LINE;
      term?.scrollLines(lines);
    }
    e.preventDefault();
  }

  function onTouchPointerUp(e: PointerEvent): void {
    if (!e.isPrimary) return;
    touchScrolling = false;
  }

  function connect(): void {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    closedByUs = false;
    disconnected = false;
    // When a shareToken is supplied (guest share view) use the otto-bearer
    // subprotocol so the token travels in Sec-WebSocket-Protocol instead of
    // the URL query string (keeps it out of access logs). The stored owner
    // login token path (wsUrl) is unchanged for all normal sessions.
    if (shareToken) {
      const wsBase = wsUrl(`/ws/term/${sessionId}`).replace(/\?token=.*$/, '');
      sock = new WebSocket(wsBase, [WS_BEARER_SUBPROTOCOL, shareToken]);
    } else {
      sock = new WebSocket(wsUrl(`/ws/term/${sessionId}`));
    }
    sock.binaryType = 'arraybuffer';

    sock.onopen = () => {
      connected = true;
      reconnecting = false;
      reconnectAttempts = 0;
      // Sync the PTY to our size first, then request a history-inclusive
      // snapshot. Ask for as many lines as xterm is configured to retain
      // (scrollback option, default 10 000) so reconnect never silently loses
      // scrollback that the server ring still holds. Clamped server-side to
      // the actual retained history depth, so this is safe to over-request.
      sendResize(true);
      const want = term?.options.scrollback ?? 10_000;
      sendJson({ type: 'scrollback', lines: want });
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
          case 'search_result': {
            // Server-side ring-buffer search result. Populate the find-bar's
            // match list (next/prev navigation) and forward to any parent listener.
            const frame = msg as WsSearchResultFrame;
            serverSearchPending = false;
            // Only apply if the result matches the current query (avoid a
            // stale response overwriting results from a newer, faster one).
            if (frame.query === findQuery) {
              serverMatches = frame.matches;
              // Jump to the first match automatically when the list refreshes.
              serverMatchIdx = -1;
              if (frame.matches.length > 0) goToServerMatch(0);
            }
            onsearchresult?.(frame);
            break;
          }
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
    // Clear server-side results so they don't linger on next open.
    serverMatches = [];
    serverMatchIdx = -1;
    serverSearchPending = false;
    if (serverSearchTimer !== null) {
      clearTimeout(serverSearchTimer);
      serverSearchTimer = null;
    }
    term?.focus();
  }

  const searchDecorations = {
    matchOverviewRuler: '#febc2e',
    activeMatchColorOverviewRuler: '#ff9f0a',
  };

  function findNext(back = false): void {
    if (findQuery === '') return;
    // Local xterm search (visible buffer + decorations).
    if (search) {
      if (back) search.findPrevious(findQuery, { decorations: searchDecorations });
      else search.findNext(findQuery, { decorations: searchDecorations });
    }
    // Server ring-buffer navigation: cycle through matches.
    if (serverMatches.length > 0) {
      goToServerMatch(back ? serverMatchIdx - 1 : serverMatchIdx + 1);
    }
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
  // `:line` so we don't match clock times (12:34). The path may be absolute (an
  // optional leading `/`) or relative; it allows dirs, `.`, `~`, `@`, `+`, `-`,
  // `_`. The negative lookbehind deliberately does NOT include `/`, so a leading
  // `/abs/path` isn't blocked. `host:port` false positives (example.com:8080) are
  // filtered in scanLine via SOURCE_FILE_EXTS for refs that contain no `/`.
  const FILE_RE = /(?<![\w.@~+-])(\/?(?:[\w.@~+-]+\/)*[\w.@~+-]+\.([A-Za-z0-9]{1,8})):(\d+)(?::(\d+))?\b/g;

  // Source/text file extensions we'll treat as a file link even when the ref has
  // no `/` (so `file.go:7` links but `example.com:8080` / `redis.io:6379` don't).
  // Refs that DO contain a `/` always link (an explicit path).
  const SOURCE_FILE_EXTS = new Set([
    'rs', 'ts', 'tsx', 'js', 'jsx', 'mjs', 'cjs', 'go', 'py', 'rb', 'java', 'kt',
    'c', 'h', 'cc', 'cpp', 'hpp', 'cs', 'php', 'swift', 'scala', 'sh', 'bash',
    'zsh', 'sql', 'html', 'css', 'scss', 'svelte', 'vue', 'json', 'toml', 'yaml',
    'yml', 'md', 'txt', 'xml', 'proto', 'lock', 'cfg', 'ini', 'env',
  ]);

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

    // `file:line` links only make sense for agent sessions: they open paths in
    // the local Files panel. SSH/DB connection sessions show a REMOTE filesystem
    // the local Files panel can't resolve, so we surface URLs only there.
    const isAgent = ws.sessions.find((s) => s.id === sessionId)?.kind === 'agent';
    if (!isAgent) return hits;

    FILE_RE.lastIndex = 0;
    for (let m = FILE_RE.exec(text); m; m = FILE_RE.exec(text)) {
      const start = m.index;
      const end = m.index + m[0].length;
      // Skip file refs that sit inside a URL we already matched (e.g. a port).
      if (hits.some((h) => h.kind === 'url' && start < h.end && end > h.start)) continue;
      const path = m[1];
      const ext = m[2].toLowerCase();
      const line = Number(m[3]);
      const col = m[4] ? Number(m[4]) : undefined;
      if (!Number.isFinite(line) || line < 1) continue;
      // A ref with no `/` separator could be `host:port` (example.com:8080) — only
      // treat it as a file link if its extension is a known source/text type. Any
      // ref containing a `/` is an explicit path and always links.
      if (!path.includes('/') && !SOURCE_FILE_EXTS.has(ext)) continue;
      hits.push({ start, end, kind: 'file', text: m[0], path, line, col });
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
          // IBufferRange.end.x is 1-based *inclusive* while h.end is exclusive,
          // so the two cancel out — no `+ 1` on the end (that would over-extend
          // the clickable range by one cell).
          const startX = h.start + 1;
          const endX = Math.min(h.end, cols);
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
                // Defence-in-depth: only ever hand http(s) to the OS browser.
                if (isExternalUrl(h.text)) void openExternal(h.text);
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

  // ── Effect 1: one-time xterm + WebGL init (re-runs only when RTL mode toggles)
  // This effect owns the Terminal object, addons, event handlers, ResizeObserver,
  // and the initial WS connection. It does NOT watch `sessionId` — that is handled
  // by Effect 2 below so session switches reconnect without rebuilding the GPU
  // canvas.
  $effect(() => {
    // Tracked read: toggling the experimental RTL mode re-runs this effect so the
    // terminal is rebuilt with the correct renderer (WebGL vs DOM — see below).
    const rtl = ui.rtlBidi;
    term = new Terminal({
      fontFamily: untrack(() => ui.termFontStack),
      fontSize: untrack(() => effFontSize),
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
    // ── Renderer selection: WebGL (GPU) on desktop, DOM everywhere it's risky ──
    // xterm draws to a WebGL canvas when WebglAddon is loaded; with no addon it
    // falls back to its DOM renderer (per-cell <span>s in `.xterm-rows`). The DOM
    // renderer is slower but ROBUST — it can't "go black" the way a WebGL canvas
    // can when the context is unavailable or silently lost.
    //
    // We skip WebGL when:
    //   • RTL bidi mode is on — the DOM renderer is required for the `.rtl-bidi`
    //     reflow (WebGL draws cells in raw logical order with no bidi).
    //   • on phone — mobile WKWebView/Safari WebGL is the main culprit behind the
    //     "terminal is a black void" report: a real device frequently fails to
    //     create the GL context or loses it right after first paint, and the old
    //     try/catch only caught a *synchronous* failure — an async context loss
    //     left a permanently black canvas with no fallback. The DOM renderer has
    //     no GPU dependency, so output is always visible and typing always works.
    //     (Phone terminals are small + low-throughput, so DOM perf is a non-issue.)
    const useWebgl = !rtl && !viewport.isPhone;
    if (useWebgl) {
      try {
        const webgl = new WebglAddon();
        // If the GPU context is lost AFTER load (common on laptops waking from
        // sleep, GPU resets, and some mobile browsers), dispose the addon so
        // xterm reverts to its DOM renderer instead of showing a black canvas.
        // This is xterm's own recommended recovery path for WebGL context loss.
        webgl.onContextLoss(() => {
          try {
            webgl.dispose(); // → xterm falls back to the DOM renderer (stays visible)
          } catch {
            /* already disposed */
          }
        });
        term.loadAddon(webgl);
      } catch {
        // WebGL unavailable at load time — xterm falls back to its DOM renderer.
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

    // Copy-on-select: when enabled, immediately copy any new selection to the
    // clipboard so the user doesn't have to press ⌘C. Implemented via the
    // selection-change event (no xterm built-in; works across all renderers).
    term.onSelectionChange(() => {
      if (!ui.termCopyOnSelect) return;
      if (!term || !term.hasSelection()) return;
      const text = term.getSelection();
      if (text) navigator.clipboard.writeText(text).catch(() => {/* clipboard denied */});
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
        // Initial connect uses the current sessionId prop (read untracked so this
        // effect doesn't re-run when sessionId changes; Effect 2 owns that).
        // Set termDidInit first so Effect 2 knows initial setup is underway and
        // won't race by calling connect() again for the same sessionId.
        termDidInit = true;
        untrack(connect);
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
      termDidInit = false;
      textarea?.removeEventListener('focus', onFocus);
      textarea?.removeEventListener('blur', onBlur);
      onBlur();
      sock?.close();
      sock = null;
      term?.dispose();
      term = null;
    };
  });

  // ── Effect 2: reactive session-switch — retarget the WS when sessionId changes
  // Runs after Effect 1 (Svelte 5 effects run in declaration order). On the very
  // first run `termDidInit` is still false (Effect 1's first-fit RAF hasn't fired
  // yet) so we bail early — Effect 1's initial `untrack(connect)` handles the
  // first connection. On subsequent runs (real session switches) we:
  //   1. Stop auto-reconnect and cancel any pending timer.
  //   2. Close the old socket synchronously (closedByUs suppresses the auto-reconnect
  //      in sock.onclose that the close event would otherwise trigger).
  //   3. Reset per-session overlay state (exitCode, disconnected flags, injN counter).
  //   4. Clear the xterm scrollback so old session output doesn't bleed through.
  //   5. Open a fresh WS for the new sessionId and request scrollback.
  $effect(() => {
    const _id = sessionId; // tracked: re-runs when sessionId changes
    // termDidInit is set by Effect 1 once the first fit fires; until then the xterm
    // canvas isn't ready and Effect 1's initial untrack(connect) handles the first WS.
    if (!termDidInit || !term) return;
    // 1. Cancel any pending reconnect timer — it belongs to the old session.
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    // 2. Close the old socket cleanly. Mark closedByUs BEFORE calling close() so
    //    the synchronous onclose callback (which scheduleReconnect reads) does not
    //    kick off a reconnect to the old session.
    closedByUs = true;
    sock?.close();
    sock = null;
    // 3. Reset per-session state.
    connected = false;
    disconnected = false;
    reconnecting = false;
    exitCode = null;
    reconnectAttempts = 0;
    lastInjN = 0;
    // 4. Clear the xterm viewport and scrollback so old session output is gone
    //    before the new scrollback arrives. term.reset() resets the terminal state
    //    (cursor, attrs, etc.) and clears scrollback while keeping the DOM/WebGL
    //    context intact — no GPU teardown occurs.
    term.reset();
    // 5. Connect to the new session. connect() clears closedByUs at its top so
    //    natural reconnect-on-drop works normally for the new session.
    connect();
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

  // react to terminal font-size zoom (uses the phone-floored effective size so
  // the readability floor stays applied across zoom/orientation changes too)
  $effect(() => {
    const size = effFontSize;
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

<!-- term-outer wraps the terminal canvas + the phone-only key bar below it.
     On desktop this is just a transparent flex pass-through; on phone it stacks
     the key bar underneath the canvas so the bar doesn't overlap the scrollback. -->
<div class="term-outer" class:phone={viewport.isPhone}>
  <div class="term-wrap" class:force-dark-wrap={forceDark}>
    {#if findOpen}
      <div class="find-bar">
        <input
          bind:this={findInput}
          bind:value={findQuery}
          placeholder="Find in terminal"
          oninput={() => {
            // Local search: xterm SearchAddon (immediate, visible buffer only).
            if (search && findQuery) search.findNext(findQuery, { decorations: searchDecorations });
            else search?.clearDecorations();
            // Server search: ring-buffer grep (debounced, full scrollback history).
            scheduleServerSearch(findQuery);
          }}
          onkeydown={(e) => {
            if (e.key === 'Enter') findNext(e.shiftKey);
            if (e.key === 'Escape') closeFind();
          }}
        />
        <!-- Server-match count badge (spinner while pending) -->
        {#if serverSearchPending}
          <span class="find-status" title="Searching scrollback…">…</span>
        {:else if serverMatches.length > 0}
          <span class="find-status" title="{serverMatches.length} scrollback match{serverMatches.length === 1 ? '' : 'es'}">
            {serverMatchIdx >= 0 ? serverMatchIdx + 1 : '?'}/{serverMatches.length}
          </span>
        {/if}
        <button class="icon-btn" onclick={() => findNext(true)} title="Previous (⇧↵)">↑</button>
        <button class="icon-btn" onclick={() => findNext(false)} title="Next (↵)">↓</button>
        <button class="icon-btn" onclick={closeFind} title="Close (esc)">✕</button>
      </div>
      {#if serverMatches.length > 0}
        <!-- Server ring-buffer match list: up to 8 rows shown, scroll for more.
             Clicking a row jumps the viewport to that line in the scrollback. -->
        <div class="find-results" role="listbox" aria-label="Scrollback search results">
          {#each serverMatches.slice(0, 8) as m, i (m.line)}
            <!-- svelte-ignore a11y_click_events_have_key_events -->
            <div
              class="find-result-row"
              class:active={i === serverMatchIdx}
              role="option"
              aria-selected={i === serverMatchIdx}
              tabindex="-1"
              onclick={() => goToServerMatch(i)}
            >
              <span class="find-result-line">{m.line + 1}</span>
              <span class="find-result-text">{m.text}</span>
            </div>
          {/each}
          {#if serverMatches.length > 8}
            <div class="find-result-more">{serverMatches.length - 8} more — use ↑↓ to navigate</div>
          {/if}
        </div>
      {/if}
    {/if}

    <!-- Desktop terminal toolbar: font zoom + copy-on-select toggle. Visible on
         desktop when ui.termToolbar is on; phone controls live in phone-controls
         below (unchanged). The toolbar sits flush bottom-left so it doesn't
         overlap the find-bar (top-right) or the overlay badges (also top-right). -->
    {#if !viewport.isPhone && ui.termToolbar}
      <div class="desk-toolbar" role="toolbar" aria-label="Terminal controls">
        <button
          class="tb-btn"
          onclick={() => ui.termZoomOut()}
          title="Zoom out (Ctrl+−)"
          aria-label="Zoom out"
        >−</button>
        <span class="tb-size" title="Terminal font size">{ui.termFontSize}px</span>
        <button
          class="tb-btn"
          onclick={() => ui.termZoomIn()}
          title="Zoom in (Ctrl+=)"
          aria-label="Zoom in"
        >+</button>
        <span class="tb-sep" aria-hidden="true"></span>
        <button
          class="tb-btn"
          class:tb-active={ui.termCopyOnSelect}
          onclick={() => ui.setTermCopyOnSelect(!ui.termCopyOnSelect)}
          title={ui.termCopyOnSelect ? 'Copy-on-select: on — click to disable' : 'Copy-on-select: off — click to enable'}
          aria-pressed={ui.termCopyOnSelect}
          aria-label="Copy on select"
        >copy</button>
      </div>
    {/if}

    <!-- Task 5.3: touch-scroll — pointer events drive term.scrollLines on phone.
         onpointerdown/move/up are no-ops on desktop (we check viewport.isPhone
         + e.pointerType inside the handlers). Desktop mouse-wheel uses xterm's
         own built-in scroll handler which is completely untouched. -->
    <!-- role="none" because this is a pure rendering surface managed by xterm.js;
         ARIA structure is inside the xterm canvas layer, not this host div. -->
    <div
      class="term-host"
      class:force-dark={forceDark}
      class:rtl-bidi={ui.rtlBidi}
      bind:this={container}
      role="none"
      onpointerdown={onTouchPointerDown}
      onpointermove={onTouchPointerMove}
      onpointerup={onTouchPointerUp}
      onpointercancel={onTouchPointerUp}
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

    <!-- Task 5.1 + 5.3: phone-only floating control strip (keyboard + zoom).
         Positioned in the top-right corner (below the overlay badges).
         Tap-to-focus: the ⌨ button focuses term.textarea to raise the soft
         keyboard on iOS/Android (iOS requires a real user-gesture → onclick).
         Zoom: calls termZoomIn/Out (fontSize-based, same as keyboard shortcut). -->
    {#if viewport.isPhone}
      <div class="phone-controls">
        <!-- Task 5.3: on-screen zoom buttons (fontSize-based — no CSS zoom) -->
        <button
          class="phone-btn"
          onclick={() => ui.termZoomOut()}
          aria-label="Zoom out terminal"
          title="Zoom out"
        >−</button>
        <button
          class="phone-btn"
          onclick={() => ui.termZoomIn()}
          aria-label="Zoom in terminal"
          title="Zoom in"
        >+</button>
        <!-- Task 5.1: keyboard toggle — focuses term.textarea (real user gesture) -->
        <button
          class="phone-btn"
          class:active={keybarVisible}
          onclick={() => {
            keybarVisible = !keybarVisible;
            // iOS/Android: focus MUST happen inside the onclick to count as a
            // user gesture; the soft keyboard only appears for that gesture.
            if (keybarVisible) {
              term?.focus();
            }
          }}
          aria-label="Toggle keyboard"
          title="Show/hide keyboard"
        >⌨</button>
      </div>
    {/if}
  </div>

  <!-- Task 5.2: key accessory bar — only mounted on phone, only shown when the
       keyboard is toggled on. Rendered below the terminal canvas (not overlaid)
       so it never covers the scrollback. The sendSeq prop wires directly to
       sendSeqToTerm which uses the same sendJson path as term.onData. -->
  {#if viewport.isPhone && keybarVisible}
    <TermKeysBar sendSeq={sendSeqToTerm} {readOnly} />
  {/if}
</div>

<style>
  /* ── Task 5.1/5.2/5.3: phone outer wrapper ───────────────────────────────
     On phone the outer div is a vertical flex column: the canvas fills the
     available height (flex:1) and TermKeysBar stacks below it with its natural
     height. On desktop this wrapper is transparent — just passes 100%/100%
     through to .term-wrap exactly as before. */
  .term-outer {
    width: 100%;
    height: 100%;
    display: contents; /* desktop: no layout impact — children see the parent's box */
  }
  .term-outer.phone {
    display: flex;
    flex-direction: column;
  }
  .term-outer.phone .term-wrap {
    flex: 1;
    min-height: 0; /* allow flex child to shrink below its content height */
  }

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
    inset-inline-end: 16px;
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
  /* Scrollback match count / spinner badge next to the input */
  .find-status {
    font-size: 10px;
    color: var(--text-dim);
    white-space: nowrap;
    user-select: none;
    padding: 0 2px;
  }
  /* Dropdown list of server ring-buffer matches */
  .find-results {
    position: absolute;
    top: calc(8px + 30px + 2px);
    right: 16px;
    z-index: 5;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
    width: 340px;
    max-height: 200px;
    overflow-y: auto;
    font-size: 11px;
    font-family: var(--font-mono);
  }
  .find-result-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 3px 8px;
    cursor: pointer;
    color: var(--text);
  }
  .find-result-row:hover {
    background: var(--hover);
  }
  .find-result-row.active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--text);
  }
  .find-result-line {
    color: var(--text-dim);
    min-width: 36px;
    text-align: end;
    flex-shrink: 0;
    font-size: 10px;
  }
  .find-result-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .find-result-more {
    padding: 3px 8px;
    font-size: 10px;
    color: var(--text-dim);
    font-family: var(--font-sans, sans-serif);
    text-align: center;
    border-top: 1px solid var(--border);
  }
  /* Small unobtrusive chip in the top-right — never covers the input line. */
  .term-overlay {
    position: absolute;
    top: 6px;
    inset-inline-end: 8px;
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
    inset-inline-end: 8px;
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

  /* ── Task 5.1 + 5.3: phone-only floating controls (keyboard toggle + zoom) ──
     Positioned bottom-RIGHT: terminal output (and the live prompt/cursor where
     you type) is left-aligned, so the right edge is almost always empty — this
     keeps the floating buttons from covering the input line. The overlay badges
     sit top-right, so bottom-right doesn't collide with them either. A solid-ish
     backdrop + blur keeps the glyphs legible on the rare line that reaches the
     edge. Only rendered when viewport.isPhone. */
  .phone-controls {
    position: absolute;
    bottom: 8px;
    inset-inline-end: 8px;
    z-index: 7;
    display: flex;
    flex-direction: row;
    gap: 6px;
    align-items: center;
  }
  .phone-btn {
    /* ≥44×44px tap target (WCAG 2.5.5 / iOS HIG) */
    min-width: 44px;
    min-height: 44px;
    padding: 0 8px;
    display: flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--radius-s, 8px);
    border: 1px solid var(--border, #444);
    background: color-mix(in srgb, var(--surface, #28282e) 85%, transparent);
    color: var(--text, #e8e8e0);
    font-size: 18px;
    cursor: pointer;
    touch-action: manipulation;
    -webkit-tap-highlight-color: transparent;
    transition: background 0.1s;
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
  }
  .phone-btn:active {
    background: var(--accent, #0066cc);
    color: #fff;
  }
  .phone-btn.active {
    background: var(--accent, #0066cc);
    color: #fff;
    border-color: var(--accent, #0066cc);
  }

  /* ── Desktop terminal toolbar (font zoom + copy-on-select) ─────────────
     Sits bottom-left, well away from the find-bar (top-right) and overlay
     badges. Only shown on desktop when ui.termToolbar is on. */
  .desk-toolbar {
    position: absolute;
    bottom: 6px;
    inset-inline-start: 8px;
    z-index: 5;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 2px 4px;
    background: color-mix(in srgb, var(--surface) 80%, transparent);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    backdrop-filter: blur(4px);
    -webkit-backdrop-filter: blur(4px);
    opacity: 0.7;
    transition: opacity 150ms ease-out;
  }
  .desk-toolbar:hover {
    opacity: 1;
  }
  .tb-btn {
    display: flex;
    align-items: center;
    justify-content: center;
    height: 18px;
    min-width: 18px;
    padding: 0 4px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font-size: 11px;
    cursor: pointer;
    transition: background 100ms ease-out, color 100ms ease-out;
  }
  .tb-btn:hover {
    background: var(--surface-2);
    color: var(--text);
  }
  .tb-btn.tb-active {
    color: var(--accent);
  }
  .tb-size {
    font-size: 10px;
    color: var(--text-dim);
    min-width: 28px;
    text-align: center;
    user-select: none;
  }
  .tb-sep {
    width: 1px;
    height: 12px;
    background: var(--border);
    margin: 0 2px;
  }
</style>
