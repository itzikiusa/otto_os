<script lang="ts">
  // Remote live browser: a daemon-run Chromium tab, streamed to this pane as
  // screencast frames over a WebSocket and driven back with input messages.
  // Unlike the desktop app's native child webview it works anywhere the Otto
  // UI runs — a remote web session, the PWA, a phone — and it is the surface
  // an agent drives *visibly* (ghost cursor, Take over / Hand back, approval
  // cards for outward actions).
  //
  // Pure logic lives next door and is unit-tested: geometry.ts (client ↔
  // page coordinates through letterboxing + DPR), keys.ts (which keys stay
  // with Otto), throttle.ts (move coalescing), connection.ts (reconnect
  // reducer), protocol.ts (the wire shapes). This file is the DOM glue.
  //
  // Keyboard model: a visually-hidden <textarea> is the focus sink, so IME
  // composition, soft keyboards and paste all work. Click (or Enter on the
  // focused surface) gives the page the keyboard; Esc hands it back (⇧Esc
  // sends Esc to the page). Otto's own chords (⌘K, ⌘T, ⌘J…) never reach the
  // page — lib/keys.ts handles them first on window capture.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ApiError } from '../../../lib/api/client';
  import * as liveApi from '../../../lib/api/browserLive';
  import type { BrowserTab } from '../../../lib/api/types';
  import ApprovalCard from './ApprovalCard.svelte';
  import { clientToPage, pageToBox, viewportChanged, viewportFor, type ViewportRequest } from './geometry';
  import { keyPayload, routeKey } from './keys';
  import { coalesce, mergeWheel, wheelPixels, type WheelDelta } from './throttle';
  import {
    EMPTY_METER,
    INITIAL,
    isStale,
    meterFps,
    meterFrame,
    meterRtt,
    reduce,
    type ConnEvent,
    type ConnState,
  } from './connection';
  import {
    buttonName,
    encode,
    parseBinaryFrame,
    parseServer,
    safeCursor,
    type ApprovalDecision,
    type ClientMsg,
    type FrameMeta,
    type LiveApproval,
    type LiveLock,
    type LiveViewState,
    type NavState,
    type ServerMsg,
  } from './protocol';

  interface Props {
    tab: BrowserTab;
    /** Remote element-pick mode (the urlbar's target button). */
    pickMode?: boolean;
    /** The remote page navigated (link click, redirect, history). */
    onnav?: (url: string, title: string) => void;
    /** Connection / nav / lock state for the parent's toolbar. */
    onstate?: (s: LiveViewState) => void;
    /** ⌘L inside the page: focus Otto's address bar. */
    onfocusurl?: () => void;
    /** A picked element (pick mode). */
    onpick?: (p: { selector: string; outerHtml: string; text: string; url: string }) => void;
    /** The page asked for a new tab (window.open / target=_blank). */
    onnewtab?: (url: string) => void;
    /** "Switch to Reader" from the ended state. */
    onreader?: () => void;
  }
  let { tab, pickMode = false, onnav, onstate, onfocusurl, onpick, onnewtab, onreader }: Props = $props();

  const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);
  const MOVE_INTERVAL_MS = 33; // ≈30 moves/s — plenty for hover, cheap for CDP
  const PING_MS = 2000;
  const RESIZE_DEBOUNCE_MS = 120;

  let surfaceEl = $state<HTMLDivElement | null>(null);
  let canvasEl = $state<HTMLCanvasElement | null>(null);
  let sinkEl = $state<HTMLTextAreaElement | null>(null);

  let conn: ConnState = $state(INITIAL);
  let meter = $state(EMPTY_METER);
  let now = $state(Date.now());
  let nav: NavState | null = $state(null);
  let lock: LiveLock = $state({ holder: 'none' });
  let engineLabel = $state('');
  let capabilities: string[] = $state([]);
  let cursor = $state('default');
  let approval: LiveApproval | null = $state(null);
  let deciding = $state(false);
  let ghost: { x: number; y: number; label: string } | null = $state(null);
  let kbdFocused = $state(false);
  let errorNote = $state('');
  /** Box of the surface in client px (kept by the ResizeObserver). */
  let box = $state({ width: 0, height: 0 });
  /** Size of the last decoded frame, for overlay placement. */
  let imageSize = $state({ width: 0, height: 0 });
  let frameMeta: FrameMeta | null = $state(null);

  let sock: WebSocket | null = null;
  /** Bumped on every (re)connect: late events from an old socket are dropped. */
  let gen = 0;
  let bitmap: ImageBitmap | null = null;
  let lastViewport: ViewportRequest | null = null;
  /** The URL the remote last reported (or we last asked for): the tab.url
   *  watcher only sends `navigate` when the address bar changed it. */
  let knownUrl = '';
  const downKeys = new Set<string>();
  let lastDown = { t: 0, x: 0, y: 0, button: -1, count: 0 };

  const dispatch = (ev: ConnEvent) => (conn = reduce(conn, ev, Math.random()));
  const canDrive = $derived(lock.holder !== 'agent' || !!lock.mine);
  const canPick = $derived(capabilities.includes('pick'));
  const stale = $derived(isStale(conn, now));
  const fps = $derived(meterFps(meter, now));
  const hasFrame = $derived(imageSize.width > 0);

  function send(msg: ClientMsg): void {
    if (sock && sock.readyState === WebSocket.OPEN) sock.send(encode(msg));
  }

  // ── connection ─────────────────────────────────────────────────────────

  function currentViewport(): ViewportRequest {
    return viewportFor(box.width > 0 ? box : { width: 1280, height: 800 }, window.devicePixelRatio || 1);
  }

  async function connect(tabId: string): Promise<void> {
    const mine = ++gen;
    dispatch({ type: 'connect' });
    errorNote = '';
    let wsPath: string;
    try {
      const vp = currentViewport();
      const r = await liveApi.startLive(tabId, vp);
      lastViewport = vp;
      wsPath = r.ws_path;
    } catch (e) {
      if (mine !== gen) return;
      if (e instanceof ApiError && (e.status === 403 || e.status === 404)) {
        dispatch({ type: 'ended', reason: e.message || 'This tab has no live session.' });
      } else {
        errorNote = e instanceof Error ? e.message : String(e);
        dispatch({ type: 'close', code: 1006, reason: 'Could not start the live browser.' });
      }
      return;
    }
    if (mine !== gen) return;
    const s = liveApi.openLiveSocket(wsPath);
    sock = s;
    s.onopen = () => {
      if (mine !== gen) return;
      dispatch({ type: 'open' });
      // The viewport may have changed while the socket was opening.
      sendResize(true);
    };
    s.onmessage = (ev) => {
      if (mine !== gen) return;
      if (typeof ev.data === 'string') {
        const msg = parseServer(ev.data);
        if (msg) onServer(msg);
      } else if (ev.data instanceof ArrayBuffer) {
        const f = parseBinaryFrame(ev.data);
        if (f) {
          const bytes = new Uint8Array(f.bytes); // own the buffer (BlobPart typing)
          enqueueFrame(new Blob([bytes], { type: f.mime }), { ...f.meta, seq: f.seq });
        }
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

  // (Re)connect whenever the tab changes; tear down on unmount.
  $effect(() => {
    const id = tab.id;
    untrack(() => {
      conn = INITIAL;
      nav = null;
      lock = { holder: 'none' };
      approval = null;
      ghost = null;
      imageSize = { width: 0, height: 0 };
      bitmap?.close();
      bitmap = null;
      knownUrl = tab.url;
      void connect(id);
    });
    return () => disconnect();
  });

  // Backoff: schedule the retry the reducer asked for.
  $effect(() => {
    if (conn.status !== 'reconnecting') return;
    const id = tab.id;
    const t = setTimeout(() => {
      dispatch({ type: 'retry' });
      untrack(() => void connect(id));
    }, conn.retryInMs);
    return () => clearTimeout(t);
  });

  function retryNow(): void {
    disconnect();
    conn = INITIAL;
    void connect(tab.id);
  }

  // Ping (latency + liveness) and a 1 s clock for the stale/fps readouts.
  $effect(() => {
    if (conn.status !== 'live') return;
    const ping = setInterval(() => send({ type: 'ping', t: performance.now() }), PING_MS);
    const tick = setInterval(() => (now = Date.now()), 1000);
    send({ type: 'ping', t: performance.now() });
    return () => {
      clearInterval(ping);
      clearInterval(tick);
    };
  });

  function onServer(msg: ServerMsg): void {
    switch (msg.type) {
      case 'hello':
        engineLabel = msg.engine_version ? `${msg.engine} ${msg.engine_version}` : msg.engine;
        capabilities = Array.isArray(msg.capabilities) ? msg.capabilities : [];
        applyNav(msg.nav);
        lock = msg.lock ?? { holder: 'none' };
        break;
      case 'frame': {
        const bin = atob(msg.data);
        const bytes = new Uint8Array(bin.length);
        for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
        enqueueFrame(new Blob([bytes], { type: msg.mime || 'image/jpeg' }), { ...msg.meta, seq: msg.seq });
        break;
      }
      case 'nav':
        applyNav(msg.nav);
        break;
      case 'cursor':
        cursor = safeCursor(msg.cursor);
        break;
      case 'lock':
        lock = msg.lock;
        if (lock.holder === 'agent' && !lock.mine) releaseKeys();
        break;
      case 'approval':
        approval = msg.approval;
        deciding = false;
        break;
      case 'approval_resolved':
        if (approval?.id === msg.id) {
          approval = null;
          deciding = false;
        }
        break;
      case 'agent_pointer':
        ghost = { x: msg.x, y: msg.y, label: msg.label || lock.agent_name || 'Agent' };
        break;
      case 'pick_result':
        onpick?.({ selector: msg.selector, outerHtml: msg.outer_html, text: msg.text, url: msg.url });
        break;
      case 'new_tab':
        onnewtab?.(msg.url);
        break;
      case 'pong':
        meter = meterRtt(meter, Math.max(0, performance.now() - msg.t));
        dispatch({ type: 'heartbeat', at: Date.now() });
        break;
      case 'error':
        errorNote = msg.message;
        break;
      case 'ended':
        dispatch({ type: 'ended', reason: msg.reason });
        disconnect();
        break;
    }
  }

  function applyNav(n: NavState | null | undefined): void {
    if (!n) return;
    nav = n;
    if (n.url && n.url !== knownUrl) {
      knownUrl = n.url;
      onnav?.(n.url, n.title);
    } else if (n.title && n.title !== tab.title) {
      onnav?.(n.url, n.title);
    }
  }

  // The address bar (parent) changed the tab's URL → navigate the remote.
  $effect(() => {
    const url = tab.url;
    if (conn.status !== 'live') return;
    untrack(() => {
      if (url && url !== knownUrl) {
        knownUrl = url;
        send({ type: 'navigate', url });
      }
    });
  });

  $effect(() => {
    onstate?.({ status: conn.status, nav, lock, canPick });
  });

  // ── frames ─────────────────────────────────────────────────────────────
  // Decode off the main thread (createImageBitmap) and keep only the LATEST
  // pending frame: if the viewer's machine falls behind, intermediate frames
  // are skipped rather than queued. The ack after each decode is what lets
  // the daemon pace the screencast to this client.

  let pendingFrame: { blob: Blob; meta: FrameMeta } | null = null;
  let decoding = false;

  function enqueueFrame(blob: Blob, meta: FrameMeta): void {
    pendingFrame = { blob, meta };
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
        frameMeta = f.meta;
        imageSize = { width: bmp.width, height: bmp.height };
        draw();
        const at = Date.now();
        dispatch({ type: 'frame', at });
        meter = meterFrame(meter, at);
        now = at;
      } catch {
        /* a corrupt frame: skip it, the next one repaints */
      }
      send({ type: 'frame_ack', seq: f.meta.seq });
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
  // the canvas (debounced — a window drag fires dozens of sizes).
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

  const moves = coalesce<ClientMsg>(send, MOVE_INTERVAL_MS, {
    now: () => performance.now(),
    setTimeout: (fn, ms) => setTimeout(fn, ms),
    clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
  });
  let wheelAcc: WheelDelta | null = null;
  const wheels = coalesce<null>(
    () => {
      if (wheelAcc) send({ type: 'wheel', ...wheelAcc });
      wheelAcc = null;
    },
    MOVE_INTERVAL_MS,
    {
      now: () => performance.now(),
      setTimeout: (fn, ms) => setTimeout(fn, ms),
      clearTimeout: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
    },
  );

  function toPage(clientX: number, clientY: number): { x: number; y: number } | null {
    if (!surfaceEl || !frameMeta || !hasFrame) return null;
    const r = surfaceEl.getBoundingClientRect();
    return clientToPage({ x: clientX, y: clientY }, { left: r.left, top: r.top, width: r.width, height: r.height }, imageSize, frameMeta);
  }

  function mods(e: MouseEvent | KeyboardEvent | WheelEvent): number {
    return (e.altKey ? 1 : 0) | (e.ctrlKey ? 2 : 0) | (e.metaKey ? 4 : 0) | (e.shiftKey ? 8 : 0);
  }

  const inputLive = $derived(conn.status === 'live' && canDrive && !approval);

  function clickCount(e: PointerEvent): number {
    const t = performance.now();
    const near = Math.abs(e.clientX - lastDown.x) < 5 && Math.abs(e.clientY - lastDown.y) < 5;
    const count = near && e.button === lastDown.button && t - lastDown.t < 500 ? lastDown.count + 1 : 1;
    lastDown = { t, x: e.clientX, y: e.clientY, button: e.button, count };
    return count;
  }

  // Touch: tap = click; a one- or two-finger drag scrolls (wheel at the
  // gesture's centroid). Long presses and pinch are left alone for now.
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
    e.preventDefault(); // keep focus where we put it (the sink), no text selection
    captureKeyboard();
    const p = toPage(e.clientX, e.clientY);
    if (!p) return;
    if (pickMode && canPick) {
      send({ type: 'pick', x: p.x, y: p.y });
      return;
    }
    moves.flush();
    surfaceEl?.setPointerCapture(e.pointerId);
    send({ type: 'mouse', action: 'down', ...p, button: buttonName(e.button), buttons: e.buttons, click_count: clickCount(e), modifiers: mods(e), pointer: e.pointerType === 'pen' ? 'pen' : 'mouse' });
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
    if (pickMode && canPick) return;
    const p = toPage(e.clientX, e.clientY);
    if (!p) return;
    moves.push({ type: 'mouse', action: 'move', ...p, button: 'none', buttons: e.buttons, click_count: 0, modifiers: mods(e) });
  }

  function onPointerUp(e: PointerEvent): void {
    if (e.pointerType === 'touch') {
      touches.delete(e.pointerId);
      lastCentroid = touches.size ? centroid() : null;
      if (tap && tap.id === e.pointerId && !tap.moved && performance.now() - tap.t < 600 && inputLive) {
        const p = toPage(tap.x, tap.y);
        if (p) {
          if (pickMode && canPick) {
            send({ type: 'pick', x: p.x, y: p.y });
          } else {
            const base = { ...p, click_count: 1, modifiers: 0, pointer: 'touch' as const };
            send({ type: 'mouse', action: 'move', ...base, button: 'none', buttons: 0 });
            send({ type: 'mouse', action: 'down', ...base, button: 'left', buttons: 1 });
            send({ type: 'mouse', action: 'up', ...base, button: 'left', buttons: 0 });
          }
        }
      }
      if (!touches.size) {
        tap = null;
        wheels.flush();
      }
      return;
    }
    if (!inputLive || (pickMode && canPick)) return;
    const p = toPage(e.clientX, e.clientY);
    if (!p) return;
    moves.flush();
    send({ type: 'mouse', action: 'up', ...p, button: buttonName(e.button), buttons: e.buttons, click_count: lastDown.count || 1, modifiers: mods(e), pointer: e.pointerType === 'pen' ? 'pen' : 'mouse' });
  }

  function onPointerCancel(e: PointerEvent): void {
    touches.delete(e.pointerId);
    if (!touches.size) {
      tap = null;
      lastCentroid = null;
    }
  }

  // The pointer surface is a picture of the remote screen, not a control of
  // its own — the accessible control is the keyboard sink below it — so its
  // listeners are attached here rather than as element attributes. Wheel
  // must be non-passive anyway to stop the Otto page scrolling (Svelte
  // attaches `onwheel` passively).
  $effect(() => {
    const el = surfaceEl;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!inputLive) return;
      e.preventDefault();
      const p = toPage(e.clientX, e.clientY);
      if (!p) return;
      const h = frameMeta?.device_height ?? 800;
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
   *  focus starting point on the sink, so Tab moves on to the next control
   *  and Shift+Tab / a click take the page back. */
  function release(): void {
    sinkEl?.blur();
  }

  function releaseKeys(): void {
    for (const code of downKeys) {
      send({ type: 'key', action: 'up', key: code, code, modifiers: 0, key_code: 0, repeat: false });
    }
    downKeys.clear();
  }

  function onSinkKeyDown(e: KeyboardEvent): void {
    const route = routeKey(e, isMac);
    switch (route) {
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
        downKeys.add(e.code);
        send({ type: 'key', action: 'down', ...keyPayload(e, isMac, 'down') });
    }
  }

  function onSinkKeyUp(e: KeyboardEvent): void {
    if (!downKeys.has(e.code)) return;
    e.preventDefault();
    downKeys.delete(e.code);
    send({ type: 'key', action: 'up', ...keyPayload(e, isMac, 'up') });
  }

  function sendText(text: string): void {
    if (text && inputLive) send({ type: 'text', text });
  }

  function onSinkInput(e: Event): void {
    const ie = e as InputEvent;
    if (ie.isComposing || !sinkEl) return;
    const v = sinkEl.value;
    sinkEl.value = '';
    sendText(v);
  }

  function onCompositionEnd(e: CompositionEvent): void {
    sendText(e.data);
    if (sinkEl) sinkEl.value = '';
  }

  function onPaste(e: ClipboardEvent): void {
    e.preventDefault();
    sendText(e.clipboardData?.getData('text/plain') ?? '');
  }

  // ── navigation / control (called by the parent's toolbar) ───────────────

  export function back(): void {
    send({ type: 'history', action: 'back' });
  }
  export function forward(): void {
    send({ type: 'history', action: 'forward' });
  }
  export function reload(): void {
    send({ type: 'history', action: 'reload' });
  }
  export function focusPage(): void {
    captureKeyboard();
  }

  function takeOver(): void {
    send({ type: 'take_over' });
  }
  function handBack(): void {
    releaseKeys();
    send({ type: 'hand_back' });
  }

  async function decide(decision: ApprovalDecision): Promise<void> {
    const a = approval;
    if (!a || deciding) return;
    if (decision === 'deny') {
      const why = await confirmer.promptText(`Tell ${a.agent_name || 'the agent'} why (optional). It won't do this action.`, {
        title: 'Deny this action',
        confirmLabel: 'Deny',
        placeholder: 'Wrong time slot',
      });
      if (why === null) return;
      deciding = true;
      send({ type: 'approval', id: a.id, decision, ...(why.trim() ? { reason: why.trim() } : {}) });
      return;
    }
    deciding = true;
    send({ type: 'approval', id: a.id, decision });
  }

  // Overlay positions (page coords → pane coords).
  const ghostPos = $derived(ghost && frameMeta && hasFrame ? pageToBox(ghost, box, imageSize, frameMeta) : null);
  const targetBox = $derived.by(() => {
    const t = approval?.target;
    if (!t || !frameMeta || !hasFrame) return null;
    const a = pageToBox({ x: t.x, y: t.y }, box, imageSize, frameMeta);
    const b = pageToBox({ x: t.x + t.width, y: t.y + t.height }, box, imageSize, frameMeta);
    return a && b ? { x: a.x, y: a.y, w: b.x - a.x, h: b.y - a.y } : null;
  });

  const statusText = $derived.by(() => {
    switch (conn.status) {
      case 'idle':
      case 'connecting':
        return hasFrame ? 'Reconnecting…' : 'Starting the live browser…';
      case 'live':
        return 'Live';
      case 'reconnecting':
        return `Connection lost — reconnecting (attempt ${conn.attempt})…`;
      case 'ended':
        return 'Live session ended';
    }
  });

  const isTouch = typeof matchMedia !== 'undefined' && matchMedia('(pointer: coarse)').matches;
</script>

<div class="rlv" data-testid="remote-live" data-status={conn.status}>
  {#if lock.holder === 'agent' && !lock.mine}
    <div class="drive-bar" role="status" data-testid="live-drive-bar">
      {#if lock.provider}<ProviderIcon provider={lock.provider} size={14} />{:else}<Icon name="cursor" size={14} />{/if}
      <span class="who"><strong>{lock.agent_name || 'An agent'}</strong> is driving this page</span>
      {#if lock.activity}<span class="activity" title={lock.activity}>{lock.activity}</span>{/if}
      <span class="grow"></span>
      <button class="btn small primary" onclick={takeOver}>Take over</button>
    </div>
  {:else if lock.mine && lock.session_id}
    <div class="drive-bar mine" role="status" data-testid="live-drive-bar">
      <Icon name="user" size={14} />
      <span class="who">You have control. <strong>{lock.agent_name || 'The agent'}</strong> is paused.</span>
      <span class="grow"></span>
      <button class="btn small" onclick={handBack}>Hand back</button>
    </div>
  {/if}

  <div
    class="surface"
    class:stale
    class:kbd={kbdFocused}
    class:blocked={!canDrive}
    class:picking={pickMode && canPick}
    style:cursor={pickMode && canPick ? 'crosshair' : canDrive ? cursor : 'default'}
    bind:this={surfaceEl}
  >
    <canvas bind:this={canvasEl} aria-hidden="true"></canvas>

    {#if targetBox}
      <div class="target" style:inset-inline-start="{targetBox.x}px" style:top="{targetBox.y}px" style:width="{targetBox.w}px" style:height="{targetBox.h}px" aria-hidden="true"></div>
    {/if}
    {#if ghostPos && lock.holder === 'agent'}
      <div class="ghost" style:inset-inline-start="{ghostPos.x}px" style:top="{ghostPos.y}px" aria-hidden="true">
        <Icon name="cursor" size={16} />
        <span class="ghost-label">{ghost?.label}</span>
      </div>
    {/if}

    <textarea
      class="sink"
      bind:this={sinkEl}
      aria-label={`Live page${nav?.title ? `: ${nav.title}` : ''}. Typing goes to the page; Escape gives the keyboard back to Otto.`}
      aria-roledescription="remote browser"
      autocapitalize="off"
      autocomplete="off"
      spellcheck="false"
      disabled={!inputLive}
      onkeydown={onSinkKeyDown}
      onkeyup={onSinkKeyUp}
      oninput={onSinkInput}
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
    <span>{conn.status === 'live' ? 'Live' : statusText}</span>
    {#if conn.status === 'live' && engineLabel}<span class="dim">· {engineLabel}</span>{/if}
  </div>

  {#if conn.status === 'live' && hasFrame}
    <div class="meter" aria-hidden="true" title="Frames per second · round-trip latency">
      {fps} fps{#if meter.rttMs !== null} · {meter.rttMs} ms{/if}
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

  {#if !hasFrame && (conn.status === 'connecting' || conn.status === 'idle')}
    <div class="center" role="status" aria-live="polite">
      <Icon name="globe" size={16} />
      <span>Starting the live browser…</span>
    </div>
  {/if}

  {#if conn.status === 'reconnecting' || (conn.status === 'connecting' && hasFrame)}
    <div class="banner" role="status" aria-live="polite" data-testid="live-reconnecting">
      <Icon name="warning" size={14} />
      <span>{statusText}</span>
      {#if errorNote}<span class="dim" title={errorNote}>{errorNote}</span>{/if}
      <button class="btn small" onclick={retryNow}>Retry now</button>
    </div>
  {:else if conn.status === 'live' && stale}
    <div class="banner" role="status">
      <Icon name="clock" size={14} />
      <span>No response from the live browser for a few seconds. The picture may be out of date.</span>
    </div>
  {/if}

  {#if conn.status === 'ended'}
    <div class="center ended" role="status" data-testid="live-ended">
      <h2>The live session ended</h2>
      <p>{conn.reason || 'The daemon closed this live tab.'}</p>
      <div class="row">
        <button class="btn primary" onclick={retryNow}><Icon name="refresh" size={13} /> Reconnect</button>
        {#if onreader}<button class="btn ghost" onclick={onreader}>Switch to Reader</button>{/if}
      </div>
    </div>
  {/if}

  {#if approval}
    <div class="approval-host">
      <ApprovalCard {approval} busy={deciding} ondecide={(d) => void decide(d)} />
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
  .drive-bar .who strong {
    font-weight: 600;
  }
  .drive-bar .activity {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-dim);
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
  /* Focus replacement: an inset ring (the frame fills the element, so an
     outset ring would be clipped by the pane). */
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
  .surface.blocked canvas {
    filter: saturate(0.85);
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
  .target {
    position: absolute;
    border: 2px solid var(--warning);
    border-radius: var(--radius-s);
    box-shadow: 0 0 0 3px var(--warning-soft);
    pointer-events: none;
  }
  .ghost {
    position: absolute;
    display: flex;
    align-items: flex-start;
    gap: 2px;
    color: var(--text);
    pointer-events: none;
    transition: inset-inline-start 120ms ease-out, top 120ms ease-out;
  }
  .ghost-label {
    margin-top: 12px;
    padding: 1px 6px;
    border-radius: 999px;
    border: 1px solid var(--border-strong);
    background: var(--surface);
    font-size: var(--fs-xs);
    white-space: nowrap;
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
  .drive-bar ~ .badge {
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
    background: color-mix(in srgb, var(--surface) 85%, transparent);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    pointer-events: none;
  }
  .hint {
    position: absolute;
    inset-inline-start: 50%;
    bottom: 10px;
    transform: translateX(-50%);
    padding: 2px 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    white-space: nowrap;
    pointer-events: none;
  }
  :global([dir='rtl']) .hint {
    transform: translateX(50%);
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
  .drive-bar ~ .kbd-btn {
    top: 46px;
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
    background: color-mix(in srgb, var(--bg) 80%, transparent);
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
    padding: 6px 10px;
    border: 1px solid color-mix(in srgb, var(--warning) 35%, transparent);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .drive-bar ~ .banner {
    top: 78px;
  }
  .banner :global(svg) {
    color: var(--warning);
    flex: none;
  }
  .banner .dim {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .banner .btn {
    margin-inline-start: auto;
  }
  .approval-host {
    position: absolute;
    inset-inline-start: 16px;
    top: 16px;
    bottom: 16px;
    max-width: calc(100% - 32px);
    display: flex;
    align-items: flex-start;
    pointer-events: none;
  }
  .drive-bar ~ .approval-host {
    top: 52px;
  }
  .approval-host > :global(*) {
    pointer-events: auto;
  }
  @media (max-width: 640px) {
    .approval-host {
      inset-inline: 8px;
      top: auto;
      bottom: 8px;
      max-height: calc(100% - 16px);
      max-width: none;
      align-items: flex-end;
    }
    .drive-bar ~ .approval-host {
      top: auto;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    canvas,
    .ghost {
      transition: none;
    }
  }
</style>
