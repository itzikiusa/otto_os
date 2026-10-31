///<reference types="svelte" />
;
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
function $$render() {
 const generator/*Ωignore_positionΩ*/ = (mode: 'side' | 'page')/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
   { svelteHTML.createElement("div", {  "class":`gen`,});mode === 'page';
     { svelteHTML.createElement("label", { "for":`wf-prompt`,});   }
     { svelteHTML.createElement("textarea", {            "dir":`auto`,"id":`wf-prompt`,"bind:value":prompt,"rows":mode === 'page' ? 4 : 3,"placeholder":`e.g. Ask an agent to summarize the repo, then POST the summary to our webhook.`,"onkeydown":(e) => {
        if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) generate();
      },});/*Ωignore_startΩ*/() => prompt = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
    if(mode === 'page'){
       { svelteHTML.createElement("div", { "class":`gen-actions`,});
         { svelteHTML.createElement("button", {   "class":`btn ghost`,"onclick":createBlank,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}   }
         { svelteHTML.createElement("span", { "class":`grow`,}); }
         { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":generating || prompt.trim() === '',"title":prompt.trim() === '' ? 'Describe the flow first' : 'Generate the workflow (⌘↵)',"onclick":generate,});
          if(generating){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 11px`,"aria-hidden":`true`,}); } }else{ { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":13,}});}  }
         }
       }
      if(templates.length > 0){
         { svelteHTML.createElement("div", { "class":`tpl-start`,});
           { svelteHTML.createElement("span", { "class":`tpl-start-h`,});     }
           { svelteHTML.createElement("div", { "class":`tpl-grid`,});
               for(let t of __sveltets_2_ensureArray(templates)){t.id;
               { svelteHTML.createElement("button", {     "class":`tpl tpl-card`,"onclick":() => void fromTemplate(t),"title":t.description,});
                 { svelteHTML.createElement("span", { "class":`tpl-ic`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":asIcon(t.icon, 'box'),"size":14,}});} }
                 { svelteHTML.createElement("span", { "class":`tpl-body`,});
                   { svelteHTML.createElement("span", { "class":`tpl-name`,});t.name; }
                  if(t.description){ { svelteHTML.createElement("span", { "class":`tpl-sub`,});t.description; }}
                 }
               }
            }
           }
         }
      }
    }else{
      
       { svelteHTML.createElement("button", {         "class":`btn full`,"disabled":generating || prompt.trim() === '',"title":prompt.trim() === '' ? 'Describe the flow first' : 'Generate the workflow (⌘↵)',"onclick":generate,});
        if(generating){ { svelteHTML.createElement("span", {     "class":`spinner`,"style":`--spinner-size: 11px`,"aria-hidden":`true`,}); } }else{ { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`zap`,"size":13,}});}  }
       }
       { svelteHTML.createElement("button", {   "class":`btn ghost full`,"onclick":createBlank,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":13,}});}  
       }
      if(templates.length > 0){
        
         { svelteHTML.createElement("div", { "class":`tpl-menu`,});
           { svelteHTML.createElement("button", {       "class":`btn ghost full tpl-toggle`,"aria-expanded":templatesOpen,"onclick":() => (templatesOpen = !templatesOpen),});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`grid`,"size":13,}});} 
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":templatesOpen ? 'chevronUp' : 'chevronDown',"size":12,}});}
           }
          if(templatesOpen){
             { svelteHTML.createElement("div", { "class":`tpl-pop`,});
                 for(let t of __sveltets_2_ensureArray(templates)){t.id;
                 { svelteHTML.createElement("button", {       "class":`tpl`,"onclick":() => {
                    void fromTemplate(t);
                    templatesOpen = false;
                  },"title":t.description,});
                   { svelteHTML.createElement("span", { "class":`tpl-ic`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":asIcon(t.icon, 'box'),"size":14,}});} }
                   { svelteHTML.createElement("span", { "class":`tpl-body`,});
                     { svelteHTML.createElement("span", { "class":`tpl-name`,});t.name; }
                    if(t.description){ { svelteHTML.createElement("span", { "class":`tpl-sub`,});t.description; }}
                   }
                 }
              }
             }
          }
         }
      }
    }
   }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

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

;
async () => {




 { svelteHTML.createElement("div", { "class":`wf-root`,});
 { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {   "class":`wf-bar`,"title":current?.name ?? 'Workflows',leading:() => { async ()/*Ωignore_positionΩ*/ => {
    if(viewport.isPhone && current){
       { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => void backToList(),"aria-label":`Back to workflows`,"title":`Back to workflows`,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}
       }
    }
  };return __sveltets_2_any(0)},titleContent:() => { async ()/*Ωignore_positionΩ*/ => {
    if(current && renamingId === current.id && renameInBar){
       {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {             "dir":`auto`,"class":`wf-title-edit`,"bind:value":renameValue,"aria-label":`Workflow name`,"onkeydown":(e) => {
          if (e.key === 'Enter') commitRename(current!);
          else if (e.key === 'Escape') cancelRename();
        },"onblur":() => commitRename(current!),});/*Ωignore_startΩ*/() => renameValue = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
    }else{
      current?.name ?? 'Workflows';
    }
  };return __sveltets_2_any(0)},badge:() => { async ()/*Ωignore_positionΩ*/ => {
    if(current && !(renamingId === current.id && renameInBar)){
       { svelteHTML.createElement("button", {       "class":`title-edit`,"title":`Rename workflow`,"aria-label":`Rename workflow`,"onclick":() => current && startRename(current, true),});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":13,}});}
       }
    }
    if(current && dirty){ { const $$_egdaB0C = __sveltets_2_ensureComponent(Badge); new $$_egdaB0C({ target: __sveltets_2_any(), props: {      "tone":`accent`,"label":`Unsaved`,"title":`The canvas has changes that aren’t saved yet`,}});}}
  };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
     { const $$_nottuBediuGetamotuA0C = __sveltets_2_ensureComponent(AutomateGuideButton); new $$_nottuBediuGetamotuA0C({ target: __sveltets_2_any(), props: {  "current":`workflows`,}});}
    if(current){
      
      if(!viewport.isPhone){
         { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-overflow":`1`}),...__sveltets_2_empty({"data-icon":`plus`}),"aria-expanded":paletteOpen,"aria-haspopup":`dialog`,"onclick":(e) => { anchorPop(e, 230); paletteOpen = !paletteOpen; },});
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} 
         }
         { svelteHTML.createElement("button", {           "class":`btn small`,...__sveltets_2_empty({"data-overflow":`2`}),...__sveltets_2_empty({"data-icon":`check`}),"disabled":!dirty || savingGraph,"title":dirty ? 'Save the canvas changes' : 'No unsaved changes',"onclick":save,});  }
      }
      
      if(viewport.isDesktop && run && run.context_dir){
         { svelteHTML.createElement("button", {                  "class":`icon-btn wf-tog`,...__sveltets_2_empty({"data-icon":`panel`}),"aria-pressed":ui.wfCtxOpen,"onclick":() => ui.toggleWfCtx(),"title":ui.wfCtxOpen ? 'Hide the context files & agents panel' : 'Show the context files & agents panel',"aria-label":`Context panel`,...__sveltets_2_empty({"data-label":`Context panel`}),...__sveltets_2_empty({"data-testid":`ctx-sidebar-toggle`}),});ui.wfCtxOpen;
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`panel`,"size":14,}});}
         }
      }
      if(runActive){
        
         { svelteHTML.createElement("button", {     "class":`btn small danger`,...__sveltets_2_empty({"data-keep":true}),"onclick":stop,"title":`Stop this run (finishes the current step, then halts)`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`stop`,"size":12,}});}   }
      }else{
         { svelteHTML.createElement("button", {             "class":`btn small`,"aria-pressed":runForm.open,"onclick":() => (runForm.open = !runForm.open),"title":`Run — set the input (repo_id / story_id / goals / msg) the trigger emits`,"aria-expanded":runForm.open,});!runForm.open;runForm.open;
           { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});} 
         }
      }
       { svelteHTML.createElement("button", {               "class":`icon-btn`,...__sveltets_2_empty({"data-overflow":`-10`}),...__sveltets_2_empty({"data-icon":`more`}),...__sveltets_2_empty({"data-label":`More actions…`}),"aria-haspopup":`menu`,"aria-label":`More actions`,"title":`More actions`,"onclick":wfMenu,});
         { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
       }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading, titleContent, badge, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
  
  
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":true,"padded":false,}});
 { svelteHTML.createElement("div", { "class":`wf`,});
  if(showSide){
   { svelteHTML.createElement("aside", {    "class":`side`,"style":viewport.isPhone ? '' : `width:${listW}px`,});viewport.isPhone;
    ;__sveltets_2_ensureSnippet(generator('side'));

    if(ws.activeWorkflowRuns.length > 0){
       { svelteHTML.createElement("div", {   "class":`running`,...__sveltets_2_empty({"data-testid":`running-workflows`}),});
         { svelteHTML.createElement("div", { "class":`list-h`,});
          
           { svelteHTML.createElement("span", { "class":`run-count`,});ws.activeWorkflowRuns.length; }
         }
           for(let r of __sveltets_2_ensureArray(ws.activeWorkflowRuns)){r.run_id;
           { svelteHTML.createElement("button", {        "class":`run-row`,"onclick":() => openRunById(r.workflow_id, r.run_id),"title":`${r.workflow_name} — ${runStatusLabel(r.status)}`,});run?.id === r.run_id;
             { svelteHTML.createElement("span", {   "class":`dot ${dotKey(r.status)}`,"aria-hidden":`true`,}); }
             { svelteHTML.createElement("span", { "class":`sr-only`,});runStatusLabel(r.status);  }
             { svelteHTML.createElement("span", { "class":`run-name`,});r.workflow_name; }
            if(activeWfRunCounts[r.workflow_id] > 1){
               { svelteHTML.createElement("span", {   "class":`run-ord`,"title":`run #${activeRunOrdinals[r.run_id]} of this workflow`,}); activeRunOrdinals[r.run_id]; }
            }
            if(r.waiting_approval){
               { svelteHTML.createElement("span", {       "class":`run-badge`,"role":`img`,"title":`Waiting for your approval`,"aria-label":`Waiting for your approval`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`userCheck`,"size":12,}});} }
            }
             { svelteHTML.createElement("code", {   "class":`run-id`,"title":r.run_id,});shortRunId(r.run_id); }
             { svelteHTML.createElement("span", {   "class":`run-prog`,"title":`${r.nodes_done} of ${r.nodes_total} steps done`,});r.nodes_done; r.nodes_total; }
            
             { svelteHTML.createElement("span", { "class":`run-when`,});  { const $$_emiTleR7C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR7C({ target: __sveltets_2_any(), props: {  "iso":r.started_at,"fallback":"",}});} }
           }
        }
       }
    }

     { svelteHTML.createElement("div", { "class":`list`,});
       { svelteHTML.createElement("div", { "class":`list-h`,});  }
         for(let wf of __sveltets_2_ensureArray(workflows)){wf.id;
         { svelteHTML.createElement("div", {    "class":`wf-row`,...__sveltets_2_empty({"data-testid":`wf-row-${wf.id}`}),});current?.id === wf.id;
          if(renamingId === wf.id && !renameInBar){
             {const $$action_0 = __sveltets_2_ensureAction(focusOnMount(svelteHTML.mapElementTag('input')));{ svelteHTML.createElement("input", __sveltets_2_union($$action_0), {               "dir":`auto`,"class":`row-rename`,...__sveltets_2_empty({"data-testid":`wf-rename-input`}),"bind:value":renameValue,"aria-label":`Workflow name`,"onkeydown":(e) => {
                if (e.key === 'Enter') commitRename(wf);
                else if (e.key === 'Escape') cancelRename();
              },"onblur":() => commitRename(wf),});/*Ωignore_startΩ*/() => renameValue = __sveltets_2_any(null);/*Ωignore_endΩ*/}}
          }else{
             {const $$action_0 = __sveltets_2_ensureAction(rowMenuKeys(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {        "class":`row-main`,"title":wf.name,"onclick":() => void openGuarded(wf),"oncontextmenu":(e) => { e.preventDefault(); rowMenu(e, wf); },});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`split`,"size":13,}});}
               { svelteHTML.createElement("span", { "class":`row-name`,});wf.name; }
             }}
            
             { svelteHTML.createElement("button", {           "class":`row-edit`,"title":`More actions`,"aria-label":`More actions for “${wf.name}”`,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-testid":`wf-row-more`}),"onclick":(e) => rowMenu(e, wf),});
               { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":13,}});}
             }
          }
         }
      }
      if(workflows.length > 0 && wfError){
        
         { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {        "what":`workflows`,"variant":`compact`,"error":wfError,"onretry":() => void load(),}});}
      } else if (workflows.length === 0 && (!wfError || current)){
        
         { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL5 = new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {         "what":`workflows`,"variant":`compact`,"loading":wfLoading,"error":wfError,"empty":true,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`split`,"title":`No workflows yet`,"body":`Describe one above, or start from a template.`,}});}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL5.$$prop_def;/*Ωignore_endΩ*/
          
         LoadState}
      }
     }
   }
  if(!viewport.isPhone){
     { const $$_rediviDenaP3C = __sveltets_2_ensureComponent(PaneDivider); const $$_rediviDenaP3 = new $$_rediviDenaP3C({ target: __sveltets_2_any(), props: {       width:listW,"storageKey":LIST_W_KEY,"label":`Resize the workflows list`,}});/*Ωignore_startΩ*/() => listW = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rediviDenaP3.$$bindings = 'width';}
  }
  }

  if(showMain){
   { const $$_main3 = svelteHTML.createElement("main", {       "class":`main`,"style":sideDock
      ? `padding-inline-end:${ui.wfInspSideWidth}px;--wf-insp-w:${ui.wfInspSideWidth}px`
      : '',});mainEl = $$_main3;sideDock;
    if(current){
      
      if(paletteOpen){
         { svelteHTML.createElement("div", {      "class":`palette wf-pop`,"role":`dialog`,"aria-label":`Add a node`,});__sveltets_2_ensureType(String, Number, `${popRight} `);
             for(let t of __sveltets_2_ensureArray(types)){t.kind;
             { svelteHTML.createElement("button", {   "class":`pal-item`,"onclick":() => addNode(t),});
               { svelteHTML.createElement("span", { "class":`pal-ic`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":asIcon(t.icon, 'box'),"size":12,}});} }
               { svelteHTML.createElement("span", { "class":`pal-body`,});
                 { svelteHTML.createElement("span", { "class":`pal-name`,});t.label; }
                 { svelteHTML.createElement("span", { "class":`pal-cat`,});t.category; }
               }
             }
          }
         }
      }
      if(runsOpen){
         { svelteHTML.createElement("div", {      "class":`palette runs-pop wf-pop`,"role":`dialog`,"aria-label":`Runs`,});__sveltets_2_ensureType(String, Number, `${popRight} `);
           { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL5 = new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`runs`,"variant":`compact`,"loading":runsLoading,"error":runsError,"empty":runs.length === 0,"onretry":() => void loadRuns(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("div", { "class":`runs-empty`,});         }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL5.$$prop_def;/*Ωignore_endΩ*/
            
             for(let r of __sveltets_2_ensureArray(runs)){r.id;
             { svelteHTML.createElement("button", {      "class":`run-item`,...__sveltets_2_empty({"data-testid":`run-item`}),"onclick":() => void openRunById(r.workflow_id, r.id),});run?.id === r.id;
               { svelteHTML.createElement("span", {   "class":`dot ${dotKey(r.status)}`,"aria-hidden":`true`,}); }
               { svelteHTML.createElement("span", { "class":`run-status`,});runStatusLabel(r.status); }
               { svelteHTML.createElement("span", { "class":`run-when`,});  { const $$_emiTleR8C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR8C({ target: __sveltets_2_any(), props: {  "iso":r.started_at,"fallback":"",}});} }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("code", {   "class":`run-id`,"title":r.id,});shortRunId(r.id); }
             }
          }
           LoadState}
         }
      }

      
      if(runForm.open){
         { const $$_lenaPtupnInuR4C = __sveltets_2_ensureComponent(RunInputPanel); new $$_lenaPtupnInuR4C({ target: __sveltets_2_any(), props: {     "form":runForm,running,"onrun":() => void confirmRun(),}});}
      }

      
       { const $$_slavorppAnuR4C = __sveltets_2_ensureComponent(RunApprovals); new $$_slavorppAnuR4C({ target: __sveltets_2_any(), props: {   mon,graph,nodeName,}});}

      if(validationIssues.length){
         {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('div'),({ text: `Resolve these issues before running: ${validationIssues.map((i) => i.message).join('; ')}`, assertive: true })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {   "class":`preflight`,}); { svelteHTML.createElement("strong", {});     }
            for(let issue of __sveltets_2_ensureArray(validationIssues)){
             { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":() => { selectedId = issue.node_id; selectedEdgeId = issue.edge_id; },});issue.node_id ? nodeName(issue.node_id) : issue.edge_id ? 'Connection' : 'Graph'; issue.message; }
          }
         }}
      }
      if(run?.workflow_version && current.version !== run.workflow_version){
         { svelteHTML.createElement("div", { "class":`preflight`,});    run.workflow_version;         current.version;              }
      }
       { svelteHTML.createElement("div", { "class":`canvas-wrap`,});
         { const $$_savnaCwolfkroW5C = __sveltets_2_ensureComponent(WorkflowCanvas); const $$_savnaCwolfkroW5 = new $$_savnaCwolfkroW5C({ target: __sveltets_2_any(), props: {                graph,types,runStates,"invalidNodes":validationIssues.flatMap((i) => i.node_id ? [i.node_id] : []),"invalidEdges":validationIssues.flatMap((i) => i.edge_id ? [i.edge_id] : []),selectedId,selectedEdgeId,"onselect":(id) => { selectedId = id; selectedEdgeId = null; },"onedgeselect":(id) => { selectedEdgeId = id; if (id) selectedId = null; },"onchange":() => (dirty = true),}});/*Ωignore_startΩ*/() => graph = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_savnaCwolfkroW5.$$bindings = 'graph';}
       }

      if(instructionsOpen && current){
         { svelteHTML.createElement("div", { "class":`instructions-wrap`,});
           { svelteHTML.createElement("div", { "class":`instructions-h`,});
             { svelteHTML.createElement("span", {});  }
            if(instructionsDirty){ { const $$_egdaB6C = __sveltets_2_ensureComponent(Badge); new $$_egdaB6C({ target: __sveltets_2_any(), props: {    "tone":`accent`,"label":`Unsaved`,}});}}
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":!instructionsDirty || savingInstructions,"title":instructionsDirty ? 'Save the instructions' : 'No unsaved changes',"onclick":saveInstructions,});
              savingInstructions ? 'Saving…' : 'Save instructions';
             }
           }
           { svelteHTML.createElement("p", { "class":`instructions-hint`,});
                         
           }
           { svelteHTML.createElement("textarea", {            "dir":`auto`,"class":`ri-text mono`,"rows":8,"aria-label":`Workflow instructions`,"bind:value":wfInstructions,"placeholder":`Standing rules every step follows by the letter (markdown)`,});/*Ωignore_startΩ*/() => wfInstructions = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
           { svelteHTML.createElement("label", { "class":`resume-toggle`,});
             { svelteHTML.createElement("input", {       "type":`checkbox`,"checked":(current.on_restart ?? 'resume') !== 'fail',"onchange":(e) => void saveOnRestart(e.currentTarget.checked),});}
             { svelteHTML.createElement("span", {});
                            
                         
                       
             }
           }
         }
      }

      if(triggersOpen && current){
        current.id; {
         { svelteHTML.createElement("div", { "class":`triggers-wrap`,});
           { const $$_lenaPsreggirT5C = __sveltets_2_ensureComponent(TriggersPanel); const $$_lenaPsreggirT5 = new $$_lenaPsreggirT5C({ target: __sveltets_2_any(), props: {        "workflowId":current.id,"workflowName":current.name,triggers,"ontriggers":(ts) => (triggers = ts),}});/*Ωignore_startΩ*/() => triggers = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_lenaPsreggirT5.$$bindings = 'triggers';}
         }
        }
      }

      if(versionsOpen && current){
         { svelteHTML.createElement("div", { "class":`versions-wrap`,});
           { svelteHTML.createElement("div", { "class":`versions-h`,});
             { svelteHTML.createElement("span", {});  }
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":versionsLoading || versionsBefore === undefined,"onclick":() => void loadVersions(),});  }
             { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":versionsLoading || !versionsHasMore || !!versionsError,"onclick":() => void loadVersions(versions.at(-1)?.version),});  }
             { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":versionsLoading,"onclick":() => void loadVersions(versionsBefore),});
              
             }
           }
           { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL5 = new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`version history`,"variant":`compact`,"loading":versionsLoading,"error":versionsError,"empty":versions.length === 0,"onretry":() => void loadVersions(versionsBefore),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`clock`,"title":versionsBefore === undefined ? 'No saved versions yet' : 'No older versions',"body":versionsBefore === undefined ? 'Saving the canvas or restoring a version creates one.' : '',}});}};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL5.$$prop_def;/*Ωignore_endΩ*/
            
             { svelteHTML.createElement("ul", { "class":`versions`,});
                 for(let v of __sveltets_2_ensureArray(versions)){v.id;
                 { svelteHTML.createElement("li", { "class":`ver`,});
                   { svelteHTML.createElement("span", { "class":`ver-num`,}); v.version; }
                   { svelteHTML.createElement("span", {   "class":`ver-note`,"title":v.note || undefined,});v.note || '(no note)'; }
                   { svelteHTML.createElement("span", { "class":`ver-when`,});  { const $$_emiTleR9C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR9C({ target: __sveltets_2_any(), props: {  "iso":v.created_at,"fallback":"",}});} }
                   { svelteHTML.createElement("button", {     "class":`btn small`,"title":`Restore v${v.version} — saved as a new version, so nothing is lost`,"onclick":() => restoreVersion(v),});  }
                 }
              }
             }
           LoadState}
         }
      }

      if(inspOpen){
        
         {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),(sideDock
            ? { value: ui.wfInspSideWidth, min: 280, max: Math.max(300, Math.round(window.innerWidth * 0.6)), invert: true, onChange: (w) => ui.setWfInspSideWidth(w), onReset: resetInsp, onDragStart: startInspResize, text: pxText }
            : { value: ui.runDetailHeight, min: 160, max: Math.max(160, Math.round(window.innerHeight * 0.85)), orientation: 'horizontal', invert: true, onChange: (h) => { runDetailMax = false; ui.setRunDetailHeight(h); }, onReset: resetInsp, onDragStart: startInspResize, text: (v) => `${Math.round(v)} pixels tall` })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {            "class":`insp-grip`,"role":`separator`,"aria-label":`Resize node inspector`,"title":`Drag or use arrow keys to resize · double-click or Enter to reset`,});sideDock; }}
         { svelteHTML.createElement("div", {       "class":`inspector`,"style":sideDock ? `width:${ui.wfInspSideWidth}px` : `max-height:${inspMaxPx()}px`,});runDetailMax;sideDock;
          if(sideDock){
            
             { svelteHTML.createElement("div", { "class":`insp-side-head`,});
               { svelteHTML.createElement("strong", {});  }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":closeInspector,"title":`Close the docked panel (re-open with Dock)`,"aria-label":`Close panel`,});
                 { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":13,}});}
               }
             }
          }
          
           { const $$_rotinoMnuR5C = __sveltets_2_ensureComponent(RunMonitor); const $$_rotinoMnuR5 = new $$_rotinoMnuR5C({ target: __sveltets_2_any(), props: {       mon,nodeName,selectedId,runDetailMax,"onstop":() => void stop(),"onOpenSession":openSessionInline,}});/*Ωignore_startΩ*/() => selectedId = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rotinoMnuR5.$$bindings = 'selectedId';/*Ωignore_startΩ*/() => runDetailMax = __sveltets_2_any(null);/*Ωignore_endΩ*/$$_rotinoMnuR5.$$bindings = 'runDetailMax';}

          if(selectedNode){
             { svelteHTML.createElement("div", { "class":`insp-h`,});
               { svelteHTML.createElement("strong", { "title":selectedNode.name || selectedNode.kind,});selectedNode.name || selectedNode.kind; }
               { svelteHTML.createElement("span", { "class":`mono dim`,});selectedNode.kind; }
              if(selectedRun){ { const $$_egdaBsutatS6C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS6C({ target: __sveltets_2_any(), props: {    "status":runStatus(selectedRun.status),"variant":`text`,}});}}
              if(selectedRun?.duration_ms != null){ { svelteHTML.createElement("span", { "class":`dim`,}); fmtMs(selectedRun.duration_ms); }}
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":running,"onclick":() => runFrom(selectedNode.id, false),"title":`Run this node and everything downstream`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}   }
               { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":running,"onclick":() => runFrom(selectedNode.id, true),"title":`Run only this node`,});  }
             }
             { const $$_smaraPedoN5C = __sveltets_2_ensureComponent(NodeParams); new $$_smaraPedoN5C({ target: __sveltets_2_any(), props: {              "node":selectedNode,"set":onParam,"setParams":(params) => { if (selectedNode) { selectedNode.params = params; graph = graph; dirty = true; } },"wsId":current?.workspace_id ?? ws.currentId ?? '',pickers,"workflowName":current?.name,"onzoom":(field, label) => (jsonZoom = { field, label }),}});}

            
            if(selectedNode.kind !== 'manual_trigger'){
               { svelteHTML.createElement("div", { "class":`retry-form`,});
                 { svelteHTML.createElement("span", { "class":`retry-h`,});  selectedNode.retry ? 'custom' : 'default'; effectiveRetry(selectedNode).max_attempts;   }
                 { svelteHTML.createElement("button", {   "class":`btn ghost small`,"onclick":() => { if (selectedNode) clearRetry(selectedNode); graph = graph; dirty = true; },});  }
                 { svelteHTML.createElement("div", { "class":`retry-row`,});
                   { svelteHTML.createElement("label", { "for":`np-retry-max`,});   }
                   { svelteHTML.createElement("input", {             "id":`np-retry-max`,"type":`number`,"min":`0`,"max":`5`,"value":retryNum('max_attempts', 0),"oninput":(e) => onRetry('max_attempts', Number(e.currentTarget.value)),});}
                   { svelteHTML.createElement("label", { "for":`np-retry-bo`,});  }
                   { svelteHTML.createElement("input", {               "id":`np-retry-bo`,"type":`number`,"min":`0`,"max":`60000`,"step":`100`,"value":retryNum('backoff_ms', 0),"oninput":(e) => onRetry('backoff_ms', Number(e.currentTarget.value)),});}
                 }
               }
            }
            if(inspectorError){
               { const $$_etatSdaoL5C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL5C({ target: __sveltets_2_any(), props: {        "what":`the step output`,"variant":`compact`,"error":inspectorError,"empty":true,"onretry":() => { inspectorError = null; inspectorRetry++; },}});}
            }
            if(selectedRun?.error){
               { svelteHTML.createElement("div", { "class":`err`,});selectedRun.error; }
            }
            if(selectedRun?.logs?.length){
               { svelteHTML.createElement("div", { "class":`logs`,});  for(let l of __sveltets_2_ensureArray(selectedRun.logs)){ { svelteHTML.createElement("div", {});l; }} }
            }
            if(selectedRun?.output !== undefined && selectedRun?.output !== null){
               { svelteHTML.createElement("pre", { "class":`out`,});JSON.stringify(selectedRun.output, null, 2).slice(0, 1200); }
            }
          } else if (selectedEdge){
             { svelteHTML.createElement("div", { "class":`insp-h`,});
               { svelteHTML.createElement("strong", {});  }
               { svelteHTML.createElement("span", { "class":`mono dim`,});nodeName(selectedEdge.source);  nodeName(selectedEdge.target); }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("button", {       "class":`btn small danger`,"title":`Delete connection`,"aria-label":`Delete connection`,"onclick":removeSelectedEdge,});
                 { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}
               }
             }
             { svelteHTML.createElement("label", { "for":`np-edge-cond`,});   }
             { svelteHTML.createElement("input", {            "dir":`auto`,"id":`np-edge-cond`,"type":`text`,"placeholder":`e.g. passed == true`,"value":selectedEdge.condition ?? '',"oninput":(e) => onEdgeCondition(e.currentTarget.value),});}
             { svelteHTML.createElement("p", { "class":`node-hint`,});              }
          } else if (!run){
            
             { svelteHTML.createElement("div", { "class":`insp-blank`,});
               { const $$_etatSytpmE6C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE6C({ target: __sveltets_2_any(), props: {    "icon":`split`,"title":`Select a node or connection to configure it`,}});}
             }
          }
         }
      }
    } else if (!ws.currentId){
       { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`folder`,"title":`Add a workspace to get started`,"body":`Workflows belong to a workspace. Add your project folder to create and run an automation.`,"actionLabel":`Add workspace`,"actionIcon":`plus`,"onaction":() => (ui.newWorkspaceOpen = true),}});}
    } else if (workflows.length === 0 && (wfError || wfLoading)){
       { const $$_etatSdaoL4C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL4C({ target: __sveltets_2_any(), props: {          "what":`workflows`,"variant":`page`,"loading":wfLoading,"error":wfError,"empty":true,"onretry":() => void load(),}});}
    } else if (workflows.length === 0){
      
       { svelteHTML.createElement("div", { "class":`first-run`,});
         { const $$_etatSytpmE5C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE5C({ target: __sveltets_2_any(), props: {         children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`split`,"title":`No workflows yet`,"body":`Describe an automation in plain words and an agent wires up the steps — or start blank, or from a template, and build it on the canvas.`,}});
          ;__sveltets_2_ensureSnippet(generator('page'));
           { const $$_ediuGetamotuA6C = __sveltets_2_ensureComponent(AutomateGuide); new $$_ediuGetamotuA6C({ target: __sveltets_2_any(), props: {  "current":`workflows`,}});}
         EmptyState}
       }
    }else{
      
       { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {         "variant":`page`,"icon":`split`,"title":`No workflow open`,"body":`${workflows.length} ${workflows.length === 1 ? 'workflow' : 'workflows'} in this workspace. Open one from the list, or describe a new one.`,}});}
    }
   }
  }

  if(viewport.isDesktop && run && run.context_dir && ui.wfCtxOpen){
    
     { const $$_rabediStxetnoCnuR3C = __sveltets_2_ensureComponent(RunContextSidebar); new $$_rabediStxetnoCnuR3C({ target: __sveltets_2_any(), props: {       mon,run,"contextDir":run.context_dir,nodeName,"focusSid":agentsFocusSid,}});}
  }
 }
 PageBody}
 }


if(jsonZoom){
  const jz = jsonZoom;
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":jz.label,"width":900,"onclose":() => (jsonZoom = null),}});
     { svelteHTML.createElement("textarea", {                "dir":`ltr`,"aria-label":jz.label,"class":`json-zoom mono`,...__sveltets_2_empty({"data-testid":`json-zoom-editor`}),"value":paramJson(selectedNode, jz.field),"oninput":(e) => onParamJson(jz.field, e.currentTarget.value),"spellcheck":`false`,"placeholder":'[ { "kind": "agent_prompt", "params": {} } ]',}); }
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const WorkflowsPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type WorkflowsPage__SvelteComponent_ = ReturnType<typeof WorkflowsPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default WorkflowsPage__SvelteComponent_;