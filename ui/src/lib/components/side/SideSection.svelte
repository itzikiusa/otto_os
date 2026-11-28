<script lang="ts">
  // A section of a list pane (design.md §3): a 22 px small-caps head with a
  // count that folds its rows (→ / ← too, through use:listNav). `open` is
  // bindable; a page that remembers folds stores it itself.
  //
  //   <SideSection title="Needs you" count={needs.length} bind:open={needsOpen}>
  //     {#each needs as t (t.id)}<SideRow … />{/each}
  //   </SideSection>
  import type { Snippet } from 'svelte';
  import Icon from '../Icon.svelte';

  interface Props {
    title: string;
    count?: number | null;
    open?: boolean;
    /** false: a plain heading that never folds. */
    foldable?: boolean;
    /** Trailing controls on the head (a + for "New in this section"). */
    actions?: Snippet;
    testid?: string;
    children: Snippet;
  }
  let { title, count = null, open = $bindable(true), foldable = true, actions, testid, children }: Props = $props();
</script>

<section class="side-section" aria-label={title} data-testid={testid}>
  <div class="side-section-head" class:folded={!open}>
    {#if foldable}
      <button type="button" class="ssh-btn" data-side-row data-side-toggle aria-expanded={open} onclick={() => (open = !open)}>
        <span class="ssh-title">{title}</span>
        {#if count !== null}<span class="ssh-count">{count}</span>{/if}
        <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
      </button>
    {:else}
      <h3 class="ssh-title">{title}</h3>
      {#if count !== null}<span class="ssh-count">{count}</span>{/if}
    {/if}
    {@render actions?.()}
  </div>
  {#if open}<div class="side-section-body">{@render children()}</div>{/if}
</section>
