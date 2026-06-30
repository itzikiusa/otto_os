<script lang="ts">
  // Workflows: build automations by *describing* them (agent mode) or by hand
  // on the canvas. Left = generate + list + running; center = node-graph editor + run.
  import { untrack } from 'svelte';
  import Icon from '../../lib/components/Icon.svelte';
  import WorkflowCanvas from './WorkflowCanvas.svelte';
  import RunSteps from './RunSteps.svelte';
  import TriggersPanel from './TriggersPanel.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { api } from '../../lib/api/client';
  import { listWorkflowVersions, restoreWorkflowVersion } from '../../lib/api/workflows';
  import { workflowRunBus } from '../../lib/events.svelte';
  import type {
    Workflow,
    WorkflowGraph,
    WorkflowRun,
    NodeTypeSpec,
    NodeRunState,
    WorkflowTemplate,
    WorkflowTrigger,
    WorkflowVersion,
  } from '../../lib/api/types';

  let workflows = $state<Workflow[]>([]);
  let templates = $state<WorkflowTemplate[]>([]);
  let types = $state<NodeTypeSpec[]>([]);
  let current = $state<Workflow | null>(null);
  let graph = $state<WorkflowGraph>({ nodes: [], edges: [] });
  let selectedId = $state<string | null>(null);
  let dirty = $state(false);

  let prompt = $state('');
  let generating = $state(false);
  let running = $state(false);
  let run = $state<WorkflowRun | null>(null);
  let runs = $state<WorkflowRun[]>([]);
  let runsOpen = $state(false);
  // Manual-run input editor: where you provide repo_id / story_id / goals / msg
  // that the trigger emits to the graph.
  let runInputOpen = $state(false);
  let runInputText = $state('');
  let paletteOpen = $state(false);
  let templatesOpen = $state(false);
  let triggersOpen = $state(false);
  let triggers = $state<WorkflowTrigger[]>([]);
  let approving = $state(false);

  const runStates = $derived<Record<string, NodeRunState>>(
    Object.fromEntries((run?.nodes ?? []).map((n) => [n.node_id, n])),
  );
  const selectedNode = $derived(graph.nodes.find((n) => n.id === selectedId) ?? null);
  const selectedRun = $derived(selectedId ? (runStates[selectedId] ?? null) : null);

  $effect(() => {
    if (ws.currentId) {
      void load();
      // Populate the "Running" sidebar list on entry (also kept live by the
      // store's WS-event refresh).
      void ws.refreshActiveWorkflowRuns();
    }
  });

  // Requirement D — keep a *viewed* run live, not only one started via execRun.
  // (1) Snappy: refetch the shown run when a workflow_run_updated event names it.
  $effect(() => {
    const _tick = workflowRunBus.tick; // dependency: re-run on each WS event
    void _tick;
    untrack(() => {
      const r = run;
      if (!r || running) return; // execRun already drives the run it started
      if (workflowRunBus.runId !== r.id) return;
      void api
        .get<WorkflowRun>(`/workflow-runs/${r.id}`)
        .then((nr) => {
          if (untrack(() => run)?.id === nr.id) run = nr;
        })
        .catch(() => {});
    });
  });
  // (2) Guaranteed: a slow safety poll while a viewed run is non-terminal, so it
  //     still converges if a WS event is missed (single-slot bus, or no WS).
  $effect(() => {
    const r = run;
    if (!r || running) return;
    if (r.status !== 'pending' && r.status !== 'running') return;
    const iv = setInterval(() => {
      void api
        .get<WorkflowRun>(`/workflow-runs/${r.id}`)
        .then((nr) => {
          if (untrack(() => run)?.id === r.id) run = nr;
        })
        .catch(() => {});
    }, 2500);
    return () => clearInterval(iv);
  });

  /** Open a run from the "Running" sidebar list: ensure its workflow is open,
   *  then show the run (which the auto-update effects keep live). */
  async function openRunById(workflowId: string, runId: string): Promise<void> {
    try {
      if (current?.id !== workflowId) {
        let wf = workflows.find((w) => w.id === workflowId);
        if (!wf) wf = await api.get<Workflow>(`/workflows/${workflowId}`);
        open(wf); // resets run=null + reloads the workflow's run history
      }
      run = await api.get<WorkflowRun>(`/workflow-runs/${runId}`);
      runsOpen = false;
    } catch (e) {
      toasts.error('Could not open run', e instanceof Error ? e.message : String(e));
    }
  }

  /** Compact "5m ago" for run rows. */
  function ago(iso: string): string {
    const ms = Date.now() - new Date(iso).getTime();
    if (!Number.isFinite(ms) || ms < 0) return '';
    const s = Math.floor(ms / 1000);
    if (s < 60) return `${s}s ago`;
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m ago`;
    return `${Math.floor(m / 60)}h ago`;
  }

  async function load(): Promise<void> {
    try {
      if (types.length === 0) types = await api.get<NodeTypeSpec[]>('/workflows/node-types');
      if (templates.length === 0) templates = await api.get<WorkflowTemplate[]>('/workflows/templates');
      workflows = await api.get<Workflow[]>(`/workspaces/${ws.currentId}/workflows`);
    } catch (e) {
      toasts.error('Failed to load workflows', e instanceof Error ? e.message : String(e));
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
      toasts.error('Could not create from template', e instanceof Error ? e.message : String(e));
    }
  }

  function open(wf: Workflow): void {
    current = wf;
    const g = structuredClone($state.snapshot(wf.graph)) as WorkflowGraph;
    graph = g && g.nodes ? g : { nodes: [], edges: [] };
    selectedId = null;
    selectedEdgeId = null;
    run = null;
    runsOpen = false;
    versionsOpen = false;
    versions = [];
    void loadRuns();
    dirty = false;
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
      toasts.error('Generation failed', e instanceof Error ? e.message : String(e));
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
      toasts.error('Create failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function save(): Promise<void> {
    if (!current) return;
    try {
      const wf = await api.patch<Workflow>(`/workflows/${current.id}`, { graph });
      current = wf;
      workflows = workflows.map((w) => (w.id === wf.id ? wf : w));
      dirty = false;
      toasts.success('Saved');
    } catch (e) {
      toasts.error('Save failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function del(wf: Workflow): Promise<void> {
    try {
      await api.del(`/workflows/${wf.id}`);
      workflows = workflows.filter((w) => w.id !== wf.id);
      if (current?.id === wf.id) {
        current = null;
        graph = { nodes: [], edges: [] };
      }
    } catch (e) {
      toasts.error('Delete failed', e instanceof Error ? e.message : String(e));
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

  // Max number of poll intervals before we stop even if no terminal event
  // arrives. With 700ms intervals this allows ~3.5 minutes of capped polling.
  const POLL_MAX = 300;

  async function execRun(body: Record<string, unknown>): Promise<void> {
    if (!current || running) return;
    if (dirty) await save();
    running = true;
    run = null;
    const workflowId = current.id;
    try {
      let r = await api.post<WorkflowRun>(`/workflows/${workflowId}/run`, body);
      run = r;
      const runId = r.id;
      let pollCount = 0;
      let nextPollMs = 700;

      // Event-driven refresh: workflowRunBus.tick increments each time the
      // server emits WorkflowRunUpdated for any run. We re-fetch when our
      // run_id is the active one. A capped 700ms fallback poll keeps the UI
      // live when the WS connection is unavailable.
      while ((r.status === 'pending' || r.status === 'running') && pollCount < POLL_MAX) {
        // Wait for whichever comes first: a WS event tick or the poll timer.
        const snapshot = workflowRunBus.tick;
        await new Promise<void>((resolve) => {
          const timer = setTimeout(resolve, nextPollMs);
          // Check for a new event tick in a tight loop (rAF-free; negligible CPU).
          const iv = setInterval(() => {
            if (workflowRunBus.tick !== snapshot && workflowRunBus.runId === runId) {
              clearInterval(iv);
              clearTimeout(timer);
              resolve();
            }
          }, 50);
          // Ensure the interval is always cleared when the timer fires.
          setTimeout(() => clearInterval(iv), nextPollMs + 10);
        });
        r = await api.get<WorkflowRun>(`/workflow-runs/${runId}`);
        run = r;
        pollCount += 1;
        // Back off the fallback poll slightly after the first few ticks to
        // reduce load when WS events are driving the refresh. Cap at 3s.
        nextPollMs = Math.min(nextPollMs + 100, 3000);
      }
      if (r.status === 'success') toasts.success('Run complete');
      else if (r.status === 'canceled') toasts.info('Run stopped');
      else toasts.error('Run finished with errors', r.error ?? '');
      void loadRuns();
    } catch (e) {
      toasts.error('Run failed', e instanceof Error ? e.message : String(e));
    } finally {
      running = false;
    }
  }

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
      obj.repo_id = '<repo id — copy it from the Git tab>';
      obj.base = 'main';
    }
    if (k.has('product_analyze') || k.has('product_rewrite') || k.has('product_plan') || k.has('product_publish')) {
      obj.story_id = '<product story id>';
    }
    obj.msg = 'What you want done — instructions for the agents.';
    obj.jira_ticket = 'GS-0000';
    obj.goals = ['e.g. 100% test coverage (services)', 'under 2 minutes runtime'];
    // Optional: post the result somewhere specific (else it replies to the
    // trigger's origin; a manual run posts nowhere unless you set this).
    obj.result_channel = 'slack';
    obj.result_chat = '<channel id — optional>';
    return JSON.stringify(obj, null, 2);
  }

  function parseRunInput(): Record<string, unknown> | undefined | null {
    const t = runInputText.trim();
    if (!t) return undefined;
    try {
      return JSON.parse(t);
    } catch {
      toasts.error('Run input is not valid JSON', 'Fix the JSON or clear the field to run with no input.');
      return null; // signal: invalid
    }
  }

  function openRunInput(): void {
    if (!runInputText.trim()) runInputText = suggestRunInput();
    runInputOpen = !runInputOpen;
  }

  async function confirmRun(): Promise<void> {
    const input = parseRunInput();
    if (input === null) return; // invalid JSON; toast already shown
    runInputOpen = false;
    await execRun(input === undefined ? {} : { input });
  }

  const runFrom = (nodeId: string, only: boolean): Promise<void> => {
    const input = parseRunInput();
    if (input === null) return Promise.resolve();
    return execRun({ start_node: nodeId, only_node: only, ...(input !== undefined ? { input } : {}) });
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
    toasts.success('Tidied layout');
  }

  // Copy-paste Slack message that triggers THIS workflow by name (shown on the
  // Start node inspector + the Triggers panel).
  let mtCopied = $state(false);
  const mtSlackSnippet = $derived(
    `@otto\n` +
      `Action: Workflow\n` +
      `Name: ${current?.name ?? '<workflow name>'}\n` +
      `Msg: what you want done — instructions for the agents\n` +
      `Jira ticket: GS-1234\n` +
      `Working Directory: ~/path/to/repo\n` +
      `Relevant Info: ~/path/a, ~/path/b\n` +
      `Goals:\n  - 100% test coverage (services)\n  - under 2 minutes runtime`,
  );
  async function copyMtSlack(): Promise<void> {
    try {
      await navigator.clipboard.writeText(mtSlackSnippet);
      mtCopied = true;
      setTimeout(() => (mtCopied = false), 1500);
    } catch {
      toasts.error('Copy failed', 'Select the text and copy it manually.');
    }
  }

  async function stop(): Promise<void> {
    if (!run) return;
    try {
      await api.post(`/workflow-runs/${run.id}/cancel`, {});
      toasts.info('Stopping…', 'Finishes the current step, then halts.');
    } catch (e) {
      toasts.error('Stop failed', e instanceof Error ? e.message : String(e));
    }
  }

  async function loadRuns(): Promise<void> {
    if (!current) return;
    try {
      runs = await api.get<WorkflowRun[]>(`/workflows/${current.id}/runs`);
    } catch {
      /* ignore */
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
      toasts.error('Approval failed', e instanceof Error ? e.message : String(e));
    } finally {
      approving = false;
    }
  }

  function nodeName(id: string): string {
    const n = graph.nodes.find((x) => x.id === id);
    return n?.name || n?.kind || id;
  }
  function fmtMs(ms?: number | null): string {
    if (ms == null) return '';
    return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
  }

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
  const REVIEW_PROVIDERS = ['claude', 'codex', 'agy'];

  function reviewers(): ReviewerRow[] {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    return Array.isArray(p?.reviewers) ? (p?.reviewers as ReviewerRow[]) : [];
  }
  function setReviewers(rows: ReviewerRow[]): void {
    onParam('reviewers', rows.length ? rows : undefined);
  }
  function addReviewer(): void {
    setReviewers([...reviewers(), { lens: '', providers: ['claude'] }]);
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
    updateReviewer(i, { providers: next.length ? next : ['claude'] });
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

  // --- Per-node retry policy (writes node.retry, not params) ----------------
  function retryNum(field: 'max_attempts' | 'backoff_ms', def: number): number {
    const r = selectedNode?.retry;
    const v = r ? r[field] : undefined;
    return typeof v === 'number' ? v : def;
  }
  function onRetry(field: 'max_attempts' | 'backoff_ms', value: number): void {
    if (!selectedNode) return;
    const cur = selectedNode.retry ?? { max_attempts: 0, backoff_ms: 0, factor: 2 };
    const next = { ...cur, [field]: Number.isFinite(value) && value > 0 ? value : 0 };
    // Drop the policy entirely when it's a no-op (no extra attempts).
    selectedNode.retry = next.max_attempts > 0 ? next : null;
    graph = graph;
    dirty = true;
  }

  // --- Edge condition editing ----------------------------------------------
  let selectedEdgeId = $state<string | null>(null);
  const selectedEdge = $derived(graph.edges.find((e) => e.id === selectedEdgeId) ?? null);

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

  async function loadVersions(): Promise<void> {
    if (!current) return;
    versionsLoading = true;
    try {
      versions = await listWorkflowVersions(current.id);
    } catch (e) {
      toasts.error('Failed to load versions', e instanceof Error ? e.message : String(e));
    } finally {
      versionsLoading = false;
    }
  }

  async function restoreVersion(v: WorkflowVersion): Promise<void> {
    if (!current) return;
    try {
      const wf = await restoreWorkflowVersion(current.id, v.version);
      current = wf;
      workflows = workflows.map((w) => (w.id === wf.id ? wf : w));
      open(wf);
      await loadVersions();
      toasts.success(`Restored v${v.version}`);
    } catch (e) {
      toasts.error('Restore failed', e instanceof Error ? e.message : String(e));
    }
  }
</script>

<div class="wf">
  <aside class="side">
    <div class="gen">
      <label for="wf-prompt">Describe the flow</label>
      <textarea
        id="wf-prompt"
        bind:value={prompt}
        rows="3"
        placeholder="e.g. Ask an agent to summarize the repo, then POST the summary to our webhook."
        onkeydown={(e) => {
          if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) generate();
        }}
      ></textarea>
      <button class="btn primary full" disabled={generating || prompt.trim() === ''} onclick={generate}>
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
            <Icon name={templatesOpen ? 'arrowUp' : 'arrowDown'} size={12} />
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
                  <span class="tpl-ic"><Icon name={t.icon} size={14} /></span>
                  <span class="tpl-body">
                    <span class="tpl-name">{t.name}</span>
                    <span class="tpl-sub">agent design + engine</span>
                  </span>
                </button>
              {/each}
            </div>
          {/if}
        </div>
      {/if}
    </div>

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
            title={`${r.workflow_name} — ${r.status}`}
          >
            <span class="dot {r.status}"></span>
            <span class="run-name">{r.workflow_name}</span>
            {#if r.waiting_approval}
              <span class="run-badge" title="waiting for approval">⏸</span>
            {/if}
            <span class="grow"></span>
            <span class="run-prog">{r.nodes_done}/{r.nodes_total}</span>
            <span class="run-when">{ago(r.started_at)}</span>
          </button>
        {/each}
      </div>
    {/if}

    <div class="list">
      <div class="list-h">Workflows</div>
      {#each workflows as wf (wf.id)}
        <div class="row" class:active={current?.id === wf.id}>
          <button class="row-main" onclick={() => open(wf)}>
            <Icon name="split" size={13} />
            <span class="row-name">{wf.name}</span>
          </button>
          <button class="row-del" title="Delete" onclick={() => del(wf)}><Icon name="trash" size={12} /></button>
        </div>
      {/each}
      {#if workflows.length === 0}
        <p class="empty">No workflows yet — describe one above.</p>
      {/if}
    </div>
  </aside>

  <main class="main">
    {#if current}
      <header class="bar">
        <span class="wf-title">{current.name}</span>
        {#if dirty}<span class="badge">unsaved</span>{/if}
        <span class="grow"></span>

        <div class="menu-wrap">
          <button class="btn small" onclick={() => (paletteOpen = !paletteOpen)}>
            <Icon name="plus" size={12} /> Node
          </button>
          {#if paletteOpen}
            <div class="palette">
              {#each types as t (t.kind)}
                <button class="pal-item" onclick={() => addNode(t)}>
                  <span class="pal-ic" style="--c:{t.color}"><Icon name={t.icon} size={12} /></span>
                  <span class="pal-body">
                    <span class="pal-name">{t.label}</span>
                    <span class="pal-cat">{t.category}</span>
                  </span>
                </button>
              {/each}
            </div>
          {/if}
        </div>

        {#if selectedId}
          <button class="btn small" onclick={removeSelected}><Icon name="trash" size={12} /></button>
        {/if}
        <button class="btn small" disabled={!dirty} onclick={save}>Save</button>

        <div class="menu-wrap">
          <button class="btn small" onclick={() => { runsOpen = !runsOpen; if (runsOpen) void loadRuns(); }}>
            <Icon name="clock" size={12} /> Runs
          </button>
          {#if runsOpen}
            <div class="palette runs-pop">
              {#if runs.length === 0}<div class="runs-empty">No runs yet</div>{/if}
              {#each runs as r (r.id)}
                <button class="run-item" class:active={run?.id === r.id} onclick={() => { run = r; runsOpen = false; }}>
                  <span class="dot {r.status}"></span>
                  <span class="run-status">{r.status}</span>
                  <span class="run-when">{new Date(r.started_at).toLocaleTimeString()}</span>
                </button>
              {/each}
            </div>
          {/if}
        </div>

        <!-- Triggers config toggle -->
        <button class="btn small" onclick={() => (triggersOpen = !triggersOpen)} title="Configure workflow triggers">
          <Icon name="clock" size={12} /> Triggers
        </button>

        <!-- Tidy: reflow the graph into a few readable rows -->
        <button class="btn small" onclick={tidy} title="Tidy layout into rows">
          <Icon name="grid" size={12} /> Tidy
        </button>

        <!-- Version history toggle -->
        <button
          class="btn small"
          onclick={() => { versionsOpen = !versionsOpen; if (versionsOpen) void loadVersions(); }}
          title="Version history"
        >
          <Icon name="commit" size={12} /> Versions
        </button>

        {#if running}
          <button class="btn small danger" onclick={stop}><Icon name="square" size={11} /> Stop</button>
        {/if}
        <button
          class="btn primary small"
          class:active={runInputOpen}
          disabled={running}
          onclick={openRunInput}
          title="Run — set the input (repo_id / story_id / goals / msg) the trigger emits"
        >
          {#if running}<span class="spin"></span> Running{:else}<Icon name="play" size={12} /> Run…{/if}
        </button>
      </header>

      <!-- Manual-run input editor: this is WHERE you provide the run input the
           trigger emits (repo_id, story_id, goals, msg, jira_ticket, …). -->
      {#if runInputOpen}
        <div class="run-input">
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
            placeholder={'{\n  "repo_id": "…",\n  "goals": ["…"]\n}'}
          ></textarea>
          <div class="ri-actions">
            <button class="btn primary small" disabled={running} onclick={confirmRun}>
              <Icon name="play" size={12} /> Run
            </button>
            <button class="btn small" onclick={() => (runInputOpen = false)}>Cancel</button>
          </div>
        </div>
      {/if}

      <!-- Human-approval banner: shown when a run is paused at a human_approval node -->
      {#if run?.waiting_approval && run.approval_node_id}
        <div class="approval-banner">
          <Icon name="user-check" size={14} />
          <span>Run paused — waiting for approval at <strong>{run.approval_node_id}</strong></span>
          <button class="btn primary small" disabled={approving} onclick={() => approveRun(true)}>
            Approve
          </button>
          <button class="btn small danger" disabled={approving} onclick={() => approveRun(false)}>
            Reject
          </button>
        </div>
      {/if}

      <div class="canvas-wrap">
        <WorkflowCanvas
          bind:graph
          {types}
          {runStates}
          {selectedId}
          {selectedEdgeId}
          onselect={(id) => { selectedId = id; selectedEdgeId = null; }}
          onedgeselect={(id) => { selectedEdgeId = id; if (id) selectedId = null; }}
          onchange={() => (dirty = true)}
        />
      </div>

      {#if triggersOpen && current}
        <div class="triggers-wrap">
          <TriggersPanel
            workflowId={current.id}
            workflowName={current.name}
            bind:triggers
            ontriggers={(ts) => (triggers = ts)}
          />
        </div>
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
          {#if versionsLoading && versions.length === 0}
            <p class="empty">Loading…</p>
          {:else if versions.length === 0}
            <p class="empty">No saved versions yet — edits and restores create them.</p>
          {:else}
            <ul class="versions">
              {#each versions as v (v.id)}
                <li class="ver">
                  <span class="ver-num">v{v.version}</span>
                  <span class="ver-note">{v.note || '(no note)'}</span>
                  <span class="ver-when">{new Date(v.created_at).toLocaleString()}</span>
                  <button class="btn small" onclick={() => restoreVersion(v)}>Restore</button>
                </li>
              {/each}
            </ul>
          {/if}
        </div>
      {/if}

      {#if run || selectedNode || selectedEdge}
        <div class="inspector">
          {#if run}
            <div class="timeline">
              <span class="tl-label"><span class="dot {run.status}"></span>{run.status}</span>
              {#each run.nodes as ns (ns.node_id)}
                <button
                  class="tl-step"
                  class:active={selectedId === ns.node_id}
                  data-status={ns.status}
                  onclick={() => (selectedId = ns.node_id)}
                >
                  <span class="dot {ns.status}"></span>
                  <span class="tl-name">{nodeName(ns.node_id)}</span>
                  {#if ns.duration_ms != null}<span class="tl-ms">{fmtMs(ns.duration_ms)}</span>{/if}
                </button>
              {/each}
            </div>
            <div class="run-detail"><RunSteps {run} nodeName={(id) => nodeName(id)} /></div>
          {/if}

          {#if selectedNode}
            <div class="insp-h">
              <strong>{selectedNode.name || selectedNode.kind}</strong>
              <span class="mono dim">{selectedNode.kind}</span>
              {#if selectedRun}<span class="dot {selectedRun.status}"></span>{selectedRun.status}{/if}
              {#if selectedRun?.duration_ms != null}<span class="dim">· {fmtMs(selectedRun.duration_ms)}</span>{/if}
              <span class="grow"></span>
              <button class="btn small" disabled={running} onclick={() => runFrom(selectedNode.id, false)} title="Run this node and everything downstream">▶ From here</button>
              <button class="btn small" disabled={running} onclick={() => runFrom(selectedNode.id, true)} title="Run only this node">Only this</button>
            </div>
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
                rows="3"
                placeholder="What you want done — instructions for the agents"
                value={paramStr('msg')}
                oninput={(e) => onParam('msg', e.currentTarget.value)}
              ></textarea>
              <label for="mt-wd">Working directory (where agents run)</label>
              <input
                id="mt-wd"
                type="text"
                placeholder="~/path/to/repo (default: workspace root)"
                value={paramStr('working_directory')}
                oninput={(e) => onParam('working_directory', e.currentTarget.value)}
              />
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
                placeholder="GS-1234"
                value={paramStr('jira_ticket')}
                oninput={(e) => onParam('jira_ticket', e.currentTarget.value)}
              />
              <label for="mt-goals">Goals (one per line)</label>
              <textarea
                id="mt-goals"
                rows="3"
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
                rows="4"
                value={paramStr('prompt')}
                oninput={(e) => onParam('prompt', e.currentTarget.value)}
              ></textarea>
              <label for="np-model">Model (optional)</label>
              <input
                id="np-model"
                type="text"
                placeholder="e.g. claude-opus-4-8 (default)"
                value={paramStr('model')}
                oninput={(e) => onParam('model', e.currentTarget.value)}
              />
              <label for="np-skills">Skills (comma-separated)</label>
              <input
                id="np-skills"
                type="text"
                placeholder="e.g. golang-testing, golang-code-review"
                value={paramList('skills')}
                oninput={(e) => onParamList('skills', e.currentTarget.value)}
              />
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
              <label for="np-body">Body (JSON, optional)</label>
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
              <label for="np-json">Merge JSON (object)</label>
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
              <label for="np-provider">Provider</label>
              <select
                id="np-provider"
                value={paramStr('provider') || 'claude'}
                onchange={(e) => onParam('provider', e.currentTarget.value)}
              >
                <option value="claude">Claude</option>
                <option value="codex">Codex</option>
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
                rows="2"
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
              <label for="np-body">Body (JSON, optional)</label>
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
              <label for="np-steps">Steps (JSON array)</label>
              <textarea
                id="np-steps"
                rows="5"
                placeholder={'[ { "kind": "agent_prompt", "params": {} } ]'}
                value={paramJson('steps')}
                oninput={(e) => onParamJson('steps', e.currentTarget.value)}
              ></textarea>
              <label class="np-chk">
                <input
                  type="checkbox"
                  checked={paramBool('continue_on_error')}
                  onchange={(e) => onParam('continue_on_error', e.currentTarget.checked)}
                /> Continue on step error
              </label>
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
              <div class="rv-h">
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
                    <button class="rv-del" type="button" title="Remove reviewer" onclick={() => removeReviewer(i)}>
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
                    class="rv-instr"
                    rows="2"
                    placeholder="custom instructions for this reviewer (optional)"
                    value={r.instructions ?? ''}
                    oninput={(e) => updateReviewer(i, { instructions: e.currentTarget.value })}
                  ></textarea>
                </div>
              {/each}

              <label for="np-sum-prov">Summarizer (consolidates + scores)</label>
              <input
                id="np-sum-prov"
                type="text"
                placeholder="provider (e.g. claude)"
                value={summarizerField('provider')}
                oninput={(e) => updateSummarizer('provider', e.currentTarget.value)}
              />
              <textarea
                rows="2"
                placeholder="summarizer instructions (optional)"
                value={summarizerField('instructions')}
                oninput={(e) => updateSummarizer('instructions', e.currentTarget.value)}
              ></textarea>

              <span class="np-label">Scoring guideline — % deducted per open finding</span>
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
              <label for="np-goals">Goals (one per line, optional)</label>
              <textarea
                id="np-goals"
                rows="3"
                placeholder={'No N+1 queries\nAll inputs validated'}
                value={paramLines('goals')}
                oninput={(e) => onParamLines('goals', e.currentTarget.value)}
              ></textarea>
              <label for="np-checks">Checks — commands the reviewer runs (one per line, optional)</label>
              <textarea
                id="np-checks"
                rows="3"
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
                rows="3"
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
                trigger's chat thread. No parameters.
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

            <!-- Retry policy (any node): extra attempts with exponential backoff. -->
            {#if selectedNode.kind !== 'manual_trigger'}
              <div class="retry-form">
                <span class="retry-h">Retry</span>
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
              <button class="btn small danger" title="Delete connection" onclick={removeSelectedEdge}>
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
          {:else}
            <p class="empty">Select a step or connection above to edit it.</p>
          {/if}
        </div>
      {/if}
    {:else}
      <div class="placeholder">
        <Icon name="split" size={40} />
        <h2>Build a workflow</h2>
        <p>Describe what you want on the left and we’ll wire it up — or start blank and drag nodes.</p>
      </div>
    {/if}
  </main>
</div>

<style>
  .wf {
    display: flex;
    height: 100%;
    min-height: 0;
  }
  .side {
    width: 270px;
    flex-shrink: 0;
    border-inline-end: 1px solid var(--border);
    display: flex;
    flex-direction: column;
    background: var(--surface);
    min-height: 0;
  }
  .gen {
    padding: 12px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    border-bottom: 1px solid var(--border);
  }
  .gen label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  textarea {
    width: 100%;
    resize: vertical;
    font: inherit;
    font-size: 12.5px;
    line-height: 1.45;
    padding: 7px 9px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
    background: var(--surface-2);
    color: var(--text);
  }
  textarea:focus {
    outline: none;
    border-color: var(--accent);
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
    background: color-mix(in srgb, var(--accent) 10%, transparent);
  }
  .tpl-ic {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: 7px;
    background: color-mix(in srgb, var(--accent) 16%, transparent);
    color: var(--accent);
    flex-shrink: 0;
  }
  .tpl-body {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }
  .tpl-name {
    font-size: 12.5px;
    font-weight: 600;
    color: var(--text);
  }
  .tpl-sub {
    font-size: 10.5px;
    color: var(--text-dim);
  }
  .list {
    flex: 1;
    overflow-y: auto;
    min-height: 0;
    padding: 8px;
  }
  .list-h {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 4px 6px;
  }
  .row {
    display: flex;
    align-items: center;
    border-radius: var(--radius-s);
  }
  .row.active {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
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
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row-del {
    background: none;
    border: none;
    color: var(--text-dim);
    cursor: pointer;
    padding: 6px;
    opacity: 0;
  }
  .row:hover .row-del {
    opacity: 1;
  }
  .row-del:hover {
    color: var(--status-exited);
  }
  .empty {
    font-size: 12px;
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
    font-size: 10px;
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
  .run-row:hover,
  .run-row.active {
    background: color-mix(in srgb, var(--accent) 12%, transparent);
  }
  .run-name {
    font-size: 12.5px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    max-width: 120px;
  }
  .run-badge {
    font-size: 11px;
  }
  .run-prog {
    font-size: 10.5px;
    color: var(--text-dim);
    font-variant-numeric: tabular-nums;
  }
  .run-when {
    font-size: 10px;
    color: var(--text-dim);
    margin-inline-start: 6px;
  }
  .main {
    flex: 1;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    border-bottom: 1px solid var(--border);
    background: var(--surface);
  }
  .wf-title {
    font-size: 13px;
    font-weight: 600;
  }
  .badge {
    font-size: 10px;
    color: var(--accent);
    background: color-mix(in srgb, var(--accent) 14%, transparent);
    padding: 1px 7px;
    border-radius: 99px;
  }
  .grow {
    flex: 1;
  }
  .menu-wrap {
    position: relative;
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
    font-size: 12px;
    font-weight: 600;
  }
  .pal-cat {
    font-size: 10px;
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
    max-height: 38%;
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .insp-note {
    margin: 0;
    font-size: 11.5px;
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
    background: var(--bg, #0d0f13);
    border: 1px solid var(--border);
    border-radius: 6px;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 11px;
    line-height: 1.5;
    white-space: pre-wrap;
    color: var(--text);
    overflow-x: auto;
  }
  .insp-h {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 12.5px;
  }
  .insp-h .dim {
    color: var(--text-dim);
    font-size: 11px;
  }
  .inspector label {
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
  }
  .np-label {
    display: block;
    font-size: 11px;
    font-weight: 600;
    color: var(--text-dim);
  }
  .inspector input[type='text'],
  .inspector input[type='url'],
  .inspector input[type='number'] {
    width: 100%;
    font: inherit;
    font-size: 12.5px;
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
  }
  .inspector select {
    width: 100%;
    font: inherit;
    font-size: 12.5px;
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
  }
  .err {
    color: var(--status-exited);
    font-size: 11.5px;
    background: color-mix(in srgb, var(--status-exited) 10%, transparent);
    padding: 6px 8px;
    border-radius: var(--radius-s);
  }
  .logs {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text-dim);
  }
  .out {
    font-size: 11px;
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
  }
  .run-detail {
    margin: 8px 0;
  }
  .tl-label {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-dim);
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
    font-size: 11.5px;
    cursor: pointer;
  }
  .tl-step.active {
    border-color: var(--accent);
  }
  .tl-step[data-status='success'] {
    border-color: color-mix(in srgb, var(--status-working, #28c840) 55%, var(--border));
  }
  .tl-step[data-status='error'] {
    border-color: color-mix(in srgb, var(--status-exited) 55%, var(--border));
  }
  .tl-step[data-status='running'] {
    border-color: var(--accent);
  }
  .tl-name {
    white-space: nowrap;
  }
  .tl-ms {
    font-size: 10px;
    color: var(--text-dim);
    font-family: var(--font-mono);
  }

  /* Runs history popover */
  .runs-pop {
    width: 200px;
  }
  .runs-empty {
    font-size: 12px;
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
    font-size: 12px;
    text-transform: capitalize;
  }
  .run-when {
    font-size: 10.5px;
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
  .dot.success {
    background: var(--status-working, #28c840);
  }
  .dot.error {
    background: var(--status-exited);
  }
  .dot.running {
    background: var(--status-working, #28c840);
  }
  .dot.pending,
  .dot.skipped {
    background: var(--text-dim);
  }
  .placeholder {
    margin: auto;
    text-align: center;
    color: var(--text-dim);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }
  .placeholder h2 {
    margin: 8px 0 0;
    font-size: 16px;
    color: var(--text);
  }
  .placeholder p {
    font-size: 12.5px;
    max-width: 340px;
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
  /* Human-approval banner */
  .run-input {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px;
    background: var(--panel, rgba(255, 255, 255, 0.03));
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
    font-size: 11.5px;
    color: var(--text-dim, #9aa0aa);
  }
  .ri-text {
    width: 100%;
    resize: vertical;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: 6px;
    background: var(--bg, #0d0f13);
    color: var(--text);
    font-size: 12px;
    line-height: 1.5;
  }
  .ri-actions {
    display: flex;
    gap: 8px;
  }
  .mono {
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
  }
  .approval-banner {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: var(--warn-bg, rgba(240, 192, 64, 0.12));
    border-bottom: 1px solid var(--border);
    font-size: 12.5px;
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
    font-size: 11.5px;
    color: var(--text-dim);
    margin: 2px 0 0;
  }
  /* Inspector checkbox row */
  .np-chk {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
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
  .rv-row {
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    padding: 6px;
    margin-bottom: 6px;
    display: flex;
    flex-direction: column;
    gap: 5px;
  }
  .rv-top {
    display: flex;
    gap: 6px;
    align-items: center;
  }
  .rv-lens {
    flex: 1;
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
    font-size: 11px;
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
    font-size: 11.5px;
  }
  .rv-score {
    display: flex;
    gap: 8px;
  }
  .rv-sc {
    display: flex;
    flex-direction: column;
    gap: 2px;
    font-size: 11px;
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
    font-size: 10.5px;
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
    font-size: 11px;
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
    font-size: 12px;
  }
  .ver-num {
    font-family: var(--font-mono);
    color: var(--accent);
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
    font-size: 10.5px;
    color: var(--text-dim);
    flex-shrink: 0;
  }
</style>
