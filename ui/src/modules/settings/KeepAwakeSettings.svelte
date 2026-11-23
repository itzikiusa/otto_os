<script lang="ts">
  // Settings → Keep awake (desktop app only): its global shortcut, which
  // turns keep awake on indefinitely, or off (keep_awake.rs). The durations
  // themselves live in the menu bar and ⌘K.
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import { isTauri } from '../../lib/desktop';
  import { sectionLabel } from './sections';
  import SectionIntro from './SectionIntro.svelte';
  import ShortcutRow from './ShortcutRow.svelte';
</script>

<div class="settings-section">
  <PageHeader title={sectionLabel('keep-awake')} subtitle="Keep this Mac and its display awake" />
  <PageBody width="readable">
    <SectionIntro>
      Like caffeinate: idle sleep and display sleep wait while it is on. Pick a duration from the menu-bar icon or ⌘K,
      or use the shortcut to turn it on indefinitely and off again.
    </SectionIntro>
    <div class="card ka-card">
      {#if isTauri}
        <ShortcutRow id="keep-awake" desc="Turns keep awake on indefinitely, or off, from any app." />
      {:else}
        <span class="row-desc">Keep awake is available in the Otto desktop app.</span>
      {/if}
    </div>
  </PageBody>
</div>

<style>
  .settings-section {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .ka-card {
    max-width: var(--settings-col);
    padding: 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .row-desc {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
</style>
