///<reference types="svelte" />
;// What the agent is doing RIGHT NOW, at the foot of the chat:
//   • working → one quiet line: spinner · "Claude is working" · the current
//     step ("Running cargo test …") · elapsed since your last message;
//   • waiting → a card: the agent stopped with a tool call that never got a
//     result — in Claude Code / Codex that is a permission prompt (or an
//     AskUserQuestion) on the terminal screen. Otto can't answer those for
//     you from here (the transcript doesn't carry the prompt's options), so
//     the card says what is being asked and opens the terminal to answer.
// None of these blocks is a live region: ConversationView's edge-only
// announcer speaks working / finished / waiting once. A `role="status"` here
// re-read every tool step ("Reading …", "Running …") over the user.

import Icon from '../../../lib/components/Icon.svelte';
import { now } from '../../../lib/stores/now.svelte';
import { askQuestions, fmtDuration, toolLine, type ToolCallBlock } from './format';
;

  interface Props {
    mode: 'working' | 'waiting';
    agentName: string;
    /** The newest call without a result (the current step / what is asked). */
    pending: ToolCallBlock | null;
    /** The in-progress response text is streaming (shown above this line). */
    writing?: boolean;
    /** Timestamp of your last message — the turn's elapsed time. */
    since?: string | null;
    /** Switch this pane to the terminal (null in read-only views). */
    onterminal?: (() => void) | null;
    /** The events socket is down: "working" is only the last known state, so
     *  the line says "Reconnecting…" — no spinner, no elapsed clock. */
    stale?: boolean;
    /** Tooltip for the stale line (lib/status `sessionState(...).hint`). */
    staleHint?: string;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let { mode, agentName, pending, writing = false, since = null, onterminal = null, stale = false, staleHint }: Props = $props();

  const line = $derived(pending ? toolLine(pending) : null);
  const doing = $derived(
    line ? `${line.present}${line.target ? ` ${line.target}` : ''}` : writing ? 'Writing the response' : 'Thinking',
  );
  const elapsed = $derived.by(() => {
    if (!since) return '';
    const t = Date.parse(since);
    if (!Number.isFinite(t)) return '';
    const ms = now() - t;
    // Past a few hours the "last message" is not this turn's start (a resumed
    // session, a clock skew) — say nothing rather than something wrong.
    return ms > 0 && ms < 3 * 3600_000 ? fmtDuration(Math.floor(ms / 1000) * 1000) : '';
  });
  const questions = $derived(pending?.tool === 'ask' ? askQuestions(pending.input) : []);
;
async () => {

if(mode === 'working' && stale){
   { svelteHTML.createElement("div", {     "class":`live-line stale`,...__sveltets_2_empty({"data-live-status":`stale`}),"title":staleHint,});
     { const $$_nocI1C = __sveltets_2_ensureComponent(Icon); new $$_nocI1C({ target: __sveltets_2_any(), props: {    "name":`refresh`,"size":12,}});}
     { svelteHTML.createElement("span", { "class":`who`,});  }
     { svelteHTML.createElement("span", { "class":`doing`,}); { svelteHTML.createElement("span", {   "class":`sep`,"aria-hidden":`true`,});  } agentName;       }
   }
} else if (mode === 'working'){
   { svelteHTML.createElement("div", {   "class":`live-line`,...__sveltets_2_empty({"data-live-status":`working`}),});
     { svelteHTML.createElement("span", {    "class":`spinner`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, " "); }
     { svelteHTML.createElement("span", { "class":`who`,});agentName;   }
     { svelteHTML.createElement("span", {   "class":`doing`,"title":line?.hint || doing,}); { svelteHTML.createElement("span", {   "class":`sep`,"aria-hidden":`true`,});  }  { svelteHTML.createElement("span", { });!!line?.mono;doing; } }
    if(elapsed){ { svelteHTML.createElement("span", {   "class":`elapsed`,"aria-hidden":`true`,});elapsed; }}
   }
}else{
   { svelteHTML.createElement("div", {   "class":`waiting`,...__sveltets_2_empty({"data-live-status":`waiting`}),});
     { svelteHTML.createElement("div", { "class":`w-head`,});
       { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":14,}});}
       { svelteHTML.createElement("span", { "class":`w-title`,});agentName;     }
     }
    if(questions.length){
          for(let q of __sveltets_2_ensureArray(questions)){let i = 1;i;
         { svelteHTML.createElement("p", { "class":`w-q`,});q.question; }
        if(q.options.length){
           { svelteHTML.createElement("ul", {   "class":`w-opts`,"aria-label":`Options`,});
                for(let o of __sveltets_2_ensureArray(q.options)){let j = 1;j; { svelteHTML.createElement("li", {});o; }}
           }
        }
      }
       { svelteHTML.createElement("p", { "class":`w-body`,});               }
    } else if (line){
       { svelteHTML.createElement("p", { "class":`w-body`,});
        agentName;   line.base;
         { svelteHTML.createElement("code", { "class":`mono w-target`,});line.target || pending?.name; }         
            
       }
    }
    if(onterminal){
       { svelteHTML.createElement("div", { "class":`w-actions`,});
         { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":onterminal,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`terminal`,"size":12,}});}   }
       }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LiveStatus__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LiveStatus__SvelteComponent_ = ReturnType<typeof LiveStatus__SvelteComponent_>;
/*Ωignore_endΩ*/export default LiveStatus__SvelteComponent_;