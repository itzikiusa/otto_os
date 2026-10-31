///<reference types="svelte" />
;// CSV / TSV table: sticky header, row numbers, capped at CSV_ROW_CAP rows
// (a note says how many were left out) so a large export never floods the DOM.

import { parseCsv } from '../lib/csv';
import { CSV_ROW_CAP, csvView } from './kinds';
import PreviewError from './PreviewError.svelte';
;

  interface Props {
    content: string;
    delimiter: ',' | '\t';
  };function $$render() {

  
  
  
  
  

  let { content, delimiter }: Props = $props();

  const view = $derived.by(() => {
    try {
      return { ok: true as const, v: csvView(parseCsv(content, delimiter)) };
    } catch (e) {
      return { ok: false as const, error: e instanceof Error ? e.message : String(e) };
    }
  });
  const cols = $derived(view.ok ? Array.from({ length: view.v.width }, (_, i) => i) : []);
;
async () => {

if(!view.ok){
   { const $$_rorrEweiverP0C = __sveltets_2_ensureComponent(PreviewError); new $$_rorrEweiverP0C({ target: __sveltets_2_any(), props: {    "title":`Couldn’t parse the table`,"message":view.error,}});}
} else if (view.v.header.length === 0){
   { svelteHTML.createElement("div", { "class":`state`,});       }
}else{
   { svelteHTML.createElement("div", { "class":`tbl-wrap`,});
    if(view.v.total > view.v.rows.length){
       { svelteHTML.createElement("p", { "class":`note`,});
           CSV_ROW_CAP.toLocaleString();  view.v.total.toLocaleString(); 
       }
    }
    
     { svelteHTML.createElement("div", {       "class":`scroller`,"tabindex":0,"role":`region`,"aria-label":`Table preview`,});
       { svelteHTML.createElement("table", {});
         { svelteHTML.createElement("thead", {});
           { svelteHTML.createElement("tr", {});
             { svelteHTML.createElement("th", {   "class":`num`,"scope":`col`,});  }
               for(let c of __sveltets_2_ensureArray(cols)){c;
               { svelteHTML.createElement("th", { "scope":`col`,});view.v.header[c] ?? ''; }
            }
           }
         }
         { svelteHTML.createElement("tbody", {});
              for(let r of __sveltets_2_ensureArray(view.v.rows)){let i = 1;i;
             { svelteHTML.createElement("tr", {});
               { svelteHTML.createElement("td", { "class":`num`,});i + 1; }
                 for(let c of __sveltets_2_ensureArray(cols)){c;
                 { svelteHTML.createElement("td", {});r[c] ?? ''; }
              }
             }
          }
         }
       }
     }
   }
}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const CsvPreview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type CsvPreview__SvelteComponent_ = ReturnType<typeof CsvPreview__SvelteComponent_>;
/*Ωignore_endΩ*/export default CsvPreview__SvelteComponent_;