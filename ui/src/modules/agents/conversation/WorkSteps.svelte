<script lang="ts">
  // Consecutive tool-ish blocks of one response collapse into one row —
  // "Worked for 21m 17s · 38 steps" — that expands to the per-step list
  // (tool rows, subagent cards, the thinking marker, task-list snapshots).
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ToolStep from './ToolStep.svelte';
  import SubagentCard from './SubagentCard.svelte';
  import TasksBlock from './TasksBlock.svelte';
  import { fmtDuration } from './format';
  import type { Block } from '../../../lib/api/types';

  type Step = Extract<Block, { kind: 'tool_call' | 'subagent' | 'thinking' | 'tasks' }>;

  interface Props {
    steps: Step[];
    /** Response duration (only shown on the response's first group). */
    durationMs?: number | null;
    /** Start expanded (a live response that is still working). */
    live?: boolean;
  }
  let { steps, durationMs = null, live = false }: Props = $props();

  const calls = $derived(steps.filter((s) => s.kind === 'tool_call' || s.kind === 'subagent').length);
  const thinking = $derived(steps.reduce((n, s) => (s.kind === 'thinking' ? n + s.count : n), 0));
  const failed = $derived(steps.some((s) => s.kind === 'tool_call' && s.result != null && !s.result.ok));
  const pending = $derived(steps.some((s) => s.kind === 'tool_call' && s.result == null));
  // One-liners (a single tool call) don't need the group header.
  const single = $derived(calls === 1 && steps.length === 1 && steps[0].kind === 'tool_call');
  // Initial-open only: a live response starts expanded, the reader owns it after.
  let open = $state(untrack(() => live));
  const dur = $derived(fmtDuration(durationMs));
</script>

{#if single}
  <div class="steps single" data-steps={calls}>
    <ToolStep block={steps[0] as Extract<Block, { kind: 'tool_call' }>} />
  </div>
{:else}
  <div class="steps" class:open data-steps={calls}>
    <button class="steps-head" onclick={() => (open = !open)} aria-expanded={open}>
      <span class="steps-icon" class:pending><Icon name="zap" size={12} /></span>
      <span class="steps-title">
        {#if pending}Working{dur ? ` for ${dur}` : ''}…{:else}Worked{dur ? ` for ${dur}` : ''}{/if}
        <span class="dim"> · {calls} {calls === 1 ? 'step' : 'steps'}</span>
        {#if thinking}<span class="dim"> · Thought ({thinking})</span>{/if}
      </span>
      {#if failed}<span class="chip bad steps-fail">has failures</span>{/if}
      <span class="steps-caret">{open ? '▾' : '▸'}</span>
    </button>
    {#if open}
      <div class="steps-list">
        {#each steps as s, i (s.kind === 'tool_call' ? s.id : s.kind === 'subagent' ? s.agent_id : `${s.kind}-${i}`)}
          {#if s.kind === 'tool_call'}
            <ToolStep block={s} />
          {:else if s.kind === 'subagent'}
            <SubagentCard agentId={s.agent_id} description={s.description} agentType={s.agent_type} status={s.status} />
          {:else if s.kind === 'thinking'}
            <div class="thought dim" title="Thinking is not persisted in the transcript — only that it happened">
              Thought ({s.count})
            </div>
          {:else if s.kind === 'tasks'}
            <TasksBlock tasks={s.tasks} />
          {/if}
        {/each}
      </div>
    {/if}
  </div>
{/if}

<style>
  .steps {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    margin: 6px 0;
    overflow: hidden;
  }
  .steps-head {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 10px;
    background: none;
    border: 0;
    color: var(--text);
    font: inherit;
    font-size: 12.5px;
    cursor: pointer;
    text-align: start;
  }
  .steps-head:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .steps-icon {
    display: inline-flex;
    color: var(--text-dim);
  }
  .steps-icon.pending {
    color: var(--status-warn, #febc2e);
    animation: pulse 1.2s ease-in-out infinite;
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .steps-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .steps-fail {
    height: 16px;
    font-size: 10px;
  }
  .steps-caret {
    color: var(--text-dim);
    font-size: 11px;
  }
  .steps-list {
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
  }
  .thought {
    font-size: 12px;
    padding: 5px 10px 5px 31px;
    font-style: italic;
  }
</style>
