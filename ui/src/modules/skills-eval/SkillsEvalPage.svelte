<script lang="ts">
  // Skills Evaluator module: a left list of past runs + "New evaluation", and a
  // right pane showing either the start form or a selected run's live report.
  import { untrack } from 'svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { LIST_PANE, loadPaneWidth } from '../../lib/paneResizer';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { router } from '../../lib/router.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { registry } from '../../lib/commands.svelte';
  import Skeleton from '../../lib/components/Skeleton.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import type { SkillEval, SkillEvalSummary, StartSkillEvalReq } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { runStatus } from '../../lib/status';
  import { rel } from '../../lib/stores/now.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import StartEvalForm from './StartEvalForm.svelte';
  import RunDetail from './RunDetail.svelte';
  import CompareView from './CompareView.svelte';
  import GoldenTasksView from './GoldenTasksView.svelte';
  import MatrixView from './MatrixView.svelte';

  type Mode = 'form' | 'detail';
  type Tab = 'runs' | 'golden' | 'matrix';

  interface Props {
    /** "Evaluate <skill>" hand-off from the Skills tab: open the start form
     *  with this skill pre-selected. */
    initialSkill?: { name: string; source: string } | null;
    onconsumed?: () => void;
    /** "Open run" hand-off (a skill's Evals tab): select this run. */
    initialRun?: string | null;
    onrunconsumed?: () => void;
    /** The runs list is hidden — the ONE list toggle lives in the Skills Lab
     *  header (it hands its state down). */
    listCollapsed?: boolean;
    /** Tells the host whether this view has a runs list to toggle. */
    onlistable?: (listable: boolean) => void;
  }
  let { initialSkill = null, onconsumed, initialRun = null, onrunconsumed, listCollapsed = false, onlistable }: Props = $props();
  // The skill the start form should pre-select (kept until the form is left).
  let formSkill = $state<{ name: string; source: string } | null>(null);
  $effect(() => {
    if (!initialSkill) return;
    formSkill = initialSkill;
    setTab('runs');
    compareMode = false;
    selectedId = null;
    mode = 'form';
    onconsumed?.();
  });

  $effect(() => {
    if (!initialRun) return;
    const id = initialRun;
    onrunconsumed?.();
    formSkill = null;
    compareMode = false;
    void openRunById(id);
  });

  // List rows are SUMMARIES (one paged query, no iterations); the detail pane
  // and Compare fetch full runs by id. Only ever reassigned → raw.
  let runs: SkillEvalSummary[] = $state.raw([]);
  const PAGE = 200;
  let nextCursor = $state<string | null>(null);
  let loadingMore = $state(false);
  function toSummary(e: SkillEval): SkillEvalSummary {
    const { iterations, config: _config, promoted_at: _pa, promoted_by: _pb, ...rest } = e;
    return { ...rest, iteration_count: iterations.length };
  }
  let loading = $state(true);
  // Inline load failure (with Retry) instead of a toast over an empty list.
  let loadError: string | null = $state(null);
  let mode: Mode = $state('form');
  let selectedId: string | null = $state(null);
  let starting = $state(false);
  const listHidden = $derived(listCollapsed);
  $effect(() => onlistable?.(tab === 'runs'));
  // The view is part of the route (`#/skills-eval/evaluator[/golden|/matrix]`)
  // so back/forward and deep links land on it.
  const TABS: { id: Tab; label: string; icon: 'zap' | 'target' | 'grid' }[] = [
    { id: 'runs', label: 'Runs', icon: 'zap' },
    { id: 'golden', label: 'Golden tasks', icon: 'target' },
    { id: 'matrix', label: 'Matrix', icon: 'grid' },
  ];
  const tab = $derived<Tab>(router.parts[2] === 'golden' ? 'golden' : router.parts[2] === 'matrix' ? 'matrix' : 'runs');
  function setTab(t: Tab): void {
    if (t === tab) return;
    router.go(t === 'runs' ? 'skills-eval/evaluator' : `skills-eval/evaluator/${t}`);
  }

  // Open a run from another tab (golden task run, matrix cell): switch to Runs,
  // reload, and select it.
  async function openRunById(id: string): Promise<void> {
    setTab('runs');
    const wsId = ws.currentId;
    if (wsId) await loadList(wsId);
    selectedId = id;
    mode = 'detail';
  }
  function openRun(e: SkillEval): void {
    runs = [toSummary(e), ...runs.filter((r) => r.id !== e.id)];
    void openRunById(e.id);
  }

  // ⌘K: the Evaluator's verbs while it's on screen.
  $effect(() =>
    registry.register('skills-eval', [
      { id: 'skills-eval.new', title: 'New skill evaluation', group: 'Skills Lab', keywords: 'evaluate eval start run score', run: () => { setTab('runs'); compareMode = false; newRun(); } },
      { id: 'skills-eval.golden', title: 'Open golden tasks', group: 'Skills Lab', keywords: 'regression corpus eval', run: () => setTab('golden') },
      { id: 'skills-eval.matrix', title: 'Open eval matrix', group: 'Skills Lab', keywords: 'provider skill prompt grid compare', run: () => setTab('matrix') },
    ]),
  );

  // Compare mode: pick 2+ runs from the list to view side by side.
  let compareMode = $state(false);
  let compareSel = $state<Set<string>>(new Set());
  // Compare reads iterations: full runs are fetched for the selected ids only.
  let compareFull = $state.raw<Record<string, SkillEval>>({});
  $effect(() => {
    const ids = [...compareSel];
    untrack(() => {
      for (const id of ids) {
        if (compareFull[id]) continue;
        void skillsEvalApi.get(id).then(
          (e) => (compareFull = { ...compareFull, [id]: e }),
          () => {},
        );
      }
    });
  });
  const compareRuns = $derived(
    runs.filter((r) => compareSel.has(r.id)).map((r) => compareFull[r.id]).filter((e): e is SkillEval => !!e),
  );

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
    // A workspace switch must not keep the previous workspace's run open
    // (RunDetail would fetch an id that isn't in this list) or its compare
    // selection.
    selectedId = null;
    mode = 'form';
    compareMode = false;
    compareSel = new Set();
    compareFull = {};
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
    loadError = null;
    try {
      const page = await skillsEvalApi.listSummaries(wsId, { limit: PAGE });
      runs = page.items;
      nextCursor = page.next_cursor;
      // Default to the newest run's detail if one exists; else the start form.
      if (runs.length > 0 && selectedId === null && !formSkill) {
        selectedId = runs[0].id;
        mode = 'detail';
      }
    } catch (e) {
      loadError = e instanceof Error ? e.message : String(e);
    } finally {
      loading = false;
    }
  }

  async function loadMore(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || !nextCursor || loadingMore) return;
    loadingMore = true;
    try {
      const page = await skillsEvalApi.listSummaries(wsId, { limit: PAGE, cursor: nextCursor });
      if (ws.currentId !== wsId) return;
      const have = new Set(runs.map((r) => r.id));
      runs = [...runs, ...page.items.filter((r) => !have.has(r.id))];
      nextCursor = page.next_cursor;
    } catch (e) {
      toasts.error("Couldn’t load more evaluations", e instanceof Error ? e.message : String(e));
    } finally {
      loadingMore = false;
    }
  }

  function newRun(): void {
    formSkill = null;
    selectedId = null;
    mode = 'form';
  }

  function selectRun(id: string): void {
    formSkill = null;
    selectedId = id;
    mode = 'detail';
  }

  async function start(req: StartSkillEvalReq): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || starting) return;
    starting = true;
    try {
      const created = await skillsEvalApi.start(wsId, req);
      runs = [toSummary(created), ...runs];
      formSkill = null;
      selectedId = created.id;
      mode = 'detail';
      toasts.success('Evaluation started', 'Watch progress in the report.');
    } catch (e) {
      toasts.error("Couldn’t start the evaluation", e instanceof Error ? e.message : String(e));
    } finally {
      starting = false;
    }
  }

  // Keep the list entry in sync with the live detail (status, score).
  function onRunUpdate(e: SkillEval): void {
    const i = runs.findIndex((r) => r.id === e.id);
    if (i < 0) return;
    const next = toSummary(e);
    const cur = runs[i];
    // RunDetail polls: re-render the list only when a shown field moved.
    if (cur.status === next.status && cur.best_score === next.best_score && cur.iteration_count === next.iteration_count && cur.composite_score === next.composite_score) return;
    const copy = [...runs];
    copy[i] = next;
    runs = copy;
    if (compareFull[e.id]) compareFull = { ...compareFull, [e.id]: e };
  }

  function onRunDeleted(id: string): void {
    runs = runs.filter((r) => r.id !== id);
    if (selectedId === id) {
      selectedId = runs[0]?.id ?? null;
      mode = selectedId ? 'detail' : 'form';
    }
  }

  // Resizable run list (PaneDivider): the width survives reloads.
  let sideW = $state(loadPaneWidth('skillsEval.sideW', LIST_PANE.default, LIST_PANE.min, LIST_PANE.max));
</script>

<div class="se-wrap">
  <div class="se-subhead">
    <div class="segmented" role="tablist" aria-label="Evaluator view" data-testid="eval-tabs" tabindex="-1" onkeydown={onTabKey}>
      {#each TABS as t (t.id)}
        <button role="tab" aria-selected={tab === t.id} aria-controls="eval-panel" tabindex={tab === t.id ? 0 : -1} class:active={tab === t.id} onclick={() => setTab(t.id)} data-testid="tab-{t.id}">
          <Icon name={t.icon} size={12} /> {t.label}
        </button>
      {/each}
    </div>
  </div>
  <div class="se-content" id="eval-panel" role="tabpanel">
    {#if tab === 'golden'}
      <GoldenTasksView onopenrun={openRun} />
    {:else if tab === 'matrix'}
      <MatrixView onopenrun={openRunById} />
    {:else}
<div class="se-page" class:list-hidden={listHidden}>
  <aside class="se-side" id="evaluations-list" style={viewport.isPhone ? undefined : `width:${sideW}px`}>
    <div class="se-side-head">
      <span class="se-side-title">Evaluations</span>
      <button
        class="btn small ghost"
        class:active={compareMode}
        aria-pressed={compareMode}
        onclick={toggleCompare}
        title={runs.length < 2 ? 'Needs at least two runs to compare' : compareMode ? 'Leave compare mode' : 'Compare runs side by side'}
        disabled={runs.length < 2}
      >
        <Icon name="columns" size={12} /> {compareMode ? 'Done' : 'Compare'}
      </button>
      <!-- Not .primary: the start form's "Start evaluation" is this view's primary. -->
      <button class="btn small" onclick={newRun} aria-pressed={mode === 'form' && !compareMode} title="New evaluation">
        <Icon name="plus" size={12} /> New
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
        <div aria-busy="true" aria-label="Loading evaluations"><Skeleton rows={4} height={56} /></div>
      {:else if loadError && runs.length === 0}
        <div class="se-muted se-err" role="alert">
          <span><Icon name="warning" size={12} /> <strong>Couldn’t load evaluations.</strong></span>
          <span class="se-err-detail">{loadError}</span>
          <button class="btn small" onclick={() => ws.currentId && loadList(ws.currentId)} disabled={loading}>{loading ? 'Retrying…' : 'Retry'}</button>
        </div>
      {:else if runs.length === 0}
        <div class="se-muted">No evaluations yet. Fill in the form to run the first one.</div>
      {:else}
        {#each runs as r (r.id)}
          <button
            class="se-item"
            aria-pressed={compareMode ? compareSel.has(r.id) : undefined}
            aria-current={!compareMode && mode === 'detail' && selectedId === r.id ? 'true' : undefined}
            class:active={compareMode ? compareSel.has(r.id) : mode === 'detail' && selectedId === r.id}
            onclick={() => (compareMode ? toggleCompareSel(r.id) : selectRun(r.id))}
          >
            <div class="se-item-top">
              {#if compareMode}
                <span class="se-check" class:on={compareSel.has(r.id)}>
                  {#if compareSel.has(r.id)}<Icon name="check" size={11} />{/if}
                </span>
              {/if}
              <span class="se-item-name" title={r.source_skill}>{r.source_skill}</span>
              <span class="se-dot st-{r.status}" role="img" aria-label={runStatus(r.status).label} title={runStatus(r.status).label}></span>
            </div>
            <div class="se-item-sub">
              <span class="se-task" title={r.task}>{r.task}</span>
            </div>
            <div class="se-item-meta">
              <span class="se-meta-main">{[r.impl_cli || 'Score only', r.best_score != null ? `best ${r.best_score.toFixed(0)}` : ''].filter(Boolean).join(' · ')}</span>
              <span class="grow"></span>
              <span title={new Date(r.created_at).toLocaleString()}>{rel(r.created_at)}</span>
            </div>
          </button>
        {/each}
        {#if nextCursor}
          <button class="btn small ghost se-more" onclick={() => void loadMore()} disabled={loadingMore}>
            {loadingMore ? 'Loading…' : 'Load older evaluations'}
          </button>
        {/if}
      {/if}
    </div>
  </aside>

  {#if !listHidden}<PaneDivider bind:width={sideW} storageKey="skillsEval.sideW" label="Resize the evaluations list" />{/if}

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
      <StartEvalForm {starting} onstart={start} initialSkill={formSkill} />
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
    {/if}
  </div>
</div>

<style>
  .se-subhead { display: flex; align-items: center; padding: 8px 12px; border-bottom: 1px solid var(--border); }
  .se-page.list-hidden .se-side { display: none; }
  .se-wrap {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .se-content {
    flex: 1;
    min-height: 0;
    overflow: hidden;
  }
  .se-page {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .se-side {
    /* Default width; an inline `width:{sideW}px` (drag-resizable, persisted)
       takes over at runtime. */
    width: 280px;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    background: var(--surface);
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
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
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
    font-size: var(--fs-s);
  }
  .se-err {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    gap: 6px;
    color: var(--text);
    overflow-wrap: anywhere;
  }
  .se-err :global(svg) {
    color: var(--danger);
    vertical-align: -1px;
  }
  .se-err-detail {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .se-more {
    margin-block: 8px;
    margin-inline: auto;
    display: block;
  }
  .se-item {
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
  .se-item:hover {
    background: var(--hover);
  }
  .se-item.active {
    background: var(--accent-soft);
    border-color: color-mix(in srgb, var(--accent) 30%, transparent);
  }
  .se-item-top {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .se-item-name {
    font-size: var(--fs-m);
    font-weight: 500;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .se-task {
    font-size: var(--fs-xs);
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
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .se-meta-main {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .se-dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    flex-shrink: 0;
  }
  /* Run dot, same tones as runStatus: running = info (pulsing), done =
     success, error = danger, cancelled = neutral. */
  .se-dot {
    background: var(--status-idle);
  }
  .se-dot.st-running {
    background: var(--info);
    animation: pulse 1.2s ease-in-out infinite;
  }
  .se-dot.st-done {
    background: var(--status-working);
  }
  .se-dot.st-error {
    background: var(--status-exited);
  }
  /* Phone: the run list stacks above the report (no room for a side pane). */
  @media (max-width: 640px) {
    .se-page {
      flex-direction: column;
    }
    .se-side {
      width: 100%;
      max-height: 40%;
      border-inline-end: none;
      border-bottom: 1px solid var(--border);
    }
    .se-main {
      flex: 1;
      min-height: 0;
    }
    .se-subhead {
      overflow-x: auto;
      padding: 8px;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .se-dot.st-running {
      animation: none;
    }
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
    font-size: var(--fs-xs);
    color: var(--text-dim);
    border-bottom: 1px solid var(--border);
  }
  .se-check {
    width: 14px;
    height: 14px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    display: grid;
    place-items: center;
    flex-shrink: 0;
  }
  .se-check.on {
    background: var(--accent-solid);
    color: var(--accent-contrast);
    border-color: var(--accent-solid);
  }
  .btn.active {
    background: var(--accent-soft);
    color: var(--text);
  }
  .grow {
    flex: 1;
  }
</style>
