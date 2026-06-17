<script lang="ts">
  // Collection runner: pick/create an automation, build an ORDERED list of
  // steps (each runs a saved request, with assertions + variable extraction),
  // save, then Run and read the per-step pass/fail report.
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import { apiClient } from '../../lib/stores/apiClient.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import type {
    ApiAssertion,
    ApiAutomation,
    ApiAutomationStep,
    ApiExtract,
    Id,
  } from '../../lib/api/types';

  const canEdit = $derived(ws.myRole !== 'viewer');

  // The automation being edited (a working copy so edits aren't live until Save).
  let selectedId: Id | null = $state(null);
  let name = $state('');
  let steps: ApiAutomationStep[] = $state([]);
  let dirty = $state(false);

  const selected = $derived(
    apiClient.automations.find((a) => a.id === selectedId) ?? null,
  );

  const assertKinds: ApiAssertion['kind'][] = ['status', 'json_path', 'duration_ms'];
  const assertOps: ApiAssertion['op'][] = ['eq', 'ne', 'contains', 'lt', 'gt'];

  function reqName(id: Id): string {
    return apiClient.requests.find((r) => r.id === id)?.name ?? '(deleted request)';
  }
  function reqMethod(id: Id): string {
    return apiClient.requests.find((r) => r.id === id)?.method ?? '';
  }

  // ── selection / lifecycle ──────────────────────────────────────────────────

  function loadInto(a: ApiAutomation): void {
    selectedId = a.id;
    name = a.name;
    steps = a.steps.map((s) => ({
      request_id: s.request_id,
      assertions: s.assertions.map((x) => ({ ...x })),
      extract: s.extract.map((x) => ({ ...x })),
    }));
    dirty = false;
  }

  function select(a: ApiAutomation): void {
    if (a.id === selectedId) return;
    loadInto(a);
  }

  async function create(): Promise<void> {
    if (!canEdit) return;
    const n = await confirmer.promptText('Automation name', {
      title: 'New automation',
      confirmLabel: 'Create',
    });
    if (!n) return;
    const saved = await apiClient.saveAutomation({ name: n, steps: [] });
    if (saved) loadInto(saved);
  }

  async function rename(a: ApiAutomation): Promise<void> {
    if (!canEdit) return;
    const n = await confirmer.promptText('Rename automation', {
      title: 'Rename automation',
      confirmLabel: 'Rename',
      initial: a.name,
    });
    if (!n || n === a.name) return;
    const saved = await apiClient.saveAutomation({ name: n, steps: a.steps }, a.id);
    if (saved && saved.id === selectedId) name = saved.name;
  }

  async function remove(a: ApiAutomation): Promise<void> {
    if (!canEdit) return;
    if (!(await confirmer.ask(`Delete automation “${a.name}”?`, { title: 'Delete automation' }))) return;
    await apiClient.deleteAutomation(a.id);
    if (a.id === selectedId) {
      selectedId = null;
      steps = [];
      name = '';
      dirty = false;
    }
  }

  // ── step editing ────────────────────────────────────────────────────────────

  function firstRequestId(): Id | null {
    return apiClient.requests[0]?.id ?? null;
  }

  function addStep(): void {
    const rid = firstRequestId();
    if (!rid) {
      toasts.error('No saved requests', 'Save a request first, then add it as a step.');
      return;
    }
    steps = [...steps, { request_id: rid, assertions: [], extract: [] }];
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

  function setStepRequest(i: number, request_id: Id): void {
    steps = steps.map((s, idx) => (idx === i ? { ...s, request_id } : s));
    dirty = true;
  }

  // ── assertions ──────────────────────────────────────────────────────────────

  function addAssertion(i: number): void {
    steps = steps.map((s, idx) =>
      idx === i
        ? { ...s, assertions: [...s.assertions, { kind: 'status', op: 'eq', value: '200' }] }
        : s,
    );
    dirty = true;
  }
  function updateAssertion(i: number, a: number, patch: Partial<ApiAssertion>): void {
    steps = steps.map((s, idx) =>
      idx === i
        ? { ...s, assertions: s.assertions.map((x, ai) => (ai === a ? { ...x, ...patch } : x)) }
        : s,
    );
    dirty = true;
  }
  function removeAssertion(i: number, a: number): void {
    steps = steps.map((s, idx) =>
      idx === i ? { ...s, assertions: s.assertions.filter((_, ai) => ai !== a) } : s,
    );
    dirty = true;
  }

  // ── extracts ────────────────────────────────────────────────────────────────

  function addExtract(i: number): void {
    steps = steps.map((s, idx) =>
      idx === i ? { ...s, extract: [...s.extract, { path: '', var: '' }] } : s,
    );
    dirty = true;
  }
  function updateExtract(i: number, e: number, patch: Partial<ApiExtract>): void {
    steps = steps.map((s, idx) =>
      idx === i
        ? { ...s, extract: s.extract.map((x, ei) => (ei === e ? { ...x, ...patch } : x)) }
        : s,
    );
    dirty = true;
  }
  function removeExtract(i: number, e: number): void {
    steps = steps.map((s, idx) =>
      idx === i ? { ...s, extract: s.extract.filter((_, ei) => ei !== e) } : s,
    );
    dirty = true;
  }

  // ── persistence + run ───────────────────────────────────────────────────────

  async function save(): Promise<void> {
    if (!selectedId || !canEdit) return;
    const clean = steps.map((s) => ({
      request_id: s.request_id,
      assertions: s.assertions.map((a) => ({ ...a })),
      // Drop incomplete extract rows (both path + var required).
      extract: s.extract.filter((e) => e.path.trim() !== '' && e.var.trim() !== ''),
    }));
    const saved = await apiClient.saveAutomation({ name: name.trim() || 'Untitled', steps: clean }, selectedId);
    if (saved) {
      dirty = false;
      toasts.success('Automation saved', saved.name);
    }
  }

  async function run(): Promise<void> {
    if (!selectedId) return;
    if (dirty) await save();
    await apiClient.runAutomation(selectedId);
  }

  const lastRun = $derived(apiClient.lastRun);
  const showRun = $derived(lastRun && lastRun.automation_id === selectedId ? lastRun : null);
</script>

<div class="auto-wrap">
  <!-- list of automations -->
  <div class="auto-head">
    <span class="auto-title">Automations</span>
    {#if canEdit}
      <button class="icon-btn" title="New automation" aria-label="New automation" onclick={create}>
        <Icon name="plus" size={13} />
      </button>
    {/if}
  </div>

  {#if apiClient.automations.length === 0}
    <EmptyState
      icon="zap"
      title="No automations"
      body="Chain saved requests into a run, with assertions and value extraction between steps."
      actionLabel={canEdit ? 'New automation' : undefined}
      onaction={canEdit ? create : undefined}
    />
  {:else}
    <div class="auto-list">
      {#each apiClient.automations as a (a.id)}
        <div class="auto-item" class:active={a.id === selectedId}>
          <button class="auto-pick grow" onclick={() => select(a)} title={a.name}>
            <Icon name="zap" size={12} />
            <span class="aname ellipsis grow">{a.name}</span>
            <span class="acount">{a.steps.length}</span>
          </button>
          {#if canEdit}
            <button class="icon-btn" title="Rename" aria-label="Rename" onclick={() => rename(a)}><Icon name="edit" size={11} /></button>
            <button class="icon-btn" title="Delete" aria-label="Delete" onclick={() => remove(a)}><Icon name="trash" size={11} /></button>
          {/if}
        </div>
      {/each}
    </div>
  {/if}

  <!-- editor for the selected automation -->
  {#if selected}
    <div class="editor">
      <div class="ed-row">
        <input
          class="input grow"
          placeholder="Automation name"
          value={name}
          disabled={!canEdit}
          oninput={(e) => { name = (e.currentTarget as HTMLInputElement).value; dirty = true; }}
        />
        <button class="btn small" disabled={!canEdit || apiClient.running} onclick={save}>
          <Icon name="check" size={12} />Save
        </button>
        <button class="btn small primary" disabled={steps.length === 0 || apiClient.running} onclick={run}>
          <Icon name="play" size={12} />{apiClient.running ? 'Running…' : 'Run'}
        </button>
      </div>

      <div class="steps">
        {#each steps as step, i (i)}
          <div class="step">
            <div class="step-top">
              <span class="step-idx">{i + 1}</span>
              <span class="rm rm-{reqMethod(step.request_id).toLowerCase()}">{reqMethod(step.request_id)}</span>
              <select
                class="input grow"
                value={step.request_id}
                disabled={!canEdit}
                onchange={(e) => setStepRequest(i, (e.currentTarget as HTMLSelectElement).value)}
              >
                {#each apiClient.requests as r (r.id)}
                  <option value={r.id}>{r.name} — {r.method} {r.url}</option>
                {/each}
                {#if !apiClient.requests.some((r) => r.id === step.request_id)}
                  <option value={step.request_id}>{reqName(step.request_id)}</option>
                {/if}
              </select>
              {#if canEdit}
                <button class="icon-btn" title="Move up" aria-label="Move up" disabled={i === 0} onclick={() => moveStep(i, -1)}><Icon name="arrowUp" size={12} /></button>
                <button class="icon-btn" title="Move down" aria-label="Move down" disabled={i === steps.length - 1} onclick={() => moveStep(i, 1)}><Icon name="arrowDown" size={12} /></button>
                <button class="icon-btn" title="Remove step" aria-label="Remove step" onclick={() => removeStep(i)}><Icon name="trash" size={12} /></button>
              {/if}
            </div>

            <!-- assertions -->
            <div class="mini">
              <div class="mini-head">
                <span class="mini-title">Assertions</span>
                {#if canEdit}
                  <button class="btn small ghost" onclick={() => addAssertion(i)}><Icon name="plus" size={11} />Add</button>
                {/if}
              </div>
              {#if step.assertions.length === 0}
                <div class="mini-empty">No assertions — step passes on a 2xx response.</div>
              {:else}
                {#each step.assertions as as, ai (ai)}
                  <div class="mini-row">
                    <select class="input k-kind" value={as.kind} disabled={!canEdit} onchange={(e) => updateAssertion(i, ai, { kind: (e.currentTarget as HTMLSelectElement).value as ApiAssertion['kind'] })}>
                      {#each assertKinds as k (k)}<option value={k}>{k}</option>{/each}
                    </select>
                    {#if as.kind === 'json_path'}
                      <input class="input k-path mono" placeholder="$.data.id" value={as.path ?? ''} disabled={!canEdit} oninput={(e) => updateAssertion(i, ai, { path: (e.currentTarget as HTMLInputElement).value })} />
                    {/if}
                    <select class="input k-op" value={as.op} disabled={!canEdit} onchange={(e) => updateAssertion(i, ai, { op: (e.currentTarget as HTMLSelectElement).value as ApiAssertion['op'] })}>
                      {#each assertOps as o (o)}<option value={o}>{o}</option>{/each}
                    </select>
                    <input class="input k-val mono grow" placeholder="value" value={as.value} disabled={!canEdit} oninput={(e) => updateAssertion(i, ai, { value: (e.currentTarget as HTMLInputElement).value })} />
                    {#if canEdit}
                      <button class="icon-btn" title="Remove" aria-label="Remove assertion" onclick={() => removeAssertion(i, ai)}><Icon name="x" size={11} /></button>
                    {/if}
                  </div>
                {/each}
              {/if}
            </div>

            <!-- extracts -->
            <div class="mini">
              <div class="mini-head">
                <span class="mini-title">Extract → env var</span>
                {#if canEdit}
                  <button class="btn small ghost" onclick={() => addExtract(i)}><Icon name="plus" size={11} />Add</button>
                {/if}
              </div>
              {#if step.extract.length === 0}
                <div class="mini-empty">No extracts. Pull a value into a {'{{var}}'} for later steps.</div>
              {:else}
                {#each step.extract as ex, ei (ei)}
                  <div class="mini-row">
                    <input class="input k-path mono grow" placeholder="$.token" value={ex.path} disabled={!canEdit} oninput={(e) => updateExtract(i, ei, { path: (e.currentTarget as HTMLInputElement).value })} />
                    <span class="arrow">→</span>
                    <input class="input k-var mono" placeholder="var_name" value={ex.var} disabled={!canEdit} oninput={(e) => updateExtract(i, ei, { var: (e.currentTarget as HTMLInputElement).value })} />
                    {#if canEdit}
                      <button class="icon-btn" title="Remove" aria-label="Remove extract" onclick={() => removeExtract(i, ei)}><Icon name="x" size={11} /></button>
                    {/if}
                  </div>
                {/each}
              {/if}
            </div>
          </div>
        {/each}

        {#if steps.length === 0}
          <div class="mini-empty pad">No steps yet. Add a saved request to begin.</div>
        {/if}

        {#if canEdit}
          <button class="btn small add-step" onclick={addStep}><Icon name="plus" size={12} />Add step</button>
        {/if}
      </div>

      <!-- run report -->
      {#if showRun}
        <div class="report">
          <div class="report-banner" class:ok={showRun.passed} class:fail={!showRun.passed}>
            <Icon name={showRun.passed ? 'check' : 'x'} size={13} />
            {showRun.passed ? 'All steps passed' : 'Run failed'}
            <span class="rb-meta">{showRun.steps.filter((s) => s.ok).length}/{showRun.steps.length} steps</span>
          </div>
          {#each showRun.steps as r, ri (ri)}
            <div class="r-step" class:ok={r.ok} class:fail={!r.ok}>
              <div class="r-top">
                <span class="r-dot" class:ok={r.ok} class:fail={!r.ok}></span>
                <span class="r-idx">{ri + 1}</span>
                <span class="r-name ellipsis grow">{r.name}</span>
                {#if r.status != null}<span class="r-status">{r.status}</span>{/if}
                <span class="r-dur">{r.duration_ms}ms</span>
              </div>
              {#if r.error}
                <div class="r-error mono">{r.error}</div>
              {/if}
              {#each r.assertions as a, ax (ax)}
                <div class="r-assert" class:ok={a.passed} class:fail={!a.passed}>
                  <Icon name={a.passed ? 'check' : 'x'} size={10} />
                  <span class="ellipsis">{a.desc}</span>
                </div>
              {/each}
            </div>
          {/each}
        </div>
      {/if}
    </div>
  {/if}
</div>

<style>
  .auto-wrap {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .auto-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0 2px 6px;
  }
  .auto-title {
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    color: var(--text-dim);
  }
  .auto-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .auto-item {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 28px;
    padding-right: 4px;
    border-radius: var(--radius-s);
  }
  .auto-item:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .auto-item.active {
    background: color-mix(in srgb, var(--accent) 14%, transparent);
  }
  .auto-pick {
    display: flex;
    align-items: center;
    gap: 7px;
    min-width: 0;
    border: none;
    background: transparent;
    color: var(--text);
    cursor: pointer;
    text-align: left;
    height: 100%;
    padding: 0 6px;
  }
  .aname {
    font-size: 12.5px;
    font-weight: 500;
    min-width: 0;
  }
  .acount {
    font-size: 10px;
    color: var(--text-dim);
    flex-shrink: 0;
  }

  .editor {
    display: flex;
    flex-direction: column;
    gap: 10px;
    margin-top: 12px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .ed-row {
    display: flex;
    align-items: center;
    gap: 6px;
  }

  .steps {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .step {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    padding: 8px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .step-top {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .step-idx {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    flex-shrink: 0;
    border-radius: 50%;
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
    font-size: 10px;
    font-weight: 700;
  }
  .rm {
    font-size: 9.5px;
    font-weight: 700;
    font-family: var(--font-mono);
    color: var(--text-dim);
    width: 38px;
    flex-shrink: 0;
    text-align: center;
  }
  .rm-get { color: var(--status-working); }
  .rm-post { color: var(--accent); }
  .rm-put,
  .rm-patch { color: #d2691e; }
  .rm-delete { color: var(--status-exited); }

  .mini {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding-left: 24px;
  }
  .mini-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .mini-title {
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
  }
  .mini-empty {
    font-size: 11px;
    color: var(--text-dim);
  }
  .mini-empty.pad {
    padding: 4px 2px;
  }
  .mini-row {
    display: flex;
    align-items: center;
    gap: 5px;
  }
  .mini-row .input {
    height: 26px;
    font-size: 11.5px;
  }
  .k-kind { flex: 0 0 92px; }
  .k-op { flex: 0 0 76px; }
  .k-path { flex: 0 1 120px; min-width: 0; }
  .k-val { min-width: 0; }
  .k-var { flex: 0 1 120px; min-width: 0; }
  .arrow {
    color: var(--text-dim);
    font-size: 12px;
    flex-shrink: 0;
  }
  .add-step {
    align-self: flex-start;
  }

  .report {
    display: flex;
    flex-direction: column;
    gap: 4px;
    margin-top: 4px;
  }
  .report-banner {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    font-size: 12px;
    font-weight: 600;
  }
  .report-banner .rb-meta {
    margin-left: auto;
    font-size: 11px;
    font-weight: 500;
    opacity: 0.8;
  }
  .report-banner.ok {
    color: var(--status-working);
    background: color-mix(in srgb, var(--status-working) 14%, transparent);
  }
  .report-banner.fail {
    color: var(--status-exited);
    background: color-mix(in srgb, var(--status-exited) 14%, transparent);
  }
  .r-step {
    border: 1px solid var(--border);
    border-left-width: 3px;
    border-radius: var(--radius-s);
    padding: 6px 8px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .r-step.ok { border-left-color: var(--status-working); }
  .r-step.fail { border-left-color: var(--status-exited); }
  .r-top {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .r-dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .r-dot.ok { background: var(--status-working); }
  .r-dot.fail { background: var(--status-exited); }
  .r-idx {
    font-size: 10px;
    font-weight: 700;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .r-name {
    font-size: 12px;
    font-weight: 500;
    min-width: 0;
  }
  .r-status {
    font-size: 10px;
    font-weight: 700;
    padding: 0 6px;
    height: 16px;
    line-height: 16px;
    border-radius: 999px;
    background: var(--surface-2);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .r-dur {
    font-size: 10px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .r-error {
    font-size: 11px;
    color: var(--status-exited);
    padding-left: 14px;
    white-space: pre-wrap;
    word-break: break-word;
  }
  .r-assert {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    padding-left: 14px;
    min-width: 0;
  }
  .r-assert.ok { color: var(--status-working); }
  .r-assert.fail { color: var(--status-exited); }

  .ellipsis {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grow { flex: 1; }
</style>
