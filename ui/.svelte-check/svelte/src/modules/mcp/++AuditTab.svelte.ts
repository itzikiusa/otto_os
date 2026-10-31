///<reference types="svelte" />
;
import { formatBytes } from '../../lib/metric-format';
import { untrack } from 'svelte';
import { plural } from '../../lib/plural';
// The governed-call audit ledger — every invoke (UI tester, gateway, inward
// read-only server, outward otto.* tools) writes one redacted row here. The
// table is filterable by server, tool, and decision; rows show the decision,
// ok/error, latency, bytes, and time.
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { sentenceCase } from '../../lib/labels';
import { loadErrorText } from '../../lib/loadError';
import { announce } from '../../lib/announce.svelte';
import { mcpCpApi } from '../../lib/api/mcp';
import { ws } from '../../lib/stores/workspace.svelte';
import type { McpCallLogRow, McpServerDetail } from '../../lib/api/types';
import McpPill from './McpPill.svelte';
import StatsTab from './StatsTab.svelte';
;

  interface Props {
    servers: McpServerDetail[];
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { servers }: Props = $props();

  let rows = $state<McpCallLogRow[]>([]);
  let loading = $state(false);
  /** Failed load — inline with Retry, never the empty state. */
  let loadError = $state<string | null>(null);
  let fServer = $state('');
  let fTool = $state('');
  let fDecision = $state('');
  let view = $state<'log' | 'stats'>('log');
  let filtersOpen = $state(false);

  /** Rows per request. The server pages the ledger BEFORE its per-row
   *  visibility check, so `offset` counts ledger rows, not shown ones. */
  const PAGE = 200;
  let nextOffset = 0;
  let hasMore = $state(false);
  let loadingMore = $state(false);
  let moreError = $state<string | null>(null);
  /** Bumped per (re)load so a stale "load more" never appends to new filters. */
  let generation = 0;

  /** The filters of the rows on screen: Load more pages THEM, never a tool
   *  name typed into the box but not yet applied. */
  let applied = { server: '', tool: '', decision: '' };
  /** Rows on screen come from a filtered query: an empty result then means
   *  "nothing matches", not "no calls yet" (the filter bar may be collapsed). */
  let filtered = $state(false);

  function clearFilters(): void {
    // The $effect below already reloads when the server/decision select
    // changes; load here only when the tool box was the sole filter.
    const effectReloads = !!(fServer || fDecision);
    fServer = '';
    fTool = '';
    fDecision = '';
    if (!effectReloads) void load();
  }

  function query(offset: number) {
    return {
      server_id: applied.server || undefined,
      tool: applied.tool || undefined,
      decision: applied.decision || undefined,
      limit: PAGE,
      offset,
    };
  }

  async function load(): Promise<void> {
    const gen = ++generation;
    applied = { server: fServer, tool: fTool.trim(), decision: fDecision };
    filtered = !!(applied.server || applied.tool || applied.decision);
    loading = true;
    moreError = null;
    // A page still loading for the old filters is dropped by the generation
    // guard, which also skips its `loadingMore = false` — clear it here or
    // Load more stays disabled until the tab remounts.
    loadingMore = false;
    try {
      const page = await mcpCpApi.cpAudit(query(0));
      if (gen !== generation) return;
      rows = page.rows;
      // The server reads PAGE ledger rows and THEN drops the ones this caller
      // can't see, so a short (even empty) page doesn't mean the end: it says
      // where the ledger continues and whether it has more (S17-304).
      nextOffset = page.next_offset;
      hasMore = page.has_more;
      loadError = null;
    } catch (e) {
      if (gen === generation) loadError = loadErrorText(e);
    } finally {
      if (gen === generation) loading = false;
    }
  }

  /** The next page, appended (rows already shown — e.g. shifted by new calls
   *  landing on top — are skipped by id). */
  async function loadMore(): Promise<void> {
    if (loadingMore) return;
    const gen = generation;
    loadingMore = true;
    try {
      const page = await mcpCpApi.cpAudit(query(nextOffset));
      if (gen !== generation) return;
      const seen = new Set(rows.map((r) => r.id));
      rows = [...rows, ...page.rows.filter((r) => !seen.has(r.id))];
      nextOffset = page.next_offset;
      hasMore = page.has_more;
      moreError = null;
    } catch (e) {
      if (gen === generation) {
        moreError = loadErrorText(e);
        // Spoken here: the inline line below is inserted with its text.
        announce(`Couldn’t load more rows: ${moreError}`, { assertive: true });
      }
    } finally {
      if (gen === generation) loadingMore = false;
    }
  }

  $effect(() => {
    // Server + decision selects apply immediately; the tool box applies on submit.
    void fServer;
    void fDecision;
    // untrack: load() reads the tool box, which applies on Enter / Apply —
    // tracked, every keystroke reloaded the ledger.
    untrack(() => void load());
  });

  /** Tool tooltip naming the agent session behind the call, when known. */
  function callTitle(r: McpCallLogRow): string {
    const sid = r.caller_session_id;
    if (!sid) return r.tool;
    const title = ws.getSession(sid)?.title;
    return `${r.tool} — from session ${title ?? `${sid.slice(0, 8)}…`}`;
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`audit`,...__sveltets_2_empty({"data-testid":`mcp-audit`}),});
   { svelteHTML.createElement("div", { "class":`head`,});
     { svelteHTML.createElement("h2", {});  }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    if(view === 'log'){
       { svelteHTML.createElement("span", { "class":`count`,});hasMore ? `${rows.length}+ rows` : plural(rows.length, 'row'); }
    }
     { svelteHTML.createElement("div", {     "class":`views`,"role":`group`,"aria-label":`Audit view`,});
       { svelteHTML.createElement("button", {      "aria-pressed":view === 'log',"onclick":() => (view = 'log'),});view === 'log';
        
       }
       { svelteHTML.createElement("button", {        ...__sveltets_2_empty({"data-testid":`mcp-audit-bytool`}),"aria-pressed":view === 'stats',"onclick":() => (view = 'stats'),});view === 'stats';
         
       }
     }
   }

  if(view === 'stats'){
     { const $$_baTstatS1C = __sveltets_2_ensureComponent(StatsTab); new $$_baTstatS1C({ target: __sveltets_2_any(), props: {}});}
  }else{
     { svelteHTML.createElement("div", { "class":`filters`,});
       { svelteHTML.createElement("button", {       "class":`filter-toggle`,"aria-expanded":filtersOpen,"onclick":() => (filtersOpen = !filtersOpen),});
        
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":filtersOpen ? 'chevronDown' : 'chevronRight',"size":12,}});}
       }
      if(filtersOpen){
         { svelteHTML.createElement("div", { "class":`bar`,});
           { svelteHTML.createElement("select", {   "aria-label":`Server`,"bind:value":fServer,});/*Ωignore_startΩ*/() => fServer = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});  }
               for(let s of __sveltets_2_ensureArray(servers)){s.id; { svelteHTML.createElement("option", { "value":s.id,});s.name; }}
           }
           { svelteHTML.createElement("input", {            "dir":`ltr`,"aria-label":`Filter by tool`,"bind:value":fTool,"placeholder":`Filter tool…`,"class":`mono`,"onkeydown":(e) => e.key === 'Enter' && void load(),});/*Ωignore_startΩ*/() => fTool = __sveltets_2_any(null);/*Ωignore_endΩ*/}
           { svelteHTML.createElement("select", {   "aria-label":`Decision`,"bind:value":fDecision,});/*Ωignore_startΩ*/() => fDecision = __sveltets_2_any(null);/*Ωignore_endΩ*/
             { svelteHTML.createElement("option", {"value":"",});  }
             { svelteHTML.createElement("option", { "value":`allowed`,});sentenceCase('allowed'); }
             { svelteHTML.createElement("option", { "value":`approved`,});sentenceCase('approved'); }
             { svelteHTML.createElement("option", { "value":`auto_approved`,});sentenceCase('auto_approved'); }
             { svelteHTML.createElement("option", { "value":`denied`,});sentenceCase('denied'); }
             { svelteHTML.createElement("option", { "value":`dry_run`,});sentenceCase('dry_run'); }
             { svelteHTML.createElement("option", { "value":`pending_approval`,});sentenceCase('pending_approval'); }
             { svelteHTML.createElement("option", { "value":`error`,});sentenceCase('error'); }
           }
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => void load(),});  }
         }
      }
     }
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        children:() => { return __sveltets_2_any(0); },"what":`the audit log`,loading,"error":loadError,"empty":rows.length === 0,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
        if(filtered){
           { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {            "icon":`note`,"title":`No calls match these filters`,"body":`Change the server, tool or decision filter, or clear them to see every call.`,"actionLabel":`Clear filters`,"actionKind":`secondary`,"onaction":clearFilters,}});}
        }else{
           { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`note`,"title":`No calls yet`,"body":`Calls made by external MCP clients and by sessions through the gateway appear here.`,}});}
        }
      };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("div", { "class":`grid`,});
         { svelteHTML.createElement("div", { "class":`thead`,});
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", {});  }
           { svelteHTML.createElement("span", { "class":`num`,});  }
           { svelteHTML.createElement("span", { "class":`num`,});  }
           { svelteHTML.createElement("span", { "class":`num`,});  }
         }
           for(let r of __sveltets_2_ensureArray(rows)){r.id;
          
           { svelteHTML.createElement("div", { "class":`arow`,});
             { svelteHTML.createElement("span", { "class":`cell when`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }new Date(r.created_at).toLocaleString(); }
             { svelteHTML.createElement("span", { "class":`cell`,}); { svelteHTML.createElement("span", { "class":`cl`,});  } { svelteHTML.createElement("span", {   "class":`trunc`,"title":r.server_name ?? undefined,});r.server_name ?? '—'; } }
             { svelteHTML.createElement("span", { "class":`cell mono`,}); { svelteHTML.createElement("span", { "class":`cl`,});  } { svelteHTML.createElement("span", {   "class":`trunc`,"title":callTitle(r),});r.tool; }if(r.dry_run){ { svelteHTML.createElement("span", { "class":`dry`,});  }} }
             { svelteHTML.createElement("span", {   "class":`cell`,"title":r.decision_reason ?? undefined,}); { svelteHTML.createElement("span", { "class":`cl`,});  }  { const $$_lliPpcM5C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM5C({ target: __sveltets_2_any(), props: {    "kind":`decision`,"value":r.decision,"small":true,}});} }
             { svelteHTML.createElement("span", { "class":`cell`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }  { const $$_lliPpcM5C = __sveltets_2_ensureComponent(McpPill); new $$_lliPpcM5C({ target: __sveltets_2_any(), props: {    "kind":`direction`,"value":r.direction,"small":true,}});} }
             { svelteHTML.createElement("span", { "class":`cell num`,});
               { svelteHTML.createElement("span", { "class":`cl`,});  }
              
              if(r.ok){ { svelteHTML.createElement("span", {   "class":`ok`,"title":`Succeeded`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":13,}});} { svelteHTML.createElement("span", { "class":`cl`,});  } }}else{ { svelteHTML.createElement("span", {   "class":`bad`,"title":r.error ?? 'Failed',}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":13,}});} { svelteHTML.createElement("span", { "class":`cl`,});  } }}
             }
             { svelteHTML.createElement("span", { "class":`cell num`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }r.latency_ms != null ? `${r.latency_ms}ms` : '—'; }
             { svelteHTML.createElement("span", { "class":`cell num`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }formatBytes(r.bytes); }
             { svelteHTML.createElement("details", { "class":`call-details`,});
               { svelteHTML.createElement("summary", {});  }
               { svelteHTML.createElement("dl", {});
                 { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});r.server_name ?? '—'; }
                 { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});r.tool; }
                if(r.decision_reason){ { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});r.decision_reason; }}
                if(!r.ok){ { svelteHTML.createElement("dt", {});  } { svelteHTML.createElement("dd", {});r.error ?? 'Failed'; }}
               }
             }
           }
        }
       }
      if(hasMore || moreError){
         { svelteHTML.createElement("div", { "class":`more`,});
          if(moreError){ { svelteHTML.createElement("span", { "class":`bad-text`,});moreError; }}
           { svelteHTML.createElement("button", {         "class":`btn small`,...__sveltets_2_empty({"data-testid":`mcp-audit-more`}),"disabled":loadingMore,"onclick":() => void loadMore(),});
            loadingMore ? 'Loading more rows…' : moreError ? 'Retry' : 'Load more';
           }
         }
      }
     LoadState}
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AuditTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AuditTab__SvelteComponent_ = ReturnType<typeof AuditTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default AuditTab__SvelteComponent_;