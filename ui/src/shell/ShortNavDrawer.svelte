<script lang="ts">
  // The navigator drawer on a phone held sideways (viewport.isShort): the
  // Navigator plus a footer. That layout hides the status bar and has no
  // bottom nav, so the two things only those carried — the command palette
  // entry for touch and the machine status chip — live in the footer.
  // Everything else the phone's More sheet offers (Help, Settings, every
  // module) is already in the Navigator itself.
  import Icon from '../lib/components/Icon.svelte';
  import { ui } from '../lib/stores/ui.svelte';
  import MachineChip from './MachineChip.svelte';
  import Navigator from './Navigator.svelte';
</script>

<!-- The Navigator fills its height (100 %), so it gets a flex slot that leaves
     the footer on screen instead of pushing it out of the drawer. -->
<div class="sne-drawer">
  <div class="sne-nav"><Navigator /></div>
  <div class="sne" data-testid="short-nav-extras">
    <button
      type="button"
      class="sne-row"
      onclick={() => {
        ui.navDrawerOpen = false;
        ui.openPalette('commands');
      }}
    >
      <Icon name="command" size={16} />
      <span>Commands</span>
    </button>
    <div class="sne-machine">
      <span class="sne-label">Machine</span>
      <MachineChip />
    </div>
  </div>
</div>

<style>
  .sne-drawer { display: flex; flex-direction: column; block-size: 100%; min-block-size: 0; }
  .sne-nav { flex: 1; min-block-size: 0; display: flex; flex-direction: column; }
  .sne {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 6px 8px;
    border-block-start: 1px solid var(--border);
    flex-shrink: 0;
  }
  .sne-row {
    display: flex;
    align-items: center;
    gap: 10px;
    min-block-size: 44px;
    padding-inline: 10px;
    border: none;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-m);
    cursor: pointer;
    text-align: start;
  }
  .sne-row:hover {
    background: var(--hover);
  }
  .sne-machine {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    min-block-size: 44px;
    padding-inline: 10px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  /* Hidden until the machine status has loaded (MachineChip renders nothing). */
  .sne-machine:not(:has(:global(.machine-chip))) {
    display: none;
  }
</style>
