<script lang="ts">
  // Open-file tab strip: click to switch, × / middle-click to close, a dot for
  // unsaved edits. Closing flushes the buffer first (nothing is dropped).
  import Icon from '../../lib/components/Icon.svelte';
  import { workbench } from './workbench.svelte';

  function nameOf(id: string): string {
    return workbench.open[id]?.doc?.name ?? workbench.metaOf(id)?.name ?? 'Loading…';
  }

  function onAux(e: MouseEvent, id: string): void {
    if (e.button === 1) {
      e.preventDefault();
      workbench.closeTab(id);
    }
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
      e.preventDefault();
      // Logical direction: in RTL the visual right is "previous".
      const rtl = getComputedStyle(e.currentTarget as Element).direction === 'rtl';
      const fwd = (e.key === 'ArrowRight') !== rtl;
      workbench.cycle(fwd ? 1 : -1);
      queueMicrotask(() => {
        (document.querySelector('.wb-tab.active .wb-tab-main') as HTMLElement | null)?.focus();
      });
    }
  }
</script>

{#if workbench.tabs.length > 0}
  <div class="wb-tabs" role="tablist" aria-label="Open files" tabindex="-1" onkeydown={onKey}>
    {#each workbench.tabs as id (id)}
      {@const active = workbench.active === id}
      <div class="wb-tab" class:active data-testid="wb-tab" data-doc-id={id} onauxclick={(e) => onAux(e, id)}>
        <button
          class="wb-tab-main"
          role="tab"
          aria-selected={active}
          tabindex={active ? 0 : -1}
          title={nameOf(id)}
          onclick={() => workbench.openDoc(id)}
        >
          <span class="wb-tab-name">{nameOf(id)}</span>
          {#if workbench.isDirty(id)}<span class="wb-tab-dot" aria-label="Unsaved changes"></span>{/if}
        </button>
        <button
          class="wb-tab-close"
          aria-label={`Close ${nameOf(id)}`}
          title="Close tab (⌘W)"
          onclick={() => workbench.closeTab(id)}
        >
          <Icon name="x" size={11} />
        </button>
      </div>
    {/each}
  </div>
{/if}

<style>
  .wb-tabs {
    display: flex;
    align-items: stretch;
    gap: 1px;
    overflow-x: auto;
    border-block-end: 1px solid var(--border);
    background: var(--surface-2);
    scrollbar-width: thin;
    flex: none;
  }
  .wb-tab {
    display: flex;
    align-items: center;
    max-width: 220px;
    min-width: 0;
    flex: none;
    border-inline-end: 1px solid var(--border);
    color: var(--text-dim);
  }
  .wb-tab.active {
    background: var(--surface);
    color: var(--text);
    box-shadow: inset 0 -2px 0 var(--accent);
  }
  .wb-tab-main {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    padding-block: 7px;
    padding-inline: 12px 4px;
    border: 0;
    background: transparent;
    color: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .wb-tab-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .wb-tab-dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: var(--warning);
  }
  .wb-tab-close {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    margin-inline-end: 4px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: inherit;
    cursor: pointer;
  }
  .wb-tab-close:hover {
    background: var(--hover);
    color: var(--text);
  }
  .wb-tab-main:focus-visible,
  .wb-tab-close:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
</style>
