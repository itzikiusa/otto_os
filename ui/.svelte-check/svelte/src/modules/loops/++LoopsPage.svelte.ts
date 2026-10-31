///<reference types="svelte" />
;
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { loops } from '../../lib/stores/loops.svelte';
import { loopsPagePort } from '../../lib/uiCommands/loops';
import GoalDefineForm from './GoalDefineForm.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import AutomateGuide from '../../lib/components/AutomateGuide.svelte';
import AutomateGuideButton from '../../lib/components/AutomateGuideButton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import LoopDetail from './LoopDetail.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { loopStatus } from './loopStatus';
import Icon from '../../lib/components/Icon.svelte';
import RelTime from '../../lib/components/RelTime.svelte';
import { untrack } from 'svelte';
import { router } from '../../lib/router.svelte';
import { registry } from '../../lib/commands.svelte';
import { recallSelection, rememberSelection } from '../../lib/lastSelection';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  const PHASE_LABEL: Record<string, string> = {
    planning: 'Planning',
    executing: 'Executing',
    evaluating: 'Evaluating',
    digesting: 'Digesting',
    waiting: 'Waiting on an agent',
  };

  let selectedId = $state<string | null>(null);
  let creating = $state(false);

  // (Re)load the list when the workspace changes.
  $effect(() => {
    const id = ws.currentId;
    if (id) void loops.loadList(id);
  });

  // A workspace switch closes the open loop (it belongs to the previous
  // workspace: it stayed open and controllable, and the URL kept its id) and
  // the new-goal form. The restore effect below then reopens the new
  // workspace's last loop.
  let viewWs = untrack(() => ws.currentId);
  $effect(() => {
    const id = ws.currentId;
    if (id === viewWs) return;
    viewWs = id;
    untrack(() => {
      creating = false;
      if (selectedId) back();
    });
  });

  const list = $derived(loops.list);

  function open(id: string): void {
    selectedId = id;
    creating = false;
    void loops.loadDetail(id);
  }
  function back(): void {
    selectedId = null;
    loops.closeDetail();
    const id = ws.currentId;
    if (id) void loops.loadList(id);
  }
  // The last-opened loop was remembered (rememberSelection below) but never
  // restored: reopen it once per workspace when the page lands on the bare
  // list (no `#/loops/<id>` link) and that loop still exists. Closing it is
  // respected — this runs once per workspace load, not on every list refresh.
  let restoredFor: string | null = null;
  $effect(() => {
    const w = ws.currentId;
    const items = list;
    if (!w || restoredFor === w || loops.loadingList || loops.listWs !== w || items.length === 0) return;
    untrack(() => {
      restoredFor = w;
      if (selectedId || creating || router.parts[1]) return;
      const last = recallSelection('loops');
      if (last && items.some((l) => l.id === last)) open(last);
    });
  });

  // Agent UI control (lib/uiCommands/loops.ts) opens a loop's detail here.
  $effect(() => loopsPagePort.bind({ open, selectedId: () => selectedId }));

  // The URL carries the open loop (`#/loops/<id>`) so a
  // reload / share / notification lands on it. Route → page: a link (or
  // Back/Forward) opens / closes the detail; page → route: opening or closing
  // a loop rewrites the URL in place.
  $effect(() => {
    const [mod, id] = router.parts;
    if (mod !== 'loops') return;
    untrack(() => {
      if (id && id !== selectedId) open(id);
      else if (!id && selectedId) back();
    });
  });
  let routedId: string | null = null;
  $effect(() => {
    const id = selectedId;
    if (id) rememberSelection('loops', id);
    const was = routedId;
    routedId = id;
    if (!id && !was) return;
    untrack(() => {
      if (router.module === 'loops' && (router.parts[1] ?? null) !== id) router.replace(id ? `loops/${id}` : 'loops');
    });
  });

  // ⌘K verbs for the open loop. The detail owns confirm/extend; this only registers them.
  const loopActions = {
    pause: () => {},
    resume: () => {},
    stop: () => {},
    extend: () => {},
  };
  // ⌘K: create on the list; Pause / Resume / Extend budget / Stop while a loop is open.
  $effect(() => {
    const create = { id: 'loops.new', title: 'New goal loop…', group: 'Goal Loops', keywords: 'create goal iterate objective', run: () => { if (selectedId) back(); creating = true; } };
    const openId = selectedId;
    if (!openId) return registry.register('loops', [create]);
    const loop = loops.detail?.loop.id === openId ? loops.detail.loop : null;
    const name = loop?.name ?? 'goal loop';
    const status = loop?.status;
    const resumable = status === 'paused' || status === 'blocked' || status === 'exhausted';
    const needsAnswer = loop?.ledger?.questions.some((q) => !q.answer) ?? false;
    return registry.register('loops', [
      create,
      { id: 'loops.pause', title: `Pause “${name}”`, group: 'Goal Loops', keywords: 'pause goal loop', disabled: status !== 'running', run: () => loopActions.pause() },
      { id: 'loops.resume', title: `Resume “${name}”`, group: 'Goal Loops', keywords: 'resume goal loop', disabled: !resumable || needsAnswer, run: () => loopActions.resume() },
      { id: 'loops.extend', title: `Extend budget for “${name}”`, group: 'Goal Loops', keywords: 'extend budget iterations runtime', disabled: !resumable || needsAnswer, run: () => loopActions.extend() },
      { id: 'loops.stop', title: `Stop “${name}”`, group: 'Goal Loops', keywords: 'stop goal loop', disabled: !loop || status === 'draft' || status === 'succeeded' || status === 'failed' || status === 'stopped', run: () => loopActions.stop() },
    ]);
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`loops`,});
  if(selectedId){
    
    selectedId; {
       { const $$_liateDpooL1C = __sveltets_2_ensureComponent(LoopDetail); new $$_liateDpooL1C({ target: __sveltets_2_any(), props: {      "id":selectedId,"onback":back,"actions":loopActions,}});}
    }
  } else if (creating){
     { const $$_mroFenifeDlaoG1C = __sveltets_2_ensureComponent(GoalDefineForm); new $$_mroFenifeDlaoG1C({ target: __sveltets_2_any(), props: {    "oncancel":() => (creating = false),"oncreated":open,}});}
  }else{
     { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {     "title":`Goal Loops`,"subtitle":`Agents iterate toward a goal within a budget`,actions:() => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_nottuBediuGetamotuA0C = __sveltets_2_ensureComponent(AutomateGuideButton); new $$_nottuBediuGetamotuA0C({ target: __sveltets_2_any(), props: {  "current":`loops`,}});}
        
        if(ws.currentId && list.length > 0){
           { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => (creating = true),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}    }
        }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
      
     PageHeader}
     { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: {children:() => { return __sveltets_2_any(0); },}});

    if(!ws.currentId){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {              "variant":`page`,"icon":`refresh`,"title":`Add a workspace to get started`,"body":`Goal loops belong to a workspace. Add your project folder, then define your goal and budget.`,"actionLabel":`Add workspace`,"actionIcon":`plus`,"onaction":() => (ui.newWorkspaceOpen = true),}});}
    }else{
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`goal loops`,"variant":`page`,"loading":loops.loadingList,"error":loops.listError,"empty":list.length === 0,"onretry":() => ws.currentId && void loops.loadList(ws.currentId),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
         { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"variant":`page`,"icon":`refresh`,"title":`No goal loops yet`,"body":`Define a goal and a budget — agents iterate on an isolated branch until it’s met.`,"actionLabel":`New goal loop`,"actionIcon":`plus`,"onaction":() => (creating = true),}});
           { const $$_ediuGetamotuA1C = __sveltets_2_ensureComponent(AutomateGuide); new $$_ediuGetamotuA1C({ target: __sveltets_2_any(), props: {  "current":`loops`,}});}
         EmptyState}
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("ul", { "class":`cards`,});
           for(let l of __sveltets_2_ensureArray(list)){l.id;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("button", {   "class":`loop-card`,"onclick":() => open(l.id),});
               { svelteHTML.createElement("div", { "class":`card-top`,});
                 { svelteHTML.createElement("span", {   "class":`name`,"title":l.name,});l.name; }
                 { const $$_egdaBsutatS7C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS7C({ target: __sveltets_2_any(), props: {  "status":loopStatus(l.status),}});}
               }
              if(l.definition?.summary){ { svelteHTML.createElement("span", {   "class":`goal`,"title":l.definition.summary,});l.definition.summary; }}
               { svelteHTML.createElement("div", {   "class":`bar`,"aria-hidden":`true`,}); { svelteHTML.createElement("span", {  "class":`bar-fill status-${l.status}`,});__sveltets_2_ensureType(String, Number, `${l.progress_pct}%`); } }
               { svelteHTML.createElement("div", { "class":`card-meta`,});
                 { svelteHTML.createElement("span", {}); l.current_iteration;  l.limits.max_iterations; }
                 { svelteHTML.createElement("span", {});l.progress_pct;  }
                if(l.status === 'running' && PHASE_LABEL[l.phase]){ { svelteHTML.createElement("span", { "class":`phase`,});PHASE_LABEL[l.phase]; }
                } else if (l.updated_at){ { svelteHTML.createElement("span", { "class":`when`,});  { const $$_emiTleR8C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR8C({ target: __sveltets_2_any(), props: {  "iso":l.updated_at,}});} }}
               }
             }
           }
        }
       }
     LoadState}
    }
     PageBody}
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LoopsPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LoopsPage__SvelteComponent_ = ReturnType<typeof LoopsPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default LoopsPage__SvelteComponent_;