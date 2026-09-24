<script lang="ts">
  // Remote live browser: a tab's daemon-owned Chromium, streamed into this
  // pane as screencast frames over `WS /ws/browser/{tab_id}/live` and driven
  // back with input frames (docs/contracts/ws.md §1b). Unlike the desktop
  // app's native child webview it works anywhere the Otto UI runs — a remote
  // web session, the PWA, a phone — and it is the surface an agent drives
  // *visibly*: Take over / Hand back, and approval cards for the outward
  // requests the daemon holds while an agent drives.
  //
  // Pure logic lives next door and is unit-tested: geometry.ts (client ↔
  // page coordinates through letterboxing + DPR), keys.ts (which keys stay
  // with Otto), throttle.ts (move/wheel coalescing), connection.ts (reconnect
  // reducer), protocol.ts (frame parsing + copy), approvalFacts.ts. This file
  // is the DOM glue.
  //
  // Keyboard model: a visually-hidden <textarea> is the focus sink, so IME
  // composition, soft keyboards and paste all work. Click (or Tab onto it)
  // gives the page the keyboard; Esc hands it back (⇧Esc sends Esc to the
  // page). Otto's own chords (⌘K, ⌘T, ⌘J…) never reach the page — lib/keys.ts
  // handles them first on window capture.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { auth } from '../../../lib/stores/auth.svelte';
  import { ApiError } from '../../../lib/api/client';
  import { mcpCpApi } from '../../../lib/api/mcp';
  import * as liveApi from '../../../lib/api/browserLive';
  import { browserLive } from '../../../lib/stores/browserLive.svelte';
  import { formatBytes } from '../../../lib/metric-format';
  import type { BrowserLiveSession, BrowserTab, McpApproval } from '../../../lib/api/types';
  import ApprovalCard from './ApprovalCard.svelte';
  import type { ApprovalChoice } from './approvalFacts';
  import PageDialogCard from './PageDialogCard.svelte';
  import { clientToPage, viewportChanged, viewportFor, type ViewportRequest } from './geometry';
  import { keyPayload, routeKey } from './keys';
  import { coalesce, mergeWheel, wheelPixels, type Clock, type WheelDelta } from './throttle';
  import {
    EMPTY_METER,
    INITIAL,
    isStale,
    latencySample,
    meterFps,
    meterFrame,
    meterLatency,
    reduce,
    type ConnEvent,
    type ConnState,
  } from './connection';
  import {
    blockedCopy,
    buttonName,
    closedReason,
    driverFor,
    encode,
    parseBinaryFrame,
    parseServer,
    safeCursor,
    type ClientMsg,
    type FrameHeader,
    type LiveViewState,
    type ServerMsg,
  } from './protocol';

  interface Props {
    tab: BrowserTab;
    /** The remote page navigated (link click, redirect, history). */
    onnav?: (url: string, title: string) => void;
    /** Connection + session state for the parent's toolbar. */
    onstate?: (s: LiveViewState) => void;
    /** ⌘L inside the page: focus Otto's address bar. */
    onfocusurl?: () => void;
    /** "Open in new tab" for a page's popup. */
    onnewtab?: (url: string) => void;
    /** "Switch to Reader" from the ended state. */
    onreader?: () => void;
  }
  let { tab, onnav, onstate, onfocusurl, onnewtab, onreader }: Props = $props();

  const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  const MOVE_INTERVAL_MS = 33; // ≈30 moves/s — the daemon coalesces further (≥ 8 ms)
  const RESIZE_DEBOUNCE_MS = 120;
  const PASTE_CAP = 100_000; // ws.md: paste ≤ 100 000 chars

  let surfaceEl = $state<HTMLDivElement | null>(null);
  let canvasEl = $state<HTMLCanvasElement | null>(null);
  let sinkEl = $state<HTMLTextAreaElement | null>(null);

  let conn = $state<ConnState>(INITIAL);
  let meter = $state(EMPTY_METER);
  let now = $state(Date.now());
  let session = $state<BrowserLiveSession | null>(null);
  let cursor = $state('default');
  let kbdFocused = $state(false);
  /** Watch-only: this viewer lacks Edit (the daemon said `forbidden`). */
  let watchOnly = $state(false);
  /** This viewer took control FROM an agent — offer Hand back. */
  let tookOverFromAgent = $state(false);
  let banner = $state<{ tone: 'warn' | 'info'; text: string; action?: { label: string; run: () => void } } | null>(null);
  let pageDialog = $state<Extract<ServerMsg, { type: 'dialog' }> | null>(null);
  let approval = $state<{ id: string; title: string; detail: McpApproval | null } | null>(null);
  let deciding = $state(false);
  let decideError = $state('');
  /** Why a (re)connect failed, shown in the reconnecting banner. */
  let errorNote = $state('');
  /** Box of the surface in client px (kept by the ResizeObserver). */
  let box = $state({ width: 0, height: 0 });
  /** Size of the last decoded frame. */
  let imageSize = $state({ width: 0, height: 0 });
  let header: FrameHeader | null = null;

  let sock: WebSocket | null = null;
  /** Bumped on every (re)connect: late events from an old socket are dropped. */
  let gen = 0;
  let bitmap: ImageBitmap | null = null;
  let lastViewport: ViewportRequest | null = null;
  /** The URL the remote last reported (or we last asked for): the tab.url
   *  watcher only sends `goto` when the address bar changed it. */
  let knownUrl = '';
  const downKeys = new Map<string, { key: string; code: string }>();
  let lastDown = { t: 0, x: 0, y: 0, button: -1, count: 0 };

  const dispatch = (ev: ConnEvent) => (conn = reduce(conn, ev, Math.random()));
  const driver = $derived(session ? driverFor(session.controller, session.controller_user_id, auth.me?.id ?? null) : 'me');
  const stale = $derived(isStale(conn));
  const fps = $derived(meterFps(meter, now));
  const hasFrame = $derived(imageSize.width > 0);
  /** Input goes out only while live, while this viewer may drive, and while
   *  no card needs an answer first. */
  const inputLive = $derived(conn.status === 'live' && driver !== 'agent' && !watchOnly && !pageDialog);

  function send(msg: ClientMsg): void {
    if (sock && sock.readyState === WebSocket.OPEN) sock.send(encode(msg));
  }

  /** Send a user action (not a hover move): marks the start of a latency
   *  sample for the next frame. */
  function act(msg: ClientMsg): void {
    send(msg);
    dispatch({ type: 'input', at: Date.now() });
  }

  // ── connection ─────────────────────────────────────────────────────────

  function currentViewport(): ViewportRequest {
    let size = box;
    // Before the ResizeObserver's first callback (the session is opened on
    // mount), measure directly so the first frames already fit the pane.
    if (!(size.width > 0) && surfaceEl) {
      const r = surfaceEl.getBoundingClientRect();
      size = { width: r.width, height: r.height };
    }
    return viewportFor(size.width > 0 ? size : { width: 1280, height: 800 }, window.devicePixelRatio || 1);
  }

  async function connect(t: BrowserTab): Promise<void> {
    const mine = ++gen;
    dispatch({ type: 'connect' });
    try {
      const vp = currentViewport();
      // Opens the session, or re-attaches to this user's existing one.
      const s = await liveApi.openLive(t.id, { viewport: vp, url: t.url || undefined });
      if (mine !== gen) return;
      lastViewport = vp;
      session = s;
      knownUrl = s.url || t.url;
    } catch (e) {
      if (mine !== gen) return;
      const status = e instanceof ApiError ? e.status : 0;
      const code = e instanceof ApiError ? e.code : '';
      if (status === 409 && code === 'engine_not_installed') {
        // The engine went away: back to the enable step.
        void browserLive.load();
        dispatch({ type: 'ended', reason: 'The live browser engine is not installed.' });
      } else if (status === 409) {
        dispatch({ type: 'ended', reason: 'Someone else has this tab open live. Open the page in a new tab to browse it yourself.' });
      } else if (status === 429) {
        dispatch({ type: 'ended', reason: 'Too many live tabs are open on this Mac. Close one, then reconnect.' });
      } else if (status === 400 || status === 403 || status === 404) {
        dispatch({ type: 'ended', reason: e instanceof Error && e.message ? e.message : 'This page can’t be opened live.' });
      } else {
        errorNote = e instanceof Error ? e.message : String(e);
        dispatch({ type: 'close', code: 1006, reason: 'Couldn’t start the live browser.' });
      }
      return;
    }
    const s = liveApi.openLiveSocket(t.id);
    sock = s;
    s.onopen = () => {
      if (mine !== gen) return;
      errorNote = '';
      dispatch({ type: 'open' });
      // The pane may have changed size while the session was opening.
      sendResize(true);
    };
    s.onmessage = (ev) => {
      if (mine !== gen) return;
      if (typeof ev.data === 'string') {
        const msg = parseServer(ev.data);
        if (msg) onServer(msg);
      } else if (ev.data instanceof ArrayBuffer) {
        const f = parseBinaryFrame(ev.data);
        if (f) enqueueFrame(new Blob([new Uint8Array(f.bytes)], { type: f.header.mime || 'image/jpeg' }), f.header);
      }
    };
    s.onclose = (ev) => {
      if (mine !== gen) return;
      sock = null;
      releaseKeys();
      dispatch({ type: 'close', code: ev.code, reason: ev.reason });
    };
  }

  function disconnect(): void {
    gen++;
    moves.cancel();
    wheels.cancel();
    const s = sock;
    sock = null;
    if (s) {
      s.onclose = null;
      try {
        s.close(1000);
      } catch {
        /* already closing */
      }
    }
  }

  // (Re)connect whenever the tab changes; tear down on unmount. The session
  // itself stays on the daemon (idle-reaped) so coming back re-attaches.
  $effect(() => {
    const id = tab.id;
    untrack(() => {
      conn = INITIAL;
      session = null;
      approval = null;
      pageDialog = null;
      banner = null;
      watchOnly = false;
      tookOverFromAgent = false;
      imageSize = { width: 0, height: 0 };
      bitmap?.close();
      bitmap = null;
      header = null;
      void connect(tab);
    });
    return () => {
      void id;
      disconnect();
    };
  });

  // Backoff: schedule the retry the reducer asked for.
  $effect(() => {
    if (conn.status !== 'reconnecting') return;
    const t = setTimeout(() => {
      dispatch({ type: 'retry' });
      untrack(() => void connect(tab));
    }, conn.retryInMs);
    return () => clearTimeout(t);
  });

  function retryNow(): void {
    disconnect();
    conn = INITIAL;
    void connect(tab);
  }

  // A 1 s clock for the fps readout.
  $effect(() => {
    if (conn.status !== 'live') return;
    const tick = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(tick);
  });

  function onServer(msg: ServerMsg): void {
    switch (msg.type) {
      case 'state': {
        const prev = session;
        session = msg.session;
        if (prev?.controller === 'agent' && msg.session.controller === 'human' && msg.session.controller_user_id === auth.me?.id) {
          tookOverFromAgent = true;
        }
        if (msg.session.controller === 'agent') {
          tookOverFromAgent = false;
          releaseKeys();
        }
        if (msg.session.state === 'crashed' || msg.session.state === 'closed') {
          dispatch({ type: 'ended', reason: closedReason(msg.session.state) });
          disconnect();
          return;
        }
        applyNav(msg.session);
        break;
      }
      case 'cursor':
        cursor = safeCursor(msg.cursor);
        break;
      case 'dialog':
        pageDialog = msg;
        break;
      case 'blocked':
        banner = { tone: 'warn', text: blockedCopy(msg.host) };
        break;
      case 'popup':
        banner = {
          tone: 'info',
          text: 'The page tried to open a new window.',
          action: onnewtab ? { label: 'Open in new tab', run: () => onnewtab?.(msg.url) } : undefined,
        };
        break;
      case 'download':
        if (msg.status === 'quarantined') {
          toasts.info(
            'Download saved to quarantine',
            `${msg.filename}${msg.bytes ? ` · ${formatBytes(msg.bytes)}` : ''} — kept on this Mac, never opened.`,
          );
        } else {
          toasts.warn('Download blocked', `${msg.filename} — downloads are turned off in Settings → Browser.`);
        }
        break;
      case 'approval':
        void onApproval(msg.approval_id, msg.status, msg.title);
        break;
      case 'error':
        if (msg.code === 'forbidden') {
          watchOnly = true;
          releaseKeys();
        } else if (msg.code === 'nav_failed') {
          toasts.error("Couldn't open the page", msg.message);
        } else if (msg.code === 'engine_unavailable') {
          errorNote = msg.message;
        }
        // not_driver: the drive bar already says an agent is driving;
        // input_failed / bad_frame: transient, nothing for a person to do.
        break;
      case 'closed':
        dispatch({ type: 'ended', reason: closedReason(msg.reason) });
        disconnect();
        break;
    }
  }

  function applyNav(s: BrowserLiveSession): void {
    if (s.url && s.url !== knownUrl) {
      knownUrl = s.url;
      onnav?.(s.url, s.title);
    } else if (s.title && s.title !== tab.title) {
      onnav?.(s.url || tab.url, s.title);
    }
  }

  // The address bar (parent) changed the tab's URL → navigate the remote.
  $effect(() => {
    const url = tab.url;
    if (conn.status !== 'live') return;
    untrack(() => {
      if (url && url !== knownUrl) {
        knownUrl = url;
        act({ type: 'nav', action: 'goto', url });
      }
    });
  });

  $effect(() => {
    onstate?.({ status: conn.status, session });
  });

  // ── approvals ──────────────────────────────────────────────────────────

  async function onApproval(id: string, status: 'pending' | 'approved' | 'denied', title: string): Promise<void> {
    if (status !== 'pending') {
      if (approval?.id === id) {
        approval = null;
        deciding = false;
      }
      return;
    }
    approval = { id, title, detail: null };
    decideError = '';
    try {
      const rows = await mcpCpApi.cpApprovals('pending');
      const row = rows.find((r) => r.id === id) ?? null;
      if (approval?.id === id) approval = { id, title, detail: row };
    } catch {
      /* no mcp:view — the card still shows the title and the decisions */
    }
  }

  async function decide(choice: ApprovalChoice): Promise<void> {
    const a = approval;
    if (!a || deciding) return;
    let note: string | undefined;
    if (choice === 'deny') {
      const why = await confirmer.promptText("Tell the agent why (optional). It won't send this request.", {
        title: 'Deny the request',
        confirmLabel: 'Deny',
        placeholder: 'Wrong time slot',
      });
      if (why === null) return;
      note = why.trim() || undefined;
    } else if (choice === 'take_over') {
      note = 'Taken over by the user — they will finish this step themselves.';
    }
    deciding = true;
    decideError = '';
    try {
      await mcpCpApi.cpDecide(a.id, { approved: choice === 'approve', note });
      if (choice === 'take_over') takeOver();
      if (approval?.id === a.id) approval = null;
    } catch (e) {
      decideError =
        e instanceof ApiError && e.status === 403
          ? 'You can’t decide MCP approvals in this workspace. Ask an admin, or take over the page.'
          : `Couldn’t record your decision: ${e instanceof Error ? e.message : String(e)}`;
    } finally {
      deciding = false;
    }
  }

  function answerDialog(accept: boolean, promptText?: string): void {
    send({ type: 'dialog', accept, ...(promptText !== undefined ? { prompt_text: promptText } : {}) });
    pageDialog = null;
  }

  // ── frames ─────────────────────────────────────────────────────────────
  // Decode off the main thread (createImageBitmap) and keep only the LATEST
  // pending frame: if the viewer's machine falls behind, intermediate frames
  // are skipped rather than queued. The ack goes out after the frame is
  // DRAWN — that is what paces the daemon's screencast to this viewer.

  let pendingFrame: { blob: Blob; header: FrameHeader } | null = null;
  let decoding = false;

  function enqueueFrame(blob: Blob, h: FrameHeader): void {
    pendingFrame = { blob, header: h };
    if (!decoding) void pump();
  }

  async function pump(): Promise<void> {
    decoding = true;
    const mine = gen;
    while (pendingFrame) {
      const f = pendingFrame;
      pendingFrame = null;
      try {
        const bmp = await createImageBitmap(f.blob);
        if (mine !== gen) {
          bmp.close();
          break;
        }
        bitmap?.close();
        bitmap = bmp;
        header = f.header;
        imageSize = { width: bmp.width, height: bmp.height };
        draw();
        const at = Date.now();
        const lag = latencySample(conn, at);
        if (lag !== null) meter = meterLatency(meter, lag);
        dispatch({ type: 'frame', at });
        meter = meterFrame(meter, at);
        now = at;
      } catch {
        /* a corrupt frame: skip it, the next one repaints */
      }
      send({ type: 'ack', seq: f.header.seq });
    }
    decoding = false;
  }

  function draw(): void {
    const c = canvasEl;
    if (!c || !bitmap) return;
    const dpr = window.devicePixelRatio || 1;
    const w = Math.max(1, Math.round(box.width * dpr));
    const h = Math.max(1, Math.round(box.height * dpr));
    if (c.width !== w || c.height !== h) {
      c.width = w;
      c.height = h;
    }
    const ctx = c.getContext('2d');
    if (!ctx) return;
    ctx.clearRect(0, 0, w, h);
    const scale = Math.min(w / bitmap.width, h / bitmap.height);
    const dw = bitmap.width * scale;
    const dh = bitmap.height * scale;
    ctx.imageSmoothingQuality = 'high';
    ctx.drawImage(bitmap, (w - dw) / 2, (h - dh) / 2, dw, dh);
  }

  // Keep the canvas matched to the pane and the remote viewport matched to
  // the pane (debounced — a window drag fires dozens of sizes).
  let resizeTimer: ReturnType<typeof setTimeout> | null = null;
  function sendResize(force = false): void {
    const vp = currentViewport();
    if (!force && !viewportChanged(lastViewport, vp)) return;
    lastViewport = vp;
    send({ type: 'resize', ...vp });
  }

  $effect(() => {
    const el = surfaceEl;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      const r = el.getBoundingClientRect();
      box = { width: r.width, height: r.height };
      draw();
      if (resizeTimer) clearTimeout(resizeTimer);
      resizeTimer = setTimeout(() => sendResize(), RESIZE_DEBOUNCE_MS);
    });
    ro.observe(el);
    // A DPR change (window dragged to another display) doesn't resize the
    // element — re-evaluate on the resolution media query too.
    const mq = matchMedia(`(resolution: ${window.devicePixelRatio || 1}dppx)`);
    const onDpr = () => {
      draw();
      sendResize();
    };
    mq.addEventListener('change', onDpr);
    return () => {
      ro.disconnect();
      mq.removeEventListener('change', onDpr);
      if (resizeTimer) clearTimeout(resizeTimer);
    };
  });

  // ── pointer input ───────────────────────────────────────────────────────

  const clock: Clock = {
    now: () => performance.now(),
    setTimeout: (fn, ms) => setTimeout(fn, ms),
    clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  };
  const moves = coalesce<ClientMsg>(send, MOVE_INTERVAL_MS, clock);
  let wheelAcc: WheelDelta | null = null;
  const wheels = coalesce<null>(
    () => {
      const w = wheelAcc;
      wheelAcc = null;
      if (w) act({ type: 'mouse', action: 'wheel', x: w.x, y: w.y, delta_x: w.dx, delta_y: w.dy, modifiers: w.modifiers });
    },
    MOVE_INTERVAL_MS,
    clock,
  );

  function toPage(clientX: number, clientY: number): { x: number; y: number } | null {
    if (!surfaceEl || !header || !hasFrame) return null;
    const r = surfaceEl.getBoundingClientRect();
    return clientToPage({ x: clientX, y: clientY }, { left: r.left, top: r.top, width: r.width, height: r.height }, imageSize, header);
  }

  function mods(e: MouseEvent | WheelEvent): number {
    return (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
  }

  function clickCount(e: PointerEvent): number {
    const t = performance.now();
    const near = Math.abs(e.clientX - lastDown.x) < 5 && Math.abs(e.clientY - lastDown.y) < 5;
    const count = near && e.button === lastDown.button && t - lastDown.t < 500 ? lastDown.count + 1 : 1;
    lastDown = { t, x: e.clientX, y: e.clientY, button: e.button, count };
    return count;
  }

  // Touch: tap = click; a one- or two-finger drag scrolls (wheel at the
  // gesture's centroid). Pinch and long-press are left alone.
  const touches = new Map<number, { x: number; y: number }>();
  let tap: { id: number; x: number; y: number; t: number; moved: boolean } | null = null;
  let lastCentroid: { x: number; y: number } | null = null;
  const centroid = () => {
    let x = 0;
    let y = 0;
    for (const p of touches.values()) {
      x += p.x;
      y += p.y;
    }
    return { x: x / touches.size, y: y / touches.size };
  };

  function onPointerDown(e: PointerEvent): void {
    if (!inputLive) return;
    if (e.pointerType === 'touch') {
      touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
      tap = touches.size === 1 ? { id: e.pointerId, x: e.clientX, y: e.clientY, t: performance.now(), moved: false } : null;
      lastCentroid = centroid();
      surfaceEl?.setPointerCapture(e.pointerId);
      return;
    }
    e.preventDefault(); // keep focus in the sink, no text selection on Otto
    captureKeyboard();
    const p = toPage(e.clientX, e.clientY);
    if (!p) return;
    moves.flush();
    surfaceEl?.setPointerCapture(e.pointerId);
    act({ type: 'mouse', action: 'down', ...p, button: buttonName(e.button), buttons: e.buttons, click_count: clickCount(e), modifiers: mods(e) });
  }

  function onPointerMove(e: PointerEvent): void {
    if (!inputLive) return;
    if (e.pointerType === 'touch') {
      if (!touches.has(e.pointerId)) return;
      touches.set(e.pointerId, { x: e.clientX, y: e.clientY });
      if (tap && (Math.abs(e.clientX - tap.x) > 8 || Math.abs(e.clientY - tap.y) > 8)) tap.moved = true;
      if (tap && !tap.moved) return;
      const c = centroid();
      const p = toPage(c.x, c.y);
      if (lastCentroid && p) {
        wheelAcc = mergeWheel(wheelAcc, { ...p, dx: lastCentroid.x - c.x, dy: lastCentroid.y - c.y, modifiers: 0 });
        wheels.push(null);
      }
      lastCentroid = c;
      return;
    }
    const p = toPage(e.clientX, e.clientY);
    if (!p) return;
    moves.push({ type: 'mouse', action: 'move', ...p, button: 'none', buttons: e.buttons, modifiers: mods(e) });
  }

  function onPointerUp(e: PointerEvent): void {
    if (e.pointerType === 'touch') {
      touches.delete(e.pointerId);
      lastCentroid = touches.size ? centroid() : null;
      if (tap && tap.id === e.pointerId && !tap.moved && performance.now() - tap.t < 600 && inputLive) {
        const p = toPage(tap.x, tap.y);
        if (p) {
          send({ type: 'mouse', action: 'move', ...p, button: 'none', buttons: 0, modifiers: 0 });
          act({ type: 'mouse', action: 'down', ...p, button: 'left', buttons: 1, click_count: 1, modifiers: 0 });
          send({ type: 'mouse', action: 'up', ...p, button: 'left', buttons: 0, click_count: 1, modifiers: 0 });
        }
      }
      if (!touches.size) {
        tap = null;
        wheels.flush();
      }
      return;
    }
    if (!inputLive) return;
    const p = toPage(e.clientX, e.clientY);
    if (!p) return;
    moves.flush();
    act({ type: 'mouse', action: 'up', ...p, button: buttonName(e.button), buttons: e.buttons, click_count: lastDown.count || 1, modifiers: mods(e) });
  }

  function onPointerCancel(e: PointerEvent): void {
    touches.delete(e.pointerId);
    if (!touches.size) {
      tap = null;
      lastCentroid = null;
    }
  }

  // The pointer surface is a picture of the remote screen, not a control of
  // its own — the accessible control is the keyboard sink inside it — so its
  // listeners are attached here rather than as element attributes. Wheel
  // must be non-passive anyway to stop the Otto page scrolling (Svelte
  // attaches `onwheel` passively). Right-click goes to the page, so the
  // viewer's own context menu is suppressed.
  $effect(() => {
    const el = surfaceEl;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!inputLive) return;
      e.preventDefault();
      const p = toPage(e.clientX, e.clientY);
      if (!p) return;
      const h = header?.device_height ?? 800;
      wheelAcc = mergeWheel(wheelAcc, {
        ...p,
        dx: wheelPixels(e.deltaX, e.deltaMode, h),
        dy: wheelPixels(e.deltaY, e.deltaMode, h),
        modifiers: mods(e),
      });
      moves.flush();
      wheels.push(null);
    };
    const onContext = (e: MouseEvent) => e.preventDefault();
    el.addEventListener('wheel', onWheel, { passive: false });
    el.addEventListener('contextmenu', onContext);
    el.addEventListener('pointerdown', onPointerDown);
    el.addEventListener('pointermove', onPointerMove);
    el.addEventListener('pointerup', onPointerUp);
    el.addEventListener('pointercancel', onPointerCancel);
    return () => {
      el.removeEventListener('wheel', onWheel);
      el.removeEventListener('contextmenu', onContext);
      el.removeEventListener('pointerdown', onPointerDown);
      el.removeEventListener('pointermove', onPointerMove);
      el.removeEventListener('pointerup', onPointerUp);
      el.removeEventListener('pointercancel', onPointerCancel);
    };
  });

  // ── keyboard ────────────────────────────────────────────────────────────

  function captureKeyboard(): void {
    if (!inputLive) return;
    sinkEl?.focus({ preventScroll: true });
  }

  /** Esc: give the keyboard back to Otto. Blurring keeps the sequential
   *  focus starting point on the sink, so Tab moves on to the next control. */
  function release(): void {
    sinkEl?.blur();
  }

  /** Key-up for everything still held (blur, lost control, disconnect) so
   *  the remote never sees a stuck modifier. */
  function releaseKeys(): void {
    for (const k of downKeys.values()) {
      send({ type: 'key', action: 'up', key: k.key, code: k.code, modifiers: 0 });
    }
    downKeys.clear();
  }

  function onSinkKeyDown(e: KeyboardEvent): void {
    switch (routeKey(e, isMac)) {
      case 'app':
      case 'ignore':
      case 'paste':
        return;
      case 'release':
        e.preventDefault();
        release();
        return;
      case 'url':
        e.preventDefault();
        onfocusurl?.();
        return;
      case 'reload':
        e.preventDefault();
        reload();
        return;
      case 'forward':
        e.preventDefault();
        e.stopPropagation();
        if (!inputLive) return;
        moves.flush();
        downKeys.set(e.code, { key: e.key, code: e.code });
        act({ type: 'key', action: 'down', ...keyPayload(e, 'down') });
    }
  }

  function onSinkKeyUp(e: KeyboardEvent): void {
    if (!downKeys.has(e.code)) return;
    e.preventDefault();
    downKeys.delete(e.code);
    send({ type: 'key', action: 'up', ...keyPayload(e, 'up') });
  }

  function onSinkInput(e: Event): void {
    const ie = e as InputEvent;
    if (ie.isComposing || !sinkEl) return;
    const v = sinkEl.value;
    sinkEl.value = '';
    if (v && inputLive) act({ type: 'text', text: v });
  }

  function onCompositionUpdate(e: CompositionEvent): void {
    if (!inputLive) return;
    const len = e.data.length;
    send({ type: 'ime', text: e.data, selection_start: len, selection_end: len });
  }

  function onCompositionEnd(e: CompositionEvent): void {
    if (sinkEl) sinkEl.value = '';
    if (e.data && inputLive) act({ type: 'text', text: e.data });
  }

  function onPaste(e: ClipboardEvent): void {
    e.preventDefault();
    if (!inputLive) return;
    let text = e.clipboardData?.getData('text/plain') ?? '';
    if (!text) return;
    if (text.length > PASTE_CAP) {
      text = text.slice(0, PASTE_CAP);
      toasts.warn('Paste shortened', `Only the first ${PASTE_CAP.toLocaleString()} characters were sent to the page.`);
    }
    act({ type: 'paste', text });
  }

  // ── navigation / control (called by the parent's toolbar) ───────────────

  export function back(): void {
    act({ type: 'nav', action: 'back' });
  }
  export function forward(): void {
    act({ type: 'nav', action: 'forward' });
  }
  export function reload(): void {
    act({ type: 'nav', action: 'reload' });
  }
  export function stop(): void {
    send({ type: 'nav', action: 'stop' });
  }

  function takeOver(): void {
    tookOverFromAgent = session?.controller === 'agent' || tookOverFromAgent;
    send({ type: 'control', action: 'take_over' });
  }
  function handBack(): void {
    releaseKeys();
    release();
    tookOverFromAgent = false;
    send({ type: 'control', action: 'hand_back' });
  }

  const statusText = $derived.by(() => {
    switch (conn.status) {
      case 'idle':
      case 'connecting':
        return hasFrame ? 'Reconnecting…' : 'Starting the live browser…';
      case 'live':
        return session?.state === 'starting' ? 'Starting…' : 'Live';
      case 'reconnecting':
        return `Connection lost. Reconnecting (attempt ${conn.attempt})…`;
      case 'ended':
        return 'Live session ended';
    }
  });

  const engineLabel = $derived(
    session ? `${session.build === 'chrome' ? 'Chrome' : 'Headless shell'} ${session.version.split('.')[0]}` : '',
  );

  const isTouch = typeof matchMedia !== 'undefined' && matchMedia('(pointer: coarse)').matches;
</script>

<div class="rlv" data-testid="remote-live" data-status={conn.status}>
  {#if driver === 'agent'}
    <div class="drive-bar" role="status" data-testid="live-drive-bar">
      <Icon name="cursor" size={14} />
      <span class="who"><span class="chip">Agent</span> An agent is driving this page. Its actions pause while you drive.</span>
      <span class="grow"></span>
      <button class="btn small primary" onclick={takeOver} disabled={watchOnly}>Take over</button>
    </div>
  {:else if tookOverFromAgent}
    <div class="drive-bar mine" role="status" data-testid="live-drive-bar">
      <Icon name="user" size={14} />
      <span class="who">You have control. The agent is paused.</span>
      <span class="grow"></span>
      <button class="btn small" onclick={handBack}>Hand back</button>
    </div>
  {:else if watchOnly}
    <div class="drive-bar mine" role="status" data-testid="live-drive-bar">
      <Icon name="eye" size={14} />
      <span class="who">You're watching. Driving this page needs Edit access to Browser in this workspace.</span>
    </div>
  {/if}

  <div class="surface" class:stale class:kbd={kbdFocused} style:cursor={inputLive ? cursor : 'default'} bind:this={surfaceEl}>
    <canvas bind:this={canvasEl} aria-hidden="true"></canvas>

    <textarea
      class="sink"
      bind:this={sinkEl}
      aria-label={`Live page${session?.title ? `: ${session.title}` : ''}. Typing goes to the page; Escape gives the keyboard back to Otto.`}
      aria-roledescription="remote browser"
      autocapitalize="off"
      autocomplete="off"
      spellcheck="false"
      disabled={!inputLive}
      onkeydown={onSinkKeyDown}
      onkeyup={onSinkKeyUp}
      oninput={onSinkInput}
      oncompositionupdate={onCompositionUpdate}
      oncompositionend={onCompositionEnd}
      onpaste={onPaste}
      onfocus={() => (kbdFocused = true)}
      onblur={() => {
        kbdFocused = false;
        releaseKeys();
      }}
    ></textarea>
  </div>

  <!-- chrome over the frame -->
  <div class="badge" class:warn={conn.status !== 'live'} data-testid="live-badge">
    <span class="dot" aria-hidden="true"></span>
    <span>{statusText}</span>
    {#if conn.status === 'live' && engineLabel}<span class="dim">· {engineLabel}</span>{/if}
    {#if conn.status === 'live' && session && session.viewers > 1}<span class="dim">· {session.viewers} watching</span>{/if}
  </div>

  {#if conn.status === 'live' && hasFrame}
    <div class="meter" data-testid="live-meter" title="Frames per second · input-to-frame latency">
      {fps} fps{#if meter.latencyMs !== null} · {meter.latencyMs} ms{/if}
    </div>
  {/if}

  {#if kbdFocused}
    <div class="hint" role="status">
      Typing goes to the page · <kbd>Esc</kbd> releases · <kbd>⇧Esc</kbd> sends Esc
    </div>
  {/if}

  {#if isTouch && inputLive}
    <button class="icon-btn kbd-btn" onclick={captureKeyboard} aria-label="Show keyboard" title="Show keyboard">
      <Icon name="edit" size={14} />
    </button>
  {/if}

  {#if !hasFrame && conn.status !== 'ended' && conn.status !== 'reconnecting'}
    <div class="center" role="status" aria-live="polite">
      <Icon name="globe" size={16} />
      <span>{conn.status === 'live' ? 'Loading the page…' : 'Starting the live browser…'}</span>
    </div>
  {/if}

  {#if conn.status === 'reconnecting' || (conn.status === 'connecting' && hasFrame)}
    <div class="banner" role="status" aria-live="polite" data-testid="live-reconnecting">
      <Icon name="warning" size={14} />
      <span class="text">{statusText}</span>
      {#if errorNote}<span class="dim" title={errorNote}>{errorNote}</span>{/if}
      <button class="btn small" onclick={retryNow}>Retry now</button>
    </div>
  {:else if banner}
    <div class="banner" class:info={banner.tone === 'info'} role="status" data-testid="live-banner">
      <Icon name={banner.tone === 'info' ? 'info' : 'shield'} size={14} />
      <span class="text">{banner.text}</span>
      {#if banner.action}
        <button
          class="btn small"
          onclick={() => {
            banner?.action?.run();
            banner = null;
          }}>{banner.action.label}</button
        >
      {/if}
      <button class="icon-btn" onclick={() => (banner = null)} aria-label="Dismiss" title="Dismiss">
        <Icon name="x" size={12} />
      </button>
    </div>
  {/if}

  {#if conn.status === 'ended'}
    <div class="center ended" role="status" data-testid="live-ended">
      <h2>The live session ended</h2>
      <p>{conn.reason || closedReason('closed')}</p>
      <div class="row">
        <button class="btn primary" onclick={retryNow}><Icon name="refresh" size={13} /> Reconnect</button>
        {#if onreader}<button class="btn ghost" onclick={onreader}>Switch to Reader</button>{/if}
      </div>
    </div>
  {/if}

  {#if approval || pageDialog}
    <div class="card-host">
      {#if approval}
        <ApprovalCard
          id={approval.id}
          title={approval.title}
          detail={approval.detail}
          profile={session?.profile ?? null}
          busy={deciding}
          error={decideError}
          ondecide={(c) => void decide(c)}
        />
      {:else if pageDialog}
        <PageDialogCard
          kind={pageDialog.dialog_type}
          message={pageDialog.message}
          defaultPrompt={pageDialog.default_prompt}
          url={pageDialog.url}
          canAnswer={driver !== 'agent' && !watchOnly}
          onanswer={answerDialog}
        />
      {/if}
    </div>
  {/if}
</div>

<style>
  .rlv {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface-2);
  }
  .drive-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 36px;
    padding: 4px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .drive-bar.mine {
    background: var(--info-soft);
  }
  .drive-bar .who {
    min-width: 0;
  }
  .drive-bar .chip {
    margin-inline-end: 4px;
  }
  .grow {
    flex: 1;
  }
  .surface {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow: hidden;
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
  }
  /* Keyboard focus lives in the invisible sink: show it as an inset ring on
     the frame (an outset ring would be clipped by the pane). */
  .surface.kbd {
    box-shadow: inset 0 0 0 2px color-mix(in srgb, var(--accent) 70%, transparent);
  }
  canvas {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    display: block;
    transition: opacity 140ms ease-out, filter 140ms ease-out;
  }
  .surface.stale canvas {
    opacity: 0.55;
    filter: grayscale(0.6);
  }
  .sink {
    position: absolute;
    inset-inline-start: 0;
    top: 0;
    width: 1px;
    height: 1px;
    padding: 0;
    border: 0;
    opacity: 0;
    resize: none;
    overflow: hidden;
    pointer-events: none;
    /* 16px so iOS Safari doesn't zoom the page when the sink takes focus —
       the element is invisible, nothing is read at this size. */
    font-size: 16px;
  }
  .badge {
    position: absolute;
    inset-inline-start: 10px;
    top: 10px;
    display: flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 8px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-xs);
    pointer-events: none;
  }
  .drive-bar ~ .badge,
  .drive-bar ~ .kbd-btn {
    top: 46px;
  }
  .badge .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--status-working);
  }
  .badge.warn .dot {
    background: var(--status-warn);
  }
  .dim {
    color: var(--text-dim);
  }
  .meter {
    position: absolute;
    inset-inline-end: 10px;
    bottom: 10px;
    padding: 1px 6px;
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    pointer-events: none;
  }
  .hint {
    position: absolute;
    inset-inline: 0;
    bottom: 10px;
    width: fit-content;
    max-width: calc(100% - 24px);
    margin-inline: auto;
    padding: 2px 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    pointer-events: none;
  }
  kbd {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text);
  }
  .kbd-btn {
    position: absolute;
    inset-inline-end: 10px;
    top: 10px;
    width: 36px;
    height: 36px;
    background: var(--surface);
    border: 1px solid var(--border);
  }
  .center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--text-dim);
    font-size: var(--fs-m);
    pointer-events: none;
  }
  .center.ended {
    pointer-events: auto;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    color: var(--text);
    text-align: center;
    padding: 16px;
  }
  .ended h2 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .ended p {
    margin: 0;
    color: var(--text-dim);
    max-width: 44ch;
  }
  .ended .row {
    display: flex;
    gap: 6px;
    margin-top: 4px;
  }
  .banner {
    position: absolute;
    inset-inline: 12px;
    top: 42px;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 8px 6px 10px;
    border: 1px solid color-mix(in srgb, var(--warning) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .banner.info {
    border-color: color-mix(in srgb, var(--info) 35%, transparent);
  }
  .drive-bar ~ .banner {
    top: 78px;
  }
  .banner > :global(svg) {
    color: var(--warning);
    flex: none;
  }
  .banner.info > :global(svg) {
    color: var(--info);
  }
  .banner .text {
    flex: 1;
    min-width: 0;
  }
  .banner .dim {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .card-host {
    position: absolute;
    inset-inline-start: 16px;
    top: 44px;
    bottom: 16px;
    max-width: calc(100% - 32px);
    display: flex;
    align-items: flex-start;
    pointer-events: none;
  }
  .drive-bar ~ .card-host {
    top: 80px;
  }
  .card-host > :global(*) {
    pointer-events: auto;
  }
  @media (max-width: 640px) {
    .card-host,
    .drive-bar ~ .card-host {
      inset-inline: 8px;
      top: 8px;
      bottom: 8px;
      max-width: none;
      align-items: flex-end;
    }
    .drive-bar {
      flex-wrap: wrap;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    canvas {
      transition: none;
    }
  }
</style>
