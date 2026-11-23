<script lang="ts">
  // Live Simulator / Emulator Mirror & Control Stage inside Otto Browser.
  // Streams frames over `WS /ws/simulators/{id}/stream` (binary envelope via
  // `decodeSimFrame`), routes capability-gated tap/swipe/text/key/rotate/
  // appearance/open-URL actions, and offers a collapsible live log tail.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import { downloadBlobUrl } from '../../../lib/downloadBlob';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import { simulatorsApi } from '../../../lib/api/simulators';
  import { openSimFrameStream, type SimFrameStreamHandle } from '../../../lib/simulators/simFrameStream';
  import type {
    SimAppearance,
    SimDevice,
    SimLogServerMsg,
    SimOrientation,
    SimStreamClientMsg,
  } from '../../../lib/api/types';
  import { drawContainBitmap } from '../live/canvasDraw';
  import { buildSimMoreMenuItems, isSwipeGesture, mapStageKeyToInput } from '../../../lib/simulators/simStageControls';
  import { clientToSimNormalized, simDeviceSubtitle } from './simGeometry';
  import SimulatorLogsDrawer, { type SimLogEntry } from './SimulatorLogsDrawer.svelte';

  interface Props {
    device: SimDevice;
    /** Current browser tab URL for 1-click "Open current URL in simulator". */
    currentUrl?: string;
    /** Whether shown side-by-side with the web browser (split mode). */
    split?: boolean;
    ontogglesplit?: () => void;
    ondevicechange?: (d: SimDevice) => void;
    onclose: () => void;
  }

  let {
    device,
    currentUrl = '',
    split = false,
    ontogglesplit,
    ondevicechange,
    onclose,
  }: Props = $props();

  let liveDevice = $state<SimDevice | null>(null);
  const dev = $derived(liveDevice ?? device);
  const activeDeviceId = $derived(dev.id);
  const booted = $derived(dev.state === 'booted');
  const caps = $derived(dev.capabilities);
  const canStreamLogs = $derived(dev.capabilities.logs);

  const MAX_LOG_ENTRIES = 300;

  let canvasEl = $state<HTMLCanvasElement | null>(null);
  let box = $state({ width: 0, height: 0 });
  let imageSize = $state({ width: 0, height: 0 });
  let streamState = $state<'idle' | 'connecting' | 'live' | 'error'>('idle');
  let streamError = $state('');
  let canDrive = $state(true);
  let fps = $state(5);
  let busy = $state(false);
  let showTextBar = $state(false);
  let textInput = $state('');
  let showLogs = $state(false);
  let logs = $state<SimLogEntry[]>([]);

  let streamHandle: SimFrameStreamHandle | null = null;
  let logSock: WebSocket | null = null;
  let pendingLogs: SimLogEntry[] = [];
  let logFlushRaf: number | null = null;
  let bitmap: ImageBitmap | null = null;
  let pendingFrame: { blob: Blob; seq: number; gen: number } | null = null;
  let decoding = false;
  let gen = 0;
  let logSeq = 0;

  let pointerStart: { clientX: number; clientY: number; norm: { x: number; y: number }; t: number } | null = null;

  function sendStream(msg: SimStreamClientMsg): void {
    streamHandle?.send(msg);
  }

  function draw(): void {
    drawContainBitmap(canvasEl, bitmap, box);
  }

  async function pump(): Promise<void> {
    decoding = true;
    while (pendingFrame) {
      const f = pendingFrame;
      pendingFrame = null;
      if (f.gen !== gen) continue;
      try {
        const bmp = await createImageBitmap(f.blob);
        if (f.gen !== gen) {
          bmp.close();
          continue;
        }
        bitmap?.close();
        bitmap = bmp;
        if (imageSize.width !== bmp.width || imageSize.height !== bmp.height) {
          imageSize = { width: bmp.width, height: bmp.height };
        }
        draw();
      } catch {
        /* skip corrupt frame */
      }
      if (f.gen === gen) streamHandle?.ack(f.seq);
    }
    decoding = false;
  }

  function disconnectStream(): void {
    gen++;
    pendingFrame = null;
    const h = streamHandle;
    streamHandle = null;
    h?.close();
    streamState = 'idle';
  }

  function connectStream(id: string): void {
    disconnectStream();
    const mine = ++gen;
    streamState = 'connecting';
    streamError = '';
    streamHandle = openSimFrameStream(
      id,
      { fps, format: 'jpeg', autoAck: false },
      {
        onOpen: () => {
          if (mine !== gen) return;
          streamState = 'live';
        },
        onHello: (d, drive) => {
          if (mine !== gen) return;
          liveDevice = d;
          canDrive = drive;
          ondevicechange?.(d);
        },
        onState: (d) => {
          if (mine !== gen) return;
          liveDevice = d;
          ondevicechange?.(d);
        },
        onFrame: (blob, header) => {
          if (mine !== gen) return;
          pendingFrame = { blob, seq: header.seq, gen: mine };
          if (!decoding) void pump();
        },
        onError: (msg) => {
          if (mine !== gen) return;
          streamError = msg;
          if (streamState !== 'live') streamState = 'error';
        },
        onClosed: (reason) => {
          if (mine !== gen) return;
          streamState = 'idle';
          streamError = reason;
        },
        onClose: () => {
          if (mine !== gen) return;
          streamHandle = null;
          if (streamState === 'live' || streamState === 'connecting') {
            streamState = 'idle';
          }
        },
      },
    );
  }

  // Sync device prop changes and open stream when booted.
  // Keyed strictly on derived id + booted state so parent prop refreshes on
  // `ondevicechange` (e.g. hello/state frames) don't tear down the live WS.
  const deviceId = $derived(device.id);
  const propBooted = $derived(device.state === 'booted');
  $effect(() => {
    const id = deviceId;
    const isBooted = propBooted;
    untrack(() => {
      liveDevice = device;
      imageSize = { width: 0, height: 0 };
      bitmap?.close();
      bitmap = null;
      if (isBooted && device.capabilities.stream) {
        connectStream(id);
      } else {
        disconnectStream();
      }
    });
    return () => disconnectStream();
  });

  // Observe the canvas itself (not the padded outer stage) so `box` and pointer
  // mapping share the exact drawn rectangle without inset drift.
  $effect(() => {
    const c = canvasEl;
    if (!c) return;
    const ro = new ResizeObserver(() => {
      const r = c.getBoundingClientRect();
      box = { width: r.width, height: r.height };
      draw();
    });
    ro.observe(c);
    return () => ro.disconnect();
  });

  function flushPendingLogs(): void {
    logFlushRaf = null;
    if (pendingLogs.length === 0) return;
    const batch = pendingLogs;
    pendingLogs = [];
    const total = logs.length + batch.length;
    logs =
      total > MAX_LOG_ENTRIES
        ? [...logs.slice(Math.max(0, total - MAX_LOG_ENTRIES)), ...batch].slice(-MAX_LOG_ENTRIES)
        : [...logs, ...batch];
  }

  function cancelPendingLogs(): void {
    if (logFlushRaf !== null) {
      cancelAnimationFrame(logFlushRaf);
      logFlushRaf = null;
    }
    pendingLogs = [];
  }

  // Optional live logs WebSocket. Keyed strictly on primitive id/capability
  // flags so rotate/appearance/state updates don't reconnect and duplicate backlog.
  $effect(() => {
    const id = activeDeviceId;
    const want = showLogs && booted && canStreamLogs;
    if (!want) {
      cancelPendingLogs();
      logSock?.close(1000);
      logSock = null;
      return;
    }
    const ws = simulatorsApi.openLogsSocket(id);
    logSock = ws;
    ws.onmessage = (ev) => {
      if (typeof ev.data !== 'string') return;
      try {
        const msg = JSON.parse(ev.data) as SimLogServerMsg;
        if (msg.type === 'log') {
          pendingLogs.push({ ...msg, id: ++logSeq });
          if (logFlushRaf === null) {
            logFlushRaf = requestAnimationFrame(flushPendingLogs);
          }
        }
      } catch {
        /* ignore */
      }
    };
    return () => {
      cancelPendingLogs();
      ws.close(1000);
      if (logSock === ws) logSock = null;
    };
  });

  // Pointer → tap / swipe on canvas; focus canvas so direct keyboard typing works immediately.
  function onPointerDown(e: PointerEvent): void {
    canvasEl?.focus({ preventScroll: true });
    if (!booted || !canDrive || (!caps.tap && !caps.swipe) || !canvasEl || imageSize.width <= 0) return;
    const r = canvasEl.getBoundingClientRect();
    const norm = clientToSimNormalized(
      { x: e.clientX, y: e.clientY },
      { left: r.left, top: r.top, width: r.width, height: r.height },
      imageSize,
    );
    if (!norm) return;
    pointerStart = { clientX: e.clientX, clientY: e.clientY, norm, t: performance.now() };
    canvasEl.setPointerCapture(e.pointerId);
  }

  function onPointerUp(e: PointerEvent): void {
    if (!pointerStart || !canvasEl || imageSize.width <= 0) return;
    const start = pointerStart;
    pointerStart = null;
    const r = canvasEl.getBoundingClientRect();
    const endNorm = clientToSimNormalized(
      { x: e.clientX, y: e.clientY },
      { left: r.left, top: r.top, width: r.width, height: r.height },
      imageSize,
    );
    if (!endNorm) return;
    if (caps.swipe && isSwipeGesture({ x: start.clientX, y: start.clientY }, { x: e.clientX, y: e.clientY })) {
      const dur = Math.max(80, Math.min(1500, Math.round(performance.now() - start.t)));
      if (streamState === 'live') {
        sendStream({ type: 'swipe', x1: start.norm.x, y1: start.norm.y, x2: endNorm.x, y2: endNorm.y, duration_ms: dur, normalized: true });
      } else {
        void simulatorsApi.input(dev.id, { type: 'swipe', x1: start.norm.x, y1: start.norm.y, x2: endNorm.x, y2: endNorm.y, duration_ms: dur, normalized: true }).catch((err) => toastError('Couldn’t swipe', err));
      }
    } else if (caps.tap) {
      if (streamState === 'live') {
        sendStream({ type: 'tap', x: endNorm.x, y: endNorm.y, normalized: true });
      } else {
        void simulatorsApi.input(dev.id, { type: 'tap', x: endNorm.x, y: endNorm.y, normalized: true }).catch((err) => toastError('Couldn’t tap', err));
      }
    }
  }

  function onStageKeyDown(e: KeyboardEvent): void {
    if (!booted || !canDrive) return;
    const req = mapStageKeyToInput(dev, e);
    if (!req) return;
    e.preventDefault();
    e.stopPropagation();
    if (streamState === 'live') sendStream(req);
    else void simulatorsApi.input(dev.id, req).catch((err) => toastError('Couldn’t send input', err));
  }

  async function doBoot(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      // The parent's device prop flips to booted, so the stream effect connects.
      const updated = await simulatorsApi.boot(dev.id, { wait: true });
      liveDevice = updated;
      ondevicechange?.(updated);
      toasts.success('Simulator ready', updated.name);
    } catch (e) {
      toastError('Couldn’t boot simulator', e);
    } finally {
      busy = false;
    }
  }

  async function doRestart(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      disconnectStream();
      const updated = await simulatorsApi.restart(dev.id, { wait: true });
      liveDevice = updated;
      ondevicechange?.(updated);
      if (updated.state === 'booted' && updated.capabilities.stream) connectStream(updated.id);
      toasts.success('Simulator restarted', updated.name);
    } catch (e) {
      toastError('Couldn’t restart simulator', e);
    } finally {
      busy = false;
    }
  }

  async function doPopOutNative(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      const updated =
        dev.platform === 'ios'
          ? await simulatorsApi.boot(dev.id, { wait: false, open_gui: true })
          : await simulatorsApi.restart(dev.id, { wait: true, headless: false });
      liveDevice = updated;
      ondevicechange?.(updated);
      if (updated.state === 'booted' && updated.capabilities.stream && streamState !== 'live') connectStream(updated.id);
      toasts.info('Opened native window', updated.name);
    } catch (e) {
      toastError('Couldn’t open native window', e);
    } finally {
      busy = false;
    }
  }

  function openMoreMenu(e: MouseEvent): void {
    ctxMenu.show(
      e,
      buildSimMoreMenuItems({
        dev,
        booted,
        busy,
        fps,
        split,
        showTextBar,
        showLogs,
        hasSplitToggle: Boolean(ontogglesplit),
        onToggleTextBar: () => (showTextBar = !showTextBar),
        onScreenshot: () => void doScreenshot(),
        onToggleLogs: () => (showLogs = !showLogs),
        onToggleSplit: () => ontogglesplit?.(),
        onSetFps: (next) => {
          fps = next;
          if (streamState === 'live') sendStream({ type: 'set_fps', fps: next });
        },
        onRestart: () => void doRestart(),
        onPopOutNative: () => void doPopOutNative(),
        onShutdown: () => void doShutdown(),
      }),
    );
  }

  async function doShutdown(): Promise<void> {
    if (busy) return;
    busy = true;
    try {
      const updated = await simulatorsApi.shutdown(dev.id);
      liveDevice = updated;
      ondevicechange?.(updated);
      disconnectStream();
      toasts.info('Simulator shut down', updated.name);
    } catch (e) {
      toastError('Couldn’t shut down simulator', e);
    } finally {
      busy = false;
    }
  }

  async function doOpenCurrentUrl(): Promise<void> {
    if (!currentUrl || busy) return;
    busy = true;
    try {
      const resp = await simulatorsApi.openUrl(dev.id, { url: currentUrl, auto_forward_localhost: true });
      const fwd = resp.forwarded_ports.length > 0 ? ` (forwarded port ${resp.forwarded_ports.join(', ')})` : '';
      toasts.success('Opened on simulator', `${dev.name}${fwd}`);
    } catch (e) {
      toastError('Couldn’t open URL on simulator', e);
    } finally {
      busy = false;
    }
  }

  async function doRotate(): Promise<void> {
    const next: Exclude<SimOrientation, 'unknown'> = dev.orientation === 'landscape' ? 'portrait' : 'landscape';
    try {
      await simulatorsApi.setOrientation(dev.id, { orientation: next });
      liveDevice = { ...dev, orientation: next };
      if (streamState === 'live') sendStream({ type: 'snapshot' });
    } catch (e) {
      toastError('Couldn’t rotate simulator', e);
    }
  }

  async function doToggleAppearance(): Promise<void> {
    const next: SimAppearance = dev.appearance === 'dark' ? 'light' : 'dark';
    try {
      await simulatorsApi.setAppearance(dev.id, { appearance: next });
      liveDevice = { ...dev, appearance: next };
      if (streamState === 'live') sendStream({ type: 'snapshot' });
    } catch (e) {
      toastError('Couldn’t toggle appearance', e);
    }
  }

  async function doSendKey(key: string): Promise<void> {
    try {
      if (streamState === 'live') sendStream({ type: 'key_event', key });
      else await simulatorsApi.input(dev.id, { type: 'key_event', key });
    } catch (e) {
      toastError('Couldn’t send key', e);
    }
  }

  async function doSendText(): Promise<void> {
    const t = textInput.trim();
    if (!t) return;
    try {
      if (streamState === 'live') sendStream({ type: 'text', text: t });
      else await simulatorsApi.input(dev.id, { type: 'text', text: t });
      textInput = '';
    } catch (e) {
      toastError('Couldn’t type text', e);
    }
  }

  async function doScreenshot(): Promise<void> {
    try {
      const blobUrl = await simulatorsApi.screenshotBlobUrl(dev.id, 'png');
      downloadBlobUrl(
        blobUrl,
        `otto-sim-${dev.name.toLowerCase().replace(/[^a-z0-9]+/g, '-')}-${Date.now()}.png`,
      );
      toasts.success('Simulator screenshot saved', dev.name);
    } catch (e) {
      toastError('Couldn’t capture simulator screenshot', e);
    }
  }
</script>

<section class="simv" class:split aria-label="Simulator mirror: {dev.name}">
  <header class="simv-bar scroll-thin">
    <div class="simv-id">
      <span class="simv-dot" class:on={booted} aria-hidden="true"></span>
      <span class="simv-name" title={dev.name}>{dev.name}</span>
      <span class="simv-sub">{simDeviceSubtitle(dev)}</span>
    </div>

    <div class="simv-actions">
      {#if booted && currentUrl && caps.open_url}
        <button class="btn small primary" onclick={() => void doOpenCurrentUrl()} disabled={busy} title="Open active browser URL in this simulator">
          <Icon name="external" size={12} /> Open URL
        </button>
      {/if}
      {#if booted && caps.rotate}
        <button class="icon-btn simv-icon" onclick={() => void doRotate()} aria-label="Rotate simulator" title="Rotate Portrait / Landscape">
          <Icon name="swap" size={13} />
        </button>
      {/if}
      {#if booted && caps.appearance}
        <button class="icon-btn simv-icon" onclick={() => void doToggleAppearance()} aria-label="Toggle light or dark mode" title="Toggle Light / Dark appearance">
          <Icon name="palette" size={13} />
        </button>
      {/if}
      {#if booted && caps.key_event}
        {#if dev.platform === 'android'}
          <button class="icon-btn simv-icon" onclick={() => void doSendKey('BACK')} aria-label="Simulator back" title="Back button">
            <Icon name="chevronLeft" size={13} />
          </button>
        {/if}
        <button class="icon-btn simv-icon" onclick={() => void doSendKey('HOME')} aria-label="Simulator home" title="Home button">
          <Icon name="home" size={13} />
        </button>
      {/if}
      <button
        class="icon-btn simv-icon"
        onclick={openMoreMenu}
        aria-label="More simulator actions"
        title="More simulator actions (text, logs, screenshot, FPS, pop out, shut down)"
      >
        <Icon name="more" size={13} />
      </button>
      <button class="icon-btn simv-icon" onclick={onclose} aria-label="Close simulator view" title="Close simulator view">
        <Icon name="x" size={13} />
      </button>
    </div>
  </header>

  {#if showTextBar && booted && caps.text}
    <div class="simv-textbar">
      <input
        class="simv-input"
        type="text"
        dir="auto"
        bind:value={textInput}
        placeholder="Type text to send to {dev.name}…"
        aria-label="Text to send to simulator"
        onkeydown={(e) => { if (e.key === 'Enter') void doSendText(); }}
      />
      <button class="btn small primary" onclick={() => void doSendText()} disabled={!textInput.trim()}>Send</button>
    </div>
  {/if}

  <div
    class="simv-stage"
    class:interactive={booted && canDrive && (caps.tap || caps.swipe)}
  >
    {#if !booted}
      <div class="simv-center">
        <Icon name="phone" size={24} />
        <p class="simv-msg">{dev.name} is {dev.state.replace('_', ' ')}.</p>
        {#if caps.boot}
          <button class="btn primary" onclick={() => void doBoot()} disabled={busy}>
            <Icon name="play" size={13} /> {busy ? 'Booting…' : 'Boot simulator'}
          </button>
        {/if}
      </div>
    {:else}
      <!-- Focusable stage: pointer taps/swipes and typed keys go to the device (Tab still leaves). -->
      <canvas
        bind:this={canvasEl}
        tabindex="0"
        aria-label="{dev.name} live screen"
        onpointerdown={onPointerDown}
        onpointerup={onPointerUp}
        onkeydown={onStageKeyDown}
      ></canvas>
      {#if imageSize.width === 0}
        <div class="simv-center">
          <Icon name="phone" size={24} />
          <p class="simv-msg">
            {streamState === 'error' ? (streamError || 'Couldn’t connect to simulator stream.') : 'Waiting for simulator screen…'}
          </p>
          {#if streamState === 'error' || streamState === 'idle'}
            <button class="btn small" onclick={() => connectStream(dev.id)}>
              <Icon name="refresh" size={12} /> Reconnect
            </button>
          {/if}
        </div>
      {/if}
      {#if !caps.tap && dev.platform === 'ios'}
        <div class="simv-hint">View-only mirror (interact in Simulator.app, or install idb for direct taps)</div>
      {/if}
    {/if}
  </div>

  {#if showLogs && booted}
    <SimulatorLogsDrawer deviceName={dev.name} {logs} onclear={() => (logs = [])} />
  {/if}
</section>

<style>
  .simv {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface-2);
    color: var(--text);
  }
  .simv.split {
    border-inline-start: 1px solid var(--border);
    max-width: 48%;
  }
  .simv-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    overflow-x: auto;
    flex: none;
    min-width: 0;
  }
  .simv-id {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    flex-shrink: 1;
  }
  .simv-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    flex-shrink: 0;
  }
  .simv-dot.on {
    background: var(--success);
  }
  .simv-name {
    font-size: var(--fs-s);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .simv-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    white-space: nowrap;
  }
  .simv-actions {
    display: flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .simv-icon {
    width: 26px;
    height: 26px;
  }
  .simv-textbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .simv-input {
    flex: 1;
    min-width: 0;
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
  }
  .simv-input:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .simv-stage {
    position: relative;
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    padding: 12px;
    touch-action: none;
    user-select: none;
  }
  .simv-stage.interactive {
    cursor: pointer;
  }
  .simv-stage canvas:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
  }
  .simv-stage canvas {
    position: absolute;
    inset: 10px;
    width: calc(100% - 20px);
    height: calc(100% - 20px);
    display: block;
  }
  .simv-center {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 16px;
    text-align: center;
    color: var(--text-dim);
  }
  .simv-msg {
    margin: 0;
    font-size: var(--fs-s);
    max-width: 36ch;
  }
  .simv-hint {
    position: absolute;
    inset-inline: 12px;
    bottom: 8px;
    margin-inline: auto;
    width: fit-content;
    max-width: calc(100% - 24px);
    padding: 2px 10px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    text-align: center;
    pointer-events: none;
  }
  @media (max-width: 640px) {
    .simv.split {
      max-width: none;
      border-inline-start: none;
      border-top: 1px solid var(--border);
    }
    .simv-sub {
      display: none;
    }
  }
</style>
