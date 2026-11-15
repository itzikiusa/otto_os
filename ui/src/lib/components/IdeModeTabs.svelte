<script module lang="ts">
  export type IdeMode = 'project' | 'scratches';
  let pendingFocus: IdeMode | null = null;
</script>

<script lang="ts">
  // Unified IDE header switcher (Project · Scratches), rendered in PageHeader
  // `tabs()` across IdePage (`#/ide`) and WorkbenchPage (`#/workbench`).
  import { onMount, tick } from 'svelte';
  import { onTabKey as baseOnTabKey } from '../tabKeys';
  import { router } from '../router.svelte';

  const TABS: readonly { id: IdeMode; label: string; route: string }[] = [
    { id: 'project', label: 'Project', route: 'ide' },
    { id: 'scratches', label: 'Scratches', route: 'workbench' },
  ];

  interface Props {
    active: IdeMode;
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

  async function pick(item: (typeof TABS)[number]): Promise<void> {
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
      for (let i = 0; i < 60; i += 1) {
        await new Promise<void>((r) => requestAnimationFrame(() => r()));
        if (my !== seq) return;
        const tab =
          Array.from(
            document.querySelectorAll<HTMLButtonElement>(
              `[role="tablist"][aria-label="IDE mode"] [role="tab"][data-tab="${item.id}"]`,
            ),
          ).find((el) => el.offsetParent !== null && !el.closest('[inert]')) ?? null;
        if (!tab) continue;
        const cur = document.activeElement;
        const adrift =
          !cur || cur === document.body || !!cur.closest('[role="tablist"][aria-label="IDE mode"]');
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

<div
  bind:this={listEl}
  class="segmented"
  role="tablist"
  aria-label="IDE mode"
  data-testid="ide-mode-tabs"
  tabindex="-1"
  onkeydown={onTabKey}
>
  {#each TABS as item (item.id)}
    <button
      type="button"
      role="tab"
      data-tab={item.id}
      data-testid="ide-mode-{item.id}"
      class:active={active === item.id}
      aria-selected={active === item.id}
      tabindex={active === item.id ? 0 : -1}
      onclick={() => void pick(item)}
    >{item.label}</button>
  {/each}
</div>
