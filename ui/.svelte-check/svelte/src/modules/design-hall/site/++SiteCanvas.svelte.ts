///<reference types="svelte" />
;
  export type SiteDevice = 'desktop' | 'tablet' | 'mobile';
  export const DEVICE_WIDTH: Record<SiteDevice, number> = { desktop: 1280, tablet: 834, mobile: 390 };
  export type CanvasAction = 'up' | 'down' | 'duplicate' | 'hide-mobile' | 'ask' | 'variants' | 'more';
  /** Drag payload MIME for library blocks (`<block>` or `lib:<n>`). */
  export const BLOCK_MIME = 'application/x-otto-site-block';
;;// The Site Studio canvas: the page rendered as REAL DOM (the same markup +
// site.css the export ships) inside a device frame, scaled to the chosen zoom.
// Sections are rendered per section with {@html} so an edit re-renders only
// what changed. Selection, hover, inline editing (double-click text) and
// library drops are handled by delegated listeners on the page root; the
// outlines, name tags, insertion line and the floating section toolbar live
// in an overlay measured from the DOM (clamped into the canvas, never off it).


import { scrollBehavior } from '../../../lib/motion';
import { tick, untrack } from 'svelte';
import Icon from '../../../lib/components/Icon.svelte';
import { flag, renderSection, str, type EmbedInfo } from './engine/render';
import { themeStyle, type Theme } from './engine/theme';
import { sectionLabel } from './engine/catalog';
import type { SiteDoc, SitePage } from './engine/types';
;

  interface Props {
    doc: SiteDoc;
    page: SitePage;
    theme: Theme;
    device: SiteDevice;
    /** `fit` or a percentage (50 / 75 / 100). */
    zoom: 'fit' | number;
    /** Freeze motion presets while editing (off = play them). */
    still: boolean;
    selectedSection: string | null;
    selectedBlock: string | null;
    readonly: boolean;
    /** Mount live 3D (the 3D Studio embed runtime) over 3D embeds. */
    live3d?: boolean;
    domain: string;
    embed: (uri: string) => EmbedInfo | null;
    asset: (uri: string) => string | null;
    /** Effective scale (read by the toolbar's zoom label). */
    scale?: number;
    onselect: (sectionId: string | null, blockId: string | null) => void;
    onedit: (sectionId: string, blockId: string | null, key: string, value: string) => void;
    onaction: (action: CanvasAction, sectionId: string, e: MouseEvent) => void;
    ondrop: (payload: string, index: number) => void;
  };;

  // While a text is being edited inline, its section's HTML is frozen so a
  // re-render never clobbers the caret; the commit re-renders it.
  interface Editing {
    sectionId: string;
    blockId: string | null;
    key: string;
    el: HTMLElement;
    original: string;
    originalHtml: string;
    frozen: string;
  };;

  // ── Overlay geometry (in host coordinates) ──────────────────────────────────
  interface Box {
    x: number;
    y: number;
    w: number;
    h: number;
  };;

  // ── Live 3D ────────────────────────────────────────────────────────────────
  // Each 3D embed with a source gets the 3D Studio's embed runtime, mounted in
  // a layer INSIDE the scaled frame but OUTSIDE the {@html} sections — so a
  // re-render of the section (every edit) never tears the WebGL scene down —
  // and positioned over the embed's stage. It is transparent (the stand-in
  // orb stays behind the model) and click-through (selection still works).
  // The stand-in card / poster hide once the first frame is up; on failure the
  // stand-in simply stays.
  interface Live {
    key: string;
    src: string;
    host: HTMLDivElement;
    ready: boolean;
    handle: { destroy(): void } | null;
  };function $$render() {

  
  
  
  
  
  
  
  
  
  
  
  
  
  

  let {
    doc,
    page,
    theme,
    device,
    zoom,
    still,
    selectedSection,
    selectedBlock,
    readonly,
    live3d = true,
    domain,
    embed,
    asset,
    scale = $bindable(1),
    onselect,
    onedit,
    onaction,
    ondrop,
  }: Props = $props()/*Ωignore_startΩ*/;scale;/*Ωignore_endΩ*/;

  const W = $derived(DEVICE_WIDTH[device]);
  let hostEl = $state<HTMLDivElement | null>(null);
  let scrollerEl = $state<HTMLDivElement | null>(null);
  let siteEl = $state<HTMLDivElement | null>(null);
  let viewW = $state(0);
  let viewH = $state(0);
  let frameH = $state(0);
  const z = $derived(zoom === 'fit' ? Math.max(0.2, Math.min(1, (viewW - 64) / W)) : zoom / 100);
  $effect(() => {
    scale = z;
  });

  // ── Rendering ──────────────────────────────────────────────────────────────
  /** Sources with a live 3D scene up (the renderer then hides their stand-in). */
  let liveSrcs = $state<ReadonlySet<string>>(new Set());
  const ctx = $derived.by(() => {
    const on = liveSrcs;
    return {
      theme,
      editable: true,
      homeHref: '#',
      pageHref: () => '#',
      embed: (uri: string): EmbedInfo | null => {
        const i = embed(uri);
        return i && on.has(uri.trim()) ? { ...i, live: true } : i;
      },
      asset,
    };
  });
  // Section HTML memoized by section OBJECT (SD-17): ops path-copy, so an
  // untouched section keeps its identity across edits and is not re-rendered;
  // a new render context (theme, live 3D) invalidates everything.
  let renderMemo = new WeakMap<object, string>();
  let memoCtx: unknown = null;
  const html = $derived.by(() => {
    const c = ctx;
    if (c !== memoCtx) {
      renderMemo = new WeakMap();
      memoCtx = c;
    }
    return new Map(
      page.sections.map((s) => {
        let out = renderMemo.get(s);
        if (out === undefined) {
          out = renderSection(s, c);
          renderMemo.set(s, out);
        }
        return [s.id, out];
      }),
    );
  });
  const style = $derived(themeStyle(theme));


  /**
   * The text of an edited element as authored — NOT `innerText`, which
   * applies CSS (`text-transform: uppercase` on eyebrows would save capitals).
   * `<br>` is a line break, a paragraph a blank line.
   */
  function readText(el: HTMLElement): string {
    let out = '';
    const walk = (n: Node) => {
      for (const c of Array.from(n.childNodes)) {
        if (c.nodeType === Node.TEXT_NODE) out += c.textContent ?? '';
        else if (c.nodeName === 'BR') out += '\n';
        else {
          if (c.nodeName === 'P' && out.trim()) out = out.replace(/\n*$/, '') + '\n\n';
          else if (c.nodeName === 'DIV' && out && !out.endsWith('\n')) out += '\n';
          walk(c);
        }
      }
    };
    walk(el);
    return out;
  }
  let editing = $state<Editing | null>(null);
  function shown(id: string): string {
    return editing?.sectionId === id ? editing.frozen : (html.get(id) ?? '');
  }

  const MULTILINE = new Set(['headline', 'subhead', 'heading', 'subheading', 'body', 'quote', 'answer', 'blurb', 'tagline']);

  function startEdit(el: HTMLElement, sectionId: string, blockId: string | null, key: string): void {
    if (readonly || editing) return;
    editing = { sectionId, blockId, key, el, original: readText(el), originalHtml: el.innerHTML, frozen: html.get(sectionId) ?? '' };
    try {
      el.contentEditable = 'plaintext-only';
    } catch {
      el.contentEditable = 'true';
    }
    if (el.contentEditable !== 'plaintext-only') el.contentEditable = 'true';
    el.classList.add('os-editing');
    el.focus();
    const range = document.createRange();
    range.selectNodeContents(el);
    const s = window.getSelection();
    s?.removeAllRanges();
    s?.addRange(range);
    el.addEventListener('keydown', onEditKey);
    el.addEventListener('blur', onEditBlur, { once: true });
  }

  function finishEdit(commit: boolean): void {
    const ed = editing;
    if (!ed) return;
    ed.el.removeEventListener('keydown', onEditKey);
    ed.el.removeEventListener('blur', onEditBlur);
    ed.el.removeAttribute('contenteditable');
    ed.el.classList.remove('os-editing');
    let value = readText(ed.el).replace(/ /g, ' ').replace(/\n+$/, '');
    if (!MULTILINE.has(ed.key)) value = value.replace(/\s*\n\s*/g, ' ');
    if (!commit || value === ed.original) {
      // Nothing changes in the document, so nothing re-renders: put the
      // authored markup back by hand.
      ed.el.innerHTML = ed.originalHtml;
      editing = null;
      return;
    }
    editing = null;
    onedit(ed.sectionId, ed.blockId, ed.key, value);
  }

  function onEditKey(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      finishEdit(false);
    } else if (e.key === 'Enter' && (e.metaKey || e.ctrlKey || (!e.shiftKey && !MULTILINE.has(editing?.key ?? '')))) {
      e.preventDefault();
      finishEdit(true);
    }
  }
  function onEditBlur(): void {
    finishEdit(true);
  }

  // ── Delegated pointer handling ─────────────────────────────────────────────
  let hover = $state<{ section: string; block: string | null } | null>(null);

  function hit(t: EventTarget | null): { section: string | null; block: string | null; edit: HTMLElement | null } {
    const el = t instanceof Element ? t : null;
    const sec = el?.closest<HTMLElement>('[data-os-section]') ?? null;
    const blk = el?.closest<HTMLElement>('[data-os-block]') ?? null;
    const edit = el?.closest<HTMLElement>('[data-os-edit]') ?? null;
    return { section: sec?.dataset.osSection ?? null, block: blk?.dataset.osBlock ?? null, edit };
  }

  $effect(() => {
    const el = siteEl;
    if (!el) return;
    const onMove = (e: MouseEvent) => {
      const h = hit(e.target);
      const next = h.section ? { section: h.section, block: h.block } : null;
      if (next?.section !== hover?.section || next?.block !== hover?.block) hover = next;
    };
    const onLeave = () => (hover = null);
    const onClick = (e: MouseEvent) => {
      const t = e.target instanceof Element ? e.target : null;
      if (editing && t && editing.el.contains(t)) return;
      // Links and buttons never navigate or submit inside the editor
      // (a FAQ <summary> still toggles).
      if (t?.closest('a, button, input')) e.preventDefault();
      const h = hit(t);
      if (!h.section) {
        onselect(null, null);
        return;
      }
      if (h.section === untrack(() => selectedSection) && h.block) onselect(h.section, h.block);
      else onselect(h.section, null);
    };
    const onDbl = (e: MouseEvent) => {
      const h = hit(e.target);
      if (!h.section || !h.edit || readonly) return;
      e.preventDefault();
      onselect(h.section, h.block);
      void tick().then(() => {
        // The select may have re-rendered; find the same field again.
        const fresh = siteEl?.querySelector<HTMLElement>(
          h.block
            ? `[data-os-block="${CSS.escape(h.block)}"] [data-os-edit="${CSS.escape(h.edit!.dataset.osEdit ?? '')}"], [data-os-block="${CSS.escape(h.block)}"][data-os-edit="${CSS.escape(h.edit!.dataset.osEdit ?? '')}"]`
            : `[data-os-section="${CSS.escape(h.section!)}"] [data-os-edit="${CSS.escape(h.edit!.dataset.osEdit ?? '')}"]`,
        );
        const target = h.edit!.isConnected ? h.edit! : fresh;
        if (target) startEdit(target, h.section!, h.block, target.dataset.osEdit ?? '');
      });
    };
    const onSubmit = (e: Event) => e.preventDefault();
    el.addEventListener('mousemove', onMove);
    el.addEventListener('mouseleave', onLeave);
    el.addEventListener('click', onClick);
    el.addEventListener('dblclick', onDbl);
    el.addEventListener('submit', onSubmit);
    return () => {
      el.removeEventListener('mousemove', onMove);
      el.removeEventListener('mouseleave', onLeave);
      el.removeEventListener('click', onClick);
      el.removeEventListener('dblclick', onDbl);
      el.removeEventListener('submit', onSubmit);
    };
  });

  let selBox = $state<Box | null>(null);
  let blockBox = $state<Box | null>(null);
  let hoverBox = $state<Box | null>(null);
  let dropY = $state<number | null>(null);
  let tbW = $state(0);
  let tbH = $state(0);

  function boxOf(el: Element | null | undefined): Box | null {
    if (!el || !hostEl) return null;
    const h = hostEl.getBoundingClientRect();
    const r = el.getBoundingClientRect();
    if (r.width === 0 && r.height === 0) return null;
    return { x: r.left - h.left, y: r.top - h.top, w: r.width, h: r.height };
  }
  function sectionEl(id: string | null): HTMLElement | null {
    return id && siteEl ? siteEl.querySelector<HTMLElement>(`[data-os-section="${CSS.escape(id)}"]`) : null;
  }
  function blockEl(id: string | null): HTMLElement | null {
    return id && siteEl ? siteEl.querySelector<HTMLElement>(`[data-os-block="${CSS.escape(id)}"]`) : null;
  }

  let raf = 0;
  function measure(): void {
    cancelAnimationFrame(raf);
    raf = requestAnimationFrame(() => {
      selBox = boxOf(sectionEl(selectedSection));
      blockBox = selectedBlock ? boxOf(blockEl(selectedBlock)) : null;
      hoverBox = hover && (hover.section !== selectedSection || hover.block) ? boxOf(hover.block && hover.section === selectedSection ? blockEl(hover.block) : sectionEl(hover.section)) : null;
      syncLive();
    });
  }

  /** Publish which sources have a scene up (only when that changes — it re-renders). */
  function refreshLiveSrcs(): void {
    const next = new Set([...lives.values()].filter((l) => l.ready).map((l) => l.src));
    if (next.size === liveSrcs.size && [...next].every((s) => liveSrcs.has(s))) return;
    liveSrcs = next;
  }
  const lives = new Map<string, Live>();
  let frameEl = $state<HTMLDivElement | null>(null);
  let liveLayer = $state<HTMLDivElement | null>(null);

  function syncLive(): void {
    if (!live3d || !siteEl || !frameEl || !liveLayer) return;
    const frame = frameEl.getBoundingClientRect();
    const seen = new Set<string>();
    for (const fig of Array.from(siteEl.querySelectorAll<HTMLElement>('figure.os-embed-3d[data-os-block]'))) {
      const id = fig.dataset.osBlock ?? '';
      const b = page.sections.flatMap((s) => s.blocks ?? []).find((x) => x.id === id);
      const src = b ? str(b.props, 'src').trim() : '';
      const stage = fig.querySelector<HTMLElement>('.os-embed-3d__stage');
      if (!b || !src.startsWith('otto://design/') || !stage) continue;
      seen.add(id);
      const key = `${src}|${flag(b.props, 'auto_rotate', true)}`;
      let l = lives.get(id);
      if (l && l.key !== key) {
        l.handle?.destroy();
        l.host.remove();
        lives.delete(id);
        l = undefined;
      }
      if (!l) {
        const host = document.createElement('div');
        host.className = 'live3d';
        liveLayer.appendChild(host);
        const entry: Live = { key, src, host, ready: false, handle: null };
        lives.set(id, entry);
        const label = str(b.props, 'alt') || '3D model';
        void import('../studio3d/embed')
          .then((m) => m.mountScene3dEmbed(host, { src, autoRotate: flag(b.props, 'auto_rotate', true), drag: false, tiltOnHover: false, background: 'transparent', label }))
          .then(async (h) => {
            if (lives.get(id) !== entry) return h.destroy();
            entry.handle = h;
            await h.ready;
            if (lives.get(id) !== entry) return;
            entry.ready = true;
            refreshLiveSrcs();
            measure();
          })
          .catch(() => {
            // The stand-in stays; don't retry this key.
            host.remove();
            entry.handle = { destroy() {} };
          });
        l = entry;
      }
      const r = stage.getBoundingClientRect();
      l.host.style.left = `${(r.left - frame.left) / z}px`;
      l.host.style.top = `${(r.top - frame.top) / z}px`;
      l.host.style.width = `${r.width / z}px`;
      l.host.style.height = `${r.height / z}px`;
      l.host.style.display = r.width > 0 ? '' : 'none';
    }
    for (const [id, l] of lives) {
      if (seen.has(id)) continue;
      l.handle?.destroy();
      l.host.remove();
      lives.delete(id);
    }
    refreshLiveSrcs();
  }
  $effect(() => () => {
    for (const l of lives.values()) {
      l.handle?.destroy();
      l.host.remove();
    }
    lives.clear();
  });
  $effect(() => {
    // Re-measure on anything that moves the page.
    void html;
    void selectedSection;
    void selectedBlock;
    void hover;
    void z;
    void device;
    void viewW;
    void viewH;
    void frameH;
    void tick().then(measure);
  });
  $effect(() => () => cancelAnimationFrame(raf));

  // Selecting from Layers scrolls the section into view.
  let lastScrolled: string | null = null;
  $effect(() => {
    const id = selectedSection;
    if (!id || id === lastScrolled) return;
    lastScrolled = id;
    void tick().then(() => {
      const el = sectionEl(id);
      const sc = scrollerEl;
      if (!el || !sc) return;
      const r = el.getBoundingClientRect();
      const s = sc.getBoundingClientRect();
      if (r.top < s.top + 40 || r.top > s.bottom - 80) sc.scrollBy({ top: r.top - s.top - 48, behavior: scrollBehavior() });
    });
  });

  // ── Drops from the block library ───────────────────────────────────────────
  function dropIndex(clientY: number): { index: number; y: number } {
    const secs = siteEl ? Array.from(siteEl.querySelectorAll<HTMLElement>('[data-os-section]')) : [];
    const h = hostEl?.getBoundingClientRect();
    for (let i = 0; i < secs.length; i++) {
      const r = secs[i].getBoundingClientRect();
      if (clientY < r.top + r.height / 2) return { index: i, y: r.top - (h?.top ?? 0) };
    }
    const last = secs[secs.length - 1]?.getBoundingClientRect();
    return { index: secs.length, y: last ? last.bottom - (h?.top ?? 0) : 24 };
  }
  function onDragOver(e: DragEvent): void {
    if (readonly || !e.dataTransfer?.types.includes(BLOCK_MIME)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = 'copy';
    dropY = dropIndex(e.clientY).y;
  }
  function onDrop(e: DragEvent): void {
    const data = e.dataTransfer?.getData(BLOCK_MIME);
    dropY = null;
    if (readonly || !data) return;
    e.preventDefault();
    ondrop(data, dropIndex(e.clientY).index);
  }

  // ── Floating toolbar placement (clamped into the canvas) ────────────────────
  const tb = $derived.by(() => {
    if (!selBox || editing || readonly) return null;
    if (selBox.y + selBox.h < 24 || selBox.y > viewH - 24) return null;
    const y = Math.max(8, Math.min(viewH - tbH - 8, selBox.y - tbH - 8));
    const x = Math.max(8, Math.min(viewW - tbW - 8, selBox.x + selBox.w - tbW));
    return { x, y };
  });
  const selName = $derived.by(() => {
    const s = page.sections.find((x) => x.id === selectedSection);
    return s ? sectionLabel(s) : '';
  });
  const hoverName = $derived.by(() => {
    if (!hover) return '';
    const s = page.sections.find((x) => x.id === hover!.section);
    return s ? sectionLabel(s) : '';
  });
  const selHiddenMobile = $derived(!!page.sections.find((x) => x.id === selectedSection)?.responsive?.hide?.includes('mobile'));
  const url = $derived(`${domain}${page.slug ? `/${page.slug}` : ''}`);
;
async () => {



 { const $$_div0 = svelteHTML.createElement("div", {      "class":`host`,...__sveltets_2_empty({"data-testid":`site-canvas`}),});hostEl = $$_div0;viewW= $$_div0.clientWidth;viewH= $$_div0.clientHeight;
   { const $$_div1 = svelteHTML.createElement("div", {              "class":`scroller`,"onscroll":measure,"ondragover":onDragOver,"ondragleave":() => (dropY = null),"ondrop":onDrop,"role":`presentation`,});scrollerEl = $$_div1;
     { svelteHTML.createElement("div", {   "class":`stage`,});__sveltets_2_ensureType(String, Number, `${Math.ceil(W * z)}px`);__sveltets_2_ensureType(String, Number, `${Math.ceil(frameH * z)}px`);
       { const $$_div3 = svelteHTML.createElement("div", {      "class":`frame`,});device === 'mobile';__sveltets_2_ensureType(String, Number, `${W}px`);__sveltets_2_ensureType(String, Number, `scale(${z})`);frameH= $$_div3.offsetHeight;frameEl = $$_div3;
         { svelteHTML.createElement("div", {   "class":`chrome`,"aria-hidden":`true`,});
           { svelteHTML.createElement("span", { "class":`dots`,}); { svelteHTML.createElement("i", {}); } { svelteHTML.createElement("i", {}); } { svelteHTML.createElement("i", {}); } }
           { svelteHTML.createElement("span", { "class":`url`,}); { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`lock`,"size":12,}});} url; }
         }
         { const $$_div4 = svelteHTML.createElement("div", {       "class":`os-site os-edit`,style,...__sveltets_2_empty({"data-testid":`site-page`}),});still;__sveltets_2_ensureType(String, Number, z);siteEl = $$_div4;
             for(let s of __sveltets_2_ensureArray(page.sections)){s.id;
             { svelteHTML.createElement("div", { "class":`os-slot`,}); shown(s.id); }
          }
             { svelteHTML.createElement("div", { "class":`empty-page`,});
               { const $$_nocI6C = __sveltets_2_ensureComponent(Icon); new $$_nocI6C({ target: __sveltets_2_any(), props: {    "name":`layers`,"size":16,}});}
               { svelteHTML.createElement("span", {});          { svelteHTML.createElement("strong", {});  }        }
             }
          
         }
         { const $$_div4 = svelteHTML.createElement("div", {    "class":`live-layer`,"aria-hidden":`true`,});liveLayer = $$_div4; }
       }
     }
   }

   { svelteHTML.createElement("div", {   "class":`overlay`,"aria-hidden":!tb,});
    if(hoverBox){
       { svelteHTML.createElement("div", {     "class":`outline hover`,});__sveltets_2_ensureType(String, Number, `${hoverBox.x}px`);__sveltets_2_ensureType(String, Number, `${hoverBox.y}px`);__sveltets_2_ensureType(String, Number, `${hoverBox.w}px`);__sveltets_2_ensureType(String, Number, `${hoverBox.h}px`);
        if(!hover?.block && hoverName){ { svelteHTML.createElement("span", {  "class":`box-label hover-tag`,});__sveltets_2_ensureType(String, Number, hoverBox.y < 20 ? '0' : '-20px');hoverName; }}
       }
    }
    if(selBox){
       { svelteHTML.createElement("div", {     "class":`outline sel`,});__sveltets_2_ensureType(String, Number, `${selBox.x}px`);__sveltets_2_ensureType(String, Number, `${selBox.y}px`);__sveltets_2_ensureType(String, Number, `${selBox.w}px`);__sveltets_2_ensureType(String, Number, `${selBox.h}px`);
         { svelteHTML.createElement("span", {  "class":`box-label`,});__sveltets_2_ensureType(String, Number, selBox.y < 22 ? '0' : '-22px');selName; }
       }
    }
    if(blockBox){
       { svelteHTML.createElement("div", {     "class":`outline block`,});__sveltets_2_ensureType(String, Number, `${blockBox.x}px`);__sveltets_2_ensureType(String, Number, `${blockBox.y}px`);__sveltets_2_ensureType(String, Number, `${blockBox.w}px`);__sveltets_2_ensureType(String, Number, `${blockBox.h}px`); }
    }
    if(dropY != null){
       { svelteHTML.createElement("div", {  "class":`dropline`,});__sveltets_2_ensureType(String, Number, `${dropY - 1}px`); }
    }
    if(tb && selectedSection){
      const sid = selectedSection;
       { const $$_div2 = svelteHTML.createElement("div", {           "class":`toolbar`,"role":`toolbar`,"aria-label":`Section actions`,...__sveltets_2_empty({"data-testid":`site-section-toolbar`}),});__sveltets_2_ensureType(String, Number, `${tb.x}px`);__sveltets_2_ensureType(String, Number, `${tb.y}px`);tbW= $$_div2.clientWidth;tbH= $$_div2.clientHeight;
         { svelteHTML.createElement("button", {     "class":`tb-text`,"onclick":(e) => onaction('ask', sid, e),"title":`Ask Otto about this section`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`sparkle`,"size":12,}});}   }
         { svelteHTML.createElement("button", {     "class":`tb-text`,"onclick":(e) => onaction('variants', sid, e),"title":`Ask Otto for 3 directions`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`columns`,"size":12,}});}  }
         { svelteHTML.createElement("span", { "class":`sep`,}); }
         { svelteHTML.createElement("button", {     "onclick":(e) => onaction('up', sid, e),"aria-label":`Move up`,"title":`Move up`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`arrowUp`,"size":12,}});} }
         { svelteHTML.createElement("button", {     "onclick":(e) => onaction('down', sid, e),"aria-label":`Move down`,"title":`Move down`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`arrowDown`,"size":12,}});} }
         { svelteHTML.createElement("button", {     "onclick":(e) => onaction('duplicate', sid, e),"aria-label":`Duplicate`,"title":`Duplicate`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`copy`,"size":12,}});} }
         { svelteHTML.createElement("button", {       "onclick":(e) => onaction('hide-mobile', sid, e),"aria-label":`Hidden on mobile`,"title":selHiddenMobile ? 'Show on mobile' : 'Hide on mobile',"aria-pressed":selHiddenMobile,});
           { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":selHiddenMobile ? 'eyeOff' : 'eye',"size":12,}});}
         }
         { svelteHTML.createElement("button", {       "onclick":(e) => onaction('more', sid, e),"aria-label":`More section actions`,"title":`More`,"aria-haspopup":`menu`,}); { const $$_nocI4C = __sveltets_2_ensureComponent(Icon); new $$_nocI4C({ target: __sveltets_2_any(), props: {    "name":`more`,"size":12,}});} }
       }
    }
   }
   { svelteHTML.createElement("span", { "class":`zoom-label`,});device === 'desktop' ? 'Desktop' : device === 'tablet' ? 'Tablet' : 'Mobile';  W;   Math.round(z * 100);  }
 }


};
return { props: {} as any as Props, exports: {}, bindings: __sveltets_$$bindings('scale'), slots: {}, events: {} }}
const SiteCanvas__SvelteComponent_ = __sveltets_2_fn_component($$render());
/*Ωignore_startΩ*/type SiteCanvas__SvelteComponent_ = ReturnType<typeof SiteCanvas__SvelteComponent_>;
/*Ωignore_endΩ*/export default SiteCanvas__SvelteComponent_;