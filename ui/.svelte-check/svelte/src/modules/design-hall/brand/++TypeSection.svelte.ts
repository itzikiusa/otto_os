///<reference types="svelte" />
;// Brand Kit → Typography: the three font roles (display / body / mono, each a
// local family stack — no web-font CDN) and the type scale (size / line
// height / weight per style), each rendered as a live specimen in the kit's
// own font (user content).

import type { BrandDoc, BrandFontRole } from '../../../lib/api/types';
import Icon from '../../../lib/components/Icon.svelte';
import { askTokenName, tokenMenu } from './edit';
import { FONT_ROLES, fontRoleForStyle, renameKey, tokenLabel, uniqueName } from './tokens';
;

  interface Props {
    doc: BrandDoc;
    readonly?: boolean;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { doc = $bindable(), readonly = false }: Props = $props()/*Ωignore_startΩ*/;doc;/*Ωignore_endΩ*/;

  const ROLE_LABEL: Record<BrandFontRole, string> = { display: 'Display', body: 'Body', mono: 'Mono' };
  const styles = $derived(Object.entries(doc.type ?? {}));

  function stackFor(style: string): string {
    const role = fontRoleForStyle(style);
    return doc.font?.[role]?.$value || doc.font?.display?.$value || doc.font?.body?.$value || 'system-ui, sans-serif';
  }

  function sample(style: string): string {
    const role = fontRoleForStyle(style);
    if (role === 'mono') return 'const points = order.total * 2;';
    if (role === 'display') {
      const v = doc.voice?.do?.find((x) => x.trim().length > 12);
      return v ? v.trim() : 'Make something people remember.';
    }
    return 'Every token here reaches every studio — sites, graphics, frames and 3D read the same names.';
  }

  function setFont(role: BrandFontRole, value: string): void {
    const cur = doc.font[role];
    if (cur) cur.$value = value;
    else doc.font[role] = { $value: value };
  }

  function setWeights(role: BrandFontRole, text: string): void {
    const cur = doc.font[role];
    if (!cur) return;
    const ws = text
      .split(/[\s,]+/)
      .map((x) => Number(x))
      .filter((n) => Number.isInteger(n) && n >= 1 && n <= 1000);
    if (ws.length) cur.weights = ws.slice(0, 9);
    else delete cur.weights;
  }

  function removeFont(role: BrandFontRole): void {
    const next = { ...doc.font };
    delete next[role];
    doc.font = next;
  }

  function num(e: Event): number {
    return Number((e.currentTarget as HTMLInputElement).value);
  }

  function setStyle(name: string, key: 'size' | 'line' | 'weight', value: number): void {
    const s = doc.type?.[name];
    if (s && Number.isFinite(value)) s[key] = value;
  }

  async function addStyle(): Promise<void> {
    const taken = Object.keys(doc.type ?? {});
    const name = await askTokenName('Add text style', taken, { placeholder: uniqueName('h3', taken), confirmLabel: 'Add' });
    if (!name) return;
    doc.type = { ...(doc.type ?? {}), [name]: { size: 24, line: 32, weight: 600 } };
  }

  async function renameStyle(from: string): Promise<void> {
    const to = await askTokenName('Rename text style', Object.keys(doc.type ?? {}), { initial: from, confirmLabel: 'Rename' });
    if (to && doc.type) doc.type = renameKey(doc.type, from, to);
  }

  function removeStyle(name: string): void {
    const next = { ...(doc.type ?? {}) };
    delete next[name];
    doc.type = next;
  }
;
async () => {

 { svelteHTML.createElement("section", {     "id":`brand-type`,"class":`bsec`,"aria-labelledby":`brand-type-h`,});
   { svelteHTML.createElement("div", { "class":`sec-h`,});
     { svelteHTML.createElement("h2", { "id":`brand-type-h`,});  }
     { svelteHTML.createElement("span", { "class":`meta`,});       }
   }

   { svelteHTML.createElement("div", { "class":`card fonts`,});
       for(let role of __sveltets_2_ensureArray(FONT_ROLES)){role;
      const f = doc.font?.[role];
       { svelteHTML.createElement("div", {     "class":`frow`,...__sveltets_2_empty({"data-testid":`brand-font`}),...__sveltets_2_empty({"data-role":role}),});
         { svelteHTML.createElement("span", {    "class":`aa`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, f?.$value || 'inherit');  }
         { svelteHTML.createElement("div", { "class":`fmeta`,});
           { svelteHTML.createElement("b", {});ROLE_LABEL[role]; }
           { svelteHTML.createElement("span", { "class":`mono dim`,}); role; }
         }
        if(f){
           { svelteHTML.createElement("input", {              "dir":`ltr`,"class":`input stack mono`,"value":f.$value,"oninput":(e) => setFont(role, e.currentTarget.value),"disabled":readonly,"aria-label":`${ROLE_LABEL[role]} font stack`,"spellcheck":`false`,});}
           { svelteHTML.createElement("input", {                "dir":`ltr`,"class":`input weights mono`,"value":(f.weights ?? []).join(', '),"onchange":(e) => setWeights(role, e.currentTarget.value),"disabled":readonly,"placeholder":`400, 700`,"aria-label":`${ROLE_LABEL[role]} weights`,"title":`Weights this family ships (comma-separated)`,});}
          if(!readonly){
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`Remove the ${ROLE_LABEL[role].toLowerCase()} font`,"title":`Remove`,"onclick":() => removeFont(role),});
               { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`x`,"size":14,}});}
             }
          }
        } else if (!readonly){
           { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => setFont(role, role === 'mono' ? 'ui-monospace, Menlo, monospace' : 'system-ui, sans-serif'),});
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  ROLE_LABEL[role].toLowerCase(); 
           }
        }else{
           { svelteHTML.createElement("span", { "class":`dim`,});  }
        }
       }
    }
   }

   { svelteHTML.createElement("div", {   "class":`card tspec`,...__sveltets_2_empty({"data-testid":`brand-type-scale`}),});
       for(let [name, s] of __sveltets_2_ensureArray(styles)){name;
       { svelteHTML.createElement("div", {     "class":`trow`,...__sveltets_2_empty({"data-testid":`brand-type-style`}),...__sveltets_2_empty({"data-token":name}),});
         { svelteHTML.createElement("div", { "class":`tmeta`,});
           { svelteHTML.createElement("div", { "class":`tname`,});
             { svelteHTML.createElement("b", {});tokenLabel(name); }
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`More for ${tokenLabel(name)}`,"title":`More`,"onclick":(e) => tokenMenu(e, 'type', name, { rename: () => void renameStyle(name), remove: () => removeStyle(name) }, readonly),});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
             }
           }
           { svelteHTML.createElement("span", { "class":`mono dim`,});s.size; s.line;  s.weight; }
          if(!readonly){
             { svelteHTML.createElement("div", { "class":`nums`,});
               { svelteHTML.createElement("label", {});  { svelteHTML.createElement("input", {            "class":`input num`,"type":`number`,"min":`1`,"max":`400`,"value":s.size,"oninput":(e) => setStyle(name, 'size', num(e)),});} }
               { svelteHTML.createElement("label", {});  { svelteHTML.createElement("input", {            "class":`input num`,"type":`number`,"min":`1`,"max":`600`,"value":s.line,"oninput":(e) => setStyle(name, 'line', num(e)),});} }
               { svelteHTML.createElement("label", {});  { svelteHTML.createElement("input", {              "class":`input num`,"type":`number`,"min":`100`,"max":`1000`,"step":`100`,"value":s.weight,"oninput":(e) => setStyle(name, 'weight', num(e)),});} }
             }
          }
         }
         { svelteHTML.createElement("p", {       "class":`specimen`,});__sveltets_2_ensureType(String, Number, stackFor(name));__sveltets_2_ensureType(String, Number, `${Math.min(Math.max(s.size, 8), 72)}px`);__sveltets_2_ensureType(String, Number, `${Math.min(Math.max(s.line, 8), 84)}px`);__sveltets_2_ensureType(String, Number, String(s.weight));
          sample(name);
         }
       }
    }
       { svelteHTML.createElement("p", { "class":`empty`,});                  }
    
    if(!readonly){
       { svelteHTML.createElement("div", { "class":`foot`,});
         { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":addStyle,...__sveltets_2_empty({"data-testid":`brand-type-add`}),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}    }
       }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('doc'), slots: {}, events: {} }}
const TypeSection__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TypeSection__SvelteComponent_ = ReturnType<typeof TypeSection__SvelteComponent_>;
/*Ωignore_endΩ*/export default TypeSection__SvelteComponent_;