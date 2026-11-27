<script lang="ts">
  // One run's page (ux.md §4, §5): the header (subject, the run's word, Copy
  // link, the runState's one primary, Stop never beside it), the run column (ring,
  // sentence, facts, the runState card, the steps in execution order) and the
  // step pane beside it when the content is at least 880 px wide; narrower, a
  // step opens as a pushed screen with its own back control. Mounted by the
  // route #/workflows/<wf>/runs/<run>[/<node>[/<child>]]: `nodeId`/`childId`
  // are the route's, `onselect` writes it. Live through RunSync.
  import { onMount, untrack } from 'svelte';
  import EmptyState from '../../../lib/components/EmptyState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import PageBody from '../../../lib/components/PageBody.svelte';
  import PageHeader from '../../../lib/components/PageHeader.svelte';
  import { api, ApiError } from '../../../lib/api/client';
  import { events } from '../../../lib/events.svelte';
  import { copyWithToast } from '../../../lib/clipboard';
  import type { MenuItem } from '../../../lib/contextmenu.svelte';
  import { loadErrorText } from '../../../lib/loadError';
  import { loadIdeRuntimeSessionPort, type RuntimeSessionPort } from '../../../lib/runtimeThreadPort';
  import { now as clockNow } from '../../../lib/stores/now.svelte';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import type { NodeRunState, Workflow, WorkflowGraph, WorkflowRun, WorkflowVersion } from '../../../lib/api/types';
  import { runDeepLink } from '../runRoute';
  import { RunSync } from '../runSync.svelte';
  import { graphCriteria } from '../goal/goalCheck';
  import RunOverview from './RunOverview.svelte';
  import StepList from './StepList.svelte';
  import StepPane from './StepPane.svelte';
  import { cancelQueued, pauseRun, rereview, resumeRun, retryStep, steerRun, stopRun } from './runActions';
  import { fallbackState, runTitle, runWord } from './runWords';
  import { defaultSelection, entrySteps, loopChildren, mergeStepChildren, orderSteps, RESULT, sameStepChildren, settledNodes, stepNumber, type StepChild } from './stepOrder';
  import { markRunOpened } from './seenRuns';
  import { registry } from '../../../lib/commands.svelte';
  import { workflowsUi } from '../workflowsUiState.svelte';

  interface Props {
    workspaceId: string;
    workflowId: string;
    runId: string;
    nodeId?: string | null;
    childId?: string | null;
    /** A step (or reviewer) was picked; null goes back to the run. */
    onselect: (nodeId: string | null, childId: string | null) => void;
    onback: () => void;
    onopenworkflow?: () => void;
    /** "Run again…": the start form, pre-filled from this run's input. */
    onrunagain?: (run: WorkflowRun) => void;
  }
  let { workspaceId, workflowId, runId, nodeId = null, childId = null, onselect, onback, onopenworkflow, onrunagain }: Props = $props();

  const sync = new RunSync();
  sync.track();

  let graph = $state<WorkflowGraph>({ nodes: [], edges: [] });
  let workflowName = $state('');
  let loading = $state(true);
  let error = $state<string | null>(null);
  let missing = $state(false);
  let reload = $state(0);
  /** A card's ask for a tab ("Answer in the conversation"). */
  let askedTab = $state<string | null>(null);
  let maximized = $state(false);
  let width = $state(1200);
  // Threads runtime store (through the port): keeps agent-opened sub-threads
  // live in the step list the moment they are created (before the step settles).
  let rtPort = $state<RuntimeSessionPort | null>(null);
  onMount(() => {
    markRunOpened(runId);
    let active = true;
    let unmountRt: (() => void) | null = null;
    void loadIdeRuntimeSessionPort()
      .then((port) => {
        if (!active) return;
        rtPort = port;
        unmountRt = port.runtimeThreads.mount();
      })
      .catch(() => {});
    return () => {
      active = false;
      unmountRt?.();
    };
  });

  const currentRunId = $derived(runId);
  $effect(() => {
    const id = currentRunId;
    void reload;
    loading = true;
    error = null;
    missing = false;
    (async () => {
      try {
        const run = await api.get<WorkflowRun>(`/workflow-runs/${encodeURIComponent(id)}`);
        const g = await (run.workflow_version
          ? api.get<WorkflowVersion>(`/workflows/${encodeURIComponent(run.workflow_id)}/versions/${run.workflow_version}`).catch(() => api.get<Workflow>(`/workflows/${encodeURIComponent(run.workflow_id)}`))
          : api.get<Workflow>(`/workflows/${encodeURIComponent(run.workflow_id)}`)).catch(() => null);
        if (id !== currentRunId) return;
        graph = g?.graph ?? { nodes: [], edges: [] };
        workflowName = g?.name ?? '';
        sync.run = run;
      } catch (e) {
        if (e instanceof ApiError && e.status === 404) missing = true;
        else error = loadErrorText(e);
      } finally {
        loading = false;
      }
    })();
  });

  const run = $derived(sync.run?.id === runId ? sync.run : null);
  const now = $derived(clockNow());
  const runState = $derived(run ? (run.overview?.state ?? fallbackState(run.status, run.waiting_approval)) : 'unknown');
  const live = $derived(run?.status === 'pending' || run?.status === 'running');
  // The socket is reconnecting: what the page shows may be behind (ux.md §4.6).
  const word = $derived(
    !run ? { word: '', tone: '' as const }
    : live && events.state === 'connecting' ? { word: 'Reconnecting…', tone: '' as const }
    : runWord(run.overview ?? { state: runState, attention: [] }, { startedAt: run.started_at, now, raw: run.status }),
  );
  // When the page last heard of the run: the sub line says it while the socket is down.
  let heardAt = $state(Date.now());
  $effect(() => {
    void run?.rev;
    heardAt = Date.now();
  });
  const stale = $derived(!!run && live && events.state === 'connecting');
  // The agent UI port's "the run on screen" (lib/uiCommands/workflows.ts).
  $effect(() => {
    workflowsUi.shownRun = run;
  });
  $effect(() => () => (workflowsUi.shownRun = null));
  // Watch each step's thread family and load its linked sub-threads so StepList
  // stays live across ticks, retries failed loads, and survives reconnects.
  const stepThreadKey = $derived(
    run
      ? [...new Set(
          run.nodes.flatMap((n) => [
            ...(n.thread?.runner === 'threads' && n.thread.available !== false && n.thread.thread_id ? [n.thread.thread_id] : []),
            ...(n.children ?? []).flatMap((c) => (c.thread_id ? [c.thread_id] : [])),
          ]),
        )].join(',')
      : '',
  );
  $effect(() => {
    const port = rtPort;
    const key = stepThreadKey;
    if (!port || !key) return;
    const ids = key.split(',').filter(Boolean);
    const releases = untrack(() => ids.map((tid) => port.runtimeThreads.watchGroup(tid)));
    let stopped = false;
    const timers: ReturnType<typeof setTimeout>[] = [];
    const loadFamily = (tid: string, attempt: number): void => {
      if (stopped) return;
      const ready = port.runtimeThreads.get(tid) ? Promise.resolve() : port.runtimeThreads.loadOne(tid).then(() => undefined);
      void ready
        .then(() => port.runtimeThreads.loadLinked(tid, attempt === 0 ? 2000 : 0))
        .catch((e: unknown) => {
          const status = (e as { status?: unknown } | null)?.status;
          if (stopped || status === 403 || status === 404 || attempt >= 3) return;
          timers.push(setTimeout(() => loadFamily(tid, attempt + 1), 1500 * 2 ** attempt));
        });
    };
    untrack(() => {
      for (const tid of ids) loadFamily(tid, 0);
    });
    return () => {
      stopped = true;
      for (const t of timers) clearTimeout(t);
      for (const off of releases) off();
    };
  });

  // A loop step's iterations are its child rows (from the run's checkpoints),
  // merged with every live/known agent-opened sub-thread in each step's group.
  // Keep previous node/array references when children are structurally unchanged
  // so streaming `last_active_at` updates don't churn the step pane.
  const rtSessions = $derived(rtPort ? Object.values(rtPort.runtimeThreads.byId) : []);
  let prevMerged = new Map<string, { base: NodeRunState; children: StepChild[]; node: NodeRunState }>();
  let prevNodeList: NodeRunState[] = [];
  const nodes = $derived.by(() => {
    if (!run) {
      prevMerged = new Map();
      return (prevNodeList = []);
    }
    const next = new Map<string, { base: NodeRunState; children: StepChild[]; node: NodeRunState }>();
    const out = settledNodes(run.nodes, live).map((n) => {
      const baseChildren =
        !n.children?.length && run.checkpoints?.some((c) => c.loop_id === n.node_id)
          ? loopChildren(run.checkpoints, n.node_id)
          : (n.children ?? []);
      const rootTid = n.thread?.runner === 'threads' ? n.thread.thread_id : null;
      const merged = rootTid || baseChildren.some((c) => c.thread_id)
        ? mergeStepChildren(baseChildren, rootTid, rtSessions)
        : (baseChildren as StepChild[]);
      if (merged.length === 0 && !n.children?.length) {
        next.set(n.node_id, { base: n, children: merged, node: n });
        return n;
      }
      const prev = prevMerged.get(n.node_id);
      if (prev && prev.base === n && sameStepChildren(prev.children, merged)) {
        next.set(n.node_id, prev);
        return prev.node;
      }
      const wrapped = merged !== n.children ? { ...n, children: merged } : n;
      next.set(n.node_id, { base: n, children: merged, node: wrapped });
      return wrapped;
    });
    prevMerged = next;
    if (prevNodeList.length === out.length && prevNodeList.every((x, i) => x === out[i])) return prevNodeList;
    return (prevNodeList = out);
  });
  // `orderSteps` does not depend on `now`, so 1s clock ticks don't rebuild
  // entries/openStep or churn StepPane's props while reading a live conversation.
  const entries = $derived(run ? orderSteps(nodes, graph) : []);
  const steps = $derived(entries.flatMap(entrySteps));
  const selected = $derived(nodeId ? { node: nodeId, child: childId } : defaultSelection(runState, entries, run?.overview?.failed_step?.node_id));
  const openStep = $derived(selected.node === RESULT ? null : (steps.find((s) => s.id === selected.node) ?? null));
  const wide = $derived(width >= 880);
  const pushed = $derived(!wide && !!nodeId);
  const nodeName = (id: string) => steps.find((s) => s.id === id)?.name ?? id;
  const criteria = $derived(graphCriteria(graph));
  const failedNode = $derived(run?.overview?.failed_step?.node_id ?? run?.nodes.find((n) => n.status === 'error')?.node_id ?? null);
  const title = $derived(run ? runTitle({ overview: run.overview ?? null, message: null, branch: null, repo: null, pr: null, workflowName: workflowName || 'Workflow' }) : 'Run');

  function select(node: string, child: string | null): void {
    askedTab = null;
    onselect(node, child);
  }
  function selectChild(cid: string | null): void {
    if (openStep) select(openStep.id, cid);
  }
  function toggleMaximize(): void {
    maximized = !maximized;
  }
  function openStepTab(node: string, tab?: string): void {
    onselect(node, null);
    askedTab = tab ?? null;
  }

  type Primary = { action: 'retry' | 'run-again' | 'resume'; label: string; icon: 'refresh' | 'play'; go: () => void } | null;
  const primary = $derived.by((): Primary => {
    if (!run) return null;
    if (runState === 'failed' && failedNode) return { action: 'retry', label: 'Retry step', icon: 'refresh', go: () => void retryStep(run.id, failedNode) };
    if (runState === 'paused') return { action: 'resume', label: 'Resume', icon: 'play', go: () => void resumeRun(run.id) };
    if ((runState === 'succeeded' || runState === 'canceled') && onrunagain) return { action: 'run-again', label: 'Run again…', icon: 'play', go: () => onrunagain(run) };
    return null;
  });
  const showStop = $derived(runState === 'running' || runState === 'waiting');

  // ⌘K while the run is on screen (ux.md §14.5). Approve stays on the card:
  // a publish gate needs its preview read first.
  $effect(() => {
    if (!run) return;
    const r = run;
    const g = 'Workflows';
    return registry.register('workflows.run', [
      { id: 'workflows.run.copy', title: 'Copy run link', group: g, run: () => void copyWithToast(runDeepLink(window.location, r.workflow_id, r.id), 'Run link') },
      ...(onopenworkflow ? [{ id: 'workflows.run.open', title: 'Open workflow', group: g, run: onopenworkflow }] : []),
      ...(onrunagain ? [{ id: 'workflows.run.again', title: 'Run again with these inputs…', group: g, run: () => onrunagain(r) }] : []),
      ...(primary?.action === 'retry' ? [{ id: 'workflows.run.retry', title: 'Retry step', group: g, run: primary.go }] : []),
      ...(live || ['needs_you', 'paused', 'blocked', 'exhausted'].includes(runState) ? [{ id: 'workflows.run.stop', title: 'Stop run…', group: g, keywords: 'cancel halt', run: () => void stopRun(r.id) }] : []),
    ]);
  });

  function more(): MenuItem[] {
    if (!run) return [];
    const r = run;
    const items: MenuItem[] = [
      { label: 'Copy link', icon: 'link', action: () => void copyWithToast(runDeepLink(window.location, r.workflow_id, r.id), 'Run link') },
    ];
    if (onopenworkflow) items.push({ label: 'Open workflow', icon: 'workbench', action: onopenworkflow });
    if (onrunagain && primary?.action !== 'run-again') items.push({ label: 'Run again with these inputs…', icon: 'play', action: () => onrunagain(r) });
    if (runState === 'failed' && failedNode) items.push({ label: 'Retry only this step', icon: 'refresh', action: () => void retryStep(r.id, failedNode, false) });
    if (runState === 'canceled' && steps.some((s) => s.start !== null)) {
      const last = [...steps].reverse().find((s) => s.start !== null)!;
      items.push({ label: 'Retry step', icon: 'refresh', action: () => void retryStep(r.id, last.id) });
    }
    if (runState === 'succeeded' && graph.nodes.some((n) => n.kind === 'review_run')) items.push({ label: 'Re-review', icon: 'refresh', action: () => void rereview(r.id) });
    if (runState === 'running' || runState === 'waiting') {
      items.push({ label: 'Pause', icon: 'pause', action: () => void pauseRun(r.id) });
      items.push({ label: 'Add a note for the next step…', icon: 'note', action: () => void steerRun(r.id) });
    }
    if (['needs_you', 'paused', 'blocked', 'exhausted'].includes(runState)) items.push({ separator: true }, { label: 'Stop run…', icon: 'stop', danger: true, action: () => void stopRun(r.id) });
    return items;
  }
</script>

{#if pushed}
  <!-- A pushed step, reviewer or the run's Result: Back returns to the run. -->
  <PageHeader title={openStep?.name ?? 'Result'} crumbs={[]}>
    {#snippet leading()}
      <button type="button" class="icon-btn" aria-label="Back to the run" title="Back to the run" onclick={() => onselect(null, null)}><Icon name="chevronLeft" size={16} /></button>
    {/snippet}
  </PageHeader>
{:else}
  <PageHeader {title} crumbs={workflowName && onopenworkflow && !viewport.isPhone ? [{ label: workflowName, onclick: onopenworkflow }] : []} {more}>
    {#snippet leading()}
      <button type="button" class="icon-btn" aria-label="Back to runs" title="Back to runs" onclick={onback}><Icon name="chevronLeft" size={16} /></button>
    {/snippet}
    {#snippet badge()}
      {#if run && !viewport.isPhone}<span class="rword {word.tone}">{word.word}</span>{/if}
    {/snippet}
    {#snippet actions()}
      {#if run}
        {#if !viewport.isPhone}<button type="button" class="icon-btn" data-overflow="1" aria-label="Copy link" title="Copy link" onclick={() => void copyWithToast(runDeepLink(window.location, run.workflow_id, run.id), 'Run link')}><Icon name="link" size={13} /></button>{/if}
        {#if runState === 'queued'}
          <button type="button" class="btn" data-testid="wf-run-cancel" onclick={() => void cancelQueued(run.id)}>Cancel run</button>
        {:else if showStop}
          <button type="button" class="btn danger" data-testid="wf-run-stop" data-label="Stop run…" onclick={() => void stopRun(run.id)}><Icon name="stop" size={13} />Stop run…</button>
        {/if}
        {#if primary}
          <button type="button" class="btn primary" data-testid="wf-run-primary" data-action={primary.action} onclick={primary.go}><Icon name={primary.icon} size={13} />{primary.label}</button>
        {/if}
      {/if}
    {/snippet}
  </PageHeader>
{/if}

<PageBody padded={false} fill>
  <div class="layout" class:wide bind:clientWidth={width} data-testid="wf-run-page" data-run-id={runId} data-state={runState}>
    {#if missing}
      <EmptyState variant="page" icon="workbench" title="This run no longer exists" body="It may have been deleted with its workflow." actionLabel="Go to Runs" onaction={onback} headingLevel={2} />
    {:else if !run}
      <LoadState what="the run" variant="page" {loading} {error} empty onretry={() => reload++} />
    {:else}
      {#if !pushed && !(wide && maximized)}
        <div class="col">
          <RunOverview {run} {runState} word={word.word} {entries} {sync} {now} {nodeName} {criteria} onopenstep={openStepTab} staleSince={stale ? heardAt : null} />
          <StepList {entries} {runState} clock={{ start: Date.parse(run.started_at), end: run.finished_at ? Date.parse(run.finished_at) : now }} {now} selected={wide ? selected : { node: '', child: null }} result={run.status === 'success' || run.status === 'canceled'} compact={!wide} onselect={select} />
        </div>
      {/if}
      {#if wide || pushed}
        <div class="pane-col">
          {#if wide && maximized}
            <button type="button" class="btn small runback" onclick={() => (maximized = false)}><Icon name="chevronLeft" size={12} />Run</button>
          {/if}
          <StepPane {run} {sync} {workspaceId} {runState} cardNode={pushed || (wide && maximized) || runState !== 'failed' ? null : failedNode} step={openStep} childId={selected.child} stepNo={openStep ? stepNumber(steps, openStep) : null} stepCount={steps.length} {now} tab={askedTab} {criteria} {maximized} onmaximize={wide ? toggleMaximize : undefined} onselectchild={openStep ? selectChild : undefined} />
        </div>
      {/if}
    {/if}
  </div>
</PageBody>

<style>
  .layout {
    display: flex;
    flex: 1;
    min-block-size: 0;
    min-inline-size: 0;
  }
  .col,
  .pane-col {
    display: flex;
    flex-direction: column;
    gap: var(--sp-6);
    min-block-size: 0;
    overflow-y: auto;
    padding: var(--sp-6);
  }
  .col {
    flex: 1;
  }
  .wide .col {
    flex: none;
    inline-size: clamp(320px, 34%, 440px);
    border-inline-end: 1px solid var(--border);
  }
  .pane-col {
    flex: 1;
    min-inline-size: 0;
  }
  /* When the open tab is a live Threads conversation, lock the outer column so
     ONLY Threads' own `.rt-scroll` scrolls — no nested scroller fighting it. */
  .pane-col:has(:global([data-conv-thread='1'])) {
    overflow: hidden;
  }
  .runback {
    align-self: flex-start;
  }
  .rword {
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text-dim);
  }
  .rword.danger {
    color: var(--danger);
  }
  .rword.warning {
    color: var(--warning);
  }
  .rword.success {
    color: var(--success);
  }
  @media (max-width: 640px) {
    .col,
    .pane-col {
      padding: var(--sp-5);
    }
  }
</style>
