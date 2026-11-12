<script lang="ts">
  // The header's "Related threads" chip: shown only while the thread has
  // related threads (or someone opened it from ⋯ › Related threads…). One
  // popover holds them all with their status and actions — T3 Code's
  // ThreadRelationshipsControl. A dot on the chip says one of them needs you.
  import Icon from '../../lib/components/Icon.svelte';
  import { collectRelated } from './relatedCollect';
  import { relatedPopover } from './relatedPopover.svelte';
  import RelatedThreadsList from './RelatedThreadsList.svelte';
  import ThreadPopover from './ThreadPopover.svelte';

  interface Props {
    threadId: string;
    onstartroom: (sessionId: string) => void;
  }
  let { threadId, onstartroom }: Props = $props();
  let btn = $state<HTMLButtonElement | null>(null);

  const rows = $derived(collectRelated(threadId));
  const open = $derived(relatedPopover.openFor === threadId);
  const asks = $derived(rows.some((r) => r.status === 'awaiting_permission' || r.status === 'needs_input'));
  const working = $derived(rows.filter((r) => r.status === 'working' || r.status === 'starting').length);
</script>

{#if rows.length > 0 || open}
  <button
    bind:this={btn}
    type="button"
    class="rc"
    class:on={open}
    aria-haspopup="dialog"
    aria-expanded={open}
    aria-label="Related threads: {rows.length}{working ? `, ${working} working` : ''}{asks ? ', one needs you' : ''}"
    title="Related threads"
    data-testid="threads-related-chip"
    onclick={() => (open ? relatedPopover.close() : relatedPopover.open(threadId))}
  >
    <Icon name="people" size={13} />
    <span>{rows.length}</span>
    {#if asks}<span class="rc-dot ask" aria-hidden="true"></span>{:else if working}<span class="rc-dot live" aria-hidden="true"></span>{/if}
  </button>
{/if}
{#if open && btn}
  <ThreadPopover anchor={btn} label="Related threads" testid="threads-related-popover" onclose={() => relatedPopover.close()}>
    <RelatedThreadsList {threadId} {onstartroom} ondone={() => relatedPopover.close()} />
  </ThreadPopover>
{/if}

<style>
  .rc {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    flex: none;
    block-size: 24px;
    padding-inline: 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    font-weight: 400;
    cursor: pointer;
  }
  .rc:hover,
  .rc.on {
    background: var(--hover);
    color: var(--text);
  }
  .rc:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
  }
  .rc-dot {
    inline-size: 6px;
    block-size: 6px;
    border-radius: 50%;
  }
  .rc-dot.live {
    background: var(--info);
  }
  .rc-dot.ask {
    background: var(--warning);
  }
  @media (pointer: coarse) {
    .rc {
      min-block-size: var(--hit-min);
    }
  }
</style>
