///<reference types="svelte" />
;// The right rail on wide windows: what needs you, what is running, and how
// this week's load splits across your subscriptions (with any limit hit).
// Each block owns its own loading / empty / error state so one failing
// source never blanks the rest.

import StatusDot from '../../lib/components/StatusDot.svelte';
import LiveWorkingDot from '../../lib/components/LiveWorkingDot.svelte';
import ProviderIcon from '../../lib/components/ProviderIcon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { assistant } from '../../lib/stores/assistant.svelte';
import { formatCount } from '../../lib/metric-format';
import { TASK_KIND, loadShare, providerName } from '../../lib/assistant/model';
import { whenLabel } from './format';
import NeedsYouLink from '../home/LazyNeedsYouLink.svelte';
;

  interface Props {
    onopenthread: (id: string) => void;
    onopentasks: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  

  let { onopenthread, onopentasks }: Props = $props();

  $effect(() => {
    if (assistant.limits.state === 'idle') void assistant.loadLimits();
    if (assistant.tasks.state === 'idle') void assistant.loadTasks();
    if (assistant.usageWeek.state === 'idle') void assistant.loadUsageWeek();
  });

  const needs = $derived(assistant.needs.items);
  const running = $derived(assistant.tasks.data.filter((t) => t.state === 'running'));
  const share = $derived(loadShare(assistant.usageWeek.data.filter((p) => p.provider === 'claude' || p.provider === 'codex')));
  const limited = $derived(assistant.limits.data.filter((l) => l.limited));
  const threadTitle = (id: string | null): string => (id ? (assistant.thread(id)?.title ?? 'A thread') : 'Not in a thread');
;
async () => {

 { svelteHTML.createElement("aside", {     "class":`rail`,"aria-label":`Needs you, running and subscriptions`,...__sveltets_2_empty({"data-testid":`assistant-rail`}),});
   { svelteHTML.createElement("section", {});
     { svelteHTML.createElement("div", { "class":`rail-head`,});
       { svelteHTML.createElement("h2", { "class":`section-title`,});  }
      
       { const $$_kniLuoYsdeeN3C = __sveltets_2_ensureComponent(NeedsYouLink); new $$_kniLuoYsdeeN3C({ target: __sveltets_2_any(), props: {}});}
     }
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`what needs you`,"variant":`compact`,"loading":assistant.needsState === 'loading',"error":assistant.needsState === 'error' ? 'The daemon didn’t answer.' : '',"empty":!needs.length,"onretry":() => void assistant.loadNeedsYou(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("p", { "class":`dim`,});     }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("ul", {});
           for(let n of __sveltets_2_ensureArray(needs)){n.id;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("button", {     "class":`item`,"title":n.title,"onclick":() => (n.thread_id ? onopenthread(n.thread_id) : onopentasks()),});
               { const $$_toDsutatS6C = __sveltets_2_ensureComponent(StatusDot); new $$_toDsutatS6C({ target: __sveltets_2_any(), props: {    "status":`idle`,"needsYou":true,"size":8,}});}
               { svelteHTML.createElement("span", { "class":`t`,}); { svelteHTML.createElement("span", { "class":`name`,});n.title; } { svelteHTML.createElement("span", { "class":`sub`,});threadTitle(n.thread_id); } }
             }
           }
        }
       }
     LoadState}
   }

   { svelteHTML.createElement("section", {});
     { svelteHTML.createElement("h2", { "class":`section-title`,});  }
     { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); const $$_etatSdaoL2 = new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {             children:() => { return __sveltets_2_any(0); },"what":`running tasks`,"variant":`compact`,"loading":assistant.tasks.state === 'loading',"error":assistant.tasks.state === 'error' ? assistant.tasks.error || 'The daemon didn’t answer.' : '',"empty":!running.length,"onretry":() => void assistant.loadTasks(),emptyView:() => { async ()/*Ωignore_positionΩ*/ => { { svelteHTML.createElement("p", { "class":`dim`,});  }};return __sveltets_2_any(0)},}});/*Ωignore_startΩ*/const {emptyView} = $$_etatSdaoL2.$$prop_def;/*Ωignore_endΩ*/
      
       { svelteHTML.createElement("ul", {});
           for(let t of __sveltets_2_ensureArray(running)){t.id;
           { svelteHTML.createElement("li", {});
             { svelteHTML.createElement("button", {     "class":`item`,"title":t.title,"onclick":onopentasks,});
               { const $$_toDgnikroWeviL6C = __sveltets_2_ensureComponent(LiveWorkingDot); new $$_toDgnikroWeviL6C({ target: __sveltets_2_any(), props: {  "size":8,}});}
               { svelteHTML.createElement("span", { "class":`t`,}); { svelteHTML.createElement("span", { "class":`name`,});t.title; } { svelteHTML.createElement("span", { "class":`sub`,});TASK_KIND[t.kind] ?? t.kind;  threadTitle(t.thread_id); } }
             }
           }
        }
       }
     LoadState}
   }

   { svelteHTML.createElement("section", {});
     { svelteHTML.createElement("h2", { "class":`section-title`,});   }
       for(let l of __sveltets_2_ensureArray(limited)){l.provider + (l.account_id ?? '');
       { svelteHTML.createElement("p", {   "class":`limit`,"role":`status`,});
         { const $$_nocIredivorP3C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP3C({ target: __sveltets_2_any(), props: {    "provider":l.provider,"size":12,}});}
         { svelteHTML.createElement("span", {});providerName(l.provider);  l.until ? ` · back ${whenLabel(l.until)}` : ''; }
       }
    }
    if(assistant.usageWeek.state === 'loading' && !share.length){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "what":`this week’s usage`,"variant":`compact`,"loading":true,"empty":true,}});}
    } else if (assistant.usageWeek.state === 'error' && !share.length){
       { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`this week’s usage`,"variant":`compact`,"error":assistant.usageWeek.error || 'The daemon didn’t answer.',"empty":true,"onretry":() => void assistant.loadUsageWeek(),}});}
    } else if (share.length){
       { svelteHTML.createElement("div", {     "class":`share`,"role":`img`,"aria-label":share.map((s) => `${providerName(s.provider)} ${s.pct}%`).join(', '),});
           for(let s of __sveltets_2_ensureArray(share)){s.provider;
           { svelteHTML.createElement("span", {  "class":`seg ${s.provider}`,});__sveltets_2_ensureType(String, Number, `${s.pct} `); }
        }
       }
       { svelteHTML.createElement("ul", { "class":`legend`,});
           for(let s of __sveltets_2_ensureArray(share)){s.provider;
           { svelteHTML.createElement("li", {});
             { const $$_nocIredivorP4C = __sveltets_2_ensureComponent(ProviderIcon); new $$_nocIredivorP4C({ target: __sveltets_2_any(), props: {    "provider":s.provider,"size":12,}});}
             { svelteHTML.createElement("span", { "class":`grow`,});providerName(s.provider); }
             { svelteHTML.createElement("span", { "class":`num`,});s.pct;  }
             { svelteHTML.createElement("span", { "class":`dim num`,});formatCount(s.tokens);  }
           }
        }
       }
    } else if (!limited.length){
       { svelteHTML.createElement("p", { "class":`dim`,});
        assistant.usageWeek.state === 'unsupported' ? 'Usage tracking is off or needs admin access. Limits still show here when a provider hits one.' : 'No usage recorded this week.';
       }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const NeedsYouRail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type NeedsYouRail__SvelteComponent_ = ReturnType<typeof NeedsYouRail__SvelteComponent_>;
/*Ωignore_endΩ*/export default NeedsYouRail__SvelteComponent_;