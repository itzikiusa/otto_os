///<reference types="svelte" />
;// The desktop Context sidebar for a run with a context dir (R1 + Agents):
// Files (the deliverable and the run's context-file tree) and Agents (the
// run's live agent sessions, embedded). Resizable from its left edge.

import Icon from '../../lib/components/Icon.svelte';
import { startMouseDrag } from '../../lib/dragCursor';
import { paneResizer } from '../../lib/paneResizer';
import { ui, WF_CTX_MIN, WF_CTX_MAX } from '../../lib/stores/ui.svelte';
import Tabs from '../../lib/components/Tabs.svelte';
import FileTree from '../panels/FileTree.svelte';
import RunAgents from './RunAgents.svelte';
import RunFinalOutput from './RunFinalOutput.svelte';
import { contextFs, type RunSync } from './runSync.svelte';
import type { WorkflowRun } from '../../lib/api/types';
;

  interface Props {
    mon: RunSync;
    /** The run on screen, and its context dir. */
    run: WorkflowRun;
    contextDir: string;
    nodeName: (id: string) => string;
    /** The session a step's "Open" asked the Agents tab to focus. */
    focusSid: string | null;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { mon, run, contextDir, nodeName, focusSid }: Props = $props();

  // Total agent sessions the current run spawned — the Agents-tab badge.
  const runSessionCount = $derived(
    run.nodes.reduce((a, n) => a + (n.sessions?.length ?? 0), 0),
  );
  const pxText = (v: number): string => `${Math.round(v)} pixels wide`;
  // Context-files sidebar width: drag its left edge (anchored right → dragging
  // left widens it). (R1)
  function startCtxResize(e: MouseEvent): void {
    e.preventDefault();
    const startX = e.clientX;
    const startW = ui.wfCtxWidth;
    startMouseDrag(e, { onMove: (ev) => ui.setWfCtxWidth(startW + (startX - ev.clientX)) });
  }
;
async () => {


 { svelteHTML.createElement("aside", {     "class":`ctx-sidebar`,"style":`width:${ui.wfCtxWidth}px`,...__sveltets_2_empty({"data-testid":`ctx-sidebar`}),});
   {const $$action_0 = __sveltets_2_ensureAction(paneResizer(svelteHTML.mapElementTag('div'),({ value: ui.wfCtxWidth, min: WF_CTX_MIN, max: WF_CTX_MAX, invert: true, onChange: (w) => ui.setWfCtxWidth(w), onDragStart: startCtxResize, text: pxText })));{ svelteHTML.createElement("div", __sveltets_2_union($$action_0), {           "class":`ctx-resize`,"role":`separator`,"aria-label":`Resize context panel`,"title":`Drag or use ←/→ to resize`,}); }}
   { svelteHTML.createElement("div", { "class":`ctx-head`,});
    
     { const $$_sbaT2C = __sveltets_2_ensureComponent(Tabs); new $$_sbaT2C({ target: __sveltets_2_any(), props: {           "label":`Context panel sections`,"size":`s`,"value":ui.wfCtxTab,"onchange":(id) => ui.setWfCtxTab(id),"tabs":[
        { id: 'files', label: 'Files', icon: 'folder' },
        { id: 'agents', label: 'Agents', icon: 'terminal', count: runSessionCount > 0 ? runSessionCount : undefined, testid: 'ctx-tab-agents' },
      ],}});}
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {           "class":`icon-btn`,...__sveltets_2_empty({"data-testid":`ctx-collapse`}),"onclick":() => ui.toggleWfCtx(),"title":`Collapse panel`,"aria-label":`Collapse panel`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`panel`,"size":13,}});}
     }
   }
  if(ui.wfCtxTab === 'files'){
     { svelteHTML.createElement("div", { "class":`ctx-pathline`,}); { svelteHTML.createElement("code", {   "class":`ctx-path dim`,"title":contextDir,});contextDir; } }
     { const $$_tuptuOlaniFnuR1C = __sveltets_2_ensureComponent(RunFinalOutput); new $$_tuptuOlaniFnuR1C({ target: __sveltets_2_any(), props: {  mon,run,}});}
     { svelteHTML.createElement("div", { "class":`ctx-body`,});
      contextDir; {
         { const $$_eerTeliF2C = __sveltets_2_ensureComponent(FileTree); new $$_eerTeliF2C({ target: __sveltets_2_any(), props: {     "root":contextDir,"primary":false,...contextFs(run.id, contextDir),}});}
      }
     }
  }else{
     { svelteHTML.createElement("div", { "class":`ctx-body`,});
       { const $$_stnegAnuR2C = __sveltets_2_ensureComponent(RunAgents); new $$_stnegAnuR2C({ target: __sveltets_2_any(), props: {    run,"nodeName":(id) => nodeName(id),focusSid,}});}
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunContextSidebar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunContextSidebar__SvelteComponent_ = ReturnType<typeof RunContextSidebar__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunContextSidebar__SvelteComponent_;