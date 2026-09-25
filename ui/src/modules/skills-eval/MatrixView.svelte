<script lang="ts">
  // Eval-lab matrix: provider × skill × prompt comparison. A left list of
  // matrices (+ a "New matrix" form), and a grid of scored cells for the
  // selected matrix — one row per prompt, one column per provider·skill pair,
  // the highest-scoring cell in each row highlighted as the winner.
  import { ws } from '../../lib/stores/workspace.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus, sentenceCase } from '../../lib/status';
  import { rel } from '../../lib/stores/now.svelte';
  import type { EvalMatrix, MatrixCell, MatrixPrompt, StartMatrixReq } from '../../lib/api/types';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';

  let { onopenrun } = $props<{ onopenrun?: (evalId: string) => void }>();

  let matrices: EvalMatrix[] = $state([]);
  let loading = $state(true);
  // Inline list-load failure + Retry (instead of a toast over "No matrices yet").
  let loadError: string | null = $state(null);
  let selected: EvalMatrix | null = $state(null);
  let selectedId: string | null = $state(null);
  let showForm = $state(false);
  let creating = $state(false);
  // The selected matrix's own load: shown inline in the detail pane (a toast
  // left the pane on the generic "Eval matrix" intro, as if nothing existed).
  let detailError: string | null = $state(null);
  let detailLoading = $state(false);

  // --- New-matrix form fields --------------------------------------------
  let fName = $state('');
  // Providers are picked from the live registry (a typo'd free-text name used
  // to fan out cells that could only fail).
  const providerOpts = $derived(agentProviders());
  let fProviders = $state<string[]>([defaultAgentProvider()]);
  function toggleProvider(p: string): void {
    fProviders = fProviders.includes(p) ? fProviders.filter((x) => x !== p) : [...fProviders, p];
  }
  let fSkills = $state(''); // comma-separated skill names
  let fTestCmd = $state('');
  let fIterations = $state(1);
  let fPrompts: MatrixPrompt[] = $state([{ label: '', task: '' }]);

  // Load the workspace's matrices whenever the active workspace changes.
  $effect(() => {
    const wsId = ws.currentId;
    // A workspace switch drops the previous workspace's open matrix.
    selected = null;
    selectedId = null;
    if (wsId) {
      void loadList(wsId);
    } else {
      loading = false;
      matrices = [];
    }
  });

  async function loadList(wsId: string): Promise<void> {
    loading = true;
    loadError = null;
    try {
      matrices = await skillsEvalApi.listMatrices(wsId);
      // Open on the newest matrix rather than an empty "pick one" pane.
      if (!showForm && selectedId === null && matrices.length > 0) void selectMatrix(matrices[0].id);
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function selectMatrix(id: string): Promise<void> {
    selectedId = id;
    showForm = false;
    detailError = null;
    if (selected?.id !== id) selected = null;
    detailLoading = true;
    try {
      const m = await skillsEvalApi.getMatrix(id);
      if (selectedId === id) selected = m;
      syncListEntry(m);
    } catch (e) {
      if (selectedId === id) detailError = e instanceof Error ? e.message : String(e);
    } finally {
      if (selectedId === id) detailLoading = false;
    }
  }

  /** Leave the form: back to the matrix that was open, else the newest one. */
  function closeForm(): void {
    showForm = false;
    if (matrices.length > 0) void selectMatrix(matrices[0].id);
  }

  function syncListEntry(m: EvalMatrix): void {
    matrices = matrices.map((x) => (x.id === m.id ? m : x));
  }

  // Poll the selected matrix while it's running. Keyed on a derived primitive so
  // a poll that returns the same id+status does NOT churn the interval.
  const pollKey = $derived.by(() => {
    const s = selected;
    return s && s.status === 'running' ? s.id : null;
  });
  $effect(() => {
    const id = pollKey;
    if (!id) return;
    const timer = setInterval(() => void refreshSelected(id), 2500);
    return () => clearInterval(timer);
  });

  async function refreshSelected(id: string): Promise<void> {
    try {
      const m = await skillsEvalApi.getMatrix(id);
      if (selectedId === id) selected = m;
      syncListEntry(m);
    } catch {
      /* transient — next tick retries */
    }
  }

  async function cancel(): Promise<void> {
    if (!selected) return;
    if (
      !(await confirmer.ask(`Stop the matrix "${selected.name}"? Cells still running are abandoned; scored cells are kept.`, {
        title: 'Stop matrix',
        confirmLabel: 'Stop matrix',
      }))
    )
      return;
    try {
      const m = await skillsEvalApi.cancelMatrix(selected.id);
      selected = m;
      syncListEntry(m);
    } catch (e) {
      toasts.error("Couldn't stop the matrix", e instanceof Error ? e.message : String(e));
    }
  }

  // --- Form actions ------------------------------------------------------
  function addPrompt(): void {
    fPrompts = [...fPrompts, { label: '', task: '' }];
  }
  function removePrompt(i: number): void {
    fPrompts = fPrompts.filter((_, idx) => idx !== i);
  }
  function openForm(): void {
    showForm = true;
    selectedId = null;
    selected = null;
  }

  function parseList(s: string): string[] {
    return s
      .split(',')
      .map((x) => x.trim())
      .filter((x) => x.length > 0);
  }

  const canCreate = $derived(
    !creating &&
      fProviders.length > 0 &&
      parseList(fSkills).length > 0 &&
      fPrompts.some((p) => p.task.trim().length > 0),
  );

  // Why "Create matrix" is disabled, for its tooltip.
  const createBlock = $derived.by(() => {
    if (creating) return 'Creating…';
    if (fProviders.length === 0) return 'Pick at least one agent';
    if (parseList(fSkills).length === 0) return 'Add at least one skill';
    if (!fPrompts.some((p) => p.task.trim())) return 'Add at least one prompt with a task';
    return '';
  });

  async function create(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || creating) return;
    const providers = providerOpts.filter((p) => fProviders.includes(p));
    const skillNames = parseList(fSkills);
    const prompts: MatrixPrompt[] = fPrompts
      .map((p) => ({
        label: p.label.trim() || p.task.trim().slice(0, 48),
        task: p.task.trim(),
      }))
      .filter((p) => p.task.length > 0);

    // Create stays disabled until these hold (see canCreate / createBlock).
    if (providers.length === 0 || skillNames.length === 0 || prompts.length === 0) return;

    const body: StartMatrixReq = {
      name: fName.trim() || 'Untitled matrix',
      mode: 'score_only',
      providers,
      skills: skillNames.map((reference) => ({ kind: 'library', reference })),
      prompts,
      target: { kind: 'working' },
      test_cmd: fTestCmd.trim() || null,
      iterations: Math.max(1, Math.floor(fIterations)),
    };

    creating = true;
    try {
      const m = await skillsEvalApi.createMatrix(wsId, body);
      matrices = [m, ...matrices];
      await selectMatrix(m.id);
      toasts.success('Matrix created', 'Cells score the working tree in the background.');
    } catch (e) {
      toasts.error("Couldn't create the matrix", e instanceof Error ? e.message : String(e));
    } finally {
      creating = false;
    }
  }

  // --- Grid derivations --------------------------------------------------
  const promptRows = $derived.by<string[]>(() => {
    if (!selected) return [];
    if (selected.prompts.length > 0) return selected.prompts.map((p) => p.label);
    const set = new Set<string>();
    for (const c of selected.cells) set.add(c.prompt);
    return [...set];
  });

  const cols = $derived.by<{ provider: string; skill: string }[]>(() => {
    if (!selected) return [];
    const seen = new Map<string, { provider: string; skill: string }>();
    for (const c of selected.cells) {
      const key = `${c.provider}\u0000${c.skill}`;
      if (!seen.has(key)) seen.set(key, { provider: c.provider, skill: c.skill });
    }
    return [...seen.values()];
  });

  function cellAt(prompt: string, provider: string, skill: string): MatrixCell | undefined {
    return selected?.cells.find(
      (c) => c.prompt === prompt && c.provider === provider && c.skill === skill,
    );
  }

  // eval_id of the winning (max composite) cell within a prompt row.
  function rowWinnerId(prompt: string): string | null {
    if (!selected) return null;
    let best: MatrixCell | null = null;
    for (const c of selected.cells) {
      if (c.prompt !== prompt || c.composite_score == null) continue;
      if (!best || (best.composite_score ?? -Infinity) < c.composite_score) best = c;
    }
    return best ? best.eval_id : null;
  }

  function proofClass(status: string | undefined): string {
    switch (status) {
      case 'passed':
        return 'pf-pass';
      case 'failed':
        return 'pf-fail';
      case 'partial':
        return 'pf-partial';
      default:
        return 'pf-none';
    }
  }

</script>

<div class="mx" data-testid="matrix-view">
  <aside class="mx-side">
    <div class="mx-side-head">
      <span class="mx-side-title">Matrices</span>
      <!-- Not .primary: the form's "Create matrix" / the empty state's CTA is the view's primary. -->
      <button class="btn small" data-testid="matrix-new-btn" onclick={openForm} aria-pressed={showForm}>
        <Icon name="plus" size={12} /> New matrix
      </button>
    </div>
    <div class="mx-list">
      {#if !ws.currentId}
        <div class="mx-muted">No workspace selected.</div>
      {:else if loading && matrices.length === 0}
        <div class="mx-muted" role="status">Loading matrices…</div>
      {:else if loadError && matrices.length === 0}
        <div class="mx-muted mx-err" role="alert">
          <span><Icon name="warning" size={12} /> <strong>Couldn't load matrices.</strong> <span class="mx-err-detail">{loadError}</span></span>
          <button class="btn small" onclick={() => ws.currentId && loadList(ws.currentId)} disabled={loading}>{loading ? 'Retrying…' : 'Retry'}</button>
        </div>
      {:else if matrices.length === 0}
        <div class="mx-muted">No matrices yet.</div>
      {:else}
        {#each matrices as m (m.id)}
          <button
            class="mx-item"
            aria-current={!showForm && selectedId === m.id ? 'true' : undefined}
            class:active={!showForm && selectedId === m.id}
            onclick={() => selectMatrix(m.id)}
          >
            <div class="mx-item-top">
              <span class="mx-item-name" title={m.name}>{m.name}</span>
              <span class="mx-dot st-{m.status}" role="img" aria-label={runStatus(m.status).label} title={runStatus(m.status).label}></span>
            </div>
            <div class="mx-item-meta">
              <span title="{m.providers.length} providers × {m.skills.length} skills × {m.prompts.length} prompts">{m.providers.length} × {m.skills.length} × {m.prompts.length}</span>
              <span class="grow"></span>
              <span title={new Date(m.created_at).toLocaleString()}>{rel(m.created_at)}</span>
            </div>
          </button>
        {/each}
      {/if}
    </div>
  </aside>

  <main class="mx-main">
    {#if !ws.currentId}
      <EmptyState icon="grid" title="No workspace selected" body="Pick a workspace to build a matrix." />
    {:else if showForm}
      <div class="mx-form">
        <h2>New matrix</h2>
        <p class="lede">
          A provider × skill × prompt grid — each cell is a scored run. Matrices currently score the
          workspace's working tree (score only; no improvement iterations).
        </p>

        <section class="card block">
          <div class="fld">
            <label class="field-label" for="mx-name">Name</label>
            <input id="mx-name" class="input" data-testid="matrix-name" placeholder="e.g. Logging skill bake-off" bind:value={fName} />
          </div>
          <div class="fld">
            <span class="field-label" id="mx-prov-lbl">Agents</span>
            <div class="provider-chips" role="group" aria-labelledby="mx-prov-lbl" data-testid="matrix-providers">
              {#each providerOpts as p (p)}
                <label class="chip-toggle" class:on={fProviders.includes(p)}>
                  <input type="checkbox" checked={fProviders.includes(p)} onchange={() => toggleProvider(p)} />
                  <span class="mono">{p}</span>
                </label>
              {/each}
            </div>
          </div>
          <div class="fld">
            <label class="field-label" for="mx-skills">Skills <span class="hint-inline">library skill names, comma-separated</span></label>
            <input id="mx-skills" class="input" data-testid="matrix-skills" placeholder="golang-testing, golang-code-review" bind:value={fSkills} />
          </div>
          <div class="grid2">
            <div class="fld">
              <label class="field-label" for="mx-test">Test command <span class="hint-inline">optional</span></label>
              <input id="mx-test" class="input" data-testid="matrix-test-cmd" placeholder="go test ./..." bind:value={fTestCmd} />
            </div>
            <div class="fld">
              <label class="field-label" for="mx-iter">Iterations</label>
              <input id="mx-iter" class="input" type="number" min="1" max="10" bind:value={fIterations} />
            </div>
          </div>
        </section>

        <section class="card block">
          <div class="block-head">
            <span class="field-label">Prompts</span>
            <span class="grow"></span>
            <button class="btn small" type="button" onclick={addPrompt}>
              <Icon name="plus" size={12} /> Add prompt
            </button>
          </div>
          {#each fPrompts as p, i (i)}
            <div class="prompt">
              <div class="row">
                <input class="input grow" data-testid="matrix-prompt-label" aria-label="Prompt {i + 1} label" placeholder="Happy path" bind:value={p.label} />
                {#if fPrompts.length > 1}
                  <button class="icon-btn" type="button" title="Remove prompt" aria-label="Remove prompt {i + 1}" onclick={() => removePrompt(i)}>
                    <Icon name="trash" size={14} />
                  </button>
                {/if}
              </div>
              <textarea
                class="input"
                rows="2"
                data-testid="matrix-prompt-task"
                aria-label="Prompt {i + 1} task"
                placeholder="What the agent should do…"
                bind:value={p.task}
              ></textarea>
            </div>
          {/each}
        </section>

        <div class="actions">
          <span class="grow"></span>
          <button class="btn" type="button" onclick={closeForm}>Cancel</button>
          <button class="btn primary" data-testid="matrix-create" disabled={!canCreate} title={canCreate ? undefined : createBlock} onclick={create}>
            {creating ? 'Creating…' : 'Create matrix'}
          </button>
        </div>
      </div>
    {:else if selectedId && detailError}
      <div class="mx-detail-err" role="alert">
        <Icon name="warning" size={22} />
        <strong>Couldn't load this matrix</strong>
        <p>{detailError}</p>
        <button class="btn small" onclick={() => selectedId && selectMatrix(selectedId)} disabled={detailLoading}><Icon name="refresh" size={12} /> {detailLoading ? 'Retrying…' : 'Retry'}</button>
      </div>
    {:else if selectedId && !selected}
      <div class="mx-muted mx-pad" role="status">Loading matrix…</div>
    {:else if selected}
      <div class="mx-detail">
        <div class="mx-detail-head">
          <div class="mx-detail-title">
            <h2>{selected.name}</h2>
            <span class="mx-sub">{sentenceCase(selected.mode)} · {selected.repo_key && selected.repo_key !== ws.currentId ? selected.repo_key : 'This workspace'}</span>
          </div>
          <span class="grow"></span>
          <StatusBadge status={runStatus(selected.status)} />
          {#if selected.status === 'running'}
            <button class="btn small" type="button" onclick={cancel}>
              <Icon name="square" size={12} /> Stop
            </button>
          {/if}
        </div>

        {#if cols.length === 0}
          <EmptyState icon="grid" title="No cells yet" body="Cells appear as the matrix begins running." />
        {:else}
          <div class="table-wrap">
            <table data-testid="matrix-grid">
              <thead>
                <tr>
                  <th class="rowlabel" scope="col"><span class="sr-only">Prompt</span></th>
                  {#each cols as col (col.provider + '\u0000' + col.skill)}
                    <th scope="col">
                      <div class="col-prov">{col.provider}</div>
                      <div class="col-skill mono">{col.skill}</div>
                    </th>
                  {/each}
                </tr>
              </thead>
              <tbody>
                {#each promptRows as prompt (prompt)}
                  {@const winnerId = rowWinnerId(prompt)}
                  <tr>
                    <th class="rowlabel" scope="row">{prompt}</th>
                    {#each cols as col (col.provider + '\u0000' + col.skill)}
                      {@const cell = cellAt(prompt, col.provider, col.skill)}
                      <td
                        class="cell"
                        class:winner={cell != null && winnerId != null && cell.eval_id === winnerId}
                        data-testid="matrix-cell"
                      >
                        {#snippet cellBody(c: MatrixCell)}
                          {#if c.composite_score != null}
                            <span class="score">{#if winnerId != null && c.eval_id === winnerId}<Icon name="check" size={12} /><span class="sr-only">Top score: </span>{/if}{c.composite_score.toFixed(0)}</span>
                          {:else}
                            <span class="cell-status">{runStatus(c.status).label}</span>
                          {/if}
                          <span class="proof {proofClass(c.proof_status)}" title="Proof pack status">{sentenceCase(c.proof_status || 'missing')}</span>
                        {/snippet}
                        {#if cell?.eval_id}
                          {@const evalId = cell.eval_id}
                          <button
                            type="button"
                            class="cell-in clickable"
                            title={winnerId != null && cell.eval_id === winnerId ? 'Top score in this row · open this run' : 'Open this run'}
                            onclick={() => onopenrun?.(evalId)}
                          >{@render cellBody(cell)}</button>
                        {:else if cell}
                          <div class="cell-in">{@render cellBody(cell)}</div>
                        {:else}
                          <span class="dash">—</span>
                        {/if}
                      </td>
                    {/each}
                  </tr>
                {/each}
              </tbody>
            </table>
          </div>
        {/if}
      </div>
    {:else}
      <EmptyState
        icon="grid"
        title="No matrices yet"
        body="A matrix scores several providers and skills against the same prompts in one grid, so you can see which combination does best."
        actionLabel="New matrix"
        actionIcon="plus"
        onaction={openForm}
      />
    {/if}
  </main>
</div>

<style>
  .mx {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .mx-side {
    width: 280px;
    flex-shrink: 0;
    background: var(--surface);
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .mx-side-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 12px 8px;
  }
  .mx-side-title {
    font-size: var(--fs-s);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
    flex: 1;
  }
  .mx-list {
    flex: 1;
    overflow-y: auto;
    padding: 4px 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .mx-muted {
    padding: 16px 8px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .mx-err {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 8px;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .mx-err :global(svg),
  .mx-detail-err > :global(svg) {
    color: var(--danger);
    vertical-align: -1px;
  }
  .mx-err-detail {
    color: var(--text-dim);
  }
  .mx-pad {
    padding: 30px;
    text-align: center;
  }
  .mx-detail-err {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 40px 20px;
    text-align: center;
    overflow-wrap: anywhere;
  }
  .mx-detail-err p {
    margin: 0 0 6px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  .mx-item {
    text-align: start;
    border: 1px solid transparent;
    background: transparent;
    border-radius: var(--radius-m);
    padding: 8px 10px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .mx-item:hover {
    background: var(--hover);
  }
  /* Selection is accent (same as the Runs list), not the success green. */
  .mx-item.active {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .mx-item-top {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .mx-item-name {
    font-size: var(--fs-s);
    font-weight: 600;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .mx-item-meta {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  /* Same tones as runStatus / the Runs list: running = info (pulsing), done =
     success, error = danger, cancelled/queued = neutral. */
  .mx-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
    background: var(--status-idle);
  }
  .mx-dot.st-running {
    background: var(--info);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .mx-dot.st-done {
    background: var(--status-working);
  }
  .mx-dot.st-error {
    background: var(--status-exited);
  }
  @media (max-width: 640px) {
    .mx {
      flex-direction: column;
    }
    .mx-side {
      width: 100%;
      max-height: 40%;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    .mx-main {
      flex: 1;
      min-height: 0;
    }
    .grid2 {
      grid-template-columns: minmax(0, 1fr);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .mx-dot.st-running {
      animation: none;
    }
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .mx-main {
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }

  /* Form */
  .mx-form > :global(*) {
    max-width: 760px;
    width: 100%;
    box-sizing: border-box;
  }
  .mx-form {
    padding: 18px 20px 60px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    height: 100%;
    overflow-y: auto;
    box-sizing: border-box;
  }
  h2 {
    margin: 0;
    font-size: var(--fs-l);
  }
  .lede {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .block {
    padding: 12px 14px;
    display: flex;
    flex-direction: column;
    gap: 12px;
  }
  .fld {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
  }
  .grid2 {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
    align-items: end;
  }
  /* Same field-label style as every other Otto form (app.css .field > label). */
  .field-label {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .hint-inline {
    font-weight: 400;
    font-size: var(--fs-xs);
  }
  .hint-inline::before {
    content: '· ';
  }
  /* Agent picker: the same toggle chips as the evaluation form's validations. */
  .provider-chips {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .chip-toggle {
    position: relative;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 3px 9px;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: var(--fs-xs);
    cursor: pointer;
    user-select: none;
  }
  .chip-toggle.on {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 40%, transparent);
    color: var(--text);
  }
  .chip-toggle input {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    padding: 0;
    overflow: hidden;
    clip-path: inset(50%);
    border: 0;
  }
  .chip-toggle:has(input:focus-visible) {
    outline: 2px solid color-mix(in srgb, var(--accent) 70%, transparent);
    outline-offset: 1px;
  }
  .block-head {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .row {
    display: flex;
    gap: 8px;
    align-items: center;
  }
  .prompt {
    padding: 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  textarea.input {
    resize: vertical;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-top: 4px;
  }

  /* Detail / grid */
  .mx-detail {
    padding: 16px 18px 60px;
    overflow: auto;
    height: 100%;
    box-sizing: border-box;
  }
  .mx-detail-head {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-bottom: 14px;
  }
  .mx-detail-title {
    min-width: 0;
  }
  .mx-sub {
    display: block;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .table-wrap {
    overflow-x: auto;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: var(--fs-s);
  }
  th,
  td {
    border: 1px solid var(--border);
    padding: 8px 10px;
    text-align: center;
    vertical-align: middle;
  }
  thead th {
    background: var(--surface-2);
  }
  .rowlabel {
    text-align: start;
    font-weight: 600;
    color: var(--text-dim);
    white-space: nowrap;
  }
  .col-prov {
    font-weight: 600;
  }
  .col-skill {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  /* The <td> stays a table cell (display:flex on it broke the grid's borders
     and column alignment); the stacked layout lives on the inner wrapper. */
  .cell-in {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    width: 100%;
    padding: 2px 0;
    border: none;
    border-radius: var(--radius-s);
    background: transparent;
    color: inherit;
    font: inherit;
  }
  .cell-in.clickable {
    cursor: pointer;
  }
  .cell-in.clickable:hover {
    background: var(--hover);
  }
  /* Winner: an outline + a check before the score (colour is never the only
     signal, and no success tint under the proof pill's own tone). */
  .cell.winner {
    outline: 2px solid var(--success);
    outline-offset: -2px;
  }
  .score {
    display: inline-flex;
    align-items: center;
    gap: 3px;
    font-weight: 600;
    font-size: var(--fs-m);
  }
  .cell.winner .score :global(svg) {
    color: var(--success);
  }
  .cell-status {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .dash {
    color: var(--text-dim);
  }
  .proof {
    font-size: var(--fs-xs);
    font-weight: 500;
    padding: 1px 8px;
    border-radius: 999px;
  }
  .pf-pass {
    background: var(--success-soft);
    color: var(--success);
  }
  .pf-fail {
    background: var(--danger-soft);
    color: var(--danger);
  }
  .pf-partial {
    background: var(--warning-soft);
    color: var(--warning);
  }
  .pf-none {
    background: var(--surface-2);
    color: var(--text-dim);
  }
  .grow {
    flex: 1;
  }
  .mono {
    font-family: var(--font-mono);
  }
</style>
