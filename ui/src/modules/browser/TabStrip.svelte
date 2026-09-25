<script lang="ts">
  // Tab strip for the Browser module: one row per open BrowserTab. The store
  // (`browser.svelte.ts`) owns the list — this is a thin, stateless renderer.
  // Live updates (a tab created/navigated from another device) arrive via the
  // `browser_tab_updated` WS event, which the store applies in place (see
  // `browser.applyEvent` wired from `lib/events.svelte.ts`); no polling here.

  import Icon from '../../lib/components/Icon.svelte';
  import { browser } from '../../lib/stores/browser.svelte';

  // inbar: rendered inside the PageHeader bar (no own border row / padding).
  let { onnew, inbar = false }: { onnew: () => void; inbar?: boolean } = $props();

  function close(e: MouseEvent, id: string): void {
    e.stopPropagation();
    void browser.closeTab(id);
  }
</script>

<!-- The tabs scroll in their own box and "+" sits OUTSIDE it: as the last
     child of the scroller (inside PageHeader's width-capped tab slot) it was
     scrolled/clipped off the edge once a few tabs were open. -->
<div class="strip" class:inbar>
  <div class="tabs-scroll">
  {#each browser.tabs as tab (tab.id)}
    <button
      class="tab"
      class:active={tab.id === browser.activeId}
      onclick={() => browser.select(tab.id)}
      title={tab.url}
    >
      <Icon name="globe" size={12} />
      <span class="title">{tab.title || tab.url}</span>
      <span class="close" onclick={(e) => close(e, tab.id)} role="presentation" title="Close tab">
        <Icon name="x" size={11} />
      </span>
    </button>
  {/each}
  </div>
  <button class="new" onclick={onnew} title="New tab" aria-label="New tab">
    <Icon name="plus" size={13} />
  </button>
</div>

<style>
  .strip {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    padding: 0.35rem 0.5rem;
    border-bottom: 1px solid var(--border);
    min-width: 0;
  }
  .strip.inbar {
    padding: 0;
    border-bottom: none;
    max-width: 100%;
  }
  .tabs-scroll {
    display: flex;
    align-items: center;
    gap: 0.25rem;
    flex: 0 1 auto;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: thin;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    max-width: 200px;
    padding: 0.3rem 0.5rem;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-dim);
    font-size: 0.82rem;
    cursor: pointer;
    flex-shrink: 0;
  }
  .tab:hover {
    background: var(--surface);
  }
  .tab.active {
    background: var(--surface);
    border-color: var(--border);
    color: var(--text);
  }
  .title {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close {
    flex-shrink: 0;
    display: flex;
    align-items: center;
    border-radius: var(--radius-s);
    padding: 0.1rem;
    opacity: 0.6;
  }
  .close:hover {
    opacity: 1;
    background: color-mix(in srgb, var(--text) 12%, transparent);
  }
  .new {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-dim);
    cursor: pointer;
    flex-shrink: 0;
  }
  .new:hover {
    background: var(--surface);
    color: var(--text);
  }
</style>
