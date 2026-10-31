///<reference types="svelte" />
;
  /** A section offered from another site ("From your library"). */
  export interface LibraryBlock {
    key: string;
    siteId: string;
    siteTitle: string;
    seq: number;
    section: import('./engine/types').SiteSection;
  }
;;// Site Studio left panel: Pages · Layers · Blocks.
//   Pages  — the site's pages (home first), add / rename / delete.
//   Layers — the current page's sections (drag to reorder, eye = hide
//            everywhere, a glyph when hidden at a breakpoint); the selected
//            section expands to its child blocks (embeds, cards, tiers…).
//   Blocks — the searchable library, grouped by family, each tile a LIVE
//            miniature rendered with the site's brand theme; plus "From your
//            library": sections of other sites, with provenance. Click a tile
//            to insert after the selection, or drag it onto the canvas.


import Icon from '../../../lib/components/Icon.svelte';
import { onTabKey } from '../../../lib/tabKeys';
import type { IconName } from '../../../lib/components/Icon.svelte';
import { FAMILIES, itemDef, searchBlocks, sectionLabel, type SectionDef } from './engine/catalog';
import { renderSection, str } from './engine/render';
import { themeStyle, type Theme } from './engine/theme';
import type { SiteBlock, SiteDoc, SiteSection } from './engine/types';
import { BLOCK_MIME } from './SiteCanvas.svelte';
;

  interface Props {
    doc: SiteDoc;
    pageId: string;
    theme: Theme;
    selectedSection: string | null;
    selectedBlock: string | null;
    readonly: boolean;
    tab: 'pages' | 'layers' | 'blocks';
    library: LibraryBlock[];
    libraryState: 'idle' | 'loading' | 'ready' | 'error';
    onpage: (id: string) => void;
    onaddpage: () => void;
    onpagemenu: (id: string, e: MouseEvent) => void;
    onselect: (sectionId: string, blockId: string | null) => void;
    onreorder: (sectionId: string, to: number) => void;
    ontogglehidden: (sectionId: string) => void;
    oninsert: (payload: string) => void;
    onloadlibrary: () => void;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    doc,
    pageId,
    theme,
    selectedSection,
    selectedBlock,
    readonly,
    tab = $bindable(),
    library,
    libraryState,
    onpage,
    onaddpage,
    onpagemenu,
    onselect,
    onreorder,
    ontogglehidden,
    oninsert,
    onloadlibrary,
  }: Props = $props()/*Ωignore_startΩ*/;tab;/*Ωignore_endΩ*/;

  const page = $derived(doc.pages.find((p) => p.id === pageId) ?? doc.pages[0]);
  let query = $state('');
  const found = $derived(searchBlocks(query));
  const groups = $derived(
    FAMILIES.map((f) => ({ ...f, blocks: found.filter((b) => b.family === f.id) })).filter((g) => g.blocks.length),
  );
  const libFound = $derived(
    library.filter((l) => {
      const q = query.trim().toLowerCase();
      if (!q) return true;
      return `${sectionLabel(l.section)} ${l.siteTitle} ${l.section.block}`.toLowerCase().includes(q);
    }),
  );
  $effect(() => {
    if (tab === 'blocks' && libraryState === 'idle') onloadlibrary();
  });

  const FAMILY_ICON: Record<string, IconName> = {
    nav: 'panel',
    hero: 'layout',
    features: 'grid',
    social: 'comment',
    pricing: 'tag',
    faq: 'info',
    cta: 'zap',
    content: 'file',
    media: 'image',
    footer: 'columns',
  };
  function familyIcon(block: string): IconName {
    if (block === 'media/3d-embed') return 'box';
    return FAMILY_ICON[block.split('/')[0]] ?? 'layout';
  }

  // Mini renders (live, themed) for the library tiles.
  const tileCtx = $derived({ theme, editable: false });
  const style = $derived(themeStyle(theme));
  function preview(def: SectionDef): string {
    return renderSection(
      { id: `tile-${def.block.replace('/', '-')}`, block: def.block, props: def.props, style: def.style, blocks: def.blocks.map((b, i) => ({ ...b, id: `t${i}` })) },
      tileCtx,
    );
  }

  // ── Layers drag to reorder ─────────────────────────────────────────────────
  let dragId = $state<string | null>(null);
  let dropAt = $state<number | null>(null);
  function rowDragStart(e: DragEvent, id: string): void {
    if (readonly) return;
    dragId = id;
    e.dataTransfer?.setData('text/plain', id);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'move';
  }
  function rowDragOver(e: DragEvent, index: number): void {
    if (!dragId) return;
    e.preventDefault();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    dropAt = e.clientY < r.top + r.height / 2 ? index : index + 1;
  }
  function rowDrop(e: DragEvent): void {
    if (!dragId || dropAt == null || !page) return;
    e.preventDefault();
    const from = page.sections.findIndex((s) => s.id === dragId);
    const to = dropAt > from ? dropAt - 1 : dropAt;
    if (from >= 0 && to !== from) onreorder(dragId, to);
    dragId = null;
    dropAt = null;
  }

  function childLabel(b: SiteBlock): string {
    const def = itemDef(b.block);
    if (b.block === 'embed/3d') return `3D embed · ${str(b.props, 'label') || str(b.props, 'alt') || 'artifact'}`;
    if (b.block === 'embed/image') return `Image · ${str(b.props, 'alt') || 'untitled'}`;
    const t = def ? str(b.props, def.titleKey) : '';
    return `${def?.label ?? b.block}${t ? ` · ${t}` : ''}`;
  }
  function hiddenAt(s: SiteSection): string | null {
    const h = s.responsive?.hide ?? [];
    return h.length ? `Hidden on ${h.join(', ')}` : null;
  }

  function tileDrag(e: DragEvent, payload: string): void {
    e.dataTransfer?.setData(BLOCK_MIME, payload);
    if (e.dataTransfer) e.dataTransfer.effectAllowed = 'copy';
  }
;
async () => {



 { svelteHTML.createElement("div", { "class":`panel`,});
   { svelteHTML.createElement("div", {     "class":`tabs segmented`,"role":`tablist`,"aria-label":`Site panels`,});
     { svelteHTML.createElement("button", {                "role":`tab`,"id":`site-tab-pages`,"aria-controls":`site-panel`,"aria-selected":tab === 'pages',"tabindex":tab === 'pages' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (tab = 'pages'),...__sveltets_2_empty({"data-testid":`site-tab-pages`}),});tab === 'pages';
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`file`,"size":12,}});} 
     }
     { svelteHTML.createElement("button", {                "role":`tab`,"id":`site-tab-layers`,"aria-controls":`site-panel`,"aria-selected":tab === 'layers',"tabindex":tab === 'layers' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (tab = 'layers'),...__sveltets_2_empty({"data-testid":`site-tab-layers`}),});tab === 'layers';
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`layers`,"size":12,}});} 
     }
     { svelteHTML.createElement("button", {                "role":`tab`,"id":`site-tab-blocks`,"aria-controls":`site-panel`,"aria-selected":tab === 'blocks',"tabindex":tab === 'blocks' ? 0 : -1,"onkeydown":onTabKey,"onclick":() => (tab = 'blocks'),...__sveltets_2_empty({"data-testid":`site-tab-blocks`}),});tab === 'blocks';
       { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`grid`,"size":12,}});} 
     }
   }

   { svelteHTML.createElement("div", {       "class":`body`,"role":`tabpanel`,"id":`site-panel`,"aria-labelledby":`site-tab-${tab}`,});
    if(tab === 'pages'){
       { svelteHTML.createElement("ul", {   "class":`list`,"aria-label":`Pages`,});
            for(let p of __sveltets_2_ensureArray(doc.pages)){let i = 1;p.id;
           { svelteHTML.createElement("li", {  "class":`row page-row`,});p.id === page?.id;
             { svelteHTML.createElement("button", {     "class":`row-main`,"onclick":() => onpage(p.id),"aria-current":p.id === page?.id ? 'page' : undefined,});
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":i === 0 ? 'home' : 'file',"size":12,}});}
               { svelteHTML.createElement("span", { "class":`name`,});p.title || 'Untitled page'; }
               { svelteHTML.createElement("span", { "class":`dim mono`,}); p.slug; }
             }
            if(!readonly){
               { svelteHTML.createElement("button", {         "class":`icon-btn`,"onclick":(e) => onpagemenu(p.id, e),"aria-label":`Page actions for ${p.title}`,"title":`Page actions`,"aria-haspopup":`menu`,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":12,}});}
               }
            }
           }
        }
       }
      if(!readonly){
         { svelteHTML.createElement("button", {     "class":`btn small ghost add`,"onclick":onaddpage,...__sveltets_2_empty({"data-testid":`site-add-page`}),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}   }
      }
    } else if (tab === 'layers'){
       { svelteHTML.createElement("p", { "class":`section-title`,});page?.title || 'Page';  page?.sections.length ?? 0;  }
      if(page && page.sections.length){
         { svelteHTML.createElement("ul", {     "class":`list`,"aria-label":`Sections`,"ondragleave":() => (dropAt = null),});
              for(let s of __sveltets_2_ensureArray(page.sections)){let i = 1;s.id;
            const sel = s.id === selectedSection;
            const off = hiddenAt(s);
             { svelteHTML.createElement("li", {               "class":`layer`,"draggable":!readonly,"ondragstart":(e) => rowDragStart(e, s.id),"ondragover":(e) => rowDragOver(e, i),"ondrop":rowDrop,"ondragend":() => {
                dragId = null;
                dropAt = null;
              },});dropAt === i;dropAt === i + 1 && i === page.sections.length - 1;
               { svelteHTML.createElement("div", {   "class":`row`,});sel && !selectedBlock;s.hidden;
                if(!readonly){ { svelteHTML.createElement("span", {   "class":`grip reveal-on-hover`,"aria-hidden":`true`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`grip`,"size":12,}});} }}
                 { svelteHTML.createElement("button", {       "class":`row-main`,"onclick":() => onselect(s.id, null),"aria-pressed":sel,...__sveltets_2_empty({"data-testid":`site-layer`}),});
                   { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":familyIcon(s.block),"size":12,}});}
                   { svelteHTML.createElement("span", { "class":`name`,});sectionLabel(s); }
                  if(s.derived_from){ { svelteHTML.createElement("span", {   "class":`glyph`,"title":`From your library`,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});} }}
                  if(off){ { svelteHTML.createElement("span", {   "class":`glyph warn`,"title":off,}); { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":`eyeOff`,"size":12,}});} }}
                 }
                if(!readonly){
                   { svelteHTML.createElement("button", {       "class":`icon-btn eye reveal-on-hover`,"onclick":() => ontogglehidden(s.id),"aria-label":s.hidden ? `Show ${sectionLabel(s)}` : `Hide ${sectionLabel(s)}`,"title":s.hidden ? `Show ${sectionLabel(s)}` : `Hide ${sectionLabel(s)}`,});
                     { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":s.hidden ? 'eyeOff' : 'eye',"size":12,}});}
                   }
                }
               }
              if((s.blocks ?? []).length && (sel || (s.blocks ?? []).some((b) => b.block.startsWith('embed/3d')))){
                const kids = sel ? (s.blocks ?? []) : (s.blocks ?? []).filter((b) => b.block === 'embed/3d');
                 { svelteHTML.createElement("ul", { "class":`kids`,});
                     for(let b of __sveltets_2_ensureArray((kids.slice(0, 12)))){b.id;
                     { svelteHTML.createElement("li", {  "class":`row kid`,});b.id === selectedBlock;
                       { svelteHTML.createElement("button", {     "class":`row-main`,"onclick":() => onselect(s.id, b.id),"aria-pressed":b.id === selectedBlock,});
                         { const $$_nocI7C = __sveltets_2_ensureComponent(Icon); new $$_nocI7C({ target: __sveltets_2_any(), props: {    "name":b.block === 'embed/3d' ? 'box' : b.block === 'embed/image' ? 'image' : 'square',"size":12,}});}
                         { svelteHTML.createElement("span", { "class":`name`,});childLabel(b); }
                        if(b.block === 'embed/3d' && str(b.props, 'src')){ { svelteHTML.createElement("span", {   "class":`glyph link`,"title":`Linked 3D artifact`,}); { const $$_nocI8C = __sveltets_2_ensureComponent(Icon); new $$_nocI8C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});} }}
                       }
                     }
                  }
                  if(sel && (s.blocks ?? []).length > 12){ { svelteHTML.createElement("li", { "class":`dim more`,}); (s.blocks ?? []).length - 12;  }}
                 }
              }
             }
          }
         }
      }else{
         { svelteHTML.createElement("p", { "class":`dim hint`,});     { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => (tab = 'blocks'),});  }    }
      }
    }else{
       { svelteHTML.createElement("label", { "class":`search`,});
         { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`search`,"size":12,}});}
         { svelteHTML.createElement("input", {            "dir":`ltr`,"class":`input`,"placeholder":`Filter blocks…`,"bind:value":query,"aria-label":`Filter blocks`,...__sveltets_2_empty({"data-testid":`site-block-search`}),});/*Ωignore_startΩ*/() => query = __sveltets_2_any(null);/*Ωignore_endΩ*/}
       }
      if(readonly){ { svelteHTML.createElement("p", { "class":`dim hint`,});      }}
         for(let g of __sveltets_2_ensureArray(groups)){g.id;
         { svelteHTML.createElement("section", {   "class":`group`,"aria-label":g.label,});
           { svelteHTML.createElement("h3", { "class":`section-title`,});g.label;  { svelteHTML.createElement("span", { "class":`count`,});g.blocks.length; } }
           { svelteHTML.createElement("div", { "class":`tiles`,});
               for(let def of __sveltets_2_ensureArray(g.blocks)){def.block;
               { svelteHTML.createElement("button", {                 "class":`tile`,"disabled":readonly,"draggable":!readonly,"ondragstart":(e) => tileDrag(e, def.block),"onclick":() => oninsert(def.block),"title":`${def.label} — ${def.description}. Click to insert, or drag onto the page.`,...__sveltets_2_empty({"data-testid":`site-block-tile`}),...__sveltets_2_empty({"data-block":def.block}),});
                 { svelteHTML.createElement("span", {    "class":`thumb`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, theme.surface);
                   { svelteHTML.createElement("span", {  "class":`mini os-site os-still`,style,}); preview(def); }
                 }
                 { svelteHTML.createElement("span", { "class":`tile-label`,});def.label.replace(/^[^·]+·\s*/, '') || def.label; }
               }
            }
           }
         }
      }
         { svelteHTML.createElement("p", { "class":`dim hint`,});   query;  }
      
       { svelteHTML.createElement("section", {   "class":`group`,"aria-label":`From your library`,});
         { svelteHTML.createElement("h3", { "class":`section-title`,});   if(libFound.length){ { svelteHTML.createElement("span", { "class":`count`,});libFound.length; }} }
        if(libraryState === 'loading'){
           { svelteHTML.createElement("p", { "class":`dim hint`,});     }
        } else if (libraryState === 'error'){
           { svelteHTML.createElement("p", { "class":`dim hint`,});      { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":onloadlibrary,});  } }
        } else if (!libFound.length){
           { svelteHTML.createElement("p", { "class":`dim hint`,});             }
        }else{
           { svelteHTML.createElement("div", { "class":`tiles`,});
               for(let l of __sveltets_2_ensureArray((libFound.slice(0, 24)))){l.key;
               { svelteHTML.createElement("button", {               "class":`tile`,"disabled":readonly,"draggable":!readonly,"ondragstart":(e) => tileDrag(e, `lib:${l.key}`),"onclick":() => oninsert(`lib:${l.key}`),"title":`${sectionLabel(l.section)} · from ${l.siteTitle} v${l.seq}`,...__sveltets_2_empty({"data-testid":`site-library-tile`}),});
                 { svelteHTML.createElement("span", {    "class":`thumb`,"aria-hidden":`true`,});__sveltets_2_ensureType(String, Number, theme.surface);
                   { svelteHTML.createElement("span", {  "class":`mini os-site os-still`,style,}); renderSection(l.section, tileCtx); }
                 }
                 { svelteHTML.createElement("span", { "class":`tile-label`,});sectionLabel(l.section); }
                 { svelteHTML.createElement("span", { "class":`prov`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});} l.siteTitle; l.seq; }
               }
            }
           }
        }
       }
    }
   }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('tab'), slots: {}, events: {} }}
const LeftPanel__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type LeftPanel__SvelteComponent_ = ReturnType<typeof LeftPanel__SvelteComponent_>;
/*Ωignore_endΩ*/export default LeftPanel__SvelteComponent_;