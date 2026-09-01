<script lang="ts">
  // Tab strip for the Browser module: one row per open BrowserTab. The store
  // (`browser.svelte.ts`) owns the list — this is a thin, stateless renderer.
  // Live updates (a tab created/navigated from another device) arrive via the
  // `browser_tab_updated` WS event, which the store applies in place (see
  // `browser.applyEvent` wired from `lib/events.svelte.ts`); no polling here.

  import Icon from '../../lib/components/Icon.svelte';
  import { browser } from '../../lib/stores/browser.svelte';

  let { onnew }: { onnew: () => void } = $props();

  function close(e: MouseEvent, id: string): void {
    e.stopPropagation();
    void browser.closeTab(id);
  }
</script>

<div class="strip">
  {#each browser.tabs as tab (tab.id)}
    <button
      class="tab"
      class:active={tab.id === browser.activeId}
      onclick={() => browser.select(tab.id)}
      title={tab.url}
    >
      <Icon name="globe" size={12} />
      <span class="title">{tab.title || tab.url}</span>
      <span class="close" onclick={(e) => close(e, tab.id)} role="presentation">
        <Icon name="x" size={11} />
      </span>
    </button>
  {/each}
  <button class="new" onclick={onnew} title="New tab">
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
    overflow-x: auto;
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
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close {
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
