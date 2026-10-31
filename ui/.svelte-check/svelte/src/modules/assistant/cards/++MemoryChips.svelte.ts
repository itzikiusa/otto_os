///<reference types="svelte" />
;// Memory chips on a response (the `memory` turns the daemon posts when the
// agent remembers or forgets). "Remembered" has Undo (forget it again);
// "Forgot" has Undo (restore it); "Suggested" waits for review on the Memory
// tab when memory approval is on. Every change stays visible in the thread.

import Icon from '../../../lib/components/Icon.svelte';
import { toasts } from '../../../lib/toast.svelte';
import { assistant, describeError } from '../../../lib/stores/assistant.svelte';
import type { ChatCard } from '../../../lib/assistant/model';
;

  type MemoryCard = Extract<ChatCard, { kind: 'memory' }>;;;
  interface Props {
    cards: MemoryCard[];
    onreview?: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  


  let { cards, onreview }: Props = $props();

  let busy = $state<string | null>(null);
  /** chip id → what this window did to it ('undone' | 'restored'). */
  let local = $state<Record<string, 'undone' | 'restored'>>({});

  /** "Remembered: x" → "x" (the daemon's text carries its own verb). */
  function body(text: string): string {
    return text.replace(/^(remembered|forgot|forgotten|suggested)\s*:\s*/i, '');
  }

  async function undo(c: MemoryCard): Promise<void> {
    const u = c.chip?.undo;
    if (!u) return;
    busy = c.id;
    try {
      if (u.kind === 'delete') await assistant.forgetMemory(u.memory_id);
      else await assistant.restoreMemory(u.undo_tokens);
      local = { ...local, [c.id]: u.kind === 'delete' ? 'undone' : 'restored' };
    } catch (e) {
      toasts.error(u.kind === 'delete' ? 'Couldn’t forget that memory' : 'Couldn’t restore that memory', describeError(e));
    } finally {
      busy = null;
    }
  }

  async function redo(c: MemoryCard): Promise<void> {
    const u = c.chip?.undo;
    if (!u || u.kind !== 'delete') return;
    const token = assistant.forgotten[u.memory_id];
    if (!token) return;
    busy = c.id;
    try {
      await assistant.restoreMemory([token]);
      const { [c.id]: _gone, ...rest } = local;
      local = rest;
    } catch (e) {
      toasts.error('Couldn’t restore that memory', describeError(e));
    } finally {
      busy = null;
    }
  }
;
async () => {

if(cards.length){
   { svelteHTML.createElement("ul", {   "class":`chips`,"aria-label":`Memory`,});
       for(let c of __sveltets_2_ensureArray(cards)){c.id;
      const action = c.chip?.action ?? 'remembered';
      const state = local[c.id];
      if(state === 'undone'){
         { svelteHTML.createElement("li", {   "class":`chip-m gone`,...__sveltets_2_empty({"data-testid":`memory-undone`}),});
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} { svelteHTML.createElement("span", {   "class":`t`,"title":body(c.turn.text),}); body(c.turn.text); }
           { svelteHTML.createElement("button", {       "class":`act`,"onclick":() => void redo(c),"disabled":busy === c.id,"title":`Remember it again`,});  }
         }
      } else if (action === 'remembered' || state === 'restored'){
         { svelteHTML.createElement("li", {   "class":`chip-m new`,...__sveltets_2_empty({"data-testid":`memory-new`}),});
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});} { svelteHTML.createElement("span", {   "class":`t`,"title":body(c.turn.text),}); body(c.turn.text); }
          if(c.chip?.undo?.kind === 'delete'){
             { svelteHTML.createElement("button", {         "class":`act`,"onclick":() => void undo(c),"disabled":busy === c.id,"aria-label":`Undo: forget “${body(c.turn.text)}”`,"title":`Forget this memory`,});
              busy === c.id ? 'Forgetting…' : 'Undo';
             }
          }
         }
      } else if (action === 'forgot'){
         { svelteHTML.createElement("li", { "class":`chip-m gone`,});
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} { svelteHTML.createElement("span", {   "class":`t`,"title":body(c.turn.text),}); body(c.turn.text); }
          if(c.chip?.undo?.kind === 'restore'){
             { svelteHTML.createElement("button", {       "class":`act`,"onclick":() => void undo(c),"disabled":busy === c.id,"title":`Restore what was forgotten`,});busy === c.id ? 'Restoring…' : 'Undo'; }
          }
         }
      }else{
         { svelteHTML.createElement("li", {   "class":`chip-m pending`,"title":`Memory approval is on — nothing is saved until you accept it`,});
           { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`clock`,"size":12,}});} { svelteHTML.createElement("span", { "class":`t`,}); body(c.turn.text); }
          if(onreview){ { svelteHTML.createElement("button", {   "class":`act`,"onclick":onreview,});  }}
         }
      }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MemoryChips__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MemoryChips__SvelteComponent_ = ReturnType<typeof MemoryChips__SvelteComponent_>;
/*Ωignore_endΩ*/export default MemoryChips__SvelteComponent_;