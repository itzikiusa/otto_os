///<reference types="svelte" />
;
import type { RestorePreview } from '../../lib/api/types';
import { plural } from '../../lib/plural';

;type $$ComponentProps =  {
    preview: RestorePreview; busy: boolean; reviewed?: boolean; onrestore: () => void;
  };function $$render() {

  
  
  let { preview, busy, reviewed = $bindable(false), onrestore }:/*Ωignore_startΩ*/$$ComponentProps/*Ωignore_endΩ*/ = $props()/*Ωignore_startΩ*/;reviewed;/*Ωignore_endΩ*/;
;
async () => {

     { svelteHTML.createElement("div", {   "class":`preview`,"aria-label":`Restore preview`,});
       { svelteHTML.createElement("strong", {});plural(preview.record_count, 'record');  plural(preview.file_count, 'file');  plural(preview.conflicts.length, 'conflict'); }
       { svelteHTML.createElement("details", {"open":true,}); { svelteHTML.createElement("summary", {});  } { svelteHTML.createElement("div", { "class":`scroll`,}); { svelteHTML.createElement("table", {}); { svelteHTML.createElement("thead", {}); { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("th", { "scope":`col`,});  } { svelteHTML.createElement("th", {   "scope":`col`,"class":`num`,});  } } } { svelteHTML.createElement("tbody", {});  for(let [table, count] of __sveltets_2_ensureArray((Object.entries(preview.table_counts).sort(([a], [b]) => a.localeCompare(b))))){ { svelteHTML.createElement("tr", {}); { svelteHTML.createElement("td", {});table.replaceAll('_', ' '); } { svelteHTML.createElement("td", { "class":`num`,});count; } }} } } } }
      if(preview.conflicts.length){ { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});  preview.conflicts.length;  } { svelteHTML.createElement("ul", {});  for(let conflict of __sveltets_2_ensureArray((preview.conflicts.slice(0, 100)))){ { svelteHTML.createElement("li", {});conflict.location; conflict.reason; }}if(preview.conflicts.length > 100){ { svelteHTML.createElement("li", {});     }} } }}
      if(preview.excluded.length){ { svelteHTML.createElement("details", {}); { svelteHTML.createElement("summary", {});  } { svelteHTML.createElement("ul", {});  for(let item of __sveltets_2_ensureArray(preview.excluded)){ { svelteHTML.createElement("li", {});item; }} } }}
      if(preview.reconnect.length){ { svelteHTML.createElement("details", {"open":true,}); { svelteHTML.createElement("summary", {});   } { svelteHTML.createElement("ul", {});  for(let item of __sveltets_2_ensureArray(preview.reconnect)){ { svelteHTML.createElement("li", {});item; }} } }}
      if(preview.warnings.length){ { svelteHTML.createElement("ul", { "class":`warnings`,});  for(let warning of __sveltets_2_ensureArray(preview.warnings)){ { svelteHTML.createElement("li", {});warning; }} }}
      if(!preview.can_restore){ { svelteHTML.createElement("p", { "class":`error`,});             }}
       { svelteHTML.createElement("label", { "class":`reviewed`,}); { svelteHTML.createElement("input", {      "type":`checkbox`,"bind:checked":reviewed,"disabled":busy || !preview.can_restore,});/*Ωignore_startΩ*/() => reviewed = __sveltets_2_any(null);/*Ωignore_endΩ*/}         }
       { svelteHTML.createElement("button", {     "class":`btn primary`,"disabled":busy || !preview.can_restore || !reviewed,"onclick":onrestore,});busy ? 'Restoring…' : 'Restore new items'; }
     }


};
return { props: {} as any as $$ComponentProps, exports: {}, bindings: __sveltets_$$bindings('reviewed'), slots: {}, events: {} }}
const RestorePreviewPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type RestorePreviewPanel__SvelteComponent_ = ReturnType<typeof RestorePreviewPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default RestorePreviewPanel__SvelteComponent_;