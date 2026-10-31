///<reference types="svelte" />
;// Opt-in cross-check against ccusage (root only). Nothing runs until the
// user clicks: the daemon then runs `npx ccusage` locally with a timeout and
// returns its numbers next to Otto's for the same dates, every session
// (external included) on both sides.

import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { formatCount, formatUsd } from '../../lib/metric-format';
import { usage, type TokenTotals } from '../../lib/api/usage.svelte';
function $$render() {

  
  
  
  
  
  
  
  

  const RANGES = [7, 14, 30];
  let days = $state(7);

  const check = $derived(usage.ccusage);

  function diff(ours: number, theirs: number): string {
    const d = ours - theirs;
    if (d === 0) return '0';
    return (d > 0 ? '+' : '−') + formatCount(Math.abs(d));
  }
  function pct(ours: number, theirs: number): string {
    if (theirs === 0) return '';
    const p = ((ours - theirs) / theirs) * 100;
    if (Math.abs(p) < 0.5) return 'match';
    return `${p > 0 ? '+' : '−'}${Math.abs(p).toFixed(0)}%`;
  }
  const BUCKETS: { key: keyof TokenTotals; label: string }[] = [
    { key: 'input_tokens', label: 'Input' },
    { key: 'output_tokens', label: 'Output' },
    { key: 'cache_write_tokens', label: 'Cache write' },
    { key: 'cache_read_tokens', label: 'Cache read' },
    { key: 'total_tokens', label: 'Total' },
  ];
;
async () => {

 { svelteHTML.createElement("section", {     "class":`panel card`,"aria-labelledby":`usage-ccusage-title`,...__sveltets_2_empty({"data-testid":`usage-ccusage`}),});
   { svelteHTML.createElement("div", { "class":`panel-head`,});
     { svelteHTML.createElement("h3", { "id":`usage-ccusage-title`,});   }
     { svelteHTML.createElement("div", { "class":`head-tools`,});
       { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Days to compare`,});
           for(let r of __sveltets_2_ensureArray(RANGES)){r;
           { svelteHTML.createElement("button", {      "aria-pressed":days === r,"disabled":usage.ccusageRunning,"onclick":() => (days = r),});days === r;r;  }
        }
       }
       { svelteHTML.createElement("button", {     "class":`btn small`,"disabled":usage.ccusageRunning,"onclick":() => usage.runCcusage(days),});
         { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
        usage.ccusageRunning ? 'Running ccusage…' : check ? 'Run again' : 'Compare with ccusage';
       }
     }
   }
   { svelteHTML.createElement("p", { "class":`dim small intro`,});
      { svelteHTML.createElement("span", { "class":`mono`,});  }               
                       
                     
       
   }

  if(usage.ccusageRunning){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {    "what":`the ccusage comparison`,"loading":true,"empty":true,"rows":3,}});}
  } else if (usage.ccusageError){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {      "what":`the ccusage comparison`,"error":usage.ccusageError,"empty":true,"onretry":() => usage.runCcusage(days),}});}
  } else if (check && !check.ran){
     { svelteHTML.createElement("div", {   "class":`cc-error`,"role":`alert`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
       { svelteHTML.createElement("div", {});
         { svelteHTML.createElement("div", {});   check.error ?? 'no output'; }
        if(check.command){ { svelteHTML.createElement("div", { "class":`dim small`,});  { svelteHTML.createElement("span", { "class":`mono`,});check.command; } }}
       }
     }
  } else if (check){
     { svelteHTML.createElement("p", { "class":`dim small`,});
      check.since;  check.until;    (check.duration_ms / 1000).toFixed(1);  
       { svelteHTML.createElement("span", {   "class":`mono`,"title":check.command,});check.command; }
     }
     { svelteHTML.createElement("div", { "class":`tbl-scroll`,});
       { svelteHTML.createElement("table", { "class":`cc-table`,});
         { svelteHTML.createElement("caption", { "class":`sr-only`,});    }
         { svelteHTML.createElement("thead", {});
           { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  } { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  } { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  } }
         }
         { svelteHTML.createElement("tbody", {});
             for(let b of __sveltets_2_ensureArray(BUCKETS)){b.key;
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("th", { "scope":`row`,});b.label; }
               { svelteHTML.createElement("td", { "class":`num`,});formatCount(check.totals_ours[b.key]); }
               { svelteHTML.createElement("td", { "class":`num`,});formatCount(check.totals_theirs[b.key]); }
               { svelteHTML.createElement("td", { "class":`num`,});diff(check.totals_ours[b.key], check.totals_theirs[b.key]);  { svelteHTML.createElement("span", { "class":`dim`,});pct(check.totals_ours[b.key], check.totals_theirs[b.key]); } }
             }
          }
           { svelteHTML.createElement("tr", { "class":`cost-row`,});
             { svelteHTML.createElement("th", { "scope":`row`,});  }
             { svelteHTML.createElement("td", { "class":`num dim`,});formatUsd(check.totals_ours.cost_usd); }
             { svelteHTML.createElement("td", { "class":`num dim`,});formatUsd(check.totals_theirs.cost_usd); }
             { svelteHTML.createElement("td", { "class":`num dim`,});pct(check.totals_ours.cost_usd, check.totals_theirs.cost_usd); }
           }
         }
       }
     }

     { svelteHTML.createElement("h4", {});    }
    if(check.rows.length === 0){
       { svelteHTML.createElement("p", { "class":`dim small`,});       }
    }else{
       { svelteHTML.createElement("div", { "class":`tbl-scroll`,});
         { svelteHTML.createElement("table", { "class":`cc-table`,});
           { svelteHTML.createElement("thead", {});
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("th", { "scope":`col`,});   }
               { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  }
               { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  }
               { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  }
               { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});    }
             }
           }
           { svelteHTML.createElement("tbody", {});
               for(let r of __sveltets_2_ensureArray(check.rows)){r.provider + '\u0000' + r.model;
               { svelteHTML.createElement("tr", {});
                 { svelteHTML.createElement("td", {}); { svelteHTML.createElement("span", { "class":`m-name`,});r.model || 'unknown'; }  { svelteHTML.createElement("span", { "class":`dim`,});r.provider; } }
                 { svelteHTML.createElement("td", { "class":`num`,});formatCount(r.ours.total_tokens); }
                 { svelteHTML.createElement("td", { "class":`num`,});formatCount(r.theirs.total_tokens); }
                 { svelteHTML.createElement("td", { "class":`num`,});diff(r.ours.total_tokens, r.theirs.total_tokens);  { svelteHTML.createElement("span", { "class":`dim`,});pct(r.ours.total_tokens, r.theirs.total_tokens); } }
                 { svelteHTML.createElement("td", { "class":`num dim`,});formatCount(r.ours.output_tokens);  formatCount(r.theirs.output_tokens); }
               }
            }
           }
         }
       }
    }

    if(check.daily.length > 0){
       { svelteHTML.createElement("h4", {});  }
       { svelteHTML.createElement("div", { "class":`tbl-scroll`,});
         { svelteHTML.createElement("table", { "class":`cc-table`,});
           { svelteHTML.createElement("thead", {});
             { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  } { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  } { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  } }
           }
           { svelteHTML.createElement("tbody", {});
               for(let d of __sveltets_2_ensureArray(check.daily)){d.day;
               { svelteHTML.createElement("tr", {});
                 { svelteHTML.createElement("td", {});d.day; }
                 { svelteHTML.createElement("td", { "class":`num`,});formatCount(d.ours.total_tokens); }
                 { svelteHTML.createElement("td", { "class":`num`,});formatCount(d.theirs.total_tokens); }
                 { svelteHTML.createElement("td", { "class":`num`,});diff(d.ours.total_tokens, d.theirs.total_tokens);  { svelteHTML.createElement("span", { "class":`dim`,});pct(d.ours.total_tokens, d.theirs.total_tokens); } }
               }
            }
           }
         }
       }
    }
  }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CcusagePanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CcusagePanel__SvelteComponent_ = ReturnType<typeof CcusagePanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default CcusagePanel__SvelteComponent_;