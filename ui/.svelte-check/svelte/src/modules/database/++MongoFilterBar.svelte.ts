///<reference types="svelte" />
;// Quick Mongo filter bar: type a `{ field: value }` object, press Enter, and
// the active `db.<coll>.find(...)` is rewritten with it (Replace = new
// filter, AND = merged into the existing one) and run — the one-line answer
// to "show me the docs where …" without editing the query text by hand.
// Column chips insert `"col": ` at the caret. Renders nothing unless the
// statement is a single `find` the splicer can rewrite (`applyMongoFilterObject`).

import Icon from '../../lib/components/Icon.svelte';
import { toasts } from '../../lib/toast.svelte';
import { radioKey } from '../../lib/radioKey';
import { applyMongoFilterObject, type FilterMode } from './query-filter';
;

  interface Props {
    /** The statement that PRODUCED the rows on screen (`ran_statement`). */
    statement: string;
    /** Result column names — one chip each. */
    columns: string[];
    onrun: (q: string) => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { statement, columns, onrun }: Props = $props();

  let text = $state('');
  let mode = $state<FilterMode>('set');
  let inputEl = $state<HTMLInputElement | null>(null);

  // Hidden for aggregates, batches, scripts and SELECT-translated finds — the
  // same gate the rewrite itself applies, probed with an empty filter.
  const visible = $derived(applyMongoFilterObject(statement, '{}', 'set') !== null);

  /** Chips are capped so a 200-field projection doesn't bury the input. */
  const CHIP_MAX = 24;
  const chips = $derived(columns.slice(0, CHIP_MAX));

  function submit(): void {
    const src = text.trim() || '{}';
    const q = applyMongoFilterObject(statement, src, mode);
    if (q === null) {
      toasts.error('Filter must be a { … } object');
      inputEl?.focus();
      return;
    }
    onrun(q);
  }

  /** Insert `"col": ` at the caret (wrapping an empty box in `{ … }` and
   *  separating from a preceding entry with a comma) and refocus. */
  function insertChip(col: string): void {
    const el = inputEl;
    const token = `${JSON.stringify(col)}: `;
    if (!el || text.trim() === '') {
      text = `{ ${token} }`;
      queueMicrotask(() => {
        el?.focus();
        el?.setSelectionRange(text.length - 2, text.length - 2);
      });
      return;
    }
    const start = el.selectionStart ?? text.length;
    const end = el.selectionEnd ?? start;
    const before = text.slice(0, start);
    const sep = /[^\s{,]\s*$/.test(before) ? ', ' : '';
    text = before + sep + token + text.slice(end);
    const caret = before.length + sep.length + token.length;
    queueMicrotask(() => {
      el.focus();
      el.setSelectionRange(caret, caret);
    });
  }

  function onKeydown(e: KeyboardEvent): void {
    if (e.key === 'Enter') {
      e.preventDefault();
      submit();
    } else if (e.key === 'Escape' && text) {
      e.preventDefault();
      text = '';
    }
  }
;
async () => {

if(visible){
   { svelteHTML.createElement("div", {   "class":`mfb`,...__sveltets_2_empty({"data-testid":`mongo-filter-bar`}),});
     { svelteHTML.createElement("span", {   "class":`mfb-label`,"title":`Rewrite the active find() filter and run`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}  }
     { const $$_input1 = svelteHTML.createElement("input", {                   "dir":`ltr`,"class":`mfb-input mono`,"type":`text`,"placeholder":'{ field: value }',"bind:value":text,"onkeydown":onKeydown,"spellcheck":`false`,"autocomplete":`off`,"aria-label":`Mongo filter object`,});inputEl = $$_input1;/*Ωignore_startΩ*/() => text = __sveltets_2_any(null);/*Ωignore_endΩ*/}
     { svelteHTML.createElement("div", {     "class":`mfb-mode`,"role":`radiogroup`,"aria-label":`Filter mode`,});
       { svelteHTML.createElement("button", {              "class":`mfb-seg`,"role":`radio`,"aria-checked":mode === 'set',"tabindex":mode === 'set' ? 0 : -1,"onkeydown":radioKey,"onclick":() => (mode = 'set'),"title":`Replace the current filter`,});mode === 'set';  }
       { svelteHTML.createElement("button", {              "class":`mfb-seg`,"role":`radio`,"aria-checked":mode === 'and',"tabindex":mode === 'and' ? 0 : -1,"onkeydown":radioKey,"onclick":() => (mode = 'and'),"title":`Merge into the current filter`,});mode === 'and';  }
     }
     { svelteHTML.createElement("button", {     "class":`mfb-run`,"onclick":submit,"title":`Rewrite the find() filter and run (Enter)`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`play`,"size":12,}});}  }
    if(chips.length > 0){
       { svelteHTML.createElement("div", {   "class":`mfb-chips`,"aria-label":`Insert a field`,});
           for(let col of __sveltets_2_ensureArray(chips)){col;
           { svelteHTML.createElement("button", {     "class":`mfb-chip mono`,"onclick":() => insertChip(col),"title":`Insert "${col}": at the caret`,});col; }
        }
        if(columns.length > chips.length){ { svelteHTML.createElement("span", { "class":`mfb-more`,}); columns.length - chips.length; }}
       }
    }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const MongoFilterBar__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type MongoFilterBar__SvelteComponent_ = ReturnType<typeof MongoFilterBar__SvelteComponent_>;
/*Ωignore_endΩ*/export default MongoFilterBar__SvelteComponent_;