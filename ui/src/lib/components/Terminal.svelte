<script module lang="ts">
  // ── GPU renderer budget (shared by every Terminal in this window) ─────────
  // WebKit keeps at most 16 live WebGL contexts per page and silently LOSES
  // the oldest past that. A tiled grid (15 live tiles) plus the primary pane
  // and a few embeds would cycle contexts forever, so at most this many
  // terminals render on the GPU; the rest use xterm's DOM renderer.
  export const MAX_WEBGL_TERMINALS = 10;
  let liveWebgl = 0;
  /** Escape hatch: `localStorage['otto.term.renderer'] = 'dom'` forces the
   *  DOM renderer everywhere (read once per load). */
  const FORCE_DOM_RENDERER = (() => {
    try {
      return globalThis.localStorage?.getItem('otto.term.renderer') === 'dom';
    } catch {
      return false;
    }
  })();

  // ── GPU slot arbitration (review 01 L4) ───────────────────────────────────
  // A full budget used to deny the GPU to whichever terminal asked last — the
  // focused pane included — and a lost context fell back to DOM for good.
  // Every mounted Terminal registers here: a freed slot goes to the most
  // recently focused terminal still waiting for one, and focusing a DOM-
  // rendered pane while the budget is full takes the slot of the least
  // recently focused GPU terminal.
  interface GpuClient {
    lastFocus: number;
    hasGpu(): boolean;
    /** Wants the GPU, has none, and is not backing off a context loss. */
    waiting(): boolean;
    yieldGpu(): void;
    tryGpu(): void;
  }
  const gpuClients = new Set<GpuClient>();
  let gpuGrantQueued = false;
  function releaseGpuSlot(): void {
    liveWebgl = Math.max(0, liveWebgl - 1);
    if (gpuGrantQueued) return;
    gpuGrantQueued = true;
    // Deferred: the releasing terminal finishes its own bookkeeping (a
    // context loss arms its backoff) before anyone is offered the slot.
    queueMicrotask(() => {
      gpuGrantQueued = false;
      grantFreeGpuSlots();
    });
  }
  function grantFreeGpuSlots(): void {
    while (liveWebgl < MAX_WEBGL_TERMINALS) {
      let best: GpuClient | null = null;
      for (const c of gpuClients) if (c.waiting() && (!best || c.lastFocus > best.lastFocus)) best = c;
      if (!best) return;
      best.tryGpu();
      if (!best.hasGpu()) return; // creation failed (no WebGL2): stop here
    }
  }
  function stealGpuSlotFor(me: GpuClient): void {
    if (liveWebgl < MAX_WEBGL_TERMINALS) return;
    let victim: GpuClient | null = null;
    for (const c of gpuClients) if (c !== me && c.hasGpu() && (!victim || c.lastFocus < victim.lastFocus)) victim = c;
    victim?.yieldGpu();
  }

  // ── Parked terminals (r3-09-05, see termPark.ts for the bounds) ───────────
  import type { Terminal as XTerm } from '@xterm/xterm';
  import type { FitAddon as XFit } from '@xterm/addon-fit';
  import type { SearchAddon as XSearch } from '@xterm/addon-search';
  import type { SessionStatus as ParkedStatus } from '../api/types';
  import { base64ToBytes as parkedB64 } from '../b64';
  import { snapshotApplies, withInOrderReset as parkedRis, type TermFlow as FlowT, type WriteQueue as QueueT } from './termFlow';
  import { PARK_CELL_BYTES, PARK_SCROLLBACK, TermPark } from './termPark';
  import { CompactQueue } from './termCompactQueue';

  /** Resize compacts for every Terminal in this window: one in flight, the
   *  most recently focused pane first (perf F1, termCompactQueue.ts). */
  const compactQueue = new CompactQueue();

  /** Everything a live Terminal hands over when it parks: the emulator, its
   *  socket and the socket's flow/snapshot state, so the adopter continues
   *  the SAME stream (same credit tag, same epoch) with no snapshot. */
  interface ParkedEngine {
    term: XTerm;
    fit: XFit;
    search: XSearch;
    sock: WebSocket;
    flow: FlowT;
    writes: QueueT;
    compactPending: boolean;
    resyncPending: boolean;
    snapshotEpoch: number | null;
    /** A `status` pushed while parked (revival): replayed to the adopter. */
    status: ParkedStatus | null;
    exitCode: number | null;
    lastCols: number;
    lastRows: number;
    /** The xterm reflowed locally (or was put back to the PTY grid on
     *  parking) without the PTY repainting: the adopter asks for a snapshot. */
    needsCompact: boolean;
  }

  function disposeParked(e: ParkedEngine): void {
    const s = e.sock;
    s.onopen = null;
    s.onmessage = null;
    s.onerror = null;
    s.onclose = null;
    try {
      s.close();
    } catch {
      /* already closing */
    }
    e.writes.dropQueued();
    try {
      e.term.dispose();
    } catch {
      /* already disposed */
    }
  }

  /** Estimated heap of a parked xterm (~12 B/cell), for the lot's byte
   *  budget (perf 01 N2): a full lot of wide 4000-row panes was ~115 MB. */
  function parkedBytes(e: ParkedEngine): number {
    try {
      return e.term.cols * e.term.buffer.active.length * PARK_CELL_BYTES;
    } catch {
      return 0;
    }
  }

  /** Binary snapshot headers (perf 01 N3) waiting for their payload — the
   *  NEXT binary frame on that socket. Keyed by socket, not by component: a
   *  park/adopt can swap the handler between the header and its payload. */
  const binarySnapHeaders = new WeakMap<WebSocket, { epoch?: number }>();

  const termPark = new TermPark<ParkedEngine>(disposeParked, undefined, undefined, undefined, undefined, parkedBytes);

  /** While parked the engine keeps up with its session on its own: bytes
   *  parse (the renderer is paused while detached), credit acks flow, snapshots
   *  rebuild by the same rule as a live Terminal, and a close evicts it. */
  function wireParked(key: string, e: ParkedEngine): void {
    e.flow.setSink((frame) => {
      if (e.sock.readyState === WebSocket.OPEN) e.sock.send(JSON.stringify(frame));
    });
    e.writes.rebind(
      (bytes, done) => e.term.write(bytes, done),
      () => e.sock.readyState === WebSocket.OPEN,
    );
    e.sock.onopen = null;
    e.sock.onerror = null;
    e.sock.onclose = () => termPark.evict(key, e);
    const applyParkedSnapshot = (epoch: number | null, snap: Uint8Array | null): void => {
      const buffer = e.term.buffer.active;
      const holds = () => e.term.hasSelection() || buffer.baseY - buffer.viewportY > 3;
      if (!snapshotApplies(e, epoch, holds) || !snap?.length) return;
      e.writes.dropQueued();
      e.resyncPending = false;
      if (e.writes.inflight === 0) {
        e.term.reset();
        e.writes.push(snap);
      } else {
        e.writes.push(parkedRis(snap));
      }
    };
    e.sock.onmessage = (ev: MessageEvent) => {
      if (ev.data instanceof ArrayBuffer) {
        // A binary snapshot's payload (N3): not live output, never credited.
        const hdr = binarySnapHeaders.get(e.sock);
        if (hdr) {
          binarySnapHeaders.delete(e.sock);
          applyParkedSnapshot(hdr.epoch ?? null, new Uint8Array(ev.data));
          return;
        }
        e.writes.push(new Uint8Array(ev.data), undefined, e.flow.credit);
        return;
      }
      if (typeof ev.data !== 'string') return;
      let msg: { type?: string; window?: unknown; epoch?: number; data?: string; binary?: boolean; status?: ParkedStatus; code?: number; message?: string };
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }
      switch (msg.type) {
        case 'credit':
          e.flow.granted(typeof msg.window === 'number' ? msg.window : undefined);
          break;
        case 'scrollback': {
          if (msg.binary) {
            binarySnapHeaders.set(e.sock, { epoch: msg.epoch });
            break;
          }
          applyParkedSnapshot(msg.epoch ?? null, msg.data ? parkedB64(msg.data) : null);
          break;
        }
        case 'status':
          if (!msg.status) break;
          e.status = msg.status;
          if (msg.status !== 'exited' && msg.status !== 'reconnectable') e.exitCode = null;
          break;
        case 'exit':
          e.exitCode = msg.code ?? 0;
          break;
        case 'error':
          e.term.write(`\r\n\x1b[31m[otto] ${msg.code}: ${msg.message ?? ''}\x1b[0m\r\n`);
          break;
      }
    };
  }
</script>

<script lang="ts">
  import { toastError } from '../toastError';
  // xterm.js terminal bound to WS /ws/term/{id} per docs/contracts/ws.md.
  // Binary frames → term.write; JSON control frames for status/exit/scrollback.
  import { untrack } from 'svelte';
  import { exitState } from '../status';
  import { Terminal } from '@xterm/xterm';
  // ILinkProvider isn't exported from the ambient module, so derive its shape
  // from registerLinkProvider's parameter (kept in lockstep with the version).
  type LinkProvider = Parameters<Terminal['registerLinkProvider']>[0];
  import { FitAddon } from '@xterm/addon-fit';
  import { SearchAddon } from '@xterm/addon-search';
  import { WebglAddon } from '@xterm/addon-webgl';
  import '@xterm/xterm/css/xterm.css';
  import { wsConnect, wsUrl, WS_BEARER_SUBPROTOCOL } from '../api/client';
  import type { SessionStatus, TermSearchMatch, WsSearchResultFrame, WsTermFlowFrame, WsTermProbeAckFrame, WsTermProbeFrame, WsTermResyncFrame, WsTermScrollbackRequestFrame } from '../api/types';
  import type { CompactClient } from './termCompactQueue';
  import { EMBED_SCROLLBACK, QuietRepaint, TermFlow, WriteQueue, hasCursorOrErase, resizeDecision, withInOrderReset } from './termFlow';
  import { KeyLatency, ProbeClock, fmtMs, fmtPair, loopMonitor, termLatencyEnabled, type EchoStats } from './termLatency';
  import { textToBase64, base64ToBytes, bytesToBase64 } from '../b64';
  import { terminalTheme } from '../termtheme';
  import { ui } from '../stores/ui.svelte';
  import { viewport } from '../stores/viewport.svelte';
  import { ws } from '../stores/workspace.svelte';
  import { openFile } from '../stores/openfile.svelte';
  import { openExternal } from '../external';
  import { terminalLinksForRow, resolveTerminalFile, oscTerminalLink, type TerminalLink } from './terminalLinks';
  import { keyContext, registerFindOwner } from '../keys';
  import { copyText } from '../clipboard';
  import { snipApi } from '../snip';
  import { toasts } from '../toast.svelte';
  import { registerSelectAll } from '../selectall';
  import TermKeysBar from './TermKeysBar.svelte';
  import Icon from './Icon.svelte';
  import { terminalReply } from './terminalInput';
  import { plural } from '../plural';

  interface Props {
    sessionId: string;
    readOnly?: boolean;
    /** When true a "Resume" button is shown on the exited overlay; clicking it
     *  reconnects the WS — the daemon's ensure_live will resume the session. */
    resumable?: boolean;
    /** When true the exited overlay shows a "Reconnect" button even when the
     *  session isn't WS-resumable (e.g. a plain shell, which is respawned via
     *  the parent's `onrestart` callback rather than a bare WS re-attach). */
    restartable?: boolean;
    /** Invoked by the overlay "Reconnect" button (and reused by the header
     *  restart). When set, takes precedence over the bare WS re-attach so an
     *  exited shell is actually respawned server-side. */
    onrestart?: () => void;
    /** Bumped by the parent after a successful restart so the Terminal drops the
     *  exited overlay and reconnects to the now-live PTY. */
    restartNonce?: number;
    /** When true, force the xterm palette and host background to dark regardless
     *  of the app's current light/dark scheme. Use for embedded agent CLIs
     *  (claude, codex) that render their own dark TUI canvas. */
    forceDark?: boolean;
    /** The pane hosts a full-screen agent TUI (claude/codex/grok…) that
     *  rewrites its status bar and prompt in place. (Historical name: it
     *  used to force xterm's DOM renderer, which cost ~⅓ of a core per
     *  working pane in WebKit — r3-12-01.) Now it selects TUI handling on
     *  whichever renderer is active (WebGL when available): one trailing
     *  full-viewport clean-up repaint per output burst (≤ 1×/s while a
     *  spinner never stops) mops up cells a TUI rewrote without dirtying,
     *  and a confirmed resize compacts from a server snapshot. Default
     *  false (plain shells: redraw only small cursor/erase frames). */
    preferDom?: boolean;
    /** When provided, the WS is opened with `Authorization` via the
     *  `otto-bearer` Sec-WebSocket-Protocol subprotocol carrying this token
     *  instead of the stored owner login token. Used by the guest share view
     *  (SharePage) so the scoped share token never touches localStorage.
     *  Default = undefined → falls back to today's wsUrl() behaviour. */
    shareToken?: string;
    /** Capability-only room transport; bypasses owner authentication entirely. */
    socketFactory?: () => WebSocket;
    transformFrame?: (frame: unknown) => unknown | null;
    readOnlyReason?: string;
    onstatus?: (status: SessionStatus) => void;
    /** The font size actually drawn (px) — below the user's size while the
     *  pane is too narrow for 80 columns (with an 11px readability floor). */
    onfontfit?: (px: number) => void;
    /** Called when the server returns a ring-buffer search result frame.
     *  The parent can surface results in a search-result panel. */
    onsearchresult?: (frame: WsSearchResultFrame) => void;
    /** When false, the floating zoom/copy toolbar overlay is suppressed —
     *  used by hosts that surface the same controls in their own header bar
     *  (SessionView panes) so the overlay never covers terminal content. */
    showToolbar?: boolean;
    /** Grab keyboard focus as soon as the terminal is created, so a freshly
     *  opened session is typeable without a click. Read once at init (mount-time
     *  value); later pane-focus changes go through the exported focus(). Skipped
     *  on phones — the soft keyboard may only be raised by a user gesture. */
    autoFocus?: boolean;
    /** Claim PTY size authority as soon as the socket opens — set by the
     *  PRIMARY session pane (SessionView) only. Until someone types, a session
     *  has no size owner and the LAST viewer to attach/refit sets the PTY —
     *  a later tile/preview/phone tab silently pinned the pane's TUI to its
     *  own smaller grid (content rendering at ~half the pane until a click).
     *  Claiming on attach makes the real pane own the grid from the start;
     *  typing elsewhere can still take authority over (server policy).
     *  Preview/monitor embeds (review agents, share viewers, …) must NOT set
     *  this. Default false. */
    claimOnAttach?: boolean;
    /** Local xterm scrollback depth (lines). Each line costs ~12 B/cell, so
     *  4000 lines × 200 cols ≈ 9.6 MB of JS heap per terminal — fine for the
     *  one primary pane, too much across a 15-tile grid (SA-05). Default
     *  EMBED_SCROLLBACK (2k): tiles and embedded previews mount many at once.
     *  PRIMARY hosts (SessionView, the share page, the DB SSH shell) pass
     *  PRIMARY_SCROLLBACK (4000 = the daemon's own depth, perf 01 N2), so
     *  maximizing/reconnecting restores everything the daemon has. Also the
     *  `lines` requested in every `scrollback` snapshot. */
    scrollback?: number;
    /** Park instead of dispose on unmount / session switch (termPark.ts):
     *  the xterm and its socket stay live off-screen and the next Terminal
     *  mounted for the same session adopts them — no 4000-row snapshot
     *  replay when coming back to Agents or switching tabs. Owner sockets
     *  only (ignored with shareToken/socketFactory). Default false. */
    keepAlive?: boolean;
    /** Opening this terminal resumes a suspended agent session (the daemon's
     *  `ensure_live` on attach). False = a VIEW-ONLY attach (`?view=1`, see
     *  docs/contracts/ws.md §1): looking at the pane never spawns the CLI —
     *  the first real keystroke or the Resume button does (r3-05-01). Set it
     *  on panes that show a session rather than work in it (grid tiles,
     *  embedded agent-output viewers). Automatic reconnects (dropped socket,
     *  daemon restart, window refocus) are view-only on EVERY terminal: a
     *  dropped socket is not the user asking for the CLI back. Default true. */
    resumeOnOpen?: boolean;
    /** Take ⌘F without holding focus (keys.ts routeFind): the active session
     *  pane passes > 0 so ⌘F opens THIS find bar after a click on its header
     *  or anywhere off the xterm; the highest rank among visible terminals
     *  wins (the focused split pane 2 over another view of the session 1).
     *  Only while the last click / focus was inside its `.pane` (or nothing
     *  was clicked yet) — a loop timeline or the right panel keeps the page
     *  find. A focused terminal / editor / text field still wins. Default 0. */
    findRank?: number;
  }
  let { sessionId, readOnly = false, resumable = false, restartable = false, onrestart, restartNonce = 0, forceDark = false, preferDom = false, shareToken, socketFactory, transformFrame, readOnlyReason, onstatus, onfontfit, onsearchresult, showToolbar = true, autoFocus = false, claimOnAttach = false, keepAlive = false, scrollback = EMBED_SCROLLBACK, resumeOnOpen = true, findRank = 0 }: Props = $props();
  // Scrollback results: a listbox driven from the find input (aria-activedescendant).
  const findId = $props.id();

  const effScheme = $derived(forceDark ? 'dark' : ui.resolvedScheme);

  // Effective terminal font size. On phone we apply a comfortable readability
  // floor (PHONE_MIN_FONT): a 13px monospace grid is legible on a desktop
  // monitor but cramped on a high-DPI handset held at arm's length, which is a
  // big part of why the mobile terminal felt unusable. Apply the user's zoom
  // offset above that floor so the first Zoom in changes the rendered size. Desktop
  // keeps the shared 11px readable-content floor, including dense split panes.
  const PHONE_MIN_FONT = 15;
  const effFontSize = $derived(
    viewport.isPhone ? Math.max(PHONE_MIN_FONT, PHONE_MIN_FONT + ui.termFontSize - 13) : Math.max(ui.termFontSize, 11),
  );

  let container: HTMLDivElement;
  let horizontalOverflow = $state(false);
  let fittedFontSize = $state(13);
  function scrollFocus(node: HTMLElement, overflowing: boolean) {
    const update = (value: boolean) => {
      if (value) node.tabIndex = 0;
      else node.removeAttribute('tabindex');
    };
    // Make the focused region's scrolling explicit across webviews. Handle
    // only this host; xterm input keeps its keys.
    const onKey = (event: KeyboardEvent) => {
      if (event.target !== node || node.scrollWidth <= node.clientWidth) return;
      if (event.key === 'ArrowLeft') node.scrollLeft -= 40;
      else if (event.key === 'ArrowRight') node.scrollLeft += 40;
      else if (event.key === 'Home') node.scrollLeft = 0;
      else if (event.key === 'End') node.scrollLeft = node.scrollWidth;
      else return;
      event.preventDefault();
      event.stopPropagation();
    };
    node.addEventListener('keydown', onKey);
    update(overflowing);
    return { update, destroy() { node.removeEventListener('keydown', onKey); } };
  }
  let term: Terminal | null = null;
  let fit: FitAddon | null = null;
  let search: SearchAddon | null = null;
  /** Live WebGL addon when the GPU renderer is active — kept so we can clear
   *  the glyph atlas on font/theme changes (stale atlas tiles leave "ghost"
   *  cells after TUI redraws). Null when the DOM renderer is in use. */
  let webglAddon: WebglAddon | null = null;
  /** This terminal should render on the GPU (desktop, no RTL, not forced to
   *  DOM) — whether it currently does depends on the budget / context. */
  let webglWanted = false;
  let webglRetryTimer: ReturnType<typeof setTimeout> | null = null;
  let webglRetries = 0;
  /** Context-loss recoveries per terminal: 5 s doubling to a 60 s cap (a GPU
   *  waking from sleep can take longer than the old two 5 s tries). Window
   *  focus, the page becoming visible, scrolling back into view and focusing
   *  the pane all start a fresh round (retryWebglNow). */
  const WEBGL_MAX_RETRIES = 8;
  const WEBGL_RETRY_MS = 5000;
  const WEBGL_RETRY_MAX_MS = 60_000;
  const gpuClient: GpuClient = {
    lastFocus: 0,
    hasGpu: () => webglAddon !== null,
    waiting: () => !!term && webglWanted && !webglAddon && webglRetryTimer === null,
    yieldGpu: () => {
      dropWebgl();
      forceViewportRefresh();
    },
    tryGpu: () => {
      attachWebgl();
      if (webglAddon) forceViewportRefresh();
    },
  };
  let sock: WebSocket | null = null;

  // ── Latency HUD (review 01 L1, termLatency.ts) ─────────────────────────────
  // Opt-in per mount: `localStorage['otto.debug.termLatency'] = '1'`, then
  // reopen the pane. Off, the hooks below are a null check each.
  const latencyOn = termLatencyEnabled();
  const keyLat: KeyLatency | null = latencyOn ? new KeyLatency() : null;
  const probes: ProbeClock | null = latencyOn ? new ProbeClock() : null;
  let echoStats: EchoStats | null = null;
  interface HudRow {
    k: string;
    v: string;
    title: string;
  }
  let hud = $state.raw<HudRow[] | null>(null);
  function refreshHud(): void {
    if (!keyLat || !probes) return;
    const lost = probes.outstanding > 1 ? ` · ${probes.outstanding} waiting` : '';
    const gpu = webglAddon ? 'webgl' : webglWanted ? (webglRetryTimer !== null ? 'dom · gpu retry' : 'dom · no gpu slot') : 'dom';
    hud = [
      { k: 'rtt', v: fmtPair(probes.rtt) + lost, title: 'probe → probe_ack: websocket + daemon loop, no child (p50/p95 ms)' },
      {
        k: 'echo',
        v: echoStats && echoStats.samples > 0 ? `${fmtMs(echoStats.avg_ms)} avg · ${fmtMs(echoStats.max_ms)} max` : '–',
        title: 'Daemon side: input reached the PTY → the child\'s first output (ms)',
      },
      { k: 'wire', v: fmtPair(keyLat.wire), title: 'Keystroke sent → first output frame back (p50/p95 ms)' },
      { k: 'parse', v: fmtPair(keyLat.parse), title: 'That frame handed to xterm → parsed (p50/p95 ms)' },
      { k: 'paint', v: fmtPair(keyLat.paint), title: 'Parsed → next render pass (p50/p95 ms)' },
      { k: 'drift', v: fmtPair(loopMonitor.drift), title: 'Lateness of a 1 s timer: timer throttling or a busy main thread (p50/p95 ms)' },
      { k: 'rAF', v: fmtPair(loopMonitor.raf), title: 'Animation-frame interval (p50/p95 ms); ~16.7 is healthy' },
      {
        k: 'page',
        v: `${document.visibilityState}${document.hasFocus() ? ' · focused' : ''}`,
        title: 'document.visibilityState and window focus',
      },
      { k: 'render', v: `${gpu} (${liveWebgl}/${MAX_WEBGL_TERMINALS})`, title: 'Renderer, and GPU terminals in this window / budget' },
      {
        k: 'queue',
        v: `${Math.round(writes.backlog / 1024)} KB${loopMonitor.longTaskSupported ? ` · ${loopMonitor.longTasks} long tasks` : ''}`,
        title: 'Bytes received but not yet parsed by xterm',
      },
    ];
  }

  /** Coalesce interactive-redraw refreshes (up-arrow history, multi-line
   *  composer resize, etc.) into one rAF so rapid TUI frames don't thrash. */
  let tuiRefreshRaf: number | null = null;

  /** True while an IME composition is open on xterm's textarea — the selection
   *  mirror must leave the textarea alone for its duration. */
  let composing = false;
  /** Set by the `copy` listener below. The ⌘C path leaves the browser's native
   *  copy command to run (it needs no clipboard permission); this is how it
   *  tells whether that actually happened, so the permissioned API is only
   *  used when the native one genuinely did nothing. */
  let copySawEvent = false;

  let connected = $state(false);
  let exitCode: number | null = $state(null);
  /** The current socket attached view-only (`?view=1`): the daemon did not
   *  resume the session for it. */
  let viewAttach = false;
  /** A view-only attach found the session suspended — the exited overlay then
   *  says how to wake it (typing or Resume) instead of "resumes on open". */
  let dormantView = $state(false);
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
  // The session id we currently have (or are opening) a socket for. Effect 2
  // (session-switch) reconnects ONLY when this actually changes. Without it, a
  // churning parent — e.g. the review panel re-rendering on every poll/bus tick
  // and passing a NEW agent object with the SAME session_id string — re-runs
  // Effect 2 and tears down + reconnects the WS on every render, which became a
  // ~20–90 reconnect-per-second storm that left the terminal stuck "reconnecting".
  let connectedSid: string | null = null;

  /** The socket dropped right AFTER an `exit` frame. That is the daemon going
   *  away, not the session ending on its own (a live socket outlives its
   *  session's exit): a daemon shutdown kills — or, with session persistence,
   *  detaches — its PTYs, and the exit we saw may be that shutdown. Without a
   *  re-attach the pane stayed on that stale "Ended" forever, even after the
   *  restarted daemon re-adopted the process or marked the session resumable
   *  ("my shell never reconnects after a restart"). So the pane re-attaches
   *  view-only once more and lets the `status` frame decide: live clears the
   *  overlay, suspended shows "type or Resume", exited keeps it. */
  let probingAfterExit = false;
  /** Bounded: a socket refused for good (a revoked share on an ended session)
   *  must not retry forever. ~2.5 min at the 5 s backoff cap — longer than a
   *  daemon restart takes. Reset by any `status` frame. */
  let exitProbes = 0;
  const MAX_EXIT_PROBES = 30;

  function scheduleReconnect(afterExit = false): void {
    if (closedByUs || reconnectTimer) return;
    if (exitCode !== null && !afterExit) return;
    reconnecting = true;
    const delay = Math.min(500 * 2 ** reconnectAttempts, 5000);
    reconnectAttempts++;
    reconnectTimer = setTimeout(() => {
      reconnectTimer = null;
      connect({ view: true });
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
  /** Local (SearchAddon) match position / total, from onDidChangeResults.
   *  `localIdx` is −1 past the addon's 1000-highlight cap. */
  let localIdx = $state(-1);
  let localCount = $state(0);

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
    if (serverMatches.length === 0 || !term) return;
    const clamped = ((idx % serverMatches.length) + serverMatches.length) % serverMatches.length;
    serverMatchIdx = clamped;
    const m = serverMatches[clamped];
    // Coarse jump first: the ring buffer's `line` is a LOGICAL (unwrapped)
    // index while xterm's scrollToLine addresses visual/wrapped rows, and after
    // a reconnect the client holds only the replayed tail of the history — so
    // the number alone can land on the wrong row.
    const buf = term.buffer.active;
    const row = Math.max(0, Math.min(m.line, buf.length - 1));
    term.scrollToLine(row);
    // Precise re-anchor: locate the match's actual text via the SearchAddon
    // FROM the coarse row — findNext starts at the selection (no selection =
    // the top of the buffer, which always landed on the FIRST copy of a
    // repeated line), so select the row first; it wraps if needed. This
    // scrolls to and selects the real occurrence regardless of wrapping/
    // replay offsets. When the match predates the client's replayed history
    // the text isn't in the buffer at all; the coarse jump is the best we can
    // show.
    const needle = m.text.trim();
    if (needle && search) {
      term.select(0, row, 1);
      search.findNext(needle);
    }
    // Keep the stepped-to row visible in the results list.
    queueMicrotask(() =>
      findResultsEl?.querySelector<HTMLElement>(`[data-match="${clamped}"]`)?.scrollIntoView({ block: 'nearest' }),
    );
  }
  let findResultsEl: HTMLDivElement | null = $state(null);

  function sendJson(obj: unknown): void {
    const frame = transformFrame ? transformFrame(obj) : obj;
    if (frame !== null && sock && sock.readyState === WebSocket.OPEN) sock.send(JSON.stringify(frame));
  }

  // ── Flow control (SA-02, docs/contracts/ws.md §1 "Flow control") ─────────
  // A browser WebSocket drains eagerly into the JS task queue, so the daemon
  // never feels backpressure from a slow RENDERER: xterm parses 5.5–8 MB/s in
  // WebKit while `cat`/`yes` produce far more. Unbounded, a flood kept the
  // whole app saturated for seconds and the screen kept scrolling ~9 s after
  // ^C. xterm's documented watermark pattern: count bytes handed to
  // term.write() until its callback fires; above HIGH ask the server to
  // `pause` this stream, below LOW `resume` it. The server then replaces
  // whatever it held back with ONE snapshot (the lagging-viewer resync), so a
  // flood costs a ~1 MB rebuild instead of 20–50 MB of parsing and ^C lands
  // on screen as soon as the ≤2 MB backlog drains.
  // Thresholds + keep-alive live in termFlow.ts (HIGH 2 MB / LOW 256 KB).
  // That watermark scheme is now the FALLBACK: pause's round trip let bytes
  // keep landing, so a fast producer overshot it (3–14 MB). A daemon that
  // grants `credit` (offered on open) sends at most CREDIT_WINDOW (1 MB)
  // unacknowledged bytes instead; `writes` acks as xterm consumes.
  // Flow + queue belong to the ENGINE (xterm + socket), not the component:
  // a parked engine takes them along and an adopter re-points their sinks
  // at itself (setSink / rebind), so the credit stream never restarts.
  const flowSink = (frame: WsTermFlowFrame): void => sendJson(frame);
  const writeSink = (bytes: Uint8Array, done: () => void): void => {
    if (!term) return done();
    term.write(bytes, done);
  };
  const canSendNow = (): boolean => sock?.readyState === WebSocket.OPEN;
  let flow = new TermFlow(flowSink);
  // A3: received bytes wait in `writes` and reach xterm ≤ 64 KB at a time
  // with ≤ 128 KB inside it, so the undroppable part of a backlog is tiny.
  // A keystroke over a big queue (^C mid-flood) drops the queue and asks for
  // ONE `resync` snapshot: the interrupt shows up after ≤ 128 KB of parsing
  // plus one snapshot, not after the whole ≤ 2 MB backlog scrolled past.
  let writes = new WriteQueue(writeSink, flow, canSendNow);
  /** A `resync` is in flight: don't ask again until its snapshot lands. */
  let resyncPending = false;
  function resyncOnInput(): void {
    if (resyncPending || !sock || sock.readyState !== WebSocket.OPEN) return;
    resyncPending = writes.resyncOnInput(() =>
      sendJson({ type: 'resync', lines: term?.options.scrollback ?? scrollback } satisfies WsTermResyncFrame),
    );
  }

  // ── sendSeqToTerm (Task 5.2) ─────────────────────────────────────────────
  // Shared send path for TermKeysBar — identical to what term.onData uses.
  // readOnly is enforced here so viewer shares can't type via the accessory bar.
  function sendSeqToTerm(seq: string): void {
    if (readOnly) return;
    sendJson({ type: 'input', data: textToBase64(seq) });
    resyncOnInput();
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

  // ── Pasted-image upload ───────────────────────────────────────────────────
  /** Store a pasted image on the DAEMON and type its path into the PTY.
   *  Agent CLIs (claude, codex) take an image as a file path, and that path has
   *  to resolve where the CLI runs — the daemon's machine — which is NOT the
   *  browser's whenever Otto is driven remotely. `POST /snips` already owns
   *  "bytes in → file on disk → path out" (it backs the snipping tool), so the
   *  paste rides that rather than inventing a second image store. */
  async function uploadPastedImage(file: File): Promise<void> {
    if (socketFactory || readOnly) return;
    const targetSession = sessionId;
    const targetSocket = sock;
    const targetTransform = transformFrame;
    try {
      const png = await toPngBytes(file);
      // Raw image/png body (no base64 inflation).
      const snip = await snipApi.uploadPng(new Blob([png as Uint8Array<ArrayBuffer>], { type: 'image/png' }));
      // Bracketed paste: the path lands as one literal chunk the TUI will not
      // auto-submit, so the user can still type a prompt around it.
      if (readOnly || sessionId !== targetSession || sock !== targetSocket ||
          !targetSocket || targetSocket.readyState !== WebSocket.OPEN || transformFrame !== targetTransform) {
        throw new Error('The original terminal is no longer available. Paste the image again in the intended session.');
      }
      const input = { type: 'input', data: textToBase64(`\x1b[200~${snip.path}\x1b[201~`) };
      const frame = targetTransform ? targetTransform(input) : input;
      if (frame !== null) targetSocket.send(JSON.stringify(frame));
    } catch (e) {
      toastError('Couldn’t paste image', e);
    }
  }

  /** PNG bytes for an arbitrary pasted image. The daemon stores PNG only
   *  (`store_snip`'s `png_dims` rejects anything else) and a clipboard image is
   *  frequently JPEG or TIFF — re-encode through a canvas instead of refusing
   *  the paste. PNG input is passed through untouched (no needless re-encode). */
  async function toPngBytes(file: File): Promise<Uint8Array> {
    if (file.type === 'image/png') return new Uint8Array(await file.arrayBuffer());
    const bitmap = await createImageBitmap(file);
    const canvas = document.createElement('canvas');
    canvas.width = bitmap.width;
    canvas.height = bitmap.height;
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('canvas 2d context unavailable');
    ctx.drawImage(bitmap, 0, 0);
    bitmap.close();
    const blob = await new Promise<Blob | null>((res) => canvas.toBlob(res, 'image/png'));
    if (!blob) throw new Error('could not re-encode the image as PNG');
    return new Uint8Array(await blob.arrayBuffer());
  }

  /** `view`: attach view-only (never resumes a suspended session). Default:
   *  view-only unless `resumeOnOpen`. Explicit user actions pass false. */
  function connect(opts: { view?: boolean } = {}): void {
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    // Enforce ONE socket per Terminal. connect() is reached from several paths
    // (initial fit, session-switch, scheduleReconnect, and the online/visibility
    // retry); a daemon restart + window re-activate can race two of them. Since
    // the PTY output is BROADCAST to every subscribed WS client, a leaked second
    // socket makes the terminal write each byte — including the shell's echo of
    // your keystrokes — twice (N sockets → N× duplication). Detach the old
    // socket's handlers (so its onclose can't scheduleReconnect for a superseded
    // connection) and close it before opening the new one.
    if (sock) {
      const old = sock;
      old.onopen = null;
      old.onmessage = null;
      old.onerror = null;
      old.onclose = null;
      try { old.close(); } catch { /* already closing/closed */ }
      sock = null;
    }
    closedByUs = false;
    disconnected = false;
    connectedSid = sessionId;
    compactPending = false;
    snapshotEpoch = null;
    // The snapshot this socket requests on open rebuilds the screen anyway.
    localReflowed = false;
    compactDeferred = false;
    compactGrid = null;
    compactQueue.cancel(compactClient);
    keyLat?.reset();
    // A fresh server stream starts unpaused. Bytes still queued in front of
    // xterm belong to the old stream and are superseded by the snapshot this
    // socket requests on open — drop them. What is already inside xterm stays
    // counted (`flow.pending`) and parses; the snapshot resets after it.
    flow.resetStream();
    writes.dropQueued();
    resyncPending = false;
    viewAttach = opts.view ?? !resumeOnOpen;
    dormantView = false;
    // Every bearer — the guest's share token AND the owner's login token —
    // travels in the otto-bearer subprotocol (Sec-WebSocket-Protocol), never
    // the URL query string, so it stays out of tunnel/proxy access logs (S1-13).
    if (socketFactory) {
      sock = socketFactory();
    } else if (shareToken) {
      const wsBase = wsUrl(`/ws/term/${sessionId}`).replace(/\?token=.*$/, '');
      sock = new WebSocket(wsBase, [WS_BEARER_SUBPROTOCOL, shareToken]);
    } else {
      sock = wsConnect(`/ws/term/${sessionId}${viewAttach ? '?view=1' : ''}`);
    }
    sock.binaryType = 'arraybuffer';
    wireSocket(sock);
  }

  /** Apply a `scrollback` snapshot (JSON or binary form, perf 01 N3). */
  function applySnapshot(epoch: number | null, snap: Uint8Array | null): void {
    compactQueue.done(compactClient);
    // A delayed optional compact must not erase a selection or reading
    // position established after its request. A new process/connection still rebuilds: its
    // epoch differs (or was cleared on connect).
    // A `resync` reply always rebuilds: its request already dropped
    // the queued bytes this snapshot replaces.
    const buffer = term?.buffer.active;
    const st = { compactPending, resyncPending, snapshotEpoch };
    const applies = snapshotApplies(st, epoch, () =>
      !!term?.hasSelection() || (!!buffer && buffer.baseY - buffer.viewportY > 3));
    compactPending = st.compactPending;
    snapshotEpoch = st.snapshotEpoch;
    if (!applies) return;
    // A snapshot fully reconstructs terminal state: history rows +
    // coherent current-screen frame + input modes (bracketed paste,
    // keypad). ALWAYS reset and rebuild from it — appending under the
    // locally-kept scrollback stacked a duplicate copy of the whole
    // transcript on every reconnect (the "scroll up and see the
    // conversation N times" bug), and preserved history could belong
    // to a dead process painted at a stale width. Deterministic
    // rebuild keeps the buffer identical to what a fresh attach sees.
    if (snap?.length) {
      // Everything received before this frame is already in it: the
      // queued part never needs to parse (A3).
      writes.dropQueued();
      resyncPending = false;
      if (writes.inflight === 0) {
        term?.reset();
        // Snapshot is a full-screen paint already; still force a clean
        // redraw so nothing from the previous process lingers.
        paintPtyBytes(snap, /* alwaysRedraw */ true);
      } else {
        // Output is still queued inside xterm (a flow-control resume
        // or lag resync mid-flood). term.reset() is synchronous but
        // the queue is not cleared, so those stale bytes would parse
        // AFTER the reset, above the rebuilt history. Reset in-order
        // instead: RIS (ESC c) makes xterm call the same reset() when
        // the parser reaches it, i.e. after the backlog.
        paintPtyBytes(withInOrderReset(snap), /* alwaysRedraw */ true);
      }
    }
  }

  /** This component's handlers on `s` — a socket it just opened, or one it
   *  adopted from the parking lot (already open: onopen never fires). */
  function wireSocket(s: WebSocket): void {
    s.onopen = () => {
      connected = true;
      reconnecting = false;
      reconnectAttempts = 0;
      // Credit flow control first: frames sent after the daemon's `credit`
      // reply count against the window (an older daemon ignores the offer
      // and this socket stays on pause/resume).
      // Binary snapshots (N3) only on a direct /ws/term socket: the room relay
      // (socketFactory) rejects unknown fields on its `credit` frame.
      flow.offer(!socketFactory);
      // Primary pane: take size authority BEFORE pushing our grid, so a
      // passive viewer attaching later can't stomp it (see claimOnAttach doc).
      if (claimOnAttach && !readOnly) sendJson({ type: 'claim' });
      // Re-measure against the CURRENT container box before syncing the PTY: the
      // earlier first-fit may have run while the pane was briefly narrow, and a
      // plain shell can't reflow stale output later — so fit here (forced resize
      // guarantees the freshly-(re)attached PTY gets our real grid, not the
      // server's spawn-time 80×24), then verify again as layout settles.
      safeFit();
      // Attach WITH our grid (perf F1): a viewer that may resize has the
      // daemon resize the PTY + emulator BEFORE it captures the snapshot, so
      // the snapshot already matches this xterm and the TUI's SIGWINCH
      // repaint arrives as live bytes. Recording the grid as sent makes the
      // forced confirm below find nothing changed — no follow-up compact, one
      // snapshot per attach (it used to be two: one at the old PTY grid, then
      // a compact once the forced resize confirmed).
      const want = term?.options.scrollback ?? scrollback;
      const req: WsTermScrollbackRequestFrame = { type: 'scrollback', lines: want };
      if (term && !readOnly) {
        req.cols = term.cols;
        req.rows = term.rows;
        lastCols = term.cols;
        lastRows = term.rows;
        localReflowed = false;
      }
      sendJson(req);
      sendResize(true);
      verifyFitSoon();
    };

    s.onmessage = (ev: MessageEvent) => {
      if (ev.data instanceof ArrayBuffer) {
        const bytes = new Uint8Array(ev.data);
        // The payload of a binary snapshot header (perf 01 N3): a snapshot,
        // not live output — never credited, never painted as a PTY burst.
        const hdr = binarySnapHeaders.get(s);
        if (hdr) {
          binarySnapHeaders.delete(s);
          applySnapshot(hdr.epoch ?? null, bytes);
          return;
        }
        // write() only updates the buffer + marks dirty cells; the renderer then
        // paints *those* cells. Agent TUIs rewrite status/prompt rows in place —
        // if a cell is no longer dirty, the previous frame stays (cursor ghosts,
        // stacked "-- INSERT --" lines). Agent panes get ONE trailing full
        // viewport repaint per output burst; shells only after cursor/erase
        // frames (paintPtyBytes).
        paintPtyBytes(bytes, false, flow.credit, keyLat?.frame(performance.now(), () => performance.now()));
        return;
      }
      if (typeof ev.data !== 'string') return;
      try {
        const msg = JSON.parse(ev.data);
        switch (msg.type) {
          case 'credit':
            flow.granted(typeof msg.window === 'number' ? msg.window : undefined);
            break;
          case 'scrollback':
            if (msg.binary) {
              // Header only; the bytes are the next (binary) frame.
              binarySnapHeaders.set(s, { epoch: msg.epoch });
              break;
            }
            applySnapshot(msg.epoch ?? null, msg.data ? base64ToBytes(msg.data) : null);
            break;
          case 'status':
            // A live status after an `exit` means the server moved this
            // socket onto a respawned process (chat send, channel follow-up,
            // restart from elsewhere, a daemon restart that re-adopted it) —
            // drop the exited overlay; the accompanying snapshot rebuilds the
            // screen.
            if (msg.status !== 'exited' && msg.status !== 'reconnectable') {
              exitCode = null;
              dormantView = false;
            } else if (
              (msg.status === 'reconnectable' || resumable) &&
              viewAttach &&
              (exitCode === null || probingAfterExit) &&
              !socketFactory
            ) {
              // View-only attach to a suspended session: nothing was spawned,
              // so show the exited overlay (Resume) — typing wakes it too.
              exitCode = 0;
              dormantView = true;
            }
            probingAfterExit = false;
            exitProbes = 0;
            onstatus?.(msg.status as SessionStatus);
            break;
          case 'exit':
            exitCode = msg.code ?? 0;
            break;
          case 'error':
            term?.write(`\r\n\x1b[31m[otto] ${msg.code}: ${msg.message ?? ''}\x1b[0m\r\n`);
            break;
          case 'probe_ack': {
            const ack = msg as WsTermProbeAckFrame;
            probes?.ack(typeof ack.id === 'number' ? ack.id : -1, performance.now());
            echoStats = ack.echo ?? null;
            break;
          }
          case 'search_result': {
            // Server-side ring-buffer search result. Populate the find-bar's
            // match list (next/prev navigation) and forward to any parent listener.
            const frame = msg as WsSearchResultFrame;
            serverSearchPending = false;
            // Only apply if the result matches the current query (avoid a
            // stale response overwriting results from a newer, faster one).
            if (frame.query === findQuery) {
              serverMatches = frame.matches;
              serverMatchIdx = -1;
              // Jump only when the client buffer has no hit of its own (the
              // newest ring-buffer match): auto-jumping to match 0 — the
              // OLDEST line of history — yanked the viewport off the local
              // hit a beat after every keystroke. A local pass still pending
              // makes that call itself when it lands (scheduleLocalFind).
              if (frame.matches.length > 0 && localMiss && localFindTimer === null) goToServerMatch(frame.matches.length - 1);
            }
            onsearchresult?.(frame);
            break;
          }
        }
      } catch {
        /* ignore malformed control frame */
      }
    };

    s.onclose = () => {
      connected = false;
      compactQueue.cancel(compactClient);
      if (closedByUs) return;
      if (exitCode === null) {
        disconnected = true;
        scheduleReconnect();
      } else if (!socketFactory && exitProbes < MAX_EXIT_PROBES) {
        // See `probingAfterExit`: learn the session's state after the drop.
        exitProbes++;
        probingAfterExit = true;
        scheduleReconnect(true);
      }
    };
  }

  // ── Resize policy (the "base model") ────────────────────────────────────────
  // The client xterm buffer is the single source of truth for the live display:
  // it reflows its own scrollback natively on every fit(), exactly like a
  // desktop terminal. The PTY is told about a new grid exactly ONCE per settled
  // resize (pure trailing debounce — never mid-drag, never leading), because
  // every SIGWINCH makes claude/codex reprint their live region and codex
  // re-emits transcript lines that permanently pollute scrollback. We never
  // rebuild the buffer from a server snapshot while connected — snapshots are
  // for (re)attach and the server's own lagged-drop recovery only. Rebuilding
  // mid-session is what used to smear codex duplicates over the live view and
  // yank the user's scroll position/prompt state.
  const RESIZE_SETTLE_MS = 200;
  // A settled grid must additionally re-measure IDENTICAL this much later
  // before we SIGWINCH. A macOS window/fullscreen animation can pause longer
  // than the settle window mid-flight — one such pause measured 69×44 for
  // 0.8s on a 165×48 pane, and claude re-rendered its transcript at 69 cols
  // into scrollback (permanent narrow block). A mid-animation box keeps
  // changing, so it never confirms; only the final geometry is ever sent.
  const RESIZE_CONFIRM_MS = 150;
  let resizeSendTimer: ReturnType<typeof setTimeout> | null = null;
  /** Set while a forced (connect/focus) sync is pending confirmation. */
  let resizeForcePending = false;
  /** Grid captured at settle, awaiting the identical re-measure. */
  let confirmGrid: { cols: number; rows: number } | null = null;
  // Last grid actually SENT to the PTY (not the local xterm grid, which may be
  // ahead of it while a trailing send is pending).
  let lastCols = 0;
  let lastRows = 0;
  /** The LOCAL xterm grid changed since the last resize decision — xterm
   *  reflowed its own buffer. With the PTY back at the same grid nothing
   *  repaints the TUI, so the decision asks for a snapshot (G1). */
  let localReflowed = false;

  /** Agent TUI panes measure while their box settles and resize xterm only
   *  once the grid is stability-confirmed (confirmResizeStep): the passing
   *  sizes of a tab switch, a split animation or a window restore never
   *  touch the buffer. Shells (their scrollback reflows natively) and phones
   *  (the soft keyboard must not wait) resize immediately. */
  function deferLocalResize(): boolean {
    return preferDom && connected && !viewport.isPhone;
  }

  function cancelResizeTimer(): void {
    if (resizeSendTimer !== null) {
      clearTimeout(resizeSendTimer);
      resizeSendTimer = null;
    }
    confirmGrid = null;
  }

  /** Stability step: re-measure; commit only when two consecutive
   *  measurements RESIZE_CONFIRM_MS apart agree. */
  function confirmResizeStep(): void {
    resizeSendTimer = null;
    if (!term || !connected) {
      confirmGrid = null;
      resizeForcePending = false;
      return;
    }
    // Re-measure the CURRENT box (no-op when unchanged/not laid out). A TUI
    // pane only measures here; the grid is applied once it is confirmed.
    const defer = deferLocalResize();
    const measured = safeFit(!defer) ? measuredGrid : null;
    const cols = measured?.cols ?? term.cols;
    const rows = measured?.rows ?? term.rows;
    if (confirmGrid && confirmGrid.cols === cols && confirmGrid.rows === rows) {
      confirmGrid = null;
      const force = resizeForcePending;
      resizeForcePending = false;
      if (defer && measured) applyGrid(measured, 'confirmed');
      const sentChanged = term.cols !== lastCols || term.rows !== lastRows;
      const prev = { cols: lastCols, rows: lastRows };
      const next = { cols: term.cols, rows: term.rows };
      // Forced path pushes even when unchanged: the server may hold a
      // different grid (daemon restart / another viewer) and drops same-size
      // resizes before the ioctl, so this is free when nothing changed. A
      // local reflow with an unchanged PTY grid sends nothing but still
      // compacts (resizeDecision).
      const d = resizeDecision({ sentChanged, localReflowed, force, preferDom, prev, next });
      localReflowed = false;
      if (d.send) {
        lastCols = term.cols;
        lastRows = term.rows;
        // Resize-with-grid compact (perf 01 N6): a widened agent pane that
        // can compact NOW sends ONE `scrollback` carrying the new grid
        // through the window-wide queue — the daemon reflows + resizes the
        // PTY and captures atomically, so there is no separate `resize`, no
        // RESIZE_COMPACT_MS wait and no second round trip. The TUI's
        // SIGWINCH repaint then streams in after the snapshot (as on attach).
        // Viewers that may not resize, panes off-screen / in a hidden window
        // or with a compact already in flight keep the resize + deferred
        // compact path.
        if (d.compact && preferDom && !readOnly && snapshotEpoch !== null && !compactPending && compactEligible()) {
          compactGrid = { cols: lastCols, rows: lastRows };
          if (resizeCompactTimer !== null) {
            clearTimeout(resizeCompactTimer);
            resizeCompactTimer = null;
          }
          compactQueue.request(compactClient);
          return;
        }
        // A grid still waiting in the queue is superseded by this one.
        compactGrid = null;
        sendJson({ type: 'resize', cols: lastCols, rows: lastRows });
      }
      if (d.compact) scheduleResizeCompact();
      return;
    }
    confirmGrid = { cols, rows };
    resizeSendTimer = setTimeout(confirmResizeStep, RESIZE_CONFIRM_MS);
  }

  // One-shot COMPACT after the TUI has repainted at the confirmed grid:
  // rebuild from the server snapshot, exactly what a manual reconnect did.
  // A bottom-anchored TUI (claude) repaints its live region at the screen
  // bottom after a big widen while the rejoined transcript above moved up —
  // leaving a large blank void between them (the "maximize after a split
  // looks broken until I reconnect" report). The server emulator holds the
  // contiguous truth, so one deterministic rebuild closes the gap. Safe now
  // (unlike the old mid-resize resync): geometry is stability-confirmed
  // first, codex's 3J keeps the emulator duplicate-free, and the rebuild is
  // skipped while the user is scrolled up reading (it would yank the
  // viewport to the bottom). Applies to DOM-rendered panes, including shells
  // in browser mode; selection guards below protect copy in both cases.
  const RESIZE_COMPACT_MS = 900;
  let compactPending = false;
  let snapshotEpoch: number | null = null;
  let resizeCompactTimer: ReturnType<typeof setTimeout> | null = null;
  function scheduleResizeCompact(): void {
    if (!preferDom) return;
    if (resizeCompactTimer !== null) clearTimeout(resizeCompactTimer);
    resizeCompactTimer = setTimeout(runResizeCompact, RESIZE_COMPACT_MS);
  }
  function runResizeCompact(): void {
    resizeCompactTimer = null;
    if (!term || !connected) return;
    // Wait for attach's epoch, then allow only one optional snapshot in flight.
    // Otherwise the first reply
    // clears compactPending and a later reply looks like an attach, bypassing
    // the selection/reading guard. WS replies are ordered but can be delayed.
    if (snapshotEpoch === null || compactPending) {
      scheduleResizeCompact();
      return;
    }
    if (resizeSendTimer !== null) {
      // Grid still moving (pane add/remove animates the tile layout for a
      // while — a confirm cycle is often in flight when this fires). RETRY
      // after it settles instead of abandoning: dropping the compact here is
      // what left "played with session counts → broken until I reconnect".
      scheduleResizeCompact();
      return;
    }
    // Off-screen or in a hidden window: nobody sees the pane, so its 1–2 MB
    // rebuild waits until it comes back (onCompactWake). Otherwise take a
    // turn in the window-wide queue (one compact in flight, focused first).
    if (!compactEligible()) {
      compactDeferred = true;
      return;
    }
    compactQueue.request(compactClient);
  }

  /** Is this pane on screen, in a visible window, with a live socket? */
  let onScreen = true;
  function compactEligible(): boolean {
    return !!term && connected && onScreen && document.visibilityState === 'visible';
  }
  /** A compact was due while the pane was hidden / off-screen. */
  let compactDeferred = false;
  /** Grid riding on the queued compact instead of a `resize` (perf 01 N6). */
  let compactGrid: { cols: number; rows: number } | null = null;
  /** The queued compact will not carry its grid after all (declined, pane
   *  hidden, parked): the PTY still needs the size — send it as a `resize`. */
  function flushCompactGrid(): void {
    const g = compactGrid;
    compactGrid = null;
    if (g && connected) sendJson({ type: 'resize', cols: g.cols, rows: g.rows });
  }
  const compactClient: CompactClient = {
    lastFocus: () => gpuClient.lastFocus,
    eligible: () => {
      const ok = compactEligible();
      if (!ok && connected) {
        compactDeferred = true;
        flushCompactGrid();
      }
      return ok;
    },
    run: () => {
      if (!term || compactPending) {
        flushCompactGrid();
        return false;
      }
      const buf = term.buffer.active;
      // Preserve an active selection: rebuilding resets xterm's selection and
      // would erase a drag just before the user copies it.
      // Skip also when the user is CLEARLY reading scrollback — a TUI repaint
      // routinely leaves the viewport a row or two shy of the bottom.
      if (buf.baseY - buf.viewportY > 3 || term.hasSelection()) {
        flushCompactGrid();
        return false;
      }
      compactPending = true;
      const req: WsTermScrollbackRequestFrame = { type: 'scrollback', lines: term.options.scrollback ?? scrollback };
      if (compactGrid) {
        req.cols = compactGrid.cols;
        req.rows = compactGrid.rows;
        compactGrid = null;
      }
      sendJson(req);
      return true;
    },
  };
  /** The pane came into view / the window became visible / it was focused:
   *  run the compact it skipped while hidden. */
  function onCompactWake(): void {
    if (!compactDeferred || !compactEligible()) return;
    compactDeferred = false;
    runResizeCompact();
  }

  // ── Ack withholding (perf F9, TermFlow.hold) ───────────────────────────────
  // A non-focused pane in a hidden window (another Space, minimized) keeps
  // parsing what already arrived but stops acknowledging it: the daemon
  // stops after one credit window and, only if output overflowed meanwhile,
  // sends ONE snapshot when the window shows again. The pane the user last
  // worked in keeps streaming (PR A's background-latency fix).
  function isFocusedPane(): boolean {
    if (gpuClient.lastFocus === 0) return false;
    for (const c of gpuClients) if (c.lastFocus > gpuClient.lastFocus) return false;
    return true;
  }
  function syncAckHold(): void {
    flow.hold(document.visibilityState === 'hidden' && !isFocusedPane());
  }

  function sendResize(force = false): void {
    if (!term) return;
    // Reflect the live grid on the host element — handy for debugging stale
    // grids (shows up as a too-small data-cols) and asserted by the resize E2E.
    container?.setAttribute('data-cols', String(term.cols));
    if (force) {
      // Connect-time / focus-reclaim sync: skip the settle wait but still
      // require the stability confirmation (an attach can race a window
      // restore animation just like a drag can).
      resizeForcePending = true;
      cancelResizeTimer();
      confirmResizeStep();
      return;
    }
    const grid = deferLocalResize() && measuredGrid ? measuredGrid : term;
    if (resizeSendTimer === null && grid.cols === lastCols && grid.rows === lastRows && !localReflowed) return;
    cancelResizeTimer();
    resizeSendTimer = setTimeout(confirmResizeStep, RESIZE_SETTLE_MS);
  }

  // Belt-and-suspenders for PLAIN SHELLS: re-fit a couple of times shortly after
  // the socket opens. The first fit can run while the pane is briefly narrow (a
  // split/tab open animation), and a shell's scrollback can't reflow to a later
  // resize the way an alt-screen TUI repaints — so a stale narrow grid would
  // stick and wrap every line. Re-measuring once layout settles corrects xterm
  // (which reflows) and pushes the real grid to the PTY (only when it changed,
  // so claude/codex don't get spurious SIGWINCH flicker).
  let verifyTimers: ReturnType<typeof setTimeout>[] = [];
  function verifyFitSoon(): void {
    for (const t of verifyTimers) clearTimeout(t);
    verifyTimers = [200, 650].map((ms) =>
      setTimeout(() => {
        if (!term || !connected) return;
        if (safeFit(!deferLocalResize())) sendResize();
      }, ms),
    );
  }

  // Guarded fit: only run when the container is actually visible and has real
  // dimensions. On first open the pane lives inside a CSS grid that isn't laid
  // out during the synchronous mount tick, so clientWidth/clientHeight are 0 —
  // fitting then computes a garbage grid (wrong cols/rows) which the PTY snaps
  // its scrollback to → garbled wrapping / broken scroll until the next re-fit.
  // Skipping the fit when 0×0 (or when proposeDimensions() can't measure)
  // guarantees we only ever push a correct grid to xterm and the backend.
  /** A measured grid plus what applying it needs (see applyGrid). */
  interface FitGrid {
    cols: number;
    rows: number;
    /** Columns that actually fit the box (cols may exceed it: MIN_FIT_COLS). */
    fitCols: number;
    cellWidth: number;
  }
  /** The grid the last successful safeFit measured. */
  let measuredGrid: FitGrid | null = null;

  /** Resize the xterm to `g` (it reflows its buffer when the grid differs). */
  function applyGrid(g: FitGrid, reason: string): void {
    if (!term) return;
    if (term.element) {
      const scrollbar = term.options.scrollback === 0 ? 0 : (term.options.overviewRuler?.width || 15);
      horizontalOverflow = g.cols > g.fitCols;
      term.element.style.width = horizontalOverflow
        ? `${Math.ceil(g.cols * g.cellWidth + scrollbar)}px` : '100%';
    }
    const from = `${term.cols}x${term.rows}`;
    term.resize(g.cols, g.rows);
    const to = `${term.cols}x${term.rows}`;
    if (from !== to) {
      localReflowed = true;
      // Diagnostics next to the daemon's "pty resize" log: which layout step
      // produced a passing grid on the affected Mac.
      if (latencyOn) console.debug(`[otto term ${sessionId}] local resize ${from} → ${to} (${reason}, pty ${lastCols}x${lastRows})`);
    }
    container?.setAttribute('data-cols', String(term.cols));
  }

  function safeFit(apply = true): boolean {
    if (!term || !fit || !container) return false;
    if (container.clientWidth < 1 || container.clientHeight < 1) return false;
    let dims: { cols: number; rows: number } | undefined;
    try {
      dims = fit.proposeDimensions();
    } catch {
      return false; // container not laid out / detached
    }
    if (!dims || !Number.isFinite(dims.cols) || !Number.isFinite(dims.rows)) return false;
    // Never send an intermediate narrow grid: provider TUIs can permanently
    // hard-wrap their transcript on SIGWINCH. First choose readable metrics,
    // then resize once to the final grid. Narrow desktop panes scroll locally.
    if (dims.rows < 3) return false;
    // A maximized right panel can leave only a sliver of terminal visible.
    // Preserve its existing grid entirely, as before, while keeping those
    // columns reachable inside the host rather than repainting at 80 columns.
    if (dims.cols < 20) {
      horizontalOverflow = true;
      const screen = term.element?.querySelector<HTMLElement>('.xterm-screen');
      if (term.element && screen) {
        const scrollbar = term.options.scrollback === 0 ? 0 : (term.options.overviewRuler?.width || 15);
        term.element.style.width = `${Math.ceil(screen.getBoundingClientRect().width + scrollbar)}px`;
      }
      return false;
    }
    const cur = term.options.fontSize ?? effFontSize;
    // Fit the default 13px grid, then apply the user's zoom offset. Fitting
    // the requested size itself used to undo every Zoom in on narrow panes.
    const automatic = Math.max(11, Math.min(13, Math.floor(cur * dims.cols / MIN_FIT_COLS)));
    const target = viewport.isPhone ? effFontSize
      : Math.max(11, automatic + ui.termFontSize - 13);
    fittedFontSize = target;
    onfontfit?.(target);
    if (target !== cur) {
      term.options.fontSize = target;
      clearWebglAtlas();
      dims = fit.proposeDimensions();
      if (!dims) return false;
    }
    const cols = viewport.isPhone ? dims.cols : Math.max(MIN_FIT_COLS, dims.cols);
    try {
      // Use the same measured cell metrics as FitAddon. DOM screen bounds
      // lag resize by a render frame, so dividing them by NEW rows/columns
      // oscillates the grid and prevents the stability confirmation below.
      const cell = (term as unknown as {
        _core: { _renderService: { dimensions: { css: { cell: { width: number; height: number } } } } };
      })._core._renderService.dimensions.css.cell;
      // clientHeight excludes classic horizontal scrollbars; overlay macOS
      // scrollbars consume no height. Keep the last terminal row reachable.
      const rows = Math.floor(container.clientHeight / cell.height);
      const grid: FitGrid = {
        cols,
        rows: Number.isFinite(rows) && rows >= 3 ? rows : dims.rows,
        fitCols: dims.cols,
        cellWidth: cell.width,
      };
      measuredGrid = grid;
      if (apply) applyGrid(grid, 'fit');
    } catch {
      return false; // detached mid-fit
    }
    if (target !== cur) forceViewportRefresh();
    return true;
  }

  // A stable minimum preserves provider transcript wrapping in split/tile
  // views. At the 11px readability floor, overflow stays inside the terminal.
  const MIN_FIT_COLS = 80;

  /** Cap below which a shell frame MAY be interactive (not a stream dump).
   *  Only used when we are NOT in full-redraw mode (plain shells on WebGL). */
  const TUI_FRAME_BYTES = 4096;

  /** Agent-pane ghost clean-up. xterm already repaints the dirty rows of
   *  every frame; the forced FULL repaint only mops up rows a TUI rewrote
   *  without dirtying them. It ran on every frame (SA-03), then as a leading
   *  5 Hz throttle — which, on the DOM renderer, was the pane's steady cost
   *  (~30 % of a core per working pane, r3-12-01). Now a trailing debounce:
   *  one repaint once a burst goes quiet (TUI_CLEANUP_QUIET_MS), at the
   *  latest TUI_CLEANUP_MAX_WAIT_MS after its first frame, so a ghost lives
   *  ≤ 1 s even under a spinner that never stops. */
  const tuiCleanup = new QuietRepaint(() => scheduleFullRedraw());

  /**
   * Apply PTY bytes, then REDRAW — not just "draw the dirty cells".
   *
   * xterm's normal path: parse → mark dirty cells → renderer paints dirty only.
   * Agent CLIs (claude/codex/grok) are full-screen TUIs that overwrite the same
   * rows (status bar, prompt, cursor). When a cell stops being "dirty" but its
   * previous content should be gone, partial paint leaves ghosts. So after the
   * buffer has absorbed the frame we force every visible row to repaint.
   *
   * - Agent panes (`preferDom`): one trailing full redraw per burst (tuiCleanup).
   * - Shell panes: next-frame full redraw only on small frames that move the
   *   cursor or erase (↑ history etc.).
   * - `alwaysRedraw`: snapshots / forced paths — next-frame full redraw.
   *
   * Every write goes through the `writes` queue (≤ 64 KB slices, ≤ 128 KB
   * inside xterm) and is counted for flow control (`flow`, termFlow.ts).
   */
  function paintPtyBytes(bytes: Uint8Array, alwaysRedraw = false, stream = 0, onParsed?: () => void): void {
    // No emulator: the queue still settles (and acks) the bytes, or a
    // credited stream would leak window.
    if (!term) return writes.push(bytes, onParsed, stream);
    const n = bytes.byteLength;
    const redraw: (() => void) | null = alwaysRedraw
      ? scheduleFullRedraw
      : preferDom
        ? pokeTuiCleanup
        : n < TUI_FRAME_BYTES && hasCursorOrErase(bytes)
          ? scheduleFullRedraw
          : null;
    const after = onParsed && redraw ? () => { onParsed(); redraw(); } : (onParsed ?? redraw ?? undefined);
    writes.push(bytes, after, stream);
  }

  /** Force the emulator to repaint every visible row (a true redraw). Does
   *  NOT drop the WebGL glyph atlas: re-rasterizing every glyph cost ~3 ms per
   *  small shell frame (SA-03). The font/size/theme paths that actually
   *  invalidate glyphs call clearWebglAtlas() themselves before this. */
  function forceViewportRefresh(): void {
    if (!term) return;
    try {
      term.refresh(0, Math.max(0, term.rows - 1));
    } catch {
      /* disposed mid-frame */
    }
  }

  /** Coalesce full-viewport redraws onto one animation frame per burst of PTY
   *  output so multi-chunk TUI updates pay one repaint, not N. */
  function scheduleFullRedraw(): void {
    if (tuiRefreshRaf !== null) return;
    tuiRefreshRaf = requestAnimationFrame(() => {
      tuiRefreshRaf = null;
      forceViewportRefresh();
    });
  }

  /** Agent-pane clean-up hook (runs when a frame finished parsing). */
  function pokeTuiCleanup(): void {
    tuiCleanup.poke();
  }

  /** Load the WebGL renderer when this terminal wants it and the window's GPU
   *  budget allows; otherwise xterm keeps its DOM renderer. */
  function attachWebgl(): void {
    if (!term || webglAddon || !webglWanted) return;
    if (liveWebgl >= MAX_WEBGL_TERMINALS) return;
    try {
      const webgl = new WebglAddon();
      // xterm itself waits 3 s for `webglcontextrestored` after a loss and,
      // when the context comes back, rebuilds its atlas and redraws. Only a
      // context that stays lost lands here: drop to the DOM renderer (never a
      // black canvas), repaint, and retry the GPU a couple of times later
      // (GPU reset / wake from sleep usually recovers).
      webgl.onContextLoss(() => {
        dropWebgl(webgl);
        forceViewportRefresh();
        scheduleWebglRetry();
      });
      term.loadAddon(webgl);
      webglAddon = webgl;
      liveWebgl++;
    } catch {
      // WebGL2 unavailable (headless, blocked GPU): DOM renderer.
      webglAddon = null;
    }
  }

  /** Dispose the WebGL renderer (xterm falls back to DOM) and release its
   *  slot in the budget. */
  function dropWebgl(which: WebglAddon | null = webglAddon): void {
    if (!which) return;
    try {
      which.dispose();
    } catch {
      /* already disposed */
    }
    if (webglAddon === which) {
      webglAddon = null;
      releaseGpuSlot();
    }
  }

  function scheduleWebglRetry(): void {
    if (webglRetryTimer !== null || webglRetries >= WEBGL_MAX_RETRIES) return;
    const delay = Math.min(WEBGL_RETRY_MS * 2 ** webglRetries, WEBGL_RETRY_MAX_MS);
    webglRetries++;
    webglRetryTimer = setTimeout(() => {
      webglRetryTimer = null;
      if (!term || webglAddon || !webglWanted) return;
      attachWebgl();
      if (webglAddon) forceViewportRefresh();
      // Still DOM with a free slot: the context could not be created yet
      // (GPU still waking). A full budget hands the slot over when one frees.
      else if (liveWebgl < MAX_WEBGL_TERMINALS) scheduleWebglRetry();
    }, delay);
  }

  /** The user is back (window focus, page visible, pane on screen or
   *  focused): try the GPU again now and restart the backoff. */
  function retryWebglNow(): void {
    if (!term || !webglWanted || webglAddon) return;
    cancelWebglRetry();
    webglRetries = 0;
    attachWebgl();
    if (webglAddon) forceViewportRefresh();
  }

  function cancelWebglRetry(): void {
    if (webglRetryTimer !== null) clearTimeout(webglRetryTimer);
    webglRetryTimer = null;
  }

  /** Drop cached WebGL glyph tiles after metrics/theme change so the next
   *  paint can't reuse stale cells from the previous font/palette. */
  function clearWebglAtlas(): void {
    try {
      webglAddon?.clearTextureAtlas();
    } catch {
      /* addon disposed */
    }
  }


  /** Open (or re-focus) the find bar — ⌘F, the pane header's search button.
   *  Re-opening selects the query so typing replaces it. */
  export function openFind(): void {
    findOpen = true;
    queueMicrotask(() => {
      findInput?.focus();
      findInput?.select();
    });
  }
  // The find input owns ⌘F while focused (the xterm textarea just blurred):
  // without this a second ⌘F from the bar opened the page-wide find on top.
  function onFindFocus(): void {
    keyContext.openFind = openFind;
  }
  function onFindBlur(): void {
    if (keyContext.openFind === openFind) keyContext.openFind = null;
  }
  // Pane ownership (findRank): the active session pane takes ⌘F while it is
  // bound and on screen, even with focus elsewhere. A parked / switched-away
  // engine leaves `term` null (rank 0); a hidden page (display:none keep-alive)
  // fails checkVisibility.
  $effect(() => {
    const r = findRank;
    if (r <= 0) return;
    return registerFindOwner({
      rank: () => {
        if (!term || !container?.isConnected) return 0;
        const el = container as HTMLElement & { checkVisibility?: (o?: object) => boolean };
        const shown = typeof el.checkVisibility === 'function' ? el.checkVisibility({ visibilityProperty: true }) : el.offsetParent !== null;
        return shown ? r : 0;
      },
      pane: () => container?.closest('.pane') ?? container ?? null,
      open: openFind,
    });
  });

  // Local find is a whole-buffer SearchAddon pass (13–19 ms over 10k lines
  // incl. up to 1000 decorations, SA-12) — debounce it like the server search,
  // and skip the highlight-all decorations for a 1-character query (matches
  // nearly everything; Enter still highlights).
  const LOCAL_FIND_DEBOUNCE_MS = 100;
  let localFindTimer: ReturnType<typeof setTimeout> | null = null;
  function cancelLocalFind(): void {
    if (localFindTimer !== null) {
      clearTimeout(localFindTimer);
      localFindTimer = null;
    }
  }
  function scheduleLocalFind(): void {
    cancelLocalFind();
    if (!findQuery) {
      search?.clearDecorations();
      return;
    }
    localFindTimer = setTimeout(() => {
      localFindTimer = null;
      const q = findQuery;
      if (!search || !q) return;
      // Upward from the bottom (or from the current hit, growing it while it
      // still matches): the newest output is what you're looking for — a
      // forward pass started at the TOP of the scrollback, the oldest copy.
      let found: boolean;
      if (q.length >= 2) found = search.findPrevious(q, { decorations: searchDecorations });
      else {
        search.clearDecorations();
        localIdx = -1;
        localCount = 0;
        found = search.findPrevious(q);
      }
      localMiss = !found;
      // No local hit: the ring-buffer result (maybe already in) is the only
      // place it lives — jump there.
      if (!found && serverMatches.length > 0 && serverMatchIdx < 0) goToServerMatch(serverMatches.length - 1);
    }, LOCAL_FIND_DEBOUNCE_MS);
  }
  /** The last local pass found nothing in the client's buffer. */
  let localMiss = $state(false);

  /** Drop the find bar's state (and the buffer highlights). */
  function resetFind(): void {
    findOpen = false;
    if (keyContext.openFind === openFind && document.activeElement === findInput) keyContext.openFind = null;
    cancelLocalFind();
    search?.clearDecorations();
    localIdx = -1;
    localCount = 0;
    localMiss = false;
    // Clear server-side results so they don't linger on next open.
    serverMatches = [];
    serverMatchIdx = -1;
    serverSearchPending = false;
    if (serverSearchTimer !== null) {
      clearTimeout(serverSearchTimer);
      serverSearchTimer = null;
    }
  }

  function closeFind(): void {
    resetFind();
    term?.focus();
    // onTermSelection skipped the textarea mirror while find drove the
    // selection; restore it so Edit ▸ Copy (native ⌘C) takes the last hit.
    const ta = term?.textarea;
    if (term && ta && !composing && term.hasSelection()) {
      ta.value = term.getSelection();
      ta.select();
    }
  }

  // Every match gets a fill — without matchBackground the addon paints
  // nothing in the viewport (only the overview ruler, which this terminal
  // doesn't show), so "highlight all" was invisible. xterm decorations take
  // #RRGGBB only, hence literals picked per scheme rather than tokens.
  const searchDecorations = $derived(
    effScheme === 'dark'
      ? {
          matchBackground: '#4a3d12',
          matchBorder: '#8a6d1f',
          matchOverviewRuler: '#febc2e',
          activeMatchBackground: '#8a5a00',
          activeMatchBorder: '#ff9f0a',
          activeMatchColorOverviewRuler: '#ff9f0a',
        }
      : {
          matchBackground: '#fff0b3',
          matchBorder: '#e0bb00',
          matchOverviewRuler: '#febc2e',
          activeMatchBackground: '#ffc966',
          activeMatchBorder: '#ff9f0a',
          activeMatchColorOverviewRuler: '#ff9f0a',
        },
  );

  /** Step through the matches. Terminal order, like VS Code's terminal find:
   *  ↵ / the up chevron go UP to the older match, ⇧↵ / down to the newer. The
   *  client buffer is stepped when it has the query; only when it doesn't do
   *  the ring-buffer hits step instead (stepping both fought over the
   *  viewport — every ↵ undid the local move). ↑/↓ always walk the list. */
  function findNext(newer = false): void {
    if (findQuery === '') return;
    // An explicit step supersedes the pending incremental search.
    cancelLocalFind();
    let found = false;
    if (search) {
      found = newer
        ? search.findNext(findQuery, { decorations: searchDecorations })
        : search.findPrevious(findQuery, { decorations: searchDecorations });
    }
    localMiss = !found;
    if (!found && serverMatches.length > 0) {
      goToServerMatch(newer ? serverMatchIdx + 1 : serverMatchIdx < 0 ? serverMatches.length - 1 : serverMatchIdx - 1);
    }
  }

  function onFindKey(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      findNext(e.shiftKey);
    } else if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      closeFind();
    } else if ((e.key === 'ArrowDown' || e.key === 'ArrowUp') && serverMatches.length > 0) {
      e.preventDefault();
      const down = e.key === 'ArrowDown';
      goToServerMatch(serverMatchIdx < 0 ? (down ? 0 : serverMatches.length - 1) : serverMatchIdx + (down ? 1 : -1));
    }
  }

  // Native OSC8 has xterm priority; text links supplement it without ever
  // interpreting output as a shell command or application route.
  function localFileContext(): { cwd: string; allowed: boolean } {
    const session = ws.sessions.find(s => s.id === sessionId);
    return { cwd: session?.cwd ?? '', allowed: !shareToken && !socketFactory && session?.kind === 'agent' };
  }

  function activateLink(event: MouseEvent, link: TerminalLink): void {
    event.preventDefault();
    if (link.kind === 'url') {
      if (socketFactory) { void copyText(link.text).then(ok => { if (ok) toasts.success('Link copied'); }); return; }
      void openExternal(link.text);
    } else if (link.path) {
      const context = localFileContext();
      const path = context.allowed ? resolveTerminalFile(link.path, context.cwd) : null;
      if (path) openFile.open(path, link.line, link.col);
    }
  }

  function makeLinkProvider(): LinkProvider {
    return {
      provideLinks(row, callback) {
        const buffer = term?.buffer.active;
        if (!buffer) { callback(undefined); return; }
        const context = localFileContext();
        const hits = terminalLinksForRow(buffer, row, context.allowed);
        callback(hits.map(hit => ({
          text: hit.text,
          range: hit.range,
          decorations: { pointerCursor: true, underline: true },
          activate: (event: MouseEvent) => activateLink(event, hit),
          hover: () => { if (term?.element) term.element.title = hit.path ? (resolveTerminalFile(hit.path, context.cwd) ?? hit.path) : hit.text; },
          leave: () => { term?.element?.removeAttribute('title'); },
        })));
      },
    };
  }

  // ── Engine lifecycle: build / bind / park / adopt ─────────────────────────
  // The ENGINE is the xterm (+ fit/search addons), its socket and that
  // socket's flow state. A component instance binds its own handlers to the
  // engine it currently shows (bindTerm); a keepAlive Terminal parks the
  // engine on unmount / session switch (termPark.ts) instead of disposing
  // it, and adopts a parked engine for its session instead of building one.

  /** Undo for bindTerm(): its handlers close over THIS component instance,
   *  so they must not travel with a parked engine. */
  let termBindings: (() => void) | null = null;

  /** Build a fresh xterm into the container. */
  function buildTerm(): void {
    const t = new Terminal({
      fontFamily: untrack(() => ui.termFontStack),
      fontSize: untrack(() => effFontSize),
      // Bar cursor (not block): agent TUIs (claude/codex/grok) redraw the prompt
      // aggressively on ↑ history / multi-line edits. A block cursor is a full
      // cell paint that WebGL can leave behind as solid "trails" when the TUI
      // moves the cursor without a clean repaint. A thin bar matches modern
      // terminal hosts and doesn't leave cell-sized ghosts.
      cursorStyle: 'bar',
      cursorWidth: 2,
      cursorBlink: true,
      allowProposedApi: true,
      linkHandler: {
        allowNonHttpProtocols: true,
        activate(event, target) {
          const context = localFileContext();
          const link = oscTerminalLink(target, context.cwd, context.allowed);
          if (link) activateLink(event, link);
        },
      },
      // Keep fallback-font glyphs (e.g. Hebrew from Cousine) inside their grid
      // cell when their advance width differs from the primary font's cell.
      rescaleOverlappingGlyphs: true,
      // Exact grid metrics — non-1 lineHeight/letterSpacing makes cursor paint
      // and cell clears miss by a sub-pixel and leave residue between rows.
      lineHeight: 1.0,
      letterSpacing: 0,
      scrollback: untrack(() => scrollback),
      theme: untrack(() => terminalTheme(ui.theme, untrack(() => effScheme))),
      // ⌥ as Meta is a per-device setting (Settings → Appearance → Terminal);
      // default on. Live changes go through the effect below, no rebuild.
      macOptionIsMeta: untrack(() => ui.termOptionAsMeta),
      // Screen-reader mode: xterm keeps an aria-live mirror + accessible row
      // tree. Effect 1 also forces the DOM renderer while it is on.
      screenReaderMode: untrack(() => ui.termScreenReader),
      // ⌥-drag forces a LOCAL selection even while the running app has mouse
      // reporting on. Without this there is no way to select at all in a
      // mouse-reporting TUI (claude, codex, vim, htop…) on macOS: xterm's
      // mousedown handler hands the drag to the app and cancels it, and
      // `shouldForceSelection` is `altKey && macOptionClickForcesSelection`,
      // which is false by default. Selection silently never happened, so every
      // copy path — ⌘C, right-click ▸ Copy, copy-on-select — had nothing to
      // copy and Edit ▸ Copy showed up greyed out. Matches iTerm2 / Terminal.app,
      // where ⌥-drag is the established "select anyway" gesture.
      macOptionClickForcesSelection: true,
    });
    fit = new FitAddon();
    search = new SearchAddon();
    t.loadAddon(fit);
    t.loadAddon(search);
    t.open(container);
    term = t;
  }

  // Shift+Enter must insert a newline in the agent's composer, not submit.
  // xterm emits plain `\r` for Enter regardless of Shift, and `\r` is what
  // claude/codex read as "submit". Intercept Shift+Enter and send `\x1b\r`
  // (ESC+CR) instead — the same sequence Option/Meta+Enter produces (with
  // macOptionIsMeta on, the default), which these TUIs treat as a newline.
  // Plain Enter is left untouched, so it still submits.
  function termKeyHandler(e: KeyboardEvent): boolean {
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

    // ── Copy (⌘C / Ctrl+Shift+C) ──────────────────────────────────────────
    // xterm does NOT make ⌘C work on its own. It syncs the selection into its
    // hidden textarea in exactly one place — `rightClickHandler`, on
    // `contextmenu` — so a right-click → Copy works while a plain drag-select
    // + ⌘C does nothing: the renderer paints the selection, but the document
    // never gets a real DOM selection, so the browser's copy command stays
    // disabled and the `copy` listener xterm registers never fires. Handle
    // the chord ourselves against `getSelection()`, which is always correct.
    //
    // Ctrl+Shift+C is the terminal convention on Linux/Windows (bare Ctrl+C
    // must stay SIGINT). Both are only claimed when there IS a selection —
    // otherwise ⌘C/Ctrl+Shift+C fall through untouched.
    if (e.type === 'keydown' && !e.altKey && (e.key === 'c' || e.key === 'C')) {
      const chord = (e.metaKey && !e.ctrlKey) || (e.ctrlKey && e.shiftKey && !e.metaKey);
      if (chord && term?.hasSelection()) {
        const sel = term.getSelection();
        if (sel) {
          // Suppress xterm (never send ^C to the PTY) but do NOT
          // preventDefault: let the browser run its OWN copy command. The
          // selection mirror above has given the document a real selection,
          // so the native path works — and the `copy` listener below fills
          // the clipboard with the exact terminal text.
          //
          // This ordering is the whole point. Calling preventDefault here and
          // routing through `navigator.clipboard` REPLACES a permission-free
          // native copy with a permissioned one, which is strictly worse:
          // on an origin the browser de-privileges (the self-signed
          // `0.0.0.0` listener) the async API is refused and the copy dies,
          // even though the native command would have succeeded.
          // Re-mirror RIGHT HERE, synchronously, before the browser runs its
          // copy command. `onSelectionChange` normally does this, but if it
          // was missed for any reason the textarea would be empty and the
          // native copy would have nothing to take — which is precisely the
          // greyed-out Edit ▸ Copy symptom.
          const ta = term.textarea;
          if (ta && !composing && ta.value !== sel) {
            ta.value = sel;
            ta.select();
          }
          // Do NOT preventDefault: the browser's own copy command needs no
          // permission, and the mirrored textarea gives it something to copy.
          // (Deliberately NOT gated on `document.getSelection()` — Chromium
          // does not surface a textarea's internal selection there, so that
          // check reads empty exactly when the mirror is working and would
          // send us down the permissioned path the browser is refusing.)
          copySawEvent = false;
          e.stopPropagation();
          // Ctrl+Shift+C has NO native copy command behind it (on any OS —
          // the browser's own chord is ⌘C / Ctrl+C), so leaving it to the
          // browser means no `copy` event ever fires and the fallback below
          // runs 80 ms later, outside the gesture, on the permissioned API.
          // Run the permission-free command ourselves, synchronously, while
          // the keydown is still a user gesture; `onCopy` fills the clipboard
          // with the terminal selection and sets `copySawEvent`.
          if (e.ctrlKey && e.shiftKey && !e.metaKey) {
            try {
              document.execCommand('copy');
            } catch {
              /* refused — the async fallback below still gets its turn */
            }
          }
          // If the browser never fires `copy`, nothing was copied and the
          // async API is the only route left. Say so when that is refused
          // too — a silent failure is indistinguishable from a working copy
          // until you paste and get the PREVIOUS clipboard entry.
          setTimeout(() => {
            if (copySawEvent) return;
            void copyText(sel).then((ok) => {
              if (!ok) {
                toasts.error(
                  'Copy blocked',
                  'The browser refused the clipboard write. Right-click → Copy still works.',
                );
              }
            });
          }, 80);
          return false; // suppress xterm only — never ^C to the PTY
        }
      }
    }

    // ── Paste (Ctrl+Shift+V) ──────────────────────────────────────────────
    // ⌘V / Ctrl+V need nothing from us: the browser fires a `paste` event and
    // xterm's own handler reads `clipboardData` — which works on ANY origin,
    // secure or not. Ctrl+Shift+V has no native paste behind it, so it is the
    // one chord that must read the clipboard programmatically, and
    // `readText()` is the single clipboard call with no legacy fallback (it
    // needs a secure context AND a permission grant). Best-effort: on refusal
    // we stay silent rather than claim a paste that never happened, and ⌘V
    // still works.
    if (
      e.type === 'keydown' &&
      e.key.toLowerCase() === 'v' &&
      e.ctrlKey &&
      e.shiftKey &&
      !e.metaKey &&
      !e.altKey
    ) {
      e.preventDefault();
      e.stopPropagation();
      if (!readOnly) {
        navigator.clipboard
          ?.readText?.()
          .then((t) => {
            if (t) term?.paste(t); // xterm brackets it when the app asked for it
          })
          .catch(() => {/* no permission — ⌘V still works */});
      }
      return false;
    }
    return true;
  }

  function onTermData(data: string): void {
    if (readOnly) return;
    const user = !terminalReply(data);
    if (user) keyLat?.input(performance.now());
    sendJson({ type: 'input', data: textToBase64(data), user });
    if (user) resyncOnInput();
  }

  function onTermSelection(): void {
    if (!term) return;
    // The find bar drives the selection (each step selects its hit): leave
    // the textarea and the clipboard alone — the mirror's select() must not
    // compete with the find input, and copy-on-select would overwrite the
    // clipboard with every hit. closeFind re-mirrors.
    if (findOpen && document.activeElement !== term.textarea) return;
    const text = term.hasSelection() ? term.getSelection() : '';

    // Mirror the selection into xterm's hidden textarea and select it there.
    //
    // This is what actually makes ⌘C work, and it is not optional. xterm
    // paints its selection with the renderer; the DOCUMENT has no selection,
    // so every "copy" path that asks the document what to copy comes up
    // empty. In a plain browser that means the copy command stays disabled
    // and no `copy` event ever fires. In the Tauri app it is worse: the
    // native Edit ▸ Copy item (`apps/desktop/src-tauri/src/main.rs`,
    // `PredefinedMenuItem::copy`) owns the ⌘C key equivalent at the AppKit
    // level, so the keydown never reaches the page at all — WebKit runs
    // `copy:` against an empty DOM selection and the clipboard silently keeps
    // whatever it held before. Giving the textarea a real selection fixes
    // both: the command becomes enabled and operates on the right text.
    //
    // Safe to stuff the textarea: xterm's `_inputEvent` sends `e.data` (the
    // inserted text), never `textarea.value`, which is why xterm's own
    // `rightClickHandler` already does exactly this on contextmenu. Skipped
    // mid-IME-composition, where the textarea belongs to the input method.
    const ta = term.textarea;
    if (ta && !composing) {
      ta.value = text;
      if (text) ta.select();
    }

    // Copy-on-select: when enabled, any new selection goes straight to the
    // clipboard so the user never has to press ⌘C. Goes through `copyText`
    // (never throws, falls back to execCommand): the old
    // `navigator.clipboard.writeText(...).catch(...)` threw a SYNCHRONOUS
    // TypeError on any origin without the async API — the `.catch()` never
    // ran, and the throw landed inside xterm's selection-change emitter.
    if (ui.termCopyOnSelect && text) void copyText(text);
  
  }

  function onTermBinary(data: string): void {
    if (readOnly) return;
    // raw binary path (e.g. some IME flows) — bytes are latin1 in a string
    const bytes = new Uint8Array(data.length);
    for (let i = 0; i < data.length; i++) bytes[i] = data.charCodeAt(i) & 0xff;
    sendJson({ type: 'input', data: bytesToBase64(bytes) });
  }

  // The textarea is the IME's scratch space during composition; the
  // selection mirror above must not touch it while a composition is open.
  function onCompStart(): void {
    composing = true;
  }
  function onCompEnd(): void {
    composing = false;
  }

  function onTermFocus(): void {
    keyContext.terminalFocused = true;
    keyContext.openFind = openFind;
    // Claim PTY size authority: multiple viewers share one PTY (pane +
    // tiled-overview tile + phone tab) and last-resize-wins let a passive
    // small viewer pin the size (sessions stuck at 80 cols in a 150-col
    // pane). The server only honors resizes from the typing/focused owner;
    // clicking into a pane reclaims it, then re-push our grid.
    sendJson({ type: 'claim' });
    sendResize(true);
    // The pane the user works in renders on the GPU (L4): take a slot from
    // the least recently focused terminal when the budget is full.
    gpuClient.lastFocus = performance.now();
    flow.hold(false);
    onCompactWake();
    if (term && webglWanted && !webglAddon) {
      stealGpuSlotFor(gpuClient);
      retryWebglNow();
    }
  }
  function onTermBlur(): void {
    keyContext.terminalFocused = false;
    if (keyContext.openFind === openFind) keyContext.openFind = null;
  }

  /** Attach this instance's handlers to the current xterm. */
  function bindTerm(): void {
    const t = term;
    if (!t) return;
    // Clickable links — works identically with WebGL on or off (the link layer
    // is a DOM overlay above the renderer).
    const linkProvider = t.registerLinkProvider(makeLinkProvider());
    t.attachCustomKeyEventHandler(termKeyHandler);
    const subs = [t.onData(onTermData), t.onSelectionChange(onTermSelection), t.onBinary(onTermBinary)];
    if (keyLat) subs.push(t.onRender(() => keyLat?.rendered(performance.now())));
    // The find bar's "3/12" (fires only for decorated passes — 2+ chars).
    if (search) {
      subs.push(
        search.onDidChangeResults(({ resultIndex, resultCount }) => {
          localIdx = resultIndex;
          localCount = resultCount;
        }),
      );
    }
    const textarea = t.textarea;
    textarea?.addEventListener('compositionstart', onCompStart);
    textarea?.addEventListener('compositionend', onCompEnd);
    textarea?.addEventListener('focus', onTermFocus);
    textarea?.addEventListener('blur', onTermBlur);
    // Focused before these listeners existed (auto-focus on mount): record it
    // now, or ⌃-keys, ⌘F find and terminal zoom miss the focused terminal.
    if (textarea && document.activeElement === textarea) {
      keyContext.terminalFocused = true;
      keyContext.openFind = openFind;
    }
    termBindings = () => {
      linkProvider.dispose();
      t.attachCustomKeyEventHandler(() => true);
      for (const sub of subs) sub.dispose();
      textarea?.removeEventListener('compositionstart', onCompStart);
      textarea?.removeEventListener('compositionend', onCompEnd);
      textarea?.removeEventListener('focus', onTermFocus);
      textarea?.removeEventListener('blur', onTermBlur);
    };
  }

  function unbindTerm(): void {
    termBindings?.();
    termBindings = null;
  }

  /** Close the socket and dispose the xterm (the non-parking teardown). */
  function disposeEngine(): void {
    writes.dropQueued();
    closedByUs = true;
    sock?.close();
    sock = null;
    dropWebgl();
    term?.dispose();
    term = null;
    fit = null;
    search = null;
  }

  /** Worth parking: a settled, live, attached stream (its first snapshot
   *  applied). Anything else simply reconnects next time. */
  function canPark(): boolean {
    return !!term?.element && !!fit && !!search && !!sock && sock.readyState === WebSocket.OPEN
      && connectedSid !== null && snapshotEpoch !== null && exitCode === null;
  }

  /** Hand the engine to the parking lot (caller checked canPark and already
   *  ran unbindTerm). The xterm keeps parsing, detached, with no GPU
   *  context and at most the daemon's history depth. */
  function parkEngine(): void {
    const t = term!;
    const key = connectedSid!;
    dropWebgl();
    if ((t.options.scrollback ?? 0) > PARK_SCROLLBACK) t.options.scrollback = PARK_SCROLLBACK;
    // Park at the PTY's grid, not at whatever passing size the leaving layout
    // measured: a parked engine keeps parsing the TUI's cursor moves for
    // minutes, and at the wrong size they land on the wrong cells (G1).
    let needsCompact = localReflowed || compactDeferred;
    compactDeferred = false;
    // A widen whose grid was riding on a queued compact (N6) still owes the
    // PTY its size; the adopter compacts if the parked xterm needs it.
    if (compactGrid) needsCompact = true;
    flushCompactGrid();
    compactQueue.cancel(compactClient);
    // A parked engine keeps parsing what arrives but stops acknowledging it
    // (perf F9): the daemon sends at most one credit window, and a session
    // that overflowed meanwhile is caught up with ONE snapshot on adopt.
    flow.hold(true);
    if (lastCols > 0 && lastRows > 0 && (t.cols !== lastCols || t.rows !== lastRows)) {
      try {
        t.resize(lastCols, lastRows);
        needsCompact = true;
      } catch {
        /* disposed mid-park */
      }
    }
    localReflowed = false;
    onTermBlur();
    // Out of the document entirely: xterm's IntersectionObserver pauses the
    // renderer, nothing lays it out, and no page query (or e2e locator) can
    // find a second `.xterm` for the same pane.
    t.element?.remove();
    const e: ParkedEngine = {
      term: t,
      fit: fit!,
      search: search!,
      sock: sock!,
      flow,
      writes,
      compactPending,
      resyncPending,
      snapshotEpoch,
      status: null,
      exitCode,
      lastCols,
      lastRows,
      needsCompact: needsCompact && preferDom,
    };
    wireParked(key, e);
    termPark.put(key, e);
    term = null;
    fit = null;
    search = null;
    sock = null;
    connected = false;
    connectedSid = null;
    flow = new TermFlow(flowSink);
    writes = new WriteQueue(writeSink, flow, canSendNow);
  }

  /** Take over the engine parked for `sid`, if any: same socket, same flow
   *  stream, same buffer — nothing to replay. */
  function adoptEngine(sid: string): boolean {
    const e = termPark.take(sid);
    if (!e) return false;
    if (e.sock.readyState !== WebSocket.OPEN || !e.term.element) {
      disposeParked(e);
      return false;
    }
    term = e.term;
    fit = e.fit;
    search = e.search;
    sock = e.sock;
    flow = e.flow;
    writes = e.writes;
    flow.setSink(flowSink);
    writes.rebind(writeSink, canSendNow);
    // Report what was parsed while parked: the daemon sends what it held,
    // or ONE snapshot if the parked stream overflowed (perf F9).
    syncAckHold();
    wireSocket(e.sock);
    compactPending = e.compactPending;
    resyncPending = e.resyncPending;
    snapshotEpoch = e.snapshotEpoch;
    exitCode = e.exitCode;
    lastCols = e.lastCols;
    lastRows = e.lastRows;
    // Reflowed before parking: the forced sync in afterAdopt compacts.
    localReflowed = e.needsCompact;
    keyLat?.reset();
    connectedSid = sid;
    closedByUs = false;
    connected = true;
    disconnected = false;
    reconnecting = false;
    reconnectAttempts = 0;
    container.appendChild(e.term.element);
    // Host options may differ from the parker's (tile ↔ pane depth, theme
    // or font changed while parked).
    e.term.options.scrollback = scrollback;
    e.term.options.theme = terminalTheme(ui.theme, effScheme);
    if (e.term.options.fontFamily !== ui.termFontStack) e.term.options.fontFamily = ui.termFontStack;
    e.term.options.macOptionIsMeta = ui.termOptionAsMeta;
    e.term.options.screenReaderMode = ui.termScreenReader;
    if (e.status) onstatus?.(e.status);
    return true;
  }

  /** After adopting: take size authority like an attach would and sync the
   *  PTY to THIS host's box once it has laid out. No snapshot request. */
  function afterAdopt(): void {
    if (claimOnAttach && !readOnly) sendJson({ type: 'claim' });
    requestAnimationFrame(() => {
      if (!term || !connected) return;
      // The DOM renderer xterm fell back to when the WebGL addon was dropped
      // for parking may have painted a frame while detached, caching glyph
      // widths measured as 0 (offsetWidth of a detached node). When DOM is
      // still the renderer here, make it re-measure (clears its width cache).
      if (!webglAddon) {
        try {
          (term as unknown as { _core?: { _renderService?: { handleCharSizeChanged?: () => void } } })
            ._core?._renderService?.handleCharSizeChanged?.();
        } catch {
          /* private API moved — worst case is uneven glyph spacing until a font change */
        }
      }
      // A TUI pane keeps the PTY's grid until this host's box confirms
      // (settle-then-apply); a reflow on the way compacts from a snapshot.
      safeFit(!deferLocalResize());
      sendResize(true);
      verifyFitSoon();
      forceViewportRefresh();
    });
  }

  /** Session switch in a keepAlive Terminal: park the old session's engine
   *  (or close it) and adopt / build one for `sid`. The new session never
   *  replays into the old one's buffer, and switching back is instant. */
  function switchEngine(sid: string): void {
    const hadFocus = !!term?.textarea && document.activeElement === term.textarea;
    // The find bar's hits (and its ring-buffer list) belong to the old session.
    resetFind();
    unbindTerm();
    tuiCleanup.cancel();
    if (tuiRefreshRaf !== null) {
      cancelAnimationFrame(tuiRefreshRaf);
      tuiRefreshRaf = null;
    }
    if (canPark()) parkEngine();
    else disposeEngine();
    connected = false;
    disconnected = false;
    reconnecting = false;
    exitCode = null;
    dormantView = false;
    reconnectAttempts = 0;
    lastInjN = 0;
    const adopted = adoptEngine(sid);
    // An adopted socket is already OPEN: without this the injection effect
    // below would re-send an injection that was consumed before the park.
    if (adopted) lastInjN = ws.injections[sid]?.n ?? 0;
    if (!adopted) buildTerm();
    bindTerm();
    attachWebgl();
    if (adopted) afterAdopt();
    else connect();
    if (hadFocus) term?.focus();
  }

  // ── Effect 1: xterm init (re-runs when renderer mode flips)
  // This effect owns the engine for the component's lifetime: it builds (or,
  // with keepAlive, adopts a parked) xterm, binds handlers, the
  // ResizeObserver and the initial WS connection. It does NOT watch
  // `sessionId` — Effect 2 below handles switches. Renderer mode (RTL /
  // phone) IS tracked so an RTL toggle or a phone↔desktop layout flip
  // rebuilds with the right backend (a keepAlive terminal parks and
  // re-adopts itself, keeping its buffer).
  $effect(() => {
    // Tracked reads — the ONLY ones: toggling RTL / phone layout re-runs this
    // effect so the terminal is rebuilt with the correct renderer (WebGL vs DOM).
    const rtl = ui.rtlBidi;
    // Screen-reader support needs the DOM renderer: a WebGL canvas exposes no
    // text to assistive tech, so toggling it rebuilds like RTL does.
    const wantDom = viewport.isPhone || FORCE_DOM_RENDERER || ui.termScreenReader;
    // Everything else is untracked. Loading the WebGL addon (and the xterm
    // callbacks it fires synchronously) reads component state such as
    // `connected`; tracked, the socket opening re-ran this effect, whose
    // cleanup parks the engine (connected = false) and whose re-run adopts it
    // back (connected = true) — an endless park/adopt loop minting a WebGL
    // context per turn until Svelte aborted it (effect_update_depth_exceeded,
    // then a fatal reload; only where WebGL loads, e.g. WebKit).
    return untrack(() => mountEngine(rtl, wantDom));
  });

  /** Effect 1's body, run untracked (see above). Returns its teardown. */
  function mountEngine(rtl: boolean, wantDom: boolean): () => void {
    // Decided once per mount: the host's intent and transport are fixed for
    // a Terminal's lifetime (props may already read as torn down in cleanup).
    const parkable = untrack(() => keepAlive && !shareToken && !socketFactory);
    const adopted = parkable && untrack(() => adoptEngine(sessionId));
    // Same for a mount-time adopt (see switchEngine): injections issued
    // before this mount were never meant for it.
    if (adopted) lastInjN = untrack(() => ws.injections[sessionId]?.n ?? 0);
    if (!adopted) untrack(buildTerm);
    untrack(bindTerm);
    // Edit ▸ Select All (⌘A) while this terminal has focus selects the whole
    // BUFFER. xterm renders only the visible rows, so a DOM-level select-all
    // would grab just what is on screen.
    const unregisterSelectAll = registerSelectAll(container, () => {
      term?.selectAll();
      return true;
    });
    // Mount-time focus (untracked: autoFocus/readOnly must not re-run this
    // effect — a rebuild here tears down the whole GPU canvas + WS).
    if (untrack(() => autoFocus && !readOnly) && !viewport.isPhone) term?.focus();
    // ── Renderer selection: WebGL (GPU) on desktop, DOM as the fallback ──────
    // xterm draws to a WebGL canvas when WebglAddon is loaded; with no addon it
    // falls back to its DOM renderer (per-cell <span>s in `.xterm-rows`). The DOM
    // renderer is ROBUST (no GPU context to lose) but in WebKit every repaint
    // rebuilds row spans and pays style + layout + paint on the main thread:
    // measured ~⅓ of a core per WORKING agent pane even at 2–5 KB/s (r3-12-01),
    // so 3–4 busy panes saturated the webview. Agent panes therefore use WebGL
    // too; their ghost clean-up (tuiCleanup) is a cheap GPU redraw there.
    // Context loss falls back to DOM and retries (attachWebgl), and at most
    // MAX_WEBGL_TERMINALS per window render on the GPU.
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
    //   • `localStorage['otto.term.renderer'] = 'dom'` (FORCE_DOM_RENDERER).
    webglRetries = 0;
    webglWanted = !rtl && !wantDom;
    gpuClients.add(gpuClient);
    attachWebgl();
    // GPU recovery (L4): the window regaining focus, the page becoming
    // visible again (wake from sleep, Space switch) or the pane scrolling
    // back into view restart the WebGL backoff right away.
    const onGpuWake = (): void => {
      if (document.visibilityState === 'visible') retryWebglNow();
    };
    const onPageVis = (): void => {
      syncAckHold();
      if (document.visibilityState === 'visible') onCompactWake();
    };
    document.addEventListener('visibilitychange', onPageVis);
    document.addEventListener('visibilitychange', onGpuWake);
    window.addEventListener('focus', onGpuWake);
    const gpuIo = typeof IntersectionObserver === 'function'
      ? new IntersectionObserver((entries) => {
          onScreen = entries[entries.length - 1].isIntersecting;
          if (onScreen) {
            retryWebglNow();
            onCompactWake();
          }
        })
      : null;
    gpuIo?.observe(container);
    // Latency HUD: one probe + one HUD refresh a second while enabled.
    const releaseLoop = latencyOn ? loopMonitor.acquire() : null;
    const hudTimer = latencyOn
      ? setInterval(() => { // ui-guards: allow — diagnostics clock; must run while hidden
          if (connected && probes) sendJson({ type: 'probe', id: probes.next(performance.now()) } satisfies WsTermProbeFrame);
          refreshHud();
        }, 1000)
      : null;
    // NOTE: do NOT fit() here. The container has no real size yet on first open
    // (grid/flex layout isn't resolved this tick). The ResizeObserver below
    // fires once the pane gets its real box and performs the first valid fit,
    // and connect() is deferred until then so the PTY is sized correctly.

    // Whatever triggers the copy — the page's own ⌘C, the browser's
    // right-click ▸ Copy, or the Tauri app's native Edit ▸ Copy — the bytes
    // must be the TERMINAL's selection, not whatever the mirrored textarea
    // happens to hold (it can lag a redraw, and xterm trims trailing
    // whitespace differently). Capture phase so this wins over xterm's own
    // `copy` listener, which reads the same selection but only fires when it
    // already believes there is one.
    const onCopy = (e: ClipboardEvent) => {
      const sel = term?.hasSelection() ? term.getSelection() : '';
      if (!sel) return; // nothing selected in the terminal — let the page be
      copySawEvent = true; // the native command ran; no need for the async API
      e.clipboardData?.setData('text/plain', sel);
      e.preventDefault();
    };
    container.addEventListener('copy', onCopy, true);

    // ── Image paste ───────────────────────────────────────────────────────────
    // Agent CLIs take an image as a FILE PATH, and the path has to exist on the
    // machine the CLI runs on — which is the daemon's machine, not necessarily
    // the browser's. So a pasted image is uploaded to the daemon first and the
    // stored path is injected as text. Runs in the CAPTURE phase so we get the
    // event before xterm's own paste handler (which is text/plain only and
    // would otherwise swallow the gesture as an empty paste).
    //
    // Text pastes are left entirely alone — xterm handles those correctly on
    // every origin, since `clipboardData` on a real paste event needs neither a
    // secure context nor a permission grant.
    const onPaste = (e: ClipboardEvent) => {
      if (readOnly) return;
      const items = Array.from(e.clipboardData?.items ?? []);
      const img = items.find((it) => it.kind === 'file' && it.type.startsWith('image/'));
      if (!img) return; // not an image paste — let xterm do its thing
      const file = img.getAsFile();
      if (!file) return;
      e.preventDefault();
      e.stopPropagation();
      if (socketFactory) { toasts.info('Room terminals accept text only'); return; }
      void uploadPastedImage(file);
    };
    container.addEventListener('paste', onPaste, true);

    // The WS is connected lazily on the first *valid* fit so the very first
    // sendResize(true) in sock.onopen ships a correct grid (covers first open).
    // An adopted engine is already connected and sized once.
    let didFirstFit = adopted;
    let refitTimer: ReturnType<typeof setTimeout> | null = null;
    const refit = () => {
      const ok = safeFit(!deferLocalResize());
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
    if (adopted) {
      termDidInit = true;
      untrack(afterAdopt);
    } else {
      requestAnimationFrame(() => requestAnimationFrame(refit));
    }

    // Perf-spec probe (e2e/desktop-terminal-flood-perf.spec.ts): opt-in by a
    // `window.__ottoTermProbe` array the spec installs — one property check
    // per mount otherwise. Exposes the flow backlog, scrollback depth, and a
    // renderer-agnostic view of the screen: on WebGL (3027df89) the text is
    // drawn on a canvas and there is no `.xterm-rows` DOM to read.
    const probeList = (window as unknown as { __ottoTermProbe?: unknown[] }).__ottoTermProbe;
    const probe = Array.isArray(probeList)
      ? {
          sessionId: () => sessionId,
          pending: () => flow.pending,
          queued: () => writes.queued,
          scrollback: () => term?.options.scrollback ?? 0,
          renderer: () => (webglAddon ? 'webgl' : 'dom'),
          /** The rows currently in the viewport (parsed buffer), as text. */
          text: () => {
            const b = term?.buffer.active;
            if (!term || !b) return '';
            const rows: string[] = [];
            for (let y = b.viewportY; y < b.viewportY + term.rows; y++) rows.push(b.getLine(y)?.translateToString(true) ?? '');
            return rows.join('\n');
          },
          /** Called after each renderer pass (DOM or WebGL) — "painted". */
          onRender: (cb: () => void) => term?.onRender(cb),
          disposed: false,
        }
      : null;
    if (probe) probeList!.push(probe);

    return () => {
      if (probe) probe.disposed = true;
      ro.disconnect();
      for (const t of verifyTimers) clearTimeout(t);
      if (refitTimer) clearTimeout(refitTimer);
      cancelResizeTimer();
      resizeForcePending = false;
      if (resizeCompactTimer !== null) {
        clearTimeout(resizeCompactTimer);
        resizeCompactTimer = null;
      }
      if (reconnectTimer) {
        clearTimeout(reconnectTimer);
        reconnectTimer = null;
      }
      if (tuiRefreshRaf !== null) {
        cancelAnimationFrame(tuiRefreshRaf);
        tuiRefreshRaf = null;
      }
      tuiCleanup.cancel();
      cancelWebglRetry();
      gpuClients.delete(gpuClient);
      document.removeEventListener('visibilitychange', onGpuWake);
      document.removeEventListener('visibilitychange', onPageVis);
      compactQueue.cancel(compactClient);
      window.removeEventListener('focus', onGpuWake);
      gpuIo?.disconnect();
      if (hudTimer !== null) clearInterval(hudTimer);
      releaseLoop?.();
      if (localFindTimer !== null) {
        clearTimeout(localFindTimer);
        localFindTimer = null;
      }
      termDidInit = false;
      unregisterSelectAll();
      container.removeEventListener('paste', onPaste, true);
      container.removeEventListener('copy', onCopy, true);
      // Search decorations live on the engine: a parked one must not come
      // back highlighted under a closed find bar.
      resetFind();
      unbindTerm();
      onTermBlur();
      if (parkable && canPark()) {
        closedByUs = true;
        parkEngine();
      } else {
        disposeEngine();
      }
    };
  }

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
    // No-op when the value is UNCHANGED. Svelte re-runs this effect whenever the
    // `sessionId` prop source updates — and a churning parent (review panel) passes
    // a new agent object with the same session_id on every render. Without this
    // guard each such re-run did a full close+reconnect, storming the WS. A real
    // session switch still falls through (connectedSid differs).
    if (sessionId === connectedSid) return;
    // The new session's restart nonce is not a restart of THIS view: the
    // switch below attaches to its live process anyway.
    seenRestartNonce = untrack(() => restartNonce);
    // 1. Cancel timers that belong to the old session (reconnect + pending
    //    trailing resize — a stale send would push the old pane's grid at the
    //    new session's PTY).
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    cancelResizeTimer();
    resizeForcePending = false;
    if (resizeCompactTimer !== null) {
      clearTimeout(resizeCompactTimer);
      resizeCompactTimer = null;
    }
    // keepAlive: park the old session's engine and adopt/build the new one's
    // (untracked — only `sessionId` drives this effect).
    if (untrack(() => keepAlive && !shareToken && !socketFactory)) {
      const sid = sessionId;
      untrack(() => switchEngine(sid));
      return;
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
      connect({ view: true });
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
      clearWebglAtlas();
      if (safeFit()) sendResize();
      forceViewportRefresh();
    }
  });

  // react to a host changing the scrollback depth (tile ↔ primary pane)
  $effect(() => {
    const lines = scrollback;
    if (term && term.options.scrollback !== lines) term.options.scrollback = lines;
  });

  // react to the ⌥-as-Meta / screen-reader settings (live options; the
  // renderer switch for screen-reader mode is Effect 1's rebuild)
  $effect(() => {
    const meta = ui.termOptionAsMeta;
    const sr = ui.termScreenReader;
    if (!term) return;
    if (term.options.macOptionIsMeta !== meta) term.options.macOptionIsMeta = meta;
    if (term.options.screenReaderMode !== sr) term.options.screenReaderMode = sr;
  });

  // react to terminal font-family choice (live, no rebuild needed)
  $effect(() => {
    const family = ui.termFontStack;
    if (term && term.options.fontFamily !== family) {
      term.options.fontFamily = family;
      clearWebglAtlas();
      if (safeFit()) sendResize();
      forceViewportRefresh();
    }
  });

  // A web font finishing its load AFTER the terminal measured its cell with a
  // fallback face leaves wrong metrics and fallback glyphs in the WebGL atlas.
  // Re-measure, drop the atlas, refit and repaint — only on a real font load.
  $effect(() => {
    const fonts = typeof document !== 'undefined' ? document.fonts : undefined;
    if (!fonts?.addEventListener) return;
    const onLoaded = (): void => {
      if (!term) return;
      try {
        (term as unknown as { _core?: { _charSizeService?: { measure?: () => void } } })._core?._charSizeService?.measure?.();
      } catch {
        /* private API moved — the refresh below still helps */
      }
      clearWebglAtlas();
      if (safeFit()) sendResize();
      forceViewportRefresh();
    };
    fonts.addEventListener('loadingdone', onLoaded);
    return () => fonts.removeEventListener('loadingdone', onLoaded);
  });

  // React to a parent restart: the session was respawned/resumed server-side, so
  // drop the exited overlay and reconnect to the now-live PTY. Only a nonce that
  // CHANGES after mount counts: a session restarted at some earlier point keeps
  // a non-zero nonce, and reconnecting on every mount would re-snapshot (and
  // throw away an adopted, already-live engine). The connect() is untracked so
  // this effect only re-runs on a real restart, not when sessionId churns
  // (Effect 2 owns that).
  let seenRestartNonce = untrack(() => restartNonce);
  $effect(() => {
    const n = restartNonce;
    if (!n || n === seenRestartNonce || !term) return;
    seenRestartNonce = n;
    untrack(() => {
      exitCode = null;
      disconnected = false;
      connect();
    });
  });

  // react to theme + light/dark scheme switches (respects forceDark override)
  $effect(() => {
    const theme = terminalTheme(ui.theme, effScheme);
    if (term) {
      term.options.theme = theme;
      // Palette change invalidates WebGL atlas tiles that baked the old colors.
      clearWebglAtlas();
      forceViewportRefresh();
    }
  });

  // Apply programmatic input injected into this session (e.g. DB rows → a running
  // agent), wrapped in bracketed paste so multi-line content isn't auto-submitted.
  let lastInjN = 0;
  $effect(() => {
    const inj = ws.injections[sessionId];
    if (!inj || readOnly || socketFactory || inj.n <= lastInjN) return;
    lastInjN = inj.n;
    sendJson({ type: 'input', data: textToBase64(`\x1b[200~${inj.text}\x1b[201~`) });
  });

  export function focus(): void {
    term?.focus();
  }

  /** "Redraw terminal" (pane ⋯ menu, ⌘K): sync the grid and rebuild the
   *  screen from a fresh server snapshot — the rebuild a reconnect or Reset
   *  does, without dropping the socket. One-click recovery for a garbled TUI. */
  export function redraw(): void {
    if (!term || !connected) return;
    cancelResizeTimer();
    resizeForcePending = false;
    if (safeFit() && (term.cols !== lastCols || term.rows !== lastRows)) {
      lastCols = term.cols;
      lastRows = term.rows;
      sendJson({ type: 'resize', cols: lastCols, rows: lastRows });
    }
    localReflowed = false;
    if (resizeCompactTimer !== null) {
      clearTimeout(resizeCompactTimer);
      resizeCompactTimer = null;
    }
    // Not a guarded compact: the user asked, so it applies even while
    // scrolled up (snapshotApplies only guards compactPending replies).
    compactPending = false;
    sendJson({ type: 'scrollback', lines: term.options.scrollback ?? scrollback });
  }
</script>

<!-- term-outer wraps the terminal canvas + the phone-only key bar below it.
     On desktop this is just a transparent flex pass-through; on phone it stacks
     the key bar underneath the canvas so the bar doesn't overlap the scrollback. -->
<div class="term-outer" class:phone={viewport.isPhone}>
  <div class="term-wrap" class:otto-force-dark={forceDark}>
    {#if findOpen}
      <div class="find-bar" role="search" aria-label="Find in terminal">
        <input dir="auto"
          bind:this={findInput}
          bind:value={findQuery}
          placeholder="Find in terminal"
          aria-label="Find in terminal"
          aria-controls={serverMatches.length > 0 ? `${findId}-results` : undefined}
          aria-activedescendant={serverMatches.length > 0 && serverMatchIdx >= 0 ? `${findId}-match-${serverMatchIdx}` : undefined}
          onfocus={onFindFocus}
          onblur={onFindBlur}
          oninput={() => {
            // Local search: xterm SearchAddon (debounced, client buffer).
            scheduleLocalFind();
            // Server search: ring-buffer grep (debounced, full scrollback history).
            scheduleServerSearch(findQuery);
          }}
          onkeydown={onFindKey}
        />
        <!-- Local match position (client buffer), then the scrollback count
             (spinner while the ring-buffer search is in flight). -->
        {#if findQuery && localCount > 0}
          <span class="find-status" aria-live="polite" title="{localCount}{localIdx < 0 ? '+' : ''} {localCount === 1 ? 'match' : 'matches'} on screen and in the loaded scrollback">
            {localIdx >= 0 ? `${localIdx + 1}/${localCount}` : `${localCount}+`}
          </span>
        {:else if findQuery && localMiss && !serverSearchPending && serverMatches.length === 0}
          <span class="find-status" aria-live="polite">No results</span>
        {/if}
        {#if serverSearchPending}
          <span class="find-status" title="Searching scrollback…">…</span>
        {:else if serverMatches.length > 0}
          <span class="find-status server" title="{plural(serverMatches.length, 'scrollback match', 'scrollback matches')} (↑↓ to step)">
            <Icon name="clock" size={12} />{serverMatchIdx >= 0 ? serverMatchIdx + 1 : '–'}/{serverMatches.length}
          </span>
        {/if}
        <button class="icon-btn" onclick={() => findNext(false)} title="Older match" aria-keyshortcuts="Enter" aria-label="Older match">
          <Icon name="chevronUp" size={12} />
        </button>
        <button class="icon-btn" onclick={() => findNext(true)} title="Newer match" aria-keyshortcuts="Shift+Enter" aria-label="Newer match">
          <Icon name="chevronDown" size={12} />
        </button>
        <button class="icon-btn" onclick={closeFind} title="Close find" aria-label="Close find" aria-keyshortcuts="Escape">
          <Icon name="x" size={12} />
        </button>
      </div>
      {#if serverMatches.length > 0}
        <!-- Server ring-buffer match list (≤200, the list scrolls). Clicking a
             row — or ↑↓ in the input — jumps the viewport to that line. -->
        <div class="find-results" id="{findId}-results" role="listbox" aria-label="Scrollback search results" bind:this={findResultsEl}>
          {#each serverMatches as m, i (i)}
            <!-- Picked on press (focus stays in the find input; ↑/↓ there step). -->
            <div
              id="{findId}-match-{i}"
              class="find-result-row"
              class:active={i === serverMatchIdx}
              role="option"
              aria-selected={i === serverMatchIdx}
              tabindex="-1"
              data-match={i}
              onmousedown={(e) => { e.preventDefault(); goToServerMatch(i); }}
            >
              <span class="find-result-line">{m.line + 1}</span>
              <span class="find-result-text">{m.text}</span>
            </div>
          {/each}
        </div>
      {/if}
    {/if}

    <!-- Desktop terminal toolbar: font zoom + copy-on-select toggle. Visible on
         desktop when ui.termToolbar is on; phone controls live in phone-controls
         below (unchanged). The toolbar sits flush bottom-left so it doesn't
         overlap the find-bar (top-right) or the overlay badges (also top-right). -->
    {#if hud}
      <!-- Latency HUD (opt-in diagnostics, termLatency.ts): never announced. -->
      <div class="lat-hud" aria-live="off" role="group" aria-label="Terminal latency diagnostics">
        {#each hud as row (row.k)}
          <span class="lat-k" title={row.title}>{row.k}</span><span class="lat-v">{row.v}</span>
        {/each}
      </div>
    {/if}
    {#if !viewport.isPhone && ui.termToolbar && showToolbar}
      <div class="desk-toolbar" role="toolbar" aria-label="Terminal controls">
        <button
          class="icon-btn"
          onclick={() => ui.termZoomOut()}
          title="Zoom out" aria-keyshortcuts="Meta+-"
          aria-label="Zoom out"
        ><Icon name="minus" size={12} /></button>
        <button class="btn small ghost tb-size" onclick={() => ui.termZoomReset()}
          title="Reset terminal zoom" aria-keyshortcuts="Meta+0" aria-label="Reset terminal zoom">{fittedFontSize}px</button>
        <button
          class="icon-btn"
          onclick={() => ui.termZoomIn()}
          title="Zoom in" aria-keyshortcuts="Meta+="
          aria-label="Zoom in"
        ><Icon name="plus" size={12} /></button>
        <span class="tb-sep" aria-hidden="true"></span>
        <button
          class="btn small ghost tb-copy"
          class:tb-active={ui.termCopyOnSelect}
          onclick={() => ui.setTermCopyOnSelect(!ui.termCopyOnSelect)}
          title={ui.termCopyOnSelect ? 'Copy-on-select: on — click to disable' : 'Copy-on-select: off — click to enable'}
          aria-pressed={ui.termCopyOnSelect}
          aria-label="Copy on select"
        >Copy</button>
      </div>
    {/if}

    <!-- Task 5.3: touch-scroll — pointer events drive term.scrollLines on phone.
         onpointerdown/move/up are no-ops on desktop (we check viewport.isPhone
         + e.pointerType inside the handlers). Desktop mouse-wheel uses xterm's
         own built-in scroll handler which is completely untouched. -->
    <!-- A narrow desktop grid can be scrolled horizontally with the keyboard
         from this region; terminal input retains its own xterm focus target. -->
    <div
      class="term-host"
      class:ro={readOnly}
      class:force-dark={forceDark}
      class:rtl-bidi={ui.rtlBidi}
      bind:this={container}
      role="region"
      aria-label="Terminal viewport"
      use:scrollFocus={horizontalOverflow}
      onpointerdown={onTouchPointerDown}
      onpointermove={onTouchPointerMove}
      onpointerup={onTouchPointerUp}
      onpointercancel={onTouchPointerUp}
    ></div>

    {#if exitCode !== null}
      <!-- Shared exit vocabulary (lib/status.ts): "Ended", "Suspended —
           resumes on open", or "Failed (exit N)" — never a bare "exited (0)". -->
      <!-- A dormant view (a suspended session, incl. a shell after a daemon
           restart) wakes on typing or Resume whatever its provider. -->
      {@const ex = exitState(exitCode, resumable || dormantView)}
      {@const hint = dormantView && ex.key === 'suspended' ? (readOnly ? 'Suspended' : 'Suspended — type or Resume to continue') : ex.hint}
      <div class="term-overlay">
        <span class="ov-status {ex.tone}" data-exit={ex.key} data-dormant={dormantView || undefined} title={hint}>{ex.key === 'suspended' ? hint : ex.label}</span>
        {#if (restartable || resumable || dormantView) && !readOnly}
          <button
            class="btn"
            onclick={() => {
              if (onrestart) onrestart();
              else { exitCode = null; connect({ view: false }); }
            }}
            title={resumable || dormantView ? 'Resume the session where it left off' : onrestart ? 'Start the session again in this pane' : 'Reconnect to the session'}
          >{resumable || dormantView ? 'Resume' : onrestart ? 'Restart session' : 'Reconnect'}</button>
        {/if}
      </div>
    {:else if reconnecting}
      <div class="term-overlay dim">
        <span class="ov-status">Reconnecting…</span>
        <button class="btn" onclick={() => { reconnectAttempts = 0; connect({ view: false }); }}>Reconnect now</button>
      </div>
    {:else if disconnected}
      <div class="term-overlay">
        <span class="ov-status danger">Disconnected</span>
        <button class="btn" onclick={() => connect({ view: false })}>Reconnect</button>
      </div>
    {:else if !connected}
      <div class="term-overlay dim"><span class="ov-status">Connecting…</span></div>
    {/if}

    {#if readOnly}
      <!-- An in-flow strip ABOVE the output (the host is inset below it), so the
           notice never sits on top of terminal text. -->
      <div class="ro-strip" role="note">
        <Icon name="lock" size={12} />
        <span class="ro-label">Read-only</span>
        <span class="ro-why" title={readOnlyReason ?? "Your viewer role can watch this session but not type in it."}>{readOnlyReason ?? 'Your viewer role can watch this session but not type in it.'}</span>
      </div>
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
          title="Zoom out terminal"
        ><Icon name="minus" size={14} /></button>
        <button
          class="phone-btn"
          onclick={() => ui.termZoomIn()}
          aria-label="Zoom in terminal"
          title="Zoom in terminal"
        ><Icon name="plus" size={14} /></button>
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

  /* Containment (r3-01-01 / r3-12-01): a terminal repaint used to dirty
     layout all the way up to the page root, and every frame re-ran the
     ancestor grids' track sizing. `content` (layout + paint + style) makes
     this box a layout/paint boundary without size containment — its size
     still comes from the parent (100 %), so nothing collapses. It already
     clipped (overflow: hidden), and nothing inside is position: fixed, so
     layout containment changes no geometry. */
  .term-wrap {
    position: relative;
    width: 100%;
    height: 100%;
    background: var(--term-bg);
    overflow: hidden;
    contain: content;
  }
  /* Force dark: the wrapper is a `.otto-force-dark` island (tokens.css), so
     in a light scheme it re-declares the dark token set — --term-bg, the
     surfaces, borders and text — and the find bar, overlays, badges and
     toolbar inside read as one dark widget with the canvas. */
  /* The xterm host gets full `strict` containment: its box is fixed by the
     insets (absolutely positioned), never by its content, so size
     containment is free — xterm's row rebuilds and canvas resizes stay
     inside it and never reach the page. No contain-intrinsic-size needed:
     an inset-sized abspos box has no content-based size to replace. The
     find bar, overlays and toolbar are siblings, not children, so the
     paint clip can't cut them off. */
  .term-host {
    position: absolute;
    /* Logical insets resolve against this box's own `direction: ltr`, so the
       8 px gutter stays on the left (the terminal grid is always LTR). */
    inset-block: 6px 4px;
    inset-inline: 8px 0;
    overflow-x: auto;
    overflow-y: hidden;
    direction: ltr;
    contain: strict;
  }
  .term-host.force-dark {
    background: var(--term-bg);
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
    box-shadow: var(--glass-shadow);
    max-width: calc(100% - 32px);
  }
  .find-bar:focus-within {
    border-color: var(--accent-text);
    box-shadow: var(--glass-shadow), 0 0 0 3px var(--accent-soft-strong);
  }
  .find-bar input {
    width: 180px;
    min-width: 60px;
    flex: 0 1 auto;
    border: none;
    background: transparent;
    font-size: var(--fs-s);
    color: var(--text);
    outline: none;
  }
  /* Scrollback match count / spinner badge next to the input */
  .find-status {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
    user-select: none;
    padding: 0 2px;
  }
  .find-status.server {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  /* Dropdown list of server ring-buffer matches */
  .find-results {
    position: absolute;
    top: calc(8px + 30px + 2px);
    inset-inline-end: 16px;
    z-index: 5;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--glass-shadow);
    width: 340px;
    max-width: calc(100% - 32px);
    max-height: 200px;
    overflow-y: auto;
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
  }
  .find-result-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    padding: 2px 8px;
    cursor: pointer;
    color: var(--text);
  }
  .find-result-row:hover {
    background: var(--hover);
  }
  .find-result-row.active {
    background: var(--accent-soft);
    color: var(--text);
  }
  .find-result-line {
    color: var(--text-dim);
    min-width: 36px;
    text-align: end;
    flex-shrink: 0;
    font-size: var(--fs-xs);
  }
  .find-result-text {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  /* Small unobtrusive chip in the top-right — never covers the input line. */
  .term-overlay {
    position: absolute;
    top: 6px;
    inset-inline-end: 8px;
    display: flex;
    align-items: center;
    gap: 6px;
    padding-block: 2px;
    padding-inline: 8px 4px;
    z-index: 6;
    background: color-mix(in srgb, var(--surface) 88%, transparent);
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: var(--fs-xs);
    opacity: 0.9;
  }
  .term-overlay.dim {
    opacity: 0.7;
  }
  /* The capsule is the overlay itself; its status word is plain text in the
     tone colour (Ended / Suspended / Failed / Disconnected). */
  .ov-status {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ov-status.danger {
    color: var(--danger);
  }
  .ov-status.warning {
    color: var(--warning);
  }
  .ov-status.success {
    color: var(--success);
  }
  .term-overlay .btn {
    padding: 1px 8px;
    font-size: var(--fs-xs);
  }
  /* Read-only notice: a slim strip across the top; the host starts below it. */
  .term-host.ro {
    top: 30px;
  }
  .ro-strip {
    position: absolute;
    top: 0;
    inset-inline: 0;
    height: 24px;
    z-index: 4;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 10px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    min-width: 0;
  }
  .ro-label {
    font-weight: 600;
    color: var(--text);
    flex-shrink: 0;
  }
  .ro-why {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
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
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    /* Opaque: no glass over the terminal (foundations §7). */
    background: var(--surface);
    color: var(--text);
    font-size: var(--fs-xl);
    cursor: pointer;
    touch-action: manipulation;
    -webkit-tap-highlight-color: transparent;
    transition: background var(--dur-fast);
  }
  .phone-btn:active {
    background: var(--accent-solid);
    color: var(--accent-contrast);
  }
  .phone-btn.active {
    background: var(--accent-solid);
    color: var(--accent-contrast);
    border-color: var(--accent-solid);
  }

  /* ── Latency HUD (opt-in, termLatency.ts) — top-start corner, clear of the
     find bar (top-end) and the toolbar (bottom-start). Click-through. */
  .lat-hud {
    position: absolute;
    top: 6px;
    inset-inline-start: 8px;
    z-index: 6;
    display: grid;
    grid-template-columns: auto auto;
    column-gap: 8px;
    padding: 4px 8px;
    max-width: calc(100% - 16px);
    background: color-mix(in srgb, var(--surface) 88%, transparent);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.35;
    pointer-events: none;
  }
  .lat-k {
    color: var(--text-dim);
    pointer-events: auto;
  }
  .lat-v {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
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
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    opacity: 0.7;
    transition: opacity var(--dur-enter) ease-out;
  }
  .desk-toolbar:hover {
    opacity: 1;
  }
  /* Shared .icon-btn / .btn.small.ghost; only the active copy toggle and the
     size readout differ. */
  .tb-copy.tb-active {
    color: var(--accent-text);
  }
  .tb-size {
    min-width: 28px;
    color: var(--text-dim);
    user-select: none;
  }
  .tb-sep {
    width: 1px;
    height: 12px;
    background: var(--border);
    margin: 0 2px;
  }
</style>
