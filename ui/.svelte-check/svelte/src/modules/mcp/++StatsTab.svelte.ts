///<reference types="svelte" />
;// Per-tool aggregates derived from the audit ledger: call count, error count +
// rate, average / max latency, total bytes (the cost proxy — true USD is not
// metered), and the last-called time. Read-only.

import { formatBytes } from '../../lib/metric-format';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import { plural } from '../../lib/plural';
import { loadErrorText } from '../../lib/loadError';
import { mcpCpApi } from '../../lib/api/mcp';
import type { McpToolStats } from '../../lib/api/types';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  

  let stats = $state<McpToolStats[]>([]);
  let loading = $state(false);
  /** Failed load — inline with Retry, never the empty state. */
  let loadError = $state<string | null>(null);

  async function load(): Promise<void> {
    loading = true;
    try {
      stats = await mcpCpApi.cpStats();
      loadError = null;
    } catch (e) {
      loadError = loadErrorText(e);
    } finally {
      loading = false;
    }
  }

  $effect(() => {
    void load();
  });

  function errClass(rate: number): string {
    return rate >= 0.25 ? 'bad' : rate > 0 ? 'warn' : 'ok';
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`stats`,...__sveltets_2_empty({"data-testid":`mcp-stats`}),});
   { svelteHTML.createElement("div", { "class":`bar`,});
     { svelteHTML.createElement("span", { "class":`count`,});plural(stats.length, 'tool'); }
     { svelteHTML.createElement("span", { "class":`muted small`,});            }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":() => void load(),"disabled":loading,"title":`Refresh stats`,"aria-label":`Refresh stats`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":13,}});} }
   }

   { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL1 = new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        children:() => { return __sveltets_2_any(0); },"what":`tool stats`,loading,"error":loadError,"empty":stats.length === 0,"onretry":() => void load(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => {
       { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "icon":`chart`,"title":`No tool calls recorded yet`,"body":`Per-tool stats build up from the audit ledger as governed tools are called.`,}});}
    };return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL1.$$prop_def;/*Ωignore_endΩ*/
    
     { svelteHTML.createElement("div", { "class":`grid`,});
       { svelteHTML.createElement("div", { "class":`thead`,});
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", {});  }
         { svelteHTML.createElement("span", { "class":`num`,});  }
         { svelteHTML.createElement("span", { "class":`num`,});  }
         { svelteHTML.createElement("span", { "class":`num`,});  }
         { svelteHTML.createElement("span", { "class":`num`,});  }
         { svelteHTML.createElement("span", { "class":`num`,});  }
         { svelteHTML.createElement("span", { "class":`num`,});  }
         { svelteHTML.createElement("span", { "class":`num`,});  }
         { svelteHTML.createElement("span", {});  }
       }
         for(let s of __sveltets_2_ensureArray(stats)){`${s.server_id ?? ''}:${s.tool}`;
         { svelteHTML.createElement("div", { "class":`srow`,});
          
           { svelteHTML.createElement("span", {   "class":`cell mono tool`,"title":s.tool,}); { svelteHTML.createElement("span", { "class":`cl`,});  }s.tool; }
           { svelteHTML.createElement("span", {   "class":`cell`,"title":s.server_name ?? undefined,}); { svelteHTML.createElement("span", { "class":`cl`,});  }s.server_name ?? '—'; }
           { svelteHTML.createElement("span", { "class":`cell num`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }s.calls; }
           { svelteHTML.createElement("span", { "class":`cell num`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }s.errors; }
           { svelteHTML.createElement("span", { "class":`cell num ${errClass(s.error_rate)}`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }(s.error_rate * 100).toFixed(1);  }
           { svelteHTML.createElement("span", { "class":`cell num`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }Math.round(s.avg_latency_ms);  }
           { svelteHTML.createElement("span", { "class":`cell num`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }s.max_latency_ms;  }
           { svelteHTML.createElement("span", { "class":`cell num`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }formatBytes(Math.round(s.avg_bytes)); }
           { svelteHTML.createElement("span", { "class":`cell num`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }formatBytes(s.total_bytes); }
           { svelteHTML.createElement("span", { "class":`cell when`,}); { svelteHTML.createElement("span", { "class":`cl`,});  }s.last_called_at ? new Date(s.last_called_at).toLocaleString() : '—'; }
         }
      }
     }
   LoadState}
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const StatsTab__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type StatsTab__SvelteComponent_ = ReturnType<typeof StatsTab__SvelteComponent_>;
/*Ωignore_endΩ*/export default StatsTab__SvelteComponent_;