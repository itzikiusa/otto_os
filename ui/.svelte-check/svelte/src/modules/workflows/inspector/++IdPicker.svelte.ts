///<reference types="svelte" />
;// A node param that holds another object's id (a DB connection, a broker
// cluster or topic, a product story, an issue account) as a picker over the
// workspace's objects — the person never types an id the UI doesn't show.
// A saved value that isn't in the list stays selectable ("Unknown … (id)") so
// opening the node never silently rewrites it. While the list loads the
// select is disabled ("Loading …"); only a FAILED load falls back to free
// text (with Retry), and an empty list says where to create one.

import { untrack } from 'svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import type { OptionCache } from './options.svelte';
import type { IdOption } from './util';
function $$render<T>() {

  
  
  
  
  
  
  
  
  
  
  

  interface Props {
    id: string;
    value: string;
    /** The page's list cache and the key it's fetched by (workspace, cluster…). */
    cache: OptionCache<T>;
    key: string;
    toOptions: (rows: T[] | null) => IdOption[] | null;
    /** "connection", "cluster", "story"… — for "Choose a …" / "Unknown … (id)". */
    noun: string;
    /** The free-text fallback's placeholder. */
    placeholder: string;
    /** First option's label; defaults to "Choose a {noun}…". */
    emptyLabel?: string;
    /** What the list is, for "Couldn’t load {what}" / "Loading {what}…". */
    what: string;
    /** Shown under the select when the loaded list is empty: where to create one. */
    emptyHint?: string;
    onchange: (value: string) => void;
    dir?: 'ltr' | 'auto';
  }
  let { id, value, cache, key, toOptions, noun, placeholder, emptyLabel, what, emptyHint, onchange, dir = 'auto' }: Props = $props();
  $effect(() => {
    const k = key;
    untrack(() => cache.ensure(k));
  });
  const options = $derived(toOptions(cache.rows(key)));
  const error = $derived(cache.error(key));
;
async () => {

if(options){
   { svelteHTML.createElement("select", {   id,value,"onchange":(e) => onchange(e.currentTarget.value),});
     { svelteHTML.createElement("option", {"value":"",});emptyLabel ?? `Choose a ${noun}…`; }
       for(let o of __sveltets_2_ensureArray(options)){o.id; { svelteHTML.createElement("option", {   "value":o.id,"title":o.title,});o.label; }}
    if(value && !options.some((o) => o.id === value)){ { svelteHTML.createElement("option", {value,}); noun; value;  }}
   }
  if(options.length === 0 && emptyHint){ { svelteHTML.createElement("p", { "class":`node-hint`,});emptyHint; }}
} else if (!error){
  
   { svelteHTML.createElement("select", {  id,"disabled":true,"aria-busy":`true`,});
     { svelteHTML.createElement("option", {"value":"",}); what;  }
   }
}else{
   { svelteHTML.createElement("input", {        dir,id,"type":`text`,placeholder,value,"oninput":(e) => onchange(e.currentTarget.value),});}
  if(error){ { const $$_etatSdaoL0C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL0C({ target: __sveltets_2_any(), props: {      what,"variant":`compact`,error,"empty":true,"onretry":() => cache.retry(key),}});}}
}
};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
class __sveltets_Render<T> {
    props(): ReturnType<typeof $$render<T>>['props'] { return null as any; }
    events(): ReturnType<typeof $$render<T>>['events'] { return null as any; }
    slots(): ReturnType<typeof $$render<T>>['slots'] { return null as any; }
    bindings() { return __sveltets_$$bindings(''); }
    exports() { return {}; }
}

interface $$IsomorphicComponent {
    new <T>(options: import('svelte').ComponentConstructorOptions<ReturnType<__sveltets_Render<T>['props']>>): import('svelte').SvelteComponent<ReturnType<__sveltets_Render<T>['props']>, ReturnType<__sveltets_Render<T>['events']>, ReturnType<__sveltets_Render<T>['slots']>> & { $$bindings?: ReturnType<__sveltets_Render<T>['bindings']> } & ReturnType<__sveltets_Render<T>['exports']>;
    <T>(internal: unknown, props: ReturnType<__sveltets_Render<T>['props']> & {}): ReturnType<__sveltets_Render<T>['exports']>;
    z_$$bindings?: ReturnType<__sveltets_Render<any>['bindings']>;
}
const IdPicker__SvelteComponent_: $$IsomorphicComponent = null as any;
/*Ωignore_startΩ*/type IdPicker__SvelteComponent_<T> = InstanceType<typeof IdPicker__SvelteComponent_<T>>;
/*Ωignore_endΩ*/export default IdPicker__SvelteComponent_;