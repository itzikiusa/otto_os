<script lang="ts">
  // Host for the assistant bar panel (`#/bar`, the desktop shell's `otto-bar`
  // window: transparent, non-activating, native HUD vibrancy behind the page).
  //
  // The bar itself is the shared FloatingBar.svelte (layout.md §7), pulled in
  // with a dynamic import so the main window's bundle doesn't carry it. If that
  // chunk fails to load (an update replaced the assets under a long-lived
  // window), the pill says so inline with Retry and "Open Otto" — Esc still
  // hides it.
  //
  // Contract with FloatingBar (ui/src/lib/desktop.ts):
  //   • size the window with `bar.resize(contentHeight, {radius?})` — it grows
  //     upward from the pill, capped at 560 px and the screen;
  //   • `Esc` → `bar.hide()`; "Open in Otto" → `openInOtto(route)`;
  //   • focus the input on `onDesktopEvent('otto://bar-shown', …)`;
  //   • hold-to-talk arrives as `otto://assistant-voice` `{pressed}`.
  import { onMount, type Component } from 'svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { bar, onDesktopEvent, openInOtto, useGlassWindow } from '../../lib/desktop';

  let Bar: Component | null = $state(null);
  let loading = $state(false);
  let loadError: string | null = $state(null);
  let pillEl: HTMLDivElement | undefined = $state();

  async function load(): Promise<void> {
    loading = true;
    try {
      Bar = (await import('../../lib/components/FloatingBar.svelte')).default as Component;
      loadError = null;
    } catch {
      loadError = 'Otto may have updated while this window was open.';
    } finally {
      loading = false;
    }
  }

  onMount(() => {
    useGlassWindow();
    void load();
  });

  // Error pill only: size + focus + Esc. FloatingBar owns all of this once loaded.
  $effect(() => {
    if (!loadError || Bar) return;
    void bar.resize(56).catch(() => {});
    let unlisten: (() => void) | null = null;
    void onDesktopEvent('otto://bar-shown', () => pillEl?.querySelector('button')?.focus()).then((fn) => (unlisten = fn));
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') void bar.hide().catch(() => {});
    };
    window.addEventListener('keydown', onKey);
    return () => {
      unlisten?.();
      window.removeEventListener('keydown', onKey);
    };
  });
</script>

{#if Bar}
  <Bar />
{:else if loadError}
  <div class="bar-fallback" role="dialog" aria-label="Otto assistant" bind:this={pillEl}>
    <LoadState what="the assistant bar" error={loadError} empty variant="compact" {loading} onretry={() => void load()}>
      {#snippet errorActions()}
        <button class="btn small primary" onclick={() => void openInOtto().catch(() => {})}>Open Otto</button>
      {/snippet}
    </LoadState>
  </div>
{/if}

<style>
  .bar-fallback {
    height: 100vh;
    box-sizing: border-box;
    display: flex;
    align-items: center;
    gap: 10px;
    padding-inline: 18px 10px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--bg-sidebar) 78%, transparent);
    color: var(--text);
    font-size: var(--fs-m);
  }
  @media (prefers-reduced-transparency: reduce) {
    .bar-fallback {
      background: var(--bg-sidebar);
    }
  }
  .bar-fallback > :global(*) {
    flex: 1;
    min-width: 0;
  }
</style>
