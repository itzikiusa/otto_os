<script lang="ts">
  // Nested subagent card: header from the parent's `subagent` block (or a
  // `subagents[]` meta row for grandchildren), body fetched lazily via `?sub=`
  // on first expand and rendered with the same turn renderer. Children come
  // from `Transcript.subagents[]` (parent_agent_id), never from the body.
  import { getContext } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import TurnItem from './TurnItem.svelte';
  import SubagentCard from './SubagentCard.svelte';
  import { groupTurns } from './format';
  import type { SubagentMeta, SubagentStatus } from '../../../lib/api/types';
  import { CONV_CTX, type ConvContext } from './context';

  interface Props {
    agentId: string;
    description: string;
    agentType: string;
    status: SubagentStatus | null;
    depth?: number;
  }
  let { agentId, description, agentType, status, depth = 0 }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);

  let open = $state(false);
  const meta = $derived<SubagentMeta | null>(
    ctx.conv.transcript?.subagents.find((s) => s.agent_id === agentId) ?? null,
  );
  const children = $derived(
    (ctx.conv.transcript?.subagents ?? []).filter((s) => s.parent_agent_id === agentId),
  );
  const body = $derived(ctx.conv.subagents[agentId] ?? null);
  const items = $derived(groupTurns(body?.turns ?? []));

  function toggle(): void {
    open = !open;
    if (open) void ctx.conv.loadSubagent(agentId);
  }
</script>

<div class="sub" class:open style="--depth:{depth}" data-agent={agentId} data-status={status ?? ''}>
  <button class="sub-head" onclick={toggle} aria-expanded={open}>
    <span class="sub-icon"><Icon name="radar" size={13} /></span>
    <span class="sub-title">
      <span class="sub-type">{agentType || meta?.agent_type || 'agent'}</span>
      <span class="sub-desc">{description || meta?.description || agentId}</span>
    </span>
    {#if meta?.model}<span class="chip sub-model">{meta.model}</span>{/if}
    {#if children.length}<span class="chip sub-kids" title="{children.length} nested agents">{children.length} ⤵</span>{/if}
    <span class="sub-dot {status ?? 'unknown'}"></span>
    <span class="sub-caret">{open ? '▾' : '▸'}</span>
  </button>
  {#if open}
    <div class="sub-body">
      {#if body?.loading && !body.turns.length}
        <div class="dim sub-note">Loading subagent transcript…</div>
      {:else if body?.error}
        <div class="sub-note err">Could not load: {body.error}</div>
      {:else if !items.length}
        <div class="dim sub-note">No recorded turns.</div>
      {:else}
        {#if body?.has_earlier}
          <button class="btn small ghost" disabled={body.loading} onclick={() => ctx.conv.loadSubagentEarlier(agentId)}>
            {body.loading ? 'Loading…' : 'Load earlier'}
          </button>
        {/if}
        {#each items as item (item.id)}
          <TurnItem {item} nested />
        {/each}
      {/if}
      {#if children.length}
        <div class="sub-children">
          {#each children as c (c.agent_id)}
            <SubagentCard agentId={c.agent_id} description={c.description} agentType={c.agent_type} status={null} depth={depth + 1} />
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .sub {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: color-mix(in srgb, var(--accent) 4%, var(--surface));
    margin: 4px 10px;
    overflow: hidden;
  }
  .sub-head {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    background: none;
    border: 0;
    color: var(--text);
    cursor: pointer;
    text-align: start;
    font: inherit;
    min-width: 0;
  }
  .sub-head:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .sub-icon {
    display: inline-flex;
    color: var(--accent);
    flex-shrink: 0;
  }
  .sub-title {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 6px;
    align-items: baseline;
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
  }
  .sub-type {
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .sub-desc {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .sub-model,
  .sub-kids {
    height: 16px;
    font-size: 10px;
  }
  .sub-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--text-dim);
    flex-shrink: 0;
  }
  .sub-dot.done {
    background: var(--status-working, #3fb950);
  }
  .sub-dot.error {
    background: var(--status-exited, #e5534b);
  }
  .sub-dot.running {
    background: var(--status-warn, #febc2e);
  }
  .sub-caret {
    color: var(--text-dim);
    font-size: 11px;
  }
  .sub-body {
    border-top: 1px solid var(--border);
    padding: 8px 6px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-height: 60vh;
    overflow: auto;
  }
  .sub-note {
    font-size: 12px;
    padding: 2px 6px;
  }
  .sub-note.err {
    color: var(--status-exited, #e5534b);
  }
  .sub-children {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
</style>
