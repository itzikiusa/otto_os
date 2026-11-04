<script lang="ts">
  // One work-log row of a structured runtime thread: kind icon · one-line
  // label · status at the end, updating in place as the item streams; opens
  // to its detail (a command's live output, a file change's diff, a tool's
  // input/output, the reasoning text, the task list).
  // Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
  // apps/web/src/components/chat/TimelineWorkEntryRow.tsx (row + expand),
  // WorkLog.tsx (the quiet line) and CommandOutputPanel.tsx (output that
  // follows the bottom while you do).
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import type { TurnItem } from '../../../lib/api/types';
  import InlineDiff from '../../agents/conversation/InlineDiff.svelte'; // ui-guards: allow — shared chat diff
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { lastLine, patchToDiff } from '../../agents/conversation/format'; // ui-guards: allow — shared chat helpers
  import { workLabel } from './timeline';

  interface Props {
    item: TurnItem;
    /** Live `command_output.delta` text (command items). */
    output?: string;
  }
  let { item, output = '' }: Props = $props();

  let open = $state(false);
  let outEl = $state<HTMLPreElement | null>(null);
  let follow = true;

  const ICONS: Partial<Record<TurnItem['type'], IconName>> = {
    reasoning: 'sparkle',
    command_execution: 'terminal',
    file_change: 'edit',
    web_search: 'globe',
    dynamic_tool: 'zap',
    todo_list: 'check',
    approval_request: 'shield',
    system_notice: 'info',
    error: 'warning',
  };
  const icon = $derived(ICONS[item.type] ?? 'dot');
  const running = $derived(item.status === 'running' || item.status === 'pending' || item.status === 'waiting');
  const failed = $derived(item.status === 'failed' || item.type === 'error');
  const stopped = $derived(item.status === 'cancelled' || item.status === 'interrupted');
  const label = $derived(workLabel(item));
  const commandOut = $derived(item.type === 'command_execution' ? (item.output ?? output) : '');
  const tail = $derived(running && commandOut ? lastLine(commandOut) : '');
  const patch = $derived.by(() => {
    if (item.type !== 'file_change') return null;
    if (item.diff_str) return item.diff_str;
    if (item.old_str == null && item.new_str == null) return null;
    const oldL = (item.old_str ?? '').split('\n');
    const newL = (item.new_str ?? '').split('\n');
    const p = item.file_name;
    return [`--- a/${p}`, `+++ b/${p}`, `@@ -1,${oldL.length} +1,${newL.length} @@`, ...oldL.map((l) => `-${l}`), ...newL.map((l) => `+${l}`)].join('\n');
  });
  const hasDetail = $derived(
    item.type === 'reasoning'
      ? !!item.text
      : item.type === 'command_execution' || item.type === 'dynamic_tool' || item.type === 'todo_list'
        ? true
        : item.type === 'file_change'
          ? patch !== null
          : item.type === 'web_search'
            ? (item.patterns?.length ?? 0) > 0
            : item.type === 'approval_request'
              ? !!item.reason
              : false,
  );

  // Pin the output to its bottom while the reader follows it.
  $effect(() => {
    void commandOut;
    if (outEl && follow) outEl.scrollTop = outEl.scrollHeight;
  });
  function onOutScroll(): void {
    if (outEl) follow = outEl.scrollHeight - outEl.scrollTop - outEl.clientHeight <= 8;
  }
  const json = (v: unknown): string => (typeof v === 'string' ? v : JSON.stringify(v, null, 2));
</script>

<div class="rw" class:failed class:stopped data-item-type={item.type} data-testid="runtime-item-{item.id}">
  {#if hasDetail}
    <button type="button" class="rw-line" aria-expanded={open} onclick={() => (open = !open)}>
      <Icon name={icon} size={13} />
      <span class="rw-label ellipsis" dir="auto">{label}</span>
      {#if running}<span class="spinner" aria-label="Running"></span>{:else if failed}<Icon name="x" size={12} />{/if}
      <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
  {:else}
    <div class="rw-line">
      <Icon name={icon} size={13} />
      <span class="rw-label wrap" dir="auto">{label}</span>
      {#if running}<span class="spinner" aria-label="Running"></span>{/if}
    </div>
  {/if}
  {#if item.type === 'error'}
    <p class="rw-error" dir="auto">{item.failure.message}</p>
  {/if}
  {#if tail && !open}
    <p class="rw-tail ellipsis mono" dir="auto">{tail}</p>
  {/if}
  {#if open}
    <div class="rw-detail">
      {#if item.type === 'command_execution'}
        <pre class="rw-pre" bind:this={outEl} onscroll={onOutScroll}><span class="rw-cmd">$ {item.input}</span>
{commandOut || (running ? '' : 'No output.')}</pre>
        {#if item.exit_code != null && item.exit_code !== 0}
          <p class="rw-error">Exit code {item.exit_code}</p>
        {/if}
      {:else if item.type === 'file_change' && patch}
        <InlineDiff diff={patchToDiff(patch, item.file_name)} />
      {:else if item.type === 'reasoning'}
        <div class="rw-reasoning"><Markdown md={item.text} small /></div>
      {:else if item.type === 'dynamic_tool'}
        <pre class="rw-pre">{json(item.input)}</pre>
        {#if item.output}<pre class="rw-pre">{item.output}</pre>{/if}
      {:else if item.type === 'web_search'}
        <ul class="rw-list">
          {#each item.patterns ?? [] as q (q)}<li dir="auto">{q}</li>{/each}
        </ul>
      {:else if item.type === 'todo_list'}
        <ul class="rw-list">
          {#each item.steps as s (s.id)}
            <li class="step-{s.status}" dir="auto">
              <Icon name={s.status === 'completed' ? 'check' : 'dot'} size={12} />{s.text}
            </li>
          {/each}
        </ul>
      {:else if item.type === 'approval_request' && item.reason}
        <p class="rw-note" dir="auto">{item.reason}</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .rw {
    min-width: 0;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rw-line {
    display: flex;
    align-items: center;
    gap: 8px;
    inline-size: 100%;
    min-width: 0;
    min-block-size: 28px;
    padding: 2px 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: inherit;
    font: inherit;
    text-align: start;
  }
  button.rw-line {
    cursor: pointer;
  }
  button.rw-line:hover {
    background: var(--hover);
    color: var(--text);
  }
  .rw-label {
    flex: 1;
    min-width: 0;
  }
  .rw-label.wrap {
    overflow-wrap: anywhere;
  }
  .rw.failed .rw-line {
    color: var(--danger);
  }
  .rw.stopped .rw-label {
    text-decoration: line-through;
    text-decoration-color: var(--text-dim);
  }
  .spinner {
    --spinner-size: 11px;
  }
  .rw-tail,
  .rw-error,
  .rw-note {
    margin: 0;
    padding-inline-start: 28px;
    font-size: var(--fs-xs);
  }
  .rw-tail {
    color: var(--text-dim);
  }
  .rw-error {
    color: var(--danger);
    overflow-wrap: anywhere;
  }
  .rw-detail {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin: 4px 0 6px;
    padding-inline-start: 28px;
    min-width: 0;
  }
  .rw-pre {
    margin: 0;
    max-block-size: 320px;
    overflow: auto;
    padding: 8px 10px;
    border-radius: var(--radius-m);
    background: var(--surface-2);
    color: var(--text);
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    line-height: 1.5;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
  .rw-cmd {
    color: var(--text-dim);
  }
  .rw-reasoning {
    color: var(--text-dim);
  }
  .rw-list {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .rw-list li {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow-wrap: anywhere;
  }
  .rw-list .step-completed {
    color: var(--success);
  }
  .rw-list .step-running {
    color: var(--text);
  }
</style>
