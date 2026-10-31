///<reference types="svelte" />
;// Brand Kit → Spacing & radius: the space scale (drawn to size, capped for
// the page) and the corner radii, each with an inline px value.

import type { BrandDoc } from '../../../lib/api/types';
import Icon from '../../../lib/components/Icon.svelte';
import { askDescription, askTokenName, tokenMenu } from './edit';
import { renameKey, uniqueName } from './tokens';
;

  interface Props {
    doc: BrandDoc;
    readonly?: boolean;
  };;

  type Group = 'space' | 'radius';;function $$render() {

  
  
  
  
  
  

  let { doc = $bindable(), readonly = false }: Props = $props()/*Ωignore_startΩ*/;doc;/*Ωignore_endΩ*/;

  const space = $derived(Object.entries(doc.space ?? {}));
  const radius = $derived(Object.entries(doc.radius ?? {}));

  function set(group: Group, name: string, e: Event): void {
    const v = Number((e.currentTarget as HTMLInputElement).value);
    const tok = doc[group][name];
    if (tok && Number.isFinite(v)) tok.$value = v;
  }

  async function add(group: Group): Promise<void> {
    const taken = Object.keys(doc[group]);
    const name = await askTokenName(group === 'space' ? 'Add space step' : 'Add radius', taken, {
      placeholder: uniqueName(group === 'space' ? 'md' : 'card', taken),
      confirmLabel: 'Add',
    });
    if (name) doc[group] = { ...doc[group], [name]: { $value: group === 'space' ? 16 : 12 } };
  }

  async function rename(group: Group, from: string): Promise<void> {
    const to = await askTokenName('Rename token', Object.keys(doc[group]), { initial: from, confirmLabel: 'Rename' });
    if (to) doc[group] = renameKey(doc[group], from, to);
  }

  async function describe(group: Group, name: string): Promise<void> {
    const d = await askDescription(name, doc[group][name]?.$description);
    if (d && doc[group][name]) doc[group][name].$description = d;
  }

  function remove(group: Group, name: string): void {
    const next = { ...doc[group] };
    delete next[name];
    doc[group] = next;
  }

  function menu(e: MouseEvent, group: Group, name: string): void {
    tokenMenu(e, group, name, { rename: () => void rename(group, name), describe: () => void describe(group, name), remove: () => remove(group, name) }, readonly);
  }
;
async () => {

 { svelteHTML.createElement("section", {     "id":`brand-space`,"class":`bsec`,"aria-labelledby":`brand-space-h`,});
   { svelteHTML.createElement("div", { "class":`sec-h`,});
     { svelteHTML.createElement("h2", { "id":`brand-space-h`,});   }
     { svelteHTML.createElement("span", { "class":`meta`,});        }
   }

   { svelteHTML.createElement("div", { "class":`card`,});
     { svelteHTML.createElement("h3", { "class":`section-title`,});  }
     { svelteHTML.createElement("div", {   "class":`scale`,...__sveltets_2_empty({"data-testid":`brand-space`}),});
         for(let [name, tok] of __sveltets_2_ensureArray(space)){name;
         { svelteHTML.createElement("div", { "class":`step`,});
           { svelteHTML.createElement("i", {     "class":`box`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, `${Math.min(Math.max(tok.$value, 2), 64)}px`);__sveltets_2_ensureType(String, Number, `${Math.min(Math.max(tok.$value, 2), 64)}px`); }
           { svelteHTML.createElement("div", { "class":`lbl`,});
             { svelteHTML.createElement("span", { "class":`mono nm`,});name; }
             { svelteHTML.createElement("button", {       "class":`icon-btn tiny`,"aria-label":`More for space ${name}`,"title":`More`,"onclick":(e) => menu(e, 'space', name),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":12,}});} }
           }
           { svelteHTML.createElement("input", {                "class":`input num mono`,"type":`number`,"min":`0`,"max":`10000`,"value":tok.$value,"oninput":(e) => set('space', name, e),"disabled":readonly,"aria-label":`Space ${name} in px`,});}
         }
      }
         { svelteHTML.createElement("p", { "class":`empty`,});    }
      
      if(!readonly){
         { svelteHTML.createElement("button", {   "class":`add`,"onclick":() => add('space'),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
      }
     }

     { svelteHTML.createElement("h3", { "class":`section-title`,});  }
     { svelteHTML.createElement("div", {   "class":`radii`,...__sveltets_2_empty({"data-testid":`brand-radius`}),});
         for(let [name, tok] of __sveltets_2_ensureArray(radius)){name;
         { svelteHTML.createElement("div", { "class":`rad`,});
           { svelteHTML.createElement("i", {    "class":`tile`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, `${Math.min(Math.max(tok.$value, 0), 32)}px`); }
           { svelteHTML.createElement("div", { "class":`lbl`,});
             { svelteHTML.createElement("span", { "class":`mono nm`,}); name; }
             { svelteHTML.createElement("button", {       "class":`icon-btn tiny`,"aria-label":`More for radius ${name}`,"title":`More`,"onclick":(e) => menu(e, 'radius', name),}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":12,}});} }
           }
           { svelteHTML.createElement("input", {                "class":`input num mono`,"type":`number`,"min":`0`,"max":`10000`,"value":tok.$value,"oninput":(e) => set('radius', name, e),"disabled":readonly,"aria-label":`Radius ${name} in px`,});}
         }
      }
         { svelteHTML.createElement("p", { "class":`empty`,});   }
      
      if(!readonly){
         { svelteHTML.createElement("button", {   "class":`add`,"onclick":() => add('radius'),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
      }
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('doc'), slots: {}, events: {} }}
const SpaceSection__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SpaceSection__SvelteComponent_ = ReturnType<typeof SpaceSection__SvelteComponent_>;
/*Ωignore_endΩ*/export default SpaceSection__SvelteComponent_;