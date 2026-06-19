<script lang="ts">
  // Reusable off-canvas slide-over used by the mobile shell to host the
  // Navigator (left) and the RightPanel (right). Open-state is a bound prop so
  // callers can wire it to an existing store flag (e.g. ui.railExpanded /
  // ui.rightOpen). A backdrop fades in behind the panel; tapping it — or
  // pressing Esc — dismisses. Body content is provided via a snippet.
  //
  // Desktop never renders this: callers gate it behind viewport.isMobile, so
  // there is no z-index or layout cost on the unchanged ≥1025px layout.
  import type { Snippet } from 'svelte';

  interface Props {
    /** Bound: whether the drawer is shown. */
    open: boolean;
    /** Which edge the panel slides in from. */
    side?: 'left' | 'right';
    /** Accessible label for the dialog. */
    label?: string;
    /** Panel width (CSS length). Defaults to a touch-friendly, viewport-capped value. */
    width?: string;
    children: Snippet;
  }

  let {
    open = $bindable(),
    side = 'left',
    label = 'Panel',
    width = 'min(86vw, 320px)',
    children,
  }: Props = $props();

  function close(): void {
    open = false;
  }

  // Esc closes the drawer while it's open (mirrors modal dismissal elsewhere).
  $effect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent): void {
      if (e.key === 'Escape') {
        e.preventDefault();
        close();
      }
    }
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });
</script>

{#if open}
  <!-- Backdrop: dismiss on tap. role/handlers kept minimal; the panel stops
       propagation so taps inside don't close it. -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="drawer-backdrop" onclick={close}></div>
  <div
    class="drawer {side}"
    style="width:{width}"
    role="dialog"
    aria-modal="true"
    aria-label={label}
  >
    {@render children()}
  </div>
{/if}

<style>
  .drawer-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.45);
    z-index: 90;
    animation: drawer-fade 140ms ease-out;
  }
  .drawer {
    position: fixed;
    top: 0;
    bottom: 0;
    z-index: 91;
    display: flex;
    flex-direction: column;
    background: var(--bg);
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .drawer.left {
    left: 0;
    border-right: 1px solid var(--border);
    animation: drawer-in-left 160ms ease-out;
  }
  .drawer.right {
    right: 0;
    border-left: 1px solid var(--border);
    animation: drawer-in-right 160ms ease-out;
  }
  @keyframes drawer-fade {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }
  @keyframes drawer-in-left {
    from {
      transform: translateX(-100%);
    }
    to {
      transform: translateX(0);
    }
  }
  @keyframes drawer-in-right {
    from {
      transform: translateX(100%);
    }
    to {
      transform: translateX(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .drawer-backdrop,
    .drawer.left,
    .drawer.right {
      animation: none;
    }
  }
</style>
