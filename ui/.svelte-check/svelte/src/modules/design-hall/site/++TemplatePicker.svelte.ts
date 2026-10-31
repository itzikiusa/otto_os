///<reference types="svelte" />
;// The first thing a new (empty) site shows: six starter templates, each a
// LIVE miniature rendered with the site's brand theme, plus "Start blank".
// Picking one only edits the working copy — nothing is saved until Save.

import Icon from '../../../lib/components/Icon.svelte';
import { renderPageBody } from './engine/render';
import { SITE_TEMPLATES } from './engine/templates';
import { themeStyle, type Theme } from './engine/theme';
;

  interface Props {
    theme: Theme;
    title: string;
    readonly: boolean;
    onpick: (templateId: string | null) => void;
  };function $$render() {

  
  
  
  
  
  
  

  let { theme, title, readonly, onpick }: Props = $props();

  const style = $derived(themeStyle(theme));
  const previews = $derived(
    SITE_TEMPLATES.map((t) => {
      const d = t.build('');
      const page = { ...d.pages[0], sections: d.pages[0].sections.slice(0, 4) };
      return { t, html: renderPageBody(page, { theme, editable: false }) };
    }),
  );
;
async () => {

 { svelteHTML.createElement("div", {   "class":`picker`,...__sveltets_2_empty({"data-testid":`site-templates`}),});
   { svelteHTML.createElement("header", {});
     { svelteHTML.createElement("span", { "class":`studio-tile`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`layout`,"size":16,}});} }
     { svelteHTML.createElement("div", {});
       { svelteHTML.createElement("h2", {});   }
       { svelteHTML.createElement("p", {});                        }
     }
   }
   { svelteHTML.createElement("div", { "class":`grid`,});
       for(let p of __sveltets_2_ensureArray(previews)){p.t.id;
       { svelteHTML.createElement("button", {         "class":`tpl`,"disabled":readonly,"onclick":() => onpick(p.t.id),...__sveltets_2_empty({"data-testid":`site-template`}),...__sveltets_2_empty({"data-template":p.t.id}),});
         { svelteHTML.createElement("span", {   "class":`shot`,"aria-hidden":`true`,}); { svelteHTML.createElement("span", {  "class":`mini os-site os-still`,style,}); p.html; } }
         { svelteHTML.createElement("span", { "class":`meta`,}); { svelteHTML.createElement("strong", {});p.t.name; } { svelteHTML.createElement("span", {});p.t.description; } }
       }
    }
     { svelteHTML.createElement("button", {       "class":`tpl blank`,"disabled":readonly,"onclick":() => onpick(null),...__sveltets_2_empty({"data-testid":`site-template-blank`}),});
       { svelteHTML.createElement("span", {   "class":`shot empty`,"aria-hidden":`true`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":16,}});} }
       { svelteHTML.createElement("span", { "class":`meta`,}); { svelteHTML.createElement("strong", {});  } { svelteHTML.createElement("span", {});         } }
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const TemplatePicker__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type TemplatePicker__SvelteComponent_ = ReturnType<typeof TemplatePicker__SvelteComponent_>;
/*Ωignore_endΩ*/export default TemplatePicker__SvelteComponent_;