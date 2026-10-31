///<reference types="svelte" />
;// Model breakdown: tokens per (provider, model) over the window, plus a
// per-day stacked chart by model. Tokens lead; cost is a secondary column.
// Data comes straight from UsageSummary.models / daily_models.

import Icon from '../../lib/components/Icon.svelte';
import { formatCount, formatUsd } from '../../lib/metric-format';
import type { ModelUsage, DailyModelUsage } from '../../lib/api/usage.svelte';
;

  type Day = { day: string; total: number; cost: number; parts: Map<string, number> };;
;type $$ComponentProps =  {
    models: ModelUsage[];
    dailyModels: DailyModelUsage[];
    days: number;
    onexport?: () => void;
  };function $$render() {

  
  
  
  
  
  

  let {
    models,
    dailyModels,
    days,
    onexport,
  }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let provider = $state<string>('all');
  const providers = $derived([...new Set(models.map((m) => m.provider))].sort());
  // A filter left over from another window falls back to "all".
  const activeProvider = $derived(provider === 'all' || providers.includes(provider) ? provider : 'all');

  const shown = $derived(activeProvider === 'all' ? models : models.filter((m) => m.provider === activeProvider));
  const shownDaily = $derived(
    activeProvider === 'all' ? dailyModels : dailyModels.filter((d) => d.provider === activeProvider),
  );

  // Chart series: the 5 biggest models get their own colour, the rest fold
  // into "Other" (more than 6 categorical colours stop being distinguishable).
  const PALETTE = ['var(--cat-1)', 'var(--cat-2)', 'var(--cat-3)', 'var(--cat-4)', 'var(--cat-5)', 'var(--cat-6)'];
  const label = (m: { provider: string; model: string }) => `${m.provider} · ${m.model || 'unknown'}`;
  const series = $derived.by(() => {
    const top = [...shown].sort((a, b) => b.total_tokens - a.total_tokens).slice(0, 5).map(label);
    const out = top.map((name, i) => ({ name, color: PALETTE[i] }));
    if (shown.length > top.length) out.push({ name: 'Other', color: PALETTE[5] });
    return out;
  });

  const byDay = $derived.by(() => {
    const names = new Set(series.map((s) => s.name));
    const map = new Map<string, Day>();
    for (const d of shownDaily) {
      let e = map.get(d.day);
      if (!e) {
        e = { day: d.day, total: 0, cost: 0, parts: new Map() };
        map.set(d.day, e);
      }
      const key = names.has(label(d)) ? label(d) : 'Other';
      e.parts.set(key, (e.parts.get(key) ?? 0) + d.total_tokens);
      e.total += d.total_tokens;
      e.cost += d.cost_usd;
    }
    return [...map.values()].sort((a, b) => a.day.localeCompare(b.day));
  });
  const maxDay = $derived(Math.max(1, ...byDay.map((d) => d.total)));

  let chartWidth = $state(500);
  const W = $derived(chartWidth || 500);
  let selectedDayKey = $state('');
  const selectedDay = $derived(byDay.find(d => d.day === selectedDayKey) ?? byDay.at(-1));
  const H = 120;
  const AXIS_L = 52;
  const AXIS_B = 22;
  const yTicks = $derived.by(() => {
    const raw = Math.max(maxDay / 3, 1);
    const mag = Math.pow(10, Math.floor(Math.log10(raw)));
    const nice = Math.ceil(raw / mag) * mag;
    return [0, nice, nice * 2, nice * 3];
  });
  const yMax = $derived(yTicks[yTicks.length - 1] || 1);
  const plotH = H - AXIS_B;
  function x(i: number, n: number): number {
    return AXIS_L + ((i + 0.5) / Math.max(1, n)) * (W - AXIS_L);
  }
  function showX(i: number, n: number): boolean {
    const step = Math.max(1, Math.ceil(52 * n / Math.max(1, W - AXIS_L)));
    return i % step === 0;
  }
  const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
  function shortDay(iso: string): string {
    const m = iso.match(/^\d{4}-(\d{2})-(\d{2})/);
    return m ? `${MONTHS[parseInt(m[1], 10) - 1] ?? m[1]} ${parseInt(m[2], 10)}` : iso;
  }
  function dayTitle(d: Day): string {
    const parts = series
      .map((s) => [s.name, d.parts.get(s.name) ?? 0] as const)
      .filter(([, v]) => v > 0)
      .map(([n, v]) => `${n} ${v.toLocaleString()}`);
    return `${shortDay(d.day)} · ${d.total.toLocaleString()} tokens (≈ ${formatUsd(d.cost)})\n${parts.join('\n')}`;
  }
;
async () => {

 { svelteHTML.createElement("section", {     "class":`panel card`,"aria-labelledby":`usage-model-title`,...__sveltets_2_empty({"data-testid":`usage-models`}),});
   { svelteHTML.createElement("div", { "class":`panel-head`,});
     { svelteHTML.createElement("h3", { "id":`usage-model-title`,});  }
     { svelteHTML.createElement("div", { "class":`head-tools`,});
      if(providers.length > 1){
         { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Provider`,});
           { svelteHTML.createElement("button", {    "aria-pressed":activeProvider === 'all',"onclick":() => (provider = 'all'),});activeProvider === 'all';  }
             for(let p of __sveltets_2_ensureArray(providers)){p;
             { svelteHTML.createElement("button", {    "aria-pressed":activeProvider === p,"onclick":() => (provider = p),});activeProvider === p;p; }
          }
         }
      }
      if(models.length > 0 && onexport){
         { svelteHTML.createElement("button", {       "class":`btn small ghost`,"onclick":onexport,"title":`Download models as CSV`,"aria-label":`Download models as CSV`,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`download`,"size":12,}});} 
         }
      }
     }
   }

  if(shown.length === 0){
     { svelteHTML.createElement("p", { "class":`dim small`,});                }
  }else{
    if(byDay.length > 0){
       { svelteHTML.createElement("div", { "class":`legend`,});
           for(let s of __sveltets_2_ensureArray(series)){s.name;
           { svelteHTML.createElement("span", { "class":`lg`,}); { svelteHTML.createElement("i", { "style":`background: ${s.color}`,}); }s.name; }
        }
       }
       { const $$_div1 = svelteHTML.createElement("div", {  "class":`model-chart`,});chartWidth= $$_div1.clientWidth;
       { svelteHTML.createElement("svg", {       "class":`model-svg`,"viewBox":`0 0 ${W} ${H}`,"role":`group`,"aria-label":`Daily tokens by model over the last ${days} days, peak ${formatCount(maxDay)} tokens`,});
           for(let t of __sveltets_2_ensureArray(yTicks)){t;
          const y = plotH - (t / yMax) * plotH;
           { svelteHTML.createElement("line", {          "class":`grid-line`,"x1":AXIS_L,"y1":y,"x2":W,"y2":y,});}
           { svelteHTML.createElement("text", {       "class":`axis-label`,"x":AXIS_L - 4,"y":y + 4,"text-anchor":`end`,});formatCount(t); }
        }
            for(let d of __sveltets_2_ensureArray(byDay)){let i = 1;d.day;
          const cx = x(i, byDay.length);
          const bw = Math.max(2, Math.min(40, ((W - AXIS_L) / Math.max(1, byDay.length)) * 0.7));
          const barH = (d.total / yMax) * plotH;
          const stack = series.map((s) => ({ s, h: ((d.parts.get(s.name) ?? 0) / Math.max(1, d.total)) * barH }));
              for(let seg of __sveltets_2_ensureArray(stack)){let si = 1;seg.s.name;
            if(seg.h > 0){
               { svelteHTML.createElement("rect", {             "x":cx - bw / 2,"y":plotH - barH + stack.slice(0, si).reduce((a, b) => a + b.h, 0),"width":bw,"height":seg.h,"fill":seg.s.color,"rx":`1`,});}
            }
          }
           { svelteHTML.createElement("rect", {                        "class":`bar-hit`,"x":cx - bw / 2,"y":`0`,"width":bw,"height":plotH,"role":`button`,"tabindex":selectedDay?.day === d.day ? 0 : -1,"aria-label":dayTitle(d),"aria-pressed":selectedDay?.day === d.day,"onclick":() => (selectedDayKey = d.day),"onfocus":() => (selectedDayKey = d.day),"onkeydown":(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); selectedDayKey = d.day; } },}); { svelteHTML.createElement("title", {});dayTitle(d); } }
          if(showX(i, byDay.length)){
             { svelteHTML.createElement("text", {       "class":`axis-label`,"x":cx,"y":H - 4,"text-anchor":`middle`,});shortDay(d.day); }
          }
        }
         { svelteHTML.createElement("line", {          "class":`axis-line`,"x1":AXIS_L,"y1":plotH,"x2":W,"y2":plotH,});}
       }
       }
      if(selectedDay){
         { svelteHTML.createElement("div", {   "class":`model-chart-details`,...__sveltets_2_empty({"data-testid":`usage-model-details`}),});
           { svelteHTML.createElement("label", {});  { svelteHTML.createElement("select", {       "class":`input`,"aria-label":`Model tokens day`,"value":selectedDay.day,"onchange":(event) => (selectedDayKey = event.currentTarget.value),});
               for(let d of __sveltets_2_ensureArray(byDay)){d.day; { svelteHTML.createElement("option", { "value":d.day,});d.day; }}
           } }
           { svelteHTML.createElement("p", { "role":`status`,});selectedDay.day;  dayTitle(selectedDay); }
         }
      }
    }

     { svelteHTML.createElement("div", { "class":`tbl-scroll`,});
       { svelteHTML.createElement("table", { "class":`model-table`,});
         { svelteHTML.createElement("thead", {});
           { svelteHTML.createElement("tr", {});
             { svelteHTML.createElement("th", { "scope":`col`,});  }
             { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  }
             { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  }
             { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  }
             { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  }
             { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  }
             { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  }
           }
         }
         { svelteHTML.createElement("tbody", {});
             for(let m of __sveltets_2_ensureArray(shown)){m.provider + '\u0000' + m.model;
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("td", {});
                 { svelteHTML.createElement("span", {   "class":`m-name`,"title":m.model,});m.model || 'unknown'; }
                 { svelteHTML.createElement("span", { "class":`m-prov dim`,});m.provider; }
               }
               { svelteHTML.createElement("td", {   "class":`num`,"title":m.input_tokens.toLocaleString(),});formatCount(m.input_tokens); }
               { svelteHTML.createElement("td", {   "class":`num`,"title":m.output_tokens.toLocaleString(),});formatCount(m.output_tokens); }
               { svelteHTML.createElement("td", {   "class":`num`,"title":m.cache_write_tokens.toLocaleString(),});formatCount(m.cache_write_tokens); }
               { svelteHTML.createElement("td", {   "class":`num`,"title":m.cache_read_tokens.toLocaleString(),});formatCount(m.cache_read_tokens); }
               { svelteHTML.createElement("td", {   "class":`num strong`,"title":m.total_tokens.toLocaleString(),});formatCount(m.total_tokens); }
               { svelteHTML.createElement("td", { "class":`num dim`,});formatUsd(m.cost_usd); }
             }
          }
         }
       }
     }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ModelBreakdown__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ModelBreakdown__SvelteComponent_ = ReturnType<typeof ModelBreakdown__SvelteComponent_>;
/*Ωignore_endΩ*/export default ModelBreakdown__SvelteComponent_;