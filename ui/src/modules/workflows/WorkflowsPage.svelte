<script lang="ts">
  import { announceShown } from '../../lib/announce.svelte';
  import { rowMenu as rowMenuKeys } from '../../lib/rowMenu';
  import { toastError } from '../../lib/toastError';
  import { focusOnMount } from '../../lib/focusOnMount';
  // Workflows: build automations by *describing* them (agent mode) or by hand
  // on the canvas. Left = generate + list + running; center = node-graph editor + run.
  import { tick, untrack } from 'svelte';
  import Icon, { asIcon } from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import Badge from '../../lib/components/Badge.svelte';
  import { runStatus } from '../../lib/status';
  import Modal from '../../lib/components/Modal.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import PageBody from '../../lib/components/PageBody.svelte';
  import PaneDivider from '../../lib/components/PaneDivider.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import AutomateGuide from '../../lib/components/AutomateGuide.svelte';
  import AutomateGuideButton from '../../lib/components/AutomateGuideButton.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
  import { effectiveRetry, updateRetry, clearRetry } from './retryPolicy';
  import WorkflowCanvas from './WorkflowCanvas.svelte';
  import RunMonitor from './RunMonitor.svelte';
  import RunApprovals from './RunApprovals.svelte';
  import RunContextSidebar from './RunContextSidebar.svelte';
  import { RunSync } from './runSync.svelte';
  import TriggersPanel from './TriggersPanel.svelte';
  import NodeParams from './inspector/NodeParams.svelte';
  import { InspectorOptions } from './inspector/options.svelte';
  import { jsonValue, paramJson, withParam } from './inspector/util';
  import { ui } from '../../lib/stores/ui.svelte';
  import { startMouseDrag } from '../../lib/dragCursor';
  import { LIST_PANE, loadPaneWidth, paneResizer } from '../../lib/paneResizer';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { api } from '../../lib/api/client';
  import { patchWorkflow, workflowProgress, workflowNodeDetail, listWorkflowVersions, restoreWorkflowVersion } from '../../lib/api/workflows';
  import { fmtStepMs, sharedNodeBodies } from './runProgress';
  import RelTime from '../../lib/components/RelTime.svelte';
  import { workflowsPagePort } from '../../lib/uiCommands/workflows';
  import { router } from '../../lib/router.svelte';
  import { guardUnsaved } from '../../lib/leaveGuard';
  import { registry } from '../../lib/commands.svelte';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import { RunInputForm } from './runInputForm.svelte';
  import RunInputPanel from './RunInputPanel.svelte';
  import type {
    Workflow,
    WorkflowGraph,
    WorkflowRun,
    NodeTypeSpec,
    NodeRunState,
    WorkflowTemplate,
    WorkflowTrigger,
    WorkflowVersionSummary,
    RunWorkflowReq,
  } from '../../lib/api/types';

  let workflows = $state<Workflow[]>([]);
  /** List load state — a failed load renders inline with Retry, never as the
   *  "Build a workflow" empty state. */
  let wfLoading = $state(true);
  let wfError = $state<string | null>(null);
  let templates = $state<WorkflowTemplate[]>([]);
  let types = $state<NodeTypeSpec[]>([]);
  let current = $state<Workflow | null>(null);
  let graph = $state<WorkflowGraph>({ nodes: [], edges: [] });
  let selectedId = $state<string | null>(null);
  // The list pane's width: the shared LIST_PANE range, remembered per device.
  // A width set under the old key (ui.wfSideWidth) carries over on first read.
  const LIST_W_KEY = 'workflows.listW';
  let listW = $state(loadPaneWidth(LIST_W_KEY, Math.max(LIST_PANE.min, Math.min(LIST_PANE.max, ui.wfSideWidth)), LIST_PANE.min, LIST_PANE.max));
  let dirty = $state(false);
  let savingGraph = $state(false);

  let prompt = $state('');
  let generating = $state(false);
  /** Only while the run POST is in flight (double-click guard). It used to stay
   *  set until the started run finished — hours for agent steps — disabling Run
   *  on EVERY workflow and pointing the header's Cancel at whatever run was on
   *  screen. The header now follows the viewed run (`runActive`). */
  let running = $state(false);
  // The run on screen and its live sync, approvals and final output
  // (runSync.svelte.ts). The page assigns `mon.run`; reads use `run`.
  const mon = new RunSync();
  mon.track();
  const run = $derived(mon.run);
  let runs = $state<Pick<WorkflowRun, 'id' | 'workflow_id' | 'status' | 'started_at' | 'rev'>[]>([]);
  let requestedRunId: string | null = null;
  let runsOpen = $state(false);
  // Manual-run input editor: where you provide repo_id / story_id / goals / msg
  // that the trigger emits to the graph.
  let paletteOpen = $state(false);
  let templatesOpen = $state(false);
  let triggersOpen = $state(false);
  let triggers = $state<WorkflowTrigger[]>([]);

  // Instructions editor: standing free-text guidance every step follows,
  // distinct from `description`. Saved via an explicit action (like graph
  // Save), not tied into the canvas's `dirty` flag.
  let instructionsOpen = $state(false);
  let wfInstructions = $state('');
  let savingInstructions = $state(false);

  const runStates = $derived<Record<string, NodeRunState>>(
    Object.fromEntries((run?.nodes ?? []).map((n) => [n.node_id, n])),
  );
  const selectedNode = $derived(graph.nodes.find((n) => n.id === selectedId) ?? null);
  const selectedSummary = $derived(selectedId ? (runStates[selectedId] ?? null) : null);
  let inspectorBody = $state<NodeRunState | null>(null);
  /** Why the step-output fetch failed (null = ok) → "Couldn’t load the step output · Retry". */
  let inspectorError = $state<string | null>(null);
  let inspectorRetry = $state(0);
  const selectedRun = $derived(inspectorBody ?? selectedSummary);
  // A RUNNING node's detail_version bumps per log line: refetch its body at
  // most ~1/s (first read of a newly selected node is immediate), and abort a
  // superseded read instead of letting it hold a connection (backlog B6 /
  // SD-12). The previous body stays shown until the new one lands.
  let inspectorFor = '';
  $effect(() => {
    const id = run?.id, node = selectedSummary;
    void inspectorRetry; // Retry re-runs this effect
    const target = id && node ? `${id}\0${node.node_id}` : '';
    if (target !== inspectorFor) {
      inspectorFor = target;
      inspectorBody = null;
      inspectorError = null;
    }
    if (!id || !node?.detail_version) return;
    const expected = node.detail_version;
    // The step list may already hold this exact body (one shared fetch path,
    // so a node that is both expanded and selected is read once per version).
    const cached = sharedNodeBodies.peek<NodeRunState>(id, node.node_id, expected);
    if (cached) {
      inspectorBody = cached;
      return;
    }
    const ctl = new AbortController();
    // untrack: reading the body must not re-run this effect when it lands.
    const delay = untrack(() => inspectorBody) && node.status === 'running' ? 1000 : 0;
    const timer = setTimeout(() => {
      void sharedNodeBodies.fetch(id, node.node_id, (signal) => workflowNodeDetail(id, node.node_id, signal), ctl.signal).then(result => {
        if (!ctl.signal.aborted && result.detail_version === expected) {
          inspectorBody = result.body;
          inspectorError = null;
        }
      }).catch((e: unknown) => {
        // The previous body (if any) stays; a failure is said, not swallowed.
        if (!ctl.signal.aborted) inspectorError = loadErrorText(e);
      });
    }, delay);
    return () => { clearTimeout(timer); ctl.abort(); };
  });
  const instructionsDirty = $derived(wfInstructions !== (current?.instructions ?? ''));
  // Leaving the module (sidebar, ⌘K, a link) or switching workspace with an
  // unsaved canvas / instructions edit asks first — `discardEditsOk` only
  // covered switching workflows inside the page. Moving within Workflows
  // (`workflows/<id>`) is handled by `openGuarded`, so it's allowed here.
  $effect(() =>
    guardUnsaved(() => (dirty || instructionsDirty) && !!current, {
      what: current ? `“${current.name}”` : 'this workflow',
      allow: (to) => to === 'workflows' || to.startsWith('workflows/'),
    }),
  );

  // Tracks ONLY the workspace id: `load()` reads `current`, `types` and
  // `templates` before its first await, and tracking those re-fired the list
  // fetch ~4× per page entry as each one landed.
  $effect(() => {
    const id = ws.currentId;
    if (id) {
      untrack(() => {
        void load();
        // Populate the "Running" sidebar list on entry (also kept live by the
        // store's WS-event refresh).
        void ws.refreshActiveWorkflowRuns();
      });
    }
  });

  let destroyed = false;
  $effect(() => () => {
    destroyed = true;
  });

  /** Open a run from the "Running" sidebar list: ensure its workflow is open,
   *  then show the run (which the auto-update effects keep live). */
  async function openRunById(workflowId: string, runId: string): Promise<void> {
    requestedRunId = runId;
    try {
      if (current?.id !== workflowId) {
        if (!(await discardEditsOk()) || requestedRunId !== runId) return;
        let wf = workflows.find(w => w.id === workflowId);
        if (!wf) wf = await api.get<Workflow>(`/workflows/${workflowId}`);
        if (requestedRunId !== runId) return;
        open(wf);
        requestedRunId = runId;
      }
      const result = await workflowProgress(runId);
      if (requestedRunId !== runId || current?.id !== workflowId || destroyed) return;
      if (result.changed) mon.run = result.run;
      runsOpen = false;
    } catch (e) {toastError('Couldn’t open the run', e);}
  }

  // Never open onto an empty "build a workflow" pane when the workspace has
  // workflows: restore the last-opened one (or the first) once per workspace
  // load. Not on a phone, where the sidebar list is the first screen.
  let autoPickedFor = $state<string | null>(null);
  $effect(() => {
    const wsId = ws.currentId;
    if (!wsId || autoPickedFor === wsId || viewport.isPhone) return;
    if (current) {
      autoPickedFor = wsId;
      return;
    }
    // The list still holds the previous workspace's rows until load() lands.
    const mine = workflows.filter((w) => w.workspace_id === wsId);
    if (mine.length === 0) return;
    autoPickedFor = wsId;
    // A deep link (`#/workflows/<id>`) wins over the remembered pick.
    const linked = untrack(() => routeWorkflowId());
    const id = linked && mine.some((w) => w.id === linked) ? linked : initialSelection('workflows', mine, (w) => w.id);
    const wf = mine.find((w) => w.id === id);
    if (wf) untrack(() => open(wf));
  });

  /** The workflow id the URL names (`#/workflows/<id>`), or null. */
  function routeWorkflowId(): string | null {
    const [mod, id] = router.parts;
    return mod === 'workflows' && id ? id : null;
  }
  // The URL carries the open workflow so a reload / share / notification lands
  // on it; the last pick is remembered as the fallback.
  // (Nothing open yet on mount must not strip a deep link before it opens:
  // only a CLOSE — an id going away — clears the URL.)
  let routedId: string | null = null;
  $effect(() => {
    const id = current?.id ?? null;
    if (id) rememberSelection('workflows', id);
    const was = routedId;
    routedId = id;
    if (!id && !was) return;
    untrack(() => {
      if (router.module === 'workflows' && routeWorkflowId() !== id) router.replace(id ? `workflows/${id}` : 'workflows');
    });
  });
  // A route change while the page is up (a notification, Back/Forward, a
  // pasted link) opens that workflow — through the unsaved-edits guard; a
  // declined switch puts the URL back on the open workflow.
  $effect(() => {
    const linked = routeWorkflowId();
    // (Re-checked once the list lands, so a cold deep link opens on a phone
    // too, where the auto-pick above stays off.)
    if (!linked || wfLoading) return;
    untrack(() => {
      if (linked === current?.id) return;
      void (async () => {
        const wf = workflows.find((w) => w.id === linked);
        if (!wf || !(await discardEditsOk())) {
          if (current && router.module === 'workflows') router.replace(`workflows/${current.id}`);
          return;
        }
        if (routeWorkflowId() === linked) open(wf);
      })();
    });
  });

  // Layout: with no workflows (and nothing running) the list pane is hidden
  // and the main pane's first-run form owns the page. On a phone the list and
  // the editor are push-navigated (the list is the first screen; opening a
  // workflow replaces it and the header gets a back button).
  const listless = $derived(workflows.length === 0 && !current && ws.activeWorkflowRuns.length === 0);
  const showSide = $derived(!listless && !(viewport.isPhone && current));
  const showMain = $derived(listless || !viewport.isPhone || !!current);
  async function backToList(): Promise<void> {
    if (!(await discardEditsOk())) return;
    current = null;
      ++viewGeneration;
      ++versionsGeneration;
    graph = { nodes: [], edges: [] };
    selectedId = null;
    mon.run = null;
    dirty = false;
  }

  // The Node palette / Runs popovers hang off header buttons, but the header's
  // action row clips overflow — so they render at the top of the editor pane,
  // horizontally under their button (clamped into the pane).
  let mainEl = $state<HTMLElement | null>(null);
  let popRight = $state(8);
  // The control that opened a popover — Esc / outside-click closes it and
  // returns focus there (the Node button, or the ⋯ menu button for Runs).
  let popOpener: HTMLElement | null = null;
  function closePops(refocus: boolean): void {
    const opener = popOpener;
    paletteOpen = false;
    runsOpen = false;
    popOpener = null;
    if (refocus && opener?.isConnected) opener.focus();
  }
  $effect(() => {
    if (!paletteOpen && !runsOpen) return;
    const onKey = (e: KeyboardEvent): void => {
      if (e.key !== 'Escape' || e.defaultPrevented) return;
      e.preventDefault();
      e.stopPropagation();
      closePops(true);
    };
    const onDown = (e: PointerEvent): void => {
      const t = e.target;
      if (!(t instanceof Node)) return;
      if ((t as Element).closest?.('.wf-pop, .ctx-menu, [role="menu"]')) return;
      if (popOpener?.contains(t)) return;
      closePops(false);
    };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('pointerdown', onDown, true);
    // Keyboard users land on the first entry of what they just opened.
    queueMicrotask(() => document.querySelector<HTMLElement>('.wf-pop .pal-item, .wf-pop .run-item')?.focus());
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('pointerdown', onDown, true);
    };
  });
  function anchorPop(e: MouseEvent, width: number): void {
    popOpener = e.currentTarget instanceof HTMLElement ? e.currentTarget : null;
    const b = (e.currentTarget as HTMLElement | null)?.getBoundingClientRect();
    const m = mainEl?.getBoundingClientRect();
    // A button collapsed into the header's "⋯" menu has no box — pin right.
    if (!b || !m || b.width === 0) {
      popRight = 8;
      return;
    }
    // Offset from the pane's inline END (inset-inline-end), so RTL mirrors.
    const rtl = mainEl ? getComputedStyle(mainEl).direction === 'rtl' : false;
    const fromEnd = rtl ? b.left - m.left : m.right - b.right;
    popRight = Math.max(8, Math.min(fromEnd, m.width - width - 8));
  }

  /** A workflow row's ⋯ / right-click menu. */
  function rowMenu(e: MouseEvent, wf: Workflow): void {
    ctxMenu.show(e, [
      { label: 'Rename', icon: 'edit', action: () => startRename(wf) },
      { label: 'Duplicate', icon: 'copy', action: () => void duplicate(wf) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void del(wf) },
    ]);
  }

  /** The header's ⋯ menu: panel toggles (checked rows) and one-off tools. */
  function wfMenu(e: MouseEvent): void {
    const opener = e.currentTarget instanceof HTMLElement ? e.currentTarget : null;
    const items: MenuItem[] = [];
    if (viewport.isPhone) {
      items.push(
        { label: 'Add node…', icon: 'plus', action: () => { popRight = 8; popOpener = opener; paletteOpen = true; } },
        { label: 'Save', icon: 'check', disabled: !dirty || savingGraph, title: dirty ? undefined : 'No unsaved changes', action: () => void save() },
      );
    }
    items.push(
      { label: 'Runs', icon: 'clock', title: 'Past and live runs of this workflow', action: () => { popRight = 8; popOpener = opener; runsOpen = true; void loadRuns(); } },
      { separator: true },
      { label: 'Instructions', checked: instructionsOpen, title: 'Standing rules every step follows, by the letter', action: () => (instructionsOpen = !instructionsOpen) },
      { label: 'Triggers', checked: triggersOpen, title: 'Configure what starts this workflow', action: () => (triggersOpen = !triggersOpen) },
      { label: 'Versions', checked: versionsOpen, title: 'Version history', action: () => { versionsOpen = !versionsOpen; if (versionsOpen) void loadVersions(); } },
      ...(viewport.isDesktop
        ? [{
            label: 'Inspector on the side',
            checked: ui.wfDockSide,
            title: ui.wfDockSide ? 'Dock the node inspector to the bottom' : 'Dock the node inspector to a resizable side panel',
            action: () => ui.setWfDockSide(!ui.wfDockSide),
          } satisfies MenuItem]
        : []),
      { separator: true },
      {
        label: validating ? 'Checking…' : 'Validate',
        icon: 'shield',
        disabled: validating,
        title: 'Check the graph for problems before running',
        action: async () => { if (await validateGraph()) toasts.success('Preflight passed'); },
      },
      { label: 'Tidy', icon: 'grid', title: 'Tidy layout into rows', action: tidy },
    );
    if (selectedId) {
      items.push({ separator: true }, { label: 'Delete selected node', icon: 'trash', danger: true, action: removeSelected }); // ui-guards: allow — an unsaved canvas edit (Discard reverts it), no confirm
    }
    ctxMenu.show(e, items);
  }

  // The workspace `workflows` belongs to, and a token so a slow load for a
  // workspace we've since left can't land over the current one.
  let wfFor: string | null = null;
  let loadSeq = 0;
  async function load(): Promise<void> {
    const wsId = ws.currentId;
    const seq = ++loadSeq;
    // Never list another workspace's rows under this one — neither while this
    // one loads nor, silently, after its load fails.
    if (wfFor !== wsId) {
      workflows = [];
      wfError = null;
      wfFor = wsId;
      if (current && current.workspace_id !== wsId) {
        current = null;
        ++viewGeneration;
        ++versionsGeneration;
        // Another workspace's run must not stay open (its liveQuery polling).
        mon.run = null;
        runs = [];
        graph = { nodes: [], edges: [] };
      }
    }
    wfLoading = true;
    try {
      // The node catalog and templates are workspace-independent: fetched once,
      // in parallel with the list rather than as two sequential round trips.
      const needTypes = types.length === 0, needTemplates = templates.length === 0;
      const [nextTypes, nextTemplates, rows] = await Promise.all([
        needTypes ? api.get<NodeTypeSpec[]>('/workflows/node-types') : null,
        needTemplates ? api.get<WorkflowTemplate[]>('/workflows/templates') : null,
        api.get<Workflow[]>(`/workspaces/${wsId}/workflows`),
      ]);
      if (nextTypes && types.length === 0) types = nextTypes;
      if (nextTemplates && templates.length === 0) templates = nextTemplates;
      if (seq !== loadSeq) return;
      workflows = rows;
      wfError = null;
    } catch (e) {
      if (seq === loadSeq) wfError = loadErrorText(e);
    } finally {
      if (seq === loadSeq) wfLoading = false;
    }
  }

  async function fromTemplate(t: WorkflowTemplate): Promise<void> {
    try {
      const wf = await api.post<Workflow>(`/workspaces/${ws.currentId}/workflows/from-template`, {
        template_id: t.id,
      });
      workflows = [wf, ...workflows];
      open(wf);
      toasts.success(`Created “${wf.name}”`, 'Ready to run.');
    } catch (e) {
      toastError('Couldn’t create the workflow from the template', e);
    }
  }

  let viewGeneration = 0;
  function open(wf: Workflow): void {
    ++viewGeneration;
    ++versionsGeneration;
    versionsLoading = false;
    requestedRunId = null;
    validationIssues = [];
    current = wf;
    const g = structuredClone($state.snapshot(wf.graph)) as WorkflowGraph;
    graph = g && g.nodes ? g : { nodes: [], edges: [] };
    selectedId = null;
    selectedEdgeId = null;
    mon.run = null;
    runsOpen = false;
    versionsOpen = false;
    versions = [];
    versionsBefore = undefined;
    versionsHasMore = false;
    versionsError = null;
    // Never show the previous workflow's runs under this one while they load.
    runs = [];
    runsError = null;
    wfInstructions = wf.instructions ?? '';
    mon.resetFinalOutput();
    void loadRuns();
    dirty = false;
  }

  /** Opening another workflow (or restoring a version) replaces the canvas,
   *  so unsaved node/edge edits would vanish silently — ask first. */
  async function discardEditsOk(): Promise<boolean> {
    if ((!dirty && !instructionsDirty) || !current) return true;
    return confirmer.ask(`“${current.name}” has unsaved changes. Discard them?`, {
      title: 'Discard unsaved changes',
      danger: true, confirmLabel: 'Discard changes',
    });
  }
  /** Sidebar row click: re-clicking the open workflow keeps its unsaved edits
   *  (it used to reload the saved graph over them). */
  async function openGuarded(wf: Workflow): Promise<void> {
    if (current?.id === wf.id && (dirty || instructionsDirty)) return;
    if (!(await discardEditsOk())) return;
    open(wf);
  }

  async function generate(): Promise<void> {
    if (generating || prompt.trim() === '') return;
    generating = true;
    try {
      const wf = await api.post<Workflow>(`/workspaces/${ws.currentId}/workflows/generate`, {
        description: prompt.trim(),
      });
      workflows = [wf, ...workflows];
      open(wf);
      prompt = '';
      toasts.success('Workflow generated', 'Tweak it on the canvas, then run.');
    } catch (e) {
      toastError('Couldn’t generate the workflow', e);
    } finally {
      generating = false;
    }
  }

  async function createBlank(): Promise<void> {
    try {
      const wf = await api.post<Workflow>(`/workspaces/${ws.currentId}/workflows`, {
        name: 'Untitled workflow',
        graph: {
          nodes: [
            { id: 'trigger', kind: 'manual_trigger', name: 'Start', x: 60, y: 80, params: null },
          ],
          edges: [],
        },
      });
      workflows = [wf, ...workflows];
      open(wf);
    } catch (e) {
      toastError('Couldn’t create the workflow', e);
    }
  }

  /** A save updates its own row; a late response must never reopen an old editor. */
  function applySavedWorkflow(wf: Workflow): void {
    if (current?.id === wf.id && (wf.version ?? 0) >= (current.version ?? 0)) current = wf;
    workflows = workflows.map((w) => (w.id === wf.id && (wf.version ?? 0) >= (w.version ?? 0) ? wf : w));
  }

  async function save(): Promise<void> {
    if (!current || savingGraph) return;
    const workflowId = current.id;
    const submitted = JSON.stringify($state.snapshot(graph));
    savingGraph = true;
    try {
      const wf = await patchWorkflow(workflowId, { graph: JSON.parse(submitted) as WorkflowGraph });
      applySavedWorkflow(wf);
      // Editing stays available during Save; only the submitted graph is saved.
      if (current?.id === workflowId && JSON.stringify($state.snapshot(graph)) === submitted) dirty = false;
      toasts.success('Saved');
    } catch (e) {
      toastError('Couldn’t save the workflow', e);
    } finally {
      savingGraph = false;
    }
  }

  // Instructions save via the same PATCH endpoint as graph save, kept
  // separate so editing instructions never depends on (or clobbers) an
  // in-flight graph edit — an instructions-only PATCH bumps the workflow
  // version exactly like a graph-changing one.
  async function saveInstructions(): Promise<void> {
    if (!current) return;
    savingInstructions = true;
    try {
      const wf = await patchWorkflow(current.id, { instructions: wfInstructions });
      applySavedWorkflow(wf);
      toasts.success('Instructions saved');
    } catch (e) {
      toastError('Couldn’t save the instructions', e);
    } finally {
      savingInstructions = false;
    }
  }

  // Restart policy toggle: saved immediately (its own PATCH — a policy flip
  // must not ride on, or wait for, an unsaved instructions edit).
  async function saveOnRestart(resume: boolean): Promise<void> {
    if (!current) return;
    try {
      const wf = await patchWorkflow(current.id, {
        on_restart: resume ? 'resume' : 'fail',
      });
      applySavedWorkflow(wf);
    } catch (e) {
      toastError('Couldn’t save the restart policy', e);
    }
  }

  // Deleting cascades (runs, triggers, node cache all go with it), so it asks
  // first; when the open workflow goes, open its neighbour instead of leaving
  // an empty "Pick a workflow" pane.
  async function del(wf: Workflow): Promise<void> {
    // The server REFUSES (409) to delete a workflow with live runs, so the
    // confirm never promises to stop them silently: with runs going it offers
    // "Cancel N runs and delete", which cancels each, waits for them to
    // settle, then deletes (S17-302).
    let liveIds: string[] | null = null;
    const isLive = (r: { status: string }) => r.status === 'pending' || r.status === 'running';
    try {
      const rows = await api.get<typeof runs>(`/workflows/${wf.id}/runs?summary=true`);
      liveIds = rows.filter(isLive).map((r) => r.id);
    } catch {
      liveIds = null; // unknown — the confirm says so rather than claiming none
    }
    const active = liveIds?.length ?? 0;
    const runsNote =
      liveIds === null
        ? ' Otto couldn’t check for running runs — if one is still going, the delete is refused until it’s canceled.'
        : active > 0
          ? ` ${active === 1 ? '1 run is' : `${active} runs are`} still going — ${active === 1 ? 'it is' : 'they are'} canceled first.`
          : '';
    const ok = await confirmer.ask(
      `Delete “${wf.name}”?${runsNote} Its run history and triggers are deleted with it. This can’t be undone.`,
      {
        title: 'Delete workflow',
        confirmLabel: active > 0 ? `Cancel ${active === 1 ? 'run' : `${active} runs`} and delete` : 'Delete workflow',
        danger: true,
      },
    );
    if (!ok) return;
    if (liveIds && liveIds.length > 0) {
      try {
        await Promise.all(liveIds.map((id) => api.post(`/workflow-runs/${encodeURIComponent(id)}/cancel`, {})));
      } catch (e) {
        toastError('Couldn’t cancel the running runs — the workflow was not deleted', e);
        return;
      }
      // A cancel settles at the run's next boundary: wait (bounded) for the
      // rows to leave pending/running before the delete.
      let settled = false;
      for (let i = 0; i < 20 && !destroyed; i++) {
        await new Promise((r) => setTimeout(r, 750));
        try {
          const rows = await api.get<typeof runs>(`/workflows/${wf.id}/runs?summary=true`);
          if (!rows.some(isLive)) {settled = true; break;}
        } catch { /* keep waiting */ }
      }
      if (!settled) {
        toasts.error('Runs are still stopping', 'The workflow was not deleted — try again in a moment.');
        return;
      }
    }
    try {
      await api.del(`/workflows/${wf.id}`);
      const idx = workflows.findIndex((w) => w.id === wf.id);
      workflows = workflows.filter((w) => w.id !== wf.id);
      if (current?.id === wf.id) {
        const next = workflows[Math.min(Math.max(idx, 0), workflows.length - 1)] ?? null;
        if (next) open(next);
        else {
          current = null;
          ++viewGeneration;
          ++versionsGeneration;
          // The deleted workflow's run view (and its live poll) goes too.
          mon.run = null;
          runs = [];
          graph = { nodes: [], edges: [] };
        }
      }
      toasts.success('Workflow deleted', wf.name);
    } catch (e) {
      toastError('Couldn’t delete the workflow', e);
    }
  }

  // Duplicate an existing WORKFLOW (a full copy of its graph), so several
  // independently-named/triggerable workflows can share one starting point
  // without going back through a template. Fetch the source fresh so we copy the
  // complete, current graph regardless of what the list row carries.
  async function duplicate(wf: Workflow): Promise<void> {
    try {
      const src = await api.get<Workflow>(`/workflows/${wf.id}`);
      const copy = await api.post<Workflow>(`/workspaces/${ws.currentId}/workflows`, {
        name: `${src.name} (copy)`,
        description: src.description,
        graph: src.graph,
      });
      workflows = [...workflows, copy];
      toasts.success(`Duplicated → “${copy.name}”`, 'Rename it to trigger it independently.');
      void open(copy);
    } catch (e) {
      toastError('Couldn’t duplicate the workflow', e);
    }
  }

  // ── rename (sidebar + header) ─────────────────────────────────────────────
  // The same template can seed many workflows; each needs its OWN name so it can
  // be triggered independently from Slack/Telegram (triggers match by name).
  let renamingId = $state<string | null>(null);
  let renameValue = $state('');

  // Where the rename was started: the page header's title, or the sidebar row.
  // Only that one surface shows the input (two autofocused inputs bound to the
  // same value would steal focus from each other and commit on blur).
  let renameInBar = $state(false);
  function startRename(wf: Workflow, inBar = false): void {
    renamingId = wf.id;
    renameValue = wf.name;
    renameInBar = inBar;
  }
  function cancelRename(): void {
    renamingId = null;
    renameValue = '';
    renameInBar = false;
  }
  async function commitRename(wf: Workflow): Promise<void> {
    const name = renameValue.trim();
    if (!name || name === wf.name) {
      cancelRename();
      return;
    }
    try {
      const updated = await patchWorkflow(wf.id, { name });
      workflows = workflows.map((w) => (w.id === wf.id ? { ...w, name: updated.name } : w));
      if (current?.id === wf.id) current = { ...current, name: updated.name };
      toasts.success(`Renamed to “${updated.name}”`);
    } catch (e) {
      toastError('Couldn’t rename the workflow', e);
    } finally {
      cancelRename();
    }
  }

  function addNode(t: NodeTypeSpec): void {
    paletteOpen = false;
    const id = `${t.kind}-${Math.random().toString(36).slice(2, 7)}`;
    const x = 80 + graph.nodes.length * 30;
    const y = 80 + graph.nodes.length * 24;
    graph.nodes = [...graph.nodes, { id, kind: t.kind, name: t.label, x, y, params: {} }];
    selectedId = id;
    dirty = true;
  }

  function removeSelected(): void {
    if (!selectedId) return;
    graph.nodes = graph.nodes.filter((n) => n.id !== selectedId);
    graph.edges = graph.edges.filter((e) => e.source !== selectedId && e.target !== selectedId);
    selectedId = null;
    dirty = true;
  }

  function removeSelectedEdge(): void {
    if (!selectedEdgeId) return;
    graph.edges = graph.edges.filter((e) => e.id !== selectedEdgeId);
    selectedEdgeId = null;
    dirty = true;
  }

  let validationIssues = $state<import('../../lib/api/types').WorkflowValidationIssue[]>([]);
  let validating = $state(false);
  async function validateGraph(): Promise<boolean> {
    if (!current) return false;
    const id = current.id;
    validating = true;
    try {
      const result = await api.post<{ valid: boolean; issues: import('../../lib/api/types').WorkflowValidationIssue[] }>(`/workflows/${id}/validate`, { graph });
      if (current?.id !== id) return false;
      validationIssues = result.issues;
      return result.valid;
    } catch (e) { toastError('Couldn’t validate the workflow', e); return false; }
    finally { validating = false; }
  }

  async function execRun(body: RunWorkflowReq): Promise<void> {
    const r = await startRun(body);
    if (r) await followRun(r);
  }

  /** Validate, save, POST the run and show it. Null when it didn't start (the
   *  reason is on the page: validation issues inline, a failed POST toasted).
   *  `running` covers only the POST; {@link followRun} just reports the end. */
  async function startRun(body: RunWorkflowReq): Promise<WorkflowRun | null> {
    if (!current || running) return null;
    const workflowId = current.id;
    const workspaceId = ws.currentId;
    const generation = viewGeneration;
    const ownsView = () => !destroyed && current?.id === workflowId && ws.currentId === workspaceId && viewGeneration === generation;
    // Reserve before preflight, so two clicks cannot both pass validation.
    running = true;
    try {
      if (!await validateGraph() || !ownsView()) return null;
      if (dirty) await save();
      if (!ownsView() || dirty) return null;
      const r = await api.post<WorkflowRun>(`/workflows/${workflowId}/run`, body);
      if (ownsView()) {
        requestedRunId = r.id;
        mon.run = r;
        // The engine starts the run before this POST answers, so its first
        // workflow_run_updated events can reach the page while `run` is still
        // unset — the fast path drops them, and the view would sit on this
        // creation snapshot ("Queued") until the run's NEXT event, possibly a
        // whole step later. Catch up once (rev-guarded, single-flight).
        if (r.status === 'pending' || r.status === 'running') void mon.refetchRun(r.id);
      }
      return r;
    } catch (e) {
      toastError('Couldn’t start the run', e);
      return null;
    } finally {
      running = false;
    }
  }

  async function followRun(r: WorkflowRun): Promise<void> {
    try {
      const done = await mon.waitRunTerminal(r.id);
      if (destroyed) return;
      if (done.status === 'success') toasts.success('Run complete');
      else if (done.status === 'canceled') toasts.info('Run canceled');
      else toasts.error('Run finished with errors', done.error ?? '');
      void loadRuns();
    } catch (e) {
      toastError('Couldn’t follow the run', e);
    }
  }

  /** ⌘K "New workflow…": the describe-it box (on a phone, back on the list first). */
  async function focusGenerator(): Promise<void> {
    if (viewport.isPhone && current) await backToList();
    await tick();
    document.getElementById('wf-prompt')?.focus();
  }

  // ⌘K: the page's verbs (run verbs only while a workflow is open).
  $effect(() => {
    const open = !!current;
    const live = runActive;
    return registry.register('workflows', [
      { id: 'workflows.new', title: 'New workflow…', group: 'Workflows', keywords: 'create describe generate automation', run: () => void focusGenerator() },
      { id: 'workflows.blank', title: 'New blank workflow', group: 'Workflows', keywords: 'create empty canvas', run: () => void createBlank() },
      ...(open && !live
        ? [{ id: 'workflows.run', title: 'Run workflow…', group: 'Workflows', keywords: 'start execute input', run: () => { runForm.open = true; } }]
        : []),
      ...(open && live
        ? [{ id: 'workflows.stop', title: 'Stop run…', group: 'Workflows', keywords: 'cancel halt', run: () => void stop() }]
        : []),
    ]);
  });

  // Agent UI control (lib/uiCommands/workflows.ts): the agent opens a
  // workflow / run and starts runs through THIS editor, so the user watches
  // the same canvas + run inspector their own clicks would show.
  $effect(() =>
    workflowsPagePort.bind({
      list: () => workflows,
      loading: () => wfLoading,
      currentId: () => current?.id ?? null,
      async open(id: string): Promise<boolean> {
        if (current?.id === id) return true;
        if (!(await discardEditsOk())) return false;
        const wf = workflows.find((w) => w.id === id) ?? (await api.get<Workflow>(`/workflows/${id}`));
        open(wf);
        return true;
      },
      openRun: (workflowId: string, runId: string) => openRunById(workflowId, runId),
      async start(body: RunWorkflowReq): Promise<WorkflowRun | null> {
        const r = await startRun(body);
        if (r) void followRun(r);
        return r;
      },
      currentRun: () => run,
    }),
  );

  // Every node kind in the graph, including the inner steps of `loop` nodes.
  function collectKinds(): Set<string> {
    const s = new Set<string>();
    for (const n of graph.nodes) {
      s.add(n.kind);
      const steps = (n.params as { steps?: { kind?: string }[] } | null)?.steps;
      if (n.kind === 'loop' && Array.isArray(steps)) {
        for (const st of steps) if (st?.kind) s.add(st.kind);
      }
    }
    return s;
  }

  // Run… panel state (prompt, JSON input, review mode): RunInputForm.
  const runForm = new RunInputForm(collectKinds);

  async function confirmRun(): Promise<void> {
    const body = runForm.request();
    if (!body) return; // invalid JSON; shown under the field
    runForm.open = false;
    await execRun(body);
  }

  const runFrom = (nodeId: string, only: boolean): Promise<void> => {
    const body = runForm.request({ start_node: nodeId, only_node: only });
    return body ? execRun(body) : Promise.resolve();
  };

  // Re-flow the graph into a few readable rows (topological order, snaking
  // left→right then right→left) so a long chain isn't one wide line.
  function tidy(): void {
    if (!current) return;
    const nodes = graph.nodes;
    if (!nodes.length) return;
    const indeg = new Map(nodes.map((n) => [n.id, 0]));
    const adj = new Map(nodes.map((n) => [n.id, [] as string[]]));
    for (const e of graph.edges) {
      if (indeg.has(e.target) && indeg.has(e.source)) {
        indeg.set(e.target, (indeg.get(e.target) ?? 0) + 1);
        adj.get(e.source)!.push(e.target);
      }
    }
    const queue = nodes.filter((n) => (indeg.get(n.id) ?? 0) === 0).map((n) => n.id);
    const order: string[] = [];
    const seen = new Set<string>();
    while (queue.length) {
      const id = queue.shift()!;
      if (seen.has(id)) continue;
      seen.add(id);
      order.push(id);
      for (const s of adj.get(id) ?? []) {
        indeg.set(s, (indeg.get(s) ?? 1) - 1);
        if ((indeg.get(s) ?? 0) <= 0) queue.push(s);
      }
    }
    for (const n of nodes) if (!seen.has(n.id)) order.push(n.id);
    const PER = 4;
    const COLW = 260;
    const ROWH = 230;
    const byId = new Map(nodes.map((n) => [n.id, n]));
    order.forEach((id, i) => {
      const row = Math.floor(i / PER);
      let col = i % PER;
      if (row % 2 === 1) col = PER - 1 - col; // snake so rows read end→start
      const n = byId.get(id);
      if (n) {
        n.x = 40 + col * COLW;
        n.y = 30 + row * ROWH;
      }
    });
    graph = { ...graph, nodes: [...nodes] };
    dirty = true;
    void save();
  }

  // Stopping can't be undone (the run halts after its current step and must
  // be started again), so it asks first — the same Stop wording as the header's
  // "Stop run…" and a Stop in Goal Loops; "Keep running" is the safe way out.
  async function stop(): Promise<void> {
    if (!run) return;
    const runId = run.id;
    const name = current?.name ?? 'this workflow';
    const ok = await confirmer.ask(
      `Stop this run of “${name}”? It finishes the current step, then halts. Completed steps and their output are kept; to continue you start a new run.`,
      { title: 'Stop run', confirmLabel: 'Stop run', cancelLabel: 'Keep running', danger: true },
    );
    if (!ok) return;
    try {
      await api.post(`/workflow-runs/${runId}/cancel`, {});
      toasts.info('Stopping the run…', 'Finishes the current step, then halts.');
    } catch (e) {
      toastError('Couldn’t stop the run', e);
    }
  }

  // Whether the selected run is still active (so a Cancel affordance makes sense
  // even when this editor didn't initiate it — e.g. opened from the Running list).
  const runActive = $derived(run?.status === 'running' || run?.status === 'pending');

  // Run-detail (inspector) resize: drag the top grip to grow the height cap; the
  // maximize toggle "zooms" the whole run to ~85vh. (R6)
  let runDetailMax = $state(false);
  function inspMaxPx(): number {
    const vh = typeof window !== 'undefined' ? window.innerHeight : 900;
    return runDetailMax ? Math.round(vh * 0.85) : ui.runDetailHeight;
  }
  // Whether the node inspector is docked as a right-side column (vs the bottom
  // strip). Only meaningful when something is selected/running.
  // Side dock is a desktop column. Phone and tablet keep the bottom inspector
  // so a 280–400px panel cannot cover a narrow viewport. The preference stays.
  const sideDock = $derived(ui.wfDockSide && viewport.isDesktop);

  const pxText = (v: number): string => `${Math.round(v)} pixels wide`;
  function resetInsp(): void {
    if (sideDock) ui.setWfInspSideWidth(400);
    else ui.setRunDetailHeight(300);
  }
  function startInspResize(e: MouseEvent): void {
    e.preventDefault();
    runDetailMax = false;
    if (sideDock) {
      // Side dock: drag the inspector's LEFT edge — leftward widens it.
      const startX = e.clientX;
      const startW = ui.wfInspSideWidth;
      startMouseDrag(e, { onMove: (ev) => ui.setWfInspSideWidth(startW + (startX - ev.clientX)) });
      return;
    }
    const startY = e.clientY;
    const startH = ui.runDetailHeight;
    startMouseDrag(e, { cursor: 'row-resize', onMove: (ev) => ui.setRunDetailHeight(startH + (startY - ev.clientY)) });
  }

  // Short run id + per-workflow ordinal, so two concurrent runs of the SAME
  // workflow are distinguishable in the Running/Runs lists (item 8).
  function shortRunId(id: string): string {
    return id.length > 6 ? id.slice(-6) : id;
  }

  // The daemon caps parallel workflow runs; a `pending` run is one PARKED in
  // its FIFO queue (it starts the moment a slot frees). Say so — "pending"
  // read as "about to start any second" and made the queue look stuck. The
  // shared run vocabulary (lib/status.ts) already maps pending → "Queued".
  function runStatusLabel(status: string): string {
    return runStatus(status).label;
  }
  /** Normalised run/step key for the status dot (`succeeded`, `failed`, …). */
  function dotKey(status: string): string {
    return runStatus(status).key;
  }
  const activeRunOrdinals = $derived.by(() => {
    const counts: Record<string, number> = {};
    const map: Record<string, number> = {};
    const sorted = [...ws.activeWorkflowRuns].sort((a, b) => a.started_at.localeCompare(b.started_at));
    for (const r of sorted) {
      counts[r.workflow_id] = (counts[r.workflow_id] ?? 0) + 1;
      map[r.run_id] = counts[r.workflow_id];
    }
    return map;
  });
  const activeWfRunCounts = $derived.by(() => {
    const c: Record<string, number> = {};
    for (const r of ws.activeWorkflowRuns) c[r.workflow_id] = (c[r.workflow_id] ?? 0) + 1;
    return c;
  });

  // Open a step's session INLINE in the WF page's Agents tab (never navigate to
  // the global Agents panel). Ensures the sidebar is open + on the Agents tab,
  // then focuses the session (reset-then-set so re-clicking the same id retriggers).
  let agentsFocusSid = $state<string | null>(null);
  function openSessionInline(sid: string): void {
    if (!ui.wfCtxOpen) ui.toggleWfCtx();
    ui.setWfCtxTab('agents');
    agentsFocusSid = null;
    queueMicrotask(() => (agentsFocusSid = sid));
  }

  // Per-run default tab: a running/pending run opens on Agents (immediate
  // visibility of the live sessions), a finished run opens on Files. Applied
  // once per run id so a later manual tab switch always wins.
  let tabDefaultedRunId: string | null = null;
  $effect(() => {
    const rid = run?.id;
    if (rid && rid !== tabDefaultedRunId) {
      tabDefaultedRunId = rid;
      ui.setWfCtxTab(runActive ? 'agents' : 'files');
    }
  });

  // Runs popover load state: a failed load says so (with Retry) instead of
  // showing a stale or empty "No runs yet" list.
  let runsLoading = $state(false);
  let runsError = $state<string | null>(null);
  async function loadRuns(): Promise<void> {
    if (!current) return;
    const workflowId = current.id;
    runsLoading = true;
    try {
      const rows = await api.get<typeof runs>(`/workflows/${workflowId}/runs?summary=true`);
      if (current?.id === workflowId) {
        runs = rows;
        runsError = null;
      }
    } catch (e) {
      if (current?.id === workflowId) runsError = loadErrorText(e);
    } finally {
      if (current?.id === workflowId) runsLoading = false;
    }
  }

  function nodeName(id: string): string {
    const n = graph.nodes.find((x) => x.id === id);
    return n?.name || n?.kind || id;
  }
  // The node forms' picker lists (repos, swarms, connections, …), fetched on
  // demand once per workspace / key for the life of the page.
  const pickers = new InspectorOptions();
  const fmtMs = fmtStepMs;

  function onParam(field: string, value: unknown): void {
    if (!selectedNode) return;
    selectedNode.params = withParam(selectedNode.params, field, value);
    graph = graph;
    dirty = true;
  }

  // JSON param field zoom (R10): open a big modal editor for the cramped node-form
  // JSON textareas (Steps / Merge JSON / Body). Edits write straight back to the param.
  let jsonZoom = $state<{ field: string; label: string } | null>(null);
  function onParamJson(field: string, raw: string): void {
    if (selectedNode) onParam(field, jsonValue(raw));
  }

  // --- Per-node retry policy (writes node.retry, not params) ----------------
  function retryNum(field: 'max_attempts' | 'backoff_ms', def: number): number {
    const r = effectiveRetry(selectedNode);
    const v = r ? r[field] : undefined;
    return typeof v === 'number' ? v : def;
  }
  function onRetry(field: 'max_attempts' | 'backoff_ms', value: number): void {
    if (!selectedNode) return;
    selectedNode.retry = updateRetry(selectedNode, field, value);
    graph = graph;
    dirty = true;
  }

  // --- Edge condition editing ----------------------------------------------
  let selectedEdgeId = $state<string | null>(null);
  const selectedEdge = $derived(graph.edges.find((e) => e.id === selectedEdgeId) ?? null);
  // The inspector is shown when a run, node, or edge is active (drives side-dock).
  const inspShown = $derived(!!(run || selectedNode || selectedEdge));
  // When docked to the side it stays open as a persistent right column (like the
  // agents Right panel) even with nothing selected — pressing Dock has an
  // immediate, visible effect and shows a "pick a node" placeholder. In bottom
  // mode the strip only appears when there's something to show.
  const inspOpen = $derived(inspShown || sideDock);

  // Close the docked panel: clear any selection and pop the dock back to the
  // bottom strip (so the × on the side panel actually dismisses it). The Dock
  // toolbar button re-opens it. A live run still surfaces in the bottom strip.
  function closeInspector(): void {
    selectedId = null;
    selectedEdgeId = null;
    if (sideDock) ui.setWfDockSide(false);
  }

  function onEdgeCondition(raw: string): void {
    if (!selectedEdge) return;
    const e = selectedEdge;
    const cond = raw.trim();
    graph.edges = graph.edges.map((x) => (x.id === e.id ? { ...x, condition: cond || null } : x));
    dirty = true;
  }

  // --- Version history ------------------------------------------------------
  let versionsGeneration = 0;
  let versionsOpen = $state(false);
  const VERSION_PAGE_SIZE = 50;
  let versions = $state<WorkflowVersionSummary[]>([]);
  let versionsBefore = $state<number | undefined>(undefined);
  let versionsHasMore = $state(false);
  let versionsLoading = $state(false);
  let versionsError = $state<string | null>(null);

  async function loadVersions(beforeVersion?: number): Promise<void> {
    if (!current) return;
    const workflowId = current.id;
    const generation = ++versionsGeneration;
    const view = viewGeneration;
    const ownsView = () => !destroyed && current?.id === workflowId && generation === versionsGeneration && view === viewGeneration;
    versionsBefore = beforeVersion;
    versionsLoading = true;
    try {
      const rows = await listWorkflowVersions(workflowId, {limit: VERSION_PAGE_SIZE, beforeVersion});
      if (!ownsView()) return;
      // Keep one window in the drawer, regardless of how far back we browse.
      versions = rows.slice(0, VERSION_PAGE_SIZE);
      versionsHasMore = rows.length >= VERSION_PAGE_SIZE;
      versionsError = null;
    } catch (e) {
      if (ownsView()) versionsError = loadErrorText(e);
    } finally {
      if (ownsView()) versionsLoading = false;
    }
  }

  async function restoreVersion(v: WorkflowVersionSummary): Promise<void> {
    if (!current || v.workflow_id !== current.id) return;
    const workflowId = v.workflow_id;
    const generation = viewGeneration;
    const ownsView = () => !destroyed && current?.id === workflowId && viewGeneration === generation;
    if (!(await discardEditsOk()) || !ownsView()) return;
    try {
      const wf = await restoreWorkflowVersion(workflowId, v.version);
      workflows = workflows.map((w) => (w.id === wf.id ? wf : w));
      if (ownsView()) { open(wf); await loadVersions(); }
      toasts.success(`Restored v${v.version}`);
    } catch (e) {
      toastError('Couldn’t restore the version', e);
    }
  }

</script>

<!-- The "new workflow" form: describe it (an agent wires the graph), start
     blank, or start from a template. Lives at the top of the list pane; with no
     workflows yet the list pane is hidden and this form IS the page's empty
     state (`page`), where Generate is the one primary action. -->
{#snippet generator(mode: 'side' | 'page')}
  <div class="gen" class:gen-page={mode === 'page'}>
    <label for="wf-prompt">Describe the flow</label>
    <textarea dir="auto"
      id="wf-prompt"
      bind:value={prompt}
      rows={mode === 'page' ? 4 : 3}
      placeholder="e.g. Ask an agent to summarize the repo, then POST the summary to our webhook."
      onkeydown={(e) => {
        if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) generate();
      }}
    ></textarea>
    {#if mode === 'page'}
      <div class="gen-actions">
        <button class="btn ghost" onclick={createBlank}><Icon name="plus" size={13} /> Start blank</button>
        <span class="grow"></span>
        <button
          class="btn primary"
          disabled={generating || prompt.trim() === ''}
          title={prompt.trim() === '' ? 'Describe the flow first' : 'Generate the workflow (⌘↵)'}
          onclick={generate}
        >
          {#if generating}<span class="spinner" style="--spinner-size: 11px" aria-hidden="true"></span> Building…{:else}<Icon name="zap" size={13} /> Generate workflow{/if}
        </button>
      </div>
      {#if templates.length > 0}
        <div class="tpl-start">
          <span class="tpl-start-h">Or start from a template</span>
          <div class="tpl-grid">
            {#each templates as t (t.id)}
              <button class="tpl tpl-card" onclick={() => void fromTemplate(t)} title={t.description}>
                <span class="tpl-ic"><Icon name={asIcon(t.icon, 'box')} size={14} /></span>
                <span class="tpl-body">
                  <span class="tpl-name">{t.name}</span>
                  {#if t.description}<span class="tpl-sub">{t.description}</span>{/if}
                </span>
              </button>
            {/each}
          </div>
        </div>
      {/if}
    {:else}
      <!-- Secondary here: with a workflow open, the header's Run… is the
           view's one primary action. -->
      <button
        class="btn full"
        disabled={generating || prompt.trim() === ''}
        title={prompt.trim() === '' ? 'Describe the flow first' : 'Generate the workflow (⌘↵)'}
        onclick={generate}
      >
        {#if generating}<span class="spinner" style="--spinner-size: 11px" aria-hidden="true"></span> Building…{:else}<Icon name="zap" size={13} /> Generate workflow{/if}
      </button>
      <button class="btn ghost full" onclick={createBlank}>
        <Icon name="plus" size={13} /> Start blank
      </button>
      {#if templates.length > 0}
        <!-- Templates collapsed into a dropdown (was an always-open list) so the
             sidebar room goes to the Workflows + Running lists. -->
        <div class="tpl-menu">
          <button
            class="btn ghost full tpl-toggle"
            aria-expanded={templatesOpen}
            onclick={() => (templatesOpen = !templatesOpen)}
          >
            <Icon name="grid" size={13} /> Templates
            <span class="grow"></span>
            <Icon name={templatesOpen ? 'chevronUp' : 'chevronDown'} size={12} />
          </button>
          {#if templatesOpen}
            <div class="tpl-pop">
              {#each templates as t (t.id)}
                <button
                  class="tpl"
                  onclick={() => {
                    void fromTemplate(t);
                    templatesOpen = false;
                  }}
                  title={t.description}
                >
                  <span class="tpl-ic"><Icon name={asIcon(t.icon, 'box')} size={14} /></span>
                  <span class="tpl-body">
                    <span class="tpl-name">{t.name}</span>
                    {#if t.description}<span class="tpl-sub">{t.description}</span>{/if}
                  </span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    {/if}
  </div>
{/snippet}

<div class="wf-root">
<PageHeader class="wf-bar" title={current?.name ?? 'Workflows'}>
  {#snippet leading()}
    {#if viewport.isPhone && current}
      <button class="icon-btn" onclick={() => void backToList()} aria-label="Back to workflows" title="Back to workflows">
        <Icon name="chevronLeft" size={16} />
      </button>
    {/if}
  {/snippet}
  {#snippet titleContent()}
    {#if current && renamingId === current.id && renameInBar}
      <input dir="auto"
        class="wf-title-edit"
        bind:value={renameValue}
        use:focusOnMount
        aria-label="Workflow name"
        onkeydown={(e) => {
          if (e.key === 'Enter') commitRename(current!);
          else if (e.key === 'Escape') cancelRename();
        }}
        onblur={() => commitRename(current!)}
      />
    {:else}
      {current?.name ?? 'Workflows'}
    {/if}
  {/snippet}
  {#snippet badge()}
    {#if current && !(renamingId === current.id && renameInBar)}
      <button class="title-edit" title="Rename workflow" aria-label="Rename workflow" onclick={() => current && startRename(current, true)}>
        <Icon name="edit" size={13} />
      </button>
    {/if}
    {#if current && dirty}<Badge tone="accent" label="Unsaved" title="The canvas has changes that aren’t saved yet" />{/if}
  {/snippet}
  {#snippet actions()}
    <AutomateGuideButton current="workflows" />
    {#if current}
      <!-- Anatomy: [secondary…][primary][⋯]. Node + Save stay in the bar with the
           primary Run…; Runs, the panel toggles and the one-off tools live in
           the ⋯ menu (wfMenu), so the row stays at ≤ 5 controls. On a phone
           Node/Save join the menu too, so there is never a second,
           auto-generated ⋯ next to ours. The ⋯ collapses first. -->
      {#if !viewport.isPhone}
        <button class="btn small" data-overflow="1" data-icon="plus" aria-expanded={paletteOpen} aria-haspopup="dialog" onclick={(e) => { anchorPop(e, 230); paletteOpen = !paletteOpen; }}>
          <Icon name="plus" size={12} /> Node
        </button>
        <button class="btn small" data-overflow="2" data-icon="check" disabled={!dirty || savingGraph} title={dirty ? 'Save the canvas changes' : 'No unsaved changes'} onclick={save}>Save</button>
      {/if}
      <!-- Context/Agents panel toggle: the sidebar's ONLY toggle when collapsed
           (no second full-height rail beside the app shell's right rail). -->
      {#if viewport.isDesktop && run && run.context_dir}
        <button
          class="icon-btn wf-tog"
          data-icon="panel"
          class:on={ui.wfCtxOpen}
          aria-pressed={ui.wfCtxOpen}
          onclick={() => ui.toggleWfCtx()}
          title={ui.wfCtxOpen ? 'Hide the context files & agents panel' : 'Show the context files & agents panel'}
          aria-label="Context panel"
          data-label="Context panel"
          data-testid="ctx-sidebar-toggle"
        >
          <Icon name="panel" size={14} />
        </button>
      {/if}
      {#if runActive}
        <!-- While the VIEWED run is live its Stop takes the primary's place (same
             verb as the confirm dialog's "Stop run"). -->
        <button class="btn small danger" data-keep onclick={stop} title="Stop this run (finishes the current step, then halts)"><Icon name="stop" size={12} /> Stop run…</button>
      {:else}
        <button
          class="btn small"
          class:primary={!runForm.open}
          class:active={runForm.open}
          aria-pressed={runForm.open}
          onclick={() => (runForm.open = !runForm.open)}
          title="Run — set the input (repo_id / story_id / goals / msg) the trigger emits"
          aria-expanded={runForm.open}
        >
          <Icon name="play" size={12} /> Run…
        </button>
      {/if}
      <button class="icon-btn" data-overflow="-10" data-icon="more" data-label="More actions…" aria-haspopup="menu" aria-label="More actions" title="More actions" onclick={wfMenu}>
        <Icon name="more" size={14} />
      </button>
    {/if}
  {/snippet}
</PageHeader>
<PageBody fill padded={false}>
<div class="wf">
  {#if showSide}
  <aside class="side" class:phone={viewport.isPhone} style={viewport.isPhone ? '' : `width:${listW}px`}>
    {@render generator('side')}

    {#if ws.activeWorkflowRuns.length > 0}
      <div class="running" data-testid="running-workflows">
        <div class="list-h">
          Running
          <span class="run-count">{ws.activeWorkflowRuns.length}</span>
        </div>
        {#each ws.activeWorkflowRuns as r (r.run_id)}
          <button
            class="run-row"
            class:active={run?.id === r.run_id}
            onclick={() => openRunById(r.workflow_id, r.run_id)}
            title={`${r.workflow_name} — ${runStatusLabel(r.status)}`}
          >
            <span class="dot {dotKey(r.status)}" aria-hidden="true"></span>
            <span class="sr-only">{runStatusLabel(r.status)}:</span>
            <span class="run-name">{r.workflow_name}</span>
            {#if activeWfRunCounts[r.workflow_id] > 1}
              <span class="run-ord" title={`run #${activeRunOrdinals[r.run_id]} of this workflow`}>#{activeRunOrdinals[r.run_id]}</span>
            {/if}
            {#if r.waiting_approval}
              <span class="run-badge" role="img" title="Waiting for your approval" aria-label="Waiting for your approval"><Icon name="userCheck" size={12} /></span>
            {/if}
            <code class="run-id" title={r.run_id}>{shortRunId(r.run_id)}</code>
            <span class="run-prog" title={`${r.nodes_done} of ${r.nodes_total} steps done`}>{r.nodes_done}/{r.nodes_total}</span>
            <!-- Ticks with the shared clock (a one-shot "5m ago" froze at load). -->
            <span class="run-when"><RelTime iso={r.started_at} fallback="" /></span>
          </button>
        {/each}
      </div>
    {/if}

    <div class="list">
      <div class="list-h">Workflows</div>
      {#each workflows as wf (wf.id)}
        <div class="wf-row" class:active={current?.id === wf.id} data-testid={`wf-row-${wf.id}`}>
          {#if renamingId === wf.id && !renameInBar}
            <input dir="auto"
              class="row-rename"
              data-testid="wf-rename-input"
              bind:value={renameValue}
              use:focusOnMount
              aria-label="Workflow name"
              onkeydown={(e) => {
                if (e.key === 'Enter') commitRename(wf);
                else if (e.key === 'Escape') cancelRename();
              }}
              onblur={() => commitRename(wf)}
            />
          {:else}
            <button use:rowMenuKeys class="row-main" title={wf.name} onclick={() => void openGuarded(wf)} oncontextmenu={(e) => { e.preventDefault(); rowMenu(e, wf); }}>
              <Icon name="split" size={13} />
              <span class="row-name">{wf.name}</span>
            </button>
            <!-- One ⋯ per row (Rename / Duplicate / Delete…), like Scheduled Tasks;
                 right-click the row opens the same menu. -->
            <button class="row-edit" title="More actions" aria-label="More actions for “{wf.name}”" aria-haspopup="menu" data-testid="wf-row-more" onclick={(e) => rowMenu(e, wf)}>
              <Icon name="more" size={13} />
            </button>
          {/if}
        </div>
      {/each}
      {#if workflows.length > 0 && wfError}
        <!-- Rows from the last good load, flagged as such. -->
        <LoadState what="workflows" variant="compact" error={wfError} onretry={() => void load()} />
      {:else if workflows.length === 0 && (!wfError || current)}
        <!-- A failed load is said ONCE: the main pane owns it (with Retry)
             unless a workflow is open there, then the rail does. -->
        <LoadState what="workflows" variant="compact" loading={wfLoading} error={wfError} empty onretry={() => void load()}>
          {#snippet emptyView()}<EmptyState icon="split" title="No workflows yet" body="Describe one above, or start from a template." />{/snippet}
        </LoadState>
      {/if}
    </div>
  </aside>
  {#if !viewport.isPhone}
    <PaneDivider bind:width={listW} storageKey={LIST_W_KEY} label="Resize the workflows list" />
  {/if}
  {/if}

  {#if showMain}
  <main
    bind:this={mainEl}
    class="main"
    class:side-dock={sideDock}
    style={sideDock
      ? `padding-inline-end:${ui.wfInspSideWidth}px;--wf-insp-w:${ui.wfInspSideWidth}px`
      : ''}
  >
    {#if current}
      <!-- Popovers for the header's Node / Runs buttons (see anchorPop). -->
      {#if paletteOpen}
        <div class="palette wf-pop" role="dialog" aria-label="Add a node" style:inset-inline-end="{popRight}px">
          {#each types as t (t.kind)}
            <button class="pal-item" onclick={() => addNode(t)}>
              <span class="pal-ic"><Icon name={asIcon(t.icon, 'box')} size={12} /></span>
              <span class="pal-body">
                <span class="pal-name">{t.label}</span>
                <span class="pal-cat">{t.category}</span>
              </span>
            </button>
          {/each}
        </div>
      {/if}
      {#if runsOpen}
        <div class="palette runs-pop wf-pop" role="dialog" aria-label="Runs" style:inset-inline-end="{popRight}px">
          <LoadState
            what="runs"
            variant="compact"
            loading={runsLoading}
            error={runsError}
            empty={runs.length === 0}
            onretry={() => void loadRuns()}
          >
            {#snippet emptyView()}<div class="runs-empty">No runs yet — choose Run… to start one.</div>{/snippet}
          {#each runs as r (r.id)}
            <button class="run-item" data-testid="run-item" class:active={run?.id === r.id} onclick={() => void openRunById(r.workflow_id, r.id)}>
              <span class="dot {dotKey(r.status)}" aria-hidden="true"></span>
              <span class="run-status">{runStatusLabel(r.status)}</span>
              <span class="run-when"><RelTime iso={r.started_at} fallback="" /></span>
              <span class="grow"></span>
              <code class="run-id" title={r.id}>{shortRunId(r.id)}</code>
            </button>
          {/each}
          </LoadState>
        </div>
      {/if}

      <!-- Manual-run input editor (RunInputPanel). -->
      {#if runForm.open}
        <RunInputPanel form={runForm} {running} onrun={() => void confirmRun()} />
      {/if}

      <!-- The run's approval gate (RunApprovals): resumed note, preview, Approve / Deny. -->
      <RunApprovals {mon} {graph} {nodeName} />

      {#if validationIssues.length}
        <div class="preflight" use:announceShown={{ text: `Resolve these issues before running: ${validationIssues.map((i) => i.message).join('; ')}`, assertive: true }}><strong>Resolve these issues before running</strong>
          {#each validationIssues as issue}
            <button class="btn ghost small" onclick={() => { selectedId = issue.node_id; selectedEdgeId = issue.edge_id; }}>{issue.node_id ? nodeName(issue.node_id) : issue.edge_id ? 'Connection' : 'Graph'}: {issue.message}</button>
          {/each}
        </div>
      {/if}
      {#if run?.workflow_version && current.version !== run.workflow_version}
        <div class="preflight">This run uses version {run.workflow_version}; retries retain that definition. The editor shows version {current.version}. Review it and choose Run to start a new run with these changes.</div>
      {/if}
      <div class="canvas-wrap">
        <WorkflowCanvas
          bind:graph
          {types}
          {runStates}
          invalidNodes={validationIssues.flatMap((i) => i.node_id ? [i.node_id] : [])}
          invalidEdges={validationIssues.flatMap((i) => i.edge_id ? [i.edge_id] : [])}
          {selectedId}
          {selectedEdgeId}
          onselect={(id) => { selectedId = id; selectedEdgeId = null; }}
          onedgeselect={(id) => { selectedEdgeId = id; if (id) selectedId = null; }}
          onchange={() => (dirty = true)}
        />
      </div>

      {#if instructionsOpen && current}
        <div class="instructions-wrap">
          <div class="instructions-h">
            <span>Instructions</span>
            {#if instructionsDirty}<Badge tone="accent" label="Unsaved" />{/if}
            <span class="grow"></span>
            <button class="btn small" disabled={!instructionsDirty || savingInstructions} title={instructionsDirty ? 'Save the instructions' : 'No unsaved changes'} onclick={saveInstructions}>
              {savingInstructions ? 'Saving…' : 'Save instructions'}
            </button>
          </div>
          <p class="instructions-hint">
            Standing rules every step follows by the letter — distinct from the workflow’s description.
          </p>
          <textarea dir="auto"
            class="ri-text mono"
            rows="8"
            aria-label="Workflow instructions"
            bind:value={wfInstructions}
            placeholder="Standing rules every step follows by the letter (markdown)"
          ></textarea>
          <label class="resume-toggle">
            <input
              type="checkbox"
              checked={(current.on_restart ?? 'resume') !== 'fail'}
              onchange={(e) => void saveOnRestart(e.currentTarget.checked)}
            />
            <span>
              Resume after a daemon restart — an interrupted run picks up from the step it
              was on (steps with external side effects are never replayed). Unchecked, a
              restart fails the run and it must be re-run manually.
            </span>
          </label>
        </div>
      {/if}

      {#if triggersOpen && current}
        {#key current.id}
        <div class="triggers-wrap">
          <TriggersPanel
            workflowId={current.id}
            workflowName={current.name}
            bind:triggers
            ontriggers={(ts) => (triggers = ts)}
          />
        </div>
        {/key}
      {/if}

      {#if versionsOpen && current}
        <div class="versions-wrap">
          <div class="versions-h">
            <span>Version history</span>
            <span class="grow"></span>
            <button class="btn small" disabled={versionsLoading || versionsBefore === undefined} onclick={() => void loadVersions()}>Newest</button>
            <button class="btn small" disabled={versionsLoading || !versionsHasMore || !!versionsError} onclick={() => void loadVersions(versions.at(-1)?.version)}>Older</button>
            <button class="btn small" disabled={versionsLoading} onclick={() => void loadVersions(versionsBefore)}>
              Refresh
            </button>
          </div>
          <LoadState
            what="version history"
            variant="compact"
            loading={versionsLoading}
            error={versionsError}
            empty={versions.length === 0}
            onretry={() => void loadVersions(versionsBefore)}
          >
            {#snippet emptyView()}<EmptyState icon="clock" title={versionsBefore === undefined ? 'No saved versions yet' : 'No older versions'} body={versionsBefore === undefined ? 'Saving the canvas or restoring a version creates one.' : ''} />{/snippet}
            <ul class="versions">
              {#each versions as v (v.id)}
                <li class="ver">
                  <span class="ver-num">v{v.version}</span>
                  <span class="ver-note" title={v.note || undefined}>{v.note || '(no note)'}</span>
                  <span class="ver-when"><RelTime iso={v.created_at} fallback="" /></span>
                  <button class="btn small" title="Restore v{v.version} — saved as a new version, so nothing is lost" onclick={() => restoreVersion(v)}>Restore</button>
                </li>
              {/each}
            </ul>
          </LoadState>
        </div>
      {/if}

      {#if inspOpen}
        <!-- Drag grip: bottom mode grows the height cap; side mode (docked to a
             right column) drags the left edge to change width. Double-click
             resets. (R6) -->
        <div
          class="insp-grip"
          class:side={sideDock}
          role="separator"
          aria-label="Resize node inspector"
          title="Drag or use arrow keys to resize · double-click or Enter to reset"
          use:paneResizer={sideDock
            ? { value: ui.wfInspSideWidth, min: 280, max: Math.max(300, Math.round(window.innerWidth * 0.6)), invert: true, onChange: (w) => ui.setWfInspSideWidth(w), onReset: resetInsp, onDragStart: startInspResize, text: pxText }
            : { value: ui.runDetailHeight, min: 160, max: Math.max(160, Math.round(window.innerHeight * 0.85)), orientation: 'horizontal', invert: true, onChange: (h) => { runDetailMax = false; ui.setRunDetailHeight(h); }, onReset: resetInsp, onDragStart: startInspResize, text: (v) => `${Math.round(v)} pixels tall` }}
        ></div>
        <div
          class="inspector"
          class:maxed={runDetailMax}
          class:side={sideDock}
          style={sideDock ? `width:${ui.wfInspSideWidth}px` : `max-height:${inspMaxPx()}px`}
        >
          {#if sideDock}
            <!-- Persistent side-panel header (mirrors the agents Right panel):
                 title on the left, a real close (×) on the right. The
                 notification bell lives in the sidebar, not here. -->
            <div class="insp-side-head">
              <strong>Inspector</strong>
              <span class="grow"></span>
              <button
                class="icon-btn"
                onclick={closeInspector}
                title="Close the docked panel (re-open with Dock)"
                aria-label="Close panel"
              >
                <Icon name="x" size={13} />
              </button>
            </div>
          {/if}
          <!-- The run on screen: run bar, timeline, steps (RunMonitor). -->
          <RunMonitor {mon} {nodeName} bind:selectedId bind:runDetailMax onstop={() => void stop()} onOpenSession={openSessionInline} />

          {#if selectedNode}
            <div class="insp-h">
              <strong title={selectedNode.name || selectedNode.kind}>{selectedNode.name || selectedNode.kind}</strong>
              <span class="mono dim">{selectedNode.kind}</span>
              {#if selectedRun}<StatusBadge status={runStatus(selectedRun.status)} variant="text" />{/if}
              {#if selectedRun?.duration_ms != null}<span class="dim">· {fmtMs(selectedRun.duration_ms)}</span>{/if}
              <span class="grow"></span>
              <button class="btn small" disabled={running} onclick={() => runFrom(selectedNode.id, false)} title="Run this node and everything downstream"><Icon name="play" size={12} /> From here</button>
              <button class="btn small" disabled={running} onclick={() => runFrom(selectedNode.id, true)} title="Run only this node">Only this</button>
            </div>
            <NodeParams
              node={selectedNode}
              set={onParam}
              setParams={(params) => { if (selectedNode) { selectedNode.params = params; graph = graph; dirty = true; } }}
              wsId={current?.workspace_id ?? ws.currentId ?? ''}
              {pickers}
              workflowName={current?.name}
              onzoom={(field, label) => (jsonZoom = { field, label })}
            />

            <!-- Retry policy (any node): extra attempts with exponential backoff. -->
            {#if selectedNode.kind !== 'manual_trigger'}
              <div class="retry-form">
                <span class="retry-h">Retry · {selectedNode.retry ? 'custom' : 'default'} ({effectiveRetry(selectedNode).max_attempts} extra attempts)</span>
                <button class="btn ghost small" onclick={() => { if (selectedNode) clearRetry(selectedNode); graph = graph; dirty = true; }}>Use default</button>
                <div class="retry-row">
                  <label for="np-retry-max">Max retries (0–5)</label>
                  <input
                    id="np-retry-max"
                    type="number"
                    min="0"
                    max="5"
                    value={retryNum('max_attempts', 0)}
                    oninput={(e) => onRetry('max_attempts', Number(e.currentTarget.value))}
                  />
                  <label for="np-retry-bo">Backoff (ms)</label>
                  <input
                    id="np-retry-bo"
                    type="number"
                    min="0"
                    max="60000"
                    step="100"
                    value={retryNum('backoff_ms', 0)}
                    oninput={(e) => onRetry('backoff_ms', Number(e.currentTarget.value))}
                  />
                </div>
              </div>
            {/if}
            {#if inspectorError}
              <LoadState what="the step output" variant="compact" error={inspectorError} empty onretry={() => { inspectorError = null; inspectorRetry++; }} />
            {/if}
            {#if selectedRun?.error}
              <div class="err">{selectedRun.error}</div>
            {/if}
            {#if selectedRun?.logs?.length}
              <div class="logs">{#each selectedRun.logs as l}<div>{l}</div>{/each}</div>
            {/if}
            {#if selectedRun?.output !== undefined && selectedRun?.output !== null}
              <pre class="out">{JSON.stringify(selectedRun.output, null, 2).slice(0, 1200)}</pre>
            {/if}
          {:else if selectedEdge}
            <div class="insp-h">
              <strong>Connection</strong>
              <span class="mono dim">{nodeName(selectedEdge.source)} → {nodeName(selectedEdge.target)}</span>
              <span class="grow"></span>
              <button class="btn small danger" title="Delete connection" aria-label="Delete connection" onclick={removeSelectedEdge}>
                <Icon name="trash" size={12} />
              </button>
            </div>
            <label for="np-edge-cond">Condition (expression, optional)</label>
            <input dir="auto"
              id="np-edge-cond"
              type="text"
              placeholder="e.g. passed == true"
              value={selectedEdge.condition ?? ''}
              oninput={(e) => onEdgeCondition(e.currentTarget.value)}
            />
            <p class="node-hint">The target runs only when this is truthy. Leave blank for an unconditional edge.</p>
          {:else if !run}
            <!-- Docked (or bottom) with nothing selected: a centered, intentional
                 empty state — not a stray line floating at the top. -->
            <div class="insp-blank">
              <EmptyState icon="split" title="Select a node or connection to configure it" />
            </div>
          {/if}
        </div>
      {/if}
    {:else if !ws.currentId}
      <EmptyState
        variant="page"
        icon="folder"
        title="Add a workspace to get started"
        body="Workflows belong to a workspace. Add your project folder to create and run an automation."
        actionLabel="Add workspace"
        actionIcon="plus"
        onaction={() => (ui.newWorkspaceOpen = true)}
      />
    {:else if workflows.length === 0 && (wfError || wfLoading)}
      <LoadState what="workflows" variant="page" loading={wfLoading} error={wfError} empty onretry={() => void load()} />
    {:else if workflows.length === 0}
      <!-- First run: no list pane; describing a workflow is the page's CTA. -->
      <div class="first-run">
        <EmptyState
          variant="page"
          icon="split"
          title="No workflows yet"
          body="Describe an automation in plain words and an agent wires up the steps — or start blank, or from a template, and build it on the canvas."
        >
          {@render generator('page')}
          <AutomateGuide current="workflows" />
        </EmptyState>
      </div>
    {:else}
      <!-- Only while the last-opened workflow is being restored (or after
           closing one): a summary, not a bare "pick one". -->
      <EmptyState
        variant="page"
        icon="split"
        title="No workflow open"
        body={`${workflows.length} ${workflows.length === 1 ? 'workflow' : 'workflows'} in this workspace. Open one from the list, or describe a new one.`}
      />
    {/if}
  </main>
  {/if}

  {#if viewport.isDesktop && run && run.context_dir && ui.wfCtxOpen}
    <!-- Desktop: the run's Files / Agents sidebar (RunContextSidebar). -->
    <RunContextSidebar {mon} {run} contextDir={run.context_dir} {nodeName} focusSid={agentsFocusSid} />
  {/if}
</div>
</PageBody>
</div>

<!-- Big editor for a cramped node-form JSON field (R10). Edits write straight
     back to the selected node's param via onParamJson. -->
{#if jsonZoom}
  {@const jz = jsonZoom}
  <Modal title={jz.label} width={900} onclose={() => (jsonZoom = null)}>
    <textarea dir="ltr" aria-label={jz.label}
      class="json-zoom mono"
      data-testid="json-zoom-editor"
      value={paramJson(selectedNode, jz.field)}
      oninput={(e) => onParamJson(jz.field, e.currentTarget.value)}
      spellcheck="false"
      placeholder={'[ { "kind": "agent_prompt", "params": {} } ]'}
    ></textarea>
  </Modal>
{/if}

<style>
  /* A pressed header toggle (Context panel) reads as on. */
  .wf-tog.on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  /* Header toggles (Instructions, Triggers, Versions, Dock, Panel) show
     that their panel is open: the selection tint, never a solid fill. */
  .wf-root button[aria-pressed='true']:not(:disabled) {
    background: var(--accent-soft);
    border-color: var(--border-strong);
    color: var(--text);
  }
  .preflight { padding: 10px; display: flex; flex-direction: column; gap: 4px; border: 1px solid var(--border); }
  .wf-root {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .wf {
    flex: 1;
    display: flex;
    min-height: 0;
  }
  /* The list pane (Workflows + Running); its width + the hairline come from
     the shared PaneDivider beside it. */
  .side {
    width: 270px;
    flex-shrink: 0;
    position: relative;
    display: flex;
    flex-direction: column;
    background: var(--surface);
    min-height: 0;
    /* The list pane's minimum is 220px. Size queries hide nonessential run meta
       before the row spills across the divider. */
    container-type: inline-size;
    container-name: wf-side;
  }
  /* Phone: the list IS the first screen (push navigation). */
  .side.phone {
    flex: 1;
    width: auto;
    border-inline-end: none;
  }
  /* First run: the generator form sits under the page empty state. */
  .first-run {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .gen.gen-page {
    width: min(560px, 100%);
    margin-top: 12px;
    padding: 0;
    border-bottom: none;
    text-align: start;
    gap: 10px;
  }
  .gen.gen-page label {
    font-size: var(--fs-s);
    font-weight: 500;
    text-transform: none;
    letter-spacing: 0;
  }
  .gen-actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .tpl-start {
    display: flex;
    flex-direction: column;
    gap: 6px;
    margin-top: 12px;
  }
  .tpl-start-h {
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .tpl-grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 8px;
  }
  .tpl.tpl-card {
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .gen {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-bottom: 1px solid var(--border);
  }
  .gen label {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: .06em;
  }
  textarea {
    width: 100%;
    resize: vertical;
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1.45;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
  }
  textarea:focus {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .full {
    width: 100%;
    justify-content: center;
  }
  .tpl-menu {
    position: relative;
  }
  .tpl-toggle {
    justify-content: flex-start;
  }
  .tpl-pop {
    margin-top: 6px;
    max-height: 240px;
    overflow-y: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface);
    padding: 4px;
  }
  .tpl {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 8px 8px;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
  }
  .tpl:hover {
    background: var(--hover);
  }
  .tpl:focus-visible {
    outline: 2px solid var(--accent-text);
    outline-offset: -2px;
  }
  .tpl-ic {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-s);
    background: var(--accent-soft);
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .tpl-body {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .tpl-name {
    font-size: var(--fs-m);
    font-weight: 600;
    color: var(--text);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .tpl-sub {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .list {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    padding: 8px;
  }
  .list-h {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: .06em;
    padding: 4px 6px;
  }
  /* Was `.row` — the global app.css class (whose gap it relied on), so it's
     prefixed now and restates that gap. */
  .wf-row {
    display: flex;
    align-items: center;
    gap: 8px;
    border-radius: var(--radius-s);
  }
  .wf-row:hover {
    background: var(--hover);
  }
  .wf-row.active {
    background: var(--accent-soft);
  }
  .row-main {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
    padding: 6px 8px;
    background: none;
    border: none;
    color: var(--text);
    cursor: pointer;
    text-align: start;
  }
  .row-name {
    font-size: var(--fs-m);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row-edit {
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 6px;
    border-radius: var(--radius-s);
    opacity: 0;
  }
  .wf-row:hover .row-edit,
  .wf-row:focus-within .row-edit,
  .wf-row.active .row-edit {
    opacity: 1;
  }
  /* No hover on touch: the row menu would otherwise never show. */
  @media (hover: none) {
    .row-edit {
      opacity: 1;
    }
  }
  .row-edit:hover {
    color: var(--text);
  }
  .row-rename {
    flex: 1;
    min-width: 0;
    margin: 4px 6px;
    padding: 4px 6px;
    font-size: var(--fs-m);
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    outline: none;
  }
  .running {
    flex-shrink: 0;
    max-height: 38%;
    overflow-y: auto;
    padding: 8px;
    border-bottom: 1px solid var(--border);
  }
  .running .list-h {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .run-count {
    display: inline-grid;
    place-items: center;
    min-width: 16px;
    height: 16px;
    padding: 0 4px;
    border-radius: var(--radius-m);
    font-size: var(--fs-xs);
    background: var(--accent-soft-strong);
    color: var(--text);
  }
  .run-row {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-width: 0;
    overflow: hidden;
    padding: 6px 8px;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
    color: var(--text);
  }
  .run-row:hover {
    background: var(--hover);
  }
  .run-row.active {
    background: var(--accent-soft);
  }
  .run-name {
    font-size: var(--fs-m);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    flex: 1;
    min-width: 0;
  }
  .run-badge {
    display: inline-flex;
    color: var(--warning);
    flex-shrink: 0;
  }
  /* Disambiguators for concurrent runs of the same workflow (item 8). */
  .run-ord {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--accent-text);
    background: var(--accent-soft);
    padding: 0 4px;
    border-radius: 999px;
    flex-shrink: 0;
  }
  .run-id {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .run-prog {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
    flex-shrink: 0;
    white-space: nowrap;
  }
  .run-when {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-inline-start: 6px;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* 18-03: a 220px list cannot hold a name, an id, and a relative time. */
  @container wf-side (max-width: 280px) {
    .running .run-id,
    .running .run-when {
      display: none;
    }
  }
  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
    overflow-y: auto;
    /* Anchor for the side-docked inspector's absolute position. */
    position: relative;
  }
  /* ── Side-docked inspector: a resizable right column instead of the bottom
     strip. The `.main` reserves padding-inline-end (set inline = width) so the
     canvas/graph is never hidden under it; the panel + its left-edge grip are
     absolutely positioned into that reserved gutter. ── */
  .inspector.side {
    position: absolute;
    inset-block: 0;
    inset-inline-end: 0;
    max-height: none;
    border-top: none;
    border-inline-start: 1px solid var(--border);
    z-index: 6;
    box-shadow: -2px 0 8px color-mix(in srgb, var(--bg) 40%, transparent);
  }
  .insp-grip.side {
    position: absolute;
    inset-block: 0;
    /* Sit on the left edge of the panel (which is inset by its width). */
    inset-inline-end: var(--wf-insp-w, 400px);
    height: auto;
    width: 10px;
    border-top: none;
    z-index: 7;
    cursor: col-resize;
  }
  .insp-grip.side::after {
    width: 3px;
    height: 40px;
  }
  /* Persistent header for the side-docked panel (mirrors the agents Right panel
     header): full-bleed to the panel edges, sticks to the top while the body
     scrolls. Holds the title and the close (×). */
  .insp-side-head {
    display: flex;
    align-items: center;
    gap: 4px;
    /* Cancel the .inspector 10px/12px padding so the header spans edge-to-edge
       and its bottom border reads as a clean divider. */
    margin: -10px -12px 6px;
    padding-block: 8px; padding-inline: 12px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    position: sticky;
    top: -10px;
    z-index: 2;
  }
  .insp-side-head strong {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .insp-side-head .grow {
    flex: 1;
  }
  /* Centered, intentional empty state for the docked panel. */
  .insp-blank {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
  }
  .title-edit {
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 4px;
    display: inline-flex;
    align-items: center;
  }
  .title-edit:hover {
    color: var(--accent-text);
  }
  .wf-title-edit {
    font: inherit;
    padding: 2px 8px;
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    outline: none;
    min-width: 0;
    max-width: 100%;
  }
  .wf-title-edit:focus-visible,
  .row-rename:focus-visible {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .grow {
    flex: 1;
  }
  /* Header popovers anchored at the top of the editor pane (right = inline). */
  .palette.wf-pop {
    top: 6px;
    left: auto;
    inset-inline-end: auto;
    max-height: min(320px, calc(100% - 12px));
  }
  .palette {
    position: absolute;
    top: 30px;
    inset-inline-end: 0;
    z-index: var(--z-sticky);
    width: 230px;
    max-height: min(320px, calc(100% - 36px)); /* the pane, not the window */
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-l);
    box-shadow: var(--glass-shadow);
    padding: 4px;
  }
  .pal-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
    color: var(--text);
  }
  .pal-item:hover {
    background: var(--accent-soft);
  }
  .pal-ic {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: var(--radius-s);
    background: var(--accent-soft);
    color: var(--accent-text);
    flex-shrink: 0;
  }
  .pal-body {
    display: flex;
    flex-direction: column;
  }
  .pal-name {
    font-size: var(--fs-s);
    font-weight: 600;
  }
  .pal-cat {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    text-transform: uppercase;
  }
  .canvas-wrap {
    flex: 1;
    min-height: 160px;
    position: relative;
  }
  .inspector {
    border-top: 1px solid var(--border);
    background: var(--surface);
    padding: 10px 12px;
    /* max-height is set inline (ui.runDetailHeight / maximize) — the run-detail
       area is resizable via the top grip. (R6) */
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    /* Breathing room between every control in every node's params — 6px read
       as a solid block once a node had more than a handful of fields. */
    gap: 10px;
    flex-shrink: 0;
  }
  /* Drag grip that sits just above the inspector (between the canvas and the
     run detail), so it never scrolls with the content. (R6) */
  .insp-grip {
    height: 12px;
    flex-shrink: 0;
    cursor: row-resize;
    border-top: 1px solid var(--border);
    display: flex;
    align-items: center;
    justify-content: center;
  }
  /* Visible drag handle so the resize affordance is discoverable. */
  .insp-grip::after {
    content: '';
    width: 40px;
    height: 3px;
    border-radius: var(--radius-s);
    background: var(--border);
  }
  .insp-grip:hover::after,
  .insp-grip:focus-visible::after {
    background: var(--accent);
  }
  .insp-grip:hover,
  .insp-grip:focus-visible {
    background: linear-gradient(
      to bottom,
      var(--accent-line),
      transparent
    );
  }
  .insp-h {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 8px;
    font-size: var(--fs-m);
    min-width: 0;
  }
  .insp-h strong {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .insp-h .dim {
    color: var(--text-dim);
    font-size: var(--fs-xs);
  }
  .inspector label {
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
    /* Each label opens a field group. With the panel's flat 10px gap alone, a
       node with a dozen params reads as one wall of controls and the section
       you want is impossible to find by eye. Applies to EVERY node kind. */
    margin-top: 6px;
  }
  /* …except the first one. The node forms' labels (and their compact rows)
     are styled with the forms, in inspector/NodeParams.svelte. */
  .inspector label:first-child {
    margin-top: 0;
  }
  .inspector input[type='text'],
  .inspector input[type='number'] {
    width: 100%;
    font: inherit;
    font-size: var(--fs-m);
    padding: 6px 8px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
  }
  .inspector input[type='text']:focus,
  .inspector input[type='number']:focus {
    outline: none;
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }
  .err {
    color: var(--danger);
    font-size: var(--fs-s);
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
    padding: 6px 8px;
    border-radius: var(--radius-s);
  }
  .logs {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .out {
    font-size: var(--fs-xs);
    background: var(--surface-2);
    padding: 8px;
    border-radius: var(--radius-s);
    overflow-x: auto;
    margin: 0;
  }

  .json-zoom {
    width: 100%;
    min-height: 60vh;
    resize: vertical;
    font-family: var(--font-mono);
    font-size: var(--fs-m);
    line-height: 1.5;
    padding: 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    outline: none;
  }
  .json-zoom:focus {
    border-color: var(--accent-text);
    box-shadow: 0 0 0 3px var(--accent-soft-strong);
  }

  /* Runs history popover */
  .runs-pop {
    width: 200px;
  }
  .runs-empty {
    font-size: var(--fs-s);
    color: var(--text-dim);
    padding: 8px;
  }
  .run-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    cursor: pointer;
    color: var(--text);
  }
  .run-item:hover,
  .run-item.active {
    background: var(--accent-soft);
  }
  .run-status {
    flex: 1;
    text-align: start;
    font-size: var(--fs-s);
  }
  .run-when {
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    display: inline-block;
  }
  /* Run/step dots use the shared run vocabulary (lib/status.ts runStatus):
     running is info-blue and pulses — never the succeeded green. */
  .dot.succeeded {
    background: var(--status-working);
  }
  .dot.failed {
    background: var(--status-exited);
  }
  .dot.running {
    background: var(--info);
    animation: otto-pulse 1.6s ease-in-out infinite;
  }
  .dot.waiting {
    background: var(--status-warn);
  }
  .dot.queued,
  .dot.cancelled,
  .dot.skipped {
    background: var(--text-dim);
  }
  
  @media (prefers-reduced-motion: reduce) {
    .dot.running {
      animation: none;
    }
  }
  
  /* Triggers panel: collapsible section below the canvas */
  .triggers-wrap {
    border-top: 1px solid var(--border);
    max-height: 320px;
    overflow-y: auto;
  }
  /* Instructions panel: collapsible section below the canvas (mirrors
     .versions-wrap / .versions-h). */
  .instructions-wrap {
    border-top: 1px solid var(--border);
    background: var(--surface);
    /* Standing instructions are prepended to EVERY agent in the workflow —
       they are read far more often than they are edited, and 320px showed a
       fraction of a real one. */
    max-height: min(60vh, 560px);
    overflow-y: auto;
    padding: 10px 12px;
  }
  .instructions-h {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    margin-bottom: 6px;
  }
  .instructions-hint {
    margin: 0 0 6px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .resume-toggle {
    display: flex;
    align-items: flex-start;
    gap: 8px;
    margin-top: 8px;
    font-size: var(--fs-s);
    color: var(--text-dim);
    cursor: pointer;
  }
  .resume-toggle input {
    margin-top: 1px;
    accent-color: var(--accent);
  }
  .mono {
    font-family: var(--font-mono);
  }
  /* Node hint / info text in the inspector */
  .node-hint {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin: 2px 0 0;
  }
  /* Per-node retry sub-form */
  .retry-form {
    border-top: 1px solid var(--border);
    padding-top: 8px;
    margin-top: 2px;
  }
  .retry-h {
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
  }
  .retry-row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
  }
  .retry-row label {
    white-space: nowrap;
  }
  .retry-row input {
    width: 90px;
  }
  /* Versions panel: collapsible section below the canvas */
  .versions-wrap {
    border-top: 1px solid var(--border);
    background: var(--surface);
    max-height: 320px;
    overflow-y: auto;
    padding: 10px 12px;
  }
  .versions-h {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-xs);
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: .06em;
    color: var(--text-dim);
    margin-bottom: 8px;
  }
  .versions {
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .ver {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    font-size: var(--fs-s);
  }
  .ver-num {
    font-family: var(--font-mono);
    color: var(--accent-text);
    font-weight: 600;
    flex-shrink: 0;
  }
  .ver-note {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ver-when {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .insp-grip:focus-visible {
    outline: none;
  }
</style>
