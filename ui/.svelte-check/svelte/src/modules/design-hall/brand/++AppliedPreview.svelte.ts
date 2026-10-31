///<reference types="svelte" />
;// "Applied to": three small renders of the kit — a landing hero (Site), a
// social tile (Graphics) and a 3D card swatch (3D) — that re-tint as tokens
// change. They are USER CONTENT previews (the kit's own colours and fonts),
// driven entirely by `--pv-*` properties set from the document; nothing here
// uses the app's accent. Decorative: the real artifacts are in "Used in".

import type { BrandDoc } from '../../../lib/api/types';
import StudioBadge from '../StudioBadge.svelte';
import { brandPalette, fontTokens, onColor, pxTokens, typeStyles } from './tokens';
;

  interface Props {
    doc: BrandDoc;
  };function $$render() {

  
  
  
  
  
  
  
  

  let { doc }: Props = $props();

  const pal = $derived(brandPalette(doc));
  const fonts = $derived(fontTokens(doc));
  const display = $derived(fonts.find((f) => f.name === 'display')?.stack ?? fonts[0]?.stack ?? 'system-ui, sans-serif');
  const body = $derived(fonts.find((f) => f.name === 'body')?.stack ?? display);
  const displayWeight = $derived(String(typeStyles(doc).find((t) => t.name === 'display')?.weight ?? 800));
  const radius = $derived.by(() => {
    const r = pxTokens(doc, 'radius');
    const pick = r.find((x) => ['card', 'md', 'lg'].includes(x.name)) ?? r.find((x) => x.px < 100) ?? r[0];
    return `${Math.min(pick?.px ?? 12, 28)}px`;
  });
  const onPrimary = $derived(onColor(pal.primary, pal.ink));
  const eyebrow = $derived((doc.name || 'Brand').replace(/\s*brand kit\s*$/i, '').trim() || 'Brand');
  const headline = $derived(firstLine(doc.voice?.do) ?? 'Every purchase moves you up.');
  const cta = $derived(shortCta(doc.voice?.do) ?? 'Join free');

  function firstLine(list: string[] | undefined): string | null {
    const s = list?.find((x) => x.trim().length > 12);
    return s ? s.trim().replace(/[.!]+$/, '') + '.' : null;
  }
  function shortCta(list: string[] | undefined): string | null {
    return list?.find((x) => x.trim().length > 0 && x.trim().length <= 14)?.trim() ?? null;
  }
;
async () => {

  { svelteHTML.createElement("div", {              "class":`applied`,...__sveltets_2_empty({"data-testid":`brand-applied`}),});__sveltets_2_ensureType(String, Number, pal.primary);__sveltets_2_ensureType(String, Number, pal.accent);__sveltets_2_ensureType(String, Number, pal.ink);__sveltets_2_ensureType(String, Number, pal.surface);__sveltets_2_ensureType(String, Number, pal.surfaceAlt);__sveltets_2_ensureType(String, Number, onPrimary);__sveltets_2_ensureType(String, Number, display);__sveltets_2_ensureType(String, Number, body);__sveltets_2_ensureType(String, Number, displayWeight);__sveltets_2_ensureType(String, Number, radius);
   { svelteHTML.createElement("div", { "class":`head`,});
     { svelteHTML.createElement("span", { "class":`section-title`,});  }
     { svelteHTML.createElement("span", { "class":`dim`,});  }
   }

   { svelteHTML.createElement("figure", { "class":`ap`,});
     { svelteHTML.createElement("div", {   "class":`hero`,"aria-hidden":`true`,});
       { svelteHTML.createElement("div", { "class":`copy`,});
         { svelteHTML.createElement("span", { "class":`eb`,});  eyebrow; }
         { svelteHTML.createElement("b", {});headline; }
         { svelteHTML.createElement("span", { "class":`cta`,});cta; }
       }
       { svelteHTML.createElement("div", { "class":`blob`,}); }
       { svelteHTML.createElement("div", { "class":`card3`,}); }
     }
     { svelteHTML.createElement("figcaption", {}); { const $$_egdaBoidutS3C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS3C({ target: __sveltets_2_any(), props: {  "studio":`site`,}});}   }
   }

   { svelteHTML.createElement("div", { "class":`row2`,});
     { svelteHTML.createElement("figure", { "class":`ap`,});
       { svelteHTML.createElement("div", {   "class":`social`,"aria-hidden":`true`,});
         { svelteHTML.createElement("span", { "class":`eb`,});eyebrow; }
         { svelteHTML.createElement("b", {});    }
         { svelteHTML.createElement("div", { "class":`blob`,}); }
         { svelteHTML.createElement("div", { "class":`card3`,}); }
       }
       { svelteHTML.createElement("figcaption", {}); { const $$_egdaBoidutS4C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS4C({ target: __sveltets_2_any(), props: {  "studio":`graphics`,}});}   }
     }
     { svelteHTML.createElement("figure", { "class":`ap`,});
       { svelteHTML.createElement("div", {   "class":`three`,"aria-hidden":`true`,});
         { svelteHTML.createElement("div", { "class":`card-wrap`,});
           { svelteHTML.createElement("div", { "class":`card`,});
             { svelteHTML.createElement("span", { "class":`name`,});eyebrow; }
             { svelteHTML.createElement("span", { "class":`chip3`,}); }
           }
         }
       }
       { svelteHTML.createElement("figcaption", {}); { const $$_egdaBoidutS4C = __sveltets_2_ensureComponent(StudioBadge); new $$_egdaBoidutS4C({ target: __sveltets_2_any(), props: {  "studio":`3d`,}});}    }
     }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const AppliedPreview__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type AppliedPreview__SvelteComponent_ = ReturnType<typeof AppliedPreview__SvelteComponent_>;
/*Ωignore_endΩ*/export default AppliedPreview__SvelteComponent_;