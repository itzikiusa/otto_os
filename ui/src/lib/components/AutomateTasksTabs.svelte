<script module lang="ts">
  import type { AutomateTasksTab } from '../automateTasksTabs';
  export type { AutomateTasksTab };
  let pendingFocus: AutomateTasksTab | null = null;
</script>

<script lang="ts">
  // Shared 3-way segmented switcher (Tasks · Agents · Agent channels) rendered
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
    const list = (event.currentTarget as HTMLElement).closest<HTMLElement>('[role="tablist"]');
    const my = ++seq;
    if (kbd) pendingFocus = item.id;
    const ok = await router.goChecked(item.route);
    if (!ok && pendingFocus === item.id) pendingFocus = null;
    await tick();
    await Promise.resolve();
    if (my !== seq || !list?.isConnected || list.closest('[inert]')) return;
    if (pendingFocus === item.id) pendingFocus = null;
    if (kbd) {
      list.querySelector<HTMLButtonElement>('[role="tab"][aria-selected="true"]')?.focus({ preventScroll: true });
    }
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

<div bind:this={listEl} class="segmented" role="tablist" aria-label="Scheduled tasks view" tabindex="-1" onkeydown={onTabKey}>
  {#each AUTOMATE_TASKS_TABS as item (item.id)}
    <button
      type="button"
      role="tab"
      class:active={active === item.id}
      aria-selected={active === item.id}
      tabindex={active === item.id ? 0 : -1}
      onclick={(event) => void pick(item, event)}
    >{item.label}</button>
  {/each}
</div>
