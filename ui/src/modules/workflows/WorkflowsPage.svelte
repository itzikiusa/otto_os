<script lang="ts">
  // Workflows: build automations by *describing* them (agent mode) or by hand
  // on the canvas. Left = generate + list; center = node-graph editor + run.
  import Icon from '../../lib/components/Icon.svelte';
  import WorkflowCanvas from './WorkflowCanvas.svelte';
  import RunSteps from './RunSteps.svelte';
  import TriggersPanel from './TriggersPanel.svelte';
  import { ws } from '../../lib/stores/workspace.svelte';
  import { toasts } from '../../lib/toast.svelte';
  import { api } from '../../lib/api/client';
  import { workflowRunBus } from '../../lib/events.svelte';
  import type {
    Workflow,
    WorkflowGraph,
    WorkflowRun,
    NodeTypeSpec,
    NodeRunState,
    WorkflowTemplate,
    WorkflowTrigger,
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
  let triggersOpen = $state(false);
  let triggers = $state<WorkflowTrigger[]>([]);
  let approving = $state(false);

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

        <!-- Triggers config toggle -->
        <button class="btn small" onclick={() => (triggersOpen = !triggersOpen)} title="Configure workflow triggers">
          <Icon name="clock" size={12} /> Triggers
        </button>

        {#if running}
          <button class="btn small danger" onclick={stop}><Icon name="square" size={11} /> Stop</button>
        {/if}
        <button class="btn primary small" disabled={running} onclick={runWorkflow}>
          {#if running}<span class="spin"></span> Running{:else}<Icon name="play" size={12} /> Run{/if}
        </button>
      </header>

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
          onselect={(id) => (selectedId = id)}
          onchange={() => (dirty = true)}
        />
      </div>

      {#if triggersOpen && current}
        <div class="triggers-wrap">
          <TriggersPanel
            workflowId={current.id}
            bind:triggers
            ontriggers={(ts) => (triggers = ts)}
          />
        </div>
      {/if}

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
            <!-- Per-kind param forms. Each kind exposes only its meaningful
                 params; unrecognised kinds fall through to a raw JSON editor. -->
            {#if selectedNode.kind === 'agent_prompt'}
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
            {:else if selectedNode.kind === 'product_analyze' || selectedNode.kind === 'product_rewrite' || selectedNode.kind === 'product_plan' || selectedNode.kind === 'review_run'}
              <p class="node-hint stub">
                <Icon name="alert-circle" size={12} />
                This node kind is registered but not yet wired in the engine. It will
                run as a stub and pass a "not wired" marker to downstream nodes.
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
  /* Triggers panel: collapsible section below the canvas */
  .triggers-wrap {
    border-top: 1px solid var(--border);
    max-height: 320px;
    overflow-y: auto;
  }
  /* Human-approval banner */
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
  .node-hint.stub {
    display: flex;
    align-items: center;
    gap: 5px;
    color: var(--warn, #b07a00);
  }
</style>
