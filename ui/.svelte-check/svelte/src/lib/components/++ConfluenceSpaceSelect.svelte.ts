///<reference types="svelte" />
;
import { tick } from 'svelte';
import Skeleton from './Skeleton.svelte';
import { listConfluenceSpaces } from '../api/issues';
import type { ConfluenceSpace } from '../api/types/product';
import { loadErrorText } from '../loadError';
import { truncatedNote } from '../listingPage';

;type $$ComponentProps =  {
    accountId: string;
    value: string;
    onchange: (key: string) => void;
    id?: string;
    disabled?: boolean;
    /** When set, the space is optional: a first `""` option with this label,
     *  and no auto-pick of the first space. */
    emptyOption?: string;
    /** Accessible name when no `<label for={id}>` names the select. */
    ariaLabel?: string;
  };function $$render() {

  
  
  
  
  
  

  let { accountId, value, onchange, id = 'confluence-space', disabled = false, emptyOption = undefined, ariaLabel = undefined }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props();

  let spaces = $state<ConfluenceSpace[]>([]);
  let loading = $state(false);
  let error = $state('');
  let truncated = $state(false);
  let seq = 0;

  async function load(): Promise<void> {
    const account = accountId;
    const mine = ++seq;
    spaces = []; error = ''; truncated = false;
    if (!account) return;
    loading = true;
    const ctl = new AbortController();
    abort = () => ctl.abort();
    try {
      const page = await listConfluenceSpaces(account, ctl.signal);
      if (mine !== seq) return;
      spaces = page.items;
      truncated = page.truncated;
      const listed = spaces.some((s) => s.key === value);
      if (emptyOption !== undefined) { if (value && !listed) onchange(''); }
      else if (spaces.length > 0 && !listed) onchange(spaces[0].key);
    } catch (e) {
      if (mine === seq && !ctl.signal.aborted) error = `Couldn’t load Confluence spaces. ${loadErrorText(e)}`;
    } finally {
      if (mine === seq) loading = false;
    }
  }
  let abort: () => void = () => {};

  let selectEl = $state<HTMLSelectElement>();
  let retryEl = $state<HTMLButtonElement>();
  /** Retry swaps its own button for a Skeleton, then the select (or a fresh
   *  Retry): put focus on whichever arrived, not on <body>. */
  async function retry(): Promise<void> {
    await load();
    await tick();
    (selectEl ?? retryEl)?.focus();
  }

  $effect(() => {
    void accountId;
    void load();
    return () => { ++seq; abort(); };
  });
;
async () => {


if(loading){
   { const $$_notelekS0C = __sveltets_2_ensureComponent(Skeleton); new $$_notelekS0C({ target: __sveltets_2_any(), props: {      "rows":1,"height":27,"label":`spaces`,}});}
} else if (error){
   { svelteHTML.createElement("div", {   "role":`alert`,"class":`field-error`,});error; }
   { const $$_button0 = svelteHTML.createElement("button", {      "type":`button`,"class":`btn small`,"onclick":() => void retry(),});retryEl = $$_button0;  }
}else{
   { const $$_select0 = svelteHTML.createElement("select", {            id,"aria-label":ariaLabel,"class":`input`,value,"disabled":disabled || (spaces.length === 0 && emptyOption === undefined),"onchange":(e) => onchange((e.currentTarget as HTMLSelectElement).value),});selectEl = $$_select0;
    if(emptyOption !== undefined){
       { svelteHTML.createElement("option", {"value":"",});emptyOption; }
    }
    if(spaces.length === 0 && emptyOption === undefined){
       { svelteHTML.createElement("option", {"value":"",});   }
    }
    if(spaces.length > 0){
         for(let sp of __sveltets_2_ensureArray(spaces)){sp.key;
         { svelteHTML.createElement("option", { "value":sp.key,});sp.name; sp.key;  }
      }
    }
   }
  if(truncated){
     { svelteHTML.createElement("p", {   "class":`trunc-note`,"role":`note`,});truncatedNote('spaces', spaces.length); }
  }
}


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ConfluenceSpaceSelect__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ConfluenceSpaceSelect__SvelteComponent_ = ReturnType<typeof ConfluenceSpaceSelect__SvelteComponent_>;
/*Ωignore_endΩ*/export default ConfluenceSpaceSelect__SvelteComponent_;