///<reference types="svelte" />
;// Brand Kit → Colors: one card per colour token with a colour input, the hex,
// its CSS variable and live WCAG contrast badges (vs white and the kit's ink),
// plus "Try a primary" presets and "Rule from team" chips from learned rules.
// The swatches are the kit's own colours (user content), set inline.

import type { BrandDoc, DesignLearnedRule } from '../../../lib/api/types';
import Icon from '../../../lib/components/Icon.svelte';
import { router } from '../../../lib/router.svelte';
import { askDescription, askTokenName, tokenMenu } from './edit';
import { contrast, contrastLevel, cssVarName, inkOf, normalizeHex, tokenLabel, uniqueName, renameKey } from './tokens';
;

  interface Props {
    doc: BrandDoc;
    readonly?: boolean;
    rules?: DesignLearnedRule[];
  };function $$render() {

  
  
  
  
  
  
  
  
  

  let { doc = $bindable(), readonly = false, rules = [] }: Props = $props()/*Ωignore_startΩ*/;doc;/*Ωignore_endΩ*/;

  const PRESETS: [string, string][] = [
    ['#5B3DF5', 'Violet'],
    ['#2F6FED', 'Signal blue'],
    ['#0F9D8A', 'Teal'],
    ['#E0457B', 'Berry'],
    ['#1D1D1F', 'Graphite'],
  ];

  const ink = $derived(inkOf(doc));
  const entries = $derived(Object.entries(doc.color ?? {}));
  const primaryName = $derived(doc.color?.primary ? 'primary' : (entries[0]?.[0] ?? null));

  function setColor(name: string, value: string): void {
    const tok = doc.color[name];
    if (tok) tok.$value = value;
  }

  function onPick(name: string, e: Event): void {
    setColor(name, (e.currentTarget as HTMLInputElement).value.toUpperCase());
  }

  function badge(fg: string | null, bg: string, label: string): { text: string; tone: string; title: string } {
    const r = fg ? contrast(fg, bg) : null;
    if (r == null) return { text: `${label} —`, tone: 'fail', title: 'Not a hex color yet' };
    const lvl = contrastLevel(r);
    const tone = lvl === 'AAA' || lvl === 'AA' ? 'pass' : lvl === 'AA-large' ? 'mid' : 'fail';
    const sub =
      lvl === 'fail' ? 'Below 3:1 — not for text or icons' : lvl === 'AA-large' ? 'Only for text 24 px and larger, and icons' : 'Passes WCAG AA for body text';
    return { text: `${label} ${r.toFixed(1)}`, tone, title: `${label}: ${r.toFixed(2)}:1. ${sub}` };
  }

  async function add(): Promise<void> {
    const name = await askTokenName('Add color', Object.keys(doc.color), { placeholder: uniqueName('brand', Object.keys(doc.color)), confirmLabel: 'Add' });
    if (!name) return;
    doc.color[name] = { $value: '#888888' };
  }

  async function rename(from: string): Promise<void> {
    const to = await askTokenName('Rename color', Object.keys(doc.color), { initial: from, confirmLabel: 'Rename' });
    if (to) doc.color = renameKey(doc.color, from, to);
  }

  async function describe(name: string): Promise<void> {
    const d = await askDescription(name, doc.color[name]?.$description);
    if (d == null || !doc.color[name]) return;
    if (d) doc.color[name].$description = d;
    else delete doc.color[name].$description;
  }

  function remove(name: string): void {
    const next = { ...doc.color };
    delete next[name];
    doc.color = next;
  }

  function preset(hex: string): void {
    if (!primaryName) {
      doc.color.primary = { $value: hex };
      return;
    }
    setColor(primaryName, hex);
  }
;
async () => {

 { svelteHTML.createElement("section", {     "id":`brand-colors`,"class":`bsec`,"aria-labelledby":`brand-colors-h`,});
   { svelteHTML.createElement("div", { "class":`sec-h`,});
     { svelteHTML.createElement("h2", { "id":`brand-colors-h`,});  }
     { svelteHTML.createElement("span", { "class":`meta`,});      ink.name ? tokenLabel(ink.name) : 'black'; }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
       for(let r of __sveltets_2_ensureArray((rules.slice(0, 2)))){r.key;
       { svelteHTML.createElement("button", {       "class":`chip rule`,"title":`An approved team rule — open it in What Otto learned`,"onclick":() => router.go('design/learned/rules'),...__sveltets_2_empty({"data-testid":`brand-rule-chip`}),});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`bulb`,"size":12,}});}    r.rule;
       }
    }
    if(rules.length > 2){ { svelteHTML.createElement("span", { "class":`chip`,}); rules.length - 2;  }}
   }

   { svelteHTML.createElement("div", {   "class":`grid`,...__sveltets_2_empty({"data-testid":`brand-colors`}),});
       for(let [name, tok] of __sveltets_2_ensureArray(entries)){name;
      const hex = normalizeHex(tok?.$value ?? '');
      const w = badge(hex, '#FFFFFF', 'on white');
      const k = badge(hex, ink.hex, `on ${ink.name ? tokenLabel(ink.name).toLowerCase() : 'black'}`);
       { svelteHTML.createElement("div", {     "class":`swc`,...__sveltets_2_empty({"data-testid":`brand-color`}),...__sveltets_2_empty({"data-token":name}),});
         { svelteHTML.createElement("div", {   "class":`swc-c`,});__sveltets_2_ensureType(String, Number, hex ?? 'transparent');!hex;
          if(!readonly){
             { svelteHTML.createElement("label", {   "class":`pick`,"title":`Edit ${tokenLabel(name)}`,});
               { svelteHTML.createElement("input", {          "type":`color`,"value":(hex ?? '#888888').slice(0, 7).toLowerCase(),"oninput":(e) => onPick(name, e),"aria-label":`Edit ${tokenLabel(name)} color`,...__sveltets_2_empty({"data-testid":`brand-color-input`}),});}
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`edit`,"size":13,}});}
             }
          }
         }
         { svelteHTML.createElement("div", { "class":`swc-b`,});
           { svelteHTML.createElement("div", { "class":`row`,});
             { svelteHTML.createElement("b", { "class":`nm`,});tokenLabel(name); }
             { svelteHTML.createElement("input", {                 "dir":`ltr`,"class":`hex mono`,"value":tok?.$value ?? '',"oninput":(e) => setColor(name, e.currentTarget.value.trim()),"disabled":readonly,"aria-label":`${tokenLabel(name)} hex value`,"spellcheck":`false`,"maxlength":9,});!hex;}
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"aria-label":`More for ${tokenLabel(name)}`,"title":`More`,"onclick":(e) => tokenMenu(e, 'color', name, { rename: () => void rename(name), describe: () => void describe(name), remove: () => remove(name) }, readonly),});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":14,}});}
             }
           }
           { svelteHTML.createElement("span", { "class":`mono var`,});cssVarName('color', name); }
          if(tok?.$description){ { svelteHTML.createElement("span", { "class":`desc`,});tok.$description; }}
           { svelteHTML.createElement("div", { "class":`cbs`,});
             { svelteHTML.createElement("span", {   "class":`cb ${w.tone}`,"title":w.title,});w.text; }
            if(!(ink.name === name)){ { svelteHTML.createElement("span", {   "class":`cb ${k.tone}`,"title":k.title,});k.text; }}
           }
         }
       }
    }
   }

  if(!readonly){
     { svelteHTML.createElement("div", { "class":`presets`,});
       { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":add,...__sveltets_2_empty({"data-testid":`brand-color-add`}),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
      if(primaryName){
         { svelteHTML.createElement("span", {   "class":`sep`,"aria-hidden":`true`,}); }
         { svelteHTML.createElement("span", { "class":`dim`,});  tokenLabel(primaryName).toLowerCase(); }
           for(let [h, n] of __sveltets_2_ensureArray(PRESETS)){h;
           { svelteHTML.createElement("button", {        "class":`pchip`,"onclick":() => preset(h),"aria-label":`Set ${primaryName} to ${n}`,"title":`Set ${primaryName} to ${n}`,});__sveltets_2_ensureType(String, Number, h); }
        }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('doc'), slots: {}, events: {} }}
const ColorsSection__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type ColorsSection__SvelteComponent_ = ReturnType<typeof ColorsSection__SvelteComponent_>;
/*Ωignore_endΩ*/export default ColorsSection__SvelteComponent_;