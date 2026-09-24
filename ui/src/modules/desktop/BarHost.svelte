<script lang="ts">
  // Host for the assistant bar panel (`#/bar`, the desktop shell's `otto-bar`
  // window: transparent, non-activating, native HUD vibrancy behind the page).
  //
  // The bar itself is the shared FloatingBar.svelte (layout.md §7). It is
  // resolved lazily through import.meta.glob so this route builds with or
  // without it; until it exists, a minimal pill keeps ⌥Space useful (Esc
  // hides, "Open Otto" jumps to the full window).
  //
  // Contract with FloatingBar (ui/src/lib/desktop.ts):
  //   • size the window with `bar.resize(contentHeight, {radius?})` — it grows
  //     upward from the pill, capped at 560 px and the screen;
  //   • `Esc` → `bar.hide()`; "Open in Otto" → `openInOtto(route)`;
  //   • focus the input on `onDesktopEvent('otto://bar-shown', …)`;
  //   • hold-to-talk arrives as `otto://assistant-voice` `{pressed}`.
  import { onMount, type Component } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import { bar, onDesktopEvent, openInOtto, useGlassWindow } from '../../lib/desktop';

  const candidates = import.meta.glob<{ default: Component }>(
    '../../lib/components/FloatingBar.svelte',
  );

  let Bar: Component | null = $state(null);
  let resolved = $state(false);
  let openBtn: HTMLButtonElement | undefined = $state();

  onMount(() => {
    useGlassWindow();
    const load = Object.values(candidates)[0];
    if (load) {
      void load()
        .then((m) => (Bar = m.default))
        .catch(() => {})
        .finally(() => (resolved = true));
    } else {
      resolved = true;
    }
  });

  // Fallback pill only: size + focus + Esc. FloatingBar owns all of this.
  $effect(() => {
    if (!resolved || Bar) return;
    void bar.resize(56).catch(() => {});
    let unlisten: (() => void) | null = null;
    void onDesktopEvent('otto://bar-shown', () => openBtn?.focus()).then((fn) => (unlisten = fn));
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
{:else if resolved}
  <div class="pill" role="dialog" aria-label="Otto assistant">
    <Icon name="sparkle" size={16} />
    <span class="msg">The assistant bar arrives with the next Otto update.</span>
    <button class="btn small primary" bind:this={openBtn} onclick={() => void openInOtto().catch(() => {})}>
      Open Otto
    </button>
  </div>
{/if}

<style>
  .pill {
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
    .pill {
      background: var(--bg-sidebar);
    }
  }
  .pill :global(svg) {
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .msg {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-dim);
  }
</style>
