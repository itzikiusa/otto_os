///<reference types="svelte" />
;
import { rowMenu } from '../../lib/rowMenu';
// Collapsible JSON tree for the response viewer. Objects/arrays fold; the
// root and, within a row budget, its child containers start open
// (`autoOpenPaths` — a wide root array no longer mounts every row). A search query highlights matching keys and
// values and opens their ancestors. Right-click (or the row's menu key) a
// node to copy its JSONPath or value. Large containers render in pages.
import Icon from '../../lib/components/Icon.svelte';
import { ctxMenu } from '../../lib/contextmenu.svelte';
import { copyTextOrThrow } from '../../lib/clipboard';
import { toasts } from '../../lib/toast.svelte';
import { autoOpenPaths, childPath, preview, searchTree } from '../../lib/api/jsonTree';
;

  interface Props {
    value: unknown;
    query?: string;
  };function $$render() {
 const node/*Ωignore_positionΩ*/ = (v: unknown, path: string, key: string | number | null, depth: number)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const container = v !== null && typeof v === 'object';
  const open = container && isOpen(path, depth);
   { svelteHTML.createElement("div", {   "class":`row`,});found.hits.has(path);__sveltets_2_ensureType(String, Number, `${depth * 16} `);
    if(container){
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {          "class":`tog`,"onclick":() => toggle(path, depth),"oncontextmenu":(e) => menu(e, path, v),"aria-expanded":open,"title":path,});
         { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":open ? 'chevronDown' : 'chevronRight',"size":12,}});}
        if(key !== null){ { svelteHTML.createElement("span", { "class":`k`,});typeof key === 'number' ? key : `"${key}"`; } { svelteHTML.createElement("span", { "class":`p`,});  }}
         { svelteHTML.createElement("span", { "class":`p`,});Array.isArray(v) ? '[' : '{'; }
        if(!open){ { svelteHTML.createElement("span", { "class":`pv`,});preview(v); } { svelteHTML.createElement("span", { "class":`p`,});Array.isArray(v) ? ']' : '}'; }}
       }}
    }else{
       {const $$action_0 = __sveltets_2_ensureAction(rowMenu(svelteHTML.mapElementTag('button')));{ svelteHTML.createElement("button", __sveltets_2_union($$action_0), {        "class":`leaf`,"oncontextmenu":(e) => menu(e, path, v),"onkeydown":(e) => { if (e.key === 'ContextMenu' || (e.shiftKey && e.key === 'F10')) menu(e, path, v); },"title":path,});
        if(key !== null){ { svelteHTML.createElement("span", { "class":`k`,});typeof key === 'number' ? key : `"${key}"`; } { svelteHTML.createElement("span", { "class":`p`,});  }}
         { svelteHTML.createElement("span", { "class":`v ${kind(v)}`,});typeof v === 'string' ? JSON.stringify(v) : String(v); }
       }}
    }
   }
  if(open){
    const all = entries(v);
    const limit = shown[path] ?? PAGE;
       for(let [k, child] of __sveltets_2_ensureArray((all.slice(0, limit)))){k;
      ;__sveltets_2_ensureSnippet(node(child, childPath(path, k), k, depth + 1));
    }
    if(all.length > limit){
       { svelteHTML.createElement("div", {  "class":`row`,});__sveltets_2_ensureType(String, Number, `${(depth + 1) * 16} `);
         { svelteHTML.createElement("button", {   "class":`more`,"onclick":() => (shown[path] = limit + PAGE),}); Math.min(PAGE, all.length - limit);   all.length - limit; }
       }
    }
     { svelteHTML.createElement("div", {  "class":`row close`,});__sveltets_2_ensureType(String, Number, `${depth * 16 + 14} `); { svelteHTML.createElement("span", { "class":`p`,});Array.isArray(v) ? ']' : '}'; } }
  }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  

  let { value, query = '' }: Props = $props();

  const PAGE = 200;
  const found = $derived(searchTree(value, query));
  const autoOpen = $derived(autoOpenPaths(value, PAGE));
  let toggled = $state<Record<string, boolean>>({});
  let shown = $state<Record<string, number>>({});

  function isOpen(path: string, depth: number): boolean {
    if (path in toggled) return toggled[path];
    if (query.trim()) return found.open.has(path);
    return depth < 2 && autoOpen.has(path);
  }
  function toggle(path: string, depth: number): void {
    toggled[path] = !isOpen(path, depth);
  }
  function entries(v: unknown): [string | number, unknown][] {
    return Array.isArray(v) ? v.map((x, i) => [i, x]) : Object.entries(v as Record<string, unknown>);
  }
  function kind(v: unknown): string {
    return v === null ? 'null' : Array.isArray(v) ? 'array' : typeof v;
  }
  async function copy(text: string, what: string): Promise<void> {
    try {
      await copyTextOrThrow(text);
      toasts.success(`Copied ${what}`);
    } catch {
      toasts.error('Couldn’t copy', 'The clipboard isn’t available here. Select the text and copy it manually.');
    }
  }
  function menu(e: MouseEvent | KeyboardEvent, path: string, v: unknown): void {
    ctxMenu.show(e, [
      { label: 'Copy JSONPath', icon: 'link', action: () => void copy(path, 'JSONPath') },
      { label: 'Copy value', icon: 'copy', action: () => void copy(typeof v === 'string' ? v : JSON.stringify(v, null, 2), 'value') },
    ]);
  }
;
async () => {

 { svelteHTML.createElement("div", {   "class":`jtree mono`,"dir":`ltr`,});
  ;__sveltets_2_ensureSnippet(node(value, '$', null, 0));
 }




};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const JsonTree__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type JsonTree__SvelteComponent_ = ReturnType<typeof JsonTree__SvelteComponent_>;
/*Ωignore_endΩ*/export default JsonTree__SvelteComponent_;