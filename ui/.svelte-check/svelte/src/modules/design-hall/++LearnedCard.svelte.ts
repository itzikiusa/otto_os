///<reference types="svelte" />
;
import { plural } from '../../lib/plural';
import LoadState from '../../lib/components/LoadState.svelte';
// Lobby rail: "What Otto learned from your team". The real pending rule
// proposals for this workspace (Learning v1, suggest-only) with Keep
// (approve), Dismiss (reject) and Details — each decision asks first — plus
// the active-rule count and how many patterns are still forming. When
// nothing is pending it says how rules appear; no invented rules.
import { untrack } from 'svelte';
import { loadErrorText } from '../../lib/loadError';
import Icon from '../../lib/components/Icon.svelte';
import { router } from '../../lib/router.svelte';
import { auth } from '../../lib/stores/auth.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { designBus } from '../../lib/events.svelte';
import type { DesignLearnedEdit, DesignLearnedResp } from '../../lib/api/types';
import { getLearned } from './assist/api';
import { editHeadline } from './assist/model';
import { approveRule, rejectRule } from './assist/ruleActions';
function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let learned = $state<DesignLearnedResp | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let busy = $state<string | null>(null);
  let seq = 0;

  async function load(): Promise<void> {
    const w = ws.currentId;
    if (!w) {
      loading = false;
      return;
    }
    const my = ++seq;
    try {
      const l = await getLearned(w);
      if (my !== seq) return;
      learned = l;
      error = null;
    } catch (e) {
      if (my === seq) error = loadErrorText(e);
    } finally {
      if (my === seq) loading = false;
    }
  }

  $effect(() => {
    void ws.currentId;
    void designBus.resyncTick;
    untrack(() => void load());
  });

  let seen = designBus.seq;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      const evs = designBus.since(seen);
      seen = now;
      if (evs.some((e) => e.type === 'design_learning_update' && e.workspace_id === ws.currentId)) void load();
    });
  });

  const pending = $derived(learned?.pending ?? []);
  const forming = $derived((learned?.candidates ?? []).filter((c) => !c.ready).length);
  const canEdit = $derived(auth.can('design', 'edit'));

  async function keep(e: DesignLearnedEdit): Promise<void> {
    busy = e.edit_id;
    if (await approveRule(e, learned?.skill)) await load();
    busy = null;
  }
  async function dismiss(e: DesignLearnedEdit): Promise<void> {
    busy = e.edit_id;
    if (await rejectRule(e)) await load();
    busy = null;
  }
  const details = (e: DesignLearnedEdit) => router.go(`design/learned/pending/${encodeURIComponent(e.edit_id)}`);
;
async () => {

 { svelteHTML.createElement("section", {     "class":`card rail-card`,"aria-labelledby":`dh-learned-h`,...__sveltets_2_empty({"data-testid":`design-learned-card`}),});
   { svelteHTML.createElement("header", {});
     { svelteHTML.createElement("span", { "class":`ico`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`bulb`,"size":14,}});} }
     { svelteHTML.createElement("h2", { "id":`dh-learned-h`,});      }
   }
  if(loading || error){
     { const $$_etatSdaoL1C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL1C({ target: __sveltets_2_any(), props: {        "what":`team rules`,"variant":`compact`,loading,error,"empty":true,"onretry":() => void load(),}});}
  } else if (learned){
     { svelteHTML.createElement("p", { "class":`counts`,});
      if(pending.length){ { svelteHTML.createElement("span", { "class":`chip new`,});pending.length;  }}
       { svelteHTML.createElement("span", { "class":`dim`,});plural(learned.active.length, 'active rule'); }
      if(learned.mode === 'off'){ { svelteHTML.createElement("span", { "class":`dim`,});   }}
     }
    if(pending.length === 0){
       { svelteHTML.createElement("p", { "class":`dim small`,});
                       forming ? ` — ${forming} pattern${forming === 1 ? ' is' : 's are'} forming` : '';
       }
    }else{
       { svelteHTML.createElement("ul", { "class":`props`,});
           for(let e of __sveltets_2_ensureArray((pending.slice(0, 3)))){e.edit_id;
           { svelteHTML.createElement("li", { ...__sveltets_2_empty({"data-testid":`design-learned-proposal`}),});
             { svelteHTML.createElement("p", { "class":`rule`,});editHeadline(e); }
            if(e.rationale){ { svelteHTML.createElement("p", { "class":`why`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});} e.rationale; }}
             { svelteHTML.createElement("div", { "class":`acts`,});
               { svelteHTML.createElement("button", {       "class":`btn small`,"disabled":!canEdit || busy === e.edit_id,"onclick":() => void keep(e),...__sveltets_2_empty({"data-testid":`design-learned-keep`}),});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});} 
               }
               { svelteHTML.createElement("button", {       "class":`btn small ghost`,"disabled":!canEdit || busy === e.edit_id,"onclick":() => void dismiss(e),...__sveltets_2_empty({"data-testid":`design-learned-dismiss`}),});  }
               { svelteHTML.createElement("span", { "class":`grow`,}); }
               { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => details(e),});  }
             }
           }
        }
       }
    }
  }
   { svelteHTML.createElement("a", {   "class":`more`,"href":pending.length ? '#/design/learned/pending' : '#/design/learned/rules',});
    pending.length ? 'Review all' : 'See team rules';  { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
   }
 }


};
return { props: {} as Record<string, never>, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LearnedCard__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LearnedCard__SvelteComponent_ = ReturnType<typeof LearnedCard__SvelteComponent_>;
/*Ωignore_endΩ*/export default LearnedCard__SvelteComponent_;