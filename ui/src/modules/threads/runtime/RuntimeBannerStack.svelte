<script lang="ts">
  // The notices above the composer, in one stack with one order (error > UI
  // control > plan > queue > notices): one quiet notice shows in full (an
  // urgent one always does), the rest fold into "2 more: plan, queued message";
  // on a short screen or with the keyboard up all quiet ones fold. What the agent waits on
  // the person for (approval, question, secret) is the attention dock below
  // this stack, never folded.
  // T3 Code's ComposerBannerStack (docs/internals/web-ui.md).
  import type { Snippet } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { splitBanners, type BannerBlock } from './banners';
  import { keyboard } from '../../../lib/stores/keyboard.svelte';
  import { viewport } from '../../../lib/stores/viewport.svelte';

  interface Props {
    /** In priority order, highest first. */
    blocks: BannerBlock[];
    block: Snippet<[string]>;
  }
  let { blocks, block }: Props = $props();
  let expanded = $state(false);
  // A phone on its side, or the on-screen keyboard up, leaves a few lines
  // of conversation: every quiet notice folds into the one "N more" line.
  const split = $derived(splitBanners(blocks, viewport.isShort || keyboard.open ? 0 : undefined));
  const shown = $derived(expanded ? blocks : split.shown);
  // A new set of notices starts folded again.
  $effect(() => {
    if (split.folded.length === 0) expanded = false;
  });
</script>

{#if blocks.length > 0}
  <div class="bs" data-testid="runtime-banners">
    {#each shown as b (b.key)}
      <div class="bs-item" data-banner={b.key}>{@render block(b.key)}</div>
    {/each}
    {#if split.folded.length > 0}
      <button
        type="button"
        class="bs-more"
        aria-expanded={expanded}
        data-testid="runtime-banners-more"
        onclick={() => (expanded = !expanded)}
      >
        <Icon name={expanded ? 'chevronDown' : 'chevronRight'} size={12} />
        <span class="ellipsis">{expanded ? 'Show fewer' : `${split.folded.length} more: ${split.folded.map((b) => b.label).join(', ')}`}</span>
      </button>
    {/if}
  </div>
{/if}

<style>
  .bs {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-inline-size: 0;
  }
  .bs-item {
    min-inline-size: 0;
  }
  .bs-more {
    display: flex;
    align-items: center;
    gap: 6px;
    min-inline-size: 0;
    min-block-size: 24px;
    padding: 2px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    text-align: start;
    cursor: pointer;
  }
  .bs-more:hover {
    background: var(--hover);
    color: var(--text);
  }
  .bs-more:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
</style>
