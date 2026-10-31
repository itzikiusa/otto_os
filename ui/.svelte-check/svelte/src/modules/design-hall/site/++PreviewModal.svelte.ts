///<reference types="svelte" />
;// Full-size preview of the site without editor chrome: the working copy
// rendered as the standalone document the export ships (or, for a local
// publish, the HTML the daemon served). It runs in a frame with
// `sandbox="allow-same-origin"` — never scripts — so the page can't run code,
// while the studio can follow the site's own page links (tiers.html → Tiers).

import Modal from '../../../lib/components/Modal.svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { documentCss, pageFileName, renderDocument, themeCss, type EmbedInfo } from './engine/render';
import type { Theme } from './engine/theme';
import type { SiteDoc } from './engine/types';
import SITE_CSS from './engine/site.css?raw';
import { DEVICE_WIDTH, type SiteDevice } from './SiteCanvas.svelte';
;

  interface Props {
    doc: SiteDoc;
    theme: Theme;
    pageId: string;
    embed: (uri: string) => EmbedInfo | null;
    asset: (uri: string) => string | null;
    /** Server-rendered HTML per page id (a local publish) — shown instead of the working copy. */
    served?: Record<string, string> | null;
    servedUrl?: string | null;
    onclose: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  

  let { doc, theme, pageId: initialPage, embed, asset, served = null, servedUrl = null, onclose }: Props = $props();

  let pageId = $state('');
  $effect.pre(() => {
    if (!pageId) pageId = initialPage || doc.pages[0]?.id || '';
  });
  let device = $state<SiteDevice | 'fill'>('fill');
  const page = $derived(doc.pages.find((p) => p.id === pageId) ?? doc.pages[0]);
  const css = $derived(themeCss(theme) + documentCss(theme) + SITE_CSS);
  const srcdoc = $derived.by(() => {
    if (!page) return '';
    if (served) return served[page.id] ?? '';
    return renderDocument(
      doc,
      page,
      {
        theme,
        embed,
        asset,
        homeHref: doc.pages[0] ? pageFileName(doc, doc.pages[0]) : '#',
        pageHref: (id) => {
          const p = doc.pages.find((x) => x.id === id);
          return p ? pageFileName(doc, p) : '#';
        },
      },
      css,
    );
  });

  let frame = $state<HTMLIFrameElement | null>(null);
  let winH = $state(800);
  $effect(() => {
    const onResize = () => (winH = window.innerHeight);
    onResize();
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });

  /** Follow the site's own page links; anything else stays put (no navigation inside the preview). */
  function wire(): void {
    const d = frame?.contentDocument;
    if (!d) return;
    d.addEventListener('click', (e) => {
      const a = (e.target as Element | null)?.closest?.('a[href]') as HTMLAnchorElement | null;
      if (!a) return;
      const href = a.getAttribute('href') ?? '';
      if (href.startsWith('#')) return;
      e.preventDefault();
      const file = href.split(/[?#]/)[0];
      let target = doc.pages.find((p) => pageFileName(doc, p) === file);
      if (!target && served) {
        // Daemon-rendered pages link to …/preview[/<slug or id>]?publish=…
        const m = /\/preview(?:\/([^/]+))?$/.exec(file);
        if (m) target = m[1] ? doc.pages.find((p, i) => i > 0 && (p.slug === m[1] || p.id === m[1])) : doc.pages[0];
      }
      if (target) pageId = target.id;
    });
  }
  const width = $derived(device === 'fill' ? '100%' : `${DEVICE_WIDTH[device]}px`);
;
async () => {

 { const $$_ladoM0C = __sveltets_2_ensureComponent(Modal); new $$_ladoM0C({ target: __sveltets_2_any(), props: {    children:() => { return __sveltets_2_any(0); },"title":`Preview`,"width":1480,onclose,}});
   { svelteHTML.createElement("div", { "class":`bar`,});
    if(doc.pages.length > 1){
       { svelteHTML.createElement("select", {     "class":`input`,"bind:value":pageId,"aria-label":`Page`,});/*Ωignore_startΩ*/() => pageId = __sveltets_2_any(null);/*Ωignore_endΩ*/
           for(let p of __sveltets_2_ensureArray(doc.pages)){p.id; { svelteHTML.createElement("option", { "value":p.id,});p.title || 'Untitled page'; }}
       }
    }
     { svelteHTML.createElement("div", {     "class":`segmented`,"role":`group`,"aria-label":`Width`,});
       { svelteHTML.createElement("button", {     "aria-pressed":device === 'fill',"onclick":() => (device = 'fill'),});device === 'fill';  }
       { svelteHTML.createElement("button", {     "aria-pressed":device === 'desktop',"onclick":() => (device = 'desktop'),});device === 'desktop';  }
       { svelteHTML.createElement("button", {     "aria-pressed":device === 'tablet',"onclick":() => (device = 'tablet'),});device === 'tablet';  }
       { svelteHTML.createElement("button", {     "aria-pressed":device === 'mobile',"onclick":() => (device = 'mobile'),});device === 'mobile';  }
     }
     { svelteHTML.createElement("span", { "class":`grow`,}); }
    if(servedUrl){
       { svelteHTML.createElement("span", {   "class":`served`,"title":servedUrl,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`shield`,"size":12,}});}            }
    }else{
       { svelteHTML.createElement("span", { "class":`served`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`info`,"size":12,}});}          }
    }
   }
   { svelteHTML.createElement("div", {  "class":`well`,});__sveltets_2_ensureType(String, Number, `${Math.max(320, winH - 220)}px`);
     { const $$_iframe2 = svelteHTML.createElement("iframe", {            "title":`Preview of ${page?.title ?? 'the site'}`,"sandbox":`allow-same-origin`,srcdoc,"onload":wire,...__sveltets_2_empty({"data-testid":`site-preview-frame`}),});frame = $$_iframe2;__sveltets_2_ensureType(String, Number, width); }
   }
 Modal}


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const PreviewModal__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type PreviewModal__SvelteComponent_ = ReturnType<typeof PreviewModal__SvelteComponent_>;
/*Ωignore_endΩ*/export default PreviewModal__SvelteComponent_;