///<reference types="svelte" />
;// Keep only a source identity. Open/parked query tabs are already live in the
// Explorer store: no extra document, query, copied rows or editing callbacks.

import type { Snippet } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import EnvBadge from '../../lib/components/EnvBadge.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { database } from '../../lib/stores/database.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { registry } from '../../lib/commands.svelte';
import { toasts } from '../../lib/toast.svelte';
import ResultsGrid from './ResultsGrid.svelte';

;type $$ComponentProps =  { children: Snippet; enabled?: boolean };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { children, enabled = true }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();
  let pinned = $state<{ connId: string; tabId: number; scope: string } | null>(null);
  const scope = $derived(JSON.stringify([ws.currentId, auth.me?.id, database.accessRevision]));
  const tabs = $derived(database.openQueryTabs());
  const source = $derived(pinned?.scope === scope
    ? tabs.find(({ connId, tab }) => connId === pinned?.connId && tab.id === pinned?.tabId) ?? null
    : null);
  const connection = $derived(source ? database.connections.find((c) => c.id === source.connId) : null);
  const submitted = $derived(source?.tab.error ? source.tab.err_statement : source?.tab.ran_statement);
  const active = $derived(tabs.find(({ connId, tab }) => connId === database.selectedConnId && tab.id === database.tab?.id));

  // Closure or access/identity changes remove both the reference and any modal
  // the grid had opened. Reopening an identically named tab never re-pins it.
  $effect(() => { if (pinned && !source) pinned = null; });

  function pinActive(): void {
    if (active) {
      database.setMainTab('query');
      pinned = { connId: active.connId, tabId: active.tab.id, scope };
    }
  }
  function selectSource(value: string): void {
    const next = tabs.find(({ connId, tab }) => `${connId}:${tab.id}` === value);
    pinned = next ? { connId: next.connId, tabId: next.tab.id, scope } : null;
  }
  async function focusSource(): Promise<void> {
    if (!source) return;
    try { await database.focusTab(source.tab.id); }
    catch { toasts.error('Couldn’t focus the source', 'Open its connection tab and try again.'); }
  }
  function sourceLabel(connId: string, name: string): string {
    const connectionName = database.connections.find((c) => c.id === connId)?.name ?? 'Connection';
    return `${connectionName} · ${name || 'Query'}`;
  }
  $effect(() => registry.register('database-comparison', [
    { id: 'db.compare-results', title: source ? 'Clear comparison' : 'Compare results', group: 'Database', keywords: 'compare connection query results side by side', run: () => { if (source) pinned = null; else pinActive(); } },
    ...(source ? [{ id: 'db.comparison-focus', title: 'Focus comparison source', group: 'Database', keywords: 'compare connection query edit', run: focusSource }] : []),
  ]));
;
async () => {

 { svelteHTML.createElement("div", {  "class":`comparison-workbench`,});enabled && source !== null;
  if(enabled){
   { svelteHTML.createElement("div", { "class":`comparison-tools`,});
     { svelteHTML.createElement("button", {         "class":`btn small ghost`,"aria-pressed":source !== null,"disabled":!active,"onclick":() => source ? (pinned = null) : pinActive(),"title":`Keep loaded results beside the current query`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`columns`,"size":13,}});} 
     }
    if(source){ { svelteHTML.createElement("span", { "class":`comparison-note`,});     }}
   }
  }
   { svelteHTML.createElement("div", { "class":`comparison-content`,});
     { svelteHTML.createElement("div", { "class":`comparison-editor`,});;__sveltets_2_ensureSnippet(children()); }
    if(enabled && source && connection){
       { svelteHTML.createElement("aside", {     "class":`comparison-reference`,"aria-label":`Comparison results`,...__sveltets_2_empty({"data-testid":`connection-comparison`}),});
         { svelteHTML.createElement("div", { "class":`reference-head`,});
           { svelteHTML.createElement("span", { "class":`reference-title`,});  }
           { const $$_egdaBvnE4C = __sveltets_2_ensureComponent(EnvBadge); new $$_egdaBvnE4C({ target: __sveltets_2_any(), props: {    "env":connection.environment,"readOnly":connection.read_only,}});}
           { svelteHTML.createElement("span", { "class":`reference-grow`,}); }
           { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => void focusSource(),"title":`Open this source tab for editing`,});  }
           { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => (pinned = null),"aria-label":`Clear comparison`,"title":`Clear comparison`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":13,}});} }
         }
         { svelteHTML.createElement("label", { "class":`source-label`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("select", {     "aria-label":`Comparison source`,"value":`${source.connId}:${source.tab.id}`,"onchange":(event) => selectSource(event.currentTarget.value),});
               for(let entry of __sveltets_2_ensureArray(tabs)){`${entry.connId}:${entry.tab.id}`;
               { svelteHTML.createElement("option", { "value":`${entry.connId}:${entry.tab.id}`,});sourceLabel(entry.connId, entry.tab.name); }
            }
           }
         }
         { svelteHTML.createElement("div", { "class":`reference-query`,});
           { svelteHTML.createElement("span", { "class":`query-label`,});  }
           { svelteHTML.createElement("code", {   ...__sveltets_2_empty({"data-testid":`comparison-statement`}),"title":submitted ?? '',});submitted ?? 'This tab has not run a query yet.'; }
         }
         { svelteHTML.createElement("div", { "class":`reference-results`,});
          `${scope}:${source.connId}:${source.tab.id}`; {
            if(!source.tab.result && !source.tab.error && !source.tab.running){
               { const $$_etatSytpmE4C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE4C({ target: __sveltets_2_any(), props: {      "icon":`grid`,"title":`No results in this tab`,"body":`Focus the source to run a query. Selecting a comparison never runs it.`,}});}
            }else{
               { const $$_dirGstluseR4C = __sveltets_2_ensureComponent(ResultsGrid); new $$_dirGstluseR4C({ target: __sveltets_2_any(), props: {      "result":source.tab.result,"error":source.tab.error,"running":source.tab.running,}});}
            }
          }
         }
       }
    }
   }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ConnectionComparison__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ConnectionComparison__SvelteComponent_ = ReturnType<typeof ConnectionComparison__SvelteComponent_>;
/*Ωignore_endΩ*/export default ConnectionComparison__SvelteComponent_;