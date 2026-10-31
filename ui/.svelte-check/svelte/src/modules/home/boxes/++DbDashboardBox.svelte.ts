///<reference types="svelte" />
;// DB dashboard box: embeds one Database Explorer dashboard (its widgets, each
// running its own query on the dashboard's refresh cadence) via the same
// WidgetCard the Dashboards tab uses. The dashboard is chosen once (stored in
// the box config); an unset box shows an inline picker.

import { untrack } from 'svelte';
import EmptyState from '../../../lib/components/EmptyState.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../../lib/loadError';
import WidgetCard from '../../database/WidgetCard.svelte';
import { database } from '../../../lib/stores/database.svelte';
import { ws } from '../../../lib/stores/workspace.svelte';
import { router } from '../../../lib/router.svelte';
import { home, type HomeBox } from '../home.svelte';
;

  interface Props {
    box: HomeBox;
    viewId: string;
    zoomed: boolean;
    tick: number;
    /** False while this box's Home space is off screen (review 06 F3). */
    active?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { box, viewId, zoomed, tick, active = true }: Props = $props();

  // Starts true: a cold box shows the skeleton, never a flash of "No DB dashboards yet".
  let loading = $state(true);
  let loadError = $state('');
  let loadedFor = '';

  // Dashboards + widgets are workspace-scoped; load them once per workspace
  // (the Connections page does the same on mount). `connections` feeds the
  // per-widget connection chip.
  async function ensureLoaded(force = false): Promise<void> {
    const id = ws.currentId;
    if (!id) {
      loading = false;
      return;
    }
    if (loadedFor === id && !force) return;
    loadedFor = id;
    loading = true;
    loadError = '';
    try {
      if (database.connections.length === 0) await database.loadConnections();
      await database.loadDashboards();
      // The store keeps its failures in state (loadDashboards never throws) —
      // surface a failed connection load here so it can't read as "none yet".
      // A failed dashboards load is read live from the store (`error` below).
      if (database.connectionsError) loadError = loadErrorText(database.connectionsError);
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    void ws.currentId;
    if (!active) return;
    untrack(() => void ensureLoaded());
  });
  // Manual refresh (frame button): reload on a CHANGED tick only — on mount
  // the effect above is already loading, and a forced second pass doubled
  // connections + dashboards + widgets on every cold Home boot.
  let seenTick = untrack(() => tick);
  $effect(() => {
    const t = tick;
    if (t === seenTick) return;
    seenTick = t;
    untrack(() => void ensureLoaded(true));
  });

  // A failed dashboards load (a daemon error, a timeout) must not fall through
  // to "No DB dashboards yet" with a create CTA and no way to retry.
  const error = $derived(loadError || (database.dashboardsError ?? ''));

  const dashboardId = $derived(typeof box.config.dashboardId === 'string' ? box.config.dashboardId : '');
  const dashboard = $derived(database.dashboards.find((d) => d.id === dashboardId) ?? null);
  const widgets = $derived(dashboardId ? database.widgets.filter((w) => w.dashboard_id === dashboardId) : []);

  function pick(id: string): void {
    home.updateBoxConfig(viewId, box.id, { dashboardId: id });
  }
;
async () => {

 { svelteHTML.createElement("div", {  "class":`dbd`,});zoomed;
  if((loading || error) && database.dashboards.length === 0){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {          "what":`DB dashboards`,"variant":`compact`,loading,error,"empty":true,"onretry":() => void ensureLoaded(true),}});}
  } else if (database.dashboards.length === 0){
    
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"icon":`db`,"title":`No DB dashboards yet`,"body":`Create one in Connections → a database → Dashboards, then pick it here.`,}});
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => router.go('connections'),});  }
     EmptyState}
  } else if (!dashboard){
     { svelteHTML.createElement("div", { "class":`picker`,});
       { svelteHTML.createElement("p", {});      }
       { svelteHTML.createElement("select", {     "aria-label":`Dashboard`,"class":`input`,"value":"","onchange":(e) => pick((e.currentTarget as HTMLSelectElement).value),});
         { svelteHTML.createElement("option", {"value":"","disabled":true,});   }
           for(let d of __sveltets_2_ensureArray(database.dashboards)){d.id;
           { svelteHTML.createElement("option", { "value":d.id,});d.name; }
        }
       }
     }
  } else if (widgets.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`db`,"title":`“${dashboard.name}” has no widgets`,"body":`Add widgets from a query result in the Database Explorer.`,}});}
  }else{
     { svelteHTML.createElement("div", { "class":`grid`,});
         for(let w of __sveltets_2_ensureArray(widgets)){w.id;
         { const $$_draCtegdiW2C = __sveltets_2_ensureComponent(WidgetCard); new $$_draCtegdiW2C({ target: __sveltets_2_any(), props: {     "widget":w,"refreshSecs":dashboard.refresh_secs ?? null,active,}});}
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const DbDashboardBox__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type DbDashboardBox__SvelteComponent_ = ReturnType<typeof DbDashboardBox__SvelteComponent_>;
/*Ωignore_endΩ*/export default DbDashboardBox__SvelteComponent_;