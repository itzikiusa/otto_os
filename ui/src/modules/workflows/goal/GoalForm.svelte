<script lang="ts">
  // The Goal form (goal.md §3.3, ux.md §7.1): what the goal is, when it is
  // done (criteria and how each is checked), where it works, its budget and
  // models. Start goal builds the goal's workflow on the server and lands on
  // its run page. A bottom sheet on a phone. Fields ⇄ spec: goalForm.ts.
  import { untrack } from 'svelte';
  import { announceShown } from '../../../lib/announce.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import { api } from '../../../lib/api/client';
  import { continueLoopAsGoal, startGoal } from '../../../lib/api/goals';
  import type { BranchInfo, Repo } from '../../../lib/api/types';
  import { loadErrorText } from '../../../lib/loadError';
  import { defaultAgentProvider } from '../../../lib/providers';
  import { router } from '../../../lib/router.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import StepModel from '../inspector/StepModel.svelte';
  import { runRoutePath } from '../runRoute';
  import {
    ACTIVE_OPTIONS, applyTemplate, emptyFields, firstProblem, MAX_ITERATIONS, moveCriterion, newCriterion, setGoal, toSpec, VERIFY_LABEL, willLine,
    type FormVerify, type GoalFields,
  } from './goalForm';
  import { goalTemplates, type GoalTemplateId } from './goalTemplates';
  import type { Criterion } from '../../../lib/api/types';

  let { workspaceId, initial = null, onclose }: { workspaceId: string; initial?: GoalFields | null; onclose: () => void } = $props();

  const ids = $props.id();
  // The form's own copy: the starting fields are read once.
  let f = $state<GoalFields>(structuredClone(untrack(() => $state.snapshot(initial)) ?? emptyFields()) as GoalFields);
  let template = $state<GoalTemplateId | null>(null);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let tried = $state(false);
  const defaultProvider = defaultAgentProvider();

  let repos = $state<Repo[]>([]);
  let reposError = $state<string | null>(null);
  let branches = $state<BranchInfo[]>([]);
  $effect(() => {
    const w = workspaceId;
    api.get<Repo[]>(`/workspaces/${encodeURIComponent(w)}/repos`).then(
      (rows) => {
        repos = rows;
        if (!f.repoId && !f.continueLoop && rows.length === 1) f.repoId = rows[0].id;
      },
      (e) => (reposError = loadErrorText(e)),
    );
  });
  $effect(() => {
    const id = f.work === 'repo' && !f.continueLoop ? f.repoId : '';
    branches = [];
    if (id) api.get<BranchInfo[]>(`/repos/${encodeURIComponent(id)}/branches`).then((b) => { if (f.repoId === id) branches = b; }, () => {});
  });

  const problem = $derived(firstProblem(f));
  const repoName = $derived(repos.find((r) => r.id === f.repoId)?.name ?? null);
  const example = $derived(goalTemplates.find((t) => t.id === template)?.example ?? 'Make the export stream rows instead of buffering them, keeping the CSV format.');
  const activeChoices = $derived(ACTIVE_OPTIONS.some((o) => o.s === f.activeS) ? ACTIVE_OPTIONS : [...ACTIVE_OPTIONS, { s: f.activeS, label: `${Math.round(f.activeS / 60)}m` }].sort((a, b) => a.s - b.s));

  function patch(i: number, p: Partial<Criterion>): void {
    f.criteria = f.criteria.map((c, k) => (k === i ? { ...c, ...p } : c));
  }
  function pickTemplate(id: GoalTemplateId): void {
    template = id;
    f = applyTemplate(f, id);
  }

  async function start(): Promise<void> {
    tried = true;
    if (problem || busy) return;
    busy = true;
    error = null;
    try {
      const spec = toSpec(f);
      const res = f.continueLoop ? await continueLoopAsGoal(f.continueLoop.id, spec) : await startGoal(workspaceId, spec);
      toasts.success('Goal started', spec.title);
      onclose();
      router.go(runRoutePath(res.workflow.id, res.run.id));
    } catch (e) {
      error = loadErrorText(e);
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={f.continueLoop ? 'Continue as a goal' : 'New goal'} width={640} bottom dismissable={!busy} {onclose}>
  <form class="gf" id="{ids}-form" data-testid="wf-goal-form" onsubmit={(e) => { e.preventDefault(); void start(); }}>
    <label class="lbl" for="{ids}-goal">Goal</label>
    <textarea id="{ids}-goal" class="input gf-in" rows="3" dir="auto" data-testid="wf-goal-field-goal" placeholder={example}
      value={f.goal} oninput={(e) => (f = setGoal(f, e.currentTarget.value))}></textarea>
    {#if !f.continueLoop}
      <div class="chips" role="group" aria-label="Start from a kind of goal">
        <span class="hint">Start from…</span>
        {#each goalTemplates as t (t.id)}
          <button type="button" class="tpl" class:on={template === t.id} aria-pressed={template === t.id} title={t.description} onclick={() => pickTemplate(t.id)}>{t.title}</button>
        {/each}
      </div>
    {/if}
    <label class="lbl" for="{ids}-title">Title</label>
    <input id="{ids}-title" class="input gf-in" type="text" dir="auto" maxlength="120" data-testid="wf-goal-field-title" value={f.title}
      oninput={(e) => { f.title = e.currentTarget.value; f.titleEdited = true; }} />

    <div class="sec">
      <span class="lbl" id="{ids}-crit">Done when</span>
      <button type="button" class="btn small ghost" onclick={() => (f.criteria = [...f.criteria, newCriterion(f.criteria)])}><Icon name="plus" size={12} />Add criterion</button>
    </div>
    <ul class="crit" aria-labelledby="{ids}-crit" data-testid="wf-goal-field-criteria">
      {#each f.criteria as c, i (c.id)}
        <li class="crow" data-testid="wf-goal-criterion-row">
          <div class="top">
            <input class="input gf-in" type="text" dir="auto" aria-label="Criterion {i + 1}: what must be true" placeholder="What must be true" value={c.text}
              oninput={(e) => patch(i, { text: e.currentTarget.value })} />
            <button type="button" class="icon-btn" aria-label="Move criterion {i + 1} up" title="Move up" disabled={i === 0} onclick={() => (f.criteria = moveCriterion(f.criteria, i, -1))}><Icon name="arrowUp" size={12} /></button>
            <button type="button" class="icon-btn" aria-label="Move criterion {i + 1} down" title="Move down" disabled={i === f.criteria.length - 1} onclick={() => (f.criteria = moveCriterion(f.criteria, i, 1))}><Icon name="arrowDown" size={12} /></button>
            <button type="button" class="icon-btn" aria-label="Remove criterion {i + 1}" title="Remove" onclick={() => (f.criteria = f.criteria.filter((_, k) => k !== i))}><Icon name="trash" size={12} /></button>
          </div>
          <select class="input gf-in" aria-label="How criterion {i + 1} is checked" value={c.verify} onchange={(e) => patch(i, { verify: e.currentTarget.value as FormVerify })}>
            {#each Object.entries(VERIFY_LABEL) as [v, label] (v)}<option value={v}>{label}</option>{/each}
          </select>
          {#if c.verify === 'command'}
            <input class="input gf-in mono" type="text" dir="ltr" aria-label="Command for criterion {i + 1}" placeholder="go test ./..." value={c.command ?? ''}
              oninput={(e) => patch(i, { command: e.currentTarget.value })} />
          {:else}
            <textarea class="input gf-in" rows="2" dir="auto" aria-label="What to inspect for criterion {i + 1}" placeholder={c.verify === 'human' ? 'What you will look at' : 'Where the agent should look for evidence'}
              value={c.how} oninput={(e) => patch(i, { how: e.currentTarget.value })}></textarea>
          {/if}
        </li>
      {/each}
    </ul>

    <span class="lbl">Work in</span>
    {#if f.continueLoop}
      <p class="fixed" data-testid="wf-goal-field-work">This loop’s kept work (<code>{f.continueLoop.path}</code>)</p>
      <p class="hint">Earlier person verifications are not carried over: they were for earlier work.</p>
    {:else}
      <div class="work" data-testid="wf-goal-field-work">
        <label class="radio"><input type="radio" name="{ids}-work" checked={f.work === 'repo'} onchange={() => (f.work = 'repo')} />A repository</label>
        {#if f.work === 'repo'}
          <div class="pick">
            <select class="input gf-in" aria-label="Repository" value={f.repoId} onchange={(e) => (f.repoId = e.currentTarget.value)}>
              <option value="">{reposError ? 'Couldn’t load the repositories' : repos.length ? 'Pick a repository' : 'No repositories in this workspace'}</option>
              {#each repos as r (r.id)}<option value={r.id} title={r.path}>{r.name}</option>{/each}
            </select>
            <select class="input gf-in" aria-label="From" value={f.base} disabled={!f.repoId} onchange={(e) => (f.base = e.currentTarget.value)}>
              <option value="">From the current commit</option>
              {#each branches as b (b.name)}<option value={b.name}>From {b.name}</option>{/each}
            </select>
          </div>
        {/if}
        <label class="radio"><input type="radio" name="{ids}-work" checked={f.work === 'research'} onchange={() => (f.work = 'research')} />Research — no repository</label>
      </div>
    {/if}

    <span class="lbl">Budget</span>
    <div class="budget" data-testid="wf-goal-field-budget">
      <label class="inline">Up to
        <input class="input gf-in num" type="number" min="1" max={MAX_ITERATIONS} aria-label="Iterations" value={f.iterations}
          oninput={(e) => (f.iterations = Number(e.currentTarget.value) || 1)} /> iterations</label>
      <label class="inline">and
        <select class="input gf-in" aria-label="Active time" value={f.activeS} onchange={(e) => (f.activeS = Number(e.currentTarget.value))}>
          {#each activeChoices as o (o.s)}<option value={o.s}>{o.label}</option>{/each}
        </select> of active time</label>
    </div>
    <p class="hint">Paused and waiting time is not counted.</p>

    <span class="lbl">Models</span>
    <div class="models" data-testid="wf-goal-field-models">
      <span class="ml">Work</span><StepModel value={f.worker} {defaultProvider} label="Work" onchange={(v) => (f.worker = v)} />
      <span class="ml">Check</span><StepModel value={f.checker} {defaultProvider} label="Check" onchange={(v) => (f.checker = v)} />
    </div>

    <label class="radio toggle" data-testid="wf-goal-field-review">
      <input type="checkbox" checked={f.finalReview} onchange={(e) => (f.finalReview = e.currentTarget.checked)} />
      <span>Review when done<span class="hint block">A separate agent reviews the finished work; findings block success.</span></span>
    </label>

    <p class="will" data-testid="wf-goal-will"><strong>This goal will</strong> {willLine(f, repoName).replace(/^W/, 'w')}</p>
    {#if error || (tried && problem)}
      {@const text = error ?? problem?.text ?? ''}
      <p class="err" use:announceShown={{ text, assertive: true }}><Icon name="warning" size={13} /><span dir="auto">{text}</span></p>
    {/if}
  </form>
  {#snippet footer()}
    <button type="button" class="btn" disabled={busy} onclick={onclose}>Cancel</button>
    <button type="submit" class="btn primary" form="{ids}-form" data-testid="wf-goal-start" disabled={busy} title={problem?.text}>{busy ? 'Starting…' : 'Start goal'}</button>
  {/snippet}
</Modal>

<style>
  .gf {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    font-size: var(--fs-m);
  }
  .lbl {
    margin-block-start: var(--sp-3);
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .gf-in {
    inline-size: 100%;
    min-inline-size: 0;
  }
  .mono {
    font-family: var(--font-mono);
  }
  .hint {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .hint.block {
    display: block;
    font-weight: 400;
  }
  .chips {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: var(--sp-2);
  }
  .tpl {
    padding: 2px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius-l);
    background: var(--surface);
    color: var(--text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .tpl:hover {
    border-color: var(--border-strong);
  }
  .tpl.on {
    border-color: var(--accent-solid);
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .sec {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-block-start: var(--sp-3);
  }
  .sec .lbl {
    margin: 0;
  }
  .crit {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .crow {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    padding: var(--sp-3) var(--sp-4);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
  }
  .top {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .work,
  .models {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
  }
  .pick {
    display: flex;
    gap: var(--sp-3);
    padding-inline-start: var(--sp-6);
  }
  .radio {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-3);
  }
  .toggle {
    margin-block-start: var(--sp-3);
    font-weight: 600;
    font-size: var(--fs-s);
  }
  .budget {
    display: flex;
    flex-wrap: wrap;
    gap: var(--sp-3) var(--sp-5);
  }
  .inline {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-2);
  }
  .inline .gf-in {
    inline-size: auto;
  }
  .num {
    inline-size: 4.5em !important;
  }
  .models {
    display: grid;
    grid-template-columns: auto 1fr;
    align-items: center;
    gap: var(--sp-2) var(--sp-4);
  }
  .ml {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .fixed {
    margin: 0;
    overflow-wrap: anywhere;
  }
  .will {
    margin: var(--sp-3) 0 0;
    padding: var(--sp-3) var(--sp-4);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    font-size: var(--fs-s);
  }
  .err {
    display: flex;
    align-items: flex-start;
    gap: var(--sp-2);
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-s);
  }
  @media (max-width: 640px) {
    .pick {
      flex-direction: column;
      padding-inline-start: 0;
    }
  }
</style>
