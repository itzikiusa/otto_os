///<reference types="svelte" />
;// Where the agent reasoned. Claude persists thinking blocks EMPTY (a marker
// only) and Codex drops reasoning items (counted per turn), so there is no
// text to show — but "it thought here, this much" must never be confused
// with the answer: its own violet tint, italic, a sparkle mark, and the
// turn's thinking-token count when the transcript has usage. Expanding says
// why the text is not there.

import Icon from '../../../lib/components/Icon.svelte';
import { fmtTokens } from './format';
;

  interface Props {
    /** Thinking blocks / Codex reasoning items this marker stands for. */
    count?: number;
    /** Thinking tokens of the response (null when not recorded). */
    tokens?: number | null;
    agentName: string;
    /** Codex: reasoning items are dropped by the CLI, not blanked. */
    codex?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { count = 1, tokens = null, agentName, codex = false }: Props = $props();
  let open = $state(false);
  const label = $derived(codex ? `Reasoned${count > 1 ? ` · ${count} steps` : ''}` : 'Thought');
;
async () => {

 { svelteHTML.createElement("div", {   "class":`think`,...__sveltets_2_empty({"data-thinking":count}),});open;
   { svelteHTML.createElement("button", {       "class":`think-row`,"onclick":() => (open = !open),"aria-expanded":open,"title":`Reasoning — not part of the answer`,});
     { svelteHTML.createElement("span", {   "class":`think-mark`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});} }
     { svelteHTML.createElement("span", { "class":`think-label`,});label; }
    if(tokens){ { svelteHTML.createElement("span", { "class":`think-tok`,});fmtTokens(tokens);   }}
     { svelteHTML.createElement("span", {   "class":`think-caret`,"aria-hidden":`true`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});} }
   }
  if(open){
     { svelteHTML.createElement("p", { "class":`think-note`,});
      agentName;    
      codex ? 'Codex does not save its reasoning to the rollout — only that it happened.' : 'Claude Code does not save the reasoning text to the transcript — only that it happened.';
      if(tokens){   tokens.toLocaleString();  }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ThinkingMarker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ThinkingMarker__SvelteComponent_ = ReturnType<typeof ThinkingMarker__SvelteComponent_>;
/*Ωignore_endΩ*/export default ThinkingMarker__SvelteComponent_;