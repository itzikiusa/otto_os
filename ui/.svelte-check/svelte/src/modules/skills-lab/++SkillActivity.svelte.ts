///<reference types="svelte" />
;// Skills Lab → Skills → Evals / Usage for one skill, from data Otto already
// keeps (no new API):
//   Evals — Skills Evaluator runs whose source skill is this one: the latest
//           score, a score trend, and the run list. "New evaluation" goes to
//           the Evaluator tab (its form picks the skill).
//   Usage — where the skill is installed (each copy), what Otto feature
//           loads it (by category), and Otto's own activity on it in this
//           workspace (reviews, evaluations, golden tasks). Otto doesn't
//           record when an agent invokes a skill mid-session, and says so.

import type { GoldenTask, SkillEvalSummary, SkillReview } from '../../lib/api/types';
import { loadErrorText } from '../../lib/loadError';
import { skillsEvalApi } from '../../lib/api/skillsEval';
import { skillReviewApi } from '../../lib/api/skillReview';
import { rel } from '../../lib/stores/now.svelte';
import { runStatus } from '../../lib/status';
import Icon from '../../lib/components/Icon.svelte';
import Badge from '../../lib/components/Badge.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import EmptyState from '../../lib/components/EmptyState.svelte';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import Sparkline from '../../lib/components/Sparkline.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import { sourceLabel, type SkillGroup } from './skillGroups';
;

  interface Props {
    group: SkillGroup;
    view: 'evals' | 'usage';
    wsId: string;
    onevaluate: () => void;
    onreview: () => void;
    /** Open one evaluation run in the Evaluator. */
    onopenrun?: (id: string) => void;
    /** Open one existing review in the Review tab. */
    onopenreview?: (id: string) => void;
    /** Switch the detail pane to another tab (Usage → Evals). */
    onview?: (v: 'evals' | 'usage') => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { group, view, wsId, onevaluate, onreview, onopenrun, onopenreview, onview }: Props = $props();

  // This skill's runs only, as summary rows (was: every run of the workspace
  // with every iteration, filtered here).
  let evals = $state.raw<SkillEvalSummary[] | null>(null);
  let evalsError = $state<string | null>(null);
  let evalsFor = '';
  let evalsSeq = 0;
  async function loadEvals(ws: string, skill: string): Promise<void> {
    const my = ++evalsSeq;
    evals = null;
    evalsError = null;
    try {
      const page = await skillsEvalApi.listSummaries(ws, { skill, limit: 500 });
      if (my === evalsSeq) evals = page.items;
    } catch (e) {
      if (my !== evalsSeq) return;
      evals = [];
      evalsError = loadErrorText(e);
    }
  }
  $effect(() => {
    const key = `${wsId}\u0000${group.name}`;
    if (!wsId || evalsFor === key) return;
    evalsFor = key;
    void loadEvals(wsId, group.name);
  });
  let reviews = $state<SkillReview[] | null>(null);
  let golden = $state<GoldenTask[] | null>(null);
  let loadedFor = '';

  let wsError = $state<string | null>(null);
  async function load(ws: string): Promise<void> {
    wsError = null;
    reviews = golden = null;
    const [r, g] = await Promise.allSettled([
      skillReviewApi.list(ws),
      skillsEvalApi.listGolden(ws),
    ]);
    reviews = r.status === 'fulfilled' ? r.value : [];
    golden = g.status === 'fulfilled' ? g.value : [];
    // Any failed source is an error, not a zero: "Not evaluated yet" over a
    // failed load would be a lie.
    const failed = [r, g].find((x) => x.status === 'rejected') as PromiseRejectedResult | undefined;
    if (failed) wsError = loadErrorText(failed.reason);
  }
  const error = $derived(evalsError ?? wsError);
  $effect(() => {
    if (!wsId || loadedFor === wsId) return;
    loadedFor = wsId;
    void load(wsId);
  });

  const myEvals = $derived((evals ?? []).filter((x) => x.source_skill === group.name || x.dim_skill === group.name));
  const myReviews = $derived((reviews ?? []).filter((x) => x.skill_name === group.name));
  const myGolden = $derived((golden ?? []).filter((x) => x.skill === group.name));

  function scoreOf(e: SkillEvalSummary): number | null {
    const s = e.composite_score ?? e.best_score;
    return s == null || !Number.isFinite(s) ? null : s;
  }
  // Oldest → newest for the trend.
  const chrono = $derived([...myEvals].sort((a, b) => a.created_at.localeCompare(b.created_at)));
  const latest = $derived(chrono.at(-1) ?? null);
  const latestScore = $derived(latest ? scoreOf(latest) : null);
  const passCount = $derived(myEvals.filter((e) => e.status === 'done').length);
  const failCount = $derived(myEvals.filter((e) => e.status === 'error').length);

  // The shared run vocabulary (lib/status.ts). A `done` run finished — it
  // didn't necessarily *pass* (a 20/100 run is still `done`), so it must not
  // read "Passed".
  function fmtScore(v: number): string {
    return `${Math.round(v)}`;
  }

  const lastActivity = $derived.by(() => {
    const ts = [
      ...myEvals.map((e) => e.created_at),
      ...myReviews.map((r) => r.updated_at || r.created_at),
      ...myGolden.map((g) => g.updated_at || g.created_at),
    ].filter(Boolean);
    return ts.length ? ts.sort().at(-1)! : null;
  });

  /** What in Otto loads a skill of this category. */
  const loadedBy = $derived.by(() => {
    switch (group.category) {
      case 'review':
        return 'Offered as a lens in Code Review and PR review workflows.';
      case 'product':
        return 'Used by Product analysis (story refinement, plans).';
      case 'insights':
        return 'Runs the Insights reports.';
      case 'swarm':
        return 'Loaded by Agent Swarm role agents.';
      default:
        return 'Materialized into agent sessions from the library; agents load it by name when it fits the task.';
    }
  });

  function location(source: string): string {
    switch (source) {
      case 'library':
        return 'Otto library · <data dir>/library/skills/' + group.name;
      case 'bundled':
        return 'Compiled into Otto (read-only catalog)';
      case 'claude':
        return '~/.claude/skills/' + group.name;
      case 'codex':
        return '~/.codex/skills/' + group.name;
      case 'agy':
        return '~/.gemini/skills/' + group.name;
      default:
        return `~/.${source}/skills/${group.name}`;
    }
  }
;
async () => {

if(error){
   { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {     "what":`activity for this skill`,error,"empty":true,"onretry":() => { void load(wsId); void loadEvals(wsId, group.name); },}});}
} else if (!wsId){
   { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {      "title":`No workspace selected`,"body":`Evaluations and reviews belong to a workspace. Pick one in the sidebar to see this skill’s activity.`,"icon":`folder`,}});}
} else if (evals == null){
   { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {    "what":`activity for ${group.name}`,"loading":true,"empty":true,"rows":3,}});}
} else if (view === 'evals'){
  if(myEvals.length === 0){
     { const $$_etatSytpmE0C = __sveltets_2_ensureComponent(EmptyState); new $$_etatSytpmE0C({ target: __sveltets_2_any(), props: {             "icon":`target`,"title":`Not evaluated yet`,"body":`The Evaluator runs this skill on a real task, scores the result and suggests improvements — so you know a change helped before you ship it.`,"actionLabel":`New evaluation`,"actionIcon":`play`,"onaction":onevaluate,}});}
  }else{
     { svelteHTML.createElement("div", { "class":`stats`,});
       { svelteHTML.createElement("div", { "class":`card stat`,});
         { svelteHTML.createElement("div", { "class":`stat-label`,});  }
         { svelteHTML.createElement("div", { "class":`stat-value`,});latestScore != null ? fmtScore(latestScore) : '—'; { svelteHTML.createElement("span", { "class":`dim unit`,});latestScore != null ? ' / 100' : ''; } }
        if(chrono.filter((e) => scoreOf(e) != null).length >= 2){
           { const $$_enilkrapS2C = __sveltets_2_ensureComponent(Sparkline); new $$_enilkrapS2C({ target: __sveltets_2_any(), props: {            "values":chrono.map(scoreOf),"labels":chrono.map((e) => new Date(e.created_at).toLocaleDateString()),"name":`Evaluation score`,"format":fmtScore,"width":140,"height":26,}});}
        }
       }
       { svelteHTML.createElement("div", { "class":`card stat`,});
         { svelteHTML.createElement("div", { "class":`stat-label`,});  }
         { svelteHTML.createElement("div", { "class":`stat-value`,});myEvals.length; }
         { svelteHTML.createElement("div", { "class":`stat-sub`,}); { svelteHTML.createElement("span", { "class":`ok-text`,});passCount;  }if(failCount){  { svelteHTML.createElement("span", { "class":`bad-text`,});failCount;  }} }
       }
       { svelteHTML.createElement("div", { "class":`card stat`,});
         { svelteHTML.createElement("div", { "class":`stat-label`,});  }
         { svelteHTML.createElement("div", {   "class":`stat-value small`,"title":latest ? new Date(latest.created_at).toLocaleString() : '',});latest ? rel(latest.created_at) : '—'; }
        if(latest){ { svelteHTML.createElement("div", { "class":`stat-sub`,}); { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {  "status":runStatus(latest.status),}});} }}
       }
     }
     { svelteHTML.createElement("div", { "class":`list-head`,});
       { svelteHTML.createElement("h3", { "class":`section-title`,});  }
       { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":onevaluate,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}   }
     }
     { svelteHTML.createElement("ul", { "class":`rows`,});
         for(let e of __sveltets_2_ensureArray([...chrono].reverse())){e.id;
         { svelteHTML.createElement("li", {});
           { svelteHTML.createElement("button", {       "class":`rowi run-row`,"onclick":() => onopenrun?.(e.id),"title":`Open this run in the Evaluator`,"disabled":!onopenrun,});
             { const $$_egdaBsutatS3C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS3C({ target: __sveltets_2_any(), props: {    "status":runStatus(e.status),"variant":`text`,}});}
             { svelteHTML.createElement("span", {   "class":`grow ellipsis`,"title":e.task,});e.task || e.summary || 'Evaluation'; }
             { svelteHTML.createElement("span", { "class":`dim mono`,});e.impl_cli; }
             { svelteHTML.createElement("span", { "class":`score`,});scoreOf(e) != null ? fmtScore(scoreOf(e)!) : '—'; }
             { svelteHTML.createElement("span", {   "class":`dim when`,"title":new Date(e.created_at).toLocaleString(),});rel(e.created_at); }
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
           }
         }
      }
     }
  }
}else{
   { svelteHTML.createElement("section", { "class":`block`,});
     { svelteHTML.createElement("h3", { "class":`section-title`,});  }
     { svelteHTML.createElement("ul", { "class":`rows`,});
         for(let v of __sveltets_2_ensureArray(group.variants)){v.source;
         { svelteHTML.createElement("li", { "class":`rowi`,});
           { svelteHTML.createElement("span", { "class":`src-icon`,});
            if(v.source === 'library'){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`book`,"size":14,}});}} else if (v.source === 'bundled'){ { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":14,}});}}else{ { const $$_nocIredivorP4C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP4C({ target: __sveltets_2_any(), props: {    "provider":v.source,"size":14,}});}}
           }
           { svelteHTML.createElement("span", { "class":`src-name`,});sourceLabel(v.source); }
           { svelteHTML.createElement("span", {     "class":`grow dim mono ellipsis`,"dir":`ltr`,"title":location(v.source),});location(v.source); }
          if(group.driftedSources.includes(v.source)){ { const $$_egdaB3C = __sveltets_2_ensureComponent(Badge); new $$_egdaB3C({ target: __sveltets_2_any(), props: {    "tone":`warn`,"label":`Drifted`,}});}}
          if(v.source === 'bundled'){
             { svelteHTML.createElement("span", { "class":`chip`,}); v.bundledVersion;v.bundledState === 'not_installed' ? ' · not installed' : ''; }
          }
         }
      }
     }
   }
   { svelteHTML.createElement("section", { "class":`block`,});
     { svelteHTML.createElement("h3", { "class":`section-title`,});  }
     { svelteHTML.createElement("p", { "class":`para`,});loadedBy; }
   }
   { svelteHTML.createElement("section", { "class":`block`,});
     { svelteHTML.createElement("div", { "class":`list-head`,});
       { svelteHTML.createElement("h3", { "class":`section-title`,});     }
      if(lastActivity){ { svelteHTML.createElement("span", {   "class":`dim`,"title":new Date(lastActivity).toLocaleString(),}); rel(lastActivity); }}
     }
     { svelteHTML.createElement("div", { "class":`stats`,});
       { svelteHTML.createElement("button", {       "class":`card stat link`,"onclick":() => (myReviews[0] && onopenreview ? onopenreview(myReviews[0].id) : onreview()),"title":myReviews[0] ? 'Open the latest review' : 'Start a review of this skill',});
         { svelteHTML.createElement("div", { "class":`stat-label`,});  }
         { svelteHTML.createElement("div", { "class":`stat-value`,});myReviews.length; }
         { svelteHTML.createElement("div", { "class":`stat-sub dim`,});myReviews[0] ? `Latest: ${myReviews[0].static_report?.verdict ?? myReviews[0].status}` : 'Run a multi-agent review'; }
       }
       { svelteHTML.createElement("button", {       "class":`card stat link`,"onclick":() => (myEvals.length > 0 && onview ? onview('evals') : onevaluate()),"title":myEvals.length > 0 ? 'See its evaluation runs' : 'Evaluate this skill on a real task',});
         { svelteHTML.createElement("div", { "class":`stat-label`,});  }
         { svelteHTML.createElement("div", { "class":`stat-value`,});myEvals.length; }
         { svelteHTML.createElement("div", { "class":`stat-sub dim`,});latest ? `Last ${rel(latest.created_at)}` : 'Score it on a real task'; }
       }
       { svelteHTML.createElement("div", { "class":`card stat`,});
         { svelteHTML.createElement("div", { "class":`stat-label`,});  }
         { svelteHTML.createElement("div", { "class":`stat-value`,});myGolden.length; }
         { svelteHTML.createElement("div", { "class":`stat-sub dim`,});myGolden.length ? 'Regression cases pinned to it' : 'None pinned to this skill'; }
       }
     }
     { svelteHTML.createElement("p", { "class":`dim note`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}                                 }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const SkillActivity__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SkillActivity__SvelteComponent_ = ReturnType<typeof SkillActivity__SvelteComponent_>;
/*Ωignore_endΩ*/export default SkillActivity__SvelteComponent_;