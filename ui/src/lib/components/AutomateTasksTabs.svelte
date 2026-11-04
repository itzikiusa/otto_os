<script module lang="ts">
  import type { AutomateTasksTab } from '../automateTasksTabs';
  export type { AutomateTasksTab };
  let pendingFocus: AutomateTasksTab | null = null;
</script>

<script lang="ts">
  // Shared 3-way segmented switcher (Agents · Scheduled tasks · Agent channels) rendered
  // in PageHeader `tabs()` across ScheduledTasksPage and PersonalAgentsPage.
  import { onMount, tick } from 'svelte';
  import { AUTOMATE_TASKS_TABS } from '../automateTasksTabs';
  import { onTabKey as baseOnTabKey } from '../tabKeys';
  import { router } from '../router.svelte';

  interface Props {
    active: AutomateTasksTab;
  }
  let { active }: Props = $props();

  let listEl: HTMLDivElement | undefined = $state();
  let fromKeyboard = false;
  let seq = 0;

  function onTabKey(event: KeyboardEvent): void {
    fromKeyboard = true;
    baseOnTabKey(event, { activate: 'click' });
    queueMicrotask(() => {
      fromKeyboard = false;
    });
  }

  async function pick(item: (typeof AUTOMATE_TASKS_TABS)[number], event: MouseEvent): Promise<void> {
    const kbd = fromKeyboard;
    fromKeyboard = false;
    if (item.id === active) return;
    const my = ++seq;
    if (kbd) pendingFocus = item.id;
    const ok = await router.goChecked(item.route);
    if (!ok && pendingFocus === item.id) pendingFocus = null;
    await tick();
    await Promise.resolve();
    if (my !== seq) return;
    if (kbd) {
      // The page may remount its header (a new tablist), lazy-load, or move
      // focus on arrival, and the route→`active` prop can land a frame late.
      // So for a few frames, focus the PICKED tab (by id, not [aria-selected])
      // in whichever tablist is live, until it holds focus.
      // ~1 s: a page can re-render its header once its data lands, which
      // drops focus to <body>; re-seat it then, but never steal focus the
      // user has moved elsewhere.
      for (let i = 0; i < 60; i += 1) {
        await new Promise<void>((r) => requestAnimationFrame(() => r()));
        if (my !== seq) return;
        // A kept-alive previous page can still hold the old (hidden) tablist,
        // so pick the visible one, not merely a connected one.
        const tab = Array.from(document.querySelectorAll<HTMLButtonElement>(`[role="tablist"][aria-label="Personal agents view"] [role="tab"][data-tab="${item.id}"]`))
          .find((el) => el.offsetParent !== null && !el.closest('[inert]')) ?? null;
        if (!tab) continue;
        const cur = document.activeElement;
        const adrift = !cur || cur === document.body || !!cur.closest('[role="tablist"][aria-label="Personal agents view"]');
        if (cur !== tab && adrift) tab.focus({ preventScroll: true });
        else if (cur !== tab) break;
      }
      if (pendingFocus === item.id) pendingFocus = null;
      return;
    }
    if (pendingFocus === item.id) pendingFocus = null;
  }

  onMount(() => {
    if (pendingFocus === active) {
      pendingFocus = null;
      void tick().then(() => {
        listEl?.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')?.focus({ preventScroll: true });
      });
    }
  });
</script>

<div bind:this={listEl} class="segmented" role="tablist" aria-label="Personal agents view" tabindex="-1" onkeydown={onTabKey}>
  {#each AUTOMATE_TASKS_TABS as item (item.id)}
    <button
      type="button"
      role="tab"
      data-tab={item.id}
      class:active={active === item.id}
      aria-selected={active === item.id}
      tabindex={active === item.id ? 0 : -1}
      onclick={(event) => void pick(item, event)}
    >{item.label}</button>
  {/each}
</div>
