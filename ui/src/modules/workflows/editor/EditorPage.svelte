<script lang="ts">
  // One workflow's editor (ux.md §9; a lazy chunk): the canvas drawn top to
  // bottom and fitted on open, the inspector in a 360 px column (a bottom
  // sheet over a read-only outline on a phone), Instructions / Triggers /
  // Versions in that column from ⋯, one Save (⌘S, "Unsaved", the version
  // lock) and Run… (the start form). Validation is one line above the canvas.
  import { untrack } from 'svelte';
  import PageHeader from '../../../lib/components/PageHeader.svelte';
  import PageBody from '../../../lib/components/PageBody.svelte';
  import LoadState from '../../../lib/components/LoadState.svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { api } from '../../../lib/api/client';
  import { listWorkflowVersions, patchWorkflow, restoreWorkflowVersion } from '../../../lib/api/workflows';
  import type { NodeTypeSpec, Workflow, WorkflowGraph, WorkflowNode, WorkflowTrigger, WorkflowValidationIssue, WorkflowVersionSummary } from '../../../lib/api/types';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { guardUnsaved } from '../../../lib/leaveGuard';
  import { loadErrorText } from '../../../lib/loadError';
  import { defaultAgentProvider } from '../../../lib/providers';
  import { router } from '../../../lib/router.svelte';
  import { rel } from '../../../lib/stores/now.svelte';
  import { viewport } from '../../../lib/stores/viewport.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { toastError } from '../../../lib/toastError';
  import { nextEdgeId } from '../canvasGeometry';
  import TriggersPanel from '../TriggersPanel.svelte';
  import StepInspector from '../inspector/StepInspector.svelte';
  import { InspectorOptions } from '../inspector/options.svelte';
  import { withParam } from '../inspector/util';
  import Modal from '../../../lib/components/Modal.svelte';
  import StartRunModal from '../runs/StartRunModal.svelte';
  import { workflowsUi } from '../workflowsUiState.svelte';
  import { deleteWorkflow, duplicateWorkflow, renameWorkflow } from '../list/workflowActions';
  import { autoLayout } from './autoLayout';
  import EditorCanvas from './EditorCanvas.svelte';
  import PhoneOutline from './PhoneOutline.svelte';
  import { continueClashes, missingSkills, stepProvider, wireAfter } from './stepRules';

  interface Props {
    workflowId: string;
    /** "Runs of this workflow": the Runs tab, filtered. */
    onrunsof?: (workflowId: string) => void;
    ondeleted?: () => void;
  }
  let { workflowId, onrunsof, ondeleted }: Props = $props();

  let wf = $state<Workflow | null>(null);
  let graph = $state<WorkflowGraph>({ nodes: [], edges: [] });
  let types = $state<NodeTypeSpec[]>([]);
  let triggers = $state<WorkflowTrigger[]>([]);
  let loadError = $state<string | null>(null);
  let dirty = $state(false);
  let selectedId = $state<string | null>(null);
  let selectedEdgeId = $state<string | null>(null);
  let issues = $state<WorkflowValidationIssue[]>([]);
  let blockers = $state<string[]>([]);
  let showProblems = $state(false);
  let panel = $state<'instructions' | 'triggers' | 'versions' | null>(null);
  let starting = $state(false);
  let saving = $state(false);
  let instructions = $state('');
  /** "Open the prompt in a larger editor": one text param of the selected step. */
  let zoom = $state<{ field: string; label: string } | null>(null);
  const pickers = new InspectorOptions();
  const defaultProvider = $derived(defaultAgentProvider());
  const phone = $derived(viewport.isPhone);

  async function load(): Promise<void> {
    loadError = null;
    try {
      const [w, t, tr] = await Promise.all([
        api.get<Workflow>(`/workflows/${encodeURIComponent(workflowId)}`),
        api.get<NodeTypeSpec[]>('/workflows/node-types'),
        api.get<WorkflowTrigger[]>(`/workflows/${encodeURIComponent(workflowId)}/triggers`).catch(() => []),
      ]);
      apply(w);
      types = t;
      triggers = tr;
    } catch (e) {
      loadError = loadErrorText(e);
    }
  }
  function apply(w: Workflow): void {
    wf = w;
    graph = structuredClone(w.graph);
    instructions = w.instructions ?? '';
    dirty = false;
    issues = [];
    blockers = [];
  }
  $effect(() => {
    void workflowId;
    untrack(() => void load());
  });
  $effect(() => guardUnsaved(() => dirty, { what: wf ? `“${wf.name}”` : 'this workflow' }));

  const selected = $derived(selectedId ? graph.nodes.find((n) => n.id === selectedId) ?? null : null);
  const zoomText = $derived(zoom && selected ? String((selected.params as Record<string, unknown> | null)?.[zoom.field] ?? '') : '');
  const invalid = $derived(Object.fromEntries(issues.filter((i) => i.node_id).map((i) => [i.node_id!, i.message])));
  const problems = $derived([...blockers, ...issues.map((i) => i.message)]);

  function edit(mutate: (n: WorkflowNode) => void): void {
    if (!selected) return;
    mutate(selected);
    dirty = true;
  }
  function connect(source: string, target: string): void {
    if (source === target || graph.edges.some((e) => e.source === source && e.target === target)) return;
    graph.edges = [...graph.edges, { id: nextEdgeId(graph.edges, source, target), source, target }];
    dirty = true;
  }
  function deleteNode(id: string): void {
    graph.nodes = graph.nodes.filter((n) => n.id !== id);
    graph.edges = graph.edges.filter((e) => e.source !== id && e.target !== id);
    if (selectedId === id) selectedId = null;
    dirty = true;
  }
  function deleteEdge(id: string): void {
    graph.edges = graph.edges.filter((e) => e.id !== id);
    if (selectedEdgeId === id) selectedEdgeId = null;
    dirty = true;
  }
  /** "+ Step": after the selected step (else the last one), wired in. */
  function addStep(t: NodeTypeSpec): void {
    const id = `${t.kind}-${Math.random().toString(36).slice(2, 7)}`;
    const layers = autoLayout(graph).layers;
    const after = selectedId ?? layers.at(-1)?.at(-1) ?? null;
    graph.nodes = [...graph.nodes, { id, kind: t.kind, name: t.label, x: 0, y: 0, params: {} }];
    const edges: { id: string }[] = [...graph.edges];
    graph.edges = wireAfter(graph.edges, after, id, (s, tg) => {
      const eid = nextEdgeId(edges, s, tg);
      edges.push({ id: eid });
      return eid;
    });
    selectedId = id;
    selectedEdgeId = null;
    dirty = true;
  }
  function stepMenu(e: MouseEvent): void {
    const items: MenuItem[] = types.filter((t) => t.kind !== 'manual_trigger').map((t) => ({ label: t.label, hint: t.category, title: t.description, action: () => addStep(t) }));
    ctxMenu.showAt(e.currentTarget as HTMLElement, items, { filter: true, filterPlaceholder: 'Find a step…' });
  }

  /** What blocks a save on this side: continuers that clash, skills not installed. */
  function localBlockers(): string[] {
    const out = continueClashes(graph);
    for (const n of graph.nodes) {
      const skills = Array.isArray((n.params as { skills?: unknown } | null)?.skills) ? ((n.params as { skills: string[] }).skills) : [];
      if (!skills.length) continue;
      const p = stepProvider(n, defaultProvider);
      const rows = pickers.skills.rows(p);
      for (const s of missingSkills(skills, rows ? new Set(rows.map((r) => r.name)) : null)) out.push(`${n.name || n.id}: ${s} isn’t installed for ${p}.`);
    }
    return out;
  }
  async function save(): Promise<boolean> {
    if (!wf || saving) return false;
    const id = wf.id;
    blockers = localBlockers();
    if (blockers.length) {
      showProblems = true;
      toasts.error('Not saved — resolve the problems above the canvas');
      return false;
    }
    saving = true;
    try {
      const check = await api.post<{ valid: boolean; issues: WorkflowValidationIssue[] }>(`/workflows/${id}/validate`, { graph });
      issues = check.issues ?? [];
      if (!check.valid) {
        showProblems = true;
        toasts.error('Not saved — resolve the problems above the canvas');
        return false;
      }
      const submitted = $state.snapshot(graph);
      const ok = await workflowsUi.saveGraph(id, submitted, wf.version, (w) => { wf = w; }, apply);
      if (ok && JSON.stringify($state.snapshot(graph)) === JSON.stringify(submitted)) dirty = false;
      if (ok) toasts.success('Saved');
      return ok;
    } catch (e) {
      toastError('Couldn’t save the workflow', e);
      return false;
    } finally {
      saving = false;
    }
  }
  function onKey(e: KeyboardEvent): void {
    if ((e.metaKey || e.ctrlKey) && e.key === 's') {
      e.preventDefault();
      void save();
    }
  }

  async function saveInstructions(): Promise<void> {
    if (!wf) return;
    try {
      wf = await patchWorkflow(wf.id, { instructions, expected_version: wf.version });
      toasts.success('Instructions saved');
    } catch (e) {
      toastError('Couldn’t save the instructions', e);
    }
  }
  let versions = $state<WorkflowVersionSummary[] | null>(null);
  let versionsError = $state<string | null>(null);
  let versionsMore = $state(false);
  const VERSION_PAGE = 30;
  /** Versions, newest first, a page at a time: `before` ("Show older")
   *  appends the page below that version (an exclusive cursor, so a save
   *  made meanwhile never shifts it). */
  async function openVersions(before?: number): Promise<void> {
    panel = 'versions';
    versionsError = null;
    try {
      const page = await listWorkflowVersions(workflowId, { limit: VERSION_PAGE, beforeVersion: before });
      versions = before ? [...(versions ?? []), ...page] : page;
      versionsMore = page.length === VERSION_PAGE;
    } catch (e) {
      versionsError = loadErrorText(e);
    }
  }
  async function restore(v: number): Promise<void> {
    const id = workflowId; // the version the person confirms, whatever opens meanwhile
    if (!(await confirmer.ask(`The workflow goes back to version ${v}, saved as a new version. The current one stays in Versions.`, { title: `Restore version ${v}?`, confirmLabel: 'Restore' }))) return;
    try {
      const w = await restoreWorkflowVersion(id, v);
      if (id === workflowId) {
        apply(w);
        void openVersions();
      }
    } catch (e) {
      toastError('Couldn’t restore that version', e);
    }
  }
  async function rename(): Promise<void> {
    const w = wf && (await renameWorkflow(wf));
    if (w && wf) wf = { ...wf, name: w.name, version: w.version ?? wf.version };
  }
  async function duplicate(): Promise<void> {
    const copy = wf && (await duplicateWorkflow(wf.workspace_id, wf));
    if (copy) router.go(`workflows/${copy.id}`);
  }
  const more = (): MenuItem[] => [
    { label: 'Runs of this workflow', icon: 'clock', action: () => onrunsof?.(workflowId) },
    { label: 'Optimize…', icon: 'zap', action: () => router.go(`workflows/${workflowId}/optimize`) },
    { separator: true },
    { label: 'Triggers', checked: panel === 'triggers', action: () => (panel = panel === 'triggers' ? null : 'triggers') },
    { label: 'Instructions', checked: panel === 'instructions', action: () => (panel = panel === 'instructions' ? null : 'instructions') },
    { label: 'Versions', checked: panel === 'versions', action: () => (panel === 'versions' ? (panel = null) : void openVersions()) },
    { separator: true },
    { label: 'Validate', action: () => void validate() },
    { label: 'Rename…', action: () => void rename() },
    { label: 'Duplicate', icon: 'copy', action: () => void duplicate() },
    { separator: true },
    { label: 'Delete workflow…', icon: 'trash', danger: true, action: async () => { if (wf && (await deleteWorkflow(wf))) { dirty = false; ondeleted?.(); } } },
  ];
  async function validate(): Promise<void> {
    blockers = localBlockers();
    try {
      issues = (await api.post<{ valid: boolean; issues: WorkflowValidationIssue[] }>(`/workflows/${workflowId}/validate`, { graph })).issues ?? [];
      if (!problems.length) toasts.success('No problems found');
      else showProblems = true;
    } catch (e) {
      toastError('Couldn’t validate the workflow', e);
    }
  }

  // Phone: a step sheet edits a copy until Done saves it.
  let phoneBefore: WorkflowGraph | null = null;
  function openSheet(id: string): void {
    phoneBefore = $state.snapshot(graph);
    selectedId = id;
  }
  function cancelSheet(): void {
    if (phoneBefore) graph = phoneBefore;
    phoneBefore = null;
    selectedId = null;
    dirty = false;
  }
  async function doneSheet(): Promise<void> {
    if (!dirty || (await save())) {
      phoneBefore = null;
      selectedId = null;
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<PageHeader title={wf?.name ?? 'Workflow'} crumbs={phone ? [] : [{ label: 'Workflows', onclick: () => router.go('workflows/all') }]} {more}>
  {#snippet leading()}
    {#if phone}
      <button type="button" class="icon-btn" aria-label="Back to workflows" title="Back to workflows" onclick={() => router.go('workflows/all')}><Icon name="chevronLeft" size={16} /></button>
    {/if}
  {/snippet}
  {#snippet badge()}{#if dirty}<span class="ed-chip">Unsaved</span>{/if}{/snippet}
  {#snippet actions()}
    {#if !phone}
      <button type="button" class="btn" aria-haspopup="menu" disabled={!wf} onclick={stepMenu}><Icon name="plus" size={12} /> Step</button>
      <button type="button" class="btn" disabled={!wf || saving} onclick={() => void save()}>{saving ? 'Saving…' : 'Save'}</button>
    {/if}
    <button type="button" class="btn primary" disabled={!wf} onclick={() => (starting = true)}><Icon name="play" size={12} /> Run…</button>
  {/snippet}
</PageHeader>
<PageBody fill padded={false}>
  {#if !wf}
    <LoadState what="the workflow" variant="page" loading={!loadError} error={loadError} onretry={() => void load()} />
  {:else if phone}
    <PhoneOutline {graph} {triggers} {defaultProvider} openId={selectedId} onopen={openSheet} oncancel={cancelSheet} ondone={() => void doneSheet()} {saving}>
      {#snippet sheet(id)}
        {@const n = graph.nodes.find((x) => x.id === id)}
        {#if n}<StepInspector node={n} {graph} {types} wsId={wf?.workspace_id ?? ''} {pickers} {defaultProvider} workflowName={wf?.name} onedit={(m) => { m(n); dirty = true; }} onzoom={(field, label) => (zoom = { field, label })} />{/if}
      {/snippet}
    </PhoneOutline>
  {:else}
    <div class="ed">
      <div class="ed-main">
        {#if problems.length}
          <div class="ed-problems">
            <Icon name="warning" size={12} />{problems.length} {problems.length === 1 ? 'problem' : 'problems'} ·
            <button type="button" class="ed-link" aria-expanded={showProblems} onclick={() => (showProblems = !showProblems)}>{showProblems ? 'Hide' : 'Show'}</button>
            {#if showProblems}<ul>{#each problems as p, i (i)}<li>{p}</li>{/each}</ul>{/if}
          </div>
        {/if}
        <EditorCanvas {graph} {selectedId} {selectedEdgeId} {invalid} {triggers} {defaultProvider}
          onselect={(id) => { selectedId = id; if (id) panel = null; }} onedgeselect={(id) => (selectedEdgeId = id)}
          onconnect={connect} ondeletenode={deleteNode} ondeleteedge={deleteEdge} />
      </div>
      <aside class="ed-side" aria-label="Inspector">
        {#if panel}
          <button type="button" class="btn ghost small ed-back" onclick={() => (panel = null)}><Icon name="chevronLeft" size={12} /> {selected ? selected.name : 'Workflow'}</button>
        {/if}
        {#if panel === 'triggers'}
          <TriggersPanel {workflowId} workflowName={wf.name} bind:triggers />
        {:else if panel === 'instructions'}
          <h2 class="ed-h">Instructions</h2>
          <p class="node-hint">Every run and every step follows these.</p>
          <textarea dir="auto" class="ed-instr" rows="14" aria-label="Instructions" bind:value={instructions}></textarea>
          <button type="button" class="btn small" disabled={instructions === (wf.instructions ?? '')} onclick={() => void saveInstructions()}>Save instructions</button>
        {:else if panel === 'versions'}
          <h2 class="ed-h">Versions</h2>
          <LoadState what="the versions" variant="compact" loading={!versions && !versionsError} error={versionsError} onretry={() => void openVersions()}>
            <ol class="ed-versions">
              {#each versions ?? [] as v (v.id)}
                <li><span>Version {v.version}{v.note ? ` · ${v.note}` : ''}<span class="node-hint"> · {rel(v.created_at)}</span></span>
                  {#if v.version !== wf.version}<button type="button" class="btn small ghost" onclick={() => void restore(v.version)}>Restore</button>{/if}</li>
              {/each}
            </ol>
            {#if versionsMore && versions?.length}
              <button type="button" class="btn small ghost" onclick={() => void openVersions(versions!.at(-1)!.version)}>Show older</button>
            {/if}
          </LoadState>
        {:else if selected}
          {#key selected.id}
            <StepInspector node={selected} {graph} {types} wsId={wf.workspace_id} {pickers} {defaultProvider} workflowName={wf.name} onedit={edit} onzoom={(field, label) => (zoom = { field, label })} />
          {/key}
        {:else}
          <h2 class="ed-h">{wf.name}</h2>
          {#if wf.description}<p class="ed-desc">{wf.description}</p>{/if}
          <p class="node-hint">{graph.nodes.length} steps · version {wf.version ?? 1}{triggers.length ? ` · ${triggers.length} ${triggers.length === 1 ? 'trigger' : 'triggers'}` : ''}</p>
          {#if wf.instructions}<p class="ed-desc ed-clip">{wf.instructions}</p>{/if}
          <p class="node-hint">Select a step to change it. Instructions, Triggers and Versions are in ⋯.</p>
        {/if}
      </aside>
    </div>
  {/if}
</PageBody>

{#if zoom && selected}
  <Modal title={`${zoom.label} · ${selected.name}`} width={900} onclose={() => (zoom = null)}>
    <textarea dir="auto" class="ed-instr" rows="28" aria-label={zoom.label} value={zoomText}
      oninput={(e) => { const v = e.currentTarget.value; const f = zoom!.field; edit((n) => (n.params = withParam(n.params, f, v))); }}></textarea>
  </Modal>
{/if}

{#if starting && wf}
  <StartRunModal workflow={{ ...wf, graph }} beforeRun={async () => !dirty || (await save())} onclose={() => (starting = false)} />
{/if}

<style>
  .ed {
    display: flex;
    flex: 1;
    min-block-size: 0;
    block-size: 100%;
  }
  .ed-main {
    position: relative;
    display: flex;
    flex-direction: column;
    flex: 1;
    min-inline-size: 0;
  }
  .ed-side {
    display: flex;
    flex-direction: column;
    gap: var(--sp-2);
    flex: none;
    inline-size: 360px;
    padding: var(--sp-6);
    border-inline-start: 1px solid var(--border);
    background: var(--surface);
    overflow-y: auto;
  }
  @media (max-width: 1024px) {
    .ed-side {
      inline-size: 320px;
    }
  }
  .ed-problems {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    padding: 6px var(--sp-4);
    border-block-end: 1px solid var(--border);
    background: var(--danger-soft);
    color: var(--danger);
    font-size: var(--fs-s);
  }
  .ed-problems ul {
    flex-basis: 100%;
    margin: 0;
    padding-inline-start: var(--sp-5);
    color: var(--text);
  }
  .ed-link {
    border: none;
    background: none;
    padding: 0;
    color: inherit;
    font: inherit;
    text-decoration: underline;
    cursor: pointer;
  }
  .ed-chip {
    padding: 1px 8px;
    border: 1px solid var(--border);
    border-radius: 999px;
    font-size: var(--fs-xs);
    color: var(--text-dim);
  }
  .ed-h {
    margin: 0;
    font-size: var(--fs-l);
    font-weight: 600;
  }
  .ed-desc {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text);
  }
  .ed-clip {
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
    color: var(--text-dim);
  }
  .ed-back {
    align-self: flex-start;
  }
  .ed-instr {
    inline-size: 100%;
    font: inherit;
    font-size: var(--fs-m);
    padding: 6px 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-s);
    background: var(--surface-2);
    color: var(--text);
    resize: vertical;
  }
  .ed-versions {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .ed-versions li {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--sp-2);
    padding: 6px 0;
    border-block-end: 1px solid var(--border);
    font-size: var(--fs-s);
  }
  .node-hint {
    margin: 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
</style>
