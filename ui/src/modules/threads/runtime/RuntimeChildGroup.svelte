<!--
  One timeline row for a turn's subagents (T3 Code's V2SubagentGroup in
  TimelineV2EventRow.tsx, MIT, Copyright (c) 2026 T3 Tools Inc.): up to three
  stacked provider icons and "+N", "3 subagents", the tally in words
  ("2 working · 1 done"), elapsed, and a chevron that opens one row each —
  the thread a person or the agent delegated to (RuntimeChildCard) or the
  provider's own subagent (RuntimeSubagentCard). Each keeps its full answer
  expandable inline. One subagent is just its row.
-->
<script lang="ts">
  import type { TurnItem } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon, { hasProviderIcon } from '../../../lib/components/ProviderIcon.svelte';
  import { now } from '../../../lib/stores/now.svelte';
  import RuntimeChildCard from './RuntimeChildCard.svelte';
  import RuntimeSubagentCard from './RuntimeSubagentCard.svelte';
  import { childState, elapsedMs, kidKey, nativeTone, subagentTally, type Kid } from './childThreads';
  import { runtimeThreads } from './runtimeThreads.svelte';
  import { runtimeViews } from './runtimeView.svelte';
  import { formatDuration } from './workLog';

  interface Props {
    kids: Kid[];
    /** The thread whose timeline this is (its provider runs its own subagents). */
    sessionId?: string;
    activity?: Map<string, TurnItem[]>;
    output?: Record<string, string>;
  }
  let { kids, sessionId = '', activity = new Map(), output = {} }: Props = $props();

  const EMPTY: TurnItem[] = [];
  const own = $derived(runtimeThreads.get(sessionId)?.provider ?? null);
  const providerOf = (k: Kid): string | null => (k.kind === 'thread' ? (runtimeThreads.get(k.id)?.provider ?? k.hint?.provider ?? null) : own);
  const tones = $derived(kids.map((k) => (k.kind === 'thread' ? childState(runtimeThreads.get(k.id), runtimeViews.projections[k.id] ?? null).tone : nativeTone(k.item))));
  const tally = $derived(subagentTally(tones));
  const icons = $derived(kids.slice(0, 3).map(providerOf));
  const elapsed = $derived.by(() => {
    const t = now();
    let max = -1;
    for (const k of kids) {
      const ms =
        k.kind === 'thread'
          ? elapsedMs(runtimeViews.projections[k.id] ?? null, t)
          : k.item.started_at
            ? (k.item.status === 'running' ? t : Date.parse(k.item.completed_at ?? k.item.updated_at)) - Date.parse(k.item.started_at)
            : null;
      if (ms !== null && Number.isFinite(ms)) max = Math.max(max, ms);
    }
    return max >= 0 ? formatDuration(max) : null;
  });
  let opened = $state<boolean | null>(null);
  /** Open while one needs you, else folded until the person opens it. */
  const open = $derived(opened ?? tally.tone === 'ask');
  const settled = $derived(tally.tone === 'done' || tally.tone === 'failed');
</script>

{#snippet row(k: Kid)}
  {#if k.kind === 'thread'}
    <RuntimeChildCard id={k.id} hint={k.hint} />
  {:else}
    <RuntimeSubagentCard {sessionId} item={k.item} activity={activity.get(k.item.id) ?? EMPTY} {output} />
  {/if}
{/snippet}

{#if kids.length === 1}
  {@render row(kids[0])}
{:else if kids.length > 1}
  <section class="sg" class:settled={settled && !open} aria-label="{kids.length} subagents" data-testid="runtime-child-group">
    <button type="button" class="sg-head" aria-expanded={open} data-testid="runtime-child-group-toggle" onclick={() => (opened = !open)}>
      <span class="sg-icons" aria-hidden="true">
        {#each icons as p, i (i)}
          <span class="sg-icon">{#if p && hasProviderIcon(p)}<ProviderIcon provider={p} size={14} />{:else}<Icon name="assistant" size={14} />{/if}</span>
        {/each}
        {#if kids.length > 3}<span class="sg-more">+{kids.length - 3}</span>{/if}
      </span>
      <span class="sg-text">
        <span class="sg-title">{kids.length} subagents</span>
        <span class="sg-sub" data-tone={tally.tone} data-testid="runtime-child-group-summary">{tally.text}</span>
      </span>
      {#if elapsed}<span class="sg-time">{elapsed}</span>{/if}
      <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
    {#if open}
      <div class="sg-list">
        {#each kids as k (kidKey(k))}{@render row(k)}{/each}
      </div>
    {/if}
  </section>
{/if}

<style>
  .sg {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-inline-size: 0;
  }
  .sg.settled .sg-head {
    opacity: 0.7;
  }
  .sg-head {
    display: flex;
    align-items: center;
    gap: 10px;
    min-block-size: 40px;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    text-align: start;
    cursor: pointer;
  }
  .sg-head:hover {
    background: var(--hover);
  }
  .sg-head:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .sg-icons {
    display: inline-flex;
    align-items: center;
  }
  .sg-icon {
    display: inline-grid;
    place-items: center;
    inline-size: 22px;
    block-size: 22px;
    border: 2px solid var(--bg);
    border-radius: 50%;
    background: var(--surface-2);
  }
  .sg-icon + .sg-icon {
    margin-inline-start: -6px;
  }
  .sg-more {
    margin-inline-start: 4px;
    font-size: var(--fs-xs);
  }
  .sg-text {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-inline-size: 0;
  }
  .sg-title {
    color: var(--text);
    font-weight: 600;
  }
  .sg-sub {
    font-size: var(--fs-s);
  }
  .sg-sub[data-tone='live'] {
    color: var(--accent-text);
  }
  .sg-sub[data-tone='ask'] {
    color: var(--warning);
  }
  .sg-sub[data-tone='failed'] {
    color: var(--danger);
  }
  .sg-time {
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .sg-list {
    display: flex;
    flex-direction: column;
    padding-inline: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
  }
  /* Rows separated by hairlines inside the one container. */
  .sg-list > :global(* + *) {
    border-block-start: 1px solid var(--border);
  }
  @media (pointer: coarse) {
    .sg-head {
      min-block-size: 44px;
    }
  }
</style>
