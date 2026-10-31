///<reference types="svelte" />
;
import { loops } from '../../lib/stores/loops.svelte';
import { toastError } from '../../lib/toastError';
import type { GoalLoopIteration, LoopAgentState } from '../../lib/api/types';
import Icon from '../../lib/components/Icon.svelte';
import LoadState from '../../lib/components/LoadState.svelte';
import { loadErrorText } from '../../lib/loadError';
import StatusBadge from '../../lib/components/StatusBadge.svelte';
import { runStatus } from '../../lib/status';

;type $$ComponentProps =  {
    iter: GoalLoopIteration;
    loopId: string;
    loopStatus: string;
    executorCount: number;
    open?: boolean;
    /** Criterion id → text, so evaluation chips name what they judged. */
    criteria?: Record<string, string>;
    onopensession: (sessionId: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  

  let {
    iter,
    loopId,
    loopStatus,
    executorCount,
    open = false,
    criteria = {},
    onopensession,
  }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  // Initial expand state from the `open` prop (newest iteration starts open);
  // toggled independently thereafter.
  // svelte-ignore state_referenced_locally
  let expanded = $state(open);

  // The loop detail is a SUMMARY: only the newest iteration carries its plan /
  // context bodies. Expanding an older one reads it in full (cached per
  // idx+status in the store, so a finished iteration is fetched once).
  const needsBodies = $derived(!iter.plan && !iter.context_out && !iter.context_in);
  let bodyError = $state<string | null>(null);
  let bodyLoading = $state(false);
  /** Bumped by Retry to re-run the read for the same iteration. */
  let bodyRetry = $state(0);
  $effect(() => {
    void bodyRetry;
    if (!expanded || !needsBodies) return;
    const it = iter;
    if (loops.fullIteration(loopId, it)) return;
    const ctl = new AbortController();
    bodyError = null;
    bodyLoading = true;
    loops
      .loadIteration(loopId, it, ctl.signal)
      .catch((e: unknown) => {
        if (!ctl.signal.aborted) bodyError = loadErrorText(e);
      })
      .finally(() => {
        if (!ctl.signal.aborted) bodyLoading = false;
      });
    return () => {
      ctl.abort();
      bodyLoading = false;
    };
  });
  const bodies = $derived(needsBodies ? (loops.fullIteration(loopId, iter) ?? iter) : iter);

  function dotClass(status: string): string {
    switch (status) {
      case 'running':
        return 'dot working';
      case 'done':
        return 'dot ok';
      case 'waiting':
        return 'dot warn';
      case 'error':
        return 'dot bad';
      default:
        return 'dot';
    }
  }

  async function retry(agentIndex: number): Promise<void> {
    try {
      await loops.retryExecutor(loopId, iter.idx, agentIndex);
    } catch (e) {
      toastError('Couldn’t retry the agent', e);
    }
  }

  const VERDICT: Record<string, string> = { achieved: 'goal met', continue: 'continuing', blocked: 'blocked' };

  // Retry is only valid while the loop is blocked (no live controller); a
  // running loop's controller owns the executor and a second run would race it.
  function canRetry(a: LoopAgentState): boolean {
    return loopStatus === 'blocked' && (a.status === 'waiting' || a.status === 'error');
  }
;
async () => {

 { svelteHTML.createElement("div", { "class":`iter`,});
   { svelteHTML.createElement("button", {     "class":`iter-head`,"aria-expanded":expanded,"onclick":() => (expanded = !expanded),});
     { svelteHTML.createElement("span", { "class":`chev`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":expanded ? 'chevronDown' : 'chevronRight',"size":12,}});} }
     { svelteHTML.createElement("span", { "class":`idx`,}); iter.idx; }
     { const $$_egdaBsutatS2C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS2C({ target: __sveltets_2_any(), props: {    "status":runStatus(iter.status),"variant":`text`,}});}
    if(iter.evaluation){
       { svelteHTML.createElement("span", { "class":`prog`,});iter.evaluation.progress_pct;  VERDICT[iter.evaluation.verdict] ?? iter.evaluation.verdict; }
    }
   }

  if(expanded){
     { svelteHTML.createElement("div", { "class":`body`,});
      if(needsBodies && !bodies.plan && (bodyLoading || bodyError)){
        
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {          "what":`this iteration’s plan`,"variant":`compact`,"loading":bodyLoading,"error":bodyError,"empty":true,"onretry":() => bodyRetry++,}});}
      }
      if(bodies.plan){
         { svelteHTML.createElement("section", {});
           { svelteHTML.createElement("h4", {});  }
           { svelteHTML.createElement("pre", { "class":`text`,});bodies.plan; }
         }
      }

       { svelteHTML.createElement("section", {});
         { svelteHTML.createElement("h4", {});   }
            for(let a of __sveltets_2_ensureArray(iter.agents)){let i = 1;i;
          const agentStatus = runStatus(a.status);
           { svelteHTML.createElement("div", { "class":`agent`,});
             { svelteHTML.createElement("span", {   "class":dotClass(a.status),"aria-hidden":`true`,}); }
             { const $$_egdaBsutatS4C = __sveltets_2_ensureComponent(StatusBadge); new $$_egdaBsutatS4C({ target: __sveltets_2_any(), props: {      "status":agentStatus,"variant":`text`,"dot":false,}});}
             { svelteHTML.createElement("span", { "class":`aname`,});a.name; }
             { svelteHTML.createElement("span", { "class":`aprov`,});a.provider; }
            if(a.note || a.output_summary){
               { svelteHTML.createElement("span", {   "class":`anote`,"title":a.note || a.output_summary,});a.note || a.output_summary; }
            }
            if(a.session_id){
               { svelteHTML.createElement("button", {     "class":`btn ghost small`,"title":`Open ${a.name}’s session`,"onclick":() => onopensession(a.session_id ?? ''),});  }
            }
            if(i < executorCount && canRetry(a)){
               { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => retry(i),});  }
            }
           }
        }
       }

      if(iter.evaluation){
         { svelteHTML.createElement("section", {});
           { svelteHTML.createElement("h4", {});  }
           { svelteHTML.createElement("div", { "class":`crits`,});
               for(let c of __sveltets_2_ensureArray(iter.evaluation.criteria)){c.id;
               { svelteHTML.createElement("span", {    "class":`crit-chip`,"title":c.evidence || undefined,});c.met;
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":c.met ? 'check' : 'dot',"size":12,}});}
                 { svelteHTML.createElement("span", { "class":`crit-label`,});criteria[c.id] || c.id; }
               }
            }
           }
          if(iter.evaluation.feedback){
             { svelteHTML.createElement("p", { "class":`fb`,}); { svelteHTML.createElement("strong", {});  } iter.evaluation.feedback; }
          }
         }
      }

      if(bodies.context_out){
         { svelteHTML.createElement("section", {});
           { svelteHTML.createElement("h4", {});   }
           { svelteHTML.createElement("pre", { "class":`text dim`,});bodies.context_out; }
         }
      }
     }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const IterationRow__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type IterationRow__SvelteComponent_ = ReturnType<typeof IterationRow__SvelteComponent_>;
/*Ωignore_endΩ*/export default IterationRow__SvelteComponent_;