<script lang="ts">
  // The run's steps in execution order (ux.md §4.2): a parallel group is one
  // indented block, a fan-out's reviewers are named rows under their step
  // (working and failed first, the done ones folded with their findings, the
  // summarizer last), every row has a 3 px time bar on the run's own clock.
  // On a phone the done steps before the current one fold into one row. A
  // finished run leads with its Result. ↑/↓ move between rows.
  import Icon from '../../../lib/components/Icon.svelte';
  import ProviderIcon from '../../../lib/components/ProviderIcon.svelte';
  import { plural } from '../../../lib/plural';
  import type { RunState } from '../../../lib/api/types';
  import { childWord, stepDuration, stepLine2, stepWord, type StepGlyph } from './runWords';
  import { doneFold, orderChildren, RESULT, timeBar, type StepEntry, type StepItem } from './stepOrder';

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
  }
  let { entries, runState, clock, now, selected, result, compact, onselect }: Props = $props();

  let foldOpen = $state(false);
  let openDone = $state<Record<string, boolean>>({});
  let openQueued = $state<Record<string, boolean>>({});
  const fold = $derived(compact && !foldOpen ? doneFold(entries) : null);
  const shown = $derived(fold ? entries.slice(fold.entries) : entries);
  const bar = (s: StepItem) => timeBar(s, clock);

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

{#snippet childRow(stepId: string, c: import('../../../lib/api/types').ChildThread, after?: string)}
  {@const w = childWord(c, now)}
  <li>
    <button type="button" class="child" class:on={selected.node === stepId && selected.child === c.id} data-nav {onkeydown} data-testid="wf-step-child" data-child-id={c.id} data-status={c.status} aria-current={selected.node === stepId && selected.child === c.id ? 'true' : undefined} onclick={() => onselect(stepId, c.id)}>
      <ProviderIcon provider={c.provider} size={14} />
      <span class="name ellipsis" dir="auto">{c.label}</span>
      <span class="word {w.tone}">{after ?? w.word}</span>
    </button>
  </li>
{/snippet}

{#snippet children(s: StepItem)}
  {@const v = orderChildren(s.node.children ?? [])}
  {@const doneOpen = openDone[s.id] ?? s.node.status === 'error'}
  {@const queuedOpen = openQueued[s.id] ?? v.queued.length <= 3}
  <ol class="kids">
    {#each v.active as c (c.id)}{@render childRow(s.id, c)}{/each}
    {#if v.queued.length}
      {#if queuedOpen}
        {#each v.queued as c (c.id)}{@render childRow(s.id, c)}{/each}
      {:else}
        <li><button type="button" class="kfold" data-nav {onkeydown} aria-expanded="false" onclick={() => (openQueued[s.id] = true)}><Icon name="chevronRight" size={12} />{plural(v.queued.length, 'reviewer')} queued</button></li>
      {/if}
    {/if}
    {#if v.done.length}
      <li>
        <button type="button" class="kfold" data-nav {onkeydown} aria-expanded={doneOpen} onclick={() => (openDone[s.id] = !doneOpen)}>
          <Icon name={doneOpen ? 'chevronDown' : 'chevronRight'} size={12} />{plural(v.done.length, 'reviewer')} done{v.findings ? ` · ${plural(v.findings, 'finding')}` : ''}
        </button>
      </li>
      {#if doneOpen}{#each v.done as c (c.id)}{@render childRow(s.id, c)}{/each}{/if}
    {/if}
    {#each v.trailing as c (c.id)}
      {@render childRow(s.id, c, c.status === 'queued' || c.status === 'pending' ? (v.active.length || v.queued.length ? 'After reviewers' : undefined) : undefined)}
    {/each}
  </ol>
{/snippet}

{#snippet stepRow(s: StepItem)}
  {@const w = stepWord(s.node, { now, run: runState, kind: s.kind })}
  {@const line = stepLine2(s.node)}
  {@const b = bar(s)}
  {@const on = selected.node === s.id && !selected.child}
  <button type="button" class="step" class:on class:failed={s.node.status === 'error'} data-nav {onkeydown} data-testid="wf-step" data-node-id={s.id} data-status={s.node.status} data-phase={s.node.phase} data-step-no={s.node.step_no} aria-current={on ? 'true' : undefined} onclick={() => onselect(s.id, null)}>
    {@render glyph(w.glyph)}
    <span class="main">
      <span class="l1"><span class="name ellipsis" class:dim={w.glyph === 'dash' || w.glyph === 'pending'} dir="auto">{s.name}</span><span class="word {w.tone}">{w.word}</span></span>
      {#if line}<span class="l2" dir="auto">{line}</span>{/if}
      {#if b}<span class="bar" aria-hidden="true"><span class="fill {s.node.status} {s.node.phase ?? ''}" style:inset-inline-start="{b.at}%" style:inline-size="{b.w}%"></span></span>{/if}
    </span>
  </button>
  {#if s.node.children?.length}{@render children(s)}{/if}
{/snippet}

<div class="head"><span>Steps</span><span>Time</span></div>
<ol class="steps">
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
  .kfold {
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
  .kfold:hover {
    background: var(--hover);
  }
  .step.on,
  .child.on {
    background: var(--accent-soft);
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
  .child,
  .kfold {
    align-items: center;
    min-block-size: 30px;
    padding-block: var(--sp-2);
  }
  .kfold {
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  @media (pointer: coarse) {
    .step,
    .child,
    .kfold {
      min-block-size: 44px;
    }
  }
</style>
