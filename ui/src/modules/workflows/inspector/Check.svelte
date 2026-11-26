<script lang="ts">
  // check — "Check criteria" (goal.md §2.1): what must be true and how each is
  // checked (a command, an agent that cites evidence, you, or a rule), the
  // checker's model, and an optional independent review when all are met.
  import type { Criterion, VerifyKind, WorkflowNode } from '../../../lib/api/types';
  import Icon from '../../../lib/components/Icon.svelte';
  import StepModel from './StepModel.svelte';
  import { paramBool, paramStr, type SetParam } from './util';

  let { node, set, defaultProvider }: { node: WorkflowNode; set: SetParam; defaultProvider: string } = $props();
  const ids = $props.id();
  const params = $derived((node.params ?? {}) as { criteria?: Criterion[]; evaluator?: { provider?: string; model?: string; effort?: string } });
  const rows = $derived(Array.isArray(params.criteria) ? params.criteria : []);
  const evaluator = $derived(params.evaluator ?? {});
  const KINDS: { v: VerifyKind; label: string }[] = [
    { v: 'command', label: 'A command passes' },
    { v: 'agent', label: 'An agent finds evidence' },
    { v: 'human', label: 'I check it' },
    { v: 'expr', label: 'A rule holds' },
  ];

  const save = (next: Criterion[]): void => set('criteria', next.length ? next : undefined);
  const patch = (i: number, p: Partial<Criterion>): void => save(rows.map((c, k) => (k === i ? { ...c, ...p } : c)));
  function add(): void {
    let n = rows.length + 1;
    while (rows.some((c) => c.id === `c${n}`)) n += 1;
    save([...rows, { id: `c${n}`, text: '', verify: 'agent', how: '' }]);
  }
  function move(i: number, d: number): void {
    const next = [...rows];
    [next[i], next[i + d]] = [next[i + d], next[i]];
    save(next);
  }
</script>

<div class="rv-h">
  <span class="np-label">Done when</span>
  <button type="button" class="btn small ghost" onclick={add}><Icon name="plus" size={12} /> Add</button>
</div>
{#if rows.length === 0}<p class="insp-note">No criteria yet — add what must be true when the work is done.</p>{/if}
<div class="rv-list">
  {#each rows as c, i (c.id)}
    <div class="rv-row">
      <div class="rv-top">
        <input dir="auto" class="rv-lens" type="text" aria-label="Criterion {i + 1}" placeholder="What must be true" value={c.text}
          oninput={(e) => patch(i, { text: e.currentTarget.value })} />
        <button type="button" class="rv-del" aria-label="Move up" title="Move up" disabled={i === 0} onclick={() => move(i, -1)}><Icon name="arrowUp" size={12} /></button>
        <button type="button" class="rv-del" aria-label="Move down" title="Move down" disabled={i === rows.length - 1} onclick={() => move(i, 1)}><Icon name="arrowDown" size={12} /></button>
        <button type="button" class="rv-del" aria-label="Remove criterion" title="Remove criterion" onclick={() => save(rows.filter((_, k) => k !== i))}><Icon name="trash" size={12} /></button>
      </div>
      <select aria-label="How it is checked" value={c.verify} onchange={(e) => patch(i, { verify: e.currentTarget.value as VerifyKind })}>
        {#each KINDS as k (k.v)}<option value={k.v}>{k.label}</option>{/each}
      </select>
      {#if c.verify === 'command'}
        <input dir="ltr" type="text" class="ck-mono" aria-label="Command" placeholder="go test ./..." value={c.command ?? ''} oninput={(e) => patch(i, { command: e.currentTarget.value })} />
      {:else if c.verify === 'expr'}
        <input dir="ltr" type="text" class="ck-mono" aria-label="Rule" placeholder="steps.review.passed == true" value={c.expr ?? ''} oninput={(e) => patch(i, { expr: e.currentTarget.value })} />
      {:else}
        <textarea dir="auto" rows="2" aria-label="What to inspect" placeholder={c.verify === 'human' ? 'What you will look at' : 'Where the agent should look'} value={c.how ?? ''}
          oninput={(e) => patch(i, { how: e.currentTarget.value })}></textarea>
      {/if}
    </div>
  {/each}
</div>

<span class="np-label np-sec">Checker</span>
<StepModel value={{ provider: evaluator.provider ?? '', model: evaluator.model ?? '', effort: evaluator.effort ?? '' }} {defaultProvider} label="Checker"
  onchange={(v) => { const e = Object.fromEntries(Object.entries(v).filter(([, x]) => x)); set('evaluator', Object.keys(e).length ? e : undefined); }} />
<p class="node-hint">Judges the agent criteria on a fresh thread; commands and rules are decided first.</p>
<label for="{ids}-instr">Instructions for the checker (optional)</label>
<textarea dir="auto" id="{ids}-instr" class="np-prompt" rows="4" value={paramStr(node, 'instructions')} oninput={(e) => set('instructions', e.currentTarget.value)}></textarea>
<label class="np-chk">
  <input type="checkbox" checked={paramBool(node, 'final_review')} onchange={(e) => set('final_review', e.currentTarget.checked || undefined)} />
  When every criterion is met, have an independent review approve it
</label>

<style>
  .ck-mono {
    font-family: var(--font-mono);
  }
</style>
