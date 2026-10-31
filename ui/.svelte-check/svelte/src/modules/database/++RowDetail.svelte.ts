///<reference types="svelte" />
;// Row detail side panel for the results grid (TablePlus's "row detail"): the
// record under the grid cursor, one field per line — name + type, the value
// wrapped in full (NULL dim, JSON pretty-printed), a per-field copy. Read-only
// on purpose: editing stays in the grid / Vertical view where it is
// review-gated. ‹ › step through the rows in view.

import Icon from '../../lib/components/Icon.svelte';
import type { QueryResult } from '../../lib/api/types';
import { cellStr, copyWithToast, isComplex, prettyJson } from './results-format';
import { columnKind } from './grid-format';
;

  interface Props {
    result: QueryResult;
    row: unknown[] | null;
    /** liveRows index of `row` (shown as "Row N"). */
    rowIdx: number | null;
    /** Position of the row within the filtered/sorted view (-1 = not in view). */
    position: number;
    total: number;
    onstep: (delta: number) => void;
    /** Open a field in the full cell viewer. */
    onopen: (ci: number) => void;
    /** Open the whole row as searchable raw JSON (labelled `rawLabel`). */
    onraw?: () => void;
    rawLabel?: string;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { result, row, rowIdx, position, total, onstep, onopen, onraw, rawLabel = 'View raw JSON', onclose }: Props = $props();

  let fieldFilter = $state('');
  const fields = $derived.by(() => {
    const q = fieldFilter.trim().toLowerCase();
    return result.columns
      .map((c, ci) => ({ c, ci }))
      .filter(({ c }) => !q || c.name.toLowerCase().includes(q));
  });
  function text(v: unknown): string {
    if (v === null || v === undefined) return '';
    return isComplex(v) ? prettyJson(v) : cellStr(v);
  }
;
async () => {

 { svelteHTML.createElement("aside", {   "class":`row-detail`,"aria-label":`Row detail`,});
   { svelteHTML.createElement("header", { "class":`rd-head`,});
     { svelteHTML.createElement("span", { "class":`rd-title`,});rowIdx === null ? 'Row' : `Row ${rowIdx + 1}`; }
    if(position >= 0){ { svelteHTML.createElement("span", { "class":`rd-pos`,});position + 1;  total.toLocaleString(); }}
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    if(onraw){
       { svelteHTML.createElement("button", {         "class":`icon-btn`,"disabled":!row,"onclick":onraw,"aria-label":rawLabel,"title":rawLabel,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":13,}});}
       }
    }
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"disabled":position <= 0,"onclick":() => onstep(-1),"aria-label":`Previous row`,"title":`Previous row`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronUp`,"size":13,}});}
     }
     { svelteHTML.createElement("button", {         "class":`icon-btn`,"disabled":position < 0 || position >= total - 1,"onclick":() => onstep(1),"aria-label":`Next row`,"title":`Next row`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":13,}});}
     }
     { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":onclose,"aria-label":`Close row detail`,"title":`Close row detail`,});
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":13,}});}
     }
   }
   { svelteHTML.createElement("div", { "class":`rd-search`,});
     { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
     { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`rd-search-input`,"type":`text`,"placeholder":`Find a field…`,"aria-label":`Find a field`,"spellcheck":`false`,"bind:value":fieldFilter,});/*Ωignore_startΩ*/() => fieldFilter = __sveltets_2_any(null);/*Ωignore_endΩ*/}
   }
  if(!row){
     { svelteHTML.createElement("p", { "class":`rd-empty`,});             }
  }else{
     { svelteHTML.createElement("dl", { "class":`rd-fields`,});
         for(let { c, ci } of __sveltets_2_ensureArray(fields)){ci;
        const v = row[ci];
        const kind = columnKind(c.type_hint, [v]);
         { svelteHTML.createElement("div", { "class":`rd-field`,});
           { svelteHTML.createElement("dt", {});
             { svelteHTML.createElement("span", {   "class":`rd-name mono`,"title":c.name,});c.name; }
            if(c.type_hint){ { svelteHTML.createElement("span", { "class":`rd-type mono`,});c.type_hint; }}
             { svelteHTML.createElement("span", { "class":`grow`,}); }
             { svelteHTML.createElement("button", {         "class":`icon-btn rd-act`,"onclick":() => copyWithToast(v === null || v === undefined ? '' : cellStr(v)),"aria-label":`Copy ${c.name}`,"title":`Copy value`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} }
             { svelteHTML.createElement("button", {       "class":`icon-btn rd-act`,"onclick":() => onopen(ci),"aria-label":`Open ${c.name} in the viewer`,"title":`Open in the viewer`,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`maximize`,"size":12,}});}
             }
           }
           { svelteHTML.createElement("dd", {  "class":`mono`,});kind === 'num';
            if(v === null || v === undefined){
               { svelteHTML.createElement("span", { "class":`rd-null`,});  }
            } else if (isComplex(v)){
               { svelteHTML.createElement("pre", { "class":`rd-json`,});text(v); }
            }else{
              text(v);
            }
           }
         }
      }
      if(fields.length === 0){
         { svelteHTML.createElement("p", { "class":`rd-empty`,});   fieldFilter;  }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const RowDetail__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RowDetail__SvelteComponent_ = ReturnType<typeof RowDetail__SvelteComponent_>;
/*Ωignore_endΩ*/export default RowDetail__SvelteComponent_;