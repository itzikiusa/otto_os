<script lang="ts">
  // A run of tool activity inside one response, collapsed to ONE quiet line
  // that says what happened — "Ran 2 commands, edited retry.rs, read 3 files ·
  // 2m 7s" — with a status mark (spinner while the agent is still on it, ✓, or
  // "N failed"). Expanding lists the steps as a timeline of one-line rows
  // (ToolStep), subagent cards and plan snapshots. A group of exactly one call
  // renders that row directly. Thinking markers carry no text (the transcript
  // keeps only that it happened): in the timeline they are their own violet,
  // italic "Thought" rows, never confusable with a step or the answer.
  import { untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import ToolStep from './ToolStep.svelte';
  import SubagentCard from './SubagentCard.svelte';
  import TasksBlock from './TasksBlock.svelte';
  import { fmtDuration, stepSummary, type StepBlock } from './format';
  import type { Block } from '../../../lib/api/types';

  interface Props {
    steps: StepBlock[];
    /** Response duration (only shown on the response's first group). */
    durationMs?: number | null;
    /** The agent is working on this response right now: starts expanded. */
    live?: boolean;
    /** The session is alive and this is its newest response: a call without a
     *  result is in progress (or waiting on you), not abandoned. */
    active?: boolean;
    /** The agent is stopped waiting for you on this response's open call. */
    waiting?: boolean;
  }
  let { steps, durationMs = null, live = false, active = live, waiting = false }: Props = $props();

  const visible = $derived(steps.filter((s) => s.kind !== 'thinking'));
  const calls = $derived(steps.filter((s) => s.kind === 'tool_call' || s.kind === 'subagent').length);
  const thinking = $derived(steps.reduce((n, s) => (s.kind === 'thinking' ? n + s.count : n), 0));
  const failed = $derived(steps.filter((s) => s.kind === 'tool_call' && s.result != null && !s.result.ok).length);
  const running = $derived(active && steps.some((s) => (s.kind === 'tool_call' && s.result == null) || (s.kind === 'subagent' && s.status === 'running')));
  const summary = $derived(stepSummary(steps));
  // One-liners (a single tool call) don't need the group header.
  const single = $derived(visible.length === 1 && visible[0].kind === 'tool_call');
  // Initial-open only: a live response starts expanded, the reader owns it after.
  let open = $state(untrack(() => live));
  const dur = $derived(fmtDuration(durationMs));
  const tip = $derived(
    [`${calls} ${calls === 1 ? 'step' : 'steps'}`, dur && `took ${dur}`, thinking && `thought ${thinking}×`].filter(Boolean).join(' · '),
  );
</script>

{#if single}
  <div class="steps single" data-steps={calls}>
    <ToolStep block={visible[0] as Extract<Block, { kind: 'tool_call' }>} live={active} {waiting} />
  </div>
{:else if visible.length}
  <div class="steps" class:open data-steps={calls} data-failed={failed || undefined}>
    <button class="steps-head" onclick={() => (open = !open)} aria-expanded={open} title={tip}>
      <span class="steps-mark" class:running class:wait={running && waiting} class:bad={!running && failed > 0} aria-hidden="true">
        {#if running && waiting}<Icon name="warning" size={12} />
        {:else if running}<span class="spin"></span>
        {:else if failed}<Icon name="warning" size={12} />
        {:else}<Icon name="check" size={12} />{/if}
      </span>
      <span class="steps-title">{summary}</span>
      {#if failed}<span class="steps-fail">{failed} failed</span>{/if}
      <span class="steps-meta">{dur ? `${dur} · ` : ''}{calls} {calls === 1 ? 'step' : 'steps'}</span>
      <span class="steps-caret" aria-hidden="true"><Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} /></span>
    </button>
    {#if open}
      <div class="steps-list">
        {#each steps as s, i (s.kind === 'tool_call' ? s.id : s.kind === 'subagent' ? s.agent_id : `${s.kind}-${i}`)}
          {#if s.kind === 'thinking'}
            <div class="think-step" title="Reasoning — the text is not saved to the transcript">
              <span class="think-icon" aria-hidden="true"><Icon name="sparkle" size={12} /></span>
              <span>Thought{s.count > 1 ? ` ×${s.count}` : ''}</span>
            </div>
          {:else if s.kind === 'tool_call'}
            <ToolStep block={s} live={active} {waiting} />
          {:else if s.kind === 'subagent'}
            <SubagentCard agentId={s.agent_id} description={s.description} agentType={s.agent_type} status={s.status} />
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
    margin: 2px 0;
    min-width: 0;
  }
  .steps-head {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 4px 8px;
    background: none;
    border: 0;
    border-radius: var(--radius-s);
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
    text-align: start;
    min-width: 0;
  }
  .steps-head:hover {
    background: var(--hover);
    color: var(--text);
  }
  .steps-head:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .steps-mark {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 14px;
    flex-shrink: 0;
    color: var(--success);
  }
  .steps-mark.bad {
    color: var(--danger);
  }
  .steps-mark.wait {
    color: var(--warning);
  }
  .steps-title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text);
  }
  .steps-fail {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--danger);
    background: var(--danger-soft);
    border-radius: 99px;
    padding: 1px 7px;
  }
  .steps-meta {
    flex-shrink: 0;
    font-size: var(--fs-xs);
    white-space: nowrap;
  }
  .steps-caret {
    display: inline-flex;
    flex-shrink: 0;
  }
  .think-step {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 8px;
    font-size: var(--fs-s);
    font-style: italic;
    color: color-mix(in srgb, var(--cat-4) 70%, var(--text));
  }
  .think-icon {
    display: inline-flex;
  }
  /* The expanded steps hang off a hairline, like a timeline. */
  .steps-list {
    display: flex;
    flex-direction: column;
    margin-inline-start: 14px;
    padding-inline-start: 6px;
    border-inline-start: 1px solid var(--border);
    min-width: 0;
  }
  .spin {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 2px solid color-mix(in srgb, var(--accent) 25%, transparent);
    border-top-color: var(--accent);
  }
  @media (prefers-reduced-motion: no-preference) {
    .spin {
      animation: otto-spin 0.9s linear infinite;
    }
  }
  
  /* ≤360px: the step count / duration yield to the summary. */
  @container (max-width: 360px) {
    .steps-meta {
      display: none;
    }
  }
</style>
