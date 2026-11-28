<script lang="ts">
  // Phone (or one on its side), a session pushed from the list (`#/agents/<id>`):
  // the pane header is the page's ONE header row (design.md §7.1) — no shell
  // bar above it. `lead` is the standard ‹ Sessions (pageNav.back, from the
  // page's ListDetail); `trail` claims the row (the shell bar steps aside)
  // and ends it with the Side panel button; `below` is the agent-driving
  // strip a PageHeader would carry under its row. Nothing on desktop / tablet.
  import AgentDrivingBar from '../../lib/components/AgentDrivingBar.svelte';
  import HeaderChrome from '../../lib/components/side/HeaderChrome.svelte';
  import PhoneBack from '../../lib/components/side/PhoneBack.svelte';
  import { pageNav } from '../../lib/components/side/pageNav.svelte';
  import { uiControl } from '../../lib/stores/uiControl.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';

  let { part, head = null }: { part: 'lead' | 'trail' | 'below'; head?: HTMLElement | null } = $props();
  const back = $derived(viewport.navDrawer ? pageNav.back : null);
  $effect(() => (part === 'trail' && head && back ? uiControl.claimBar(head) : undefined));
  const hosts = $derived(!!back && !!head && uiControl.barOwner === head);
</script>

{#if part === 'lead'}
  {#if back}<PhoneBack label={back.label} onclick={back.onclick} testid="agents-back-to-list" />{/if}
{:else if hosts}
  {#if part === 'trail'}<HeaderChrome part="trail" />{:else}<AgentDrivingBar />{/if}
{/if}
