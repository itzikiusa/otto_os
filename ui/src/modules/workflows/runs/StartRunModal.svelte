<script lang="ts">
  // Start a run (ux.md §7): a form built from the Start step's inputs (else
  // the chat trigger's keys), "Run again with these inputs" pre-filled from an
  // earlier run, what the run will touch and where its result goes — the form
  // is the preview, so Run needs no second confirm. JSON is behind a
  // disclosure, both ways. Run validates the graph, starts the run and opens
  // its page; a refusal stays in the form. A Modal, a bottom sheet on a phone.
  import { untrack } from 'svelte';
  import Modal from '../../../lib/components/Modal.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { api } from '../../../lib/api/client';
  import { loadErrorText } from '../../../lib/loadError';
  import { announceShown } from '../../../lib/announce.svelte';
  import { router } from '../../../lib/router.svelte';
  import { elapsed } from '../../../lib/statusWords';
  import type { ReviewMode, RunWorkflowReq, Workflow, WorkflowRun, WorkflowValidationIssue } from '../../../lib/api/types';
  import { graphSideEffects } from '../sideEffects';
  import { resultDestinations } from '../runInput';
  import { buildInput, chatOrigin, fromJson, prefill as prefillState, startFields, startProblem, type FormState } from './startForm';

  interface Props {
    /** The workflow to run; null with `workflows`: the first field picks it. */
    workflow: Workflow | null;
    workflows?: readonly Workflow[];
    /** "Run again with these inputs": the earlier run's input and what it was. */
    prefill?: { input: unknown; from: string; originLabel?: string } | null;
    typicalMs?: number | null;
    /** Before the run starts (the editor saves its changes); false stops it. */
    beforeRun?: () => Promise<boolean>;
    /** "A goal…" in the workflow picker (the goal form). */
    ongoal?: () => void;
    onclose: () => void;
    /** Default: open the new run's page. */
    onstarted?: (run: WorkflowRun) => void;
  }
  let { workflow, workflows = [], prefill = null, typicalMs = null, beforeRun, ongoal, onclose, onstarted }: Props = $props();

  let pickedId = $state(untrack(() => workflow?.id ?? workflows[0]?.id ?? ''));
  const wf = $derived(workflow ?? workflows.find((w) => w.id === pickedId) ?? null);
  const trigger = $derived(wf?.graph.nodes.find((n) => n.kind === 'manual_trigger')?.params ?? {});
  const fields = $derived(startFields(trigger));
  const anyRequired = $derived(fields.some((f) => f.required));

  // The form's state is this modal's (and this workflow's): it dies with it.
  let fromRun = $state(untrack(() => !!prefill));
  let form = $state<FormState>({ values: {}, extra: {} });
  $effect.pre(() => {
    const f = fields;
    form = fromRun && prefill ? prefillState(f, prefill.input) : { values: {}, extra: {} };
  });
  const origin = $derived(fromRun && prefill ? chatOrigin(prefill.input) : null);
  /** "Only this page" (default, also on Run again) or the original thread. */
  let sendTo = $state<'page' | 'origin'>('page');
  const input = $derived(buildInput(fields, form, sendTo === 'origin' ? origin : null));
  const problem = $derived(startProblem(fields, form, input));

  const kinds = $derived(new Set(wf?.graph.nodes.flatMap((n) => [n.kind, ...(((n.params as { steps?: { kind?: string }[] } | null)?.steps ?? []).map((s) => s?.kind ?? ''))]) ?? []));
  let reviewMode = $state<'' | ReviewMode>('');
  const touches = $derived(wf ? graphSideEffects(wf.graph) : []);
  const fixedDest = $derived(resultDestinations(trigger));

  // JSON: shown from the fields; while typed in, parsed back into them.
  let jsonOpen = $state(false);
  let jsonDraft = $state<string | null>(null);
  let jsonError = $state('');
  const jsonText = $derived(jsonDraft ?? JSON.stringify(input, null, 2));
  function onJson(text: string): void {
    jsonDraft = text;
    const parsed = fromJson(fields, text);
    if ('error' in parsed) return void (jsonError = parsed.error);
    jsonError = '';
    // The thread pick writes these keys; a typed copy of them is not a second route.
    if (sendTo === 'origin') for (const k of ['result_channel', 'result_chat', 'result_thread']) delete parsed.extra[k];
    form = parsed;
  }

  function clear(): void {
    fromRun = false;
    sendTo = 'page';
    jsonDraft = null;
    jsonError = '';
  }

  let busy = $state(false);
  let error = $state('');
  /** Run was pressed: a missing field is said from now on. */
  let tried = $state(false);
  async function run(): Promise<void> {
    tried = true;
    if (!wf || problem || jsonError || busy) return;
    const target = wf;
    busy = true;
    error = '';
    try {
      if (beforeRun && !(await beforeRun())) return;
      const check = await api.post<{ valid: boolean; issues?: WorkflowValidationIssue[]; problems?: WorkflowValidationIssue[] }>(
        `/workflows/${target.id}/validate`, { graph: target.graph });
      const found = [...(check.problems ?? []), ...(check.issues ?? [])];
      if (!check.valid || (check.problems?.length ?? 0) > 0) {
        const n = Math.max(1, found.length);
        error = `Couldn’t start the run. The workflow has ${n} ${n === 1 ? 'problem' : 'problems'}${found[0] ? `: ${found[0].message}` : ''}. Open the workflow to fix it.`;
        return;
      }
      const body: RunWorkflowReq = { input, ...(reviewMode ? { review_mode: reviewMode } : {}) };
      const started = await api.post<WorkflowRun>(`/workflows/${target.id}/run`, body);
      onclose();
      if (onstarted) onstarted(started);
      else router.go(`workflows/${target.id}/runs/${started.id}`);
    } catch (e) {
      error = `Couldn’t start the run. ${loadErrorText(e)}`;
    } finally {
      busy = false;
    }
  }
</script>

<Modal title={workflow ? `Run ${workflow.name}` : 'Run a workflow'} width={520} bottom dismissable={!busy} {onclose}>
  <form class="sr" data-testid="wf-start-form" onsubmit={(e) => { e.preventDefault(); void run(); }}>
    {#if !workflow}
      <label class="sr-field">
        <span class="sr-label">Workflow</span>
        <select value={pickedId} data-testid="wf-start-workflow" onchange={(e) => {
          if (e.currentTarget.value === '__goal') { onclose(); ongoal?.(); return; }
          pickedId = e.currentTarget.value;
          clear();
        }}>
          {#if ongoal}<option value="__goal">A goal…</option>{/if}
          {#each workflows as w (w.id)}<option value={w.id}>{w.name}</option>{/each}
        </select>
      </label>
    {/if}
    {#if fromRun && prefill}
      <p class="sr-from"><Icon name="refresh" size={12} />Filled from the run of {prefill.from}{' · '}<button type="button" class="sr-link" onclick={clear}>Clear</button></p>
    {/if}

    {#each fields as f (f.key)}
      <label class="sr-field">
        <span class="sr-label">{f.label}{#if (anyRequired && !f.required) || f.hint}<span class="sr-opt">{' · '}{[anyRequired && !f.required ? 'optional' : '', f.hint ?? ''].filter(Boolean).join(', ')}</span>{/if}</span>
        {#if f.multiline}
          <textarea dir="auto" rows="3" data-testid="wf-start-field-{f.key}" placeholder={f.placeholder ?? ''} value={form.values[f.key] ?? ''}
            oninput={(e) => { form.values[f.key] = e.currentTarget.value; jsonDraft = null; }}></textarea>
        {:else}
          <input dir="auto" type="text" data-testid="wf-start-field-{f.key}" placeholder={f.placeholder ?? ''} value={form.values[f.key] ?? ''}
            required={f.required} oninput={(e) => { form.values[f.key] = e.currentTarget.value; jsonDraft = null; }} />
        {/if}
      </label>
    {/each}

    {#if kinds.has('review_run')}
      <label class="sr-field">
        <span class="sr-label">Review</span>
        <select bind:value={reviewMode} data-testid="wf-start-review-mode">
          <option value="">Workflow default</option>
          <option value="fan_out">Fan-out</option>
          <option value="orchestrator">Orchestrator</option>
        </select>
      </label>
    {/if}

    <div class="sr-will">
      <strong>This run will</strong>
      {#each touches as t, i (`${t.nodeId}:${i}`)}
        <p class:danger={t.danger}><Icon name="external" size={12} />{t.summary}</p>
      {/each}
      <label class="sr-dest">
        <Icon name="bell" size={12} /><span class="sr-nowrap">Send the result to:</span>
        <select bind:value={sendTo} data-testid="wf-start-send-to">
          <option value="page">Only this page</option>
          {#if origin}<option value="origin">The run’s thread{prefill?.originLabel ? ` in ${prefill.originLabel}` : ''} ({origin.channel === 'slack' ? 'Slack' : 'Telegram'})</option>{/if}
        </select>
      </label>
      {#if fixedDest.length}<p class="sr-fixed">The Start step also posts it to {fixedDest.join(' and ')}.</p>{/if}
    </div>

    <details class="sr-json" bind:open={jsonOpen}>
      <summary>Input as JSON</summary>
      {#if jsonOpen}
        <textarea dir="ltr" rows="8" spellcheck="false" aria-label="Run input (JSON)" data-testid="wf-start-json"
          aria-invalid={jsonError ? 'true' : undefined} value={jsonText} oninput={(e) => onJson(e.currentTarget.value)}
          onblur={() => { if (!jsonError) jsonDraft = null; }}></textarea>
        {#if jsonError}<p class="sr-err">{jsonError}</p>{/if}
      {/if}
    </details>
    {#if error || (tried && problem)}
      <p class="sr-err" use:announceShown={{ text: error || problem || '', assertive: true }}>{error || problem}</p>
    {/if}
  </form>
  {#snippet footer()}
    <span class="sr-usual">{#if typicalMs}<Icon name="clock" size={12} />Usually about {elapsed(typicalMs)}{/if}</span>
    <button type="button" class="btn" onclick={onclose} disabled={busy}>Cancel</button>
    <button type="button" class="btn primary" data-testid="wf-start-run" disabled={!wf || busy} onclick={() => void run()}>
      <Icon name="play" size={12} />{busy ? 'Starting…' : 'Run'}
    </button>
  {/snippet}
</Modal>

<style>
  .sr {
    display: flex;
    flex-direction: column;
    gap: var(--sp-3);
  }
  .sr-field {
    display: flex;
    flex-direction: column;
    gap: var(--sp-1);
    min-inline-size: 0;
  }
  .sr-label {
    font-size: var(--fs-s);
    font-weight: 600;
    color: var(--text);
  }
  .sr-opt {
    font-weight: 400;
    color: var(--text-dim);
  }
  .sr input,
  .sr textarea,
  .sr select {
    inline-size: 100%;
    font: inherit;
    font-size: var(--fs-m);
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
  }
  .sr textarea {
    resize: vertical;
  }
  .sr-json textarea {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
  }
  .sr input:focus,
  .sr textarea:focus,
  .sr select:focus {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .sr-from {
    display: flex;
    align-items: center;
    gap: var(--sp-1);
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .sr-link {
    border: none;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .sr-will {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    padding: var(--sp-3);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
    font-size: var(--fs-s);
  }
  .sr-will p,
  .sr-dest {
    display: flex;
    align-items: center;
    gap: var(--sp-2);
    margin: 0;
    color: var(--text);
  }
  .sr-will p.danger :global(svg) {
    color: var(--warning);
  }
  .sr-dest {
    flex-wrap: wrap;
  }
  .sr-nowrap {
    white-space: nowrap;
  }
  .sr .sr-dest select {
    flex: 1 1 12em;
    inline-size: auto;
    min-inline-size: 0;
    max-inline-size: 22em;
    padding: 2px 6px;
    font-size: var(--fs-s);
  }
  .sr-fixed {
    color: var(--text-dim);
  }
  .sr-json summary {
    cursor: pointer;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .sr-json textarea {
    margin-block-start: var(--sp-2);
  }
  .sr-err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .sr-usual {
    display: inline-flex;
    align-items: center;
    gap: var(--sp-1);
    margin-inline-end: auto;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
