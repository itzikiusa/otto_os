<script module lang="ts">
  export type DeviceKind = 'none' | 'iphone' | 'ipad' | 'desktop';
  /** Chooser entries (design §4.2): iPhone / iPad / desktop / none. */
  export const DEVICES: { id: DeviceKind; label: string }[] = [
    { id: 'none', label: 'Fit' },
    { id: 'iphone', label: 'iPhone' },
    { id: 'ipad', label: 'iPad' },
    { id: 'desktop', label: 'Desktop' },
  ];
</script>

<script lang="ts">
  // DeviceFrame — the "device frame" chooser's chrome around an HTML screen
  // (design §4.2): iPhone / iPad / desktop / none. Purely presentational: it
  // fixes the frame's CSS size so the sandboxed iframe inside renders at a
  // realistic viewport width, and paints a bezel. The `scheme` only tints the
  // bezel/backdrop — the mockup's own light/dark handling is up to its CSS
  // (`color-scheme` is forwarded to the iframe via the host's class).
  import type { Snippet } from 'svelte';

  interface Props {
    device: DeviceKind;
    scheme?: 'light' | 'dark';
    children: Snippet;
  }
  let { device, scheme = 'light', children }: Props = $props();

  /** CSS px viewport sizes (logical points). */
  const SIZES: Record<Exclude<DeviceKind, 'none'>, { w: number; h: number; label: string }> = {
    iphone: { w: 390, h: 844, label: 'iPhone 14 · 390×844' },
    ipad: { w: 820, h: 1180, label: 'iPad · 820×1180' },
    desktop: { w: 1280, h: 800, label: 'Desktop · 1280×800' },
  };
  const size = $derived(device === 'none' ? null : SIZES[device]);
</script>

{#if size}
  <div class="frame-stage" class:dark={scheme === 'dark'}>
    <div
      class="device {device}"
      style={`--dev-w:${size.w}px;--dev-h:${size.h}px`}
      title={size.label}
    >
      {#if device === 'iphone'}<span class="notch"></span>{/if}
      <div class="device-screen">{@render children()}</div>
    </div>
    <span class="frame-label">{size.label}</span>
  </div>
{:else}
  <div class="frame-fit" class:dark={scheme === 'dark'}>{@render children()}</div>
{/if}

<style>
  .frame-fit {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .frame-stage {
    flex: 1;
    min-height: 0;
    overflow: auto;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 8px;
    padding: 18px 12px;
    background:
      radial-gradient(circle at 30% 20%, color-mix(in srgb, var(--accent) 10%, transparent), transparent 55%),
      color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .frame-stage.dark {
    background:
      radial-gradient(circle at 30% 20%, color-mix(in srgb, var(--accent) 16%, transparent), transparent 55%),
      #0b1220;
  }
  .device {
    position: relative;
    flex: none;
    width: var(--dev-w);
    height: var(--dev-h);
    max-width: 100%;
    background: #fff;
    box-shadow: 0 30px 60px -30px rgba(0, 0, 0, 0.55), 0 0 0 1px rgba(0, 0, 0, 0.08);
    overflow: hidden;
  }
  .device.iphone {
    border: 12px solid #111;
    border-radius: 48px;
  }
  .device.ipad {
    border: 16px solid #1a1a1a;
    border-radius: 26px;
  }
  .device.desktop {
    border: 1px solid #333;
    border-top-width: 28px;
    border-radius: 10px;
  }
  .device.desktop::before {
    /* window traffic lights */
    content: '';
    position: absolute;
    top: -20px;
    left: 12px;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: #ff5f57;
    box-shadow: 20px 0 0 #febc2e, 40px 0 0 #28c840;
  }
  .notch {
    position: absolute;
    top: 8px;
    left: 50%;
    transform: translateX(-50%);
    width: 120px;
    height: 30px;
    border-radius: 999px;
    background: #111;
    z-index: 2;
  }
  .device-screen {
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    background: #fff;
  }
  .device-screen > :global(*) {
    flex: 1;
    min-height: 0;
  }
  .frame-label {
    font-size: 10.5px;
    color: var(--text-dim);
    font-family: var(--font-mono, monospace);
  }
  .frame-stage.dark .frame-label {
    color: #94a3b8;
  }
</style>
