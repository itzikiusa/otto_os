///<reference types="svelte" />
;// Run with Otto — the "one button" flow. Turn any source (a Jira story, a
// GitHub issue/PR, a Slack thread, a finding, a failing test) into a reviewed,
// evidence-backed PR draft. The page has three areas: the launcher (the one
// button), the runs list, and the open run's detail panel.

import { scrollBehavior } from '../../lib/motion';
import { untrack } from 'svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { runWithOtto } from '../../lib/stores/runWithOtto.svelte';
import ProofStatusChip from '../../lib/components/ProofStatusChip.svelte';
import Icon from '../../lib/components/Icon.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import RunLauncher from './RunLauncher.svelte';
import RunDetail from './RunDetail.svelte';
import RunStageRail from './RunStageRail.svelte';
import type { OttoRun } from '../../lib/api/types';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { initialSelection, rememberSelection } from '../../lib/lastSelection';
import { viewport } from '../../lib/stores/viewport.svelte';
import { runStatusInfo, sourceLabel } from './runStatus';
import { router } from '../../lib/router.svelte';
import { plural } from '../../lib/plural';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  // Load the workspace's runs whenever the active workspace changes. This effect
  // reads ONLY ws.currentId (not the run list it loads), so it never self-loops.
  // An open run from ANOTHER workspace is closed — its cached record would
  // otherwise keep showing next to this workspace's list.
  $effect(() => {
    const id = ws.currentId;
    if (!id) return;
    untrack(() => {
      const open = runWithOtto.openRun;
      if (open && open.workspace_id !== id) runWithOtto.closeDetail();
    });
    void runWithOtto.loadList(id);
  });

  const list = $derived(runWithOtto.list);
  const openRun = $derived(runWithOtto.openRun);

  let detailEl: HTMLElement | undefined = $state();

  // A list/detail page opens ON a run, never on a list with nothing beside it
  // (layout.md): once the list is in, open the remembered run (else the
  // newest). Desktop only — on narrow widths the detail stacks under the list
  // and auto-opening would push the list off-screen. Runs once per workspace
  // list, so closing the panel sticks.
  let autoOpenedFor = '';
  $effect(() => {
    const id = ws.currentId;
    const rows = list;
    if (!id || runWithOtto.loadingList || rows.length === 0 || autoOpenedFor === id) return;
    autoOpenedFor = id;
    untrack(() => {
      // A deep link (`#/run-with-otto/<id>`) opens on its own (below).
      if (runWithOtto.openRun || !viewport.isDesktop || routeRunId()) return;
      const pick = initialSelection('run-with-otto', rows, (r) => r.id);
      if (pick) void runWithOtto.open(pick);
    });
  });

  /** The run the URL names (`#/run-with-otto/<id>`), or null. */
  function routeRunId(): string | null {
    const [mod, id] = router.parts;
    return mod === 'run-with-otto' && id ? id : null;
  }
  // The URL carries the open run so a reload / share / notification lands on
  // it. Only a CLOSE clears the URL, and on arrival a link naming a DIFFERENT
  // run than the one the store kept open wins (the effect below opens it).
  let routedId: string | null = null;
  let arrived = false;
  $effect(() => {
    const id = runWithOtto.openId;
    const was = routedId;
    routedId = id;
    const first = !arrived;
    arrived = true;
    if (!id && !was) return;
    const linked = untrack(() => routeRunId());
    if (first && linked && linked !== id) return;
    untrack(() => {
      if (router.module === 'run-with-otto' && linked !== id) router.replace(id ? `run-with-otto/${id}` : 'run-with-otto');
    });
  });
  // A route change while the page is up (a notification, Back/Forward, a
  // pasted link) opens that run.
  $effect(() => {
    const linked = routeRunId();
    if (!linked) return;
    untrack(() => {
      if (linked !== runWithOtto.openId) {
        rememberSelection('run-with-otto', linked);
        void runWithOtto.open(linked);
      }
    });
  });

  function show(run: OttoRun): void {
    rememberSelection('run-with-otto', run.id);
    void runWithOtto.open(run.id);
    // Stacked layout: bring the freshly opened detail into view.
    if (!viewport.isDesktop) {
      requestAnimationFrame(() => detailEl?.scrollIntoView({ block: 'start', behavior: scrollBehavior() }));
    }
  }

  function onLaunched(run: OttoRun): void {
    if (run.workspace_id === ws.currentId) show(run);
  }

  function selectRun(run: OttoRun): void {
    show(run);
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`rwo-page`,});
  { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {    "title":`Run with Otto`,"subtitle":`From a story, issue, finding or failing test to a PR draft`,leading:() => { async ()/*Ωignore_positionΩ*/ => {
    if(viewport.isPhone && openRun){
       { svelteHTML.createElement("button", {       "class":`btn ghost`,"onclick":() => runWithOtto.closeDetail(),"aria-label":`Back to runs`,"title":`Back to runs`,}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":16,}});}  }
    }
  };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {leading} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
  
 PageHeader}
 { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});
 { svelteHTML.createElement("div", { "class":`rwo`,});

  if(ws.currentId){
    
     { svelteHTML.createElement("div", { });viewport.isPhone && !!openRun;
    ws.currentId; {
       { const $$_rehcnuaLnuR4C = __sveltets_2_ensureComponent(RunLauncher); new $$_rehcnuaLnuR4C({ target: __sveltets_2_any(), props: {   "wsId":ws.currentId,onLaunched,}});}
    }
     }

   { svelteHTML.createElement("div", { "class":`body-wrap`,});
   { svelteHTML.createElement("div", {  "class":`body`,});openRun;
     { svelteHTML.createElement("section", {    "class":`list-col`,"aria-label":`Runs`,});viewport.isPhone && !!openRun;
       { const $$_etatSdaoL6C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL6 = new $$_etatSdaoL6C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`runs`,"loading":runWithOtto.loadingList,"error":runWithOtto.listError,"empty":list.length === 0,"onretry":() => ws.currentId && void runWithOtto.loadList(ws.currentId),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
           { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`play`,"title":`No runs yet`,"body":`Paste a source above and press Run with Otto. Each run shows up here with its stage, proof and findings.`,}});}
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL6.$$prop_def;/*Ωignore_endΩ*/
        
         { svelteHTML.createElement("h2", { "class":`list-h`,});  { svelteHTML.createElement("span", { "class":`count`,});list.length; } }
         { svelteHTML.createElement("ul", { "class":`runs`,});
             for(let r of __sveltets_2_ensureArray(list)){r.id;
             { svelteHTML.createElement("li", {});
               { svelteHTML.createElement("button", {        "class":`run`,"aria-current":openRun?.id === r.id ? 'true' : undefined,"onclick":() => selectRun(r),});openRun?.id === r.id;
                 { svelteHTML.createElement("div", { "class":`run-top`,});
                   { svelteHTML.createElement("span", { "class":`chip`,});sourceLabel(r.source_kind); }
                   { svelteHTML.createElement("span", {   "class":`run-title`,"title":r.title || r.source_ref,});r.title || r.source_ref; }
                   { const $$_egdaBsutatS11C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS11C({ target: __sveltets_2_any(), props: {  "status":runStatusInfo(r.status),}});}
                 }
                 { svelteHTML.createElement("div", { "class":`run-meta`,});
                    { const $$_liaRegatSnuR11C = __sveltets_2_ensureComponent(RunStageRail); new $$_liaRegatSnuR11C({ target: __sveltets_2_any(), props: {  "status":r.status,"mini":true,}});}
                  if(r.proof_pack_id && r.proof_status){
                      { const $$_pihCsutatSfoorP11C = __sveltets_2_ensureComponent(ProofStatusChip); new $$_pihCsutatSfoorP11C({ target: __sveltets_2_any(), props: {    "status":r.proof_status,"risk":r.risk_score,"compact":true,}});}
                  }
                   { svelteHTML.createElement("span", { "class":`findings`,});
                    plural(r.findings_total, 'finding');
                    if(r.findings_blocking > 0){
                       { svelteHTML.createElement("span", { "class":`blocking`,});r.findings_blocking;  }
                    }
                   }
                   { svelteHTML.createElement("span", {   "class":`agent mono`,"title":`Executing agent (provider · model)`,});
                    r.provider;r.model ? ` · ${r.model}` : '';
                   }
                   { svelteHTML.createElement("span", { "class":`when`,}); { const $$_emiTleR12C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR12C({ target: __sveltets_2_any(), props: {  "iso":r.updated_at,}});} }
                 }
               }
             }
          }
         }
       LoadState}
     }

    if(openRun){
       { const $$_section5 = svelteHTML.createElement("section", {    "class":`detail-col`,"aria-label":`Run detail`,});detailEl = $$_section5;
        
        openRun.id; {
           { const $$_liateDnuR6C = __sveltets_2_ensureComponent(RunDetail); new $$_liateDnuR6C({ target: __sveltets_2_any(), props: {    "run":openRun,"onClose":() => runWithOtto.closeDetail(),}});}
        }
       }
    }
   }
   }
  }else{
     { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {               "variant":`page`,"icon":`folder`,"title":`Add a workspace to get started`,"body":`Runs belong to a workspace. Add your project folder, then choose the source you want Otto to work on.`,"actionLabel":`Add workspace`,"actionIcon":`plus`,"onaction":() => (ui.newWorkspaceOpen = true),}});}
  }
 }
 PageBody}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RunWithOttoPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RunWithOttoPage__SvelteComponent_ = ReturnType<typeof RunWithOttoPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default RunWithOttoPage__SvelteComponent_;