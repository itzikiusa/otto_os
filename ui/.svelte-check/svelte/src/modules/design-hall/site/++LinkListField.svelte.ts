///<reference types="svelte" />
;// Site Studio inspector — a "links" field (nav, footer, button rows): a
// label and a URL per row. Rows are keyed by a client id, not their index,
// so a removed row takes its own inputs with it (focus and caret stay on the
// row they were in); each row's controls carry that row's name; and Remove
// moves focus to the Remove now in its place (else the previous row's, else
// "Add link") instead of dropping it to the page.

import { tick } from 'svelte';
import Icon from '../../../lib/components/Icon.svelte';
import type { SiteLink } from './engine/types';
;

  interface Props {
    label: string;
    rows: SiteLink[];
    readonly: boolean;
    onchange: (rows: SiteLink[]) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { label, rows, readonly, onchange }: Props = $props();

  let seq = 0;
  let keys: number[] = [];
  /** One stable key per row; an outside change of length (undo, an agent
   *  edit) re-keys, an edit made here keeps the keys in step. */
  const keyed = $derived.by(() => {
    if (keys.length !== rows.length) keys = rows.map(() => ++seq);
    return rows.map((l, i) => ({ l, key: keys[i] }));
  });
  let listEl = $state<HTMLElement | null>(null);

  const rowName = (l: SiteLink, i: number): string => l.label.trim() || `link ${i + 1}`;
  function set(i: number, part: 'label' | 'href', value: string): void {
    onchange(rows.map((l, j) => (j === i ? { ...l, [part]: value } : l)));
  }
  function add(): void {
    keys = [...keys, ++seq];
    onchange([...rows, { label: 'New link', href: '#' }]);
  }
  async function remove(i: number): Promise<void> {
    keys = keys.filter((_, j) => j !== i);
    onchange(rows.filter((_, j) => j !== i));
    await tick();
    const removes = listEl?.querySelectorAll<HTMLElement>('[data-remove-row]') ?? [];
    (removes[Math.min(i, removes.length - 1)] ?? listEl?.querySelector<HTMLElement>('[data-add-row]'))?.focus();
  }
;
async () => {

 { const $$_div0 = svelteHTML.createElement("div", {      "class":`field wide`,"role":`group`,"aria-label":label,});listEl = $$_div0;
   { svelteHTML.createElement("span", { "class":`lbl`,});label; }
      for(let { l, key } of __sveltets_2_ensureArray(keyed)){let i = 1;key;
     { svelteHTML.createElement("div", { "class":`linkrow`,});
       { svelteHTML.createElement("input", {            "dir":`auto`,"class":`input lf-input`,"value":l.label,"aria-label":`Label of ${rowName(l, i)}`,"disabled":readonly,"oninput":(e) => set(i, 'label', e.currentTarget.value),});}
       { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input lf-input`,"value":l.href,"aria-label":`URL of ${rowName(l, i)}`,"disabled":readonly,"oninput":(e) => set(i, 'href', e.currentTarget.value),});}
       { svelteHTML.createElement("button", {         "class":`icon-btn`,...__sveltets_2_empty({"data-remove-row":true}),"onclick":() => void remove(i),"disabled":readonly,"aria-label":`Remove link ${rowName(l, i)}`,"title":`Remove link ${rowName(l, i)}`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":12,}});} }
     }
  }
  if(!readonly){ { svelteHTML.createElement("button", {   "class":`btn small ghost`,...__sveltets_2_empty({"data-add-row":true}),"onclick":add,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }}
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const LinkListField__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LinkListField__SvelteComponent_ = ReturnType<typeof LinkListField__SvelteComponent_>;
/*Ωignore_endΩ*/export default LinkListField__SvelteComponent_;