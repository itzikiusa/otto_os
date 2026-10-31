///<reference types="svelte" />
;
import { tick } from 'svelte';
import { plural } from '../../../lib/plural';
import { announceShown } from '../../../lib/announce.svelte';
// Site Studio "Design" inspector for the selection:
//   nothing selected → Page + Site settings, the brand kit, and the page's
//                      accessibility / quality findings (click one to select);
//   a section        → layout swap, content fields bound to the canvas, its
//                      repeated items, the media slot, style (brand-token
//                      swatches with an off-brand warning, spacing, alignment,
//                      motion presets, min height), responsive toggles and live
//                      WCAG contrast;
//   a child block    → its fields; a 3D embed gets source + version policy
//                      (follow approved / follow latest / pin vN) + interaction.
// Every edit is a new document via `onchange` (the studio owns undo + saving).
import Icon from '../../../lib/components/Icon.svelte';
import Switch from '../../../lib/components/Switch.svelte';
import LoadState from '../../../lib/components/LoadState.svelte';
import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
import { confirmer } from '../../../lib/confirm.svelte';
import type { DesignArtifact } from '../../../lib/api/types';
import { itemDef, layoutsOf, sectionDef, sectionLabel, type FieldDef } from './engine/catalog';
import { SITE_ICON_NAMES } from './engine/icons';
import * as ops from './engine/ops';
import { flag, lines, links, str, type EmbedInfo } from './engine/render';
import { GRADIENTS, parseColor, resolveBackground, sectionContrast, swatches, type Theme } from './engine/theme';
import type { AuditFinding } from './engine/audit';
import { urlProblem } from './engine/validate';
import type { SiteDoc, SiteMotion, SiteProps, SiteSection } from './engine/types';
import { brandLabel, parseRef, type BrandResolution } from './siteApi';
import LinkListField from './LinkListField.svelte';
;

  interface Props {
    doc: SiteDoc;
    pageId: string;
    theme: Theme;
    /** The resolved brand kit (its label, note, or load error). */
    brand: BrandResolution;
    onretrybrand: () => void;
    sectionId: string | null;
    blockId: string | null;
    readonly: boolean;
    findings: AuditFinding[];
    embed: (uri: string) => EmbedInfo | null;
    /** 3D / image artifacts the pickers offer (workspace-scoped, loaded lazily). */
    pickArtifacts: (kind: '3d' | 'image' | 'brand') => Promise<DesignArtifact[]>;
    onchange: (next: SiteDoc, coalesce?: string) => void;
    onselect: (sectionId: string | null, blockId: string | null) => void;
    onopen: (artifactId: string) => void;
    onbrandcorrection: (payload: Record<string, unknown>) => void;
  };function $$render() {
 const field/*Ωignore_positionΩ*/ = (f: FieldDef)/*Ωignore_startΩ*/: ReturnType<import('svelte').Snippet>/*Ωignore_endΩ*/ => { async ()/*Ωignore_positionΩ*/ => {
  const v = str(cur(), f.key);
  if(f.kind === 'toggle'){
     { svelteHTML.createElement("div", { "class":`switch-row`,});
       { svelteHTML.createElement("span", {});f.label; }
       { const $$_hctiwS1C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS1C({ target: __sveltets_2_any(), props: {        "checked":flag(cur(), f.key),"disabled":readonly,"onchange":(next) => setProp(f.key, next),"label":f.label,}});}
     }
  } else if (f.kind === 'links'){
     { const $$_dleiFtsiLkniL0C = __sveltets_2_ensureComponent(LinkListField); new $$_dleiFtsiLkniL0C({ target: __sveltets_2_any(), props: {       "label":f.label,"rows":links(cur(), f.key),readonly,"onchange":(rows) => setProp(f.key, rows),}});}
  }else{
    const warn = urlWarning(f, v);
     { svelteHTML.createElement("div", {  "class":`field`,});f.half;
       { svelteHTML.createElement("label", { "for":`si-${f.key}`,});f.label; }
      if(f.kind === 'textarea'){
         { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`si-${f.key}`,"class":`input`,"rows":f.key === 'body' || f.key === 'quote' ? 4 : 2,"value":v,"disabled":readonly,"oninput":(e) => setProp(f.key, e.currentTarget.value),}); }
      } else if (f.kind === 'lines'){
         { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`si-${f.key}`,"class":`input`,"rows":4,"value":rawLines(f.key),"disabled":readonly,"oninput":(e) => setProp(f.key, e.currentTarget.value.split('\n')),}); }
      } else if (f.kind === 'icon'){
         { svelteHTML.createElement("input", {                "dir":`auto`,"id":`si-${f.key}`,"class":`input`,"list":`site-icon-names`,"value":v,"disabled":readonly,"placeholder":`bolt, star, 2×…`,"oninput":(e) => setProp(f.key, e.currentTarget.value),});}
      } else if (f.kind === 'media'){
         { svelteHTML.createElement("div", { "class":`with-btn`,});
           { svelteHTML.createElement("input", {              "dir":`ltr`,"id":`si-${f.key}`,"class":`input`,"value":v,"disabled":readonly,"placeholder":`https://… or pick from the library`,"oninput":(e) => setProp(f.key, e.currentTarget.value),});}
          if(!readonly){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":(e) => void choose('image', e, (uri) => setProp(f.key, uri)),});  }}
         }
      }else{
         { svelteHTML.createElement("input", {              "dir":`auto`,"id":`si-${f.key}`,"class":`input`,"value":v,"disabled":readonly,"placeholder":f.placeholder ?? '',"oninput":(e) => setProp(f.key, e.currentTarget.value),});}
      }
      if(warn){ { svelteHTML.createElement("span", { "class":`hint bad`,}); { const $$_nocI2C = __sveltets_2_ensureComponent(Icon); new $$_nocI2C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});} warn; }} else if (f.hint){ { svelteHTML.createElement("span", { "class":`hint`,});f.hint; }}
     }
  }
};return __sveltets_2_any(0)};
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    doc,
    pageId,
    theme,
    brand,
    onretrybrand,
    sectionId,
    blockId,
    readonly,
    findings,
    embed,
    pickArtifacts,
    onchange,
    onselect,
    onopen,
    onbrandcorrection,
  }: Props = $props();

  const page = $derived(doc.pages.find((p) => p.id === pageId) ?? doc.pages[0]);
  const secAt = $derived(ops.findSection(doc, sectionId));
  const blkAt = $derived(ops.findBlock(doc, blockId));
  const section = $derived<SiteSection | null>(blkAt?.section ?? secAt?.section ?? null);
  const def = $derived(section ? sectionDef(section.block) : undefined);
  const block = $derived(blkAt?.block ?? null);
  const bdef = $derived(block ? itemDef(block.block) : undefined);
  const kitLabel = $derived(brandLabel(brand));

  // ── Focus: a drill-in, back or delete swaps the whole panel and unmounts the
  // focused control. Land on the new view's name, or (back) on the row of the
  // block just left — never on <body>, across the canvas.
  let rootEl = $state<HTMLElement | null>(null);
  async function go(s: string | null, b: string | null, from: string | null = null): Promise<void> {
    onselect(s, b);
    await tick();
    const row = from ? rootEl?.querySelector<HTMLElement>(`[data-item="${CSS.escape(from)}"]`) : null;
    (row ?? rootEl?.querySelector<HTMLElement>('[data-view-head]'))?.focus();
  }

  // ── Edits ──────────────────────────────────────────────────────────────────
  function setProp(key: string, value: SiteProps[string]): void {
    if (readonly) return;
    if (block) onchange(ops.setBlockProp(doc, block.id, key, value), `b:${block.id}:${key}`);
    else if (section) onchange(ops.setSectionProp(doc, section.id, key, value), `s:${section.id}:${key}`);
  }
  function cur(): SiteProps {
    return (block ? block.props : section?.props) ?? {};
  }

  function swapMenu(e: MouseEvent): void {
    if (!section) return;
    const items: MenuItem[] = layoutsOf(section.block).map((d) => ({
      label: d.label,
      icon: d.block === section!.block ? 'check' : 'layout',
      disabled: d.block === section!.block || readonly,
      action: () => onchange(ops.swapLayout(doc, section!.id, d.block)),
    }));
    ctxMenu.show(e, items);
  }

  async function deleteSection(): Promise<void> {
    if (!section) return;
    const ok = await confirmer.ask(`Delete “${sectionLabel(section)}” from this page? Undo (⌘Z) or any earlier version brings it back.`, {
      title: 'Delete section',
      danger: true, confirmLabel: 'Delete section',
    });
    if (ok) {
      onchange(ops.removeSection(doc, section.id));
      void go(null, null);
    }
  }

  // ── Style ──────────────────────────────────────────────────────────────────
  // A kit that failed to load leaves only the default palette: judging colours against it would mislead.
  const bg = $derived(section && !brand.error ? resolveBackground(section.style?.background, theme) : null);
  const contrastNow = $derived(section && !brand.error ? sectionContrast(section, theme) : null);
  const brandSwatches = $derived(swatches(theme));
  let hexDraft = $state('');
  $effect(() => {
    const v = section?.style?.background ?? '';
    hexDraft = v.startsWith('#') ? v : '';
  });

  function setBg(value: string): void {
    if (!section) return;
    const prev = section.style?.background ?? '';
    onchange(ops.setStyle(doc, section.id, 'background', value || undefined));
    if (prev.startsWith('#') && (value.startsWith('token:') || value.startsWith('gradient:'))) {
      onbrandcorrection({ section_id: section.id, block: section.block, from: prev, to: value });
    }
  }
  function commitHex(): void {
    const v = hexDraft.trim();
    if (!v) return;
    const hex = v.startsWith('#') ? v : `#${v}`;
    if (parseColor(hex)) setBg(hex);
  }
  /** The brand swatch closest to a raw colour (RGB distance). */
  const nearest = $derived.by(() => {
    if (!bg?.offBrand) return null;
    const c = parseColor(section?.style?.background ?? '');
    if (!c) return null;
    let best: { value: string; label: string; d: number } | null = null;
    for (const s of brandSwatches) {
      const k = parseColor(s.color);
      if (!k) continue;
      const d = (c[0] - k[0]) ** 2 + (c[1] - k[1]) ** 2 + (c[2] - k[2]) ** 2;
      if (!best || d < best.d) best = { value: s.value, label: s.label, d };
    }
    return best;
  });

  const MOTIONS: { value: SiteMotion; label: string; hint: string }[] = [
    { value: 'none', label: 'None', hint: 'Static.' },
    { value: 'fade-up', label: 'Fade up', hint: 'Content rises in as it scrolls into view.' },
    { value: 'scroll-reveal', label: 'Scroll reveal', hint: 'Cards arrive one after another.' },
    { value: 'parallax', label: 'Parallax', hint: 'Decorations drift slower than the page.' },
    { value: 'tilt-hover', label: 'Tilt on hover', hint: 'Cards and media tilt toward the pointer.' },
  ];
  const motionHint = $derived(MOTIONS.find((m) => m.value === (section?.style?.motion || 'none'))?.hint ?? '');

  // ── Media slot / 3D ────────────────────────────────────────────────────────
  const mediaChild = $derived(section && def?.media ? (section.blocks ?? []).find((b) => def!.media!.includes(b.block)) ?? null : null);
  const embedSrc = $derived(block?.block === 'embed/3d' ? str(block.props, 'src') : '');
  const embedRef = $derived(embedSrc ? parseRef(embedSrc) : null);
  const embedInfo = $derived(embedSrc ? embed(embedSrc) : null);
  const policy = $derived(!embedRef ? 'approved' : typeof embedRef.sel === 'number' ? 'pinned' : embedRef.sel === 'latest' ? 'latest' : 'approved');

  function setEmbedPolicy(p: 'approved' | 'latest' | 'pinned'): void {
    if (!embedRef) return;
    const seq = embedInfo?.seq ?? 1;
    const sel = p === 'pinned' ? `@v${seq}` : `@${p}`;
    setProp('src', `otto://design/${embedRef.id}${sel}`);
  }

  /** The clicked control, captured before the await (currentTarget is gone
   *  after it) so the menu still opens under it. */
  function anchorOf(e: MouseEvent): HTMLElement | null {
    return e.currentTarget instanceof HTMLElement ? e.currentTarget : null;
  }

  async function choose(kind: '3d' | 'image', e: MouseEvent, apply: (uri: string) => void): Promise<void> {
    await chooseAt(kind, anchorOf(e), apply);
  }
  async function chooseAt(kind: '3d' | 'image', anchor: HTMLElement | null, apply: (uri: string) => void): Promise<void> {
    let list: DesignArtifact[] = [];
    try {
      list = await pickArtifacts(kind);
    } catch {
      // A failed lookup is not "none yet": say so, and offer the retry.
      const what = kind === '3d' ? '3D artifacts' : 'images';
      ctxMenu.showAt(anchor, [
        { label: `Couldn’t load ${what}`, icon: 'warning', disabled: true },
        { label: 'Try again', icon: 'refresh', action: () => void chooseAt(kind, anchor, apply) },
      ]);
      return;
    }
    const items: MenuItem[] = list.length
      ? list.map((a) => ({
          label: `${a.title}${a.head_seq ? ` · v${a.head_seq}` : ''}`,
          icon: kind === '3d' ? 'box' : 'image',
          action: () => apply(kind === '3d' ? `otto://design/${a.id}@approved` : `otto://design/${a.id}`),
        }))
      : [{ label: kind === '3d' ? 'No 3D artifacts in this workspace yet' : 'No images in this workspace yet', disabled: true }];
    ctxMenu.showAt(anchor, items, { filter: items.length > 8, filterPlaceholder: 'Filter by title', maxVisible: 12 });
  }

  function addMedia(kind: 'embed/3d' | 'embed/image', e: MouseEvent): void {
    if (!section) return;
    if (kind === 'embed/3d') {
      void choose('3d', e, (uri) => {
        const r = ops.setMedia(doc, section!.id, 'embed/3d', { src: uri, alt: 'Interactive 3D view' });
        onchange(r.doc);
        void go(section!.id, r.id);
      });
    } else {
      const r = ops.setMedia(doc, section.id, 'embed/image', { src: '', alt: '' });
      onchange(r.doc);
      void go(section.id, r.id);
    }
  }

  // ── Page / site ────────────────────────────────────────────────────────────
  function setPage(key: 'title' | 'slug' | 'description', value: string): void {
    if (!page || readonly) return;
    const next = ops.clone(doc);
    const p = next.pages.find((x) => x.id === page!.id)!;
    if (key === 'slug') p.slug = ops.slugify(value);
    else p[key] = value;
    onchange(next, `p:${page.id}:${key}`);
  }
  function setSite(key: 'title' | 'domain' | 'lang', value: string): void {
    if (readonly) return;
    const next = ops.clone(doc);
    if (key === 'title') next.title = value;
    else next.settings = { ...(next.settings ?? {}), [key]: value };
    onchange(next, `site:${key}`);
  }
  async function linkKit(e: MouseEvent): Promise<void> {
    await linkKitAt(anchorOf(e));
  }
  async function linkKitAt(anchor: HTMLElement | null): Promise<void> {
    let kits: DesignArtifact[] = [];
    try {
      kits = await pickArtifacts('brand');
    } catch {
      ctxMenu.showAt(anchor, [
        { label: 'Couldn’t load brand kits', icon: 'warning', disabled: true },
        { label: 'Try again', icon: 'refresh', action: () => void linkKitAt(anchor) },
      ]);
      return;
    }
    const items: MenuItem[] = kits.map((k) => ({
      label: k.title,
      icon: 'palette',
      action: () => {
        const next = ops.clone(doc);
        next.brand = `otto://design/${k.id}`;
        onchange(next);
      },
    }));
    if (doc.brand) {
      items.push({ separator: true }, {
        label: 'Use the default palette',
        icon: 'x',
        action: () => {
          const next = ops.clone(doc);
          delete next.brand;
          onchange(next);
        },
      });
    }
    if (!items.length) items.push({ label: 'No brand kits in this workspace yet', disabled: true });
    ctxMenu.showAt(anchor, items);
  }

  const errors = $derived(findings.filter((f) => f.level === 'error').length);
  const warns = $derived(findings.filter((f) => f.level === 'warn').length);

  // ── Field helpers ──────────────────────────────────────────────────────────
  function urlWarning(f: FieldDef, v: string): string | null {
    if (f.kind === 'url') return urlProblem('href', v);
    if (f.kind === 'media' && f.key !== 'src') return urlProblem('src', v);
    return null;
  }
  const rawLines = (key: string): string => {
    const v = cur()[key];
    return Array.isArray(v) ? (v as string[]).join('\n') : typeof v === 'string' ? v : '';
  };
  const itemTitle = (b: { block: string; props: SiteProps }): string => {
    const d = itemDef(b.block);
    return (d ? str(b.props, d.titleKey) : '') || d?.label || b.block;
  };
;
async () => {



 { const $$_div0 = svelteHTML.createElement("div", {    "class":`insp`,...__sveltets_2_empty({"data-testid":`site-inspector`}),});rootEl = $$_div0;
   { svelteHTML.createElement("datalist", { "id":`site-icon-names`,});
       for(let n of __sveltets_2_ensureArray(SITE_ICON_NAMES)){n; { svelteHTML.createElement("option", { "value":n,}); }}
   }

  if(block && bdef && section){
    
     { svelteHTML.createElement("div", { "class":`group`,});
       { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  }
         { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => void go(section.id, null, block!.id),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronLeft`,"size":12,}});} sectionLabel(section); }
       }
       { svelteHTML.createElement("div", { "class":`ident`,});
         { svelteHTML.createElement("span", { "class":`ident-icon`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":block.block === 'embed/3d' ? 'box' : block.block === 'embed/image' ? 'image' : 'square',"size":14,}});} }
         { svelteHTML.createElement("div", {    "class":`ident-text`,"tabindex":-1,...__sveltets_2_empty({"data-view-head":true}),}); { svelteHTML.createElement("strong", {});bdef.label; } { svelteHTML.createElement("span", { "class":`dim`,});itemTitle(block); } }
        if(!readonly){
           { svelteHTML.createElement("div", { "class":`ident-actions`,});
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => onchange(ops.moveBlock(doc, block!.id, -1)),"aria-label":`Move up`,"title":`Move up`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});} }
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => onchange(ops.moveBlock(doc, block!.id, 1)),"aria-label":`Move down`,"title":`Move down`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});} }
             { svelteHTML.createElement("button", {       "class":`icon-btn`,"onclick":() => { onchange(ops.removeBlock(doc, block!.id)); void go(section!.id, null); },"aria-label":`Remove block`,"title":`Remove`,}); { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});} }
           }
        }
       }
     }

    if(block.block === 'embed/3d'){
       { svelteHTML.createElement("div", { "class":`group`,});
         { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  } }
        if(embedRef && embedInfo){
           { svelteHTML.createElement("div", {  "class":`src`,});embedInfo.broken;
             { svelteHTML.createElement("span", { "class":`src-icon`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":14,}});} }
             { svelteHTML.createElement("div", { "class":`ident-text`,});
               { svelteHTML.createElement("strong", {});embedInfo.title; }
               { svelteHTML.createElement("span", { "class":`dim`,});embedInfo.seq != null ? `v${embedInfo.seq}` : 'no version yet';  policy === 'pinned' ? 'pinned' : policy === 'latest' ? 'follows latest' : 'follows approved'; }
             }
            if(!embedInfo.broken){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":() => onopen(embedRef!.id),});    }}
           }
        }else{
           { svelteHTML.createElement("p", { "class":`dim note`,});           }
        }
        if(!readonly){ { svelteHTML.createElement("button", {     "class":`btn small`,"onclick":(e) => void choose('3d', e, (uri) => setProp('src', uri)),...__sveltets_2_empty({"data-testid":`site-pick-3d`}),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":12,}});} embedRef ? 'Replace…' : 'Choose a 3D artifact…'; }}
       }
      if(embedRef){
         { svelteHTML.createElement("div", { "class":`group`,});
           { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  } }
           { svelteHTML.createElement("div", {     "class":`radios`,"role":`radiogroup`,"aria-label":`Version policy`,});
             { svelteHTML.createElement("label", {}); { svelteHTML.createElement("input", {          "type":`radio`,"name":`policy`,"checked":policy === 'approved',"disabled":readonly,"onchange":() => setEmbedPolicy('approved'),});}   }
             { svelteHTML.createElement("label", {}); { svelteHTML.createElement("input", {          "type":`radio`,"name":`policy`,"checked":policy === 'latest',"disabled":readonly,"onchange":() => setEmbedPolicy('latest'),});}   }
             { svelteHTML.createElement("label", {}); { svelteHTML.createElement("input", {          "type":`radio`,"name":`policy`,"checked":policy === 'pinned',"disabled":readonly,"onchange":() => setEmbedPolicy('pinned'),});}  embedInfo?.seq != null ? `v${embedInfo.seq}` : 'this version'; }
           }
           { svelteHTML.createElement("p", { "class":`note`,});
            policy === 'approved'
              ? 'This page updates when a new 3D version is approved. Drafts never leak in.'
              : policy === 'latest'
                ? 'Live co-design: every saved 3D version shows up here.'
                : 'Frozen at this version; newer ones show as “available”.';
                    
           }
         }
      }
       { svelteHTML.createElement("div", { "class":`group`,});
         { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});   } }
           for(let f of __sveltets_2_ensureArray(bdef.fields.filter((f) => f.key !== 'src'))){f.key;;__sveltets_2_ensureSnippet(field(f));}
       }
    }else{
       { svelteHTML.createElement("div", { "class":`group`,});
         { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`dim small`,});    } }
         { svelteHTML.createElement("div", { "class":`fields`,});
             for(let f of __sveltets_2_ensureArray(bdef.fields)){f.key;;__sveltets_2_ensureSnippet(field(f));}
         }
       }
    }
  } else if (section && def){
    
     { svelteHTML.createElement("div", { "class":`group`,});
       { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  } }
       { svelteHTML.createElement("div", { "class":`ident`,});
         { svelteHTML.createElement("span", { "class":`ident-icon`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":section.block === 'media/3d-embed' ? 'box' : 'layout',"size":14,}});} }
         { svelteHTML.createElement("div", {    "class":`ident-text`,"tabindex":-1,...__sveltets_2_empty({"data-view-head":true}),}); { svelteHTML.createElement("strong", {});sectionLabel(section); } { svelteHTML.createElement("span", { "class":`dim`,});def.label.split(' · ')[1] ?? def.description; } }
        if(layoutsOf(section.block).length > 1){
           { svelteHTML.createElement("button", {         "class":`btn small`,"onclick":swapMenu,"disabled":readonly,"aria-haspopup":`menu`,...__sveltets_2_empty({"data-testid":`site-swap-layout`}),});   { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`chevronDown`,"size":12,}});} }
        }
       }
      if(section.derived_from){
        const d = ops.parseDerived(section.derived_from);
         { svelteHTML.createElement("p", { "class":`prov`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`link`,"size":12,}});}   d?.seq ? ` · v${d.seq}` : '';
          if(d){ { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => onopen(d.artifactId),});  }}
         }
      }
     }

    if(def.fields.length){
       { svelteHTML.createElement("div", { "class":`group`,});
         { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`dim small`,});    } }
         { svelteHTML.createElement("div", { "class":`fields`,});
             for(let f of __sveltets_2_ensureArray(def.fields)){f.key;;__sveltets_2_ensureSnippet(field(f));}
         }
       }
    }

    if(def.item){
      const items = (section.blocks ?? []).filter((b) => b.block === def!.item);
      const idef = itemDef(def.item);
       { svelteHTML.createElement("div", { "class":`group`,});
         { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});idef?.label ?? 'Items';  items.length; }
          if(!readonly){ { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":() => { const r = ops.addItem(doc, section!.id); onchange(r.doc); if (r.id) void go(section!.id, r.id); },...__sveltets_2_empty({"data-testid":`site-add-item`}),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`plus`,"size":12,}});}  }}
         }
         { svelteHTML.createElement("ul", { "class":`items`,});
             for(let b of __sveltets_2_ensureArray(items)){b.id;
             { svelteHTML.createElement("li", {}); { svelteHTML.createElement("button", {     "class":`item`,...__sveltets_2_empty({"data-item":b.id}),"onclick":() => void go(section!.id, b.id),}); { svelteHTML.createElement("span", { "class":`name`,});itemTitle(b); } { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});} } }
          }
         }
       }
    }

    if(def.media){
       { svelteHTML.createElement("div", { "class":`group`,});
         { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  } }
        if(mediaChild){
           { svelteHTML.createElement("button", {     "class":`item`,...__sveltets_2_empty({"data-item":mediaChild.id}),"onclick":() => void go(section!.id, mediaChild!.id),});
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":mediaChild.block === 'embed/3d' ? 'box' : 'image',"size":12,}});}
             { svelteHTML.createElement("span", { "class":`name`,});mediaChild.block === 'embed/3d' ? `3D · ${embed(str(mediaChild.props, 'src'))?.title ?? (str(mediaChild.props, 'label') || 'stand-in card')}` : `Image · ${str(mediaChild.props, 'alt') || 'untitled'}`; }
             { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`chevronRight`,"size":12,}});}
           }
        }
        if(!readonly){
           { svelteHTML.createElement("div", { "class":`row-btns`,});
            if(def.media.includes('embed/3d')){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":(e) => addMedia('embed/3d', e),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`box`,"size":12,}});} mediaChild?.block === 'embed/3d' ? 'Replace 3D…' : 'Use a 3D artifact…'; }}
            if(def.media.includes('embed/image')){ { svelteHTML.createElement("button", {   "class":`btn small`,"onclick":(e) => addMedia('embed/image', e),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`image`,"size":12,}});}    }}
           }
        }
       }
    }

     { svelteHTML.createElement("div", { "class":`group`,});
       { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  } { svelteHTML.createElement("span", { "class":`dim small from`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`palette`,"size":12,}});} kitLabel ? `from ${kitLabel}` : 'default palette'; } }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("span", { "class":`lbl`,});  }
         { svelteHTML.createElement("div", {     "class":`swatches`,"role":`group`,"aria-label":`Background`,});
           { svelteHTML.createElement("button", {            "class":`sw none`,"aria-pressed":!section.style?.background,"onclick":() => setBg(''),"disabled":readonly,"aria-label":`Default background`,"title":`Default (surface)`,});!section.style?.background; }
             for(let s of __sveltets_2_ensureArray(brandSwatches)){s.value;
             { svelteHTML.createElement("button", {             "class":`sw`,"aria-pressed":section.style?.background === s.value,"onclick":() => setBg(s.value),"disabled":readonly,"aria-label":`Brand ${s.label}`,"title":`${s.label} (${s.color})`,});section.style?.background === s.value;__sveltets_2_ensureType(String, Number, s.color); }
          }
             for(let g of __sveltets_2_ensureArray(GRADIENTS)){g;
             { svelteHTML.createElement("button", {                "class":`sw grad`,...__sveltets_2_empty({"data-g":g}),"aria-pressed":section.style?.background === `gradient:${g}`,"onclick":() => setBg(`gradient:${g}`),"disabled":readonly,"aria-label":`Gradient ${g}`,"title":`Gradient · ${g}`,});section.style?.background === `gradient:${g}`;__sveltets_2_ensureType(String, Number, g === 'ink' ? theme.night : g === 'soft' ? theme.surfaceAlt : theme.primary);__sveltets_2_ensureType(String, Number, g === 'soft' ? theme.primary : theme.accent); }
          }
           { svelteHTML.createElement("input", {              "dir":`auto`,"class":`input hex`,"placeholder":`# hex`,"bind:value":hexDraft,"disabled":readonly,"aria-label":`Custom color (hex)`,"onchange":commitHex,});/*Ωignore_startΩ*/() => hexDraft = __sveltets_2_any(null);/*Ωignore_endΩ*/}
         }
        if(bg?.offBrand){
           { svelteHTML.createElement("p", { "class":`warnchip`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}      kitLabel ?? 'the palette';
            if(nearest && !readonly){ { svelteHTML.createElement("button", {   "class":`linkbtn`,"onclick":() => setBg(nearest!.value),}); nearest.label; }}
           }
        } else if (bg?.unknownToken){
           {const $$action_0 = __sveltets_2_ensureAction(announceShown(svelteHTML.mapElementTag('p'),(`The brand kit has no “${bg.unknownToken}”. Showing the default.`)));{ svelteHTML.createElement("p", __sveltets_2_union($$action_0), {   "class":`warnchip`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`warning`,"size":12,}});}      bg.unknownToken;     }}
        }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("span", { "class":`lbl`,});  }
         { svelteHTML.createElement("div", {     "class":`segmented full`,"role":`group`,"aria-label":`Spacing`,});
             for(let sp of __sveltets_2_ensureArray((['s', 'm', 'l', 'xl']))){sp;
             { svelteHTML.createElement("button", {       "aria-pressed":(section.style?.spacing || 'm') === sp,"disabled":readonly,"onclick":() => onchange(ops.setStyle(doc, section!.id, 'spacing', sp as 's')),});(section.style?.spacing || 'm') === sp;sp.toUpperCase(); }
          }
         }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("span", { "class":`lbl`,});  }
         { svelteHTML.createElement("div", {     "class":`segmented full`,"role":`group`,"aria-label":`Alignment`,});
             for(let al of __sveltets_2_ensureArray((['left', 'center']))){al;
             { svelteHTML.createElement("button", {       "aria-pressed":(section.style?.align || 'left') === al,"disabled":readonly,"onclick":() => onchange(ops.setStyle(doc, section!.id, 'align', al as 'left')),});(section.style?.align || 'left') === al;al === 'left' ? 'Left' : 'Center'; }
          }
         }
       }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`si-motion`,});  }
         { svelteHTML.createElement("select", {           "id":`si-motion`,"class":`input`,"value":section.style?.motion || 'none',"disabled":readonly,"onchange":(e) => onchange(ops.setStyle(doc, section!.id, 'motion', e.currentTarget.value as SiteMotion)),...__sveltets_2_empty({"data-testid":`site-motion`}),});
             for(let m of __sveltets_2_ensureArray(MOTIONS)){m.value; { svelteHTML.createElement("option", { "value":m.value,});m.label; }}
         }
         { svelteHTML.createElement("span", { "class":`hint`,});motionHint;        }
       }
      if(section.block.startsWith('hero/')){
         { svelteHTML.createElement("div", { "class":`field`,});
           { svelteHTML.createElement("span", { "class":`lbl`,});  }
           { svelteHTML.createElement("div", {     "class":`segmented full`,"role":`group`,"aria-label":`Min height`,});
               for(let mh of __sveltets_2_ensureArray((['auto', '80vh', '100vh']))){mh;
               { svelteHTML.createElement("button", {       "aria-pressed":(section.style?.min_height || 'auto') === mh,"disabled":readonly,"onclick":() => onchange(ops.setStyle(doc, section!.id, 'min_height', mh as 'auto')),});(section.style?.min_height || 'auto') === mh;mh === 'auto' ? 'Auto' : mh; }
            }
           }
         }
      }
     }

     { svelteHTML.createElement("div", { "class":`group`,});
       { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  } }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("span", { "class":`lbl`,});  }
         { svelteHTML.createElement("div", { "class":`pills`,});
             for(let bp of __sveltets_2_ensureArray((['desktop', 'tablet', 'mobile']))){bp;
            const hidden = (section.responsive?.hide ?? []).includes(bp as 'mobile');
             { svelteHTML.createElement("button", {        "class":`pill-toggle`,"aria-pressed":!hidden,"disabled":readonly,"onclick":() => onchange(ops.toggleHide(doc, section!.id, bp as 'mobile')),});!hidden;
               { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":hidden ? 'eyeOff' : 'eye',"size":12,}});} bp[0].toUpperCase() + bp.slice(1);
             }
          }
         }
       }
      if(def.media || section.block === 'features/alternating'){
         { svelteHTML.createElement("div", { "class":`switch-row`,});
           { svelteHTML.createElement("span", {});        }
           { const $$_hctiwS3C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS3C({ target: __sveltets_2_any(), props: {         "checked":section.responsive?.stack === 'media-first',"disabled":readonly,"onchange":(next) => onchange(ops.setResponsive(doc, section!.id, 'stack', next ? 'media-first' : undefined)),"label":`Media above the text on mobile`,}});}
         }
      }
       { svelteHTML.createElement("div", { "class":`switch-row`,});
         { svelteHTML.createElement("span", {});     }
         { const $$_hctiwS3C = __sveltets_2_ensureComponent(Switch); new $$_hctiwS3C({ target: __sveltets_2_any(), props: {         "checked":section.responsive?.mobile_align === 'center',"disabled":readonly,"onchange":(next) => onchange(ops.setResponsive(doc, section!.id, 'mobile_align', next ? 'center' : undefined)),"label":`Center the text on mobile`,}});}
       }
     }

    if(contrastNow){
       { svelteHTML.createElement("div", { "class":`group`,});
         { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  } }
         { svelteHTML.createElement("div", { "class":`pills`,});
           { svelteHTML.createElement("span", {     "class":`chip`,"title":`Body text on this background (WCAG AA needs 4.5:1)`,});(contrastNow.text ?? 0) >= 4.5;(contrastNow.text ?? 0) < 4.5;
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":(contrastNow.text ?? 0) >= 4.5 ? 'check' : 'warning',"size":12,}});}  contrastNow.text ?? '–';
           }
           { svelteHTML.createElement("span", {     "class":`chip`,"title":`Button label on its fill (WCAG AA needs 4.5:1)`,});(contrastNow.button ?? 0) >= 4.5;(contrastNow.button ?? 0) < 4.5;
             { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":(contrastNow.button ?? 0) >= 4.5 ? 'check' : 'warning',"size":12,}});}  contrastNow.button ?? '–';
           }
         }
       }
    }

    if(!readonly){
       { svelteHTML.createElement("div", { "class":`group end`,});
         { svelteHTML.createElement("button", {     "class":`btn small danger`,"onclick":() => void deleteSection(),...__sveltets_2_empty({"data-testid":`site-delete-section`}),}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`trash`,"size":12,}});}   }
       }
    }
  } else if (page){
    
     { svelteHTML.createElement("div", { "class":`group`,});
       { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", {    "class":`k`,"tabindex":-1,...__sveltets_2_empty({"data-view-head":true}),});  } }
       { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`sp-title`,});  } { svelteHTML.createElement("input", {            "dir":`auto`,"id":`sp-title`,"class":`input`,"value":page.title,"disabled":readonly,"oninput":(e) => setPage('title', e.currentTarget.value),});} }
       { svelteHTML.createElement("div", { "class":`field`,});
         { svelteHTML.createElement("label", { "for":`sp-slug`,});  }
        if(page === doc.pages[0]){
            { svelteHTML.createElement("input", {        "dir":`ltr`,"id":`sp-slug`,"class":`input`,"value":`/ (home)`,"disabled":true,});}
        }else{
           { svelteHTML.createElement("input", {            "dir":`ltr`,"id":`sp-slug`,"class":`input`,"value":page.slug,"disabled":readonly,"onchange":(e) => setPage('slug', e.currentTarget.value),});}
        }
       }
       { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`sp-desc`,});   } { svelteHTML.createElement("textarea", {             "dir":`auto`,"id":`sp-desc`,"class":`input`,"rows":2,"value":page.description ?? '',"disabled":readonly,"oninput":(e) => setPage('description', e.currentTarget.value),}); } }
     }
     { svelteHTML.createElement("div", { "class":`group`,});
       { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  } }
       { svelteHTML.createElement("div", { "class":`field`,}); { svelteHTML.createElement("label", { "for":`ss-title`,});  } { svelteHTML.createElement("input", {            "dir":`auto`,"id":`ss-title`,"class":`input`,"value":doc.title ?? '',"disabled":readonly,"oninput":(e) => setSite('title', e.currentTarget.value),});} }
       { svelteHTML.createElement("div", { "class":`field half`,}); { svelteHTML.createElement("label", { "for":`ss-domain`,});  } { svelteHTML.createElement("input", {              "dir":`auto`,"id":`ss-domain`,"class":`input`,"value":doc.settings?.domain ?? '',"placeholder":`example.com`,"disabled":readonly,"oninput":(e) => setSite('domain', e.currentTarget.value),});} }
       { svelteHTML.createElement("div", { "class":`field half`,}); { svelteHTML.createElement("label", { "for":`ss-lang`,});  } { svelteHTML.createElement("input", {            "dir":`auto`,"id":`ss-lang`,"class":`input`,"value":doc.settings?.lang ?? 'en',"disabled":readonly,"oninput":(e) => setSite('lang', e.currentTarget.value),});} }
     }
     { svelteHTML.createElement("div", { "class":`group`,});
       { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  }
        if(!readonly){ { svelteHTML.createElement("button", {     "class":`btn small ghost`,"onclick":(e) => void linkKit(e),...__sveltets_2_empty({"data-testid":`site-link-kit`}),}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`palette`,"size":12,}});} kitLabel ? 'Change…' : 'Link a brand kit…'; }}
       }
      if(brand.error){
         { const $$_etatSdaoL2C = __sveltets_2_ensureComponent(LoadState); new $$_etatSdaoL2C({ target: __sveltets_2_any(), props: {        "variant":`compact`,"what":`the brand kit`,"empty":true,"error":`The preview uses the default palette. ${brand.error}`,"onretry":onretrybrand,}});}
      }else{
         { svelteHTML.createElement("p", { "class":`kit`,}); { svelteHTML.createElement("strong", {});kitLabel ?? 'Default palette'; }if(brand.note){ { svelteHTML.createElement("span", { "class":`dim`,});  brand.note; }} }
      }
       { svelteHTML.createElement("div", {   "class":`kit-sw`,"aria-hidden":`true`,});
            for(let c of __sveltets_2_ensureArray(([theme.primary, theme.accent, theme.ink, theme.surface, theme.surfaceAlt]))){let i = 1;i; { svelteHTML.createElement("span", { });__sveltets_2_ensureType(String, Number, c); }}
       }
     }
     { svelteHTML.createElement("div", { "class":`group`,});
       { svelteHTML.createElement("div", { "class":`gh`,}); { svelteHTML.createElement("span", { "class":`k`,});  }
         { svelteHTML.createElement("span", { "class":`dim small`,});errors ? `${errors} to fix` : 'No blockers';warns ? ` · ${plural(warns, 'warning')}` : ''; }
       }
      if(findings.length){
         { svelteHTML.createElement("ul", { "class":`findings`,});
             for(let f of __sveltets_2_ensureArray(findings)){f.key;
             { svelteHTML.createElement("li", {});
               { svelteHTML.createElement("button", {       "class":`finding`,...__sveltets_2_empty({"data-level":f.level}),"onclick":() => f.sectionId && void go(f.sectionId, f.blockId),"disabled":!f.sectionId,});
                 { const $$_nocI5C = __sveltets_2_ensureComponent(Icon); new $$_nocI5C({ target: __sveltets_2_any(), props: {    "name":f.level === 'error' ? 'warning' : 'info',"size":12,}});}
                 { svelteHTML.createElement("span", {});f.message; }
               }
             }
          }
         }
      }else{
         { svelteHTML.createElement("p", { "class":`dim note`,}); { const $$_nocI3C = __sveltets_2_ensureComponent(Icon); new $$_nocI3C({ target: __sveltets_2_any(), props: {    "name":`check`,"size":12,}});}        }
      }
     }
  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings(''), slots: {}, events: {} }}
const Inspector__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type Inspector__SvelteComponent_ = ReturnType<typeof Inspector__SvelteComponent_>;
/*Ωignore_endΩ*/export default Inspector__SvelteComponent_;