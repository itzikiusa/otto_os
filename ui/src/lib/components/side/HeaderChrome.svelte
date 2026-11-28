<script lang="ts">
  // The shell's controls inside the page's top header row on a compact screen
  // (design.md §7.1, decision 12: phone AND tablet have ONE header row):
  //   part="lead"   on a phone ☰ (the Navigator drawer) or the list's ‹ Back;
  //                 on desktop / tablet "Show <pane>" while a list pane is hidden
  //   part="trail"  the Side panel button, always the row's last control
  // History arrows and the bell stay in the Rail / Navigator (or the drawer).
  import Icon from '../Icon.svelte';
  import PhoneBack from './PhoneBack.svelte';
  import { ui } from '../../stores/ui.svelte';
  import { SIDE_PANEL } from '../../rightTabs';

  interface Props {
    part: 'lead' | 'trail';
    /** The Navigator is a drawer here (a phone): ☰ opens it. */
    drawer?: boolean;
    back?: { label: string; onclick: () => void } | null;
    show?: { label: string; onclick: () => void } | null;
  }
  let { part, drawer = false, back = null, show = null }: Props = $props();
</script>

{#if part === 'trail'}
  <button
    type="button"
    class="icon-btn hc-btn ph-side"
    class:on={ui.rightOpenNow}
    onclick={() => ui.toggleRight()}
    title={SIDE_PANEL}
    aria-label={SIDE_PANEL}
    aria-expanded={ui.rightOpenNow}
    data-testid="open-side-panel"
  >
    <Icon name="panel" size={20} />
  </button>
{:else}
  {#if back}
    <PhoneBack label={back.label} onclick={back.onclick} />
  {:else if drawer}
    <button
      type="button"
      class="icon-btn hc-btn"
      onclick={() => (ui.navDrawerOpen = !ui.navDrawerOpen)}
      title="Open navigator"
      aria-label="Open navigator"
      aria-haspopup="dialog"
      aria-expanded={ui.navDrawerOpen}
      data-testid="open-navigator"
    >
      <Icon name="sidebar" size={20} />
    </button>
  {/if}
  {#if show}
    <button type="button" class="icon-btn" onclick={show.onclick} title="Show {show.label} (⌘B)" aria-label="Show {show.label}" aria-keyshortcuts="Meta+B" data-testid="show-list-pane">
      <Icon name="sidebar" size={14} />
    </button>
  {/if}
{/if}

<style>
  /* 44 px targets, never taller than the row (38 px on a phone on its side). */
  .hc-btn {
    flex: none;
    inline-size: 38px;
    block-size: min(44px, var(--ph-h, 44px));
  }
  .hc-btn.on {
    color: var(--accent-text);
  }
</style>
