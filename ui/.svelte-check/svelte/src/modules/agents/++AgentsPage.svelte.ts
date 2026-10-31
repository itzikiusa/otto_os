///<reference types="svelte" />
;// Agent Mode: tabbed split panes, tiled grid, or Mission Control work queue.

import { untrack } from 'svelte';
import Splits from './Splits.svelte';
import TiledView from './TiledView.svelte';
import WorkQueue from './WorkQueue.svelte';
import FirstRunCoach from './FirstRunCoach.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import Skeleton from '../../lib/components/Skeleton.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { router } from '../../lib/router.svelte';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  const tiled = $derived(ws.viewMode === 'tiled');
  const mission = $derived(ws.viewMode === 'mission');

  // First-run coach: a guided path from a fresh account to a launched agent.
  // Shown only on a truly empty Agents view (no agent sessions at all) and only
  // until the user dismisses it or launches a session (remembered per-machine).
  let coachDismissed = $state(
    (() => {
      try {
        return localStorage.getItem('otto_firstrun_dismissed') === '1';
      } catch {
        return false;
      }
    })(),
  );
  const showCoach = $derived(!coachDismissed && ws.agentSessions.length === 0);
  const preserveCoachDraft = $derived(
    showCoach && ws.currentId === null && !tiled && !mission && ws.panes.length === 0,
  );

  // Never open onto a "pick one" void when there are sessions: once per
  // workspace, if its restored layout has no panes, open the most recently
  // active session. Only on ARRIVAL — closing the last tab later leaves the
  // empty state alone (the user just chose that). An arrival that already
  // names its session (`#/agents/<id>` — a Home box, a notification, ⌘K) is
  // never overridden: its pane may not exist yet when this runs, and opening
  // the latest session here used to jump away from the one just clicked.
  let autoOpenedFor: string | null | undefined = undefined;
  $effect(() => {
    if (!ws.layoutReady || ws.sessionsLoading || ws.sessionsError) return;
    const key = ws.currentId;
    if (autoOpenedFor === key) return;
    autoOpenedFor = key;
    if (untrack(() => router.module === 'agents' && !!router.parts[1])) return;
    if (ws.panes.length > 0 || tiled || mission) return;
    const latest = [...ws.mainSessions].sort(
      (a, b) => Date.parse(b.last_active_at) - Date.parse(a.last_active_at),
    )[0];
    if (latest) ws.navigateToSession(latest.id);
  });
;
async () => {

 { svelteHTML.createElement("div", { "class":`agents`,});
   { svelteHTML.createElement("div", { "class":`agents-body`,});
  
  if(ws.sessionsError){
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`sessions`,"error":ws.sessionsError,"empty":true,"onretry":() => void ws.retrySessions(),}});}
  }
  if(ws.sessionsError && !ws.layoutReady){
    
  } else if (ws.sessionsLoading && ws.sessions.length === 0 && !preserveCoachDraft){
     { svelteHTML.createElement("div", { "style":`padding: 16px`,});
       { const $$_notelekS3C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS3C({ target: __sveltets_2_any(), props: {    "rows":3,"height":48,}});}
     }
  } else if (mission){
     { const $$_eueuQkroW2C = __sveltets_2_ensureComponent(WorkQueue); new $$_eueuQkroW2C({ target: __sveltets_2_any(), props: {}});}
  } else if (tiled){
     { const $$_weiVdeliT2C = __sveltets_2_ensureComponent(TiledView); new $$_weiVdeliT2C({ target: __sveltets_2_any(), props: {}});}
  } else if (ws.panes.length === 0){
    if(showCoach){
       { const $$_hcaoCnuRtsriF2C = __sveltets_2_ensureComponent(FirstRunCoach); new $$_hcaoCnuRtsriF2C({ target: __sveltets_2_any(), props: {  "ondismiss":() => (coachDismissed = true),}});}
    } else if (ws.activeSessions.length === 0){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {               "icon":`terminal`,"title":`No sessions yet`,"body":`Start an agent (Claude, Codex) or a plain shell in this workspace (⌘T). Sessions keep running even when you close the app.`,"variant":`page`,"actionIcon":`plus`,"actionLabel":`New session…`,"onaction":() => (ui.newSessionOpen = true),}});}
    }else{
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {               "icon":`terminal`,"title":`No open tabs`,"body":`Pick a session in the sidebar, switch to tiled view, or start a new one (⌘T).`,"variant":`page`,"actionIcon":`plus`,"actionLabel":`New session…`,"onaction":() => (ui.newSessionOpen = true),}});}
    }
  }else{
     { const $$_stilpS2C = __sveltets_2_ensureComponent(Splits); new $$_stilpS2C({ target: __sveltets_2_any(), props: {}});}
  }
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AgentsPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AgentsPage__SvelteComponent_ = ReturnType<typeof AgentsPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default AgentsPage__SvelteComponent_;