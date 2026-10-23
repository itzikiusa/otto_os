<script lang="ts">
  import { plural } from '../../lib/plural';
  // One automation, edited in the main area: an ordered list of saved requests
  // ("steps"). Each step can CHECK its response (assertions) and SAVE values
  // from it into variables that later steps use as {{name}}. Edits are a
  // working copy until Save; Run saves first, then runs in the daemon and
  // polls the saved report.
  import { toastError } from '../../lib/toastError';
  import { router } from '../../lib/router.svelte';
  import { editableAssertion, editableExtract, editableSteps, movedArrow, rowKey, savedSteps, type EditableStep } from './automationInput';
  import { KIND_LABEL, NO_VALUE_OPS, OP_LABEL, RUN_STATUS } from './automationLabels';
  import { focusAfterRowRemoval, rowListOf } from './rowFocus';
  import { announce } from '../../lib/announce.svelte';
  import { tick } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import MethodTag from './MethodTag.svelte';
  import StatusChip from './StatusChip.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { ctxMenu } from '../../lib/contextmenu.svelte';
  import { rel } from '../../lib/stores/now.svelte';
  import { formatSeconds } from '../../lib/metric-format';
  import type { ApiAutomationRun, ApiAssertion, ApiAutomation, ApiExtract, Id } from '../../lib/api/types';

  interface Props {
    automationId: Id;
    /** The automation was deleted (the page goes back to the request view). */
    ondeleted?: () => void;
  }
  let { automationId, ondeleted }: Props = $props();

  const canEdit = $derived(ws.myRole !== 'viewer');
  const automation = $derived(apiClient.automations.find((a) => a.id === automationId) ?? null);

  // Working copy (edits aren't live until Save).
  let steps: EditableStep[] = $state([]);
  let stepsEl = $state<HTMLOListElement | null>(null);
  let dirty = $state(false);
  let environmentId = $state('');
  let stopOnFailure = $state(false);
  let datasetText = $state('');
  let historyRun = $state<ApiAutomationRun | null>(null);
  let loadedFor = $state<string | null>(null);
  let leavePending: Promise<boolean> | null = null;
  export function approveLeave(): Promise<boolean> {
    if (!dirty) return Promise.resolve(true);
    if (leavePending) return leavePending;
    leavePending = (async () => {
      const { value } = await confirmer.choose('This automation has unsaved changes.', {
        title: 'Save automation changes?',
        options: [{label:'Save', value:'save', kind:'primary'}, {label:'Discard', value:'discard', kind:'danger'}],
      });
      if (value === 'save') return save();
      if (value !== 'discard') return false;
      dirty = false;
      return true;
    })().finally(() => { leavePending = null; });
    return leavePending;
  }
  $effect(() => router.guard(() => approveLeave()));

  /** Run-list rows are summaries (no steps / snapshot): load the whole run. */
  async function openHistoryRun(id: string): Promise<void> {
    const run = await apiClient.getAutomationRun(id);
    if (run && run.automation_id === automationId) historyRun = run;
  }

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

  // Id → request, built once per list change (a per-step `.find` re-scanned
  // every saved request for every step on every render).
  const reqById = $derived(new Map(apiClient.requests.map((r) => [r.id, r])));
  const req = (id: Id) => reqById.get(id);
  const reqLabel = (r: { name: string; method: string; url: string }) => `${r.name} · ${r.method} ${r.url}`;

  // One shared, filterable picker instead of a <select> per step holding every
  // saved request as an <option> (20 steps × 3k requests = 60k nodes, all
  // re-diffed on every requests change). The menu searches name/method/url and
  // renders at most 50 matches at a time.
  function pickRequest(e: MouseEvent, i: number): void {
    if (!canEdit) return;
    const current = steps[i]?.request_id;
    ctxMenu.show(
      e,
      apiClient.requests.map((q) => ({
        label: reqLabel(q),
        checked: q.id === current,
        action: () => { if (steps[i] && steps[i].request_id !== q.id) patchStep(i, { request_id: q.id }); },
      })),
      { filter: true, filterPlaceholder: 'Search saved requests', maxVisible: 50 },
    );
  }

  // Run report: render the first STEP_PAGE step results, more on demand (a
  // data-driven run can report 1,000 steps, each re-diffed on every poll).
  const STEP_PAGE = 100;
  let reportLimit = $state(STEP_PAGE);
  // The (large) snapshot JSON is only built while its disclosure is open.
  let snapshotOpen = $state(false);

  function addStep(): void {
    const rid = apiClient.requests[0]?.id;
    if (!rid) return;
    steps = [...steps, { key: rowKey(), request_id: rid, assertions: [editableAssertion({ kind: 'status', op: 'lt', value: '400' })], extract: [] }];
    dirty = true;
  }
  // a11y-6-03: removal keeps focus in the list (the next step's Remove, else
  // the previous one's, else "Add step"); a move keeps it on the moved step.
  function removeStep(i: number): void {
    steps = steps.filter((_, idx) => idx !== i);
    dirty = true;
    void focusAfterRowRemoval(stepsEl?.parentElement ?? null, i, '.add-step', '[data-remove-step]');
  }
  async function moveStep(i: number, dir: -1 | 1): Promise<void> {
    const j = i + dir;
    if (j < 0 || j >= steps.length) return;
    const moved = steps[i].key;
    const next = [...steps];
    [next[i], next[j]] = [next[j], next[i]];
    steps = next;
    dirty = true;
    const { arrow } = movedArrow(i, dir, steps.length);
    await tick();
    stepsEl?.querySelector<HTMLElement>(`[data-step-key="${moved}"] [data-move="${arrow}"]`)?.focus();
    announce(`Step moved to position ${j + 1} of ${steps.length}`);
  }
  function patchStep(i: number, patch: Partial<EditableStep>): void {
    steps = steps.map((s, idx) => (idx === i ? { ...s, ...patch } : s));
    dirty = true;
  }
  function addAssertion(i: number): void {
    patchStep(i, { assertions: [...steps[i].assertions, editableAssertion({ kind: 'status', op: 'eq', value: '200' })] });
  }
  function updateAssertion(i: number, a: number, patch: Partial<ApiAssertion>): void {
    patchStep(i, { assertions: steps[i].assertions.map((x, ai) => (ai === a ? { ...x, ...patch } : x)) });
  }
  function removeAssertion(e: Event, i: number, a: number): void {
    const list = rowListOf(e, '.sub');
    patchStep(i, { assertions: steps[i].assertions.filter((_, ai) => ai !== a) });
    void focusAfterRowRemoval(list, a, '.add');
  }
  function addExtract(i: number): void {
    patchStep(i, { extract: [...steps[i].extract, editableExtract({ path: '', var: '' })] });
  }
  function updateExtract(i: number, e: number, patch: Partial<ApiExtract>): void {
    patchStep(i, { extract: steps[i].extract.map((x, ei) => (ei === e ? { ...x, ...patch } : x)) });
  }
  function removeExtract(ev: Event, i: number, e: number): void {
    const list = rowListOf(ev, '.sub');
    patchStep(i, { extract: steps[i].extract.filter((_, ei) => ei !== e) });
    void focusAfterRowRemoval(list, e, '.add');
  }

  async function save(): Promise<boolean> {
    if (!automation || !canEdit) return false;
    const snapshot = JSON.stringify(steps);
    const clean = savedSteps(steps);
    const saved = await apiClient.saveAutomation({ name: automation.name, steps: clean }, automation.id);
    if (saved) {
      dirty = JSON.stringify(steps) !== snapshot;
      toasts.success('Automation saved', saved.name);
    }
    return !!saved && !dirty;
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

  // Stop the in-flight run: the requests already sent stay sent, the remaining
  // steps never fire. Halting HTTP steps ends no agent work and can simply be
  // re-run, so it is a neutral Stop without a confirm; a failure says so.
  let stopping = $state(false);
  async function stopRun(runId: Id): Promise<void> {
    if (stopping) return;
    stopping = true;
    try {
      await apiClient.cancelAutomationRun(runId);
      await apiClient.loadAutomationRuns(automationId);
    } catch (e) {
      toastError('Couldn’t stop the run', e);
    } finally {
      stopping = false;
    }
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
  /** The live run's polls kept failing: it may still be running server-side,
   *  so its partial report is not a verdict ("Last run failed" was wrong). */
  // a11y-6-01: the run error line is plain text; the live run's error is
  // spoken once, assertively, where it first appears (an opened past run is
  // read on demand, not announced).
  let spokenRunError = '';
  $effect(() => {
    const err = !historyRun && runDetails?.automation_id === automationId ? runDetails.error ?? '' : '';
    if (err && err !== spokenRunError) announce(`Run failed: ${err}`, { assertive: true });
    spokenRunError = err;
  });
  const lost = $derived(!historyRun && !apiClient.running && !!showRun && apiClient.runLost != null && apiClient.currentRun?.id === apiClient.runLost);
</script>

{#if !automation}
  <EmptyState icon="zap" title="This automation no longer exists" body="It may have been deleted. Pick another one from the list." />
{:else}
  <div class="auto">
    <header class="view-head">
      <div class="vh-text">
        <h2>{automation.name}{#if dirty}<span class="unsaved" title="Unsaved changes"> · edited</span>{/if}</h2>
        <p>Runs saved requests in order. Each step can <strong>check</strong> the response and <strong>save values</strong> from it (like a token or an id) as <code>{'{{variables}}'}</code> for the steps after it. Saved values only live for the run; environments aren’t changed.</p>
        <p>Run sends {steps.length} saved {steps.length === 1 ? 'request' : 'requests'} per dataset row to their configured URLs, using <strong>{apiClient.environments.find(e => e.id === environmentId)?.name || apiClient.activeEnv?.name || 'no environment'}</strong>. These calls leave this Mac and can change data on the destination servers.</p>
      </div>
      <div class="vh-actions">
        {#if showRun}
          <button class="chip result" class:ok={!apiClient.running && !lost && showRun.passed} class:bad={!apiClient.running && !lost && !showRun.passed} onclick={showReport} title={lost ? 'Lost track of this run — refresh run history to see how it ended' : 'Show the last run’s results'}>
            {apiClient.running ? 'Running…' : lost ? 'Lost track of this run' : showRun.passed ? 'Last run passed' : 'Last run failed'} · {showRun.steps.filter((s) => s.ok).length}/{showRun.steps.length}
          </button>
        {/if}
        <button class="icon-btn" onclick={menu} aria-label="Automation options" title="Automation options"><Icon name="more" size={14} /></button>
        <button class="btn small" onclick={() => void save()} disabled={!canEdit || !dirty || apiClient.running}>Save</button>
        {#if runDetails?.status === 'running' && runDetails.automation_id === automationId}
          <button class="btn small" onclick={() => void stopRun(runDetails!.id)} disabled={stopping} data-testid="automation-stop" title="Stop the run — steps already sent stay sent"><Icon name="stop" size={12} />{stopping ? 'Stopping…' : 'Stop run'}</button>
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
      <textarea dir="ltr" class="input mono" aria-label="Dataset rows" rows="4" bind:value={datasetText} placeholder={'[{"customer_id":"cus_1"},{"customer_id":"cus_2"}]'} disabled={apiClient.running}></textarea>
      {#if datasetError}<p class="err" role="alert">{datasetError}</p>{/if}
    </details>

    <ol class="steps" bind:this={stepsEl}>
      {#each steps as step, i (step.key)}
        {@const r = req(step.request_id)}
        <li class="step" data-step-key={step.key}>
          <div class="step-top">
            <span class="step-idx" aria-hidden="true">{i + 1}</span>
            <MethodTag method={r?.method ?? ''} fixed />
            <button type="button" class="input grow req-pick" aria-label="Request for step {i + 1}" aria-haspopup="menu" disabled={!canEdit}
              title={r ? reqLabel(r) : 'Deleted request'} onclick={(e) => pickRequest(e, i)}>
              <span class="req-pick-label">{r ? reqLabel(r) : 'Deleted request'}</span>
              <Icon name="chevronDown" size={12} />
            </button>
            {#if canEdit}
              <button class="icon-btn" title="Move up" aria-label="Move step {i + 1} up" data-move="up" disabled={i === 0} onclick={() => void moveStep(i, -1)}><Icon name="arrowUp" size={14} /></button>
              <button class="icon-btn" title="Move down" aria-label="Move step {i + 1} down" data-move="down" disabled={i === steps.length - 1} onclick={() => void moveStep(i, 1)}><Icon name="arrowDown" size={14} /></button>
              <button class="icon-btn" title="Remove step" aria-label="Remove step {i + 1}" data-remove-step onclick={() => removeStep(i)}><Icon name="trash" size={14} /></button>
            {/if}
          </div>

          <div class="sub">
            <div class="sub-title">Check that</div>
            {#if step.assertions.length === 0}
              <p class="sub-empty">No checks, so the step passes on any 2xx response.</p>
            {/if}
            {#each step.assertions as as, ai (as.key)}
              <div class="rule">
                <select class="input k-kind" aria-label="What to check" value={as.kind} disabled={!canEdit}
                  onchange={(e) => updateAssertion(i, ai, { kind: (e.currentTarget as HTMLSelectElement).value as ApiAssertion['kind'] })}>
                  {#each Object.entries(KIND_LABEL) as [k, label] (k)}<option value={k}>{label}</option>{/each}
                </select>
                {#if as.kind === 'json_path'}
                  <input dir="ltr" class="input mono k-path" aria-label="JSONPath" placeholder="$.data.id" value={as.path ?? ''} disabled={!canEdit}
                    oninput={(e) => updateAssertion(i, ai, { path: (e.currentTarget as HTMLInputElement).value })} />
                {:else if as.kind === 'header'}
                  <input dir="ltr" class="input mono k-path" aria-label="Header name" placeholder="Content-Type" value={as.path ?? ''} disabled={!canEdit}
                    oninput={(e) => updateAssertion(i, ai, { path: (e.currentTarget as HTMLInputElement).value })} />
                {/if}
                <select class="input k-op" aria-label="Comparison" value={as.op} disabled={!canEdit}
                  onchange={(e) => updateAssertion(i, ai, { op: (e.currentTarget as HTMLSelectElement).value as ApiAssertion['op'] })}>
                  {#each Object.entries(OP_LABEL) as [o, label] (o)}<option value={o}>{label}</option>{/each}
                </select>
                {#if !NO_VALUE_OPS.has(as.op)}
                  <input dir="ltr" class="input mono k-val" aria-label={as.op === 'matches' ? 'Regular expression' : 'Expected value'}
                    placeholder={as.op === 'matches' ? '^ORD-\\d+' : '200'} value={as.value} disabled={!canEdit}
                    oninput={(e) => updateAssertion(i, ai, { value: (e.currentTarget as HTMLInputElement).value })} />
                {/if}
                {#if canEdit}
                  <button class="icon-btn" title="Remove check" aria-label="Remove check on {as.path || KIND_LABEL[as.kind]} in step {i + 1}" data-remove-row onclick={(e) => removeAssertion(e, i, ai)}><Icon name="x" size={12} /></button>
                {/if}
              </div>
            {/each}
            {#if canEdit}<button class="btn ghost small add" onclick={() => addAssertion(i)}><Icon name="plus" size={12} />Add check</button>{/if}
          </div>

          <div class="sub">
            <div class="sub-title">Save for later steps</div>
            {#each step.extract as ex, ei (ex.key)}
              <div class="rule">
                <span class="word">Take</span>
                <input dir="ltr" class="input mono k-path" aria-label="JSONPath, header:Name or status to take" title="A JSONPath into the body ($.access_token), header:Name for a response header, or status" placeholder="$.access_token" value={ex.path} disabled={!canEdit}
                  oninput={(e) => updateExtract(i, ei, { path: (e.currentTarget as HTMLInputElement).value })} />
                <span class="word">as</span>
                <span class="var-in">
                  <span class="brace" aria-hidden="true">{'{{'}</span>
                  <input dir="ltr" class="input mono k-var" aria-label="Variable name" placeholder="api_token" value={ex.var} disabled={!canEdit}
                    oninput={(e) => updateExtract(i, ei, { var: (e.currentTarget as HTMLInputElement).value })} />
                  <span class="brace" aria-hidden="true">{'}}'}</span>
                </span>
                {#if canEdit}
                  <button class="icon-btn" title="Remove" aria-label="Remove saved value {ex.var || ex.path || ei + 1} in step {i + 1}" data-remove-row onclick={(e) => removeExtract(e, i, ei)}><Icon name="x" size={12} /></button>
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
        <div class="report-banner" class:ok={!lost && showRun.passed} class:fail={!lost && !showRun.passed}>
          <Icon name={!lost && showRun.passed ? 'check' : 'warning'} size={14} />
          {apiClient.running ? 'Running…' : lost ? 'Lost track of this run — refresh run history to see how it ended' : showRun.passed ? 'All steps passed' : 'Run failed'}
          <span class="rb-meta">{showRun.steps.filter((s) => s.ok).length} of {showRun.steps.length} steps passed</span>
        </div>
        {#if runDetails && runDetails.automation_id === automationId}
          <p class="run-meta">
            Run <span class="mono" title="Run id">{runDetails.id}</span> · {RUN_STATUS[runDetails.status]} · {rel(runDetails.created_at)}{#if runDetails.dataset_rows}{' '}· {plural(runDetails.dataset_rows, 'data row')}{/if}
          </p>
          {#if runDetails.error}<p class="err">{runDetails.error}</p>{/if}
        {/if}
        {#each showRun.steps.length > reportLimit ? showRun.steps.slice(0, reportLimit) : showRun.steps as r, ri (ri)}<!-- ui-guards: allow (read-only run report; never reordered) -->
          <div class="r-step" class:ok={r.ok} class:fail={!r.ok}>
            <div class="r-top">
              <Icon name={r.ok ? 'check' : 'x'} size={12} />
              <span class="r-idx">{ri + 1}</span>
              <span class="r-name">{r.name}</span>
              <StatusChip status={r.status} small />
              <span class="r-dur">{formatSeconds(r.duration_ms / 1000)}</span>
            </div>
            {#if r.error}<div class="r-error mono">{r.error}</div>{/if}
            {#each r.assertions as a, ax (ax)}<!-- ui-guards: allow (read-only run report) -->
              <div class="r-assert" class:ok={a.passed} class:fail={!a.passed}>
                <span class="r-word">{a.passed ? 'Passed' : 'Failed'}</span><span class="r-desc">{a.desc}</span>
              </div>
            {/each}
          </div>
        {/each}
        {#if showRun.steps.length > reportLimit}
          <button class="btn ghost small" onclick={() => (reportLimit += STEP_PAGE)}>
            Show {Math.min(STEP_PAGE, showRun.steps.length - reportLimit)} more of {showRun.steps.length - reportLimit} remaining steps
          </button>
        {/if}
        {#if runDetails && runDetails.automation_id === automationId}
          <details class="disclosure" bind:open={snapshotOpen}><summary>Request versions used by this run</summary>{#if snapshotOpen}<pre class="snap mono">{JSON.stringify(runDetails.snapshot, null, 2)}</pre>{/if}</details>
        {/if}
      </section>
    {/if}

    <details class="disclosure">
      <summary>Run history ({runs.length})</summary>
      <div class="runs">
        <LoadState
          what="run history"
          variant="compact"
          loading={apiClient.runsLoading}
          error={apiClient.runsError}
          empty={runs.length === 0}
          onretry={() => void apiClient.loadAutomationRuns(automationId)}
        >
          {#snippet emptyView()}<p class="sub-empty">No runs yet.</p>{/snippet}
          {#each runs as savedRun (savedRun.id)}
            <button class="run-row" class:sel={historyRun?.id === savedRun.id} onclick={() => void openHistoryRun(savedRun.id)} title={new Date(savedRun.created_at).toLocaleString()}>
              {RUN_STATUS[savedRun.status]} · {savedRun.steps_total ?? savedRun.report.steps.length} steps · {rel(savedRun.created_at)}
            </button>
          {/each}
        </LoadState>
        <div class="runs-actions">
          <button class="btn ghost small" onclick={async () => { await apiClient.loadAutomationRuns(automationId); if (historyRun) await openHistoryRun(historyRun.id); }}>
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
  .req-pick {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    text-align: start;
  }
  .req-pick:disabled {
    cursor: default;
  }
  .req-pick-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .req-pick:focus-visible {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
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
