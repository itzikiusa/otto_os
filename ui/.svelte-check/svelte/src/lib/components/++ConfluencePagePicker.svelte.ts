///<reference types="svelte" />
;
import { tick, untrack } from 'svelte';
import { searchConfluence } from '../api/issues';
import type { ConfluencePageSummary } from '../api/types/product';
import { loadErrorText } from '../loadError';

;type $$ComponentProps =  {
    accountId: string;
    spaceKey: string;
    value: string | null;
    title?: string | null;
    onchange: (id: string | null, title: string | null) => void;
    label?: string;
    id?: string;
    disabled?: boolean;
  };function $$render() {

  
  
  
  

  let {
    accountId, spaceKey, value, title = null, onchange,
    label = 'Parent page', id = 'confluence-parent', disabled = false,
  }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let q = $state('');
  let hits = $state<ConfluencePageSummary[]>([]);
  let searching = $state(false);
  let error = $state('');
  let open = $state(false);
  let active = $state(-1);
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let ctl: AbortController | null = null;
  const listId = $derived(`${id}-results`);

  function cancel(): void {
    ++seq;
    if (timer) clearTimeout(timer);
    ctl?.abort();
    ctl = null;
  }

  function schedule(): void {
    cancel();
    error = '';
    const text = q.trim();
    if (!text || !accountId || !spaceKey) { hits = []; searching = false; return; }
    const mine = seq;
    searching = true;
    timer = setTimeout(async () => {
      const c = new AbortController();
      ctl = c;
      try {
        const res = await searchConfluence({ accountId, q: text, space: spaceKey }, c.signal);
        if (mine !== seq) return;
        hits = res;
        active = res.length > 0 ? 0 : -1;
      } catch (e) {
        if (mine === seq && !c.signal.aborted) { error = `Couldn’t search pages. ${loadErrorText(e)}`; hits = []; }
      } finally {
        if (mine === seq) searching = false;
      }
    }, 250);
  }

  // A different space or account invalidates the current pick.
  let lastScope = '';
  $effect(() => {
    const scope = `${accountId}\u0000${spaceKey}`;
    if (lastScope && scope !== lastScope && untrack(() => value)) onchange(null, null);
    lastScope = scope;
    hits = []; q = ''; open = false;
    return cancel;
  });

  // Picking unmounts the focused input and Clear unmounts itself: hand focus
  // to the control that replaces each, or it falls to <body> (outside the
  // Modal's flow).
  let inputEl = $state<HTMLInputElement | null>(null);
  let clearEl = $state<HTMLButtonElement | null>(null);

  async function pick(h: ConfluencePageSummary): Promise<void> {
    onchange(h.id, h.title);
    q = ''; hits = []; open = false; active = -1;
    await tick();
    clearEl?.focus();
  }

  async function clear(): Promise<void> {
    onchange(null, null);
    await tick();
    inputEl?.focus();
  }

  /** Move the highlighted option and keep it inside the capped list. */
  async function moveActive(next: number): Promise<void> {
    open = true;
    active = next;
    await tick();
    document.getElementById(`${listId}-${active}`)?.scrollIntoView({ block: 'nearest' });
  }

  function onkey(e: KeyboardEvent): void {
    if (e.key === 'ArrowDown' && hits.length) { e.preventDefault(); void moveActive((active + 1) % hits.length); }
    else if (e.key === 'ArrowUp' && hits.length) { e.preventDefault(); void moveActive((active - 1 + hits.length) % hits.length); }
    else if (e.key === 'Enter' && open && active >= 0 && hits[active]) { e.preventDefault(); void pick(hits[active]); }
    else if (e.key === 'Escape' && open) { e.preventDefault(); e.stopPropagation(); open = false; }
  }
;
async () => {


 { svelteHTML.createElement("div", { "class":`cpp`,});
   { svelteHTML.createElement("label", {   "class":`label`,"for":id,});label;  { svelteHTML.createElement("span", { "class":`dim`,});  } }
  if(value){
     { svelteHTML.createElement("div", { "class":`picked`,});
       { svelteHTML.createElement("span", {   "class":`picked-title`,"title":title ?? undefined,});title ?? `Page ${value}`; }
       { const $$_button2 = svelteHTML.createElement("button", {             "type":`button`,"class":`btn small`,disabled,"aria-label":`Clear ${label.toLowerCase()}`,"title":`Clear ${label.toLowerCase()}`,"onclick":() => void clear(),});clearEl = $$_button2;  }
     }
  }else{
     { const $$_input1 = svelteHTML.createElement("input", {                                 "dir":`auto`,id,"class":`input`,"type":`search`,"role":`combobox`,"autocomplete":`off`,"spellcheck":`false`,"aria-autocomplete":`list`,"aria-controls":listId,"aria-expanded":open && hits.length > 0,"aria-activedescendant":open && active >= 0 ? `${listId}-${active}` : undefined,"placeholder":spaceKey ? `Search pages in ${spaceKey}…` : 'Pick a space first',"disabled":disabled || !spaceKey,"bind:value":q,"oninput":() => { open = true; schedule(); },"onkeydown":onkey,});/*Ωignore_startΩ*/() => q = __sveltets_2_any(null);/*Ωignore_endΩ*/inputEl = $$_input1;}
    if(open && hits.length > 0){
       { svelteHTML.createElement("ul", {       "id":listId,"class":`results`,"role":`listbox`,"aria-label":`${label} results`,});
            for(let h of __sveltets_2_ensureArray(hits)){let i = 1;h.id;
           { svelteHTML.createElement("li", {            "id":`${listId}-${i}`,"role":`option`,"tabindex":-1,"aria-selected":i === active,"onmousedown":(e) => { e.preventDefault(); void pick(h); },});i === active;h.title; }
        }
       }
    }
     { svelteHTML.createElement("p", {   "class":`hint`,"aria-live":`polite`,});
      if(error){ { svelteHTML.createElement("span", { "class":`err`,});error; }
      } else if (searching){ } else if (open && q.trim() && hits.length === 0){   q.trim(); }else{           }
     }
  }
 }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ConfluencePagePicker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ConfluencePagePicker__SvelteComponent_ = ReturnType<typeof ConfluencePagePicker__SvelteComponent_>;
/*Ωignore_endΩ*/export default ConfluencePagePicker__SvelteComponent_;