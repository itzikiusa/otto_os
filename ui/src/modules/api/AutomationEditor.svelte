<script lang="ts">
  // One automation, edited in the main area: an ordered list of saved requests
  // ("steps"). Each step can CHECK its response (assertions) and SAVE values
  // from it into variables that later steps use as {{name}}. Edits are a
  // working copy until Save; Run saves first, then runs in the daemon and
  // polls the saved report.
  import { editableSteps } from './automationInput';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import MethodTag from './MethodTag.svelte';
  import StatusChip from './StatusChip.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { formatSeconds } from '../../lib/metric-format';
  import type { ApiAutomationRun, ApiAssertion, ApiAutomation, ApiAutomationStep, ApiExtract, Id } from '../../lib/api/types';

  interface Props {
    automationId: Id;
    /** The automation was deleted (the page goes back to the request view). */
    ondeleted?: () => void;
  }
  let { automationId, ondeleted }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');
  const automation = $derived(apiClient.automations.find((a) => a.id === automationId) ?? null);

  // Working copy (edits aren't live until Save).
  let steps: ApiAutomationStep[] = $state([]);
  let dirty = $state(false);
  let environmentId = $state('');
  let stopOnFailure = $state(false);
  let datasetText = $state('');
  let historyRun = $state<ApiAutomationRun | null>(null);
  let loadedFor = $state<string | null>(null);

  function loadInto(a: ApiAutomation): void {
    loadedFor = a.id;
    steps = editableSteps(a.steps);
    dirty = false;
    historyRun = null;
    datasetText = '';
    void apiClient.loadAutomationRuns(a.id);
  }
  $effect(() => {
    const a = automation;
    if (a && a.id !== loadedFor) loadInto(a);
  });

  const KIND_LABEL: Record<ApiAssertion['kind'], string> = {
    status: 'Status code',
    json_path: 'JSON value at',
    duration_ms: 'Response time (ms)',
  };
  const OP_LABEL: Record<ApiAssertion['op'], string> = {
    eq: 'equals',
    ne: 'does not equal',
    contains: 'contains',
    lt: 'is less than',
    gt: 'is greater than',
  };
  const RUN_STATUS: Record<ApiAutomationRun['status'], string> = {
    running: 'running', passed: 'passed', failed: 'failed', cancelled: 'canceled', interrupted: 'interrupted',
  };

  const req = (id: Id) => apiClient.requests.find((r) => r.id === id);

  function addStep(): void {
    const rid = apiClient.requests[0]?.id;
    if (!rid) return;
    steps = [...steps, { request_id: rid, assertions: [{ kind: 'status', op: 'lt', value: '400' }], extract: [] }];
    dirty = true;
  }
  function removeStep(i: number): void {
    steps = steps.filter((_, idx) => idx !== i);
    dirty = true;
  }
  function moveStep(i: number, dir: -1 | 1): void {
    const j = i + dir;
    if (j < 0 || j >= steps.length) return;
    const next = [...steps];
    [next[i], next[j]] = [next[j], next[i]];
    steps = next;
    dirty = true;
  }
  function patchStep(i: number, patch: Partial<ApiAutomationStep>): void {
    steps = steps.map((s, idx) => (idx === i ? { ...s, ...patch } : s));
    dirty = true;
  }
  function addAssertion(i: number): void {
    patchStep(i, { assertions: [...steps[i].assertions, { kind: 'status', op: 'eq', value: '200' }] });
  }
  function updateAssertion(i: number, a: number, patch: Partial<ApiAssertion>): void {
    patchStep(i, { assertions: steps[i].assertions.map((x, ai) => (ai === a ? { ...x, ...patch } : x)) });
  }
  function removeAssertion(i: number, a: number): void {
    patchStep(i, { assertions: steps[i].assertions.filter((_, ai) => ai !== a) });
  }
  function addExtract(i: number): void {
    patchStep(i, { extract: [...steps[i].extract, { path: '', var: '' }] });
  }
  function updateExtract(i: number, e: number, patch: Partial<ApiExtract>): void {
    patchStep(i, { extract: steps[i].extract.map((x, ei) => (ei === e ? { ...x, ...patch } : x)) });
  }
  function removeExtract(i: number, e: number): void {
    patchStep(i, { extract: steps[i].extract.filter((_, ei) => ei !== e) });
  }

  async function save(): Promise<boolean> {
    if (!automation || !canEdit) return false;
    const clean = steps.map((s) => ({
      request_id: s.request_id,
      assertions: s.assertions.map((a) => ({ ...a })),
      // Drop incomplete rows (both path + variable name are required).
      extract: s.extract.filter((e) => e.path.trim() !== '' && e.var.trim() !== ''),
    }));
    const saved = await apiClient.saveAutomation({ name: automation.name, steps: clean }, automation.id);
    if (saved) {
      dirty = false;
      toasts.success('Automation saved', saved.name);
    }
    return !!saved;
  }

  let datasetError = $state('');
  async function run(): Promise<void> {
    if (!automation) return;
    if (dirty && !(await save())) return; // never run a different persisted definition
    let dataset: Record<string, unknown>[] = [];
    datasetError = '';
    try {
      if (datasetText.trim()) dataset = JSON.parse(datasetText);
      if (!Array.isArray(dataset) || dataset.some((r) => !r || Array.isArray(r) || typeof r !== 'object')) throw new Error('Use a JSON array of objects.');
    } catch (e) {
      datasetError = `Dataset rows aren’t valid: ${e instanceof Error ? e.message : String(e)}`;
      return;
    }
    historyRun = null;
    await apiClient.runAutomation(automation.id, { environment_id: environmentId || null, stop_on_failure: stopOnFailure, dataset });
    showReport();
  }

  // The report sits below the steps: bring it into view when a run finishes
  // (or from the header's result chip) so a Run never ends off-screen.
  let reportEl = $state<HTMLElement | null>(null);
  function showReport(): void {
    requestAnimationFrame(() => {
      const reduce = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
      reportEl?.scrollIntoView({ block: 'start', behavior: reduce ? 'auto' : 'smooth' });
    });
  }

  async function rename(): Promise<void> {
    if (!automation) return;
    const n = await confirmer.promptText('Name', { title: 'Rename automation', confirmLabel: 'Rename', initial: automation.name });
    if (!n || n === automation.name) return;
    await apiClient.saveAutomation({ name: n, steps: automation.steps }, automation.id);
  }
  async function remove(): Promise<void> {
    if (!automation) return;
    if (!(await confirmer.ask(`Delete the automation “${automation.name}” and its run history? The saved requests it uses are kept.`, { title: 'Delete automation' }))) return;
    await apiClient.deleteAutomation(automation.id);
    ondeleted?.();
  }
  function menu(e: MouseEvent): void {
    ctxMenu.show(e, [
      { label: 'Rename…', icon: 'edit', action: () => void rename(), disabled: !canEdit },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void remove(), disabled: !canEdit },
    ]);
  }

  const lastRun = $derived(historyRun?.report ?? apiClient.lastRun);
  const runDetails = $derived(historyRun ?? apiClient.currentRun);
  const showRun = $derived(lastRun && lastRun.automation_id === automationId ? lastRun : null);
  const runs = $derived(apiClient.automationRuns.filter((r) => r.automation_id === automationId));
</script>

{#if !automation}
  <EmptyState icon="zap" title="This automation no longer exists" body="It may have been deleted. Pick another one from the list." />
{:else}
  <div class="auto">
    <header class="view-head">
      <div class="vh-text">
        <h2>{automation.name}{#if dirty}<span class="unsaved" title="Unsaved changes"> · edited</span>{/if}</h2>
        <p>Runs saved requests in order. Each step can <strong>check</strong> the response and <strong>save values</strong> from it (like a token or an id) as <code>{'{{variables}}'}</code> for the steps after it. Saved values only live for the run; environments aren’t changed.</p>
      </div>
      <div class="vh-actions">
        {#if showRun}
          <button class="chip result" class:ok={!apiClient.running && showRun.passed} class:bad={!apiClient.running && !showRun.passed} onclick={showReport} title="Show the last run’s results">
            {apiClient.running ? 'Running…' : showRun.passed ? 'Last run passed' : 'Last run failed'} · {showRun.steps.filter((s) => s.ok).length}/{showRun.steps.length}
          </button>
        {/if}
        <button class="icon-btn" onclick={menu} aria-label="Automation options" title="Automation options"><Icon name="more" size={14} /></button>
        <button class="btn small" onclick={() => void save()} disabled={!canEdit || !dirty || apiClient.running}>Save</button>
        {#if runDetails?.status === 'running' && runDetails.automation_id === automationId}
          <button class="btn small" onclick={async () => { await apiClient.cancelAutomationRun(runDetails!.id); await apiClient.loadAutomationRuns(automationId); }}>Cancel run</button>
        {/if}
        <button class="btn small primary" onclick={run} disabled={!canEdit || steps.length === 0 || apiClient.running} title={steps.length === 0 ? 'Add a step first' : 'Save and run every step'}>
          <Icon name="play" size={12} />{apiClient.running ? 'Running…' : 'Run'}
        </button>
      </div>
    </header>

    <div class="run-opts">
      <label class="opt">
        <span>Environment</span>
        <select class="input" bind:value={environmentId} disabled={apiClient.running}>
          <option value="">The active one when the run starts{apiClient.activeEnv ? ` (${apiClient.activeEnv.name})` : ''}</option>
          {#each apiClient.environments as environment (environment.id)}<option value={environment.id}>{environment.name}</option>{/each}
        </select>
      </label>
      <label class="checkbox-row"><input type="checkbox" bind:checked={stopOnFailure} disabled={apiClient.running} /> Stop at the first failed step</label>
    </div>

    <details class="disclosure">
      <summary>Run once per data row (advanced)</summary>
      <p class="hint">A JSON array of objects. Each object runs every step once, with its keys available as <code>{'{{variables}}'}</code> on top of the environment. Values stay in memory. Up to 1 MiB and 1,000 requests per run.</p>
      <textarea class="input mono" aria-label="Dataset rows" rows="4" bind:value={datasetText} placeholder={'[{"customer_id":"cus_1"},{"customer_id":"cus_2"}]'} disabled={apiClient.running}></textarea>
      {#if datasetError}<p class="err" role="alert">{datasetError}</p>{/if}
    </details>

    <ol class="steps">
      {#each steps as step, i (i)}
        {@const r = req(step.request_id)}
        <li class="step">
          <div class="step-top">
            <span class="step-idx" aria-hidden="true">{i + 1}</span>
            <MethodTag method={r?.method ?? ''} fixed />
            <select class="input grow" aria-label="Request for step {i + 1}" value={step.request_id} disabled={!canEdit}
              onchange={(e) => patchStep(i, { request_id: (e.currentTarget as HTMLSelectElement).value })}>
              {#each apiClient.requests as q (q.id)}
                <option value={q.id}>{q.name} · {q.method} {q.url}</option>
              {/each}
              {#if !r}<option value={step.request_id}>Deleted request</option>{/if}
            </select>
            {#if canEdit}
              <button class="icon-btn" title="Move up" aria-label="Move step {i + 1} up" disabled={i === 0} onclick={() => moveStep(i, -1)}><Icon name="arrowUp" size={14} /></button>
              <button class="icon-btn" title="Move down" aria-label="Move step {i + 1} down" disabled={i === steps.length - 1} onclick={() => moveStep(i, 1)}><Icon name="arrowDown" size={14} /></button>
              <button class="icon-btn" title="Remove step" aria-label="Remove step {i + 1}" onclick={() => removeStep(i)}><Icon name="trash" size={14} /></button>
            {/if}
          </div>

          <div class="sub">
            <div class="sub-title">Check that</div>
            {#if step.assertions.length === 0}
              <p class="sub-empty">No checks, so the step passes on any 2xx response.</p>
            {/if}
            {#each step.assertions as as, ai (ai)}
              <div class="rule">
                <select class="input k-kind" aria-label="What to check" value={as.kind} disabled={!canEdit}
                  onchange={(e) => updateAssertion(i, ai, { kind: (e.currentTarget as HTMLSelectElement).value as ApiAssertion['kind'] })}>
                  {#each Object.entries(KIND_LABEL) as [k, label] (k)}<option value={k}>{label}</option>{/each}
                </select>
                {#if as.kind === 'json_path'}
                  <input class="input mono k-path" aria-label="JSONPath" placeholder="$.data.id" value={as.path ?? ''} disabled={!canEdit}
                    oninput={(e) => updateAssertion(i, ai, { path: (e.currentTarget as HTMLInputElement).value })} />
                {/if}
                <select class="input k-op" aria-label="Comparison" value={as.op} disabled={!canEdit}
                  onchange={(e) => updateAssertion(i, ai, { op: (e.currentTarget as HTMLSelectElement).value as ApiAssertion['op'] })}>
                  {#each Object.entries(OP_LABEL) as [o, label] (o)}<option value={o}>{label}</option>{/each}
                </select>
                <input class="input mono k-val" aria-label="Expected value" placeholder="200" value={as.value} disabled={!canEdit}
                  oninput={(e) => updateAssertion(i, ai, { value: (e.currentTarget as HTMLInputElement).value })} />
                {#if canEdit}
                  <button class="icon-btn" title="Remove check" aria-label="Remove check" onclick={() => removeAssertion(i, ai)}><Icon name="x" size={12} /></button>
                {/if}
              </div>
            {/each}
            {#if canEdit}<button class="btn ghost small add" onclick={() => addAssertion(i)}><Icon name="plus" size={12} />Add check</button>{/if}
          </div>

          <div class="sub">
            <div class="sub-title">Save for later steps</div>
            {#each step.extract as ex, ei (ei)}
              <div class="rule">
                <span class="word">Take</span>
                <input class="input mono k-path" aria-label="JSONPath to take" placeholder="$.access_token" value={ex.path} disabled={!canEdit}
                  oninput={(e) => updateExtract(i, ei, { path: (e.currentTarget as HTMLInputElement).value })} />
                <span class="word">as</span>
                <span class="var-in">
                  <span class="brace" aria-hidden="true">{'{{'}</span>
                  <input class="input mono k-var" aria-label="Variable name" placeholder="api_token" value={ex.var} disabled={!canEdit}
                    oninput={(e) => updateExtract(i, ei, { var: (e.currentTarget as HTMLInputElement).value })} />
                  <span class="brace" aria-hidden="true">{'}}'}</span>
                </span>
                {#if canEdit}
                  <button class="icon-btn" title="Remove" aria-label="Remove saved value" onclick={() => removeExtract(i, ei)}><Icon name="x" size={12} /></button>
                {/if}
              </div>
            {/each}
            {#if canEdit}<button class="btn ghost small add" onclick={() => addExtract(i)}><Icon name="plus" size={12} />Save a value</button>{/if}
          </div>
        </li>
      {/each}
    </ol>

    {#if apiClient.requests.length === 0}
      <p class="sub-empty">Steps run saved requests, and there are none yet. Save a request (⌘S) first.</p>
    {:else if canEdit}
      <button class="btn small add-step" onclick={addStep}><Icon name="plus" size={12} />Add step</button>
    {/if}

    {#if showRun}
      <section class="report" aria-label="Last run" bind:this={reportEl}>
        <div class="report-banner" class:ok={showRun.passed} class:fail={!showRun.passed}>
          <Icon name={showRun.passed ? 'check' : 'warning'} size={14} />
          {apiClient.running ? 'Running…' : showRun.passed ? 'All steps passed' : 'Run failed'}
          <span class="rb-meta">{showRun.steps.filter((s) => s.ok).length} of {showRun.steps.length} steps passed</span>
        </div>
        {#if runDetails && runDetails.automation_id === automationId}
          <p class="run-meta">
            Run <span class="mono" title="Run id">{runDetails.id}</span> · {RUN_STATUS[runDetails.status]} · {rel(runDetails.created_at)}{#if runDetails.dataset_rows}{' '}· {runDetails.dataset_rows} data row{runDetails.dataset_rows === 1 ? '' : 's'}{/if}
          </p>
          {#if runDetails.error}<p class="err" role="status">{runDetails.error}</p>{/if}
        {/if}
        {#each showRun.steps as r, ri (ri)}
          <div class="r-step" class:ok={r.ok} class:fail={!r.ok}>
            <div class="r-top">
              <Icon name={r.ok ? 'check' : 'x'} size={12} />
              <span class="r-idx">{ri + 1}</span>
              <span class="r-name">{r.name}</span>
              <StatusChip status={r.status} small />
              <span class="r-dur">{formatSeconds(r.duration_ms / 1000)}</span>
            </div>
            {#if r.error}<div class="r-error mono">{r.error}</div>{/if}
            {#each r.assertions as a, ax (ax)}
              <div class="r-assert" class:ok={a.passed} class:fail={!a.passed}>
                <span class="r-word">{a.passed ? 'Passed' : 'Failed'}</span><span class="r-desc">{a.desc}</span>
              </div>
            {/each}
          </div>
        {/each}
        {#if runDetails && runDetails.automation_id === automationId}
          <details class="disclosure"><summary>Request versions used by this run</summary><pre class="snap mono">{JSON.stringify(runDetails.snapshot, null, 2)}</pre></details>
        {/if}
      </section>
    {/if}

    <details class="disclosure">
      <summary>Run history ({runs.length})</summary>
      <div class="runs">
        {#each runs as savedRun (savedRun.id)}
          <button class="run-row" class:sel={historyRun?.id === savedRun.id} onclick={() => (historyRun = savedRun)} title={new Date(savedRun.created_at).toLocaleString()}>
            {RUN_STATUS[savedRun.status]} · {savedRun.report.steps.length} steps · {rel(savedRun.created_at)}
          </button>
        {:else}
          <p class="sub-empty">No runs yet.</p>
        {/each}
        <div class="runs-actions">
          <button class="btn ghost small" onclick={async () => { await apiClient.loadAutomationRuns(automationId); if (historyRun) historyRun = apiClient.automationRuns.find((r) => r.id === historyRun?.id) ?? historyRun; }}>
            <Icon name="refresh" size={12} />Refresh
          </button>
          {#if apiClient.automationRuns.length >= 50}
            <button class="btn ghost small" onclick={() => apiClient.loadAutomationRuns(automationId, apiClient.automationRuns.at(-1)!.id)}>Load older runs</button>
          {/if}
        </div>
      </div>
    </details>
  </div>
{/if}

<style>
  .auto {
    display: flex;
    flex-direction: column;
    gap: 14px;
    padding: 16px 20px 32px;
    min-height: 0;
    overflow-y: auto;
    flex: 1;
  }
  .view-head {
    display: flex;
    align-items: flex-start;
    gap: 16px;
  }
  .vh-text {
    flex: 1;
    min-width: 0;
  }
  h2 {
    margin: 0 0 4px;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .unsaved {
    font-weight: 400;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .view-head p,
  .hint {
    margin: 0;
    max-width: 760px;
    font-size: var(--fs-s);
    line-height: 1.5;
    color: var(--text-dim);
  }
  .vh-actions {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  code {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    color: var(--text);
  }
  .result {
    cursor: pointer;
  }
  .run-opts {
    display: flex;
    align-items: center;
    gap: 16px;
    flex-wrap: wrap;
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .opt select {
    max-width: 320px;
  }
  .disclosure > summary {
    cursor: pointer;
    font-size: var(--fs-s);
    color: var(--text-dim);
    user-select: none;
  }
  .disclosure[open] > summary {
    margin-bottom: 8px;
  }
  .disclosure textarea {
    width: 100%;
    margin-top: 6px;
    font-size: var(--fs-s);
  }
  .steps {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .step {
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    padding: 10px 12px;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .step-top {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .step-idx {
    display: grid;
    place-items: center;
    width: 20px;
    height: 20px;
    flex-shrink: 0;
    border-radius: 50%;
    background: var(--surface-2);
    border: 1px solid var(--border);
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text);
  }
  .sub {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding-inline-start: 28px;
  }
  .sub-title {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .sub-empty {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .rule {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-wrap: wrap;
  }
  .k-kind { width: 170px; }
  .k-op { width: 150px; }
  .k-path { width: 200px; min-width: 0; font-size: var(--fs-s); }
  .k-val { width: 140px; min-width: 0; font-size: var(--fs-s); }
  .k-var { width: 140px; min-width: 0; font-size: var(--fs-s); }
  .word {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .var-in {
    display: inline-flex;
    align-items: center;
    gap: 2px;
  }
  .brace {
    font-family: var(--font-mono);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .add,
  .add-step {
    align-self: flex-start;
  }
  .report {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .report-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    min-height: 32px;
    padding: 0 12px;
    border-radius: var(--radius-m);
    font-size: var(--fs-m);
    font-weight: 600;
    border: 1px solid var(--border);
  }
  .report-banner.ok { color: var(--success); background: var(--success-soft); }
  .report-banner.fail { color: var(--danger); background: var(--danger-soft); }
  .rb-meta {
    margin-inline-start: auto;
    font-size: var(--fs-s);
    font-weight: 500;
  }
  .run-meta {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .r-step {
    border: 1px solid var(--border);
    border-inline-start-width: 3px;
    border-radius: var(--radius-s);
    padding: 6px 10px;
    display: flex;
    flex-direction: column;
    gap: 4px;
    background: var(--surface);
  }
  .r-step.ok { border-inline-start-color: var(--success); }
  .r-step.fail { border-inline-start-color: var(--danger); }
  .r-top {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-m);
  }
  .r-step.ok .r-top :global(svg) { color: var(--success); }
  .r-step.fail .r-top :global(svg) { color: var(--danger); }
  .r-idx {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .r-name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .r-dur {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .r-error {
    color: var(--danger);
    white-space: pre-wrap;
    word-break: break-word;
    padding-inline-start: 22px;
  }
  .r-assert {
    display: flex;
    gap: 8px;
    font-size: var(--fs-s);
    padding-inline-start: 22px;
    min-width: 0;
  }
  .r-word {
    font-weight: 600;
    flex-shrink: 0;
  }
  .r-assert.ok .r-word { color: var(--success); }
  .r-assert.fail .r-word { color: var(--danger); }
  .r-desc {
    color: var(--text);
    min-width: 0;
    overflow-wrap: anywhere;
  }
  .snap {
    max-height: 240px;
    overflow: auto;
    margin: 0;
    padding: 8px;
    background: var(--surface-2);
    border-radius: var(--radius-s);
  }
  .err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  .runs {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .run-row {
    text-align: start;
    height: 28px;
    padding: 0 8px;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .run-row:hover { background: var(--hover); }
  .run-row.sel { background: var(--accent-soft); }
  .runs-actions {
    display: flex;
    gap: 6px;
    margin-top: 4px;
  }
  @media (max-width: 640px) {
    .auto { padding: 12px 14px 24px; }
    .view-head { flex-direction: column; }
    .sub { padding-inline-start: 0; }
  }
</style>
