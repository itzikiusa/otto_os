///<reference types="svelte" />
;// ⌘P quick open: fuzzy (subsequence) filter over the workbench's files.

import Modal from '../../lib/components/Modal.svelte';
import { workbench, sortDocs } from './workbench.svelte';
import type { WorkbenchDoc } from '../../lib/api/types';
;

  interface Props {
    onclose: () => void;
    onopen: (id: string) => void;
  };function $$render() {

  
  
  
  

  let { onclose, onopen }: Props = $props();

  let query = $state('');
  let index = $state(0);
  let input: HTMLInputElement | undefined = $state();

  /** Subsequence score: lower is better; -1 = no match. Contiguous runs and
   *  matches at the start of the name rank first. */
  function score(name: string, q: string): number {
    if (!q) return 0;
    const n = name.toLowerCase();
    const direct = n.indexOf(q);
    if (direct >= 0) return direct;
    let pos = -1;
    let gaps = 0;
    for (const ch of q) {
      const next = n.indexOf(ch, pos + 1);
      if (next < 0) return -1;
      if (pos >= 0) gaps += next - pos - 1;
      pos = next;
    }
    return 100 + gaps;
  }

  const results = $derived.by((): WorkbenchDoc[] => {
    const q = query.trim().toLowerCase();
    const all = sortDocs(workbench.docs);
    if (!q) return all.slice(0, 50);
    return all
      .map((d) => ({ d, s: score(d.name, q) }))
      .filter((x) => x.s >= 0)
      .sort((a, b) => a.s - b.s)
      .slice(0, 50)
      .map((x) => x.d);
  });

  $effect(() => {
    void query;
    index = 0;
  });

  $effect(() => {
    input?.focus();
  });

  // Keep the active option visible as the arrows move it through a long list.
  $effect(() => {
    document.getElementById(`wb-qo-${index}`)?.scrollIntoView({ block: 'nearest' });
  });

  function pick(d: WorkbenchDoc | undefined): void {
    if (!d) return;
    onopen(d.id);
    onclose();
  }

  function onKey(e: KeyboardEvent): void {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      index = Math.min(index + 1, results.length - 1);
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      index = Math.max(index - 1, 0);
    } else if (e.key === 'Enter') {
      e.preventDefault();
      pick(results[index]);
    }
  }
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {  children:() => { return __sveltets_2_any(0); },"title":`Open file`,onclose,}});
  
   { const $$_input1 = svelteHTML.createElement("input", {                           "dir":`ltr`,"class":`wb-qo-input`,"type":`search`,"placeholder":`e.g. deploy.sh`,"aria-label":`File name`,"role":`combobox`,"aria-expanded":`true`,"aria-autocomplete":`list`,"aria-controls":`wb-qo-list`,"aria-activedescendant":results.length > 0 ? `wb-qo-${index}` : undefined,"bind:value":query,"onkeydown":onKey,...__sveltets_2_empty({"data-testid":`wb-quickopen-input`}),});input = $$_input1;/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   { svelteHTML.createElement("div", {       "class":`wb-qo-list`,"id":`wb-qo-list`,"role":`listbox`,"aria-label":`Files`,});
        for(let d of __sveltets_2_ensureArray(results)){let i = 1;d.id;
       { svelteHTML.createElement("button", {                  "id":`wb-qo-${i}`,"tabindex":-1,"role":`option`,"aria-selected":i === index,"class":`wb-qo-row`,"onmousedown":(e) => e.preventDefault(),"onclick":() => pick(d),"onmouseenter":() => (index = i),});i === index;
         { svelteHTML.createElement("span", { "class":`wb-qo-name`,});d.name; }
         { svelteHTML.createElement("span", { "class":`wb-qo-lang`,});d.language === 'auto' ? '' : d.language; }
       }
    }
       { svelteHTML.createElement("div", { "class":`wb-qo-empty`,});   }
    
   }
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const QuickOpen__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type QuickOpen__SvelteComponent_ = ReturnType<typeof QuickOpen__SvelteComponent_>;
/*Ωignore_endΩ*/export default QuickOpen__SvelteComponent_;