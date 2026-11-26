<script lang="ts">
  // Workflows (ux.md §1): one route switch. `#/workflows` is Runs (the
  // inbox, the module's landing tab), `#/workflows/all` the definitions,
  // `#/workflows/<wf>` the editor, `#/workflows/<wf>/optimize` the Optimize
  // review and `#/workflows/<wf>/runs/<run>[/<node>[/<child>]]` the run page
  // — the address notices and "Copy link" have always handed out. The editor,
  // Optimize, the start form and New workflow are lazy chunks. The page owns
  // the Runs · Workflows header, the start form (Run…, Run again…), the list
  // the screens share, ⌘K and the agent UI port.
  import { untrack } from 'svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import LazyMount from '../../lib/components/LazyMount.svelte';
  import AutomateGuide from '../../lib/components/AutomateGuide.svelte';
  import AutomateGuideButton from '../../lib/components/AutomateGuideButton.svelte';
  import { api } from '../../lib/api/client';
  import type { RunWorkflowReq, Workflow, WorkflowRun, WorkflowValidationIssue } from '../../lib/api/types';
  import { copyWithToast } from '../../lib/clipboard';
  import { registry } from '../../lib/commands.svelte';
  import type { MenuItem } from '../../lib/contextmenu.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { router } from '../../lib/router.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { onTabKey } from '../../lib/tabKeys';
  import { toasts } from '../../lib/toast.svelte';
  import { toastError } from '../../lib/toastError';
  import { workflowsPagePort } from '../../lib/uiCommands/workflows';
  import WorkflowList from './list/WorkflowList.svelte';
  import { createBlankWorkflow, deleteWorkflow, duplicateWorkflow, renameWorkflow } from './list/workflowActions';
  import { EditorLazy, NewWorkflowLazy, OptimizeLazy, RunPageLazy, StartRunLazy } from './lazyViews';
  import RunsInbox from './runs/RunsInbox.svelte';
  import { stopRun } from './runs/runActions';
  import { clockTime, fromActive, inboxFold, runTitle, type InboxRun } from './runs/runWords';
  import { parseWorkflowsRoute, runDeepLink, runRoutePath } from './runRoute';
  import { workflowsUi } from './workflowsUiState.svelte';

  const route = $derived(parseWorkflowsRoute(router.parts));
  const routeWorkflowId = $derived(route.view === 'runs' || route.view === 'list' ? null : route.workflowId);

  // The workspace's workflows: the Workflows tab, the inbox's filter, the
  // start form's picker and the agent port read the same list.
  let workflows = $state<Workflow[]>([]);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let loadSeq = 0;
  async function load(): Promise<void> {
    const wsId = ws.currentId;
    if (!wsId) return;
    const seq = ++loadSeq;
    loading = true;
    try {
      const rows = await api.get<Workflow[]>(`/workspaces/${wsId}/workflows`);
      if (seq !== loadSeq) return;
      workflows = rows;
      error = null;
    } catch (e) {
      if (seq === loadSeq) error = loadErrorText(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }
  // Per workspace, and again whenever a tab is shown (a save or a run in
  // another screen changed its rows).
  $effect(() => {
    const id = ws.currentId;
    const tab = route.view === 'runs' || route.view === 'list';
    if (!id || !tab) return;
    untrack(() => {
      workflows = workflows.filter((w) => w.workspace_id === id);
      void load();
      void ws.refreshActiveWorkflowRuns();
    });
  });

  const needsCount = $derived(ws.activeWorkflowRuns.filter((r) => inboxFold(fromActive(r).state) === 'needs').length);

  // ── the start form (Run…, Run again…) and New workflow ─────────────────
  let starting = $state<{ workflow: Workflow | null; prefill?: { input: unknown; from: string; originLabel?: string }; typicalMs?: number | null } | null>(null);
  let creating = $state(false);

  function startRun(wf: Workflow | null = null): void {
    starting = { workflow: wf };
  }
  const dayTime = (iso: string): string => {
    const d = new Date(iso);
    const day = d.toDateString() === new Date().toDateString() ? 'today' : d.toLocaleDateString([], { month: 'short', day: 'numeric' });
    return `${day}, ${clockTime(iso)}`;
  };
  async function runAgain(run: WorkflowRun): Promise<void> {
    try {
      const wf = workflows.find((w) => w.id === run.workflow_id) ?? (await api.get<Workflow>(`/workflows/${encodeURIComponent(run.workflow_id)}`));
      const ov = run.overview ?? null;
      const subject = runTitle({ overview: ov, message: null, branch: null, repo: null, pr: null, workflowName: wf.name });
      starting = { workflow: wf, prefill: { input: run.input, from: `${subject} (${dayTime(run.started_at)})`, originLabel: ov?.origin?.channel_name ?? undefined }, typicalMs: ov?.typical_ms ?? null };
    } catch (e) {
      toastError('Couldn’t open the start form', e);
    }
  }
  function created(wf: Workflow): void {
    creating = false;
    workflows = [wf, ...workflows.filter((w) => w.id !== wf.id)];
    router.go(`workflows/${wf.id}`);
  }
  async function newBlank(): Promise<void> {
    const wf = ws.currentId ? await createBlankWorkflow(ws.currentId) : null;
    if (wf) created(wf);
  }

  /** "Runs of this workflow": the Runs tab, filtered. */
  function runsOf(workflowId: string): void {
    workflowsUi.runsFilter = workflowId;
    router.go('workflows');
  }

  function runMenu(r: InboxRun): MenuItem[] {
    const items: MenuItem[] = [
      { label: 'Copy link', icon: 'link', action: () => void copyWithToast(runDeepLink(window.location, r.workflowId, r.id), 'Run link') },
      { label: 'Open workflow', icon: 'workbench', action: () => router.go(`workflows/${r.workflowId}`) },
      { label: 'Run again with these inputs…', icon: 'play', action: () => void api.get<WorkflowRun>(`/workflow-runs/${encodeURIComponent(r.id)}`).then(runAgain, (e) => toastError('Couldn’t read the run', e)) },
    ];
    if (inboxFold(r.state) !== 'recent') items.push({ separator: true }, { label: 'Stop run…', icon: 'stop', danger: true, action: () => void stopRun(r.id) });
    return items;
  }
  function workflowMenu(wf: Workflow): MenuItem[] {
    const wsId = ws.currentId ?? '';
    return [
      { label: 'Open', icon: 'external', action: () => router.go(`workflows/${wf.id}`) },
      { label: 'Run…', icon: 'play', action: () => startRun(wf) },
      { label: 'Runs of this workflow', icon: 'clock', action: () => runsOf(wf.id) },
      { label: 'Optimize…', icon: 'zap', action: () => router.go(`workflows/${wf.id}/optimize`) },
      { separator: true },
      { label: 'Rename…', icon: 'edit', action: async () => { const w = await renameWorkflow(wf); if (w) workflows = workflows.map((x) => (x.id === w.id ? { ...x, name: w.name } : x)); } },
      { label: 'Duplicate', icon: 'copy', action: async () => { const c = await duplicateWorkflow(wsId, wf); if (c) created(c); } },
      { separator: true },
      { label: 'Delete workflow…', icon: 'trash', danger: true, action: async () => { if (await deleteWorkflow(wf)) workflows = workflows.filter((x) => x.id !== wf.id); } },
    ];
  }

  // ⌘K: the module's verbs (the run page and the editor add their own).
  $effect(() => {
    const open = routeWorkflowId;
    return registry.register('workflows', [
      { id: 'workflows.new', title: 'New workflow…', group: 'Workflows', keywords: 'create describe generate automation template', run: () => (creating = true) },
      { id: 'workflows.blank', title: 'New blank workflow', group: 'Workflows', keywords: 'create empty canvas', run: () => void newBlank() },
      { id: 'workflows.run', title: 'Run workflow…', group: 'Workflows', keywords: 'start execute input', run: () => startRun(workflows.find((w) => w.id === open) ?? null) },
      ...(open && route.view === 'editor'
        ? [{ id: 'workflows.optimize', title: 'Optimize workflow…', group: 'Workflows', keywords: 'faster parallel speed', run: () => router.go(`workflows/${open}/optimize`) }]
        : []),
    ]);
  });

  // Agent UI control (lib/uiCommands/workflows.ts): the agent opens workflows
  // and runs through the same routes a person's clicks use.
  $effect(() =>
    workflowsPagePort.bind({
      list: () => workflows,
      loading: () => loading,
      currentId: () => routeWorkflowId,
      open: (id: string) => router.goChecked(`workflows/${id}`),
      openRun: async (workflowId: string, runId: string) => void (await router.goChecked(runRoutePath(workflowId, runId))),
      async start(body: RunWorkflowReq): Promise<WorkflowRun | null> {
        const id = routeWorkflowId;
        if (!id) return null;
        try {
          const wf = await api.get<Workflow>(`/workflows/${encodeURIComponent(id)}`);
          const check = await api.post<{ valid: boolean; issues: WorkflowValidationIssue[] }>(`/workflows/${id}/validate`, { graph: wf.graph });
          if (!check.valid) {
            toasts.error('The workflow has problems', check.issues[0]?.message ?? 'Open the workflow to fix them.');
            return null;
          }
          const run = await api.post<WorkflowRun>(`/workflows/${id}/run`, body);
          router.go(runRoutePath(id, run.id));
          return run;
        } catch (e) {
          toastError('Couldn’t start the run', e);
          return null;
        }
      },
      currentRun: () => workflowsUi.shownRun,
    }),
  );
</script>

<div class="wf-root">
{#if route.view === 'runs' || route.view === 'list'}
  {@const tab = route.view}
  <PageHeader title="Workflows" tabsPlacement={viewport.isPhone ? 'below' : 'inline'} more={() => [{ label: 'Refresh', icon: 'refresh', action: () => void load() }]}>
    {#snippet tabs()}
      <div class="segmented" role="tablist" tabindex="-1" aria-label="Workflows view" onkeydown={(e) => onTabKey(e, { activate: 'click' })}>
        <button type="button" role="tab" data-testid="wf-tab-runs" tabindex={tab === 'runs' ? 0 : -1} aria-selected={tab === 'runs'} class:active={tab === 'runs'} onclick={() => router.go('workflows')}>
          Runs{#if needsCount}<span class="needs" title="{needsCount} waiting on you">{needsCount}</span>{/if}
        </button>
        <button type="button" role="tab" data-testid="wf-tab-workflows" tabindex={tab === 'list' ? 0 : -1} aria-selected={tab === 'list'} class:active={tab === 'list'} onclick={() => router.go('workflows/all')}>Workflows</button>
      </div>
    {/snippet}
    {#snippet actions()}
      <AutomateGuideButton current="workflows" />
      {#if tab === 'runs' && workflows.length}
        <button type="button" class="btn primary" onclick={() => startRun()}><Icon name="play" size={12} /> Run…</button>
      {:else if tab === 'list' && workflows.length && !viewport.isPhone}
        <button type="button" class="btn primary" onclick={() => (creating = true)}><Icon name="plus" size={12} /> New workflow</button>
      {/if}
    {/snippet}
  </PageHeader>
  <PageBody padded={false}>
    {#if !loading && !error && !workflows.length && !ws.activeWorkflowRuns.length}
      <!-- A first visit: which Automate tool fits, beside the empty state. -->
      <div class="read-col wf-guide"><AutomateGuide current="workflows" open /></div>
    {/if}
    {#if tab === 'runs'}
      <RunsInbox workspaceId={ws.currentId ?? ''} active={ws.activeWorkflowRuns} workflows={workflows} bind:workflowFilter={workflowsUi.runsFilter}
        onopen={(wf, run) => router.go(runRoutePath(wf, run))} menu={runMenu} onrun={() => startRun()} onnewworkflow={viewport.isPhone ? undefined : () => (creating = true)} />
    {:else}
      <WorkflowList {workflows} {loading} {error} onretry={() => void load()} onopen={(wf) => router.go(`workflows/${wf.id}`)} menu={workflowMenu} onnew={viewport.isPhone ? undefined : () => (creating = true)} />
    {/if}
  </PageBody>
{:else if route.view === 'run'}
  {#key route.runId}
    {@const r = route}
    <LazyMount lazy={RunPageLazy} what="the run" props={{
      workspaceId: ws.currentId ?? '', workflowId: r.workflowId, runId: r.runId, nodeId: r.nodeId ?? null, childId: r.childId ?? null,
      onselect: (n: string | null, c: string | null) => router.go(runRoutePath(r.workflowId, r.runId, n, c)),
      onback: () => router.go('workflows'),
      onopenworkflow: () => router.go(`workflows/${r.workflowId}`),
      onrunagain: (run: WorkflowRun) => void runAgain(run),
    }} />
  {/key}
{:else if route.view === 'optimize'}
  {#key route.workflowId}<LazyMount lazy={OptimizeLazy} what="Optimize" props={{ workflowId: route.workflowId }} />{/key}
{:else}
  {#key route.workflowId}
    <LazyMount lazy={EditorLazy} what="the workflow" props={{ workflowId: route.workflowId, onrunsof: runsOf, ondeleted: () => router.go('workflows/all') }} />
  {/key}
{/if}
</div>

{#if starting}
  <LazyMount lazy={StartRunLazy} what="the start form" quiet onerror={() => (starting = null)}
    props={{ workflow: starting.workflow, workflows: workflows.filter((w) => w.origin !== 'goal'), prefill: starting.prefill ?? null, typicalMs: starting.typicalMs ?? null, onclose: () => (starting = null) }} />
{/if}
{#if creating && ws.currentId}
  <LazyMount lazy={NewWorkflowLazy} what="New workflow" quiet onerror={() => (creating = false)} props={{ workspaceId: ws.currentId, oncreated: created, onclose: () => (creating = false) }} />
{/if}

<style>
  .wf-root {
    display: flex;
    flex-direction: column;
    block-size: 100%;
    min-block-size: 0;
  }
  .wf-guide {
    padding-block-start: var(--sp-6);
  }
  .needs {
    margin-inline-start: var(--sp-2);
    padding-inline: 6px;
    border-radius: 999px;
    background: var(--warning-soft);
    color: var(--warning);
    font-size: var(--fs-xs);
    font-weight: 600;
  }
</style>
