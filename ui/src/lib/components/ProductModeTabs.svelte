<script module lang="ts">
  export type ProductMode = 'stories' | 'design' | 'learnings';
  let pendingFocus: ProductMode | null = null;
</script>

<script lang="ts">
  // Shared unified Product mode switcher (Stories · Designs · Learnings),
  // rendered in PageHeader `tabs()` across ProductPage and Design Hall's Lobby.
  // Avoids importing the `product` store statically so Design Hall's chunk does
  // not pull Product's store into its static import closure.
  import { onMount, tick } from 'svelte';
  import { onTabKey as baseOnTabKey } from '../tabKeys';
  import { router } from '../router.svelte';
  import { auth } from '../stores/auth.svelte';

  interface Props {
    active: ProductMode;
    onselect?: (target: ProductMode) => Promise<void> | void;
  }
  let { active, onselect }: Props = $props();

  let listEl: HTMLDivElement | undefined = $state();
  let fromKeyboard = false;
  let seq = 0;

  const canProduct = $derived(auth.can('product', 'view'));
  const canDesign = $derived(auth.can('design', 'view'));

  const items = $derived.by(() => {
    const out: { id: ProductMode; label: string }[] = [];
    if (canProduct) out.push({ id: 'stories', label: 'Stories' });
    if (canDesign) out.push({ id: 'design', label: 'Designs' });
    if (canProduct) out.push({ id: 'learnings', label: 'Learnings' });
    return out;
  });

  function onTabKey(event: KeyboardEvent): void {
    fromKeyboard = true;
    baseOnTabKey(event, { activate: 'click' });
    queueMicrotask(() => {
      fromKeyboard = false;
    });
  }

  async function pick(target: ProductMode, event: MouseEvent): Promise<void> {
    const kbd = fromKeyboard;
    fromKeyboard = false;
    if (target === active) return;
    const list = (event.currentTarget as HTMLElement).closest<HTMLElement>('[role="tablist"]');
    const my = ++seq;
    if (kbd) pendingFocus = target;
    if (onselect) {
      await onselect(target);
    } else {
      const route = target === 'design' ? 'design' : target === 'learnings' ? 'product/learnings' : 'product';
      const ok = await router.goChecked(route);
      if (!ok && pendingFocus === target) pendingFocus = null;
    }
    await tick();
    await Promise.resolve();
    if (my !== seq || !list?.isConnected || list.closest('[inert]')) return;
    if (pendingFocus === target) pendingFocus = null;
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

{#if items.length > 1}
  <div bind:this={listEl} class="segmented m-view-toggle" role="tablist" aria-label="View" tabindex="-1" onkeydown={onTabKey}>
    {#each items as item (item.id)}
      <button
        type="button"
        role="tab"
        class:active={active === item.id}
        aria-selected={active === item.id}
        tabindex={active === item.id ? 0 : -1}
        onclick={(event) => void pick(item.id, event)}
      >{item.label}</button>
    {/each}
  </div>
{/if}

<style>
  @media (max-width: 640px) {
    .m-view-toggle > button {
      min-height: var(--hit-min);
      font-size: var(--fs-m);
      padding: 0 12px;
    }
  }
</style>
