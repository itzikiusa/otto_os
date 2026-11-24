<script lang="ts">
  // One work-log row of a structured runtime thread: kind icon · one-line
  // label · status at the end, updating in place as the item streams; opens
  // to its detail (a command's live output, a file change's diff, a tool's
  // input/output, the reasoning text, the task list).
  // Ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
  // apps/web/src/components/chat/TimelineWorkEntryRow.tsx (row + expand),
  // WorkLog.tsx (the quiet line) and CommandOutputPanel.tsx (output that
  // follows the bottom while you do). An approval that gated the call folds
  // into its row: Approve / Decline inline while pending, the outcome after
  // (ComposerPendingApprovalActions.tsx).
  import { untrack } from 'svelte';
  import Icon, { type IconName } from '../../../lib/components/Icon.svelte';
  import type { ApprovalDecision, TurnItem } from '../../../lib/api/types';
  import InlineDiff from '../../agents/conversation/InlineDiff.svelte'; // ui-guards: allow — shared chat diff
  import Markdown from '../../agents/conversation/Markdown.svelte'; // ui-guards: allow — shared chat markdown
  import { lastLine, patchToDiff } from '../../agents/conversation/format'; // ui-guards: allow — shared chat helpers
  import RuntimeApprovalRow from './RuntimeApprovalRow.svelte';
  import RuntimeRunNotice from './RuntimeRunNotice.svelte';
  import RuntimeChildNotice from './RuntimeChildNotice.svelte';
  import { threadNoticeOf } from './childThreads';
  import LazyMount from '../../../lib/components/LazyMount.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import { lazyComponent } from '../../../lib/lazy-component.svelte';
  const RuntimeToolImages = lazyComponent(() => import('./RuntimeToolImages.svelte'));
  import { errorText, openedTarget, runNoticeOf, toolError, type PendingApproval } from './timeline';
  import { exitText, formatDuration, itemDuration, relativeToCwd, rowLabel } from './workLog';
  import { now } from '../../../lib/stores/now.svelte';
  import { ws } from '../../../lib/stores/workspace.svelte';
  import { threads } from '../threadsStore.svelte';
  import { runtimeViews } from './runtimeView.svelte';

  interface Props {
    sessionId?: string;
    item: TurnItem;
    /** Live `command_output.delta` text (command items). */
    output?: string;
    /** The pending approval gating this call (answered inline). */
    pending?: PendingApproval;
    /** What became of its approval ("Withdrawn by Claude", "Denied"). */
    note?: string | null;
    onrespond?: (requestId: string, decision: ApprovalDecision) => Promise<void>;
    /** The personal agent whose chat this is: a run-result notice opens its report. */
    agentId?: string | null;
    /** The thread's folder: paths and `cd <it> &&` drop out of the label. */
    cwd?: string | null;
    /** Opened from the start (Agents & tasks' drill-in). */
    startOpen?: boolean;
  }
  let { sessionId = '', item, output = '', pending, note = null, onrespond, agentId = null, cwd = null, startOpen = false }: Props = $props();
  const runNotice = $derived(item.type === 'system_notice' ? runNoticeOf(item.message) : null);
  /** Threads opened from this one: their live cards / "finished" line. */
  const childNotice = $derived(item.type === 'system_notice' ? threadNoticeOf(item.message) : null);

  let open = $state(untrack(() => startOpen));
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
  const label = $derived(rowLabel(item, cwd));
  const liveReasoningElapsed = $derived.by(() => {
    if (!running || item.type !== 'reasoning' || !item.started_at) return null;
    const startMs = Date.parse(item.started_at);
    if (!Number.isFinite(startMs)) return null;
    const elapsed = Math.max(0, now() - startMs);
    return elapsed >= 1000 ? formatDuration(elapsed) : null;
  });
  // An error has no meaningful duration ("Provider error · 0ms" read as noise).
  const took = $derived(item.type === 'error' ? null : (itemDuration(item) ?? liveReasoningElapsed));
  function hasAtLeastNewlines(raw: string, minCount: number): boolean {
    let found = 0;
    for (let i = 0; i < raw.length; i++) {
      if (raw.charCodeAt(i) === 10 && ++found >= minCount) return true;
    }
    return false;
  }

  function previewReasoningText(raw: string): string {
    const MAX_CHARS = 520;
    const MAX_LINES = 6;
    let lines = 0;
    let cut = raw.length;
    for (let i = 0; i < raw.length; i++) {
      if (i >= MAX_CHARS) {
        cut = i;
        break;
      }
      if (raw.charCodeAt(i) === 10 && ++lines >= MAX_LINES) {
        cut = i;
        break;
      }
    }
    if (cut >= raw.length) return raw;
    let slice = raw.slice(0, cut);
    const fenceMatches = slice.match(/^(?:```|~~~)/gm);
    if (fenceMatches && fenceMatches.length % 2 === 1) {
      slice += `\n${fenceMatches[fenceMatches.length - 1]}`;
    }
    return slice;
  }

  /** Thinking reads inline (T3): its text under a quiet line, clamped until opened. */
  const thinking = $derived(item.type === 'reasoning' && !!item.text);
  const longThought = $derived(
    item.type === 'reasoning' && (Boolean(item.detail_omitted) || item.text.length > 240 || hasAtLeastNewlines(item.text, 3)),
  );
  const thinkingMd = $derived(
    item.type !== 'reasoning' ? '' : open || !longThought ? item.text : previewReasoningText(item.text),
  );
  /** The thread / session this call opened or spoke to — a one-click link. */
  const target = $derived(running ? null : openedTarget(item));
  function goTarget(e: MouseEvent): void {
    if (!target) return;
    if (target.kind === 'session') ws.navigateToSession(target.id);
    else threads.selectThread(target.id, { split: e.shiftKey });
  }
  const error = $derived.by(() => {
    const e = toolError(item);
    return e ? relativeToCwd(exitText(e), cwd) : null;
  });
  const commandOut = $derived(item.type === 'command_execution' ? (item.output ?? output) : '');
  const tail = $derived(running && commandOut ? lastLine(commandOut) : '');
  const detailLoading = $derived(Boolean(item.detail_omitted && sessionId && runtimeViews.loadingItem[`${sessionId}:${item.id}`]));
  const itemDetailError = $derived(sessionId ? (runtimeViews.itemError[`${sessionId}:${item.id}`] ?? null) : null);
  const hasFilePatch = $derived(
    item.type === 'file_change' && Boolean(item.detail_omitted || item.diff_str || item.old_str != null || item.new_str != null),
  );
  const patch = $derived.by(() => {
    if (!open || item.type !== 'file_change') return null;
    if (item.diff_str) return item.diff_str;
    if (item.old_str == null && item.new_str == null) return null;
    const oldL = (item.old_str ?? '').split('\n');
    const newL = (item.new_str ?? '').split('\n');
    const p = item.file_name;
    return [`--- a/${p}`, `+++ b/${p}`, `@@ -1,${oldL.length} +1,${newL.length} @@`, ...oldL.map((l) => `-${l}`), ...newL.map((l) => `+${l}`)].join('\n');
  });
  const hasDetail = $derived(
    error
      ? true
      : item.type === 'reasoning'
      ? false
      : item.type === 'command_execution' || item.type === 'dynamic_tool' || item.type === 'todo_list'
        ? true
        : item.type === 'file_change'
          ? hasFilePatch
          : item.type === 'web_search'
            ? (item.patterns?.length ?? 0) > 0
            : item.type === 'approval_request'
              ? !!item.reason
              : false,
  );

  $effect(() => {
    if (open && item.detail_omitted && sessionId) {
      void runtimeViews.ensureItemDetail(sessionId, item.id);
    }
  });

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

{#if runNotice}
  <RuntimeRunNotice notice={runNotice} {agentId} />
{:else if childNotice}
  <RuntimeChildNotice notice={childNotice} />
{:else}
<div class="rw" class:failed class:stopped={stopped && !note} data-item-type={item.type} data-testid="runtime-item-{item.id}">
  {#if hasDetail}
    <button type="button" class="rw-line" aria-expanded={open} onclick={() => (open = !open)}>
      <Icon name={icon} size={13} />
      <span class="rw-label ellipsis" dir="auto">{label}</span>
      {#if took}<span class="rw-took">{took}</span>{/if}
      {#if running}<span class="spinner" aria-label="Running"></span>{:else if failed}<Icon name="x" size={12} />{/if}
      <Icon name={open ? 'chevronDown' : 'chevronRight'} size={12} />
    </button>
  {:else}
    <div class="rw-line">
      <Icon name={icon} size={13} />
      <span class="rw-label wrap" dir="auto">{label}</span>
      {#if took}<span class="rw-took">{took}</span>{/if}
      {#if running}<span class="spinner" aria-label="Running"></span>{/if}
    </div>
  {/if}
  {#if thinking && item.type === 'reasoning'}
    <!-- Clamped to three lines until "More" (a long thought). -->
    <div class="rw-think" class:all={open || !longThought}><Markdown md={thinkingMd} small streaming={running} /></div>
    {#if longThought}
      <button type="button" class="rw-more" aria-expanded={open} onclick={() => (open = !open)}>{open ? 'Less' : 'More'}</button>
    {/if}
  {/if}
  {#if target}
    <button
      type="button"
      class="rw-open"
      onclick={goTarget}
      title={target.kind === 'thread' ? 'Open this thread (⇧-click: beside)' : 'Open this session'}
      data-testid="runtime-item-open"
    >
      <Icon name={target.kind === 'thread' ? 'comment' : 'terminal'} size={12} />
      {target.kind === 'thread' ? 'Open thread' : 'Open session'}
      <Icon name="chevronRight" size={12} />
    </button>
  {/if}
  {#if item.type === 'error'}
    <p class="rw-error" dir="auto">{errorText(item.failure.message)}</p>
  {/if}
  {#if note}
    <p class="rw-note" dir="auto" data-testid="runtime-approval-note">{note}</p>
  {/if}
  {#if pending && onrespond}
    <RuntimeApprovalRow approval={pending} {onrespond} inline />
  {/if}
  {#if error && !open}
    <!-- A tool that didn't work, in one calm plain-language line (the full
         text is a click away); the turn usually recovers on its own. -->
    <p class="rw-tail ellipsis" dir="auto" title={error}>{error}</p>
  {/if}
  {#if tail && !open}
    <p class="rw-tail ellipsis mono" dir="auto">{tail}</p>
  {/if}
  {#if open}
    <div class="rw-detail">
      {#if error}
        <pre class="rw-pre rw-failure" data-testid="runtime-item-error">{error}</pre>
      {/if}
      {#if item.detail_omitted}
        {#if itemDetailError && sessionId}
          <LoadState
            variant="compact"
            what="item details"
            loading={detailLoading}
            error={itemDetailError}
            empty
            onretry={() => void runtimeViews.ensureItemDetail(sessionId, item.id)}
          />
        {:else}
          <p class="rw-tail" data-testid="runtime-item-loading">{detailLoading ? 'Loading details…' : 'Details unavailable.'}</p>
        {/if}
      {:else if item.type === 'command_execution'}
        <pre class="rw-pre" bind:this={outEl} onscroll={onOutScroll}><span class="rw-cmd">$ {item.input}</span>
{commandOut || (running ? '' : 'No output.')}</pre>
        {#if item.exit_code != null && item.exit_code !== 0}
          <p class="rw-error">exit {item.exit_code}</p>
        {/if}
      {:else if item.type === 'file_change' && patch}
        <InlineDiff diff={patchToDiff(patch, item.file_name)} />
      {:else if item.type === 'dynamic_tool'}
        <pre class="rw-pre">{json(item.input)}</pre>
        {#if item.output}<pre class="rw-pre">{item.output}</pre>{/if}
        {#if item.images?.length || item.image_files?.length}
          <LazyMount lazy={RuntimeToolImages} what="the tool images" variant="compact" props={{ images: item.images ?? [], files: item.image_files ?? [], itemId: item.id }} />
        {/if}
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
{/if}

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
  .rw-open {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    margin-inline-start: 24px;
    padding: 2px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .rw-open:hover {
    background: var(--accent-soft);
  }
  .rw-open:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 1px;
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
  .rw-failure {
    color: var(--danger);
    user-select: text;
  }
  .rw-cmd {
    color: var(--text-dim);
  }
  .rw-took {
    flex: none;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .rw-think {
    display: -webkit-box;
    -webkit-box-orient: vertical;
    -webkit-line-clamp: 3;
    line-clamp: 3;
    overflow: hidden;
    margin-inline-start: 28px;
    color: var(--text-dim);
  }
  .rw-think.all {
    display: block;
  }
  .rw-more {
    margin-inline-start: 28px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .rw-more:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: 2px;
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
