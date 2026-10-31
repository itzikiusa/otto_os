///<reference types="svelte" />
;// Superset-style dashboards: pick/create/rename/delete a dashboard, set an
// auto-refresh cadence, and render its widgets in a responsive grid. New
// widgets are added from a query result via the "Add widget" sheet.

import Icon from '../../lib/components/Icon.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import Modal from '../../lib/components/Modal.svelte';
import WidgetCard from './WidgetCard.svelte';
import { database } from '../../lib/stores/database.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { confirmer } from '../../lib/confirm.svelte';
import type { DbViz, DbWidget } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  const canEdit = $derived(ws.myRole !== 'viewer');
  const dashboard = $derived(database.selectedDashboard);
  const widgets = $derived(database.widgetsForSelectedDashboard());

  const REFRESH_OPTS: { label: string; secs: number | null }[] = [
    { label: 'Off', secs: null },
    { label: '10s', secs: 10 },
    { label: '30s', secs: 30 },
    { label: '1m', secs: 60 },
    { label: '5m', secs: 300 },
  ];

  async function newDashboard(): Promise<void> {
    const name = await confirmer.promptText('Dashboard name', {
      title: 'New dashboard',
      confirmLabel: 'Create',
    });
    if (name) await database.createDashboard(name);
  }
  async function rename(): Promise<void> {
    if (!dashboard) return;
    const name = await confirmer.promptText('Rename dashboard', {
      title: 'Rename dashboard',
      confirmLabel: 'Rename',
      initial: dashboard.name,
    });
    if (name && name !== dashboard.name) await database.renameDashboard(dashboard.id, name);
  }
  async function remove(): Promise<void> {
    if (!dashboard) return;
    if (await confirmer.ask(`Delete dashboard “${dashboard.name}” and its widgets?`, { title: 'Delete dashboard', confirmLabel: 'Delete', danger: true })) {
      await database.deleteDashboard(dashboard.id);
    }
  }

  // ── Add / edit widget sheet (same form; `editing` switches the save path) ──
  let adding = $state(false);
  let editing = $state<DbWidget | null>(null);
  let wTitle = $state('');
  let wStatement = $state('');
  let wViz = $state<DbViz>('table');
  let wX = $state('');
  let wY = $state('');
  let saving = $state(false);

  function openAdd(): void {
    // Seed from the active query tab so "save this result as a widget" is one step.
    const t = database.tab;
    wStatement = t.statement;
    wTitle = t.name && t.name !== 'Query' ? t.name : '';
    wX = t.result?.columns[0]?.name ?? '';
    wY = t.result?.columns[1]?.name ?? '';
    wViz = 'table';
    editing = null;
    adding = true;
  }

  function openEdit(w: DbWidget): void {
    wTitle = w.title;
    wStatement = w.statement;
    wViz = w.viz;
    wX = w.mapping.x ?? w.mapping.category ?? '';
    wY = w.mapping.y?.[0] ?? w.mapping.value ?? '';
    editing = w;
    adding = true;
  }

  // The connection the edited widget runs on (fixed at creation — shown so a
  // widget silently bound to the "wrong" focused connection is at least visible).
  const editingConnName = $derived.by(() => {
    const w = editing;
    if (!w) return null;
    return database.connections.find((c) => c.id === w.connection_id)?.name ?? w.connection_id;
  });

  const VIZ: DbViz[] = ['table', 'number', 'line', 'bar', 'area', 'pie'];
  const needsAxes = $derived(wViz === 'line' || wViz === 'bar' || wViz === 'area');
  // Axis options: Add seeds from the active Query tab (its result IS the seeded
  // statement); Edit offers the widget's OWN last-run columns — the Query tab
  // may hold an unrelated result. A widget maps axes by column NAME, so the
  // option set is the distinct names — a join result (`SELECT * … JOIN …`)
  // repeats `id`, and a duplicate would also be a duplicate `{#each}` key,
  // which throws and blanks the sheet.
  const resultColNames = $derived.by(() => {
    const w = editing;
    if (w) return database.dash.widgetColumns[w.id] ?? [];
    return [...new Set((database.tab.result?.columns ?? []).map((c) => c.name))];
  });
  // The connection a NEW widget binds to — the focused Explorer connection.
  const addConnName = $derived(database.selectedConn?.name ?? null);
  const addBlocked = $derived(!editing && !database.selectedConnId);

  async function saveWidget(): Promise<void> {
    if (!dashboard || addBlocked || !wTitle.trim() || !wStatement.trim()) return;
    saving = true;
    const mapping = {
      x: wX || undefined,
      y: wY ? [wY] : undefined,
      category: wViz === 'pie' ? wX || undefined : undefined,
      value: wViz === 'number' ? wY || undefined : undefined,
    };
    try {
      if (editing) {
        // updateWidget swaps the store's widget object, which re-runs the tile's
        // query via WidgetCard's $effect — no explicit refresh needed here. A
        // rejected (write verb) or failed save returns null and has already
        // toasted: keep the sheet open so the edit isn't lost.
        const w = await database.updateWidget(editing.id, {
          title: wTitle.trim(),
          statement: wStatement.trim(),
          viz: wViz,
          mapping,
        });
        if (w) {
          adding = false;
          editing = null;
        }
        return;
      }
      const w = await database.createWidget({
        title: wTitle.trim(),
        statement: wStatement.trim(),
        viz: wViz,
        mapping,
        dashboard_id: dashboard.id,
      });
      if (w) adding = false;
    } finally {
      saving = false;
    }
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`dash`,});
   { svelteHTML.createElement("div", { "class":`dash-bar`,});
     { svelteHTML.createElement("select", {           "class":`input dash-pick`,"value":dashboard?.id ?? '',"onchange":(e) => (database.selectedDashboardId = (e.currentTarget as HTMLSelectElement).value || null),"disabled":database.dashboards.length === 0,"aria-label":`Dashboard`,});
      if(database.dashboards.length === 0){
         { svelteHTML.createElement("option", {"value":"",});
          database.dashboardsLoading ? 'Loading dashboards…' : database.dashboardsError ? 'Couldn’t load dashboards' : 'No dashboards';
         }
      }
         for(let d of __sveltets_2_ensureArray(database.dashboards)){d.id;
         { svelteHTML.createElement("option", { "value":d.id,});d.name; }
      }
     }

    if(canEdit){
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":newDashboard,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
    }

    if(dashboard){
      if(canEdit){
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":rename,"title":`Rename`,"aria-label":`Rename dashboard`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":13,}});} }
         { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":remove,"title":`Delete`,"aria-label":`Delete dashboard`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":13,}});} }
      }
       { svelteHTML.createElement("span", { "class":`grow`,}); }
       { svelteHTML.createElement("div", { "class":`refresh-pick`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
         { svelteHTML.createElement("select", {        "aria-label":`Auto-refresh interval`,"class":`input`,"value":String(dashboard.refresh_secs ?? ''),"onchange":(e) => {
            const v = (e.currentTarget as HTMLSelectElement).value;
            void database.setDashboardRefresh(dashboard.id, v === '' ? null : Number(v));
          },});
             for(let o of __sveltets_2_ensureArray(REFRESH_OPTS)){o.label;
             { svelteHTML.createElement("option", { "value":o.secs == null ? '' : String(o.secs),});o.label; }
          }
         }
       }
      if(canEdit){
         { svelteHTML.createElement("button", {   "class":`btn small primary`,"onclick":openAdd,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }
      }
    }
   }

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`dashboards`,"variant":`page`,"loading":database.dashboardsLoading,"error":database.dashboardsError,"empty":database.dashboards.length === 0,"onretry":() => void database.loadDashboards(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {           "icon":`grid`,"title":`No dashboards yet`,"body":`Create a dashboard, then add widgets from your query results to build live charts.`,"actionLabel":canEdit ? 'New dashboard' : undefined,"onaction":canEdit ? newDashboard : undefined,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
    if(dashboard && widgets.length === 0){
       { const $$_etatSytpmE2C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE2C({ target: __sveltets_2_any(), props: {           "icon":`box`,"title":`No widgets`,"body":`Run a query in the Query tab, then click “Add widget” to chart it here.`,"actionLabel":canEdit ? 'Add widget' : undefined,"onaction":canEdit ? openAdd : undefined,}});}
    } else if (dashboard){
       { svelteHTML.createElement("div", { "class":`widget-grid`,});
           for(let w of __sveltets_2_ensureArray(widgets)){w.id;
           { const $$_draCtegdiW3C = __sveltets_2_ensureComponent(WidgetCard); new $$_draCtegdiW3C({ target: __sveltets_2_any(), props: {      "widget":w,"refreshSecs":dashboard.refresh_secs,"onedit":canEdit ? openEdit : undefined,}});}
        }
       }
    }
   LoadState}
 }

if(adding){
   { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); const $$_ladoM0 = new $$_ladoM0C({ target: __sveltets_2_any(), props: {     children:() => { return __sveltets_2_any(0); },"title":editing ? 'Edit widget' : 'Add widget',"width":520,"onclose":() => (adding = false),footer:() => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("button", {   "class":`btn`,"onclick":() => (adding = false),});  }
       { svelteHTML.createElement("button", {         "class":`btn primary`,"disabled":saving || addBlocked || !wTitle.trim() || !wStatement.trim(),"title":addBlocked ? 'Select a connection in Database Explorer first' : undefined,"onclick":saveWidget,});
        saving ? 'Saving…' : editing ? 'Save' : 'Add widget';
       }
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {footer} = $$_ladoM0.$$prop_def;/*Ωignore_endΩ*/
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`w-title`,});  }
       { svelteHTML.createElement("input", {          "dir":`auto`,"id":`w-title`,"class":`input`,"bind:value":wTitle,"placeholder":`Daily signups`,});/*Ωignore_startΩ*/() => wTitle = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     }
    if(editing){
       { svelteHTML.createElement("p", {   "class":`conn-note`,"title":editing.connection_id,});
            { svelteHTML.createElement("strong", {});editingConnName; }   
       }
    } else if (addConnName){
       { svelteHTML.createElement("p", {   "class":`conn-note`,"title":database.selectedConnId ?? '',});
           { svelteHTML.createElement("strong", {});addConnName; }            
       }
    }else{
       { svelteHTML.createElement("p", { "class":`conn-note`,});              }
    }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("label", { "for":`w-stmt`,});  }
       { svelteHTML.createElement("textarea", {           "dir":`ltr`,"id":`w-stmt`,"class":`input mono`,"rows":4,"bind:value":wStatement,"spellcheck":`false`,});/*Ωignore_startΩ*/() => wStatement = __sveltets_2_any(null);/*Ωignore_endΩ*/ }
     }
     { svelteHTML.createElement("div", { "class":`field`,});
       { svelteHTML.createElement("span", {   "class":`field-label`,"id":`w-viz-label`,});  }
       { svelteHTML.createElement("div", {     "class":`viz-row`,"role":`group`,"aria-labelledby":`w-viz-label`,});
           for(let v of __sveltets_2_ensureArray(VIZ)){v;
           { svelteHTML.createElement("button", {        "type":`button`,"class":`viz-chip`,"aria-pressed":wViz === v,"onclick":() => (wViz = v),});wViz === v;v; }
        }
       }
     }
    if(needsAxes || wViz === 'pie' || wViz === 'number'){
       { svelteHTML.createElement("div", { "class":`field-row`,});
        if(needsAxes || wViz === 'pie'){
           { svelteHTML.createElement("div", { "class":`field grow`,});
             { svelteHTML.createElement("label", { "for":`w-x`,});wViz === 'pie' ? 'Category' : 'X axis'; }
             { svelteHTML.createElement("select", {     "id":`w-x`,"class":`input`,"bind:value":wX,});/*Ωignore_startΩ*/() => wX = __sveltets_2_any(null);/*Ωignore_endΩ*/
               { svelteHTML.createElement("option", {"value":"",});  }
              if(wX && !resultColNames.includes(wX)){ { svelteHTML.createElement("option", { "value":wX,});wX; }}
                 for(let name of __sveltets_2_ensureArray(resultColNames)){name; { svelteHTML.createElement("option", { "value":name,});name; }}
             }
           }
        }
         { svelteHTML.createElement("div", { "class":`field grow`,});
           { svelteHTML.createElement("label", { "for":`w-y`,});wViz === 'number' ? 'Value' : wViz === 'pie' ? 'Value' : 'Y series'; }
           { svelteHTML.createElement("select", {     "id":`w-y`,"class":`input`,"bind:value":wY,});/*Ωignore_startΩ*/() => wY = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});  }
            if(wY && !resultColNames.includes(wY)){ { svelteHTML.createElement("option", { "value":wY,});wY; }}
               for(let name of __sveltets_2_ensureArray(resultColNames)){name; { svelteHTML.createElement("option", { "value":name,});name; }}
           }
         }
       }
      if(resultColNames.length === 0){
         { svelteHTML.createElement("p", { "class":`axes-hint`,});
          editing
            ? 'Columns come from this widget’s last run. Refresh it once to map specific columns; otherwise columns are auto-detected.'
            : 'Run the query once in the Query tab to map specific columns; otherwise columns are auto-detected.';
         }
      } else if (editing){
         { svelteHTML.createElement("p", { "class":`axes-hint`,});       }
      }
    }
    
   Modal}
}


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Dashboards__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Dashboards__SvelteComponent_ = ReturnType<typeof Dashboards__SvelteComponent_>;
/*Ωignore_endΩ*/export default Dashboards__SvelteComponent_;