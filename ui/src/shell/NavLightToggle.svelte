<script lang="ts">
  // Compact 1-tap Full ↔ Light version switch rendered in the Navigator footer
  // on touch/mobile/tablet viewports (when ShortNavDrawer is not already showing it).
  import Icon from '../lib/components/Icon.svelte';
  import { lightMode } from '../lib/lightMode.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import { viewport } from '../lib/stores/viewport.svelte';
</script>

{#if lightMode.available && !viewport.isShort}
  <button
    class="nav-light-btn"
    data-testid="nav-light-toggle"
    onclick={() => {
      ui.navDrawerOpen = false;
      void lightMode.set(!lightMode.active);
    }}
    title={lightMode.active ? 'Switch to Full app (show all modules)' : 'Switch to Light version (Threads, Connections, Git, API, Usage)'}
  >
    <Icon name={lightMode.active ? 'maximize' : 'minimize'} size={14} />
    <span class="grow">{lightMode.active ? 'Full app' : 'Light version'}</span>
  </button>
{/if}

<style>
  .nav-light-btn {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    height: 28px;
    padding: 0 8px;
    border: none;
    background: transparent;
    border-radius: var(--radius-s);
    color: var(--text);
    font-size: var(--fs-m);
    cursor: pointer;
    text-align: start;
    transition: background var(--dur-fast) ease-out;
  }
  .nav-light-btn:hover {
    background: color-mix(in srgb, var(--text-dim) 12%, transparent);
  }
  @media (pointer: coarse), (max-width: 640px) {
    .nav-light-btn {
      height: auto;
      min-block-size: 40px;
    }
  }
</style>
