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
    <!-- The tab is a wrapper holding TWO real buttons (select + close): a
         close control nested inside the select button was a clickable span
         no keyboard or screen reader could reach. Middle-click closes too. -->
    <div class="tab" class:active={tab.id === browser.activeId}>
      <button
        class="tab-main"
        onclick={() => browser.select(tab.id)}
        onauxclick={(e) => { if (e.button === 1) close(e, tab.id); }}
        title={tab.title ? `${tab.title}\n${tab.url}` : tab.url}
        aria-current={tab.id === browser.activeId ? 'page' : undefined}
      >
        <Icon name="globe" size={12} />
        <span class="title">{tab.title || tab.url}</span>
      </button>
      <button
        class="close"
        onclick={(e) => close(e, tab.id)}
        aria-label="Close tab {tab.title || tab.url}"
        title="Close tab"
      >
        <Icon name="x" size={11} />
      </button>
    </div>
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
    gap: 4px;
    padding: 6px 8px;
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
    gap: 4px;
    flex: 0 1 auto;
    min-width: 0;
    overflow-x: auto;
    scrollbar-width: thin;
  }
  .tab {
    display: flex;
    align-items: center;
    gap: 2px;
    max-width: 200px;
    padding-inline-end: 4px;
    border-radius: var(--radius-s);
    border: 1px solid transparent;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-s);
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
  .tab-main {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding: 4px 4px 4px 8px;
    border: none;
    background: none;
    color: inherit;
    font: inherit;
    cursor: pointer;
    border-radius: var(--radius-s);
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
    justify-content: center;
    width: 18px;
    height: 18px;
    padding: 0;
    border: none;
    background: none;
    color: inherit;
    border-radius: var(--radius-s);
    cursor: pointer;
    opacity: 0.6;
  }
  .close:hover,
  .close:focus-visible {
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
