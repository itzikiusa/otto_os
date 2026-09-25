<script lang="ts">
  // The side-by-side pane's controls, drawn at the trailing end of the pane's
  // own top row (PageHeader / the Agents TabBar — lib/stores/embedChrome).
  // Rendered only inside the side pane; each button asks the host window.
  import Icon from './Icon.svelte';
  import { postToHost } from '../embedGuest';
</script>

<div class="pane-controls" role="group" aria-label="Side pane" data-testid="pane-controls">
  <span class="pc-sep" aria-hidden="true"></span>
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
