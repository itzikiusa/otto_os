<script lang="ts">
  // Skills Evaluator module: a left list of past runs + "New evaluation", and a
  // right pane showing either the start form or a selected run's live report.
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import type { SkillEval, StartSkillEvalReq } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import StartEvalForm from './StartEvalForm.svelte';
  import RunDetail from './RunDetail.svelte';
  import CompareView from './CompareView.svelte';

  type Mode = 'form' | 'detail';

  let runs: SkillEval[] = $state([]);
  let loading = $state(true);
  let mode: Mode = $state('form');
  let selectedId: string | null = $state(null);
  let starting = $state(false);

  // Compare mode: pick 2+ runs from the list to view side by side.
  let compareMode = $state(false);
  let compareSel = $state<Set<string>>(new Set());
  const compareRuns = $derived(runs.filter((r) => compareSel.has(r.id)));

  function toggleCompare(): void {
    compareMode = !compareMode;
    if (!compareMode) compareSel = new Set();
  }

  function toggleCompareSel(id: string): void {
    const next = new Set(compareSel);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    compareSel = next;
  }

  $effect(() => {
    const wsId = ws.currentId;
    if (wsId) {
      void loadList(wsId);
    } else {
      // No workspace yet (still booting, or none selected): don't hang on a
      // perpetual "Loading…".
      loading = false;
      runs = [];
    }
  });

  async function loadList(wsId: string): Promise<void> {
    loading = true;
    try {
      runs = await skillsEvalApi.list(wsId);
      // Default to the newest run's detail if one exists; else the start form.
      if (runs.length > 0 && selectedId === null) {
        selectedId = runs[0].id;
        mode = 'detail';
      }
    } catch (e) {
      toasts.error('Could not load evaluations', e instanceof Error ? e.message : String(e));
    } finally {
      loading = false;
    }
  }

  function newRun(): void {
    selectedId = null;
    mode = 'form';
  }

  function selectRun(id: string): void {
    selectedId = id;
    mode = 'detail';
  }

  async function start(req: StartSkillEvalReq): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || starting) return;
    starting = true;
    try {
      const created = await skillsEvalApi.start(wsId, req);
      runs = [created, ...runs];
      selectedId = created.id;
      mode = 'detail';
      toasts.success('Evaluation started', 'Watch progress in the report.');
    } catch (e) {
      toasts.error('Could not start evaluation', e instanceof Error ? e.message : String(e));
    } finally {
      starting = false;
    }
  }

  // Keep the list entry in sync with the live detail (status, score).
  function onRunUpdate(e: SkillEval): void {
    runs = runs.map((r) => (r.id === e.id ? e : r));
  }

  function onRunDeleted(id: string): void {
    runs = runs.filter((r) => r.id !== id);
    if (selectedId === id) {
      selectedId = runs[0]?.id ?? null;
      mode = selectedId ? 'detail' : 'form';
    }
  }

  function ago(iso: string): string {
    const d = Date.parse(iso);
    if (Number.isNaN(d)) return '';
    const s = Math.floor((Date.now() - d) / 1000);
    if (s < 60) return `${s}s ago`;
    if (s < 3600) return `${Math.floor(s / 60)}m ago`;
    if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
    return `${Math.floor(s / 86400)}d ago`;
  }
</script>

<div class="se-page">
  <aside class="se-side">
    <div class="se-side-head">
      <span class="se-side-title">Evaluations</span>
      <button
        class="btn small ghost"
        class:active={compareMode}
        onclick={toggleCompare}
        title="Compare runs side by side"
        disabled={runs.length < 2}
      >
        <Icon name="grid" size={13} /> Compare
      </button>
      <button class="btn small primary" onclick={newRun}>
        <Icon name="plus" size={13} /> New
      </button>
    </div>
    {#if compareMode}
      <div class="se-compare-hint">
        Select 2+ runs to compare ({compareSel.size} selected).
      </div>
    {/if}
    <div class="se-list">
      {#if !ws.currentId}
        <div class="se-muted">No workspace selected.</div>
      {:else if loading && runs.length === 0}
        <div class="se-muted">Loading…</div>
      {:else if runs.length === 0}
        <div class="se-muted">No evaluations yet.</div>
      {:else}
        {#each runs as r (r.id)}
          <button
            class="se-item"
            class:active={compareMode ? compareSel.has(r.id) : mode === 'detail' && selectedId === r.id}
            onclick={() => (compareMode ? toggleCompareSel(r.id) : selectRun(r.id))}
          >
            <div class="se-item-top">
              {#if compareMode}
                <span class="se-check" class:on={compareSel.has(r.id)}>
                  {#if compareSel.has(r.id)}<Icon name="check" size={11} />{/if}
                </span>
              {/if}
              <span class="se-item-name">{r.source_skill}</span>
              <span class="se-dot st-{r.status}"></span>
            </div>
            <div class="se-item-sub">
              <span class="se-task">{r.task}</span>
            </div>
            <div class="se-item-meta">
              <span>{r.impl_cli}</span>
              {#if r.best_score != null}<span class="se-score">· best {r.best_score.toFixed(0)}</span>{/if}
              <span class="grow"></span>
              <span>{ago(r.created_at)}</span>
            </div>
          </button>
        {/each}
      {/if}
    </div>
  </aside>

  <main class="se-main">
    {#if !ws.currentId}
      <EmptyState icon="zap" title="No workspace selected" body="Pick a workspace to evaluate its skills." />
    {:else if compareMode}
      {#if compareRuns.length >= 2}
        <CompareView runs={compareRuns} />
      {:else}
        <EmptyState
          icon="grid"
          title="Compare runs"
          body="Select two or more runs from the list to compare their scores side by side."
        />
      {/if}
    {:else if mode === 'form'}
      <StartEvalForm {starting} onstart={start} />
    {:else if selectedId}
      {#key selectedId}
        <RunDetail evalId={selectedId} onupdate={onRunUpdate} ondeleted={onRunDeleted} />
      {/key}
    {:else}
      <EmptyState
        icon="zap"
        title="Skills Evaluator"
        body="Test a skill by having an agent use it, validate the result, score it, and improve the skill across iterations."
        actionLabel="New evaluation"
        onaction={newRun}
      />
    {/if}
  </main>
</div>

<style>
  .se-page {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .se-side {
    width: 280px;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .se-side-head {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 12px 8px;
  }
  .se-side-title {
    font-size: 12px;
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    color: var(--text-dim);
    flex: 1;
  }
  .se-list {
    flex: 1;
    overflow-y: auto;
    padding: 4px 8px 12px;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .se-muted {
    padding: 16px 8px;
    color: var(--text-dim);
    font-size: 12px;
  }
  .se-item {
    text-align: start;
    border: 1px solid transparent;
    background: transparent;
    border-radius: var(--radius-m, 8px);
    padding: 8px 10px;
    cursor: pointer;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .se-item:hover {
    background: color-mix(in srgb, var(--text-dim) 8%, transparent);
  }
  .se-item.active {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .se-item-top {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .se-item-name {
    font-size: 12.5px;
    font-weight: 600;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .se-task {
    font-size: 11px;
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    display: block;
  }
  .se-item-meta {
    display: flex;
    align-items: center;
    gap: 5px;
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .se-score {
    color: var(--text);
  }
  .se-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  .se-dot.st-running {
    background: var(--status-working, var(--accent));
    animation: pulse 1.2s ease-in-out infinite;
  }
  .se-dot.st-done {
    background: var(--status-idle, #6bbf6b);
  }
  .se-dot.st-error,
  .se-dot.st-cancelled {
    background: var(--status-exited, #d66);
  }
  @keyframes pulse {
    50% {
      opacity: 0.35;
    }
  }
  .se-main {
    flex: 1;
    min-width: 0;
    overflow: hidden;
  }
  .se-compare-hint {
    padding: 6px 12px;
    font-size: 11px;
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .se-check {
    width: 14px;
    height: 14px;
    border: 1px solid var(--border);
    border-radius: 3px;
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .se-check.on {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }
  .btn.active {
    background: color-mix(in srgb, var(--accent) 18%, transparent);
    color: var(--accent);
  }
  .grow {
    flex: 1;
  }
</style>
