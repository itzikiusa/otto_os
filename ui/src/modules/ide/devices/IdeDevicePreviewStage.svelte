<script lang="ts">
  // Live screen preview, screenshot capture, Open URL bar, and tap input for IDE Devices.
  // Keys stream lifecycle strictly on primitive `deviceId`/`deviceBooted` so rotating or toggling
  // appearance never interrupts an active Live View stream.
  import { onDestroy, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { downloadBlobUrl } from '../../../lib/downloadBlob';
  import { loadErrorText } from '../../../lib/loadError';
  import { simulatorsApi } from '../../../lib/api/simulators';
  import {
    openSimFrameStream,
    type SimFrameStreamHandle,
  } from '../../../lib/simulators/simFrameStream';
  import { isSwipeGesture } from '../../../lib/simulators/simStageControls';
  import type { SimDevice, SimInputReq } from '../../../lib/api/types';
  import type { DetectedDevServer } from './ideDevServerDetect';
  import { ideDevices } from './ideDevicesState.svelte';

  interface Props {
    device: SimDevice;
    detectedServers: readonly DetectedDevServer[];
    urlInput: string;
    onurlchange: (next: string) => void;
    onopenbrowser: (urlOverride?: string) => void;
  }

  let { device, detectedServers, urlInput, onurlchange, onopenbrowser }: Props = $props();

  let streaming = $state(false);
  let userPaused = $state(false);
  let frameUrl = $state<string | null>(null);
  let frameError = $state<string | null>(null);
  let showTextBar = $state(false);
  let textInput = $state('');
  let streamHandle: SimFrameStreamHandle | null = null;
  let lastAutoUrl = '';
  let pointerStart: { clientX: number; clientY: number; x: number; y: number; t: number } | null = null;

  const deviceId = $derived(device.id);
  const deviceBooted = $derived(device.state === 'booted');
  const deviceCanStream = $derived(device.capabilities.stream);
  const firstDetectedUrl = $derived(detectedServers[0]?.url ?? '');

  // Auto-seed Open URL only when a *new* dev-server URL appears, reading urlInput inside untrack()
  // so manually clearing the field never snaps back to the detected URL.
  $effect(() => {
    const first = firstDetectedUrl;
    if (first && first !== lastAutoUrl) {
      lastAutoUrl = first;
      untrack(() => {
        if (!urlInput.trim()) onurlchange(first);
      });
    }
  });

  function revokeFrame(): void {
    if (frameUrl) {
      URL.revokeObjectURL(frameUrl);
      frameUrl = null;
    }
  }

  function stopStream(): void {
    streamHandle?.close();
    streamHandle = null;
    streaming = false;
  }

  function startStream(dev: SimDevice): void {
    stopStream();
    if (dev.state !== 'booted' || !dev.capabilities.stream) return;
    frameError = null;
    streaming = true;
    streamHandle = openSimFrameStream(
      dev.id,
      { fps: 4, format: 'jpeg', autoAck: true },
      {
        onFrame(blob) {
          const nextUrl = URL.createObjectURL(blob);
          revokeFrame();
          frameUrl = nextUrl;
        },
        onError(message) {
          frameError = message;
        },
        onClosed(reason) {
          if (reason) frameError = reason;
          streaming = false;
          streamHandle = null;
        },
        onClose() {
          streaming = false;
          streamHandle = null;
        },
      },
    );
  }

  // A fresh PNG from the device (the live frame is a JPEG), as the Browser stage saves it.
  async function saveScreenshot(): Promise<void> {
    try {
      const blobUrl = await simulatorsApi.screenshotBlobUrl(device.id, 'png');
      downloadBlobUrl(blobUrl, `${device.name.replace(/[^a-z0-9_-]+/gi, '-').toLowerCase()}-screenshot.png`);
    } catch (e) {
      frameError = loadErrorText(e);
    }
  }

  // Over the live socket when it is open, else over HTTP.
  function send(req: SimInputReq): void {
    if (streamHandle?.send(req)) return;
    void simulatorsApi.input(device.id, req).catch((err) => {
      frameError = loadErrorText(err);
    });
  }

  function sendKey(key: string): void {
    if (device.state !== 'booted' || !device.capabilities.key_event) return;
    send({ type: 'key_event', key });
  }

  function sendText(): void {
    const t = textInput.trim();
    if (!t || device.state !== 'booted' || !device.capabilities.text) return;
    textInput = '';
    send({ type: 'text', text: t });
  }

  function handlePointerDown(e: PointerEvent): void {
    if (device.state !== 'booted' || (!device.capabilities.tap && !device.capabilities.swipe)) return;
    const target = e.currentTarget as HTMLElement;
    const rect = target.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;
    const x = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    const y = Math.max(0, Math.min(1, (e.clientY - rect.top) / rect.height));
    pointerStart = { clientX: e.clientX, clientY: e.clientY, x, y, t: performance.now() };
    target.setPointerCapture(e.pointerId);
  }

  function handlePointerUp(e: PointerEvent): void {
    if (!pointerStart) return;
    const start = pointerStart;
    pointerStart = null;
    const target = e.currentTarget as HTMLElement;
    const rect = target.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;
    const x = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
    const y = Math.max(0, Math.min(1, (e.clientY - rect.top) / rect.height));
    if (device.capabilities.swipe && isSwipeGesture({ x: start.clientX, y: start.clientY }, { x: e.clientX, y: e.clientY })) {
      const duration_ms = Math.max(80, Math.min(1500, Math.round(performance.now() - start.t)));
      send({ type: 'swipe', x1: start.x, y1: start.y, x2: x, y2: y, duration_ms, normalized: true });
    } else if (device.capabilities.tap) {
      send({ type: 'tap', x, y, normalized: true });
    }
  }

  function openMoreMenu(e: MouseEvent): void {
    const items: MenuItem[] = [];
    if (device.state === 'booted') {
      if (device.capabilities.text) {
        items.push({
          label: showTextBar ? 'Hide text bar' : 'Type text…',
          icon: 'edit',
          action: () => (showTextBar = !showTextBar),
        });
      }
      if (device.capabilities.screenshot) {
        items.push({
          label: 'Save screenshot (PNG)',
          icon: 'image',
          action: () => void saveScreenshot(),
        });
      }
      if (device.capabilities.boot) {
        items.push(
          { separator: true },
          {
            label: device.platform === 'ios' ? 'Pop out in Simulator.app' : 'Restart in native window',
            icon: 'external',
            disabled: Boolean(ideDevices.busyById[device.id]),
            action: () => void ideDevices.popOutNative(device),
          },
        );
      }
    }
    ctxMenu.show(e, items);
  }

  // Embedded-first: auto-start the live stream when a booted device is selected
  // (unless explicitly paused by the user). Keyed strictly on primitive id/booted.
  // Busy is tracked too: a restart or Android pop-out ends the old stream while the
  // device stays booted in the store, so the view resumes once the action settles.
  let prevDeviceId: string | null = null;
  $effect(() => {
    const id = deviceId;
    const booted = deviceBooted;
    const canStream = deviceCanStream;
    const paused = userPaused;
    const busy = Boolean(ideDevices.busyById[id]);
    untrack(() => {
      if (id !== prevDeviceId) {
        prevDeviceId = id;
        userPaused = false;
        stopStream();
        revokeFrame();
        frameError = null;
        if (booted && canStream) startStream(device);
      } else if (!booted) {
        stopStream();
        frameError = null;
      } else if (booted && canStream && !paused && !busy && !streaming) {
        startStream(device);
      }
    });
  });

  onDestroy(() => {
    stopStream();
    revokeFrame();
  });
</script>

<div class="sim-preview-pane">
  <form
    class="sim-url-bar"
    onsubmit={(e) => {
      e.preventDefault();
      void ideDevices.openUrl(device, urlInput);
    }}
  >
    <Icon name="globe" size={12} />
    <input
      type="url"
      class="input sim-url-input"
      dir="ltr"
      aria-label="URL to open on device"
      placeholder="http://127.0.0.1:5173 or myapp://…"
      value={urlInput}
      oninput={(e) => onurlchange(e.currentTarget.value)}
    />
    <button
      type="submit"
      class="btn small"
      disabled={device.state !== 'booted' || !urlInput.trim() || !device.capabilities.open_url}
      title="Open URL on this simulator (auto-forwards Android localhost ports)"
    >
      Open
    </button>
  </form>

  {#if detectedServers.length > 0}
    <div class="sim-dev-urls">
      <span class="sim-dev-urls-lbl">Running:</span>
      {#each detectedServers as srv (srv.url)}
        <button
          type="button"
          class="sim-url-chip mono"
          dir="ltr"
          onclick={() => {
            onurlchange(srv.url);
            if (device.state === 'booted') void ideDevices.openUrl(device, srv.url);
          }}
          title={device.state === 'booted' ? `Open ${srv.url} on ${device.name}` : `Fill ${srv.url}`}
        >
          {srv.url}
        </button>
      {/each}
    </div>
  {/if}

  <div class="sim-stage-bar">
    {#if device.state === 'booted'}
      {#if device.capabilities.key_event}
        {#if device.platform === 'android'}
          <button
            type="button"
            class="icon-btn"
            aria-label="Device back"
            title="Back"
            onclick={() => sendKey('BACK')}
          >
            <Icon name="chevronLeft" size={12} />
          </button>
        {/if}
        <button
          type="button"
          class="icon-btn"
          aria-label="Device home"
          title="Home"
          onclick={() => sendKey('HOME')}
        >
          <Icon name="home" size={12} />
        </button>
      {/if}
      <button
        type="button"
        class="btn small ghost"
        disabled={!device.capabilities.stream}
        onclick={() => {
          if (streaming) {
            userPaused = true;
            stopStream();
          } else {
            userPaused = false;
            startStream(device);
          }
        }}
      >
        <Icon name={streaming ? 'stop' : 'eye'} size={12} />
        {streaming ? 'Pause' : 'Live view'}
      </button>
      <button
        type="button"
        class="icon-btn"
        aria-label="More device preview actions"
        title="More actions (type text, screenshot, native window)"
        onclick={openMoreMenu}
      >
        <Icon name="more" size={12} />
      </button>
    {:else}
      <button
        type="button"
        class="btn small primary"
        disabled={!device.capabilities.boot || Boolean(ideDevices.busyById[device.id])}
        onclick={() => void ideDevices.boot(device)}
      >
        <Icon name="play" size={12} />
        Boot {device.name}
      </button>
    {/if}
    <button
      type="button"
      class="btn small ghost sim-stage-browser"
      onclick={() => onopenbrowser(urlInput)}
    >
      <Icon name="external" size={12} />
      Preview in Browser
    </button>
  </div>

  {#if showTextBar && device.state === 'booted' && device.capabilities.text}
    <form
      class="sim-url-bar"
      onsubmit={(e) => {
        e.preventDefault();
        sendText();
      }}
    >
      <Icon name="edit" size={12} />
      <input
        type="text"
        class="input sim-url-input"
        dir="auto"
        aria-label="Text to send to device"
        placeholder="Type text to send to {device.name}…"
        bind:value={textInput}
      />
      <button type="submit" class="btn small primary" disabled={!textInput.trim()}>Send</button>
    </form>
  {/if}

  {#if frameError}
    <div class="sim-frame-err" use:announceShown={{ text: frameError, assertive: true }}>
      {frameError}
    </div>
  {/if}

  <div class="sim-stage">
    {#if frameUrl}
      <button
        type="button"
        class="sim-frame-btn"
        class:is-interactive={device.capabilities.tap || device.capabilities.swipe}
        aria-label={device.capabilities.tap ? `Interact with ${device.name} screen` : `${device.name} screenshot`}
        onpointerdown={handlePointerDown}
        onpointerup={handlePointerUp}
      >
        <img class="sim-frame-img" src={frameUrl} alt={`${device.name} screen`} draggable="false" />
      </button>
    {:else if device.state === 'booted'}
      <div class="sim-stage-placeholder">
        {streaming
          ? `Connecting to ${device.name}…`
          : userPaused
            ? `Live view paused for ${device.name}.`
            : `Live view stopped for ${device.name}.`}
      </div>
    {:else}
      <div class="sim-stage-placeholder">
        {device.name} is {device.state}. Boot it to run embedded inside the IDE.
      </div>
    {/if}
  </div>
</div>

<style>
  .sim-preview-pane {
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }
  .sim-url-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-block-end: 1px solid var(--separator);
    flex-shrink: 0;
  }
  .sim-url-input {
    flex: 1;
    min-inline-size: 0;
    block-size: 26px;
    font-size: var(--fs-xs);
  }
  .sim-dev-urls {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 8px;
    overflow-x: auto;
    border-block-end: 1px solid var(--separator);
    flex-shrink: 0;
  }
  .sim-dev-urls-lbl {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .sim-url-chip {
    flex-shrink: 0;
    padding: 2px 6px;
    border-radius: var(--radius-s);
    border: 1px solid var(--accent-line);
    background: var(--accent-soft);
    color: var(--accent-text);
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .sim-stage-bar {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
    padding: 4px 8px;
    border-block-end: 1px solid var(--separator);
    flex-shrink: 0;
  }
  .sim-stage-browser {
    margin-inline-start: auto;
  }
  .sim-frame-err {
    padding: 6px 8px;
    font-size: var(--fs-xs);
    color: var(--danger);
    background: var(--danger-soft);
    flex-shrink: 0;
  }
  .sim-stage {
    flex: 1;
    min-block-size: 0;
    overflow: auto;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 8px;
    background: var(--bg);
  }
  .sim-frame-btn {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    padding: 0;
    background: transparent;
    overflow: hidden;
    max-block-size: 100%;
    max-inline-size: 100%;
    display: inline-flex;
  }
  .sim-frame-btn.is-interactive {
    cursor: pointer;
    touch-action: none;
    user-select: none;
  }
  .sim-frame-img {
    display: block;
    max-block-size: 100%;
    max-inline-size: 100%;
    object-fit: contain;
  }
  .sim-stage-placeholder {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    text-align: center;
    padding: 12px;
  }
</style>
