<script lang="ts">
  // The start of the page's top header row (PageHeader `.ph-leading`):
  // on a phone ☰ (open the Navigator drawer) or the list's ‹ Back; on desktop
  // and tablet "Show <pane>" while a list pane is hidden (design.md §7.1, §7.3).
  import Icon from '../Icon.svelte';
  import PhoneBack from './PhoneBack.svelte';
  import { ui } from '../../stores/ui.svelte';

  interface Props {
    /** This header is the phone's one top row. */
    phoneRow: boolean;
    back: { label: string; onclick: () => void } | null;
    show: { label: string; onclick: () => void } | null;
  }
  let { phoneRow, back, show }: Props = $props();
</script>

{#if back}
  <PhoneBack label={back.label} onclick={back.onclick} />
{:else if phoneRow}
  <button
    type="button"
    class="icon-btn ph-phone-btn"
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
