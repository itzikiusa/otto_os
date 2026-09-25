<script lang="ts">
  import PathField from '../../lib/components/PathField.svelte';
  // Workflows: build automations by *describing* them (agent mode) or by hand
  // on the canvas. Left = generate + list + running; center = node-graph editor + run.
  import { untrack } from 'svelte';
  import { marked } from 'marked';
  import Icon, { asIcon } from '../../lib/components/Icon.svelte';
  import StatusBadge from '../../lib/components/StatusBadge.svelte';
  import { runStatus } from '../../lib/status';
  import Modal from '../../lib/components/Modal.svelte';
  import PageHeader from '../../lib/components/PageHeader.svelte';
  import EmptyState from '../../lib/components/EmptyState.svelte';
  import LoadState from '../../lib/components/LoadState.svelte';
  import { loadErrorText } from '../../lib/loadError';
  import { initialSelection, rememberSelection } from '../../lib/lastSelection';
  import { effectiveRetry, updateRetry, clearRetry } from './retryPolicy';
  import WorkflowCanvas from './WorkflowCanvas.svelte';
  import RunSteps from './RunSteps.svelte';
  import RunAgents from './RunAgents.svelte';
  import FileTree from '../panels/FileTree.svelte';
  import TriggersPanel from './TriggersPanel.svelte';
  import { ui } from '../../lib/stores/ui.svelte';
  import { agentProviders, defaultAgentProvider } from '../../lib/providers';
  import ModelPicker from '../../lib/components/ModelPicker.svelte';
  import { viewport } from '../../lib/stores/viewport.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { confirmer } from '../../lib/confirm.svelte';
  import { api } from '../../lib/api/client';
  import { workflowProgress, workflowNodeDetail, listWorkflowVersions, restoreWorkflowVersion } from '../../lib/api/workflows';
  import { mergeRunProgress, fmtStepMs } from './runProgress';
  import RelTime from '../../lib/components/RelTime.svelte';
  import { workflowRunBus } from '../../lib/events.svelte';
  import { workflowsPagePort } from '../../lib/uiCommands/workflows';
  import { copyTextOrThrow } from '../../lib/clipboard';
  import { ctxMenu, type MenuItem } from '../../lib/contextmenu.svelte';
  import type {
    Workflow,
    WorkflowGraph,
    WorkflowRun,
    NodeTypeSpec,
    NodeRunState,
    WorkflowTemplate,
    WorkflowTrigger,
    WorkflowVersion,
    FsRead,
    ReviewMode,
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
  let dirty = $state(false);
  let savingGraph = $state(false);

  let prompt = $state('');
  let generating = $state(false);
  let running = $state(false);
  let run = $state<WorkflowRun | null>(null);
  let runs = $state<Pick<WorkflowRun, 'id' | 'workflow_id' | 'status' | 'started_at' | 'rev'>[]>([]);
  let requestedRunId: string | null = null;
  let runsOpen = $state(false);
  // Manual-run input editor: where you provide repo_id / story_id / goals / msg
  // that the trigger emits to the graph.
  let runInputOpen = $state(false);
  let runInputText = $state('');
  // Prompt box shown above the JSON textarea in the run popover — merged into
  // the parsed input as `prompt` (non-empty only) on run.
  let runPromptText = $state('');
  // Per-run review-mode override, posted as `review_mode` only when set. Wins
  // over every review_run node's own `params.mode` for this run.
  let runReviewMode = $state<'' | ReviewMode>('');
  let paletteOpen = $state(false);
  let templatesOpen = $state(false);
  let triggersOpen = $state(false);
  let triggers = $state<WorkflowTrigger[]>([]);
  let approving = $state(false);

  // Instructions editor: standing free-text guidance every step follows,
  // distinct from `description`. Saved via an explicit action (like graph
  // Save), not tied into the canvas's `dirty` flag.
  let instructionsOpen = $state(false);
  let wfInstructions = $state('');
  let savingInstructions = $state(false);

  // Final-output panel: `<context_dir>/final-output.md`, fetched once per
  // successful run id. `finalOutputRunId` doubles as the "already attempted"
  // guard so a run with no deliverable doesn't get refetched forever.
  let finalOutputRunId = $state<string | null>(null);
  let finalOutputHtml = $state('');
  let finalOutputAvailable = $state(false);

  const runStates = $derived<Record<string, NodeRunState>>(
    Object.fromEntries((run?.nodes ?? []).map((n) => [n.node_id, n])),
  );
  const selectedNode = $derived(graph.nodes.find((n) => n.id === selectedId) ?? null);
  const selectedSummary = $derived(selectedId ? (runStates[selectedId] ?? null) : null);
  let inspectorBody = $state<NodeRunState | null>(null);
  const selectedRun = $derived(inspectorBody ?? selectedSummary);
  $effect(() => {
    const id = run?.id, node = selectedSummary;
    inspectorBody = null;
    if (!id || !node?.detail_version) return;
    const expected = node.detail_version;
    let active = true;
    void workflowNodeDetail(id,node.node_id).then(result => {
      if (active && result.detail_version === expected) inspectorBody = result.body;
    }).catch(() => {});
    return () => {active = false;};
  });
  const instructionsDirty = $derived(wfInstructions !== (current?.instructions ?? ''));

  $effect(() => {
    if (ws.currentId) {
      void load();
      // Populate the "Running" sidebar list on entry (also kept live by the
      // store's WS-event refresh).
      void ws.refreshActiveWorkflowRuns();
    }
  });

  // ── live run sync ──────────────────────────────────────────────────────────
  // ONE path keeps the viewed run live, whatever started it. Updates are
  // MERGED INTO the existing `run` object (never a wholesale replacement), so
  // everything the user is looking at — an expanded step, the timeline
  // selection, scroll positions — survives every tick. The run's monotonic
  // `rev` guards against stale/out-of-order snapshots ever regressing the view.
  //
  // Sources, all funneled through applyRunSnapshot / the WS fast-path:
  //   1. workflow_run_updated events carrying the changed node + a contiguous
  //      rev → merged in place, no network at all.
  //   2. Events with a rev gap / no node payload / terminal status → ONE
  //      single-flight, rev-guarded GET of the full run.
  //   3. A 2.5s fallback poll while the viewed run is non-terminal (missed
  //      events, or no WS connection).

  let destroyed = false;
  $effect(() => () => {
    destroyed = true;
  });

  /** Cheap change signature over the fields a node update can touch. Object
   *  fields (output/logs/sessions) are only reassigned when it changes, so
   *  unchanged step subtrees don't re-render on every merge. */
  function applyRunSnapshot(nr: WorkflowRun): void {
    if (run) mergeRunProgress(run,nr);
  }

  // Single-flight refetch: at most one GET in the air; a request that arrives
  // while one is flying coalesces into one trailing fetch.
  let runRead: AbortController | null = null;
  let runRefetchQueued = false;
  async function refetchRun(runId: string): Promise<void> {
    if (destroyed || document.hidden || run?.id !== runId) return;
    if (runRead) {runRefetchQueued = true; return;}
    const ac = new AbortController();
    runRead = ac;
    try {
      do {
        runRefetchQueued = false;
        const result = await workflowProgress(runId,run?.summary ? run.rev : undefined,ac.signal);
        if (ac.signal.aborted || document.hidden || run?.id !== runId) return;
        if (result.changed) applyRunSnapshot(result.run);
      } while (runRefetchQueued && run?.id === runId && !destroyed);
    } catch { /* The visible fallback poll heals transient errors. */ }
    finally {if (runRead === ac) runRead = null;}
  }
  $effect(() => {
    const id = run?.id;
    return () => {if (id) {runRead?.abort(); runRead = null;}};
  });
  $effect(() => {
    const visibility = () => {
      if (document.hidden) {runRead?.abort();runRead = null;}
      else if (run) void refetchRun(run.id);
    };
    document.addEventListener('visibilitychange',visibility);
    return () => document.removeEventListener('visibilitychange',visibility);
  });

  // (1) WS fast-path: apply the event to the viewed run without refetching
  // when it carries the changed node and the very next rev.
  $effect(() => {
    const _tick = workflowRunBus.tick; // dependency: re-run on each WS event
    void _tick;
    untrack(() => {
      const cur = run;
      if (!cur || workflowRunBus.runId !== cur.id) return;
      const evRev = workflowRunBus.rev;
      const curRev = cur.rev ?? 0;
      if (evRev > 0 && evRev <= curRev) return; // already have this state
      // Poll the small projection even for contiguous events: a node event
      // cannot describe checkpoint-only freshness or its current body version.
      void refetchRun(cur.id);
    });
  });

  // (2) Guaranteed: a slow safety poll while the viewed run is non-terminal,
  // so the view still converges with no WS connection at all.
  $effect(() => {
    const cur = run;
    if (!cur) return;
    if (cur.status !== 'pending' && cur.status !== 'running') return;
    const id = cur.id;
    const iv = setInterval(() => void refetchRun(id), 2500);
    return () => clearInterval(iv);
  });

  // ── final-output panel ──────────────────────────────────────────────────
  // On a successful run, `<context_dir>/final-output.md` holds the run's
  // deliverable (see workflow_context.rs's write_final_output). Rendered the
  // same way FileTree previews markdown: `marked` → a sandboxed iframe
  // (srcdoc, no scripts run) — that's the only "sanitizer" FileTree applies,
  // so mirror it exactly rather than injecting raw HTML into the page.
  // The srcdoc is themed from the app's tokens (resolved values copied in,
  // like PluginFrame's `otto:init` theme) so it reads in light AND dark; it
  // re-renders when the theme/scheme/accent changes (`finalOutputSrcdoc`).
  const FINAL_OUTPUT_CSS = `
    body { font: 14px/1.6 var(--font-ui); margin: 16px; color: var(--text); background: transparent; }
    h1,h2,h3 { line-height: 1.25; } h1,h2 { border-bottom: 1px solid var(--border); padding-bottom: .2em; }
    a { color: var(--accent-text); } code { background: var(--surface-2); padding: .15em .35em; border-radius: 4px; font-family: var(--font-mono); }
    pre { background: var(--surface-2); padding: 12px; border-radius: 6px; overflow: auto; } pre code { background: none; padding: 0; }
    table { border-collapse: collapse; } th,td { border: 1px solid var(--border); padding: 4px 8px; }
    blockquote { border-left: 3px solid var(--border-strong); margin: 0; padding-left: 12px; color: var(--text-dim); }
    img { max-width: 100%; }
  `;
  const FINAL_OUTPUT_TOKENS = ['--text', '--text-dim', '--border', '--border-strong', '--surface-2', '--accent-text', '--font-ui', '--font-mono'];
  /** Markdown → HTML body (stored); the themed document is `finalOutputSrcdoc`. */
  function renderFinalOutputSrcdoc(md: string): string {
    try {
      return marked.parse(md, { async: false, gfm: true, breaks: true }) as string;
    } catch {
      return `<pre>${md.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' }[c] ?? c))}</pre>`;
    }
  }
  function themedFinalOutput(body: string, scheme: 'light' | 'dark'): string {
    const cs = getComputedStyle(document.documentElement);
    const vars = FINAL_OUTPUT_TOKENS.map((v) => `${v}: ${cs.getPropertyValue(v).trim()};`).join(' ');
    return `<!doctype html><html><head><meta charset="utf-8"><style>
  :root { color-scheme: ${scheme}; ${vars} }${FINAL_OUTPUT_CSS}</style></head><body>${body}</body></html>`;
  }
  const finalOutputSrcdoc = $derived.by(() => {
    // Theme inputs are read so the frame re-themes when they change.
    void ui.theme;
    void ui.accent;
    return finalOutputHtml ? themedFinalOutput(finalOutputHtml, ui.resolvedScheme) : '';
  });
  async function loadFinalOutput(runId: string, contextDir: string): Promise<void> {
    finalOutputRunId = runId; // mark attempted up front — no duplicate fetches
    finalOutputAvailable = false;
    finalOutputHtml = '';
    try {
      const data = await api.get<FsRead>(`/fs/read?path=${encodeURIComponent(`${contextDir}/final-output.md`)}`);
      if (run?.id !== runId) return; // the view moved on while this was in flight
      finalOutputHtml = renderFinalOutputSrcdoc(data.content);
      finalOutputAvailable = true;
    } catch {
      // no final-output.md (or unreadable) — the panel just stays hidden
    }
  }
  $effect(() => {
    const cur = run;
    if (!cur || cur.status !== 'success' || !cur.context_dir) return;
    const runId = cur.id;
    const contextDir = cur.context_dir;
    untrack(() => {
      if (finalOutputRunId === runId) return;
      void loadFinalOutput(runId, contextDir);
    });
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
      if (result.changed) run = result.run;
      runsOpen = false;
    } catch (e) {toasts.error('Couldn’t open the run',e instanceof Error ? e.message : String(e));}
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
    const id = initialSelection('workflows', mine, (w) => w.id);
    const wf = mine.find((w) => w.id === id);
    if (wf) untrack(() => open(wf));
  });
  $effect(() => {
    if (current?.id) rememberSelection('workflows', current.id);
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
    graph = { nodes: [], edges: [] };
    selectedId = null;
    run = null;
    dirty = false;
  }

  // The Node palette / Runs popovers hang off header buttons, but the header's
  // action row clips overflow — so they render at the top of the editor pane,
  // horizontally under their button (clamped into the pane).
  let mainEl = $state<HTMLElement | null>(null);
  let popRight = $state(8);
  function anchorPop(e: MouseEvent, width: number): void {
    const b = (e.currentTarget as HTMLElement | null)?.getBoundingClientRect();
    const m = mainEl?.getBoundingClientRect();
    // A button collapsed into the header's "⋯" menu has no box — pin right.
    if (!b || !m || b.width === 0) {
      popRight = 8;
      return;
    }
    popRight = Math.max(8, Math.min(m.right - b.right, m.width - width - 8));
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
    const items: MenuItem[] = [];
    if (viewport.isPhone) {
      items.push(
        { label: 'Add node…', icon: 'plus', action: () => { popRight = 8; paletteOpen = true; } },
        { label: 'Save', icon: 'check', disabled: !dirty || savingGraph, title: dirty ? undefined : 'No unsaved changes', action: () => void save() },
        { label: 'Runs', icon: 'clock', action: () => { popRight = 8; runsOpen = true; void loadRuns(); } },
        { separator: true },
      );
    }
    items.push(
      { label: 'Instructions', checked: instructionsOpen, title: 'Standing rules every step follows, by the letter', action: () => (instructionsOpen = !instructionsOpen) },
      { label: 'Triggers', checked: triggersOpen, title: 'Configure what starts this workflow', action: () => (triggersOpen = !triggersOpen) },
      { label: 'Versions', checked: versionsOpen, title: 'Version history', action: () => { versionsOpen = !versionsOpen; if (versionsOpen) void loadVersions(); } },
      {
        label: 'Inspector on the side',
        checked: sideDock,
        title: sideDock ? 'Dock the node inspector to the bottom' : 'Dock the node inspector to a resizable side panel',
        action: () => ui.setWfDockSide(!sideDock),
      },
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
      items.push({ separator: true }, { label: 'Delete selected node', icon: 'trash', danger: true, action: removeSelected });
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
        graph = { nodes: [], edges: [] };
      }
    }
    wfLoading = true;
    try {
      if (types.length === 0) types = await api.get<NodeTypeSpec[]>('/workflows/node-types');
      if (templates.length === 0) templates = await api.get<WorkflowTemplate[]>('/workflows/templates');
      const rows = await api.get<Workflow[]>(`/workspaces/${wsId}/workflows`);
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
      toasts.error('Couldn’t create the workflow from the template', e instanceof Error ? e.message : String(e));
    }
  }

  function open(wf: Workflow): void {
    requestedRunId = null;
    validationIssues = [];
    current = wf;
    const g = structuredClone($state.snapshot(wf.graph)) as WorkflowGraph;
    graph = g && g.nodes ? g : { nodes: [], edges: [] };
    selectedId = null;
    selectedEdgeId = null;
    run = null;
    runsOpen = false;
    versionsOpen = false;
    versions = [];
    versionsError = null;
    // Never show the previous workflow's runs under this one while they load.
    runs = [];
    runsError = null;
    wfInstructions = wf.instructions ?? '';
    finalOutputRunId = null;
    finalOutputHtml = '';
    finalOutputAvailable = false;
    void loadRuns();
    dirty = false;
  }

  /** Opening another workflow (or restoring a version) replaces the canvas,
   *  so unsaved node/edge edits would vanish silently — ask first. */
  async function discardEditsOk(): Promise<boolean> {
    if ((!dirty && !instructionsDirty) || !current) return true;
    return confirmer.ask(`“${current.name}” has unsaved changes. Discard them?`, {
      title: 'Discard unsaved changes',
      confirmLabel: 'Discard changes',
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
      toasts.error('Couldn’t generate the workflow', e instanceof Error ? e.message : String(e));
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
      toasts.error('Couldn’t create the workflow', e instanceof Error ? e.message : String(e));
    }
  }

  /** A save updates its own row; a late response must never reopen an old editor. */
  function applySavedWorkflow(wf: Workflow): void {
    if (current?.id === wf.id && wf.version >= current.version) current = wf;
    workflows = workflows.map((w) => (w.id === wf.id && wf.version >= w.version ? wf : w));
  }

  async function save(): Promise<void> {
    if (!current || savingGraph) return;
    const workflowId = current.id;
    const submitted = JSON.stringify($state.snapshot(graph));
    savingGraph = true;
    try {
      const wf = await api.patch<Workflow>(`/workflows/${workflowId}`, { graph: JSON.parse(submitted) });
      applySavedWorkflow(wf);
      // Editing stays available during Save; only the submitted graph is saved.
      if (current?.id === workflowId && JSON.stringify($state.snapshot(graph)) === submitted) dirty = false;
      toasts.success('Saved');
    } catch (e) {
      toasts.error('Couldn’t save the workflow', e instanceof Error ? e.message : String(e));
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
      const wf = await api.patch<Workflow>(`/workflows/${current.id}`, { instructions: wfInstructions });
      applySavedWorkflow(wf);
      toasts.success('Instructions saved');
    } catch (e) {
      toasts.error('Couldn’t save the instructions', e instanceof Error ? e.message : String(e));
    } finally {
      savingInstructions = false;
    }
  }

  // Restart policy toggle: saved immediately (its own PATCH — a policy flip
  // must not ride on, or wait for, an unsaved instructions edit).
  async function saveOnRestart(resume: boolean): Promise<void> {
    if (!current) return;
    try {
      const wf = await api.patch<Workflow>(`/workflows/${current.id}`, {
        on_restart: resume ? 'resume' : 'fail',
      });
      applySavedWorkflow(wf);
    } catch (e) {
      toasts.error('Couldn’t save the restart policy', e instanceof Error ? e.message : String(e));
    }
  }

  // Deleting cascades (runs, triggers, node cache all go with it), so it asks
  // first; when the open workflow goes, open its neighbour instead of leaving
  // an empty "Pick a workflow" pane.
  async function del(wf: Workflow): Promise<void> {
    const ok = await confirmer.ask(
      `Delete “${wf.name}”? Its run history and triggers are deleted with it. This can't be undone.`,
      { title: 'Delete workflow', confirmLabel: 'Delete workflow' },
    );
    if (!ok) return;
    try {
      await api.del(`/workflows/${wf.id}`);
      const idx = workflows.findIndex((w) => w.id === wf.id);
      workflows = workflows.filter((w) => w.id !== wf.id);
      if (current?.id === wf.id) {
        const next = workflows[Math.min(Math.max(idx, 0), workflows.length - 1)] ?? null;
        if (next) open(next);
        else {
          current = null;
          graph = { nodes: [], edges: [] };
        }
      }
      toasts.success('Workflow deleted', wf.name);
    } catch (e) {
      toasts.error('Couldn’t delete the workflow', e instanceof Error ? e.message : String(e));
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
      toasts.error('Couldn’t duplicate the workflow', e instanceof Error ? e.message : String(e));
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
      const updated = await api.patch<Workflow>(`/workflows/${wf.id}`, { name });
      workflows = workflows.map((w) => (w.id === wf.id ? { ...w, name: updated.name } : w));
      if (current?.id === wf.id) current = { ...current, name: updated.name };
      toasts.success(`Renamed to “${updated.name}”`);
    } catch (e) {
      toasts.error('Couldn’t rename the workflow', e instanceof Error ? e.message : String(e));
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

  /** Await a run's terminal status WITHOUT driving the view: while the run is
   *  the one on screen, its live-merged state is authoritative (no network);
   *  once the user switches the view elsewhere, fall back to a cheap poll.
   *  This is what lets a user inspect another run while one is in flight —
   *  nothing here ever writes `run`. */
  async function waitRunTerminal(runId: string): Promise<{ status: string; error?: string | null }> {
    for (;;) {
      if (destroyed) return { status: 'canceled', error: null };
      const cur = untrack(() => run);
      let status: string;
      let error: string | null | undefined;
      if (cur?.id === runId) {
        status = cur.status;
        error = cur.error;
      } else {
        try {
          const g = await workflowProgress(runId);
          status = g.changed ? g.run.status : 'running';
          error = g.changed ? g.run.error : null;
        } catch {
          status = 'running'; // transient fetch error — keep waiting
        }
      }
      if (status !== 'pending' && status !== 'running') return { status, error };
      await new Promise((res) => setTimeout(res, 1000));
    }
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
    } catch (e) { toasts.error('Couldn’t validate the workflow', e instanceof Error ? e.message : String(e)); return false; }
    finally { validating = false; }
  }

  async function execRun(body: RunWorkflowReq): Promise<void> {
    const r = await startRun(body);
    if (r) await followRun(r);
  }

  /** Validate, save, POST the run and show it. Null when it didn't start (the
   *  reason is on the page: validation issues inline, a failed POST toasted).
   *  On success `running` stays set until {@link followRun} settles it. */
  async function startRun(body: RunWorkflowReq): Promise<WorkflowRun | null> {
    if (!current || running) return null;
    if (!await validateGraph()) return null;
    if (dirty) await save();
    if (dirty) return null; // failed save: never execute an older persisted graph
    running = true;
    const workflowId = current.id;
    try {
      const r = await api.post<WorkflowRun>(`/workflows/${workflowId}/run`, body);
      // Show the new run (a user-initiated view switch); from here the shared
      // live-run sync streams its progress in.
      requestedRunId = r.id;
      run = r;
      return r;
    } catch (e) {
      toasts.error('Couldn’t start the run', e instanceof Error ? e.message : String(e));
      running = false;
      return null;
    }
  }

  async function followRun(r: WorkflowRun): Promise<void> {
    try {
      const done = await waitRunTerminal(r.id);
      if (destroyed) return;
      if (done.status === 'success') toasts.success('Run complete');
      else if (done.status === 'canceled') toasts.info('Run cancelled');
      else toasts.error('Run finished with errors', done.error ?? '');
      void loadRuns();
    } catch (e) {
      toasts.error('Couldn’t start the run', e instanceof Error ? e.message : String(e));
    } finally {
      running = false;
    }
  }

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

  // A starter run-input JSON tailored to what this graph needs (repo for
  // review/PR nodes, story for product nodes), so the user knows what to fill in.
  function suggestRunInput(): string {
    const k = collectKinds();
    const obj: Record<string, unknown> = {};
    // Where the agents run (the repo/path to work in). Defaults to the workspace
    // root if omitted; set it to operate on a different repo.
    obj.working_directory = '~/path/to/repo';
    if (k.has('review_run') || k.has('git_pr')) {
      // Declare the repos/branches the run operates on — source AND
      // destination; several entries (branches, worktrees, repos) supported.
      // Omitted `source` ⇒ the repo's detected default branch.
      obj.repos = [
        { repo: '<repo id, name, or path>', type: 'branch', name: '<work branch>', source: '<target branch — optional>' },
      ];
    }
    if (k.has('product_analyze') || k.has('product_rewrite') || k.has('product_plan') || k.has('product_publish')) {
      obj.story_id = '<product story id>';
    }
    obj.msg = 'What you want done — instructions for the agents.';
    obj.jira_ticket = 'PROJ-0000';
    obj.goals = ['e.g. 100% test coverage (services)', 'under 2 minutes runtime'];
    // Optional: post the result somewhere specific (else it replies to the
    // trigger's origin; a manual run posts nowhere unless you set this).
    obj.result_channel = 'slack';
    obj.result_chat = '<channel id — optional>';
    return JSON.stringify(obj, null, 2);
  }

  // Invalid run-input JSON is a field error: said inline under the field (the
  // panel opens if a "Run from here" found it), never as a toast.
  let runInputError = $state('');
  $effect(() => {
    void runInputText;
    runInputError = '';
  });
  function parseRunInput(): Record<string, unknown> | undefined | null {
    const t = runInputText.trim();
    if (!t) return undefined;
    try {
      return JSON.parse(t);
    } catch (e) {
      runInputError = `Run input isn't valid JSON (${e instanceof Error ? e.message : 'parse error'}). Fix it, or clear the field to run with no input.`;
      runInputOpen = true;
      return null; // signal: invalid
    }
  }

  function openRunInput(): void {
    if (!runInputText.trim()) runInputText = suggestRunInput();
    runInputOpen = !runInputOpen;
  }

  /** Merge the Prompt box into the parsed run input as `prompt`, only when
   *  non-empty — leaves `input` untouched otherwise (including `undefined`,
   *  which still means "run with no input"). */
  function mergeRunPrompt(input: Record<string, unknown> | undefined): Record<string, unknown> | undefined {
    const p = runPromptText.trim();
    if (!p) return input;
    return { ...(input ?? {}), prompt: p };
  }

  async function confirmRun(): Promise<void> {
    const input = parseRunInput();
    if (input === null) return; // invalid JSON; shown under the field
    const merged = mergeRunPrompt(input);
    runInputOpen = false;
    await execRun({
      ...(merged !== undefined ? { input: merged } : {}),
      ...(runReviewMode ? { review_mode: runReviewMode } : {}),
    });
  }

  const runFrom = (nodeId: string, only: boolean): Promise<void> => {
    const input = parseRunInput();
    if (input === null) return Promise.resolve();
    const merged = mergeRunPrompt(input);
    return execRun({
      start_node: nodeId,
      only_node: only,
      ...(merged !== undefined ? { input: merged } : {}),
      ...(runReviewMode ? { review_mode: runReviewMode } : {}),
    });
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

  // Copy-paste Slack message that triggers THIS workflow by name (shown on the
  // Start node inspector + the Triggers panel).
  let mtCopied = $state(false);
  const mtSlackSnippet = $derived(
    `@otto\n` +
      `Action: Workflow\n` +
      `Name: ${current?.name ?? '<workflow name>'}\n` +
      `Msg: what you want done — instructions for the agents\n` +
      `Jira ticket: PROJ-1234\n` +
      `Working Directory: ~/path/to/repo\n` +
      `Relevant Info: ~/path/a, ~/path/b\n` +
      `Goals:\n  - 100% test coverage (services)\n  - under 2 minutes runtime`,
  );
  async function copyMtSlack(): Promise<void> {
    try {
      await copyTextOrThrow(mtSlackSnippet);
      mtCopied = true;
      setTimeout(() => (mtCopied = false), 1500);
    } catch {
      toasts.error('Couldn’t copy to the clipboard', 'Select the text and copy it manually.');
    }
  }

  // Cancelling can't be undone (the run halts after its current step and must
  // be started again), so it asks first — the same rule as the inspector's
  // "Cancel run" and a Stop in Goal Loops.
  async function stop(): Promise<void> {
    if (!run) return;
    const name = current?.name ?? 'this workflow';
    const ok = await confirmer.ask(
      `Cancel this run of “${name}”? It finishes the current step, then halts. Completed steps and their output are kept; to continue you start a new run.`,
      { title: 'Cancel run', confirmLabel: 'Cancel run' },
    );
    if (!ok || !run) return;
    try {
      await api.post(`/workflow-runs/${run.id}/cancel`, {});
      toasts.info('Cancelling run…', 'Finishes the current step, then halts.');
    } catch (e) {
      toasts.error('Couldn’t cancel the run', e instanceof Error ? e.message : String(e));
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
  const sideDock = $derived(ui.wfDockSide);

  function startInspResize(e: MouseEvent): void {
    e.preventDefault();
    runDetailMax = false;
    if (sideDock) {
      // Side dock: drag the inspector's LEFT edge — leftward widens it.
      const startX = e.clientX;
      const startW = ui.wfInspSideWidth;
      const onMove = (ev: MouseEvent) => ui.setWfInspSideWidth(startW + (startX - ev.clientX));
      const onUp = () => {
        window.removeEventListener('mousemove', onMove);
        window.removeEventListener('mouseup', onUp);
        document.body.style.cursor = '';
        document.body.style.userSelect = '';
      };
      window.addEventListener('mousemove', onMove);
      window.addEventListener('mouseup', onUp);
      document.body.style.cursor = 'col-resize';
      document.body.style.userSelect = 'none';
      return;
    }
    const startY = e.clientY;
    const startH = ui.runDetailHeight;
    const onMove = (ev: MouseEvent) => ui.setRunDetailHeight(startH + (startY - ev.clientY));
    const onUp = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'row-resize';
    document.body.style.userSelect = 'none';
  }

  // Left panel (Workflows list + Running) resize: drag its right edge (anchored
  // left → dragging right widens it). Mirrors startCtxResize.
  function startSideResize(e: MouseEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = ui.wfSideWidth;
    const onMove = (ev: MouseEvent) => ui.setWfSideWidth(startW + (ev.clientX - startX));
    const onUp = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
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

  // Total agent sessions the current run spawned — the Agents-tab badge.
  const runSessionCount = $derived(
    (run?.nodes ?? []).reduce((a, n) => a + (n.sessions?.length ?? 0), 0),
  );

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

  // Context-files sidebar width: drag its left edge (anchored right → dragging
  // left widens it). (R1)
  function startCtxResize(e: MouseEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = ui.wfCtxWidth;
    const onMove = (ev: MouseEvent) => ui.setWfCtxWidth(startW + (startX - ev.clientX));
    const onUp = () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      document.body.style.cursor = '';
      document.body.style.userSelect = '';
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
  }

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

  async function approveRun(approved: boolean): Promise<void> {
    if (!run?.waiting_approval || !run.approval_node_id || approving) return;
    approving = true;
    try {
      await api.post(`/workflow-runs/${run.id}/approve`, {
        node_id: run.approval_node_id,
        approved,
      });
      toasts.success(approved ? 'Approved — run resuming' : 'Rejected — run will error');
    } catch (e) {
      toasts.error('Couldn’t record the approval', e instanceof Error ? e.message : String(e));
    } finally {
      approving = false;
    }
  }

  function nodeName(id: string): string {
    const n = graph.nodes.find((x) => x.id === id);
    return n?.name || n?.kind || id;
  }
  const fmtMs = fmtStepMs;

  function onParam(field: string, value: unknown): void {
    if (!selectedNode) return;
    const params = { ...((selectedNode.params as Record<string, unknown>) ?? {}) };
    if (value === '' || value === null || value === undefined) {
      delete params[field];
    } else {
      params[field] = value;
    }
    selectedNode.params = params;
    graph = graph;
    dirty = true;
  }

  function paramStr(field: string): string {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    const v = p?.[field];
    return typeof v === 'string' ? v : v != null ? String(v) : '';
  }

  function paramNum(field: string, def: number): number {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    const v = p?.[field];
    return typeof v === 'number' ? v : typeof v === 'string' && v !== '' ? Number(v) : def;
  }

  function paramJson(field: string): string {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    const v = p?.[field];
    if (v == null) return '';
    return typeof v === 'string' ? v : JSON.stringify(v, null, 2);
  }

  // JSON param field zoom (R10): open a big modal editor for the cramped node-form
  // JSON textareas (Steps / Merge JSON / Body). Edits write straight back to the param.
  let jsonZoom = $state<{ field: string; label: string } | null>(null);

  function onParamJson(field: string, raw: string): void {
    if (!selectedNode) return;
    try {
      const parsed = JSON.parse(raw);
      onParam(field, parsed);
    } catch {
      // Keep invalid JSON as a string so the user can see what they typed
      // without losing the edit; the engine will reject it on run.
      onParam(field, raw);
    }
  }

  function paramBool(field: string, def = false): boolean {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    const v = p?.[field];
    return typeof v === 'boolean' ? v : def;
  }

  /** A string[] param rendered one-per-line (e.g. review goals). */
  function paramLines(field: string): string {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    const v = p?.[field];
    if (Array.isArray(v)) return v.filter((x) => typeof x === 'string').join('\n');
    return typeof v === 'string' ? v : '';
  }
  function onParamLines(field: string, raw: string): void {
    const lines = raw.split('\n').map((s) => s.trim()).filter((s) => s !== '');
    onParam(field, lines.length ? lines : undefined);
  }

  /** A string[] param rendered comma-separated (e.g. skills, providers, lenses). */
  function paramList(field: string): string {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    const v = p?.[field];
    if (Array.isArray(v)) return v.filter((x) => typeof x === 'string').join(', ');
    return typeof v === 'string' ? v : '';
  }
  function onParamList(field: string, raw: string): void {
    const items = raw.split(',').map((s) => s.trim()).filter((s) => s !== '');
    onParam(field, items.length ? items : undefined);
  }

  // --- review_run rich config: per-lens reviewers + summarizer + scoring -----
  // Mirrors the PR-review config (each lens gets its own provider set + optional
  // custom instructions), plus a generic per-severity scoring guideline.
  interface ReviewerRow {
    lens?: string;
    providers?: string[];
    instructions?: string;
  }
  // Reviewer providers come from the live registry (built-ins + custom).
  const REVIEW_PROVIDERS = $derived(agentProviders());

  function reviewers(): ReviewerRow[] {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    return Array.isArray(p?.reviewers) ? (p?.reviewers as ReviewerRow[]) : [];
  }
  function setReviewers(rows: ReviewerRow[]): void {
    onParam('reviewers', rows.length ? rows : undefined);
  }
  function addReviewer(): void {
    setReviewers([...reviewers(), { lens: '', providers: [defaultAgentProvider()] }]);
  }
  function removeReviewer(i: number): void {
    setReviewers(reviewers().filter((_, idx) => idx !== i));
  }
  function updateReviewer(i: number, patch: Partial<ReviewerRow>): void {
    setReviewers(reviewers().map((r, idx) => (idx === i ? { ...r, ...patch } : r)));
  }
  function toggleReviewerProvider(i: number, prov: string): void {
    const cur = reviewers()[i]?.providers ?? [];
    const next = cur.includes(prov) ? cur.filter((x) => x !== prov) : [...cur, prov];
    updateReviewer(i, { providers: next.length ? next : [defaultAgentProvider()] });
  }
  function summarizerField(field: 'provider' | 'instructions'): string {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    const s = p?.summarizer as Record<string, unknown> | undefined;
    const v = s?.[field];
    return typeof v === 'string' ? v : '';
  }
  function updateSummarizer(field: string, value: string): void {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    const next = { ...((p?.summarizer as Record<string, unknown>) ?? {}) };
    if (value === '') delete next[field];
    else next[field] = value;
    onParam('summarizer', Object.keys(next).length ? next : undefined);
  }
  function scoringField(sev: 'bug' | 'warn' | 'info', def: number): number {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    const s = p?.scoring as Record<string, unknown> | undefined;
    const v = s?.[sev];
    return typeof v === 'number' ? v : def;
  }
  function updateScoring(sev: string, value: number): void {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    const next = { ...((p?.scoring as Record<string, unknown>) ?? {}), [sev]: value };
    onParam('scoring', next);
  }

  // --- loop sub-steps: each step is a node-like {kind,name,params}. The loop
  // runs them in order each iteration. Agent-running sub-steps (agent_prompt,
  // prepare_context, review_run, product_*, canvas) get a real Provider/Model
  // editor here — no more hand-editing raw JSON to set a sub-step's provider. ---
  interface LoopStep {
    kind: string;
    name?: string;
    params?: Record<string, unknown>;
  }
  // Sub-step kinds that spawn a single agent → Provider + Model (+ Prompt).
  const LOOP_AGENT_KINDS = [
    'agent_prompt',
    'prepare_context',
    'product_analyze',
    'product_rewrite',
    'product_plan',
    'canvas',
  ];
  // Kinds a loop sub-step can be (agent kinds + review_run + the common
  // non-agent utilities). Anything else stays editable via Advanced JSON.
  const LOOP_STEP_KINDS = [...LOOP_AGENT_KINDS, 'review_run', 'http_request', 'db_query', 'delay', 'transform'];

  // Kinds that render their OWN Provider control (inline or a richer multi-agent
  // config). EVERY OTHER kind gets the shared universal Provider+Model block, so
  // NO node is ever missing a provider selector — including git_pr (drafts the PR
  // message with an agent), swarm_task, and the utility nodes.
  const KINDS_WITH_OWN_PROVIDER = [
    'agent_prompt',
    'prepare_context',
    'product_analyze',
    'product_rewrite',
    'product_plan',
    'canvas',
    'review_run',
    'self_improve',
    'loop',
    'budget_gate',
  ];

  function loopSteps(): LoopStep[] {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    return Array.isArray(p?.steps) ? (p?.steps as LoopStep[]) : [];
  }
  function setLoopSteps(steps: LoopStep[]): void {
    onParam('steps', steps.length ? steps : undefined);
  }
  function addLoopStep(): void {
    setLoopSteps([...loopSteps(), { kind: 'agent_prompt', name: '', params: {} }]);
  }
  function removeLoopStep(i: number): void {
    setLoopSteps(loopSteps().filter((_, idx) => idx !== i));
  }
  function updateLoopStep(i: number, patch: Partial<LoopStep>): void {
    setLoopSteps(loopSteps().map((s, idx) => (idx === i ? { ...s, ...patch } : s)));
  }
  function loopStepParam(i: number, key: string): string {
    const v = (loopSteps()[i]?.params ?? {})[key];
    return typeof v === 'string' ? v : v != null ? String(v) : '';
  }
  function loopStepParamList(i: number, key: string): string {
    const v = (loopSteps()[i]?.params ?? {})[key];
    if (Array.isArray(v)) return v.filter((x) => typeof x === 'string').join(', ');
    return typeof v === 'string' ? v : '';
  }
  function updateLoopStepParam(i: number, key: string, value: unknown): void {
    const step = loopSteps()[i];
    if (!step) return;
    const params = { ...(step.params ?? {}) };
    if (value == null || value === '' || (Array.isArray(value) && value.length === 0)) delete params[key];
    else params[key] = value;
    updateLoopStep(i, { params });
  }
  function updateLoopStepSummarizer(i: number, provider: string): void {
    const step = loopSteps()[i];
    if (!step) return;
    const params = { ...(step.params ?? {}) };
    if (provider.trim() === '') delete params.summarizer;
    else params.summarizer = { ...((params.summarizer as Record<string, unknown>) ?? {}), provider };
    updateLoopStep(i, { params });
  }
  function loopStepSummarizer(i: number): string {
    const s = (loopSteps()[i]?.params ?? {}).summarizer as Record<string, unknown> | undefined;
    return typeof s?.provider === 'string' ? s.provider : '';
  }

  // --- self_improve: reflection can run on one or more agents; the node's
  // `providers` array overrides the workspace's Self-Improvement providers. Empty
  // ⇒ use the configured set (Settings → Self-Improvement). ---
  function selfImproveProviders(): string[] {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    return Array.isArray(p?.providers) ? (p.providers as string[]) : [];
  }
  function toggleSelfImproveProvider(prov: string): void {
    const cur = selfImproveProviders();
    const next = cur.includes(prov) ? cur.filter((x) => x !== prov) : [...cur, prov];
    onParam('providers', next.length ? next : undefined);
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
  let versionsOpen = $state(false);
  let versions = $state<WorkflowVersion[]>([]);
  let versionsLoading = $state(false);
  let versionsError = $state<string | null>(null);

  async function loadVersions(): Promise<void> {
    if (!current) return;
    versionsLoading = true;
    try {
      versions = await listWorkflowVersions(current.id);
      versionsError = null;
    } catch (e) {
      // A failed load is shown inline in the panel, with Retry.
      versionsError = loadErrorText(e);
    } finally {
      versionsLoading = false;
    }
  }

  async function restoreVersion(v: WorkflowVersion): Promise<void> {
    if (!current) return;
    if (!(await discardEditsOk())) return;
    try {
      const wf = await restoreWorkflowVersion(current.id, v.version);
      current = wf;
      workflows = workflows.map((w) => (w.id === wf.id ? wf : w));
      open(wf);
      await loadVersions();
      toasts.success(`Restored v${v.version}`);
    } catch (e) {
      toasts.error('Couldn’t restore the version', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<!-- Label + zoom button for a cramped node-form JSON field (R10). -->
{#snippet jsonLabel(forId: string, label: string, field: string)}
  <div class="np-jsonlabel">
    <label for={forId}>{label}</label>
    <button
      type="button"
      class="np-zoom"
      title="Zoom — edit in a big editor"
      aria-label="Zoom JSON editor"
      onclick={() => (jsonZoom = { field, label })}
    >
      <Icon name="maximize" size={12} />
    </button>
  </div>
{/snippet}

<!-- The "new workflow" form: describe it (an agent wires the graph), start
     blank, or start from a template. Lives at the top of the list pane; with no
     workflows yet the list pane is hidden and this form IS the page's empty
     state (`page`), where Generate is the one primary action. -->
{#snippet generator(mode: 'side' | 'page')}
  <div class="gen" class:gen-page={mode === 'page'}>
    <label for="wf-prompt">Describe the flow</label>
    <textarea
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
          {#if generating}<span class="spin"></span> Building…{:else}<Icon name="zap" size={13} /> Generate workflow{/if}
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
        {#if generating}<span class="spin"></span> Building…{:else}<Icon name="zap" size={13} /> Generate workflow{/if}
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
      <!-- svelte-ignore a11y_autofocus -->
      <input
        class="wf-title-edit"
        bind:value={renameValue}
        autofocus
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
    {#if current && dirty}<span class="badge" title="The canvas has changes that aren't saved yet">Unsaved</span>{/if}
  {/snippet}
  {#snippet actions()}
    {#if current}
      <!-- Four everyday verbs stay in the bar (Node, Save, Runs, Run…); the
           panel toggles and one-off tools live in the ⋯ menu (wfMenu) so the
           row stays at ≤ 5 controls. On a phone Node/Save/Runs join the menu
           too, so there is never a second, auto-generated ⋯ next to ours. -->
      {#if !viewport.isPhone}
        <button class="btn small" data-overflow="1" data-icon="plus" aria-expanded={paletteOpen} onclick={(e) => { anchorPop(e, 230); paletteOpen = !paletteOpen; }}>
          <Icon name="plus" size={12} /> Node
        </button>
        <button class="btn small" data-overflow="2" data-icon="check" disabled={!dirty || savingGraph} title={dirty ? 'Save the canvas changes' : 'No unsaved changes'} onclick={save}>Save</button>
        <button class="btn small" data-overflow="1" data-icon="clock" aria-expanded={runsOpen} onclick={(e) => { anchorPop(e, 200); runsOpen = !runsOpen; if (runsOpen) void loadRuns(); }}>
          <Icon name="clock" size={12} /> Runs
        </button>
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
      <button class="icon-btn" data-keep aria-haspopup="menu" aria-label="More actions" title="More actions" onclick={wfMenu}>
        <Icon name="more" size={14} />
      </button>
      {#if running}
        <!-- While a run is live its Cancel takes the primary's place (same word as
             the inspector's "Cancel run" and the run's final "Cancelled"). -->
        <button class="btn small danger" data-keep onclick={stop} title="Cancel this run (finishes the current step, then halts)"><Icon name="square" size={11} /> Cancel run…</button>
      {:else}
        <button
          class="btn primary small"
          class:active={runInputOpen}
          onclick={openRunInput}
          title="Run — set the input (repo_id / story_id / goals / msg) the trigger emits"
          aria-expanded={runInputOpen}
        >
          <Icon name="play" size={12} /> Run…
        </button>
      {/if}
    {/if}
  {/snippet}
</PageHeader>
<div class="wf">
  {#if showSide}
  <aside class="side" class:phone={viewport.isPhone} style={viewport.isPhone ? '' : `width:${ui.wfSideWidth}px`}>
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
            <span class="run-name">{r.workflow_name}</span>
            {#if activeWfRunCounts[r.workflow_id] > 1}
              <span class="run-ord" title={`run #${activeRunOrdinals[r.run_id]} of this workflow`}>#{activeRunOrdinals[r.run_id]}</span>
            {/if}
            {#if r.waiting_approval}
              <span class="run-badge" role="img" title="Waiting for your approval" aria-label="Waiting for your approval"><Icon name="userCheck" size={12} /></span>
            {/if}
            <span class="grow"></span>
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
            <!-- svelte-ignore a11y_autofocus -->
            <input
              class="row-rename"
              data-testid="wf-rename-input"
              bind:value={renameValue}
              autofocus
              aria-label="Workflow name"
              onkeydown={(e) => {
                if (e.key === 'Enter') commitRename(wf);
                else if (e.key === 'Escape') cancelRename();
              }}
              onblur={() => commitRename(wf)}
            />
          {:else}
            <button class="row-main" title={wf.name} onclick={() => void openGuarded(wf)} oncontextmenu={(e) => { e.preventDefault(); rowMenu(e, wf); }}>
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
          {#snippet emptyView()}<p class="empty">No workflows yet — describe one above.</p>{/snippet}
        </LoadState>
      {/if}
    </div>
    {#if !viewport.isPhone}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="side-resize" onmousedown={startSideResize} title="Drag to resize"></div>
    {/if}
  </aside>
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
        <div class="palette wf-pop" style="right:{popRight}px">
          {#each types as t (t.kind)}
            <button class="pal-item" onclick={() => addNode(t)}>
              <span class="pal-ic" style="--c:{t.color}"><Icon name={asIcon(t.icon, 'box')} size={12} /></span>
              <span class="pal-body">
                <span class="pal-name">{t.label}</span>
                <span class="pal-cat">{t.category}</span>
              </span>
            </button>
          {/each}
        </div>
      {/if}
      {#if runsOpen}
        <div class="palette runs-pop wf-pop" style="right:{popRight}px">
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

      <!-- Manual-run input editor: this is WHERE you provide the run input the
           trigger emits (repo_id, story_id, goals, msg, jira_ticket, …). -->
      {#if runInputOpen}
        <div class="run-input">
          <div class="ri-head">
            <strong>Prompt</strong>
            <span class="ri-hint">What you want done this run — merged into the JSON below as `prompt` (optional).</span>
          </div>
          <textarea
            class="ri-prompt"
            rows="3"
            bind:value={runPromptText}
            placeholder="What you want done — instructions for the agents."
          ></textarea>
          <div class="ri-head">
            <strong>Run input</strong>
            <span class="ri-hint">JSON the Start trigger emits to the graph — fill in repo_id / story_id / goals as needed. Leave empty to run with no input.</span>
            <button class="btn small ghost" onclick={() => { runInputText = suggestRunInput(); }} title="Reset to a suggested template">Suggest</button>
          </div>
          <textarea
            class="ri-text mono"
            rows="8"
            bind:value={runInputText}
            spellcheck="false"
            aria-label="Run input (JSON)"
            aria-invalid={runInputError ? 'true' : undefined}
            aria-describedby={runInputError ? 'wf-run-input-err' : undefined}
            placeholder={'{\n  "repo_id": "…",\n  "goals": ["…"]\n}'}
          ></textarea>
          {#if runInputError}<p class="ri-err" id="wf-run-input-err" role="alert">{runInputError}</p>{/if}
          <div class="ri-head">
            <strong>Review mode</strong>
            <span class="ri-hint">Overrides the execution mode of every review step in this run, including steps that set their own.</span>
          </div>
          <select
            class="ri-select"
            data-testid="run-review-mode"
            bind:value={runReviewMode}
            title="Overrides the execution mode of every review step in this run, including steps that set their own."
          >
            <option value="">Workflow default</option>
            <option value="fan_out">Fan-out</option>
            <option value="orchestrator">Orchestrator</option>
          </select>
          <div class="ri-actions">
            <button class="btn small" onclick={() => (runInputOpen = false)}>Cancel</button>
            <button class="btn primary small" disabled={running} onclick={confirmRun}>
              <Icon name="play" size={12} /> Run
            </button>
          </div>
        </div>
      {/if}

      <!-- Human-approval banner: shown when a run is paused at a human_approval node -->
      {#if run && (run.resume_attempts ?? 0) > 0}
        <div class="resumed-banner" title="A daemon restart interrupted this run; it picked up from the step it was on">
          <Icon name="refresh" size={13} />
          <span>Resumed after a daemon restart (attempt {run.resume_attempts})</span>
        </div>
      {/if}

      {#if run?.waiting_approval && run.approval_node_id}
        <div class="approval-banner">
          <Icon name="userCheck" size={14} />
          <span>Run paused — waiting for approval at <strong title={run.approval_node_id}>{nodeName(run.approval_node_id)}</strong></span>
          <button class="btn primary small" disabled={approving} onclick={() => approveRun(true)}>
            Approve
          </button>
          <button class="btn small danger" disabled={approving} onclick={() => approveRun(false)}>
            Reject
          </button>
        </div>
      {/if}

      {#if validationIssues.length}
        <div class="preflight" role="alert"><strong>Resolve these issues before running</strong>
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
            {#if instructionsDirty}<span class="badge">unsaved</span>{/if}
            <span class="grow"></span>
            <button class="btn small" disabled={!instructionsDirty || savingInstructions} title={instructionsDirty ? 'Save the instructions' : 'No unsaved changes'} onclick={saveInstructions}>
              {savingInstructions ? 'Saving…' : 'Save instructions'}
            </button>
          </div>
          <p class="instructions-hint">
            Standing rules every step follows by the letter — distinct from the workflow's description.
          </p>
          <textarea
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
            <button class="btn small" disabled={versionsLoading} onclick={() => void loadVersions()}>
              Refresh
            </button>
          </div>
          <LoadState
            what="version history"
            variant="compact"
            loading={versionsLoading}
            error={versionsError}
            empty={versions.length === 0}
            onretry={() => void loadVersions()}
          >
            {#snippet emptyView()}<p class="empty">No saved versions yet — edits and restores create them.</p>{/snippet}
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
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="insp-grip"
          class:side={sideDock}
          onmousedown={startInspResize}
          ondblclick={() => (sideDock ? ui.setWfInspSideWidth(400) : ui.setRunDetailHeight(300))}
          title="Drag to resize · double-click to reset"
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
          {#if run}
            <!-- Run bar: live status + Cancel (R7) + maximize/zoom (R6). -->
            <div class="insp-bar">
              <span class="tl-label"><StatusBadge status={runStatus(run.status)} testid="run-status" /></span>
              <span class="grow"></span>
              {#if runActive}
                <button
                  class="btn small danger"
                  data-testid="run-cancel"
                  onclick={stop}
                  title="Cancel this run (finishes the current step, then halts)"
                >
                  <Icon name="square" size={11} /> Cancel run
                </button>
              {/if}
              <button
                class="icon-btn"
                data-testid="run-detail-max"
                onclick={() => (runDetailMax = !runDetailMax)}
                aria-pressed={runDetailMax}
                aria-label={runDetailMax ? 'Restore run-detail height' : 'Maximize run detail'}
                title={runDetailMax ? 'Restore run-detail height' : 'Maximize run detail'}
              >
                <Icon name={runDetailMax ? 'minimize' : 'maximize'} size={13} />
              </button>
            </div>
            <div class="timeline">
              {#each run.nodes as ns (ns.node_id)}
                <button
                  class="tl-step"
                  class:active={selectedId === ns.node_id}
                  data-status={ns.status}
                  onclick={() => (selectedId = ns.node_id)}
                >
                  <span class="dot {dotKey(ns.status)}" role="img" aria-label={runStatusLabel(ns.status)}></span>
                  <span class="tl-name">{nodeName(ns.node_id)}</span>
                  {#if ns.duration_ms != null}<span class="tl-ms">{fmtMs(ns.duration_ms)}</span>{/if}
                </button>
              {/each}
            </div>
            <div class="run-detail"><RunSteps {run} nodeName={(id) => nodeName(id)} onOpenSession={openSessionInline} onRunUpdated={applyRunSnapshot} onRefresh={() => { if (run) void refetchRun(run.id); }} /></div>
            {#if run.status === 'success' && run.context_dir && finalOutputAvailable && finalOutputRunId === run.id && !viewport.isDesktop}
              <!-- Mobile/tablet: the run's deliverable, above the context-file tree
                   below. On desktop this moves into the Context-files sidebar. -->
              <details open class="final-output">
                <summary>
                  <Icon name="check" size={13} />
                  <span>Final output</span>
                </summary>
                <iframe class="final-output-frame" title="Final output" sandbox="allow-same-origin" srcdoc={finalOutputSrcdoc}></iframe>
              </details>
            {/if}
            {#if run.context_dir && !viewport.isDesktop}
              <!-- Mobile/tablet: the run's context files inline (instruction brief,
                   repos.json, per-step handoffs). On desktop these move to the
                   full-height Context-files sidebar (see below) so they get real
                   room — same browsable tree + viewer the agent Files panel uses. -->
              <details class="ctx-files">
                <summary>
                  <Icon name="folder" size={13} />
                  <span>Context files</span>
                  <code class="dim">{run.context_dir}</code>
                </summary>
                <div class="ctx-files-tree"><FileTree root={run.context_dir} /></div>
              </details>
            {/if}
          {/if}

          {#if selectedNode}
            <div class="insp-h">
              <strong>{selectedNode.name || selectedNode.kind}</strong>
              <span class="mono dim">{selectedNode.kind}</span>
              {#if selectedRun}<StatusBadge status={runStatus(selectedRun.status)} variant="text" />{/if}
              {#if selectedRun?.duration_ms != null}<span class="dim">· {fmtMs(selectedRun.duration_ms)}</span>{/if}
              <span class="grow"></span>
              <button class="btn small" disabled={running} onclick={() => runFrom(selectedNode.id, false)} title="Run this node and everything downstream"><Icon name="play" size={11} /> From here</button>
              <button class="btn small" disabled={running} onclick={() => runFrom(selectedNode.id, true)} title="Run only this node">Only this</button>
            </div>
            <!-- Shared Provider + Model editor for every agent-running node —
                 sourced from the live registry (built-ins + custom, e.g. grok)
                 so no node is hardcoded to claude. Empty provider = default. -->
            {#snippet agentProviderModel()}
              <label for="np-provider">Provider</label>
              <select
                id="np-provider"
                value={paramStr('provider') || ''}
                onchange={(e) => onParam('provider', e.currentTarget.value || undefined)}
              >
                <option value="">default ({defaultAgentProvider()})</option>
                {#each agentProviders() as p (p)}<option value={p}>{p}</option>{/each}
              </select>
              <!-- Empty provider = default → resolve so the picker lists the
                   models that will actually run. Hides itself for a provider
                   with no model-flag template. -->
              <ModelPicker
                provider={paramStr('provider') || defaultAgentProvider()}
                value={paramStr('model')}
                onchange={(m) => onParam('model', m)}
              />
            {/snippet}
            <!-- Per-kind param forms. Each kind exposes only its meaningful
                 params; unrecognised kinds fall through to a raw JSON editor. -->
            {#if selectedNode.kind === 'manual_trigger'}
              <p class="insp-note">
                These fields are the <strong>run input</strong> the workflow starts with — fill them
                in here, then press <strong>Run</strong> (top-right). A Slack trigger or the
                <strong>Run…</strong> editor override them per key.
              </p>
              <label for="mt-msg">Message / prompt</label>
              <textarea
                id="mt-msg"
                class="np-prompt"
                rows="6"
                placeholder="What you want done — instructions for the agents"
                value={paramStr('msg')}
                oninput={(e) => onParam('msg', e.currentTarget.value)}
              ></textarea>
              <label for="mt-wd">Working directory (where agents run)</label>
              <PathField value={paramStr('working_directory')} onpick={(path) => onParam('working_directory', path)}><input
                id="mt-wd"
                type="text"
                placeholder="~/path/to/repo (default: workspace root)"
                value={paramStr('working_directory')}
                oninput={(e) => onParam('working_directory', e.currentTarget.value)}
              /></PathField>
              <label for="mt-repo">Repo ID (for review / PR steps)</label>
              <input
                id="mt-repo"
                type="text"
                placeholder="git repo id — copy from the Git tab"
                value={paramStr('repo_id')}
                oninput={(e) => onParam('repo_id', e.currentTarget.value)}
              />
              <label for="mt-base">Base branch</label>
              <input
                id="mt-base"
                type="text"
                placeholder="main"
                value={paramStr('base')}
                oninput={(e) => onParam('base', e.currentTarget.value)}
              />
              <label for="mt-story">Story ID (for product steps)</label>
              <input
                id="mt-story"
                type="text"
                placeholder="product story id"
                value={paramStr('story_id')}
                oninput={(e) => onParam('story_id', e.currentTarget.value)}
              />
              <label for="mt-jira">Jira ticket</label>
              <input
                id="mt-jira"
                type="text"
                placeholder="PROJ-1234"
                value={paramStr('jira_ticket')}
                oninput={(e) => onParam('jira_ticket', e.currentTarget.value)}
              />
              <label for="mt-goals">Goals (one per line)</label>
              <textarea
                id="mt-goals"
                class="np-prompt"
                rows="6"
                placeholder={'100% test coverage (services)\nunder 2 minutes runtime'}
                value={paramLines('goals')}
                oninput={(e) => onParamLines('goals', e.currentTarget.value)}
              ></textarea>
              <div class="insp-slack">
                <div class="is-head">
                  <strong>Or trigger from Slack</strong>
                  <button class="btn small" onclick={copyMtSlack}>{mtCopied ? 'Copied' : 'Copy'}</button>
                </div>
                <p class="insp-note">Post this where the Otto bot is configured for your workspace (matched by Name):</p>
                <pre class="is-snip">{mtSlackSnippet}</pre>
              </div>
            {:else if selectedNode.kind === 'agent_prompt'}
              <label for="np-prompt">Prompt</label>
              <textarea
                id="np-prompt"
                class="np-prompt np-prompt-lg"
                rows="14"
                value={paramStr('prompt')}
                oninput={(e) => onParam('prompt', e.currentTarget.value)}
              ></textarea>
              {@render agentProviderModel()}
              <label for="np-skills">Skills (comma-separated)</label>
              <input
                id="np-skills"
                type="text"
                placeholder="e.g. golang-testing, golang-code-review"
                value={paramList('skills')}
                oninput={(e) => onParamList('skills', e.currentTarget.value)}
              />
            {:else if selectedNode.kind === 'prepare_context'}
              <p class="insp-note">
                Fetches the Jira ticket from the input (if any) into a context file, then
                — if you give it a prompt — runs an agent to consolidate a brief. The
                Provider/Model below drive that agent phase.
              </p>
              <label for="np-pc-prompt">Prompt (optional — runs an agent when set)</label>
              <textarea
                id="np-pc-prompt"
                class="np-prompt np-prompt-lg"
                rows="14"
                placeholder="e.g. Read the ticket + repos and produce a consolidated brief…"
                value={paramStr('prompt')}
                oninput={(e) => onParam('prompt', e.currentTarget.value)}
              ></textarea>
              {@render agentProviderModel()}
              <label for="np-pc-skills">Skills (comma-separated)</label>
              <input
                id="np-pc-skills"
                type="text"
                placeholder="e.g. golang-testing"
                value={paramList('skills')}
                oninput={(e) => onParamList('skills', e.currentTarget.value)}
              />
              <label for="np-pc-account">Jira account ID (optional — else the default)</label>
              <input
                id="np-pc-account"
                type="text"
                placeholder="inherits the default Jira account"
                value={paramStr('account_id')}
                oninput={(e) => onParam('account_id', e.currentTarget.value || undefined)}
              />
              <label class="np-chk">
                <input
                  type="checkbox"
                  checked={paramBool('require')}
                  onchange={(e) => onParam('require', e.currentTarget.checked || undefined)}
                /> Fail the run if the Jira fetch fails
              </label>
            {:else if selectedNode.kind === 'http_request'}
              <label for="np-method">Method</label>
              <select
                id="np-method"
                value={paramStr('method') || 'GET'}
                onchange={(e) => onParam('method', e.currentTarget.value)}
              >
                {#each ['GET','POST','PUT','PATCH','DELETE','HEAD'] as m (m)}
                  <option value={m}>{m}</option>
                {/each}
              </select>
              <label for="np-url">URL</label>
              <input
                id="np-url"
                type="url"
                placeholder="https://example.com/api"
                value={paramStr('url')}
                oninput={(e) => onParam('url', e.currentTarget.value)}
              />
              {@render jsonLabel('np-body', 'Body (JSON, optional)', 'body')}
              <textarea
                id="np-body"
                rows="3"
                placeholder="&#123;&#125;"
                value={paramJson('body')}
                oninput={(e) => onParamJson('body', e.currentTarget.value)}
              ></textarea>
            {:else if selectedNode.kind === 'delay'}
              <label for="np-ms">Wait (ms)</label>
              <input
                id="np-ms"
                type="number"
                min="0"
                max="10000"
                step="100"
                value={paramNum('ms', 0)}
                oninput={(e) => onParam('ms', Number(e.currentTarget.value))}
              />
            {:else if selectedNode.kind === 'transform'}
              {@render jsonLabel('np-json', 'Merge JSON (object)', 'json')}
              <textarea
                id="np-json"
                rows="4"
                placeholder="&#123;&#125;"
                value={paramJson('json')}
                oninput={(e) => onParamJson('json', e.currentTarget.value)}
              ></textarea>
            {:else if selectedNode.kind === 'game_engine'}
              <label for="np-game">Game type</label>
              <select
                id="np-game"
                value={paramStr('game') || 'slots'}
                onchange={(e) => onParam('game', e.currentTarget.value)}
              >
                <option value="slots">Slots (5×3)</option>
                <option value="crash">Crash (Aviator-style)</option>
                <option value="scratch">Scratch card</option>
              </select>
            {:else if selectedNode.kind === 'db_query'}
              <label for="np-conn">Connection ID</label>
              <input
                id="np-conn"
                type="text"
                placeholder="DB-Explorer connection id"
                value={paramStr('connection_id')}
                oninput={(e) => onParam('connection_id', e.currentTarget.value)}
              />
              <label for="np-stmt">SQL / query statement</label>
              <textarea
                id="np-stmt"
                rows="4"
                placeholder="SELECT * FROM users LIMIT 100"
                value={paramStr('statement')}
                oninput={(e) => onParam('statement', e.currentTarget.value)}
              ></textarea>
              <label for="np-maxrows">Max rows (default 100)</label>
              <input
                id="np-maxrows"
                type="number"
                min="1"
                max="1000"
                value={paramNum('max_rows', 100)}
                oninput={(e) => onParam('max_rows', Number(e.currentTarget.value))}
              />
            {:else if selectedNode.kind === 'broker_peek'}
              <label for="np-cid">Cluster ID</label>
              <input
                id="np-cid"
                type="text"
                placeholder="Broker cluster id"
                value={paramStr('cluster_id')}
                oninput={(e) => onParam('cluster_id', e.currentTarget.value)}
              />
              <label for="np-topic">Topic</label>
              <input
                id="np-topic"
                type="text"
                placeholder="my-topic"
                value={paramStr('topic')}
                oninput={(e) => onParam('topic', e.currentTarget.value)}
              />
              <label for="np-limit">Limit (max 50)</label>
              <input
                id="np-limit"
                type="number"
                min="1"
                max="50"
                value={paramNum('limit', 20)}
                oninput={(e) => onParam('limit', Number(e.currentTarget.value))}
              />
            {:else if selectedNode.kind === 'channel_notify'}
              <label for="np-msg">Message</label>
              <textarea
                id="np-msg"
                rows="3"
                placeholder="Workflow step completed: &#123;reply&#125;"
                value={paramStr('message')}
                oninput={(e) => onParam('message', e.currentTarget.value)}
              ></textarea>
              <label for="np-ch">Channel (optional)</label>
              <select
                id="np-ch"
                value={paramStr('channel') || ''}
                onchange={(e) => onParam('channel', e.currentTarget.value || undefined)}
              >
                <option value="">Any enabled</option>
                <option value="slack">Slack</option>
                <option value="telegram">Telegram</option>
              </select>
            {:else if selectedNode.kind === 'budget_gate'}
              <label for="np-provider">Provider (whose usage budget to check)</label>
              <select
                id="np-provider"
                value={paramStr('provider') || defaultAgentProvider()}
                onchange={(e) => onParam('provider', e.currentTarget.value)}
              >
                {#each agentProviders() as pv (pv)}<option value={pv}>{pv}</option>{/each}
              </select>
              <p class="node-hint">Errors the run if the provider budget is exceeded and enforcement is on.</p>
            {:else if selectedNode.kind === 'human_approval'}
              <label for="np-aprompt">Approval prompt</label>
              <input
                id="np-aprompt"
                type="text"
                placeholder="Please review and approve to continue"
                value={paramStr('prompt')}
                oninput={(e) => onParam('prompt', e.currentTarget.value)}
              />
              <p class="node-hint">Pauses the run until an operator calls the resume endpoint or clicks Approve above.</p>
            {:else if selectedNode.kind === 'swarm_task'}
              <label for="np-swarm">Swarm ID</label>
              <input
                id="np-swarm"
                type="text"
                placeholder="Swarm id"
                value={paramStr('swarm_id')}
                oninput={(e) => onParam('swarm_id', e.currentTarget.value)}
              />
              <label for="np-proj">Project ID</label>
              <input
                id="np-proj"
                type="text"
                placeholder="Swarm project id"
                value={paramStr('project_id')}
                oninput={(e) => onParam('project_id', e.currentTarget.value)}
              />
              <label for="np-title">Task title</label>
              <input
                id="np-title"
                type="text"
                placeholder="Workflow-generated task title"
                value={paramStr('title')}
                oninput={(e) => onParam('title', e.currentTarget.value)}
              />
              <label for="np-desc">Description (optional)</label>
              <textarea
                id="np-desc"
                class="np-prompt"
                rows="6"
                placeholder="Task details…"
                value={paramStr('description')}
                oninput={(e) => onParam('description', e.currentTarget.value)}
              ></textarea>
            {:else if selectedNode.kind === 'api_run'}
              <label for="np-method">Method</label>
              <select
                id="np-method"
                value={paramStr('method') || 'GET'}
                onchange={(e) => onParam('method', e.currentTarget.value)}
              >
                {#each ['GET','POST','PUT','PATCH','DELETE'] as m (m)}
                  <option value={m}>{m}</option>
                {/each}
              </select>
              <label for="np-url">URL</label>
              <input
                id="np-url"
                type="url"
                placeholder="https://api.example.com/endpoint"
                value={paramStr('url')}
                oninput={(e) => onParam('url', e.currentTarget.value)}
              />
              {@render jsonLabel('np-body', 'Body (JSON, optional)', 'body')}
              <textarea
                id="np-body"
                rows="3"
                placeholder="&#123;&#125;"
                value={paramJson('body')}
                oninput={(e) => onParamJson('body', e.currentTarget.value)}
              ></textarea>
            {:else if selectedNode.kind === 'condition'}
              <label for="np-expr">Expression</label>
              <input
                id="np-expr"
                type="text"
                placeholder="e.g. score >= 80"
                value={paramStr('expr')}
                oninput={(e) => onParam('expr', e.currentTarget.value)}
              />
              <p class="node-hint">Truthy → downstream nodes run; falsy → they're skipped.</p>
            {:else if selectedNode.kind === 'loop'}
              <label for="np-maxiter">Max iterations (1–10)</label>
              <input
                id="np-maxiter"
                type="number"
                min="1"
                max="10"
                value={paramNum('max_iterations', 3)}
                oninput={(e) => onParam('max_iterations', Number(e.currentTarget.value))}
              />
              <label for="np-until">Until (expression, optional)</label>
              <input
                id="np-until"
                type="text"
                placeholder="e.g. passed == true"
                value={paramStr('until')}
                oninput={(e) => onParam('until', e.currentTarget.value)}
              />
              <!-- Structured sub-step editor: the loop body runs these in order
                   each iteration. Agent sub-steps expose Provider + Model + Prompt
                   directly (no more raw-JSON editing to set a provider). -->
              <div class="rv-h np-sec">
                <span class="np-label">Steps — run in order each iteration</span>
                <button class="btn small ghost" type="button" onclick={addLoopStep}>
                  <Icon name="plus" size={11} /> Add step
                </button>
              </div>
              {#if loopSteps().length === 0}
                <p class="insp-note">No steps yet — add one (e.g. a <code>review_run</code> then an <code>agent_prompt</code> “fix”).</p>
              {/if}
              {#each loopSteps() as step, i (i)}
                <div class="rv-row">
                  <div class="rv-top">
                    <select
                      class="ls-kind"
                      value={step.kind}
                      onchange={(e) => updateLoopStep(i, { kind: e.currentTarget.value })}
                    >
                      {#each LOOP_STEP_KINDS as k (k)}<option value={k}>{k}</option>{/each}
                      {#if !LOOP_STEP_KINDS.includes(step.kind)}<option value={step.kind}>{step.kind}</option>{/if}
                    </select>
                    <input
                      class="rv-lens"
                      type="text"
                      placeholder="name (e.g. fix)"
                      value={step.name ?? ''}
                      oninput={(e) => updateLoopStep(i, { name: e.currentTarget.value || undefined })}
                    />
                    <button class="rv-del" type="button" title="Remove step" aria-label="Remove step" onclick={() => removeLoopStep(i)}>
                      <Icon name="trash" size={11} />
                    </button>
                  </div>
                  {#if LOOP_AGENT_KINDS.includes(step.kind)}
                    <textarea
                      class="rv-instr np-prompt"
                      rows="6"
                      placeholder="prompt / instructions for this agent step"
                      value={loopStepParam(i, 'prompt')}
                      oninput={(e) => updateLoopStepParam(i, 'prompt', e.currentTarget.value)}
                    ></textarea>
                    <div class="ls-pm">
                      <select
                        class="ls-prov"
                        value={loopStepParam(i, 'provider')}
                        onchange={(e) => updateLoopStepParam(i, 'provider', e.currentTarget.value || undefined)}
                      >
                        <option value="">default ({defaultAgentProvider()})</option>
                        {#each agentProviders() as pv (pv)}<option value={pv}>{pv}</option>{/each}
                      </select>
                      <input
                        class="ls-model"
                        type="text"
                        placeholder="model (optional)"
                        value={loopStepParam(i, 'model')}
                        oninput={(e) => updateLoopStepParam(i, 'model', e.currentTarget.value || undefined)}
                      />
                    </div>
                  {:else if step.kind === 'review_run'}
                    <input
                      class="rv-lens"
                      type="text"
                      placeholder="reviewer providers (comma-separated, e.g. claude, codex)"
                      value={loopStepParamList(i, 'providers')}
                      oninput={(e) => updateLoopStepParam(i, 'providers', e.currentTarget.value.split(',').map((s) => s.trim()).filter(Boolean))}
                    />
                    <input
                      class="rv-lens"
                      type="text"
                      placeholder="summarizer provider (optional)"
                      value={loopStepSummarizer(i)}
                      oninput={(e) => updateLoopStepSummarizer(i, e.currentTarget.value)}
                    />
                  {:else}
                    <p class="insp-note">
                      <code>{step.kind}</code> takes no agent — edit its params in Advanced (JSON) below.
                    </p>
                  {/if}
                </div>
              {/each}
              <label class="np-chk">
                <input
                  type="checkbox"
                  checked={paramBool('continue_on_error')}
                  onchange={(e) => onParam('continue_on_error', e.currentTarget.checked)}
                /> Continue on step error
              </label>
              <details class="ls-advanced">
                <summary>Advanced — edit steps as raw JSON</summary>
                {@render jsonLabel('np-steps', 'Steps (JSON array)', 'steps')}
                <textarea
                  id="np-steps"
                  rows="5"
                  placeholder={'[ { "kind": "agent_prompt", "params": {} } ]'}
                  value={paramJson('steps')}
                  oninput={(e) => onParamJson('steps', e.currentTarget.value)}
                ></textarea>
              </details>
            {:else if selectedNode.kind === 'review_run'}
              <p class="insp-note">
                Leave Repo&nbsp;ID and Base empty to review exactly where the implementer worked
                (the run's working directory + base). Set them only to override.
              </p>
              <label for="np-repo">Repo ID (optional — inherits from the implementer)</label>
              <input
                id="np-repo"
                type="text"
                placeholder="inherits from the working directory"
                value={paramStr('repo_id')}
                oninput={(e) => onParam('repo_id', e.currentTarget.value)}
              />
              <label for="np-base">Base branch (optional — inherits, else main)</label>
              <input
                id="np-base"
                type="text"
                placeholder="inherits from the run, else main"
                value={paramStr('base')}
                oninput={(e) => onParam('base', e.currentTarget.value)}
              />
              <label for="np-threshold">Pass threshold (0–100)</label>
              <input
                id="np-threshold"
                type="number"
                min="0"
                max="100"
                value={paramNum('threshold', 80)}
                oninput={(e) => onParam('threshold', Number(e.currentTarget.value))}
              />
              <!-- How the lenses are spread over sessions. Empty value = no
                   `mode` key at all, so the run override / stored config wins. -->
              <label for="np-review-mode">Execution mode</label>
              <select
                id="np-review-mode"
                data-testid="review-mode-select"
                value={paramStr('mode')}
                onchange={(e) => onParam('mode', e.currentTarget.value || undefined)}
              >
                <option value="">Default (fan-out unless the run overrides it)</option>
                <option value="fan_out">Fan-out — one reviewer agent per lens × provider</option>
                <option value="orchestrator">Orchestrator — one agent per provider runs every lens as its own sub-agents</option>
              </select>
              <p class="insp-note">
                Both modes end with one summarizer. Orchestrator opens N sessions instead of
                N × lenses; fan-out finishes sooner on a fast machine, orchestrator is easier on
                file descriptors and CPU. A run started with an explicit review mode overrides
                this setting.
              </p>
              <div class="rv-h np-sec">
                <span class="np-label">Reviewers — one per lens, each its own agents (like PR review)</span>
                <button class="btn small ghost" type="button" onclick={addReviewer}>
                  <Icon name="plus" size={11} /> Add
                </button>
              </div>
              {#if reviewers().length === 0}
                <p class="insp-note">
                  No reviewers — leave empty to use the default PR-review config, or add one
                  (e.g. <code>correctness-review</code> on claude + codex).
                </p>
              {/if}
              <div class="rv-list">
              {#each reviewers() as r, i (i)}
                <div class="rv-row">
                  <div class="rv-top">
                    <input
                      class="rv-lens"
                      type="text"
                      placeholder="lens / skill (e.g. correctness-review)"
                      value={r.lens ?? ''}
                      oninput={(e) => updateReviewer(i, { lens: e.currentTarget.value })}
                    />
                    <button class="rv-del" type="button" title="Remove reviewer" aria-label="Remove reviewer" onclick={() => removeReviewer(i)}>
                      <Icon name="trash" size={11} />
                    </button>
                  </div>
                  <div class="rv-provs">
                    {#each REVIEW_PROVIDERS as prov (prov)}
                      <label class="rv-chip" class:on={(r.providers ?? []).includes(prov)}>
                        <input
                          type="checkbox"
                          checked={(r.providers ?? []).includes(prov)}
                          onchange={() => toggleReviewerProvider(i, prov)}
                        />
                        {prov}
                      </label>
                    {/each}
                  </div>
                  <textarea
                    class="rv-instr np-prompt"
                    rows="5"
                    placeholder="custom instructions for this reviewer (optional)"
                    value={r.instructions ?? ''}
                    oninput={(e) => updateReviewer(i, { instructions: e.currentTarget.value })}
                  ></textarea>
                </div>
              {/each}
              </div>

              <label class="np-sec" for="np-sum-prov">Summarizer (consolidates + scores)</label>
              <input
                id="np-sum-prov"
                type="text"
                placeholder="provider (e.g. claude)"
                value={summarizerField('provider')}
                oninput={(e) => updateSummarizer('provider', e.currentTarget.value)}
              />
              <textarea
                class="np-prompt np-prompt-lg"
                rows="14"
                placeholder="summarizer instructions (optional)"
                value={summarizerField('instructions')}
                oninput={(e) => updateSummarizer('instructions', e.currentTarget.value)}
              ></textarea>

              <span class="np-label np-sec">Scoring guideline — % deducted per open finding</span>
              <div class="rv-score">
                <label class="rv-sc">
                  Critical
                  <input type="number" min="0" max="100" value={scoringField('bug', 20)}
                    oninput={(e) => updateScoring('bug', Number(e.currentTarget.value))} />
                </label>
                <label class="rv-sc">
                  High
                  <input type="number" min="0" max="100" value={scoringField('warn', 5)}
                    oninput={(e) => updateScoring('warn', Number(e.currentTarget.value))} />
                </label>
                <label class="rv-sc">
                  Low
                  <input type="number" min="0" max="100" value={scoringField('info', 5)}
                    oninput={(e) => updateScoring('info', Number(e.currentTarget.value))} />
                </label>
              </div>
              <label class="np-sec" for="np-goals">Goals (one per line, optional)</label>
              <textarea
                id="np-goals"
                class="np-prompt"
                rows="7"
                placeholder={'No N+1 queries\nAll inputs validated'}
                value={paramLines('goals')}
                oninput={(e) => onParamLines('goals', e.currentTarget.value)}
              ></textarea>
              <label class="np-sec" for="np-checks">Checks — commands the reviewer runs (one per line, optional)</label>
              <textarea
                id="np-checks"
                class="np-prompt"
                rows="7"
                placeholder={'go test -tags=component ./...\ngo test -tags=integration ./...'}
                value={paramLines('checks')}
                oninput={(e) => onParamLines('checks', e.currentTarget.value)}
              ></textarea>
              <p class="insp-note">
                The reviewer agent runs these in the repo and reports any failure as a blocking
                finding — a safety net for a check the implementer may have skipped.
              </p>
              <label class="np-chk">
                <input
                  type="checkbox"
                  checked={paramBool('await', true)}
                  onchange={(e) => onParam('await', e.currentTarget.checked)}
                /> Wait for the review to finish
              </label>
              <label class="np-chk">
                <input
                  type="checkbox"
                  checked={paramBool('require_pass')}
                  onchange={(e) => onParam('require_pass', e.currentTarget.checked)}
                /> Require pass (fail this step if below threshold)
              </label>
            {:else if selectedNode.kind === 'product_analyze' || selectedNode.kind === 'product_rewrite' || selectedNode.kind === 'product_plan'}
              <label for="np-story">Story ID</label>
              <input
                id="np-story"
                type="text"
                placeholder="product story id"
                value={paramStr('story_id')}
                oninput={(e) => onParam('story_id', e.currentTarget.value)}
              />
              <label for="np-instruction">Extra instruction (optional)</label>
              <input
                id="np-instruction"
                type="text"
                placeholder="Focus on…"
                value={paramStr('instruction')}
                oninput={(e) => onParam('instruction', e.currentTarget.value)}
              />
              {@render agentProviderModel()}
              {#if selectedNode.kind !== 'product_analyze'}
                <label class="np-chk">
                  <input
                    type="checkbox"
                    checked={paramBool('persist')}
                    onchange={(e) => onParam('persist', e.currentTarget.checked)}
                  /> Persist as a product version
                </label>
              {/if}
            {:else if selectedNode.kind === 'product_publish'}
              <label for="np-story">Story ID</label>
              <input
                id="np-story"
                type="text"
                placeholder="product story id"
                value={paramStr('story_id')}
                oninput={(e) => onParam('story_id', e.currentTarget.value)}
              />
              <label for="np-pubkind">Publish as</label>
              <select
                id="np-pubkind"
                value={paramStr('kind') || 'rfc'}
                onchange={(e) => onParam('kind', e.currentTarget.value)}
              >
                <option value="rfc">RFC (Confluence)</option>
                <option value="jira">Jira story</option>
              </select>
              <label class="np-chk">
                <input
                  type="checkbox"
                  checked={paramBool('dry_run', true)}
                  onchange={(e) => onParam('dry_run', e.currentTarget.checked)}
                /> Dry run (preview only)
              </label>
              {#if !paramBool('dry_run', true)}
                <label for="np-account">Account ID</label>
                <input
                  id="np-account"
                  type="text"
                  placeholder="Jira/Confluence account id"
                  value={paramStr('account_id')}
                  oninput={(e) => onParam('account_id', e.currentTarget.value)}
                />
                {#if (paramStr('kind') || 'rfc') === 'jira'}
                  <label for="np-project">Project key</label>
                  <input
                    id="np-project"
                    type="text"
                    placeholder="e.g. PROJ"
                    value={paramStr('project_key')}
                    oninput={(e) => onParam('project_key', e.currentTarget.value)}
                  />
                  <label for="np-issuetype">Issue type</label>
                  <input
                    id="np-issuetype"
                    type="text"
                    placeholder="Story"
                    value={paramStr('issue_type')}
                    oninput={(e) => onParam('issue_type', e.currentTarget.value)}
                  />
                {:else}
                  <label for="np-space">Space key</label>
                  <input
                    id="np-space"
                    type="text"
                    placeholder="Confluence space key"
                    value={paramStr('space_key')}
                    oninput={(e) => onParam('space_key', e.currentTarget.value)}
                  />
                  <label for="np-parent">Parent page id (optional)</label>
                  <input
                    id="np-parent"
                    type="text"
                    placeholder="parent page id"
                    value={paramStr('parent_id')}
                    oninput={(e) => onParam('parent_id', e.currentTarget.value)}
                  />
                  <label for="np-pubtitle">Title (optional)</label>
                  <input
                    id="np-pubtitle"
                    type="text"
                    placeholder="page title"
                    value={paramStr('title')}
                    oninput={(e) => onParam('title', e.currentTarget.value)}
                  />
                {/if}
              {/if}
            {:else if selectedNode.kind === 'canvas'}
              <label for="np-cprompt">Prompt</label>
              <textarea
                id="np-cprompt"
                class="np-prompt"
                rows="7"
                placeholder="Diagram the request flow described in the input…"
                value={paramStr('prompt')}
                oninput={(e) => onParam('prompt', e.currentTarget.value)}
              ></textarea>
              <label for="np-cmode">Mode</label>
              <select
                id="np-cmode"
                value={paramStr('mode') || 'mermaid'}
                onchange={(e) => onParam('mode', e.currentTarget.value)}
              >
                <option value="mermaid">Mermaid</option>
                <option value="excalidraw">Excalidraw</option>
              </select>
              {@render agentProviderModel()}
            {:else if selectedNode.kind === 'git_pr'}
              <p class="insp-note">
                Leave Repo&nbsp;ID and Base empty to <strong>inherit the reference</strong> the
                implementer/reviewer used (the run's working directory and base, or the upstream
                review). Set them only to override. A run that changed several repos opens
                <strong>one PR per repo</strong> (from fanned-in reviews, or enable “detect changed”).
              </p>
              <label for="np-repo">Repo ID (optional — inherits from reference)</label>
              <input
                id="np-repo"
                type="text"
                placeholder="inherits from the upstream review / working directory"
                value={paramStr('repo_id')}
                oninput={(e) => onParam('repo_id', e.currentTarget.value)}
              />
              <label for="np-base">Base branch (optional — inherits from reference)</label>
              <input
                id="np-base"
                type="text"
                placeholder="inherits (per-repo base), else main"
                value={paramStr('base')}
                oninput={(e) => onParam('base', e.currentTarget.value)}
              />
              <label class="np-chk">
                <input
                  type="checkbox"
                  checked={paramBool('open')}
                  onchange={(e) => onParam('open', e.currentTarget.checked)}
                /> Open PR automatically on pass (gate the incoming edge on the review passing)
              </label>
              <label class="np-chk">
                <input
                  type="checkbox"
                  checked={paramBool('detect_changed')}
                  onchange={(e) => onParam('detect_changed', e.currentTarget.checked)}
                /> Detect changed repos — open a PR for every registered repo that has changes
              </label>
            {:else if selectedNode.kind === 'self_improve'}
              <p class="insp-note">
                Reflects on the workspace's recent agent sessions and <strong>offers</strong>
                skill/memory improvements. They are <strong>queued for approval</strong> in
                Self-Improvement — never auto-applied — and the offered list is posted to the
                trigger's chat thread.
              </p>
              <span class="np-label">Providers — the agent(s) that reflect (override Self-Improvement settings)</span>
              <div class="rv-provs">
                {#each agentProviders() as prov (prov)}
                  <label class="rv-chip" class:on={selfImproveProviders().includes(prov)}>
                    <input
                      type="checkbox"
                      checked={selfImproveProviders().includes(prov)}
                      onchange={() => toggleSelfImproveProvider(prov)}
                    />
                    {prov}
                  </label>
                {/each}
              </div>
              <p class="node-hint">
                Leave all unchecked to use the providers configured in
                <strong>Settings → Self-Improvement</strong>.
              </p>
            {:else if selectedNode.kind !== 'manual_trigger' && selectedNode.kind !== 'log' && selectedNode.kind !== 'verifier'}
              <!-- Fallback raw-JSON editor for unrecognised or future node kinds -->
              <label for="np-raw">Params (JSON)</label>
              <textarea
                id="np-raw"
                rows="5"
                placeholder="&#123;&#125;"
                value={selectedNode.params != null ? JSON.stringify(selectedNode.params, null, 2) : ''}
                oninput={(e) => {
                  try {
                    selectedNode.params = JSON.parse(e.currentTarget.value);
                    graph = graph;
                    dirty = true;
                  } catch { /* keep typing */ }
                }}
              ></textarea>
            {/if}

            <!-- Universal Provider + Model: EVERY node that doesn't render its own
                 provider control gets one here, so no node is ever missing it.
                 For agent-running kinds (git_pr drafts the PR message, swarm_task,
                 …) it drives the agent; for pure utility nodes it's simply present
                 and unused. Kinds with their own control are excluded to avoid a
                 duplicate. -->
            {#if !KINDS_WITH_OWN_PROVIDER.includes(selectedNode.kind) && selectedNode.kind !== 'manual_trigger'}
              {@render agentProviderModel()}
            {/if}

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
            <input
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
              <Icon name="split" size={30} />
              <p>Select a node or connection<br />to configure it.</p>
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

  <!-- Right sidebar (R1 + Agents): desktop-only, for a run with a context dir.
       Tabbed — Files (the run's context-file tree) and Agents (the run's live
       agent sessions, embedded). Collapse via the header "Panel" button so there's
       no second full-height rail beside the app shell's right rail. Resizable. -->
  {#if viewport.isDesktop && run && run.context_dir && ui.wfCtxOpen}
    <aside class="ctx-sidebar" style="width:{ui.wfCtxWidth}px" data-testid="ctx-sidebar">
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="ctx-resize" onmousedown={startCtxResize} title="Drag to resize"></div>
      <div class="ctx-head">
        <div class="ctx-tabs" role="tablist">
          <button
            class="ctx-tab"
            class:active={ui.wfCtxTab === 'files'}
            role="tab"
            aria-selected={ui.wfCtxTab === 'files'}
            onclick={() => ui.setWfCtxTab('files')}
          >
            <Icon name="folder" size={12} /> Files
          </button>
          <button
            class="ctx-tab"
            class:active={ui.wfCtxTab === 'agents'}
            role="tab"
            aria-selected={ui.wfCtxTab === 'agents'}
            data-testid="ctx-tab-agents"
            onclick={() => ui.setWfCtxTab('agents')}
          >
            <Icon name="terminal" size={12} /> Agents{#if runSessionCount > 0}<span class="tab-count">{runSessionCount}</span>{/if}
          </button>
        </div>
        <span class="grow"></span>
        <button
          class="icon-btn"
          data-testid="ctx-collapse"
          onclick={() => ui.toggleWfCtx()}
          title="Collapse panel"
          aria-label="Collapse panel"
        >
          <Icon name="panel" size={13} />
        </button>
      </div>
      {#if ui.wfCtxTab === 'files'}
        <div class="ctx-pathline"><code class="ctx-path dim" title={run.context_dir}>{run.context_dir}</code></div>
        {#if run.status === 'success' && finalOutputAvailable && finalOutputRunId === run.id}
          <details open class="final-output">
            <summary>
              <Icon name="check" size={13} />
              <span>Final output</span>
            </summary>
            <iframe class="final-output-frame" title="Final output" sandbox="allow-same-origin" srcdoc={finalOutputSrcdoc}></iframe>
          </details>
        {/if}
        <div class="ctx-body">
          {#key run.context_dir}
            <FileTree root={run.context_dir} primary={false} />
          {/key}
        </div>
      {:else}
        <div class="ctx-body">
          <RunAgents {run} nodeName={(id) => nodeName(id)} focusSid={agentsFocusSid} />
        </div>
      {/if}
    </aside>
  {/if}
</div>
</div>

<!-- Big editor for a cramped node-form JSON field (R10). Edits write straight
     back to the selected node's param via onParamJson. -->
{#if jsonZoom}
  {@const jz = jsonZoom}
  <Modal title={jz.label} width={900} onclose={() => (jsonZoom = null)}>
    <textarea
      class="json-zoom mono"
      data-testid="json-zoom-editor"
      value={paramJson(jz.field)}
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
  .ri-err {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--danger);
  }
  /* Header toggles (Instructions, Triggers, Versions, Dock, Panel) show
     that their panel is open: the selection tint, never a solid fill. */
  .wf-root button[aria-pressed='true']:not(:disabled) {
    background: var(--accent-soft);
    border-color: var(--border-strong);
    color: var(--text);
  }
  .preflight { padding: 10px; display: flex; flex-direction: column; gap: 5px; border: 1px solid var(--border); }
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
  .side {
    width: 270px;
    flex-shrink: 0;
    position: relative;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    min-height: 0;
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
  /* Drag the left panel's right edge to resize it. */
  .side-resize {
    position: absolute;
    inset-inline-end: -3px;
    top: 0;
    bottom: 0;
    width: 7px;
    cursor: col-resize;
    z-index: 5;
  }
  .side-resize:hover {
    background: linear-gradient(
      to right,
      transparent,
      color-mix(in srgb, var(--accent) 40%, transparent),
      transparent
    );
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
    letter-spacing: 0.04em;
  }
  textarea {
    width: 100%;
    resize: vertical;
    font: inherit;
    font-size: var(--fs-m);
    line-height: 1.45;
    padding: 7px 9px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
  }
  /* EVERY textarea in the inspector holds a prompt, an instruction list or a
     JSON blob — the things you actually need to read. A `rows="2"` box showing
     four lines of a 2,000-word prompt is not an editor. Give them all a
     readable floor, cap them against the viewport, and scroll the CONTENT
     inside the box; the panel itself scrolls for the rest. Node-kind agnostic
     on purpose — this must hold for every agent and every step, not just the
     reviewer. */
  .inspector textarea {
    min-height: 110px;
    max-height: min(45vh, 460px);
    overflow-y: auto;
  }
  textarea:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  /* Prompt-sized textareas (reviewer/summarizer instructions, goals, checks).
     `rows` alone loses to the inspector's own scroll: a 3-row box holding a
     2,000-word summarizer prompt shows four lines of it and the rest is
     reachable only by dragging the resize corner. Give them real height, cap
     it against the viewport, and let the text scroll INSIDE the box so the
     panel around it doesn't have to grow. */
  .inspector textarea.np-prompt {
    min-height: 120px;
    max-height: min(40vh, 380px);
    overflow-y: auto;
    resize: vertical;
  }
  .inspector textarea.np-prompt-lg {
    min-height: 260px;
    max-height: min(60vh, 620px);
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
    outline: 2px solid var(--accent);
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
    letter-spacing: 0.04em;
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
    padding: 7px 8px;
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
    padding: 5px 7px;
    font-size: var(--fs-m);
    background: var(--surface-2);
    color: var(--text);
    border: 1px solid var(--accent);
    border-radius: var(--radius-s);
    outline: none;
  }
  .empty {
    font-size: var(--fs-s);
    color: var(--text-dim);
    padding: 8px 6px;
  }
  /* "Running" sidebar list — in-flight runs across the workspace, live. */
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
    border-radius: 8px;
    font-size: var(--fs-xs);
    background: color-mix(in srgb, var(--accent) 22%, transparent);
    color: var(--text);
  }
  .run-row {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
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
    max-width: 120px;
  }
  .run-badge {
    display: inline-flex;
    color: var(--warning);
    flex-shrink: 0;
  }
  /* Disambiguators for concurrent runs of the same workflow (item 8). */
  .run-ord {
    font-size: var(--fs-xs);
    font-weight: 700;
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    padding: 0 5px;
    border-radius: 99px;
    flex-shrink: 0;
  }
  .run-id {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex-shrink: 0;
  }
  .run-prog {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .run-when {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    margin-inline-start: 6px;
  }
  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
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
    padding: 8px 10px 8px 12px;
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
    gap: 10px;
    padding: 24px 16px;
    color: var(--text-dim);
    text-align: center;
  }
  .insp-blank :global(svg) {
    opacity: 0.5;
  }
  .insp-blank p {
    font-size: var(--fs-s);
    line-height: 1.5;
    margin: 0;
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
    min-width: 220px;
  }
  .wf-title-edit:focus-visible,
  .row-rename:focus-visible {
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .badge {
    font-size: var(--fs-xs);
    color: var(--accent-text);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    padding: 1px 7px;
    border-radius: 99px;
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
    z-index: 40;
    width: 230px;
    max-height: 320px;
    overflow-y: auto;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    box-shadow: var(--shadow);
    padding: 5px;
  }
  .pal-item {
    display: flex;
    align-items: center;
    gap: 9px;
    width: 100%;
    padding: 7px 8px;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    cursor: pointer;
    text-align: start;
    color: var(--text);
  }
  .pal-item:hover {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .pal-ic {
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    background: color-mix(in srgb, var(--c) 18%, transparent);
    color: var(--c);
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
    min-height: 0;
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
    border-radius: 3px;
    background: var(--border);
  }
  .insp-grip:hover::after {
    background: var(--accent);
  }
  .insp-grip:hover {
    background: linear-gradient(
      to bottom,
      color-mix(in srgb, var(--accent) 40%, transparent),
      transparent
    );
  }
  /* Run bar: sticky so status + Cancel + maximize stay reachable while the run
     detail scrolls. (R6/R7) */
  .insp-bar {
    display: flex;
    align-items: center;
    gap: 8px;
    position: sticky;
    top: -10px;
    margin: -10px -12px 4px;
    padding: 8px 12px;
    background: var(--surface);
    border-bottom: 1px solid var(--border);
    z-index: 2;
  }
  .insp-bar .grow {
    flex: 1;
  }
  .insp-note {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
    line-height: 1.5;
  }
  .insp-slack {
    margin-top: 8px;
    padding-top: 8px;
    border-top: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .is-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .is-snip {
    margin: 0;
    padding: 8px 10px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: var(--fs-xs);
    line-height: 1.5;
    white-space: pre-wrap;
    color: var(--text);
    overflow-x: auto;
  }
  .insp-h {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: var(--fs-m);
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
  /* …except the first one, and labels that are part of a compact row
     (checkbox rows, the per-reviewer score fields) — those set their own. */
  .inspector label:first-child,
  .inspector .np-chk,
  .inspector .rv-sc,
  .inspector .rv-chip {
    margin-top: 0;
  }
  /* Opens a major section inside a node's params (Reviewers, Summarizer,
     Scoring, Goals, Checks, loop Steps …). A rule, not a per-node style: any
     node kind can mark a group with it and get the same separation. */
  .inspector .np-sec {
    margin-top: 16px;
    padding-top: 12px;
    border-top: 1px solid var(--border);
  }
  .np-label {
    display: block;
    font-size: var(--fs-xs);
    font-weight: 600;
    color: var(--text-dim);
  }
  .inspector input[type='text'],
  .inspector input[type='url'],
  .inspector input[type='number'] {
    width: 100%;
    font: inherit;
    font-size: var(--fs-m);
    padding: 6px 9px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
  }
  .inspector input[type='text']:focus,
  .inspector input[type='url']:focus,
  .inspector input[type='number']:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .inspector select {
    width: 100%;
    font: inherit;
    font-size: var(--fs-m);
    padding: 6px 9px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
    cursor: pointer;
  }
  .inspector select:focus {
    outline: none;
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .err {
    color: var(--status-exited);
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

  /* Run timeline */
  .timeline {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow-x: auto;
    padding-bottom: 8px;
    margin-bottom: 8px;
    border-bottom: 1px solid var(--border);
    /* overflow-x zeroes a flex item's automatic min-height, so the flex-column
       inspector would vertically compress (clip) this row. Pin it. (R8) */
    flex-shrink: 0;
  }
  .run-detail {
    margin: 8px 0;
  }
  .ctx-files {
    margin: 8px 0;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
  }
  .ctx-files > summary {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
    user-select: none;
    min-width: 0;
  }
  .ctx-files > summary code {
    font-size: var(--fs-xs);
    font-weight: 400;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: ltr;
  }
  .ctx-files-tree {
    border-top: 1px solid var(--border);
    max-height: 420px;
    overflow: auto;
  }
  /* Final-output panel: the run's deliverable, shown above the context-file
     tree (both the mobile inline block and the desktop sidebar). Same shell
     as .ctx-files; content is a sandboxed iframe like FileTree's own markdown
     preview (see renderFinalOutputSrcdoc). */
  .final-output {
    margin: 8px 0;
    border: 1px solid var(--border);
    border-radius: 8px;
    background: var(--surface-2);
    flex-shrink: 0;
  }
  .final-output > summary {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    font-size: var(--fs-s);
    font-weight: 600;
    cursor: pointer;
    user-select: none;
  }
  .final-output-frame {
    display: block;
    width: 100%;
    height: 320px;
    border: none;
    border-top: 1px solid var(--border);
    background: var(--surface); /* the srcdoc body is transparent + token-themed */
  }
  .tl-label {
    display: inline-flex;
    align-items: center;
    flex-shrink: 0;
    padding-inline-end: 4px;
  }
  .tl-step {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    padding: 5px 10px;
    border: 1px solid var(--border);
    border-radius: 99px;
    background: var(--surface-2);
    color: var(--text);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .tl-step.active {
    border-color: var(--accent);
  }
  .tl-step[data-status='success'] {
    border-color: color-mix(in srgb, var(--success) 55%, var(--border));
  }
  .tl-step[data-status='error'] {
    border-color: color-mix(in srgb, var(--danger) 55%, var(--border));
  }
  /* Running is info-blue (lib/status.ts) — the accent border is `.active`
     (selection), so a running step no longer looks selected. */
  .tl-step[data-status='running'] {
    border-color: color-mix(in srgb, var(--info) 60%, var(--border));
  }
  .tl-name {
    white-space: nowrap;
  }
  .tl-ms {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    font-family: var(--font-mono);
  }

  /* Context-files sidebar (R1): a full-height file viewer on the right edge. */
  .ctx-sidebar {
    position: relative;
    flex-shrink: 0;
    height: 100%;
    display: flex;
    flex-direction: column;
    border-inline-start: 1px solid var(--border);
    background: var(--bg);
    min-height: 0;
  }
  .ctx-resize {
    position: absolute;
    inset-inline-start: -3px;
    top: 0;
    bottom: 0;
    width: 7px;
    cursor: col-resize;
    z-index: 5;
  }
  .ctx-resize:hover {
    background: linear-gradient(
      to right,
      transparent,
      color-mix(in srgb, var(--accent) 40%, transparent),
      transparent
    );
  }
  .ctx-head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 7px 8px 6px 10px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
    flex-shrink: 0;
    min-width: 0;
  }
  .ctx-tabs {
    display: flex;
    gap: 2px;
    min-width: 0;
  }
  .ctx-tab {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    border: none;
    background: transparent;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    padding: 3px 8px;
    border-radius: var(--radius-s);
    cursor: pointer;
    border-bottom: 2px solid transparent;
  }
  .ctx-tab:hover {
    color: var(--text);
  }
  .ctx-tab.active {
    color: var(--text);
    border-bottom-color: var(--accent);
  }
  .tab-count {
    font-family: var(--font-mono);
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0;
    color: var(--text-dim);
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    padding: 0 5px;
    border-radius: 99px;
  }
  .ctx-pathline {
    padding: 4px 10px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
  }
  .ctx-path {
    display: block;
    min-width: 0;
    font-size: var(--fs-xs);
    font-family: var(--font-mono);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    direction: ltr;
  }
  .ctx-body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }

  /* Node-form JSON field: label + zoom button; the editor lives in a modal. (R10) */
  .np-jsonlabel {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .np-zoom {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--text-dim);
    padding: 2px 5px;
    border-radius: var(--radius-s);
    cursor: pointer;
    flex-shrink: 0;
  }
  .np-zoom:hover {
    color: var(--text);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
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
    border-color: var(--accent);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 22%, transparent);
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
    padding: 7px 8px;
    background: none;
    border: none;
    border-radius: var(--radius-s);
    cursor: pointer;
    color: var(--text);
  }
  .run-item:hover,
  .run-item.active {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
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
  .btn.danger {
    color: var(--status-exited);
    border-color: color-mix(in srgb, var(--status-exited) 45%, var(--border));
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
    animation: wf-dot-pulse 1.6s ease-in-out infinite;
  }
  .dot.waiting {
    background: var(--status-warn);
  }
  .dot.queued,
  .dot.cancelled,
  .dot.skipped {
    background: var(--text-dim);
  }
  @keyframes wf-dot-pulse {
    50% {
      opacity: 0.4;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .dot.running {
      animation: none;
    }
  }
  .spin {
    width: 11px;
    height: 11px;
    border: 2px solid currentColor;
    border-inline-end-color: transparent;
    border-radius: 50%;
    display: inline-block;
    animation: rot 0.7s linear infinite;
  }
  @keyframes rot {
    to {
      transform: rotate(360deg);
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
    letter-spacing: 0.04em;
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
  /* Human-approval banner */
  .run-input {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
  }
  .ri-head {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-wrap: wrap;
  }
  .ri-hint {
    flex: 1;
    min-width: 200px;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  /* `.ri-text` is the JSON run-input box; the prompt box above it is
     `.ri-prompt` — same tokens, its own hook (a single `.ri-text` is what
     identifies the run input, in the UI and in e2e). */
  .ri-text,
  .ri-prompt {
    width: 100%;
    min-height: 120px;
    resize: vertical;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--text);
    font-size: var(--fs-s);
    line-height: 1.5;
  }
  /* Same tokens as .ri-text, but sized to its content — the popover is a
     column flexbox, so an auto width needs the cross-axis opt-out too. */
  .ri-select {
    width: auto;
    align-self: flex-start;
    padding: 6px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .ri-actions {
    display: flex;
    gap: 8px;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }
  .resumed-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 12px;
    background: var(--surface-2);
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .approval-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: var(--warning-soft);
    border-bottom: 1px solid var(--border);
    font-size: var(--fs-m);
    color: var(--text);
  }
  .approval-banner strong {
    font-weight: 700;
  }
  .approval-banner > span {
    flex: 1;
  }
  /* Node hint / info text in the inspector */
  .node-hint {
    font-size: var(--fs-s);
    color: var(--text-dim);
    margin: 2px 0 0;
  }
  /* Inspector checkbox row */
  .np-chk {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text);
    margin-top: 2px;
  }
  .np-chk input {
    width: auto;
  }
  /* review_run reviewer editor (per-lens agents + summarizer + scoring) */
  .rv-h {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-top: 4px;
  }
  /* A reviewer list is never capped or scrolled on its own: hiding agents to
     make the sections below reachable just moves the invisibility. EVERY agent
     renders at full size and the inspector panel scrolls. */
  .rv-list {
    display: flex;
    flex-direction: column;
    gap: 10px;
  }
  .rv-row {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 10px;
    /* A tint so each agent reads as its own card in a long list. */
    background: color-mix(in srgb, var(--surface-2) 55%, transparent);
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .rv-top {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .rv-lens {
    flex: 1;
  }
  /* loop sub-step editor */
  .ls-kind {
    flex: 0 0 auto;
    max-width: 150px;
  }
  .ls-pm {
    display: flex;
    gap: 6px;
  }
  .ls-prov {
    flex: 1;
    min-width: 0;
  }
  .ls-model {
    flex: 1;
    min-width: 0;
  }
  .ls-advanced {
    margin-top: 2px;
  }
  .ls-advanced summary {
    font-size: var(--fs-xs);
    color: var(--text-dim);
    cursor: pointer;
    user-select: none;
  }
  .rv-del {
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 2px;
  }
  .rv-del:hover {
    color: var(--status-exited);
  }
  .rv-provs {
    display: flex;
    flex-wrap: wrap;
    gap: 5px;
  }
  .rv-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    font-size: var(--fs-xs);
    padding: 2px 7px;
    border: 1px solid var(--border);
    border-radius: 999px;
    cursor: pointer;
    color: var(--text-dim);
  }
  .rv-chip.on {
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    border-color: var(--accent);
    color: var(--text);
  }
  .rv-chip input {
    width: auto;
    margin: 0;
  }
  .rv-instr {
    width: 100%;
    font-size: var(--fs-s);
  }
  .rv-score {
    display: flex;
    gap: 8px;
  }
  .rv-sc {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
    flex: 1;
  }
  .rv-sc input {
    width: 100%;
  }
  /* Per-node retry sub-form */
  .retry-form {
    border-top: 1px solid var(--border);
    padding-top: 8px;
    margin-top: 2px;
  }
  .retry-h {
    font-size: var(--fs-xs);
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.05em;
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
    letter-spacing: 0.04em;
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
</style>
