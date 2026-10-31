///<reference types="svelte" />
;// Placeholders detected in the active workbench file — the SAME syntax as the
// DB Explorer's variables / Run on… (`:name` in SQL, `{name}`, `{{name}}`).
// Values live with the file (the parent persists them) and feed Send to…:
// "Copy with placeholders filled", the API client / session / vault targets,
// and the DB Run on… sheet, where a list (`1,2,3,4`) becomes one run each.

import { untrack } from 'svelte';
import { detectPlaceholders, isSqlLike } from './lib/placeholders';
;

  interface Props {
    content: string;
    lang: string;
    values: Record<string, string>;
    onchange: (values: Record<string, string>) => void;
  };function $$render() {

  
  
  
  
  
  
  

  let { content, lang, values, onchange }: Props = $props();

  // The scan is a whole-buffer pass (for SQL a quote/comment-aware parse), so
  // it reads a 200 ms debounced copy of the buffer — the list is display only.
  // A language change applies at once.
  let scanned = $state(untrack(() => content));
  let scannedLang = untrack(() => lang);
  $effect(() => {
    const next = content;
    const l = lang;
    if (l !== scannedLang) {
      scannedLang = l;
      scanned = next;
      return;
    }
    if (next === untrack(() => scanned)) return;
    const t = setTimeout(() => (scanned = next), 200);
    return () => clearTimeout(t);
  });
  const found = $derived(detectPlaceholders(scanned, lang));
  const sql = $derived(isSqlLike(lang));

  /** The Run on… sheet's list rule: newline-separated if multi-line, else
   *  comma-separated (mirrors database/multi-run.ts `parseValues`). */
  function parseListValue(text: string): string[] {
    const parts = text.includes('\n') ? text.split('\n') : text.split(',');
    return parts.map((v) => v.trim()).filter((v) => v.length > 0);
  }

  function set(name: string, value: string): void {
    onchange({ ...values, [name]: value });
  }
;
async () => {

 { svelteHTML.createElement("section", {     "class":`ph`,...__sveltets_2_empty({"data-testid":`wb-placeholders`}),"aria-label":`Placeholders`,});
  if(found.length === 0){
     { svelteHTML.createElement("p", { "class":`ph-empty`,});
         
      if(sql){ { svelteHTML.createElement("code", {});  } }
       { svelteHTML.createElement("code", {});'{name}'; }   { svelteHTML.createElement("code", {});'{{name}}'; }
     }
  }else{
     { svelteHTML.createElement("p", { "class":`ph-hint`,});
              if(sql){    { svelteHTML.createElement("code", {});  }         }else{   }
     }
     { svelteHTML.createElement("ul", { "class":`ph-list`,});
         for(let p of __sveltets_2_ensureArray(found)){p.name;
        const v = values[p.name] ?? '';
        const list = parseListValue(v);
         { svelteHTML.createElement("li", { "class":`ph-row`,});
           { svelteHTML.createElement("div", { "class":`ph-head`,});
             { svelteHTML.createElement("span", {   "class":`ph-name mono`,"title":p.name,});p.name; }
             { svelteHTML.createElement("span", {   "class":`ph-badge mono`,"title":`Placeholder syntax`,});p.syntax; }
            if(p.count > 1){ { svelteHTML.createElement("span", {   "class":`ph-count`,"title":`Occurrences`,}); p.count; }}
           }
           { svelteHTML.createElement("input", {                "dir":`ltr`,"class":`input ph-input mono`,"type":`text`,"value":v,"placeholder":sql ? 'value, or 1,2,3 to sweep' : 'value',"spellcheck":`false`,"aria-label":`Value for ${p.name}`,"oninput":(e) => set(p.name, (e.currentTarget as HTMLInputElement).value),});}
          if(list.length > 1){
             { svelteHTML.createElement("span", {   "class":`ph-sweep`,"title":`Database — Run on… runs once per value`,});
              list.length;      
             }
          }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PlaceholdersPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PlaceholdersPanel__SvelteComponent_ = ReturnType<typeof PlaceholdersPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default PlaceholdersPanel__SvelteComponent_;