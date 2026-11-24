<script lang="ts">
  // One inbox section (patterns.md "Inbox rows, status words"): a calm
  // heading ("Pinned"), or a fold line — "Working 3 ›" / "History 12 ›" —
  // with the count and chevron at its end when `ontoggle` is given. The rows
  // (InboxRow) are the children; a page decides what a folded section still
  // shows (Threads keeps the focused row visible).
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  interface Props {
    label: string;
    /** Section landmark name (defaults to `label`). */
    ariaLabel?: string;
    count?: number;
    expanded?: boolean;
    ontoggle?: () => void;
    /** On the fold button. */
    testid?: string;
    children: Snippet;
  }

  let { label, ariaLabel, count, expanded = false, ontoggle, testid, children }: Props = $props();
</script>

<section class="if-group" aria-label={ariaLabel ?? label}>
  {#if ontoggle}
    <button type="button" class="if-fold" aria-expanded={expanded} data-testid={testid} onclick={ontoggle}>
      <span>{label}</span>
      {#if count}<span class="if-count">{count}</span>{/if}
      <Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
  {:else}
    <h2 class="if-label">{label}</h2>
  {/if}
  {@render children()}
</section>

<style>
  .if-group {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .if-label {
    margin: 0;
    padding: var(--sp-2) var(--sp-5);
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .if-fold {
    display: flex;
    flex-shrink: 0;
    align-items: center;
    gap: var(--sp-3);
    block-size: 32px;
    padding-inline: var(--sp-5);
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .if-fold:hover {
    background: var(--hover);
    color: var(--text);
  }
  .if-fold :global(svg) {
    flex-shrink: 0;
  }
  .if-count {
    margin-inline-start: auto;
    font-size: var(--fs-s);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  @media (hover: none), (pointer: coarse) {
    .if-fold {
      block-size: 44px;
    }
  }
</style>
