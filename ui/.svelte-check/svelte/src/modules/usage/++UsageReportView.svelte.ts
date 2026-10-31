///<reference types="svelte" />
;// Usage report: daily / monthly / per-model / per-session tables for the
// page's window, tokens first and cost as the last (secondary) column — in
// the style of ccusage. "Download HTML" saves the same tables as one
// self-contained file (reportHtml.ts). Root also gets the opt-in ccusage
// cross-check below the tables.

import { onMount } from 'svelte';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { toasts } from '../../lib/toast.svelte';
import { downloadText } from '../../lib/components/exporters';
import { usage } from '../../lib/api/usage.svelte';
import { buildReportHtml, reportFileName, tokenCells, TOKEN_HEADERS } from './reportHtml';
import CcusagePanel from './CcusagePanel.svelte';

;type $$ComponentProps =  { admin: boolean };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { admin }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  const SESSION_PAGE = 100;
  let sessionLimit = $state(SESSION_PAGE);

  onMount(() => {
    void usage.loadReport();
  });

  const report = $derived(usage.report);
  const sessions = $derived(report ? report.sessions.slice(0, sessionLimit) : []);

  async function download(): Promise<void> {
    // The page holds the slim report; the export needs every session and the
    // day×model table, fetched once on demand.
    const full = await usage.fullReport();
    if (!full) {
      toasts.error('Couldn’t load the full usage report', usage.reportError ?? undefined);
      return;
    }
    const file = reportFileName(full);
    downloadText(buildReportHtml(full), file, 'text/html');
    toasts.success('Saved the usage report', file);
  }

  async function showMore(): Promise<void> {
    if (!usage.reportFull) await usage.fullReport();
    sessionLimit += SESSION_PAGE * 4;
  }

  function scopeLabel(): string {
    if (!report) return '';
    if (report.scope === 'own') return 'Your sessions';
    return report.otto_only ? 'Otto sessions' : 'All sessions on this Mac';
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`report`,...__sveltets_2_empty({"data-testid":`usage-report`}),});
   { svelteHTML.createElement("div", { "class":`report-bar`,});
     { svelteHTML.createElement("span", { "class":`dim small`,});
      if(report){scopeLabel();   report.days;         report.priced_as_of;}
     }
     { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":!report || usage.reportLoading,"onclick":() => void download(),"title":report ? 'Save this report as one HTML file' : 'Load the report first',});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});}  
     }
   }

  if(usage.reportError && !report){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`the usage report`,"error":usage.reportError,"empty":true,"onretry":() => void usage.loadReport(),"loading":usage.reportLoading,}});}
  } else if (!report){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {    "what":`the usage report`,"loading":true,"empty":true,"rows":6,}});}
  } else if (report.totals.total_tokens === 0 && report.daily.length === 0){
     { const $$_etatSytpmE1C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE1C({ target: __sveltets_2_any(), props: {      "icon":`chart`,"title":`No usage in this window`,"body":`Pick a longer window, or switch to All to include sessions run outside Otto.`,}});}
  }else{
     const tokenTable/*Ωignore_positionΩ*/ = (caption: string, lead: string[], rows: { key: string; lead: string[]; r: Parameters<typeof tokenCells>[0] }[], total?: Parameters<typeof tokenCells>[0])/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
       { svelteHTML.createElement("section", { "class":`panel card`,});
         { svelteHTML.createElement("h3", {});caption; }
        if(rows.length === 0){
           { svelteHTML.createElement("p", { "class":`dim small`,});     }
        }else{
           { svelteHTML.createElement("div", { "class":`tbl-scroll`,});
             { svelteHTML.createElement("table", {});
               { svelteHTML.createElement("thead", {});
                 { svelteHTML.createElement("tr", {});
                     for(let h of __sveltets_2_ensureArray(lead)){h; { svelteHTML.createElement("th", { "scope":`col`,});h; }}
                     for(let h of __sveltets_2_ensureArray(TOKEN_HEADERS)){h; { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});h; }}
                 }
               }
               { svelteHTML.createElement("tbody", {});
                   for(let row of __sveltets_2_ensureArray(rows)){row.key;
                   { svelteHTML.createElement("tr", {});
                        for(let c of __sveltets_2_ensureArray(row.lead)){let i = 1;i; { svelteHTML.createElement("td", { });i === 0;c; }}
                        for(let c of __sveltets_2_ensureArray(tokenCells(row.r))){let i = 1;i; { svelteHTML.createElement("td", {   "class":`num`,});i === 5;i === 4;c; }}
                   }
                }
               }
              if(total){
                 { svelteHTML.createElement("tfoot", {});
                   { svelteHTML.createElement("tr", {});
                     { svelteHTML.createElement("th", {   "scope":`row`,"colspan":lead.length,});  }
                        for(let c of __sveltets_2_ensureArray(tokenCells(total))){let i = 1;i; { svelteHTML.createElement("td", {  "class":`num`,});i === 5;c; }}
                   }
                 }
              }
             }
           }
        }
       }
    };return __sveltets_2_any(0)};if(usage.reportError){
       { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`the usage report`,"error":usage.reportError,"onretry":() => void usage.loadReport(),"loading":usage.reportLoading,}});}
    }
    

    ;__sveltets_2_ensureSnippet(tokenTable('Daily', ['Date'], report.daily.map((d) => ({ key: d.day, lead: [d.day], r: d })), report.totals));
    ;__sveltets_2_ensureSnippet(tokenTable('Monthly', ['Month'], report.monthly.map((m) => ({ key: m.month, lead: [m.month], r: m })), report.totals));
    ;__sveltets_2_ensureSnippet(tokenTable(
      'By model',
      ['Model', 'Provider'],
      report.models.map((m) => ({ key: m.provider + '\u0000' + m.model, lead: [m.model || 'unknown', m.provider], r: m })),
      report.totals,
    ));
    ;__sveltets_2_ensureSnippet(tokenTable(
      'By session',
      ['Session', 'Workspace', 'Model', 'Last active'],
      sessions.map((s) => ({
        key: s.session_id,
        lead: [
          s.title ?? (s.kind != null ? s.session_id.slice(0, 12) : 'Session outside Otto'),
          s.workspace_name ?? '—',
          `${s.provider} · ${s.model || 'unknown'}`,
          s.last_active.slice(0, 16),
        ],
        r: s,
      })),
    ));
    if(report.sessions.length > sessionLimit){
       { svelteHTML.createElement("div", { "class":`more`,});
         { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":usage.reportLoading,"onclick":() => void showMore(),});
          if(usage.reportFull){   report.sessions.length - sessionLimit; }else{  }
         }
       }
    }
  }

  if(admin){
     { const $$_lenaPegasucC1C = __sveltets_2_ensureComponent(CcusagePanel); new $$_lenaPegasucC1C({ target: __sveltets_2_any(), props: {}});}
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const UsageReportView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type UsageReportView__SvelteComponent_ = ReturnType<typeof UsageReportView__SvelteComponent_>;
/*Ωignore_endΩ*/export default UsageReportView__SvelteComponent_;