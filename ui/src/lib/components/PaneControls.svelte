<script lang="ts">
  // The side-by-side pane's controls, drawn at the trailing end of the pane's
  // own top row (PageHeader / the Agents TabBar — lib/stores/embedChrome).
  // Browser controls ask the iframe host; native controls are also available
  // on the primary pane and operate on the authenticated native pair.
  import Icon from './Icon.svelte';
  import { postToHost } from '../embedGuest';
  import { isTauri } from '../desktop';
  import { ctxMenu } from '../contextmenu.svelte';
  import { sidePane } from '../stores/sidePane.svelte';
  import { observeNativePane, type PaneState } from '../nativePane';
  import { paneWindowMenuItems } from '../nativePaneMenu';

  let { pane = 'side' }: { pane?: 'side' | 'primary' } = $props();
  let nativeState: PaneState | null = $state(null);
  $effect(() => isTauri ? observeNativePane((state) => { nativeState = state; }) : undefined);
  function showWindowMenu(event: MouseEvent): void {
    const items = paneWindowMenuItems();
    if (nativeState?.mode === 'attached') {
      items.push({ separator: true },
        { label: 'Swap panes', icon: 'swap', action: () => pane === 'side' ? postToHost({ type: 'swap' }) : sidePane.swap() },
        { label: 'Open in main pane', icon: 'maximize', action: () => pane === 'side' ? postToHost({ type: 'promote' }) : sidePane.promote() },
        { label: 'Close side pane', icon: 'x', action: () => pane === 'side' ? postToHost({ type: 'close' }) : sidePane.close() },
      );
    }
    ctxMenu.show(event, items);
  }
</script>

<div class="pane-controls" role="group" aria-label={pane === 'side' ? 'Side pane' : 'Main pane'} data-testid="pane-controls">
  <span class="pc-sep" aria-hidden="true"></span>
  {#if isTauri}
    <button class="icon-btn" onclick={showWindowMenu} aria-label="Pane window" title="Pane window" data-testid="pane-window-menu" aria-haspopup="menu" disabled={!nativeState}>
      <Icon name="more" size={14} />
    </button>
  {:else}
  <button
    class="icon-btn"
    onclick={() => postToHost({ type: 'swap' })}
    aria-label="Swap panes"
    title="Swap panes"
    data-testid="side-pane-swap"
  >
    <Icon name="swap" size={14} />
  </button>
  <button
    class="icon-btn"
    onclick={() => postToHost({ type: 'promote' })}
    aria-label="Open in main pane"
    title="Open in main pane"
    data-testid="side-pane-promote"
  >
    <Icon name="maximize" size={14} />
  </button>
  <button
    class="icon-btn"
    onclick={() => postToHost({ type: 'close' })}
    aria-label="Close side pane"
    title="Close side pane (⌘\)"
    data-testid="side-pane-close"
  >
    <Icon name="x" size={14} />
  </button>
  {/if}
</div>

<style>
  .pane-controls {
    display: flex;
    align-items: center;
    gap: 2px;
    flex-shrink: 0;
  }
  /* A hairline between the page's own actions and the pane's. */
  .pc-sep {
    width: 1px;
    height: 16px;
    margin-inline: 4px 6px;
    background: var(--separator);
  }
</style>
