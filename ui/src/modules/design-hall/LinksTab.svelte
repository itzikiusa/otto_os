<script lang="ts">
  // The Links tab of an open design: its links, with "Find references" (the
  // library search that pins inspiration as `references` links) one click in.
  import type { Snippet } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';

  interface Props {
    /** The References finder is open instead of the link list. */
    refs: boolean;
    links: Snippet;
    references: Snippet;
  }
  let { refs = $bindable(), links, references }: Props = $props();
</script>

<div class="head">
  {#if refs}
    <button class="btn small ghost" onclick={() => (refs = false)} data-testid="design-refs-back"><Icon name="chevronLeft" size={12} /> Links</button>
    <strong>Find references</strong>
  {:else}
    <button class="btn small" onclick={() => (refs = true)} data-testid="design-find-references"><Icon name="search" size={12} /> Find references</button>
  {/if}
</div>
{#if refs}{@render references()}{:else}{@render links()}{/if}

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 14px 0;
    font-size: var(--fs-s);
  }
</style>
