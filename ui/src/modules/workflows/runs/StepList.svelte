<script lang="ts">
  // The run's steps in execution order (ux.md §4.2): a parallel group is one
  // indented block, a loop's children are grouped by round (current round open
  // and highlighted, earlier rounds folded to one line each, reviewers and
  // summarizer nested under that round's review), a fan-out's reviewers are
  // named rows under their step, and every waiting-on-subagents / summarizer
  // wait names the threads it waits on as links into the pane. ↑/↓ move rows.
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import { scrollBehavior } from '../../../lib/motion';
  import { plural } from '../../../lib/plural';
  import type {
    GoalProgress,
    RunLimits,
    RunState,
    WorkflowCheckpoint,
    WorkflowGraph,
  } from '../../../lib/api/types';
  import {
    childWord,
    stepDuration,
    stepLine2,
    stepWord,
    summarizerWaitWord,
    waitingThreadLabel,
    type StepGlyph,
  } from './runWords';
  import {
    childNoun,
    doneFold,
    groupLoopRounds,
    isWaitingOnSubagents,
    loopPositionLine,
    orderChildTree,
    RESULT,
    timeBar,
    waitingReviewers,
    waitingThreads,
    type LoopRound,
    type LoopRoundStep,
    type NestedChildView,
    type StepChild,
    type StepEntry,
    type StepItem,
    type WaitingThreadRef,
  } from './stepOrder';

  interface Props {
    entries: readonly StepEntry[];
    runState: RunState;
    /** The run's clock: its start and its end (or now). */
    clock: { start: number; end: number };
    now: number;
    selected: { node: string; child: string | null };
    /** Show the Result row (the run ended). */
    result: boolean;
    /** Fold the done steps before the current one (phone, narrow tablet). */
    compact: boolean;
    onselect: (node: string, child: string | null) => void;
    /** Optional run context for round grouping and scroll-on-open. */
    runId?: string;
    graph?: WorkflowGraph;
    checkpoints?: readonly WorkflowCheckpoint[];
    goal?: GoalProgress | null;
    limits?: RunLimits | null;
  }
  let {
    entries,
    runState,
    clock,
    now,
    selected,
    result,
    compact,
    onselect,
    runId = '',
    graph,
    checkpoints = [],
    goal = null,
    limits = null,
  }: Props = $props();

  let foldOpen = $state(false);
  let openDone = $state<Record<string, boolean>>({});
  let openQueued = $state<Record<string, boolean>>({});
  let openRounds = $state<Record<string, boolean>>({});
  let openWaits = $state<Record<string, boolean>>({});
  let listEl = $state<HTMLElement | null>(null);
  let scrolledFor = '';

  const fold = $derived(compact && !foldOpen ? doneFold(entries) : null);
  const shown = $derived(fold ? entries.slice(fold.entries) : entries);
  const bar = (s: StepItem) => timeBar(s, clock);
  const graphById = $derived(new Map((graph?.nodes ?? []).map((n) => [n.id, n] as const)));

  // Scroll the live (or selected) row into view once when a run opens.
  $effect(() => {
    const id = runId || 'default';
    if (!listEl || entries.length === 0 || scrolledFor === id) return;
    const target =
      listEl.querySelector<HTMLElement>('[data-live-row="true"]') ??
      listEl.querySelector<HTMLElement>('[aria-current="true"]');
    if (!target) return;
    scrolledFor = id;
    requestAnimationFrame(() => {
      target.scrollIntoView({ block: 'nearest', behavior: scrollBehavior() });
    });
  });

  function roundGlyph(status: LoopRoundStep['status']): StepGlyph {
    switch (status) {
      case 'success': return 'done';
      case 'error': return 'failed';
      case 'running': return 'working';
      case 'needs_you': return 'needs';
      case 'skipped': return 'dash';
      default: return 'pending';
    }
  }

  function onkeydown(e: KeyboardEvent): void {
    if (e.key !== 'ArrowDown' && e.key !== 'ArrowUp') return;
    const list = (e.currentTarget as HTMLElement).closest('.steps');
    const rows = [...(list?.querySelectorAll<HTMLButtonElement>('button[data-nav]') ?? [])];
    const i = rows.indexOf(document.activeElement as HTMLButtonElement);
    const next = rows[i + (e.key === 'ArrowDown' ? 1 : -1)];
    if (next) {
      e.preventDefault();
      next.focus();
    }
  }
</script>

{#snippet glyph(g: StepGlyph)}
  {#if g === 'done'}<span class="g done"><Icon name="check" size={13} /></span>
  {:else if g === 'failed'}<span class="g failed"><Icon name="x" size={13} /></span>
  {:else if g === 'clock'}<span class="g dim"><Icon name="clock" size={13} /></span>
  {:else if g === 'working'}<span class="g"><span class="spinner"></span></span>
  {:else if g === 'needs'}<span class="g needs" aria-hidden="true">!</span>
  {:else if g === 'dash'}<span class="g dim" aria-hidden="true">–</span>
  {:else}<span class="g"><span class="hollow"></span></span>{/if}
{/snippet}

{#snippet waitingLinks(stepId: string, heading: string, list: readonly WaitingThreadRef[], testid: string, waitKey: string)}
  {#if list.length}
    {@const expanded = openWaits[waitKey] ?? false}
    {@const visible = expanded ? list : list.slice(0, 6)}
    <div class="wait-box" data-testid={testid}>
      <span class="wait-head">{heading}</span>
      <ul class="wait-list">
        {#each visible as w, idx (w.childId ?? `${w.label}-${idx}`)}
          {@const text = waitingThreadLabel(w, now)}
          <li>
            {#if w.childId}
              <button type="button" class="wait-link" data-testid="wf-waiting-thread" data-child-id={w.childId} onclick={() => onselect(stepId, w.childId)}>
                <ProviderIcon provider={w.provider} size={12} />
                <span class="ellipsis" dir="auto">{text}</span>
              </button>
            {:else}
              <span class="wait-static" data-testid="wf-waiting-thread">
                {#if w.provider}<ProviderIcon provider={w.provider} size={12} />{/if}
                <span class="ellipsis" dir="auto">{text}</span>
              </span>
            {/if}
          </li>
        {/each}
        {#if list.length > 6}
          <li>
            <button
              type="button"
              class="wait-more"
              data-testid="wf-waiting-more"
              aria-expanded={expanded}
              onclick={() => (openWaits[waitKey] = !expanded)}
            >
              {expanded ? 'Show fewer' : `+${list.length - 6} more`}
            </button>
          </li>
        {/if}
      </ul>
    </div>
  {/if}
{/snippet}

{#snippet childRow(
  stepId: string,
  c: StepChild,
  depth = 0,
  parentLabel: string | null = null,
  after?: string,
  isLive = false,
  summWaiting?: readonly WaitingThreadRef[],
)}
  {@const w = childWord(c, now)}
  {@const active = selected.node === stepId && (selected.child === c.id || (!!c.thread_id && selected.child === c.thread_id))}
  <li class="kid-item" style:--depth={depth}>
    <button
      type="button"
      class="child"
      class:on={active}
      class:live={isLive && !active}
      title={c.retried_reason || undefined}
      data-nav
      {onkeydown}
      data-testid="wf-step-child"
      data-child-id={c.id}
      data-thread-id={c.thread_id}
      data-depth={depth}
      data-status={c.status}
      data-retried={c.retried_attempt != null ? 'true' : undefined}
      data-live-row={isLive ? 'true' : undefined}
      aria-current={active ? 'true' : undefined}
      onclick={() => onselect(stepId, c.id)}
    >
      <ProviderIcon provider={c.provider} size={14} />
      <span class="main">
        <span class="l1">
          <span class="name ellipsis" dir="auto">{c.label}{#if depth > 0 && parentLabel}<span class="sr-only"> (sub-thread of {parentLabel})</span>{/if}</span>
          <span class="word {w.tone}">{after ?? w.word}</span>
        </span>
        {#if c.retried_reason}<span class="l2 ellipsis" dir="auto">{c.retried_reason}</span>{/if}
      </span>
    </button>
    {#if summWaiting?.length}
      {@render waitingLinks(stepId, `Waiting on ${plural(summWaiting.length, 'reviewer')}:`, summWaiting, 'wf-summarizer-waiting', `${stepId}:summ:${c.id}`)}
    {/if}
  </li>
{/snippet}

{#snippet childTreeList(
  stepId: string,
  foldKey: string,
  kind: string,
  v: NestedChildView,
  allChildren: readonly StepChild[],
  stepErrored: boolean,
  stepLive: boolean,
  extraClass = '',
)}
  {@const doneOpen = openDone[foldKey] ?? stepErrored}
  {@const queuedOpen = openQueued[foldKey] ?? v.queued.length <= 3}
  <ol class="kids {extraClass}">
    {#each v.active as e (e.child.id)}
      {@const liveChild = stepLive && (e.child.status === 'running' || e.child.status === 'needs_you')}
      {@render childRow(stepId, e.child, e.depth, e.parentLabel, undefined, liveChild)}
    {/each}
    {#if v.queued.length}
      {#if queuedOpen}
        {#each v.queued as e (e.child.id)}{@render childRow(stepId, e.child, e.depth, e.parentLabel)}{/each}
      {:else}
        <li><button type="button" class="kfold" data-nav {onkeydown} aria-expanded="false" onclick={() => (openQueued[foldKey] = true)}><Icon name="chevronRight" size={12} />{plural(v.queued.length, childNoun(kind, v.queued[0].child))} queued</button></li>
      {/if}
    {/if}
    {#if v.done.length}
      <li>
        <button type="button" class="kfold" data-nav {onkeydown} aria-expanded={doneOpen} onclick={() => (openDone[foldKey] = !doneOpen)}>
          <Icon name={doneOpen ? 'chevronDown' : 'chevronRight'} size={12} />{plural(v.done.length, childNoun(kind, v.done[0].child))} done{v.findings ? ` · ${plural(v.findings, 'finding')}` : ''}
        </button>
      </li>
      {#if doneOpen}{#each v.done as e (e.child.id)}{@render childRow(stepId, e.child, e.depth, e.parentLabel)}{/each}{/if}
    {/if}
    {#each v.trailing as e (e.child.id)}
      {@const revWait = waitingReviewers(allChildren, e.child.id)}
      {@const summPending = e.child.status === 'queued' || e.child.status === 'pending'}
      {@const summAfter = summPending ? summarizerWaitWord(revWait.running.length, revWait.queued) : undefined}
      {@const liveSumm = stepLive && (e.child.status === 'running' || e.child.status === 'needs_you')}
      {@render childRow(stepId, e.child, e.depth, e.parentLabel, summAfter, liveSumm, summPending ? revWait.running : undefined)}
    {/each}
  </ol>
{/snippet}

{#snippet roundStepRow(s: StepItem, r: LoopRound, rs: LoopRoundStep)}
  {@const singleDirect = rs.kind !== 'review_run' && rs.allChildren.length === 1 && rs.allChildren[0].retried_attempt == null}
  {@const singleChild = singleDirect ? rs.allChildren[0] : null}
  {@const on = singleChild ? (selected.node === s.id && (selected.child === singleChild.id || (!!singleChild.thread_id && selected.child === singleChild.thread_id))) : false}
  {@const isLive = s.node.status === 'running' && r.current && (rs.status === 'running' || rs.status === 'needs_you')}
  {@const hasRunningNested = !singleDirect && rs.allChildren.some((c) => c.status === 'running' || c.status === 'needs_you')}
  {@const rowLive = isLive && !hasRunningNested}
  {@const word = singleChild ? childWord(singleChild, now) : { word: rs.summary || (rs.status === 'running' ? 'Working' : rs.status === 'success' ? 'Done' : rs.status === 'error' ? 'Failed' : rs.status === 'needs_you' ? 'Needs you' : rs.status === 'skipped' ? 'Skipped' : ''), tone: rs.status === 'error' ? 'danger' : rs.status === 'needs_you' ? 'warning' : '' }}
  <li class="rstep-item" data-testid="wf-round-step" data-round={r.iteration} data-step-kind={rs.kind} data-status={rs.status}>
    <button
      type="button"
      class="rstep"
      class:on
      class:live={rowLive && !on}
      data-nav
      {onkeydown}
      data-live-row={rowLive ? 'true' : undefined}
      aria-current={on ? 'true' : undefined}
      onclick={() => onselect(s.id, rs.childId)}
    >
      {@render glyph(roundGlyph(rs.status))}
      <span class="main">
        <span class="l1">
          <span class="name ellipsis" class:dim={rs.status === 'pending' || rs.status === 'skipped'} dir="auto">{rs.name}</span>
          {#if word.word}<span class="word {word.tone}">{word.word}</span>{/if}
        </span>
      </span>
    </button>
    {#if !singleDirect && rs.allChildren.length > 0}
      {@render childTreeList(s.id, rs.key, rs.kind, rs.children, rs.allChildren, rs.status === 'error', isLive, 'round-kids')}
    {/if}
  </li>
{/snippet}

{#snippet loopRounds(s: StepItem, view: NonNullable<ReturnType<typeof groupLoopRounds>>)}
  <ol class="rounds" data-testid="wf-loop-rounds" data-loop-id={s.id}>
    {#each view.rounds as r (r.iteration)}
      {@const rKey = `${s.id}:${r.iteration}`}
      {@const isOpen = openRounds[rKey] ?? r.current}
      {@const roundTitle = r.maxIterations && r.current ? `Round ${r.iteration} of ${r.maxIterations}` : `Round ${r.iteration}`}
      <li class="round" class:current={r.current} data-testid="wf-loop-round" data-round={r.iteration} data-current={r.current ? 'true' : undefined}>
        <button
          type="button"
          class="round-head"
          data-nav
          {onkeydown}
          data-testid="wf-loop-round-toggle"
          aria-expanded={isOpen}
          onclick={() => (openRounds[rKey] = !isOpen)}
        >
          <Icon name={isOpen ? 'chevronDown' : 'chevronRight'} size={12} />
          <span class="rtitle">{roundTitle}</span>
          {#if !isOpen && r.summary}
            <span class="rsummary ellipsis" dir="auto">— {r.summary}</span>
          {:else if isOpen && r.scoreText}
            <span class="rscore">{r.scoreText}</span>
          {/if}
        </button>
        {#if isOpen}
          <ol class="rsteps">
            {#each r.steps as rs (rs.key)}{@render roundStepRow(s, r, rs)}{/each}
          </ol>
        {/if}
      </li>
    {/each}
    {#if view.ungrouped.length}
      <li>
        {@render childTreeList(s.id, `${s.id}:other`, s.kind, orderChildTree(view.ungrouped), view.ungrouped, s.node.status === 'error', s.node.status === 'running')}
      </li>
    {/if}
  </ol>
{/snippet}

{#snippet stepRow(s: StepItem)}
  {@const w = stepWord(s.node, { now, run: runState, kind: s.kind })}
  {@const rounds = s.kind === 'loop' ? groupLoopRounds(s, { graphNode: graphById.get(s.id), checkpoints, goal, limits: limits?.loops[s.id] ?? null }) : null}
  {@const posLine = rounds ? loopPositionLine(s, rounds) : ''}
  {@const rawLine = stepLine2(s.node)}
  {@const line = posLine || rawLine}
  {@const waiting = isWaitingOnSubagents(s.node) ? waitingThreads(s.node, { excludeSummarizer: s.kind === 'review_run' }) : []}
  {@const b = bar(s)}
  {@const on = selected.node === s.id && !selected.child}
  {@const stepLive = s.node.status === 'running' && !(s.node.children?.some((c) => c.status === 'running' || c.status === 'needs_you')) && !(rounds?.rounds.some((r) => r.current && r.steps.some((rs) => rs.status === 'running' || rs.status === 'needs_you')))}
  <button
    type="button"
    class="step"
    class:on
    class:live={stepLive && !on}
    class:failed={s.node.status === 'error'}
    data-nav
    {onkeydown}
    data-testid="wf-step"
    data-node-id={s.id}
    data-status={s.node.status}
    data-phase={s.node.phase}
    data-step-no={s.node.step_no}
    data-live-row={stepLive ? 'true' : undefined}
    aria-current={on ? 'true' : undefined}
    onclick={() => onselect(s.id, null)}
  >
    {@render glyph(w.glyph)}
    <span class="main">
      <span class="l1"><span class="name ellipsis" class:dim={w.glyph === 'dash' || w.glyph === 'pending'} dir="auto">{s.name}</span><span class="word {w.tone}">{w.word}</span></span>
      {#if line}<span class="l2" data-testid={posLine ? 'wf-loop-position' : undefined} dir="auto">{line}</span>{/if}
      {#if b}<span class="bar" aria-hidden="true"><span class="fill {s.node.status} {s.node.phase ?? ''}" style:inset-inline-start="{b.at}%" style:inline-size="{b.w}%"></span></span>{/if}
    </span>
  </button>
  {#if waiting.length}
    <div class="step-wait">
      {@render waitingLinks(s.id, `Waiting on ${plural(waiting.length, 'thread')}:`, waiting, 'wf-step-waiting', `${s.id}:step`)}
    </div>
  {/if}
  {#if rounds}
    {@render loopRounds(s, rounds)}
  {:else if s.node.children?.length}
    {@render childTreeList(s.id, s.id, s.kind, orderChildTree((s.node.children ?? []) as StepChild[]), (s.node.children ?? []) as StepChild[], s.node.status === 'error', s.node.status === 'running')}
  {/if}
{/snippet}

<div class="head"><span>Steps</span><span>Time</span></div>
<ol class="steps" bind:this={listEl}>
  {#if result}
    <li>
      <button type="button" class="step result" class:on={selected.node === RESULT} data-nav {onkeydown} data-testid="wf-step" data-node-id={RESULT} aria-current={selected.node === RESULT ? 'true' : undefined} onclick={() => onselect(RESULT, null)}>
        <span class="g"><Icon name="file" size={13} /></span>
        <span class="main"><span class="l1"><span class="name">Result</span><span class="word">final-output.md</span></span></span>
      </button>
    </li>
  {/if}
  {#if fold}
    <li>
      <button type="button" class="step" data-nav {onkeydown} data-testid="wf-step-fold" data-count={fold.done + fold.skipped} aria-expanded="false" onclick={() => (foldOpen = true)}>
        <span class="g done"><Icon name="check" size={13} /></span>
        <span class="main"><span class="l1"><span class="name">{plural(fold.done, 'step')} done{fold.skipped ? ` · ${fold.skipped} skipped` : ''} · {stepDuration(fold.ms)}</span><Icon name="chevronRight" size={12} /></span></span>
      </button>
    </li>
  {/if}
  {#each shown as e, i (e.type === 'step' ? e.step.id : `g${i}-${e.steps[0].id}`)}
    {#if e.type === 'group'}
      <li class="group" data-testid="wf-step-group">
        <span class="glabel">In parallel</span>
        <ol>{#each e.steps as s (s.id)}<li>{@render stepRow(s)}</li>{/each}</ol>
      </li>
    {:else}
      <li>{@render stepRow(e.step)}</li>
    {/if}
  {/each}
</ol>

<style>
  .head {
    display: flex;
    justify-content: space-between;
    padding-block: var(--sp-4);
    padding-inline: var(--sp-2);
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text-dim);
  }
  ol {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .steps {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
  }
  .group {
    margin-inline-start: var(--sp-3);
    padding-inline-start: var(--sp-4);
    border-inline-start: 2px solid var(--border-strong);
  }
  .glabel {
    display: block;
    padding-block: var(--sp-2);
    padding-inline: var(--sp-4);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .step,
  .child,
  .kfold,
  .round-head,
  .rstep {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-4);
    inline-size: 100%;
    padding-block: var(--sp-3);
    padding-inline: var(--sp-4);
    border: 0;
    border-radius: var(--radius-m);
    background: transparent;
    color: var(--text);
    font: inherit;
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .step {
    min-block-size: 40px;
  }
  .step:hover,
  .child:hover,
  .kfold:hover,
  .round-head:hover,
  .rstep:hover {
    background: var(--hover);
  }
  .step.on,
  .child.on,
  .rstep.on {
    background: var(--accent-soft);
  }
  .step.live,
  .child.live,
  .rstep.live {
    background: var(--accent-soft);
    border-inline-start: 2px solid var(--accent);
  }
  .main {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    flex: 1;
    min-inline-size: 0;
  }
  .l1 {
    display: flex;
    align-items: center;
    gap: var(--sp-4);
    min-inline-size: 0;
  }
  .name {
    flex: 1;
    min-inline-size: 0;
  }
  .name.dim {
    color: var(--text-dim);
  }
  .word {
    flex: none;
    color: var(--text-dim);
    font-size: var(--fs-s);
    font-variant-numeric: tabular-nums;
  }
  .word.danger {
    color: var(--danger);
  }
  .word.warning {
    color: var(--warning);
  }
  .word.success {
    color: var(--success);
  }
  .l2 {
    font-size: var(--fs-s);
    color: var(--text-dim);
    overflow-wrap: anywhere;
  }
  .bar {
    position: relative;
    display: block;
    overflow: hidden;
    block-size: 3px;
    margin-block-start: var(--sp-1);
    border-radius: var(--radius-s);
    background: var(--surface-2);
  }
  .fill {
    position: absolute;
    inset-block: 0;
    border-radius: var(--radius-s);
    background: var(--text-dim);
  }
  .fill.running {
    background: var(--accent);
  }
  .fill.needs_you {
    background: var(--warning);
  }
  .fill.error {
    background: var(--danger);
  }
  .g {
    flex: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    inline-size: 16px;
    block-size: 18px;
    color: var(--text-dim);
    font-weight: 600;
  }
  .g.done {
    color: var(--success);
  }
  .g.failed {
    color: var(--danger);
  }
  .g.needs {
    color: var(--warning);
  }
  .hollow {
    inline-size: 12px;
    block-size: 12px;
    border: 1.5px solid var(--border-strong);
    border-radius: 50%;
  }
  .kids {
    margin-inline-start: var(--sp-8);
  }
  .kids.round-kids {
    margin-inline-start: var(--sp-6);
  }
  .kid-item {
    padding-inline-start: calc(var(--depth, 0) * var(--sp-6));
  }
  .child,
  .kfold,
  .round-head,
  .rstep {
    align-items: center;
    min-block-size: 30px;
    padding-block: var(--sp-2);
  }
  .kfold {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .rounds {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    margin-inline-start: var(--sp-6);
    padding-block-end: var(--sp-2);
  }
  .round {
    border-inline-start: 2px solid var(--border);
    padding-inline-start: var(--sp-2);
  }
  .round.current {
    border-inline-start-color: var(--accent-line);
    background: var(--accent-faint);
    border-radius: var(--radius-m);
    padding-block-end: var(--sp-1);
  }
  .round-head {
    gap: var(--sp-2);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .round.current .round-head {
    color: var(--text);
  }
  .rtitle {
    flex: none;
    font-weight: 600;
  }
  .rsummary {
    flex: 1;
    min-inline-size: 0;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .rscore {
    margin-inline-start: auto;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .rsteps {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding-inline-start: var(--sp-2);
  }
  .step-wait {
    margin-inline-start: var(--sp-8);
    padding-inline: var(--sp-4);
    padding-block-end: var(--sp-2);
  }
  .wait-box {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    padding-block: var(--sp-1);
    padding-inline-start: var(--sp-4);
  }
  .step-wait .wait-box {
    padding-inline-start: 0;
  }
  .wait-head {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .wait-list {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-1) var(--sp-2);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .wait-link,
  .wait-static {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    max-inline-size: 100%;
    padding-block: var(--sp-1);
    padding-inline: var(--sp-2);
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    text-align: start;
  }
  .wait-link {
    cursor: pointer;
  }
  .wait-link:hover {
    background: var(--hover);
    border-color: var(--accent-line);
  }
  .wait-static {
    color: var(--text-dim);
  }
  .wait-more {
    display: inline-flex;
    align-items: center;
    padding-block: var(--sp-1);
    padding-inline: var(--sp-2);
    border: 1px dashed var(--border);
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text-dim);
    font: inherit;
    font-size: var(--fs-xs);
    cursor: pointer;
  }
  .wait-more:hover {
    background: var(--hover);
    color: var(--text);
  }
  @media (max-width: 640px) {
    .kids {
      margin-inline-start: var(--sp-5);
    }
    .kids.round-kids {
      margin-inline-start: var(--sp-4);
    }
    .rounds {
      margin-inline-start: var(--sp-3);
    }
    .step-wait {
      margin-inline-start: var(--sp-4);
    }
  }
  @media (pointer: coarse) {
    .step,
    .child,
    .kfold,
    .round-head,
    .rstep {
      min-block-size: 44px;
    }
    .wait-link,
    .wait-more {
      min-block-size: var(--hit-min);
    }
  }
</style>
