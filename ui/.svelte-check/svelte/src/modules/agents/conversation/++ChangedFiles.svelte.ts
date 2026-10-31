///<reference types="svelte" />
;// "2 changed files +29 −12 · Show files · Open diff" under a settled
// response (t3code's changed-files card): what the response did to the
// tree, from its edit / write calls. A file row opens that file's diff in
// the side panel; renderable files (md, html, svg, json, csv, images) also
// get a Preview.

import { getContext } from 'svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { changedDiff, relPath, type ChangedFile } from './format';
import { previewKind } from './preview';
import { CONV_CTX, type ConvContext } from './context';
;

  interface Props {
    files: ChangedFile[];
  };function $$render() {

  
  
  
  
  
  
  
  
  
  

  let { files }: Props = $props();
  const ctx = getContext<ConvContext>(CONV_CTX);
  const cwd = $derived(ctx.cwd ?? null);
  let open = $state(false);
  const add = $derived(files.reduce((n, f) => n + f.add, 0));
  const del = $derived(files.reduce((n, f) => n + f.del, 0));

  function split(p: string): { dir: string; base: string } {
    const r = relPath(p, cwd);
    const at = r.lastIndexOf('/');
    return at < 0 ? { dir: '', base: r } : { dir: r.slice(0, at + 1), base: r.slice(at + 1) };
  }
  function openDiff(focus: string | null = null): void {
    const n = files.length;
    ctx?.openPreview?.({ kind: 'diff', title: `${n} changed ${n === 1 ? 'file' : 'files'}`, diff: changedDiff(files), focus });
  }
  function preview(f: ChangedFile): void {
    ctx?.openPreview?.({ kind: 'file', path: f.path, content: f.content, diff: changedDiff([f]) });
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`cf`,...__sveltets_2_empty({"data-changed-files":files.length}),});open;
   { svelteHTML.createElement("div", { "class":`cf-head`,});
     { svelteHTML.createElement("button", {       "class":`cf-toggle`,"onclick":() => (open = !open),"aria-expanded":open,"title":open ? 'Hide the files' : 'Show the files',});
       { svelteHTML.createElement("span", {   "class":`cf-caret`,"aria-hidden":`true`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});} }
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":12,}});}
       { svelteHTML.createElement("span", { "class":`cf-title`,});files.length;  files.length === 1 ? 'file' : 'files'; }
       { svelteHTML.createElement("span", { "class":`cf-stat mono`,}); { svelteHTML.createElement("span", { "class":`add`,}); add; }  { svelteHTML.createElement("span", { "class":`del`,}); del; } }
       { svelteHTML.createElement("span", { "class":`cf-show`,});open ? 'Hide files' : 'Show files'; }
     }
     { svelteHTML.createElement("button", {     "class":`btn small cf-diff`,"onclick":() => openDiff(),"title":`Open this response’s changes in the side panel`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`split`,"size":12,}});}   }
   }
  if(open){
     { svelteHTML.createElement("ul", { "class":`cf-list`,});
         for(let f of __sveltets_2_ensureArray(files)){f.path;
        const p = split(f.path);
         { svelteHTML.createElement("li", { "class":`cf-row`,});
           { svelteHTML.createElement("button", {     "class":`cf-file`,"onclick":() => openDiff(f.path),"title":`Diff of ${f.path}`,});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});}
             { svelteHTML.createElement("span", { "class":`cf-path mono`,}); { svelteHTML.createElement("span", { "class":`cf-dir`,});p.dir; }p.base; }
            if(f.created){ { svelteHTML.createElement("span", { "class":`cf-new`,});  }}
             { svelteHTML.createElement("span", { "class":`cf-stat mono`,}); { svelteHTML.createElement("span", { "class":`add`,}); f.add; }  { svelteHTML.createElement("span", { "class":`del`,}); f.del; } }
           }
          if(previewKind(f.path) !== 'code'){
             { svelteHTML.createElement("button", {       "class":`icon-btn cf-prev`,"onclick":() => preview(f),"aria-label":`Preview ${p.base}`,"title":`Preview ${p.base}`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`eye`,"size":12,}});} }
          }
         }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const ChangedFiles__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ChangedFiles__SvelteComponent_ = ReturnType<typeof ChangedFiles__SvelteComponent_>;
/*Ωignore_endΩ*/export default ChangedFiles__SvelteComponent_;