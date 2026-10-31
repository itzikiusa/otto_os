///<reference types="svelte" />
;// Attribution drilldown panel (B1): "why did this cost so much?"
// Groups usage_events by a chosen work-graph dimension (repo/branch/PR/…) and
// shows cost + tokens per group. Supports CSV and JSON export via exporters.ts.

import { api } from '../../lib/api/client';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { toasts } from '../../lib/toast.svelte';
import { loadErrorText } from '../../lib/loadError';
import { exportCsv, downloadJson, copyAsJson } from '../../lib/components/exporters';
import type { AttributionRow, AttributionDim } from './types';
import { DIM_LABELS } from './types';
import { formatCount, formatUsd } from '../../lib/metric-format';

;type $$ComponentProps =  { days?: number };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  // Injected from the parent (UsagePage) — the current look-back window.
  let { days = 30 }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  const DIMS: AttributionDim[] = [
    'origin',
    'repo',
    'branch',
    'pr',
    'story',
    'swarm_task',
    'workflow',
    'channel',
    'review',
  ];

  let selectedDim: AttributionDim = $state('origin');
  let rows: AttributionRow[] = $state([]);
  let loading = $state(false);
  /** Last load failure — shown inline with Retry (an empty table would read
   *  as "nothing attributed", which is not what happened). */
  let error = $state('');
  let copiedRow: string | null = $state(null);

  // Request token: switching dimension/window while a query is in flight must
  // not let the older (often slower) response land under the new labels.
  let seq = 0;

  // The dimension/window the current `rows` belong to: a new grouping clears
  // them so the old dimension's keys never sit under the new column label.
  let rowsKey = '';

  async function load(): Promise<void> {
    const mine = ++seq;
    const key = `${selectedDim}:${days}`;
    if (key !== rowsKey) {
      rows = [];
      rowsKey = key;
    }
    loading = true;
    try {
      const next = await api.get<AttributionRow[]>(
        `/usage/attribution?by=${selectedDim}&days=${days}`,
      );
      if (mine === seq) {
        rows = next;
        error = '';
      }
    } catch (e) {
      if (mine !== seq) return;
      error = loadErrorText(e);
      rows = [];
    } finally {
      if (mine === seq) loading = false;
    }
  }

  // Reload whenever dim or window changes.
  $effect(() => {
    void load();
    // eslint-disable-next-line @typescript-eslint/no-unused-expressions
    selectedDim; days;
  });

  const totalCost = $derived(rows.reduce((s, r) => s + r.cost_usd, 0));
  const totalTokens = $derived(rows.reduce((s, r) => s + r.tokens, 0));

  function pct(cost: number): number {
    return totalCost > 0 ? (cost / totalCost) * 100 : 0;
  }

  function exportRowsCsv(): void {
    exportCsv(
      rows.map((r) => ({
        [DIM_LABELS[selectedDim]]: r.key,
        cost_usd: r.cost_usd,
        tokens: r.tokens,
        sessions: r.sessions,
      })),
      `otto-attribution-${selectedDim}-${days}d.csv`,
    );
    toasts.success('Exported attribution as CSV', `otto-attribution-${selectedDim}-${days}d.csv`);
  }

  function exportRowsJson(): void {
    downloadJson(rows, `otto-attribution-${selectedDim}-${days}d.json`);
    toasts.success('Exported attribution as JSON', `otto-attribution-${selectedDim}-${days}d.json`);
  }

  async function copyRow(r: AttributionRow): Promise<void> {
    // A blocked clipboard must not flash the "copied" tick — swallow it here
    // and leave the row unmarked so the failure is visible, not silent.
    try {
      await copyAsJson(r);
    } catch {
      toasts.error("Couldn’t copy the row", 'Clipboard access was blocked. Select the row’s text and copy it manually.');
      return;
    }
    copiedRow = r.key;
    setTimeout(() => {
      copiedRow = null;
    }, 1500);
  }
;
async () => {

 { svelteHTML.createElement("section", {   "class":`card attribution-panel`,"aria-labelledby":`attr-title`,});
   { svelteHTML.createElement("div", { "class":`attr-header`,});
     { svelteHTML.createElement("div", { "class":`attr-heading`,});
       { svelteHTML.createElement("h3", {   "class":`attr-title`,"id":`attr-title`,});  }
       { svelteHTML.createElement("span", { "class":`attr-subtitle`,});           }
     }
     { svelteHTML.createElement("div", { "class":`attr-controls`,});
       { svelteHTML.createElement("label", {   "class":`dim-label`,"for":`attr-dim-select`,});  }
       { svelteHTML.createElement("select", {     "id":`attr-dim-select`,"class":`input dim-select`,"bind:value":selectedDim,});/*Ωignore_startΩ*/() => selectedDim = __sveltets_2_any(null);/*Ωignore_endΩ*/
          for(let dim of __sveltets_2_ensureArray(DIMS)){
           { svelteHTML.createElement("option", { "value":dim,});DIM_LABELS[dim]; }
        }
       }
      if(rows.length > 0){
         { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":exportRowsCsv,"title":`Download as CSV`,"aria-label":`Download attribution as CSV`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
         }
         { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":exportRowsJson,"title":`Download as JSON`,"aria-label":`Download attribution as JSON`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
         }
      }
     }
   }

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {           children:() => { return __sveltets_2_any(0); },"what":`cost attribution`,loading,error,"empty":rows.length === 0,"rows":3,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {       "icon":`chart`,"title":`Nothing attributed by ${DIM_LABELS[selectedDim].toLowerCase()} in the last ${days} days`,"body":selectedDim === 'origin'
          ? 'Usage shows up here once agents run in this window.'
          : 'Reviews, product stories, swarms and workflows tag the sessions they start. Ad-hoc sessions only appear under Origin.',}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`attr-table-wrap`,});
       { svelteHTML.createElement("table", { "class":`attr-table`,});
         { svelteHTML.createElement("thead", {});
           { svelteHTML.createElement("tr", {});
             { svelteHTML.createElement("th", { "class":`col-key`,});DIM_LABELS[selectedDim]; }
             { svelteHTML.createElement("th", { "class":`col-bar`,}); { svelteHTML.createElement("span", { "class":`sr-only`,});   } }
             { svelteHTML.createElement("th", { "class":`col-cost`,});  }
             { svelteHTML.createElement("th", { "class":`col-pct`,});  }
             { svelteHTML.createElement("th", { "class":`col-tokens`,});  }
             { svelteHTML.createElement("th", { "class":`col-sessions`,});  }
             { svelteHTML.createElement("th", { "class":`col-copy`,}); { svelteHTML.createElement("span", { "class":`sr-only`,});  } }
           }
         }
         { svelteHTML.createElement("tbody", {});
             for(let r of __sveltets_2_ensureArray(rows)){r.key;
             { svelteHTML.createElement("tr", { "class":`attr-row`,});
               { svelteHTML.createElement("td", { "class":`col-key`,}); { svelteHTML.createElement("span", {   "class":`attr-key`,"title":r.key,});r.key; } }
               { svelteHTML.createElement("td", { "class":`col-bar`,});
                 { svelteHTML.createElement("div", { "class":`bar-track`,});
                   { svelteHTML.createElement("div", {   "class":`bar-fill`,"style":`width: ${pct(r.cost_usd).toFixed(1)}%`,}); }
                 }
               }
               { svelteHTML.createElement("td", { "class":`col-cost`,});formatUsd(r.cost_usd); }
               { svelteHTML.createElement("td", { "class":`col-pct dim-pct`,});pct(r.cost_usd).toFixed(1);  }
               { svelteHTML.createElement("td", {   "class":`col-tokens`,"title":`${r.tokens.toLocaleString()} tokens`,});formatCount(r.tokens); }
               { svelteHTML.createElement("td", { "class":`col-sessions`,});r.sessions; }
               { svelteHTML.createElement("td", { "class":`col-copy`,});
                 { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => copyRow(r),"title":copiedRow === r.key ? 'Copied' : 'Copy row as JSON',"aria-label":copiedRow === r.key ? 'Copied' : 'Copy row as JSON',});
                   { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":copiedRow === r.key ? 'check' : 'copy',"size":14,}});}
                 }
               }
             }
          }
         }
         { svelteHTML.createElement("tfoot", {});
           { svelteHTML.createElement("tr", { "class":`attr-total`,});
             { svelteHTML.createElement("td", { "class":`col-key`,});  }
             { svelteHTML.createElement("td", { "class":`col-bar`,}); }
             { svelteHTML.createElement("td", { "class":`col-cost`,});formatUsd(totalCost); }
             { svelteHTML.createElement("td", { "class":`col-pct`,});  }
             { svelteHTML.createElement("td", { "class":`col-tokens`,});formatCount(totalTokens); }
             { svelteHTML.createElement("td", { "class":`col-sessions`,}); }
             { svelteHTML.createElement("td", { "class":`col-copy`,}); }
           }
         }
       }
     }
   LoadState}
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AttributionDrilldown__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AttributionDrilldown__SvelteComponent_ = ReturnType<typeof AttributionDrilldown__SvelteComponent_>;
/*Ωignore_endΩ*/export default AttributionDrilldown__SvelteComponent_;