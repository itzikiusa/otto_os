<script lang="ts">
  // Reusable off-canvas slide-over used by the mobile shell to host the
  // Navigator (left) and the RightPanel (right). Open-state is a bound prop so
  // callers can wire it to an existing store flag (e.g. ui.navDrawerOpen /
  // ui.rightOpen). A backdrop fades in behind the panel; tapping it — or
  // pressing Esc — dismisses. Body content is provided via a snippet.
  //
  // `inline` presents the same mounted children in the desktop layout. This
  // lets draft-bearing panels cross a breakpoint without destroying state.
  import { tick, untrack, type Snippet } from 'svelte';
  import { dialogFocus } from '../lib/dialogFocus';
  import { ui } from '../lib/stores/ui.svelte';
  import Icon from '../lib/components/Icon.svelte';

  interface Props {
    /** Bound: whether the drawer is shown. */
    open: boolean;
    /** Present children in normal flow, without modal behavior. */
    inline?: boolean;
    /** Which edge the panel slides in from. */
    side?: 'left' | 'right';
    /** Accessible label for the dialog. */
    label?: string;
    /** A visible header (title + ✕) above a scrolling body, instead of the
     *  floating ✕ over the content — for drawers that host a long form/list. */
    title?: string;
    /** Panel width (CSS length). Defaults to a touch-friendly, viewport-capped value. */
    width?: string;
    children: Snippet;
  }

  let {
    open = $bindable(),
    inline = false,
    side = 'left',
    label = 'Panel',
    title,
    width = 'min(86vw, 320px)',
    children,
  }: Props = $props();

  function close(): void {
    open = false;
  }

  let panel: HTMLDivElement | undefined = $state();
  // Once shown inline, keep its state when the mobile drawer is closed too.
  let kept = $state(false);
  $effect(() => { if (inline) kept = true; });
  $effect.pre(() => {
    void inline;
    // Changing display:contents into the overlay box can blur a descendant.
    // Restore only the control already focused here before the layout changed.
    const focused = document.activeElement;
    if (focused instanceof HTMLElement && panel?.contains(focused)) {
      void tick().then(() => { if (focused.isConnected && (inline || open)) focused.focus(); });
    }
  });
  // Register as an open overlay (like Modal) so the native browser webview and
  // global shortcuts treat an open drawer as modal. untrack: pushModal reads
  // modalCount — the effect must depend only on `inline`/`open`.
  $effect(() => {
    if (!inline && open) {
      untrack(() => ui.pushModal());
      return () => untrack(() => ui.popModal());
    }
  });
  $effect(() => {
    if (!inline && open && panel) {
      const focus = dialogFocus(panel, close);
      return () => focus.destroy();
    }
  });
</script>

{#if open && !inline}
  <!-- Backdrop: dismiss on tap. role/handlers kept minimal; the panel stops
       propagation so taps inside don't close it. -->
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="drawer-backdrop" onclick={close}></div>
{/if}
{#if open || inline || kept}
  <div
    class="drawer {side}"
    class:inline
    hidden={!inline && !open}
    bind:this={panel}
    style:width={inline ? undefined : width}
    role={inline ? undefined : 'dialog'}
    aria-modal={inline ? undefined : true}
    aria-label={inline ? undefined : (title ?? label)}
  >
    <!-- Always-visible close affordance: tapping the thin backdrop sliver left by
         a wide drawer is hard on a phone, so give an explicit ✕. -->
    {#if !inline && title}
      <header class="drawer-head">
        <h2>{title}</h2>
        <button class="icon-btn" onclick={close} aria-label="Close {title}" title="Close {title}" aria-keyshortcuts="Escape">
          <Icon name="x" size={14} />
        </button>
      </header>
      <div class="drawer-body">{@render children()}</div>
    {:else}
      {#if !inline}
        <button class="icon-btn drawer-close" onclick={close} aria-label="Close {label}" title="Close {label}" aria-keyshortcuts="Escape">
          <Icon name="x" size={14} />
        </button>
      {/if}
      {@render children()}
    {/if}
  </div>
{/if}

<style>
  .drawer-backdrop {
    position: fixed;
    inset: 0;
    background: var(--scrim);
    z-index: var(--z-drawer);
    animation: otto-fade-in var(--dur-fast) var(--ease-out);
  }
  .drawer {
    position: fixed;
    top: 0;
    bottom: 0;
    z-index: calc(var(--z-drawer) + 1);
    display: flex;
    flex-direction: column;
    background: var(--bg);
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  .drawer.inline {
    display: contents;
  }
  .drawer[hidden] {
    display: none;
  }
  /* Titled drawers: a header row (title + ✕) over a body that scrolls itself. */
  .drawer-head {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 16px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .drawer-head h2 {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .drawer-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  /* Floating close button pinned to the panel's top corner, above content. */
  .drawer-close {
    position: absolute;
    top: 8px;
    inset-inline-end: 8px;
    z-index: 2;
    /* The shared 24px .icon-btn (same as Modal's ✕); this only floats it over
       the panel content with an opaque-enough tile. */
    border: 1px solid var(--border);
    background: color-mix(in srgb, var(--surface) 88%, transparent);
    color: var(--text);
  }
  .drawer-close:hover {
    background: var(--surface-2);
  }
  /* `side` is the reading-direction side: in RTL the Navigator drawer comes
     from the right (logical insets + a mirrored slide). */
  .drawer.left {
    inset-inline-start: 0;
    border-inline-end: 1px solid var(--border);
    --slide-from: -100%;
    animation: otto-slide-in var(--dur-enter) var(--ease-out);
  }
  .drawer.right {
    inset-inline-end: 0;
    border-inline-start: 1px solid var(--border);
    --slide-from: 100%;
    animation: otto-slide-in var(--dur-enter) var(--ease-out);
  }
  :global([dir='rtl']) .drawer.left {
    --slide-from: 100%;
  }
  :global([dir='rtl']) .drawer.right {
    --slide-from: -100%;
  }
  @media (prefers-reduced-motion: reduce) {
    .drawer-backdrop,
    .drawer.left,
    .drawer.right {
      animation: none;
    }
  }
</style>
