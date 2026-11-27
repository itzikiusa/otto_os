<script lang="ts">
  // The ONE phone drawer (design.md §2, §7.1–§7.2), opened by the page
  // header's ☰ and by BottomNav's "Menu" tab, portrait or on its side: the
  // Navigator (bell and Close at its top, modules, workspaces) with what the
  // old BottomNav "More" sheet carried at the top of its footer — Commands,
  // the Light / Full switch and the machine status — then Help, Settings and
  // the account. The whole drawer scrolls as one: a phone on its side has
  // ~300 px, less than the Navigator's head and foot.
  import Icon from '../lib/components/Icon.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { lightMode } from '../lib/lightMode.svelte';
  import MachineChip from './MachineChip.svelte';
  import Navigator from './Navigator.svelte';
</script>

<div class="pnd">
  <Navigator>
    {#snippet extras()}
      <div class="pnd-extras" data-testid="short-nav-extras">
        <button
          type="button"
          class="pnd-row"
          onclick={() => {
            ui.navDrawerOpen = false;
            ui.openPalette('commands');
          }}
        >
          <Icon name="command" size={14} />
          <span class="pnd-grow">Commands</span>
          <kbd>⌘K</kbd>
        </button>
        {#if lightMode.available}
          <button
            type="button"
            class="pnd-row"
            data-testid="light-toggle"
            onclick={() => {
              ui.navDrawerOpen = false;
              void lightMode.set(!lightMode.active);
            }}
          >
            <Icon name={lightMode.active ? 'maximize' : 'minimize'} size={14} />
            <span class="pnd-grow">{lightMode.active ? 'Full app' : 'Light version'}</span>
          </button>
        {/if}
        <div class="pnd-machine" data-testid="more-machine">
          <span class="pnd-grow">Machine</span>
          <MachineChip />
        </div>
      </div>
    {/snippet}
  </Navigator>
</div>

<style>
  .pnd {
    block-size: 100%;
    min-block-size: 0;
    overflow-y: auto;
  }
  .pnd :global(.navigator) {
    block-size: auto;
    min-block-size: 100%;
    inline-size: 100%;
  }
  .pnd :global(.nav-scroll) {
    flex: none;
    overflow: visible;
  }
  .pnd-extras {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding-block-end: 6px;
    margin-block-end: 6px;
    border-block-end: 1px solid var(--separator);
  }
  .pnd-row,
  .pnd-machine {
    display: flex;
    align-items: center;
    gap: 8px;
    min-block-size: 40px;
    padding-inline: 8px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .pnd-row:hover {
    background: var(--hover);
  }
  .pnd-row > :global(svg) {
    color: var(--text-dim);
  }
  .pnd-grow {
    flex: 1;
    min-inline-size: 0;
  }
  .pnd-row kbd {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .pnd-machine {
    cursor: default;
    color: var(--text-dim);
  }
  /* Hidden until the machine status has loaded (MachineChip renders nothing). */
  .pnd-machine:not(:has(:global(.machine-chip))) {
    display: none;
  }
</style>
