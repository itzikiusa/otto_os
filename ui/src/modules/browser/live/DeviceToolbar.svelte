<script lang="ts">
  // Contextual Device Emulation bar for RemoteLiveView: shown ONLY when the
  // user toggles Device Preview on (preserves Otto's single-toolbar-row rule
  // during ordinary browsing).
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu } from '../../../lib/contextmenu.svelte';
  import {
    DEVICE_PRESETS,
    clampDim,
    clampDpr,
    rotateEmulation,
    stateForPreset,
    type DeviceEmulationState,
    type DevicePresetId,
  } from './devicePresets';

  interface Props {
    value: DeviceEmulationState;
    capturing?: boolean;
    onchange: (next: DeviceEmulationState) => void;
    onscreenshot?: (mode: 'viewport' | 'full_page') => void;
    onclose: () => void;
  }
  let { value, capturing = false, onchange, onscreenshot, onclose }: Props = $props();

  const isResponsive = $derived(value.presetId === 'responsive');

  function onPresetChange(e: Event): void {
    const id = (e.currentTarget as HTMLSelectElement).value as DevicePresetId;
    onchange(stateForPreset(id, { colorScheme: value.colorScheme, orientation: value.orientation, enabled: true }));
  }

  function onWidthInput(e: Event): void {
    const raw = Number((e.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(raw) || raw < 200) return;
    const width = clampDim(raw, 200, 3840);
    onchange({
      ...value,
      presetId: 'custom',
      width,
      orientation: width > value.height ? 'landscape' : 'portrait',
    });
  }

  function onHeightInput(e: Event): void {
    const raw = Number((e.currentTarget as HTMLInputElement).value);
    if (!Number.isFinite(raw) || raw < 200) return;
    const height = clampDim(raw, 200, 2160);
    onchange({
      ...value,
      presetId: 'custom',
      height,
      orientation: value.width > height ? 'landscape' : 'portrait',
    });
  }

  function onDprChange(e: Event): void {
    const dpr = clampDpr(Number((e.currentTarget as HTMLSelectElement).value));
    onchange({ ...value, dpr });
  }

  function onSchemeChange(e: Event): void {
    const colorScheme = (e.currentTarget as HTMLSelectElement).value as DeviceEmulationState['colorScheme'];
    onchange({ ...value, colorScheme });
  }

  function toggleRotate(): void {
    onchange(rotateEmulation(value));
  }

  function toggleTouch(): void {
    onchange({ ...value, hasTouch: !value.hasTouch });
  }

  function openScreenshotMenu(e: MouseEvent): void {
    if (!onscreenshot) return;
    ctxMenu.show(e, [
      { label: 'Capture viewport (PNG)', icon: 'image', action: () => onscreenshot('viewport') },
      { label: 'Capture full page (PNG)', icon: 'file', action: () => onscreenshot('full_page') },
    ]);
  }
</script>

<div class="devbar scroll-thin" role="toolbar" aria-label="Device emulation">
  <select
    class="dev-select"
    value={value.presetId}
    onchange={onPresetChange}
    aria-label="Device preset"
    title="Device preset"
  >
    {#each DEVICE_PRESETS as p (p.id)}
      <option value={p.id}>{p.label}</option>
    {/each}
  </select>

  {#if !isResponsive}
    <div class="dev-dims">
      <input
        class="dev-num"
        type="number"
        min="200"
        max="3840"
        step="1"
        value={value.width}
        onchange={onWidthInput}
        aria-label="Viewport width"
        title="Viewport width (CSS px)"
      />
      <span class="dev-sep" aria-hidden="true">×</span>
      <input
        class="dev-num"
        type="number"
        min="200"
        max="2160"
        step="1"
        value={value.height}
        onchange={onHeightInput}
        aria-label="Viewport height"
        title="Viewport height (CSS px)"
      />
    </div>

    <button
      class="icon-btn dev-btn"
      onclick={toggleRotate}
      aria-label="Rotate orientation ({value.orientation})"
      title="Rotate ({value.orientation === 'portrait' ? 'Portrait → Landscape' : 'Landscape → Portrait'})"
    >
      <Icon name="swap" size={13} />
    </button>

    <select
      class="dev-select dev-mini"
      value={String(value.dpr)}
      onchange={onDprChange}
      aria-label="Device pixel ratio"
      title="Device pixel ratio (DPR)"
    >
      <option value="1">1×</option>
      <option value="2">2×</option>
      <option value="2.75">2.75×</option>
      <option value="3">3×</option>
    </select>
  {/if}

  <button
    class="btn small"
    class:dev-active={value.hasTouch}
    onclick={toggleTouch}
    aria-pressed={value.hasTouch}
    title={value.hasTouch ? 'Touch emulation on' : 'Enable touch emulation'}
  >
    Touch
  </button>

  <select
    class="dev-select dev-mini"
    value={value.colorScheme}
    onchange={onSchemeChange}
    aria-label="Emulated color scheme"
    title="Emulated prefers-color-scheme"
  >
    <option value="system">Auto theme</option>
    <option value="light">Light</option>
    <option value="dark">Dark</option>
  </select>

  <span class="dev-spacer"></span>

  {#if onscreenshot}
    <button
      class="btn small"
      onclick={openScreenshotMenu}
      disabled={capturing}
      title="Capture screenshot"
    >
      <Icon name="image" size={12} />
      <span>{capturing ? 'Saving…' : 'Screenshot'}</span>
    </button>
  {/if}

  <button
    class="icon-btn dev-btn"
    onclick={onclose}
    aria-label="Exit device preview"
    title="Exit device preview"
  >
    <Icon name="x" size={13} />
  </button>
</div>

<style>
  .devbar {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    min-width: 0;
    overflow-x: auto;
    flex: none;
  }
  .dev-select {
    height: 26px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-xs);
    flex-shrink: 0;
  }
  .dev-select:focus-visible,
  .dev-num:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .dev-mini {
    min-width: 0;
  }
  .dev-dims {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex-shrink: 0;
  }
  .dev-num {
    width: 56px;
    height: 26px;
    padding: 0 6px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--bg);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    text-align: center;
  }
  .dev-sep {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .dev-btn {
    width: 26px;
    height: 26px;
    flex-shrink: 0;
  }
  .dev-active {
    background: var(--accent-soft);
    color: var(--accent-text);
    border-color: var(--accent-line);
  }
  .dev-spacer {
    flex: 1;
    min-width: 4px;
  }
  @media (pointer: coarse), (max-width: 640px) {
    .dev-select,
    .dev-num {
      height: 32px;
      font-size: var(--fs-s);
    }
    .dev-btn {
      width: 32px;
      height: 32px;
    }
  }
</style>
