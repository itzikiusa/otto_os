///<reference types="svelte" />
;
import { mapLimit } from '../../lib/poll';
import { rowMenu } from '../../lib/rowMenu';
// Personal Agents module. Routes: `#/personal-agents` (agent cards),
// `#/personal-agents/rooms` (agent channels), `#/personal-agents/<agentId>[/<tab>]`
// (one agent's page). The first list GET seeds four disabled example agents
// server-side — they render as normal cards, marked "Example".
import RelTime from '../../lib/components/RelTime.svelte';
import { onTabKey } from '../../lib/tabKeys';
import { personalAgents } from '../../lib/stores/personalAgents.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import { confirmRunAndDeliver } from '../../lib/confirmOutward';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import { ws } from '../../lib/stores/workspace.svelte';
import { ui } from '../../lib/stores/ui.svelte';
import { router } from '../../lib/router.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { registry } from '../../lib/commands.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import AutomateGuide from '../../lib/components/AutomateGuide.svelte';
import AutomateGuideButton from '../../lib/components/AutomateGuideButton.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import PageHeader from '../../lib/components/PageHeader.svelte';
import PageBody from '../../lib/components/PageBody.svelte';
import Icon from '../../lib/components/Icon.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import AgentAvatar from './AgentAvatar.svelte';
import { lazyComponent } from '../../lib/lazy-component.svelte';

import Skeleton from '../../lib/components/Skeleton.svelte';
import RoomsView from './RoomsView.svelte';
import { loadErrorOf } from './loadError';
import { personalAgentsApi } from '../../lib/api/personalAgents';
import type { PersonalAgent, PersonalAgentAutonomy } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  
  
  
  
  

  const AgentEditSheetLazy = lazyComponent(() => import('./AgentEditSheet.svelte'));
  const AgentPageLazy = lazyComponent(() => import('./AgentPage.svelte'));

  const sub = $derived(router.parts[1] ?? '');
  const agentId = $derived(sub && sub !== 'rooms' ? sub : null);

  let creating = $state(false);
  /** Agent id whose Run now / Enable / Pause is in flight (no double fire). */
  let busyId = $state<string | null>(null);

  $effect(() => {
    if (ws.currentId) void personalAgents.loadAgents(ws.currentId);
  });

  // Live run events refetch runs/schedules only while this page is mounted.
  $effect(() => personalAgents.watch());

  // Autonomy summaries (primary / proactive) for the cards — one small GET per
  // agent, best-effort (a failure just hides the badges).
  let autonomyById = $state<Record<string, PersonalAgentAutonomy>>({});
  const agentIdsKey = $derived(personalAgents.agents.map((a) => a.id).join(','));
  $effect(() => {
    const ids = agentIdsKey ? agentIdsKey.split(',') : [];
    let disposed = false;
    autonomyById = {};
    void mapLimit(ids, 2, async (id) => {
      if (disposed) return;
      try {
        const autonomy = await personalAgentsApi.autonomy(id);
        if (!disposed) autonomyById = { ...autonomyById, [id]: autonomy };
      } catch { /* Badge details are best-effort. */ }
    });
    return () => { disposed = true; };
  });
  // "Your agent" (primary) leads the list — the one entry point.
  const agents = $derived(
    [...personalAgents.agents].sort(
      (a, b) => Number(!!autonomyById[b.id]?.primary) - Number(!!autonomyById[a.id]?.primary),
    ),
  );
  const primaryAgent = $derived(agents.find((a) => autonomyById[a.id]?.primary) ?? null);
  const loadError = $derived(loadErrorOf(personalAgents, 'agentsError'));

  // ⌘K verbs while the module is open: create, the rooms tab, and one
  // Open / Chat pair per agent.
  $effect(() =>
    registry.register('personal-agents', [
      { id: 'personal-agents.new', title: 'New personal agent…', group: 'Personal Agents', keywords: 'create agent assistant schedule', run: () => { if (sub) router.go('personal-agents'); creating = true; } },
      { id: 'personal-agents.rooms', title: 'Open agent channels', group: 'Personal Agents', keywords: 'channel agent rooms talk', run: () => router.go('personal-agents/rooms') },
      ...agents.flatMap((a) => [
        { id: `personal-agents.open.${a.id}`, title: `Open ${a.name}`, group: 'Personal Agents', detail: 'Personal agent', keywords: 'agent page settings runs', run: () => router.go(`personal-agents/${a.id}`) },
        { id: `personal-agents.chat.${a.id}`, title: `Chat with ${a.name}`, group: 'Personal Agents', keywords: 'agent message talk', run: () => router.go(`personal-agents/${a.id}/chat`) },
      ]),
    ]),
  );

  async function toggle(a: PersonalAgent): Promise<void> {
    busyId = a.id;
    try {
      await personalAgents.setEnabled(a.id, !a.enabled);
    } catch (e) {
      toasts.error(a.enabled ? `Couldn’t pause ${a.name}` : `Couldn’t enable ${a.name}`, loadErrorText(e));
    } finally {
      busyId = null;
    }
  }

  async function runNow(a: PersonalAgent): Promise<void> {
    // Outward-facing: the run delivers its report the moment it finishes.
    if (!(await confirmRunAndDeliver(a.name, a.delivery))) return;
    busyId = a.id;
    try {
      await personalAgents.runNow(a.id);
      toasts.success(`${a.name} is running`, 'Its report lands in Runs when it finishes.');
      router.go(`personal-agents/${a.id}/runs`);
    } catch (e) {
      toasts.error(`Couldn’t run ${a.name}`, loadErrorText(e));
    } finally {
      busyId = null;
    }
  }

  async function remove(a: PersonalAgent): Promise<void> {
    if (!(await confirmer.ask(`Delete personal agent “${a.name}”? Its schedules, memory and run history go with it.`, { title: 'Delete personal agent', danger: true, confirmLabel: 'Delete' }))) return;
    try {
      await personalAgents.remove(a.id);
      toasts.success(`Deleted ${a.name}`);
    } catch (e) {
      toasts.error(`Couldn’t delete ${a.name}`, loadErrorText(e));
    }
  }

  function cardMenu(e: MouseEvent | KeyboardEvent, a: PersonalAgent): void {
    ctxMenu.show(e, [
      { label: 'Open', icon: 'user', action: () => router.go(`personal-agents/${a.id}`) },
      { label: 'Run now', icon: 'play', action: () => void runNow(a) },
      { label: a.enabled ? 'Pause' : 'Enable', icon: a.enabled ? 'clock' : 'check', action: () => void toggle(a) },
      { separator: true },
      { label: 'Delete…', icon: 'trash', danger: true, action: () => void remove(a) },
    ]);
  }

  /** A seeded example: disabled, never run, still carrying its shipped persona. */
  function isExample(a: PersonalAgent): boolean {
    return !a.enabled && !a.chat_session_id && (personalAgents.runsByAgent[a.id]?.length ?? 0) === 0;
  }

  function providerText(a: PersonalAgent): string {
    return a.model ? `${a.provider} · ${a.model}` : a.provider;
  }

;
async () => {

if(agentId){
  agentId; {
    if(AgentPageLazy.error){
       { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {        "what":`this agent`,"variant":`page`,"error":AgentPageLazy.error,"onretry":() => AgentPageLazy.retry(),}});}
    } else if (AgentPageLazy.component){
       { const $$_tnenopmoc_yzaLegaPtnegA0C = __sveltets_2_ensureComponent(AgentPageLazy.component); new $$_tnenopmoc_yzaLegaPtnegA0C({ target: __sveltets_2_any(), props: { agentId,}});}
    }else{
        { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {    "what":`this agent`,"variant":`page`,"loading":true,}});}
    }
  }
}else{
   { svelteHTML.createElement("div", { "class":`pa-page`,});
   { const $$_redaeHegaP1C = __sveltets_2_ensureComponent(PageHeader); const $$_redaeHegaP1 = new $$_redaeHegaP1C({ target: __sveltets_2_any(), props: {     "title":`Personal Agents`,"subtitle":`Named agents with their own persona, schedules and memory`,tabs:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("div", {         "class":`segmented`,"role":`tablist`,"aria-label":`Personal agents view`,"tabindex":-1,"onkeydown":onTabKey,});
         { svelteHTML.createElement("button", {        "role":`tab`,"aria-selected":sub !== 'rooms',"tabindex":sub !== 'rooms' ? 0 : -1,"onclick":() => router.go('personal-agents'),});sub !== 'rooms';  }
         { svelteHTML.createElement("button", {        "role":`tab`,"aria-selected":sub === 'rooms',"tabindex":sub === 'rooms' ? 0 : -1,"onclick":() => router.go('personal-agents/rooms'),});sub === 'rooms';  }
       }
    };return __sveltets_2_any(0)},actions:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_nottuBediuGetamotuA0C = __sveltets_2_ensureComponent(AutomateGuideButton); new $$_nottuBediuGetamotuA0C({ target: __sveltets_2_any(), props: {  "current":`personal-agents`,}});}
      
      if(ws.currentId && sub !== 'rooms' && agents.length > 0){
         { svelteHTML.createElement("button", {     "class":`btn small primary`,...__sveltets_2_empty({"data-icon":`plus`}),"onclick":() => (creating = true),}); { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
      }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {tabs, actions} = $$_redaeHegaP1.$$prop_def;/*Ωignore_endΩ*/
    
    
   PageHeader}
   { const $$_ydoBegaP1C = __sveltets_2_ensureComponent(PageBody); new $$_ydoBegaP1C({ target: __sveltets_2_any(), props: { children:() => { return __sveltets_2_any(0); },"fill":sub === 'rooms',}});
   { svelteHTML.createElement("div", { "class":`pa`,});
    if(!ws.currentId){
       { const $$_etatSytpmE3C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE3C({ target: __sveltets_2_any(), props: {              "variant":`page`,"icon":`user`,"title":`Add a workspace to get started`,"body":`Personal agents and their channels belong to a workspace. Add your project folder to create them.`,"actionLabel":`Add workspace`,"actionIcon":`plus`,"onaction":() => (ui.newWorkspaceOpen = true),}});}
    } else if (sub === 'rooms'){
       { const $$_weiVsmooR3C = __sveltets_2_ensureComponent(RoomsView); new $$_weiVsmooR3C({ target: __sveltets_2_any(), props: {}});}
    }else{
       { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL3 = new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"what":`personal agents`,"loading":personalAgents.loadingAgents,"error":loadError,"empty":agents.length === 0,"variant":`page`,"rows":3,"onretry":() => ws.currentId && void personalAgents.loadAgents(ws.currentId),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
           { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {               children:() => { return __sveltets_2_any(0); },"icon":`user`,"title":`No personal agents yet`,"body":`A personal agent is a named persona on a pinned provider and model, with its own schedules, memory and delivery. Agents talk to each other only in channels you can read. Start blank or from a template.`,"actionLabel":`New agent`,"actionIcon":`plus`,"variant":`page`,"onaction":() => (creating = true),}});
             { const $$_ediuGetamotuA1C = __sveltets_2_ensureComponent(AutomateGuide); new $$_ediuGetamotuA1C({ target: __sveltets_2_any(), props: {  "current":`personal-agents`,}});}
           EmptyState}
        };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL3.$$prop_def;/*Ωignore_endΩ*/
        
        if(!primaryAgent && Object.keys(autonomyById).length > 0){
           { svelteHTML.createElement("p", {   "class":`pa-hint`,"role":`note`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`star`,"size":12,}});}      { svelteHTML.createElement("strong", {});  }             }
        }
         { svelteHTML.createElement("ul", {   "class":`cards`,...__sveltets_2_empty({"data-testid":`pa-cards`}),});
             for(let a of __sveltets_2_ensureArray(agents)){a.id;
            const example = isExample(a);
            const auto = autonomyById[a.id];
             {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('li')));{ svelteHTML.createElement("li", __sveltets_2_union($$action_0), {      "class":`pa-card`,"oncontextmenu":(e) => cardMenu(e, a),});!a.enabled;!!auto?.primary;
               { svelteHTML.createElement("div", { "class":`card-top`,});
                 { const $$_ratavAtnegA7C = __sveltets_2_ensureComponent(AgentAvatar); new $$_ratavAtnegA7C({ target: __sveltets_2_any(), props: {      "avatar":a.avatar,"name":a.name,"size":36,}});}
                 { svelteHTML.createElement("div", { "class":`card-id`,});
                  
                   { svelteHTML.createElement("button", {     "class":`name`,"title":a.name,"onclick":() => router.go(`personal-agents/${a.id}`),});a.name; }
                   { svelteHTML.createElement("span", {   "class":`prov`,"title":providerText(a),});
                     { const $$_nocIredivorP9C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP9C({ target: __sveltets_2_any(), props: {    "provider":a.provider,"size":12,}});} { svelteHTML.createElement("span", { "class":`prov-t`,});providerText(a); }
                   }
                 }
                 { svelteHTML.createElement("button", {        "class":`icon-btn card-more`,"aria-label":`More actions for ${a.name}`,"title":`More actions`,"onclick":(e) => cardMenu(e, a),}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});} }
               }
               { svelteHTML.createElement("div", { "class":`card-meta`,});
                if(example){
                   { svelteHTML.createElement("span", {   "class":`chip`,"title":`A shipped example — it stays off until you enable it. Run now still runs it once.`,});  }
                } else if (!a.enabled){
                   { svelteHTML.createElement("span", {   "class":`chip`,"title":`Schedules don’t fire while paused. Run now and chat still work.`,});  }
                }else{
                   { svelteHTML.createElement("span", { "class":`chip ok`,});  }
                }
                if(a.browser){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`Runs and chat can drive the in-app browser`,});  }}
                if(auto?.primary){ { svelteHTML.createElement("span", {   "class":`chip pa-accent`,"title":`Your primary assistant`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`star`,"size":12,}});}   }}
                if(auto?.proactive.enabled){ { svelteHTML.createElement("span", {   "class":`chip`,"title":`Works its standing goals in the background, read-only`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});}  }}
                
                if(a.enabled){
                  if(personalAgents.schedulesError[a.id]){
                     { svelteHTML.createElement("span", {   "class":`meta`,"title":personalAgents.schedulesError[a.id],});       }
                  } else if (personalAgents.schedulesLoading[a.id] && !(a.id in personalAgents.schedulesByAgent)){
                     { const $$_notelekS7C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS7C({ target: __sveltets_2_any(), props: {      "rows":1,"height":16,"label":`schedule`,}});}
                  }else{
                     { svelteHTML.createElement("span", { "class":`meta`,});   { const $$_emiTleR8C = __sveltets_2_ensureComponent(RelTime); new $$_emiTleR8C({ target: __sveltets_2_any(), props: {    "iso":personalAgents.nextRunAt(a.id),"fallback":`not scheduled`,}});} }
                  }
                }
               }
               { svelteHTML.createElement("div", { "class":`card-actions`,});
                if(auto?.primary){
                   { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":() => router.go(`personal-agents/${a.id}/chat`),}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`comment`,"size":12,}});}  }
                }
                if(a.enabled){
                   { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":busyId === a.id,"aria-label":`Run now ${a.name}`,"onclick":() => void runNow(a),}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}   }
                   { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":busyId === a.id,"aria-label":`Pause ${a.name}`,"onclick":() => void toggle(a),});  }
                }else{
                   { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":busyId === a.id,"aria-label":`Enable ${a.name}`,"onclick":() => void toggle(a),}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}  }
                   { svelteHTML.createElement("button", {         "class":`btn small`,"disabled":busyId === a.id,"aria-label":`Run once ${a.name}`,"onclick":() => void runNow(a),"title":`Run it once without enabling its schedules`,});  }
                }
               }
             }}
          }
         }
       LoadState}
    }

    if(creating){
      if(AgentEditSheetLazy.component){
         { const $$_tnenopmoc_yzaLteehStidEtnegA3C = __sveltets_2_ensureComponent(AgentEditSheetLazy.component); new $$_tnenopmoc_yzaLteehStidEtnegA3C({ target: __sveltets_2_any(), props: {    "agent":null,"onclose":() => (creating = false),}});}
      }else{
         { const $$_etatSdaoL3C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL3C({ target: __sveltets_2_any(), props: {        "what":`new-agent form`,"error":AgentEditSheetLazy.error,"loading":!AgentEditSheetLazy.error,"empty":true,"onretry":() => AgentEditSheetLazy.retry(),}});}
      }
    }
   }
   PageBody}
   }
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PersonalAgentsPage__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PersonalAgentsPage__SvelteComponent_ = ReturnType<typeof PersonalAgentsPage__SvelteComponent_>;
/*Ωignore_endΩ*/export default PersonalAgentsPage__SvelteComponent_;