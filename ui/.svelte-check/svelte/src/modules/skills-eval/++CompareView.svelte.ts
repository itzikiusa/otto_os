///<reference types="svelte" />
;// Side-by-side comparison of several runs (A/B two skills, or any N runs):
// best score, iterations, and per-validation scores from each run's best
// iteration. Pure client-side over already-loaded runs.

import type { SkillEval } from '../../lib/api/types';
import Icon from '../../lib/components/Icon.svelte';
;

  interface Props {
    runs: SkillEval[];
  };function $$render() {

  
  
  
  
  

  let { runs }: Props = $props();

  function bestIteration(r: SkillEval) {
    if (r.best_iteration != null) {
      const m = r.iterations.find((i) => i.iter === r.best_iteration);
      if (m) return m;
    }
    const done = r.iterations.filter((i) => i.status === 'done');
    return done.length > 0 ? done[done.length - 1] : r.iterations[r.iterations.length - 1] ?? null;
  }

  // validation name -> mean score, for a run's best iteration.
  function valScores(r: SkillEval): Record<string, number> {
    const it = bestIteration(r);
    const out: Record<string, number> = {};
    if (!it) return out;
    const groups: Record<string, number[]> = {};
    for (const a of it.agents) {
      if (a.status !== 'done') continue;
      (groups[a.validation] ??= []).push(a.score);
    }
    for (const [k, arr] of Object.entries(groups)) {
      out[k] = arr.reduce((s, x) => s + x, 0) / arr.length;
    }
    return out;
  }

  const cols = $derived(runs.map((r) => ({ run: r, scores: valScores(r) })));
  const dimensions = $derived.by(() => {
    const set = new Set<string>();
    for (const c of cols) for (const k of Object.keys(c.scores)) set.add(k);
    return [...set].sort();
  });

  function scoreClass(n: number | undefined): string {
    if (n == null) return '';
    if (n >= 85) return 'good';
    if (n >= 60) return 'ok';
    return 'bad';
  }
  function best(values: (number | undefined)[]): number | null {
    const nums = values.filter((v): v is number => v != null);
    return nums.length ? Math.max(...nums) : null;
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`cmp`,});
   { svelteHTML.createElement("h2", {}); runs.length;  }
   { svelteHTML.createElement("p", { "class":`lede`,});       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}        }
   { svelteHTML.createElement("div", { "class":`table-wrap`,});
     { svelteHTML.createElement("table", {});
       { svelteHTML.createElement("thead", {});
         { svelteHTML.createElement("tr", {});
           { svelteHTML.createElement("th", {   "class":`rowlabel`,"scope":`col`,}); { svelteHTML.createElement("span", { "class":`sr-only`,});  } }
             for(let c of __sveltets_2_ensureArray(cols)){c.run.id;
             { svelteHTML.createElement("th", {   "scope":`col`,"title":c.run.task,});
               { svelteHTML.createElement("div", { "class":`col-skill`,});c.run.source_skill; }
               { svelteHTML.createElement("div", { "class":`col-cli mono`,});c.run.impl_cli || 'score only'; }
              if(c.run.task){ { svelteHTML.createElement("div", { "class":`col-task`,});c.run.task; }}
             }
          }
         }
       }
       { svelteHTML.createElement("tbody", {});
         { svelteHTML.createElement("tr", { "class":`major`,});
           { svelteHTML.createElement("th", {   "class":`rowlabel`,"scope":`row`,});  }
             for(let c of __sveltets_2_ensureArray(cols)){c.run.id;
            const bs = c.run.best_score ?? undefined;
            const top = best(cols.map((x) => x.run.best_score ?? undefined));
             { svelteHTML.createElement("td", {});
              if(bs != null){
                 { svelteHTML.createElement("span", {    "class":`score ${scoreClass(bs)}`,"title":top != null && bs === top ? 'Top score' : undefined,});top != null && bs === top;if(top != null && bs === top){ { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}bs.toFixed(0); }
              }else{
                 { svelteHTML.createElement("span", { "class":`dash`,});  }
              }
             }
          }
         }
         { svelteHTML.createElement("tr", {});
           { svelteHTML.createElement("th", {   "class":`rowlabel`,"scope":`row`,});  }
             for(let c of __sveltets_2_ensureArray(cols)){c.run.id;
             { svelteHTML.createElement("td", {});c.run.iterations.length; }
          }
         }
           for(let dim of __sveltets_2_ensureArray(dimensions)){dim;
          const top = best(cols.map((c) => c.scores[dim]));
           { svelteHTML.createElement("tr", {});
             { svelteHTML.createElement("th", {   "class":`rowlabel`,"scope":`row`,});dim; }
               for(let c of __sveltets_2_ensureArray(cols)){c.run.id;
              const v = c.scores[dim];
               { svelteHTML.createElement("td", {});
                if(v != null){
                   { svelteHTML.createElement("span", {    "class":`score ${scoreClass(v)}`,"title":top != null && v === top ? 'Top score' : undefined,});top != null && v === top;if(top != null && v === top){ { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}}v.toFixed(0); }
                }else{
                   { svelteHTML.createElement("span", { "class":`dash`,});  }
                }
               }
            }
           }
        }
       }
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CompareView__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CompareView__SvelteComponent_ = ReturnType<typeof CompareView__SvelteComponent_>;
/*Ωignore_endΩ*/export default CompareView__SvelteComponent_;