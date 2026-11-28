<script lang="ts">
  import { toastError } from '../../lib/toastError';
  // Skills Evaluator module: a left list of past runs + "New evaluation", and a
  // right pane showing either the start form or a selected run's live report.
  import { onDestroy, untrack } from 'svelte';
  import ListDetail from '../../lib/components/side/ListDetail.svelte';
  import ListPane from '../../lib/components/side/ListPane.svelte';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { skillsLabState } from '../../lib/stores/skillsLabState.svelte';
  import { router } from '../../lib/router.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { registry } from '../../lib/commands.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { skillsEvalApi } from '../../lib/api/skillsEval';
  import type { SkillEval, SkillEvalSummary, StartSkillEvalReq } from '../../lib/api/types';
  import Icon from '../../lib/components/Icon.svelte';
  import { runStatus } from '../../lib/status';
  import { rel } from '../../lib/stores/now.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import StartEvalForm from './StartEvalForm.svelte';
  import RunDetail from './RunDetail.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import { lazyComponent } from '../../lib/lazy-component.svelte';

  const CompareViewLazy = lazyComponent(() => import('./CompareView.svelte'));
  const GoldenTasksViewLazy = lazyComponent(() => import('./GoldenTasksView.svelte'));
  const MatrixViewLazy = lazyComponent(() => import('./MatrixView.svelte'));
  const EvalDefaultsSheetLazy = lazyComponent(() => import('./EvalDefaultsSheet.svelte'));

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
  }
  let { initialSkill = null, onconsumed, initialRun = null, onrunconsumed }: Props = $props();
  const initialEval = skillsLabState.evalFor(ws.currentId);
  let formSkill = $state<{ name: string; source: string } | null>(initialEval.formSkill);
  /** Phone: the open run / form / comparison is pushed over the list. */
  let phoneOpen = $state(false);
  $effect(() => {
    if (!initialSkill) return;
    formSkill = initialSkill;
    setTab('runs');
    compareMode = false;
    selectedId = null;
    mode = 'form';
    phoneOpen = true;
    onconsumed?.();
  });

  $effect(() => {
    if (!initialRun) return;
    const id = initialRun;
    onrunconsumed?.();
    formSkill = null;
    compareMode = false;
    phoneOpen = true;
    void openRunById(id);
  });

  // List rows are SUMMARIES (one paged query, no iterations); the detail pane
  // and Compare fetch full runs by id. Only ever reassigned → raw.
  let runs: SkillEvalSummary[] = $state.raw(initialEval.runs);
  const PAGE = 200;
  let nextCursor = $state<string | null>(initialEval.nextCursor);
  let loadingMore = $state(false);
  function toSummary(e: SkillEval): SkillEvalSummary {
    const { iterations, config: _config, promoted_at: _pa, promoted_by: _pb, ...rest } = e;
    return { ...rest, iteration_count: iterations.length };
  }
  let loading = $state(!initialEval.loaded);
  // Inline load failure (with Retry) instead of a toast over an empty list.
  let loadError: string | null = $state(null);
  let mode: Mode = $state(initialEval.mode);
  let selectedId: string | null = $state(initialEval.selectedId);
  const starting = $derived(skillsLabState.startingEval);
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
    phoneOpen = true;
  }
  function openRun(e: SkillEval): void {
    runs = [toSummary(e), ...runs.filter((r) => r.id !== e.id)];
    void openRunById(e.id);
  }

  // ⌘K: the Evaluator's verbs while it's on screen.
  $effect(() =>
    registry.register('skills-eval', [
      { id: 'skills-eval.new', title: 'New skill evaluation…', group: 'Skills Lab', keywords: 'evaluate eval start run score', run: () => { setTab('runs'); compareMode = false; newRun(); } },
      { id: 'skills-eval.golden', title: 'Open golden tasks', group: 'Skills Lab', keywords: 'regression corpus eval', run: () => setTab('golden') },
      { id: 'skills-eval.matrix', title: 'Open eval matrix', group: 'Skills Lab', keywords: 'provider skill prompt grid compare', run: () => setTab('matrix') },
    ]),
  );

  // Compare mode: pick 2+ runs from the list to view side by side.
  let compareMode = $state(initialEval.compareMode);
  let compareSel = $state<Set<string>>(new Set(initialEval.compareSel));
  // Compare reads iterations: full runs are fetched for the selected ids only.
  let compareFull = $state.raw<Record<string, SkillEval>>(initialEval.compareFull);
  let compareErrors = $state.raw<Record<string, string>>({});
  let compareLoading = $state.raw<Record<string, boolean>>({});
  let disposed = false;
  onDestroy(() => { disposed = true; });
  function loadCompareRun(id: string): void {
    const workspace = ws.currentId;
    const nextErr = { ...compareErrors };
    delete nextErr[id];
    compareErrors = nextErr;
    compareLoading = { ...compareLoading, [id]: true };
    void skillsEvalApi.get(id).then(
      (e) => {
        if (disposed || ws.currentId !== workspace) return;
        compareFull = { ...compareFull, [id]: e };
      },
      (err) => {
        if (disposed || ws.currentId !== workspace) return;
        compareErrors = { ...compareErrors, [id]: loadErrorText(err) };
      },
    ).finally(() => {
      if (disposed || ws.currentId !== workspace) return;
      const nextLoad = { ...compareLoading };
      delete nextLoad[id];
      compareLoading = nextLoad;
    });
  }
  $effect(() => {
    const ids = [...compareSel];
    untrack(() => {
      for (const id of ids) {
        if (compareFull[id] || compareLoading[id] || compareErrors[id]) continue;
        loadCompareRun(id);
      }
    });
  });
  const compareRuns = $derived(
    runs.filter((r) => compareSel.has(r.id)).map((r) => compareFull[r.id]).filter((e): e is SkillEval => !!e),
  );
  const selectedCompareErrors = $derived(
    [...compareSel]
      .filter((id) => !!compareErrors[id])
      .map((id) => ({
        id,
        name: runs.find((r) => r.id === id)?.source_skill ?? id,
        error: compareErrors[id],
      })),
  );
  const comparePending = $derived(
    [...compareSel].some((id) => !compareFull[id] && !compareErrors[id]),
  );

  // Evaluator defaults (was Settings → Skills evaluator): the Lab's page ⋯
  // opens it, and `#/skills-eval/evaluator/defaults` — where the old Settings
  // route forwards — opens it on arrival.
  let defaultsOpen = $state(false);
  export function openDefaults(): void {
    defaultsOpen = true;
  }
  $effect(() => {
    if (router.parts[2] !== 'defaults') return;
    defaultsOpen = true;
    router.replace('skills-eval/evaluator');
  });

  function toggleCompare(): void {
    compareMode = !compareMode;
    if (!compareMode) compareSel = new Set();
    else if (compareSel.size < 2) phoneOpen = false;
  }
  // The runs pane's ⋯ (after Hide Evaluations ⌘B).
  function paneMenu(): MenuItem[] {
    return [{ label: compareTitle, icon: 'columns', disabled: runs.length < 2, action: toggleCompare }];
  }

  function toggleCompareSel(id: string): void {
    const next = new Set(compareSel);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    compareSel = next;
  }

  const initialLoadedWs = ws.currentId && initialEval.loaded ? ws.currentId : null;
  let loadedWs = $state<string | null>(initialLoadedWs);
  let listGeneration = 0;
  async function loadList(wsId: string, background = false): Promise<void> {
    const generation = ++listGeneration;
    if (!background) {
      loading = true;
      loadError = null;
    }
    try {
      const page = await skillsEvalApi.listSummaries(wsId, { limit: PAGE });
      if (generation !== listGeneration || ws.currentId !== wsId) return;
      loadError = null;
      if (background && runs.length > page.items.length) {
        const freshIds = new Set(page.items.map((r) => r.id));
        runs = [...page.items, ...runs.filter((r) => !freshIds.has(r.id))];
      } else {
        runs = page.items;
        nextCursor = page.next_cursor;
      }
      // Default to the newest run's detail if one exists; else the start form.
      if (runs.length > 0 && selectedId === null && !formSkill) {
        selectedId = runs[0].id;
        mode = 'detail';
      }
    } catch (e) {
      if (generation === listGeneration && ws.currentId === wsId && (!background || runs.length === 0)) {
        loadError = loadErrorText(e);
      }
    } finally {
      if (!background && generation === listGeneration && ws.currentId === wsId) loading = false;
    }
  }

  if (initialLoadedWs) void loadList(initialLoadedWs, true);
  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId) {
      loadedWs = null;
      loading = false;
      runs = [];
      return;
    }
    if (wsId === loadedWs) return;
    const cached = untrack(() => skillsLabState.evalFor(wsId));
    loadedWs = wsId;
    compareErrors = {};
    compareLoading = {};
    phoneOpen = true;
    if (cached.loaded) {
      runs = cached.runs;
      nextCursor = cached.nextCursor;
      mode = cached.mode;
      selectedId = cached.selectedId;
      formSkill = cached.formSkill;
      compareMode = cached.compareMode;
      compareSel = new Set(cached.compareSel);
      compareFull = cached.compareFull;
      loading = false;
      void loadList(wsId, true);
      return;
    }
    selectedId = null;
    mode = 'form';
    compareMode = false;
    compareSel = new Set();
    compareFull = {};
    void loadList(wsId);
  });
  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId || loadedWs !== wsId) return;
    skillsLabState.syncEval(wsId, {
      runs,
      nextCursor,
      loaded: !loading && !loadError,
      mode,
      selectedId,
      formSkill,
      compareMode,
      compareSel: new Set(compareSel),
      compareFull,
    });
  });

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
      toastError("Couldn’t load more evaluations", e);
    } finally {
      loadingMore = false;
    }
  }

  function newRun(): void {
    formSkill = null;
    selectedId = null;
    mode = 'form';
    phoneOpen = true;
  }

  function selectRun(id: string): void {
    formSkill = null;
    selectedId = id;
    mode = 'detail';
    phoneOpen = true;
  }

  async function start(req: StartSkillEvalReq): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId || starting) return;
    const created = await skillsLabState.startEval(wsId, req);
    if (!created || ws.currentId !== wsId) return;
    runs = [toSummary(created), ...runs.filter((r) => r.id !== created.id)];
    formSkill = null;
    selectedId = created.id;
    mode = 'detail';
    phoneOpen = true;
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
    if (compareSel.has(id)) {
      const next = new Set(compareSel);
      next.delete(id);
      compareSel = next;
    }
    if (compareFull[id]) {
      const { [id]: _, ...rest } = compareFull;
      compareFull = rest;
    }
    if (compareErrors[id]) {
      const { [id]: _, ...rest } = compareErrors;
      compareErrors = rest;
    }
    if (selectedId === id) {
      selectedId = runs[0]?.id ?? null;
      mode = selectedId ? 'detail' : 'form';
    }
  }

  const compareTitle = $derived(runs.length < 2 ? 'Compare runs (needs at least two)' : compareMode ? 'Leave compare mode' : 'Compare runs side by side');
</script>

<div class="se-wrap">
  <!-- The view tabs (Runs / Golden tasks / Matrix) live in the Skills Lab
       header's tab row (SkillsLabPage) — no second strip here. -->
  <div class="se-content" id="eval-panel" role="tabpanel" aria-label={TABS.find((t) => t.id === tab)?.label}>
    {#if tab === 'golden'}
      {#if GoldenTasksViewLazy.component}
        <GoldenTasksViewLazy.component onopenrun={openRun} />
      {:else}
        <LoadState what="golden tasks" variant="page" loading={!GoldenTasksViewLazy.error} error={GoldenTasksViewLazy.error} empty onretry={() => GoldenTasksViewLazy.retry()} />
      {/if}
    {:else if tab === 'matrix'}
      {#if MatrixViewLazy.component}
        <MatrixViewLazy.component onopenrun={openRunById} />
      {:else}
        <LoadState what="matrix evaluations" variant="page" loading={!MatrixViewLazy.error} error={MatrixViewLazy.error} empty onretry={() => MatrixViewLazy.retry()} />
      {/if}
    {:else}
{#snippet compareBar()}
          <span class="se-sel-count">{compareSel.size} selected</span>
          <span class="grow"></span>
          {#if viewport.navDrawer}
            <button class="btn small" type="button" disabled={compareSel.size < 2} onclick={() => (phoneOpen = true)}>Compare</button>
          {/if}
          <button class="btn small ghost" type="button" onclick={toggleCompare} title="Leave compare mode">Done</button>
{/snippet}

<ListDetail module="skills-eval" pane="evaluations" name="Evaluations" open={phoneOpen && (!compareMode || compareSel.size >= 2)} onback={() => (phoneOpen = false)} empty={!!ws.currentId && !loading && !loadError && runs.length === 0} legacy={{ w: 'skillsEval.sideW' }}>
  {#snippet list()}
    <ListPane
      count={runs.length}
      primary={{ icon: 'plus', label: 'New evaluation', onclick: newRun }}
      menu={paneMenu}
      selection={compareMode ? compareBar : undefined}
      loading={!!ws.currentId && loading && runs.length === 0}
      error={runs.length === 0 ? loadError : null}
      onretry={() => ws.currentId && loadList(ws.currentId)}
      testid="evaluations-list"
    >
      {#if !ws.currentId}
        <div class="se-muted">No workspace selected.</div>
      {:else}
        {#each runs as r (r.id)}
          {@const on = compareMode ? compareSel.has(r.id) : mode === 'detail' && selectedId === r.id}
          <div class="side-row two-line" class:selected={on}>
            <button
              type="button"
              class="side-row-main se-item"
              class:active={on}
              data-side-row
              aria-pressed={compareMode ? compareSel.has(r.id) : undefined}
              aria-current={!compareMode && mode === 'detail' && selectedId === r.id ? 'true' : undefined}
              onclick={() => (compareMode ? toggleCompareSel(r.id) : selectRun(r.id))}
            >
              {#if compareMode}<Icon name={compareSel.has(r.id) ? 'check' : 'square'} size={14} />{/if}
              <span class="side-row-text">
                <span class="se-item-top">
                  <span class="side-row-title se-item-name" title={r.source_skill}>{r.source_skill}</span>
                  <span class="se-dot st-{r.status}" role="img" aria-label={runStatus(r.status).label} title={runStatus(r.status).label}></span>
                </span>
                <span class="side-row-meta se-task" title={r.task}>{r.task}</span>
                <span class="side-row-meta se-item-meta">
                  <span class="se-meta-main">{[r.impl_cli || 'Score only', r.best_score != null ? `best ${r.best_score.toFixed(0)}` : ''].filter(Boolean).join(' · ')}</span>
                  <span class="grow"></span>
                  <span title={new Date(r.created_at).toLocaleString()}>{rel(r.created_at)}</span>
                </span>
              </span>
            </button>
          </div>
        {/each}
        {#if nextCursor}
          <button class="btn small ghost se-more" onclick={() => void loadMore()} disabled={loadingMore}>
            {loadingMore ? 'Loading older evaluations…' : 'Load older evaluations'}
          </button>
        {/if}
      {/if}
    </ListPane>
  {/snippet}

  <main class="se-main">
    {#if !ws.currentId}
      <EmptyState icon="zap" title="No workspace selected" body="Pick a workspace to evaluate its skills." />
    {:else if compareMode}
      {#if compareSel.size < 2}
        <EmptyState
          icon="grid"
          title="Compare runs"
          body="Select two or more runs from the list to compare their scores side by side."
        />
      {:else}
        {#if selectedCompareErrors.length > 0}
          <LoadState
            what="comparison runs"
            variant={compareRuns.length >= 2 ? 'compact' : 'page'}
            error={selectedCompareErrors.map((x) => `${x.name}: ${x.error}`).join(' · ')}
            empty={compareRuns.length < 2}
            onretry={() => { for (const x of selectedCompareErrors) loadCompareRun(x.id); }}
          />
        {/if}
        {#if compareRuns.length >= 2}
          {#if CompareViewLazy.component}
            <CompareViewLazy.component runs={compareRuns} />
          {:else}
            <LoadState what="comparison runs" variant="page" loading={!CompareViewLazy.error} error={CompareViewLazy.error} empty onretry={() => CompareViewLazy.retry()} />
          {/if}
        {:else if comparePending}
          <LoadState what="comparison runs" variant="page" loading empty />
        {/if}
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
</ListDetail>
    {/if}
  </div>
</div>

{#if defaultsOpen}
  <LazyMount
    lazy={EvalDefaultsSheetLazy}
    what="evaluator defaults"
    quiet
    props={{ onclose: () => (defaultsOpen = false) }}
    onerror={() => (defaultsOpen = false)}
  />
{/if}

<style>
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
  .se-muted {
    padding: 16px 8px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  .se-more {
    margin-block: 8px;
    margin-inline: auto;
    display: block;
  }
  .se-item-top {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .se-item-name {
    flex: 1;
  }
  .se-item-meta {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .se-sel-count {
    padding-inline-start: 8px;
    font-size: var(--fs-m);
    font-weight: 600;
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
    animation: otto-pulse 1.4s ease-in-out infinite;
  }
  .se-dot.st-done {
    background: var(--status-working);
  }
  .se-dot.st-error {
    background: var(--status-exited);
  }
  @media (prefers-reduced-motion: reduce) {
    .se-dot.st-running {
      animation: none;
    }
  }
  .se-main {
    flex: 1;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .grow {
    flex: 1;
  }
</style>
