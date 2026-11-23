<script lang="ts">
  // Composes RemoteLiveView with the contextual DeviceToolbar (responsive/device
  // emulation) and opt-in LiveInspector drawer (Console / Network / Element)
  // while keeping RemoteLiveView focused on the screencast & input surface.
  import type {
    BrowserConsoleEntry,
    BrowserDevtoolsSnapshot,
    BrowserInspectedElement,
    BrowserNetworkEntry,
    BrowserTab,
  } from '../../../lib/api/types';
  import DeviceToolbar from './DeviceToolbar.svelte';
  import LiveInspector, { type InspectorTab } from './LiveInspector.svelte';
  import RemoteLiveView from './RemoteLiveView.svelte';
  import { downloadLiveScreenshot } from './screenshotDownload';
  import {
    DEFAULT_EMULATION_STATE,
    emulatedViewport,
    presetById,
    type DeviceEmulationState,
  } from './devicePresets';
  import type { LiveViewState } from './protocol';

  interface Props {
    tab: BrowserTab;
    emulation: DeviceEmulationState;
    onemulationchange: (next: DeviceEmulationState) => void;
    inspectorOpen: boolean;
    oninspectorchange: (open: boolean) => void;
    onmark?: (el: BrowserInspectedElement) => void;
    onnav?: (url: string, title: string) => void;
    onstate?: (s: LiveViewState) => void;
    onfocusurl?: () => void;
    onnewtab?: (url: string) => void;
    onreader?: () => void;
  }

  let {
    tab,
    emulation,
    onemulationchange,
    inspectorOpen,
    oninspectorchange,
    onmark,
    onnav,
    onstate,
    onfocusurl,
    onnewtab,
    onreader,
  }: Props = $props();

  let remoteView = $state<ReturnType<typeof RemoteLiveView> | null>(null);
  let capturing = $state(false);
  let consoleEntries = $state<BrowserConsoleEntry[]>([]);
  let networkEntries = $state<BrowserNetworkEntry[]>([]);
  let inspected = $state<BrowserInspectedElement | null>(null);
  let inspectPicking = $state(false);
  let inspectorTab = $state<InspectorTab>('console');

  const fixedViewport = $derived(emulatedViewport(emulation));
  const deviceFramed = $derived(emulation.enabled && fixedViewport !== null);
  const activeDeviceLabel = $derived.by(() => {
    if (!deviceFramed || !fixedViewport) return '';
    const p = presetById(emulation.presetId);
    const name = p ? p.shortLabel : 'Custom';
    return `${name} · ${fixedViewport.width}×${fixedViewport.height} @${fixedViewport.device_scale_factor}×`;
  });

  // Reset inspector buffers when switching tabs.
  const tabId = $derived(tab.id);
  $effect(() => {
    void tabId;
    consoleEntries = [];
    networkEntries = [];
    inspected = null;
    inspectPicking = false;
  });

  function onConsole(entry: BrowserConsoleEntry): void {
    consoleEntries = [...consoleEntries.slice(-199), entry];
  }

  function onNetwork(entry: BrowserNetworkEntry): void {
    const idx = networkEntries.findIndex((n) => n.request_id === entry.request_id);
    networkEntries =
      idx >= 0
        ? networkEntries.map((n, i) => (i === idx ? entry : n))
        : [...networkEntries.slice(-199), entry];
  }

  function onSnapshot(snap: BrowserDevtoolsSnapshot): void {
    consoleEntries = snap.console;
    networkEntries = snap.network;
  }

  function onInspected(el: BrowserInspectedElement | null): void {
    inspected = el;
    inspectPicking = false;
    inspectorTab = 'element';
    if (!inspectorOpen) oninspectorchange(true);
  }

  export function back(): void {
    remoteView?.back();
  }
  export function forward(): void {
    remoteView?.forward();
  }
  export function reload(): void {
    remoteView?.reload();
  }
  export function stop(): void {
    remoteView?.stop();
  }
  export function setGuard(on: boolean): boolean {
    return remoteView?.setGuard(on) ?? false;
  }
  export async function captureScreenshot(mode: 'viewport' | 'full_page' = 'viewport'): Promise<void> {
    if (capturing) return;
    capturing = true;
    try {
      const suffix = emulation.enabled && emulation.presetId !== 'responsive' ? emulation.presetId : '';
      await downloadLiveScreenshot(tab.id, tab.url, mode, suffix);
    } finally {
      capturing = false;
    }
  }
</script>

<div class="live-stage">
  {#if emulation.enabled}
    <DeviceToolbar
      value={emulation}
      {capturing}
      onchange={onemulationchange}
      onscreenshot={(mode) => void captureScreenshot(mode)}
      onclose={() => onemulationchange({ ...DEFAULT_EMULATION_STATE })}
    />
  {/if}

  {#if deviceFramed && activeDeviceLabel}
    <div class="stage-meta" data-testid="device-emulation-label">
      <span>{activeDeviceLabel}</span>
      {#if emulation.colorScheme !== 'system'}<span class="stage-dim">· {emulation.colorScheme}</span>{/if}
      {#if emulation.hasTouch}<span class="stage-dim">· touch</span>{/if}
    </div>
  {/if}

  <div class="stage-canvas" class:framed={deviceFramed}>
    <RemoteLiveView
      bind:this={remoteView}
      {tab}
      {emulation}
      {inspectorOpen}
      {inspectPicking}
      onconsole={onConsole}
      onnetwork={onNetwork}
      onsnapshot={onSnapshot}
      oninspected={onInspected}
      {onnav}
      {onstate}
      {onfocusurl}
      {onnewtab}
      {onreader}
    />
  </div>

  {#if inspectorOpen}
    <LiveInspector
      {consoleEntries}
      {networkEntries}
      {inspected}
      {inspectPicking}
      bind:activeTab={inspectorTab}
      ontogglepick={() => (inspectPicking = !inspectPicking)}
      onclear={() => {
        consoleEntries = [];
        networkEntries = [];
        remoteView?.clearDevtools();
      }}
      {onmark}
      onclose={() => {
        inspectPicking = false;
        oninspectorchange(false);
      }}
    />
  {/if}
</div>

<style>
  .live-stage {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--surface-2);
  }
  .stage-meta {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 4px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    flex: none;
  }
  .stage-dim {
    color: var(--text-dim);
  }
  .stage-canvas {
    flex: 1;
    min-width: 0;
    min-height: 0;
    display: flex;
  }
  .stage-canvas.framed {
    padding: 12px;
  }
  .stage-canvas.framed > :global(.rlv) {
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    overflow: hidden;
    box-shadow: var(--shadow-card);
  }
</style>
