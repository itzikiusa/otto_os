<script lang="ts">
  // Workflows: build automations by *describing* them (agent mode) or by hand
  // on the canvas. Left = generate + list; center = node-graph editor + run.
  import Icon from '../../lib/components/Icon.svelte';
  import WorkflowCanvas from './WorkflowCanvas.svelte';
  import RunSteps from './RunSteps.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { api } from '../../lib/api/client';
  import type {
    Workflow,
    WorkflowGraph,
    WorkflowRun,
    NodeTypeSpec,
    NodeRunState,
    WorkflowTemplate,
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
  let paletteOpen = $state(false);

  const runStates = $derived<Record<string, NodeRunState>>(
    Object.fromEntries((run?.nodes ?? []).map((n) => [n.node_id, n])),
  );
  const selectedNode = $derived(graph.nodes.find((n) => n.id === selectedId) ?? null);
  const selectedRun = $derived(selectedId ? (runStates[selectedId] ?? null) : null);

  $effect(() => {
    if (ws.currentId) void load();
  });

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
    run = null;
    runsOpen = false;
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

  async function execRun(body: Record<string, unknown>): Promise<void> {
    if (!current || running) return;
    if (dirty) await save();
    running = true;
    run = null;
    try {
      let r = await api.post<WorkflowRun>(`/workflows/${current.id}/run`, body);
      run = r;
      while (r.status === 'pending' || r.status === 'running') {
        await new Promise((res) => setTimeout(res, 700));
        r = await api.get<WorkflowRun>(`/workflow-runs/${r.id}`);
        run = r;
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

  const runWorkflow = (): Promise<void> => execRun({});
  const runFrom = (nodeId: string, only: boolean): Promise<void> =>
    execRun({ start_node: nodeId, only_node: only });

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

  function nodeName(id: string): string {
    const n = graph.nodes.find((x) => x.id === id);
    return n?.name || n?.kind || id;
  }
  function fmtMs(ms?: number | null): string {
    if (ms == null) return '';
    return ms >= 1000 ? `${(ms / 1000).toFixed(1)}s` : `${ms}ms`;
  }

  function onParam(field: string, value: string): void {
    if (!selectedNode) return;
    const params = { ...((selectedNode.params as Record<string, unknown>) ?? {}) };
    params[field] = value;
    selectedNode.params = params;
    graph = graph;
    dirty = true;
  }

  function promptParam(): string {
    const p = selectedNode?.params as Record<string, unknown> | undefined;
    return typeof p?.prompt === 'string' ? p.prompt : '';
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
    </div>

    {#if templates.length > 0}
      <div class="templates">
        <div class="list-h">Game templates</div>
        {#each templates as t (t.id)}
          <button class="tpl" onclick={() => fromTemplate(t)} title={t.description}>
            <span class="tpl-ic"><Icon name={t.icon} size={14} /></span>
            <span class="tpl-body">
              <span class="tpl-name">{t.name}</span>
              <span class="tpl-sub">agent design + engine</span>
            </span>
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

        {#if running}
          <button class="btn small danger" onclick={stop}><Icon name="square" size={11} /> Stop</button>
        {/if}
        <button class="btn primary small" disabled={running} onclick={runWorkflow}>
          {#if running}<span class="spin"></span> Running{:else}<Icon name="play" size={12} /> Run{/if}
        </button>
      </header>

      <div class="canvas-wrap">
        <WorkflowCanvas
          bind:graph
          {types}
          {runStates}
          {selectedId}
          onselect={(id) => (selectedId = id)}
          onchange={() => (dirty = true)}
        />
      </div>

      {#if run || selectedNode}
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
            {#if selectedNode.kind === 'agent_prompt'}
              <label for="np-prompt">Prompt</label>
              <textarea id="np-prompt" rows="3" value={promptParam()} oninput={(e) => onParam('prompt', e.currentTarget.value)}></textarea>
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
          {:else}
            <p class="empty">Select a step above to see its logs and output.</p>
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
    border-right: 1px solid var(--border);
    display: flex;
    flex-direction: column;
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
  .templates {
    padding: 8px;
    border-bottom: 1px solid var(--border);
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
    text-align: left;
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
    text-align: left;
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
    right: 0;
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
    text-align: left;
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
    padding-right: 4px;
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
    text-align: left;
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
    border-right-color: transparent;
    border-radius: 50%;
    display: inline-block;
    animation: rot 0.7s linear infinite;
  }
  @keyframes rot {
    to {
      transform: rotate(360deg);
    }
  }
</style>
