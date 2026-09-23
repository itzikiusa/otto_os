<script lang="ts" module>
  export type SiteDevice = 'desktop' | 'tablet' | 'mobile';
  export const DEVICE_WIDTH: Record<SiteDevice, number> = { desktop: 1280, tablet: 834, mobile: 390 };
  export type CanvasAction = 'up' | 'down' | 'duplicate' | 'hide-mobile' | 'ask' | 'variants' | 'more';
  /** Drag payload MIME for library blocks (`<block>` or `lib:<n>`). */
  export const BLOCK_MIME = 'application/x-otto-site-block';
</script>

<script lang="ts">
  // The Site Studio canvas: the page rendered as REAL DOM (the same markup +
  // site.css the export ships) inside a device frame, scaled to the chosen zoom.
  // Sections are rendered per section with {@html} so an edit re-renders only
  // what changed. Selection, hover, inline editing (double-click text) and
  // library drops are handled by delegated listeners on the page root; the
  // outlines, name tags, insertion line and the floating section toolbar live
  // in an overlay measured from the DOM (clamped into the canvas, never off it).
  import { tick, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { renderSection, type EmbedInfo } from './engine/render';
  import { themeStyle, type Theme } from './engine/theme';
  import { sectionLabel } from './engine/catalog';
  import type { SiteDoc, SitePage } from './engine/types';

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
    domain: string;
    embed: (uri: string) => EmbedInfo | null;
    asset: (uri: string) => string | null;
    /** Effective scale (read by the toolbar's zoom label). */
    scale?: number;
    onselect: (sectionId: string | null, blockId: string | null) => void;
    onedit: (sectionId: string, blockId: string | null, key: string, value: string) => void;
    onaction: (action: CanvasAction, sectionId: string, e: MouseEvent) => void;
    ondrop: (payload: string, index: number) => void;
  }
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
    domain,
    embed,
    asset,
    scale = $bindable(1),
    onselect,
    onedit,
    onaction,
    ondrop,
  }: Props = $props();

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
  const ctx = $derived({
    theme,
    editable: true,
    homeHref: '#',
    pageHref: () => '#',
    embed,
    asset,
  });
  const html = $derived(new Map(page.sections.map((s) => [s.id, renderSection(s, ctx)])));
  const style = $derived(themeStyle(theme));

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
  }

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

  // ── Overlay geometry (in host coordinates) ──────────────────────────────────
  interface Box {
    x: number;
    y: number;
    w: number;
    h: number;
  }
  let selBox = $state<Box | null>(null);
  let blockBox = $state<Box | null>(null);
  let hoverBox = $state<Box | null>(null);
  let dropY = $state<number | null>(null);
  let tbW = $state(0);

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
    });
  }
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
      if (r.top < s.top + 40 || r.top > s.bottom - 80) sc.scrollBy({ top: r.top - s.top - 48, behavior: 'smooth' });
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
    const y = Math.max(8, Math.min(viewH - 48, selBox.y - 44));
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
</script>

<div class="host" bind:this={hostEl} bind:clientWidth={viewW} bind:clientHeight={viewH} data-testid="site-canvas">
  <div
    class="scroller"
    bind:this={scrollerEl}
    onscroll={measure}
    ondragover={onDragOver}
    ondragleave={() => (dropY = null)}
    ondrop={onDrop}
    role="presentation"
  >
    <div class="stage" style:width={`${Math.ceil(W * z)}px`} style:height={`${Math.ceil(frameH * z)}px`}>
      <div class="frame" class:mobile={device === 'mobile'} style:width={`${W}px`} style:transform={`scale(${z})`} bind:offsetHeight={frameH}>
        <div class="chrome" aria-hidden="true">
          <span class="dots"><i></i><i></i><i></i></span>
          <span class="url"><Icon name="lock" size={12} /> {url}</span>
        </div>
        <div class="os-site os-edit" class:os-still={still} {style} style:--os-z={z} bind:this={siteEl} data-testid="site-page">
          {#each page.sections as s (s.id)}
            <div class="os-slot">{@html shown(s.id)}</div>
          {:else}
            <div class="empty-page">
              <Icon name="layers" size={16} />
              <span>This page is empty. Drag a block here from <strong>Blocks</strong>, or pick one in the left panel.</span>
            </div>
          {/each}
        </div>
      </div>
    </div>
  </div>

  <div class="overlay" aria-hidden={!tb}>
    {#if hoverBox}
      <div class="outline hover" style:left={`${hoverBox.x}px`} style:top={`${hoverBox.y}px`} style:width={`${hoverBox.w}px`} style:height={`${hoverBox.h}px`}>
        {#if !hover?.block && hoverName}<span class="tag hover-tag" style:top={hoverBox.y < 20 ? '0' : '-20px'}>{hoverName}</span>{/if}
      </div>
    {/if}
    {#if selBox}
      <div class="outline sel" style:left={`${selBox.x}px`} style:top={`${selBox.y}px`} style:width={`${selBox.w}px`} style:height={`${selBox.h}px`}>
        <span class="tag" style:top={selBox.y < 22 ? '0' : '-22px'}>{selName}</span>
      </div>
    {/if}
    {#if blockBox}
      <div class="outline block" style:left={`${blockBox.x}px`} style:top={`${blockBox.y}px`} style:width={`${blockBox.w}px`} style:height={`${blockBox.h}px`}></div>
    {/if}
    {#if dropY != null}
      <div class="dropline" style:top={`${dropY - 1}px`}></div>
    {/if}
    {#if tb && selectedSection}
      {@const sid = selectedSection}
      <div class="toolbar" style:left={`${tb.x}px`} style:top={`${tb.y}px`} bind:clientWidth={tbW} role="toolbar" aria-label="Section actions" data-testid="site-section-toolbar">
        <button class="tb-text" onclick={(e) => onaction('ask', sid, e)} title="Ask Otto about this section"><Icon name="sparkle" size={12} /> Ask Otto</button>
        <button class="tb-text" onclick={(e) => onaction('variants', sid, e)} title="Ask Otto for 3 directions"><Icon name="columns" size={12} /> Variants</button>
        <span class="sep"></span>
        <button onclick={(e) => onaction('up', sid, e)} aria-label="Move up" title="Move up"><Icon name="arrowUp" size={12} /></button>
        <button onclick={(e) => onaction('down', sid, e)} aria-label="Move down" title="Move down"><Icon name="arrowDown" size={12} /></button>
        <button onclick={(e) => onaction('duplicate', sid, e)} aria-label="Duplicate" title="Duplicate"><Icon name="copy" size={12} /></button>
        <button onclick={(e) => onaction('hide-mobile', sid, e)} aria-label={selHiddenMobile ? 'Show on mobile' : 'Hide on mobile'} title={selHiddenMobile ? 'Show on mobile' : 'Hide on mobile'} aria-pressed={selHiddenMobile}>
          <Icon name={selHiddenMobile ? 'eyeOff' : 'eye'} size={12} />
        </button>
        <button onclick={(e) => onaction('more', sid, e)} aria-label="More section actions" title="More" aria-haspopup="menu"><Icon name="more" size={12} /></button>
      </div>
    {/if}
  </div>
  <span class="zoom-label">{device === 'desktop' ? 'Desktop' : device === 'tablet' ? 'Tablet' : 'Mobile'} · {W} px · {Math.round(z * 100)}%</span>
</div>

<style>
  .host {
    position: relative;
    height: 100%;
    min-height: 0;
    overflow: hidden;
    background: var(--term-bg);
    background-image: radial-gradient(color-mix(in srgb, var(--text) 9%, transparent) 1px, transparent 1px);
    background-size: 16px 16px;
  }
  .scroller {
    position: absolute;
    inset: 0;
    overflow: auto;
    padding: 32px 32px 64px;
  }
  .stage {
    position: relative;
    margin-inline: auto;
    transition: width 200ms ease-out;
  }
  .frame {
    position: absolute;
    inset-block-start: 0;
    inset-inline-start: 0;
    transform-origin: 0 0;
    border-radius: var(--radius-l);
    overflow: hidden;
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
  }
  :global([dir='rtl']) .frame {
    transform-origin: 100% 0;
  }
  .chrome {
    display: flex;
    align-items: center;
    gap: 12px;
    height: 32px;
    padding: 0 12px;
    background: var(--surface-2);
    border-block-end: 1px solid var(--border);
  }
  .dots {
    display: inline-flex;
    gap: 6px;
  }
  .dots i {
    width: 9px;
    height: 9px;
    border-radius: 50%;
    background: var(--border-strong);
  }
  .url {
    flex: 1;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    max-width: 520px;
    margin-inline: auto;
    height: 20px;
    border-radius: var(--radius-s);
    background: var(--surface);
    color: var(--text-dim);
    font-size: var(--fs-xs);
    overflow: hidden;
    white-space: nowrap;
  }
  .frame.mobile .url {
    max-width: none;
  }
  .empty-page {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 48px;
    padding: 28px;
    border: 2px dashed var(--border-strong);
    border-radius: var(--radius-l);
    color: var(--text-dim);
    font-size: var(--fs-m);
  }
  .overlay {
    position: absolute;
    inset: 0;
    pointer-events: none;
    overflow: hidden;
  }
  .outline {
    position: absolute;
    box-sizing: border-box;
  }
  .outline.hover {
    border: 1px solid var(--accent);
  }
  .outline.sel {
    border: 2px solid var(--accent);
  }
  .outline.block {
    border: 1.5px dashed var(--accent);
    border-radius: var(--radius-s);
  }
  .tag {
    position: absolute;
    inset-inline-start: -2px;
    padding: 2px 8px;
    border-radius: var(--radius-s) var(--radius-s) 0 0;
    background: var(--accent-solid);
    color: var(--accent-contrast);
    font-size: var(--fs-xs);
    font-weight: 600;
    white-space: nowrap;
  }
  .hover-tag {
    background: var(--surface);
    color: var(--accent-text);
    border: 1px solid var(--accent);
    border-block-end: 0;
  }
  .dropline {
    position: absolute;
    inset-inline: 24px;
    height: 3px;
    border-radius: 2px;
    background: var(--accent-solid);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 25%, transparent);
  }
  .toolbar {
    position: absolute;
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px;
    border-radius: var(--radius-m);
    background: var(--surface);
    border: 1px solid var(--border-strong);
    box-shadow: var(--shadow);
    pointer-events: auto;
    white-space: nowrap;
  }
  .toolbar button {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 5px;
    min-width: 26px;
    height: 26px;
    padding: 0 6px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .toolbar button:hover {
    background: var(--surface-2);
  }
  .toolbar button:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .toolbar button[aria-pressed='true'] {
    color: var(--accent-text);
    background: var(--accent-soft);
  }
  .tb-text {
    font-weight: 500;
  }
  .sep {
    width: 1px;
    height: 16px;
    margin: 0 3px;
    background: var(--border);
  }
  .zoom-label {
    position: absolute;
    inset-block-end: 10px;
    inset-inline-start: 10px;
    padding: 2px 8px;
    border-radius: 999px;
    background: var(--surface);
    border: 1px solid var(--border);
    font-size: var(--fs-xs);
    color: var(--text-dim);
    pointer-events: none;
  }

  /* Editor-only affordances on the rendered site (never exported). */
  .os-edit :global(.os-slot) {
    display: contents;
  }
  .os-edit :global([data-os-section]) {
    cursor: default;
  }
  .os-edit :global([data-os-edit]) {
    cursor: text;
  }
  .os-edit :global(.os-editing) {
    outline: 2px solid var(--accent);
    outline-offset: 3px;
    border-radius: 2px;
    cursor: text;
  }
  .os-edit :global(.os-hidden) {
    opacity: 0.35;
  }
  .os-edit :global(.os-edit-badge) {
    position: absolute;
    inset-block-end: 6%;
    inset-inline-start: 50%;
    transform: translateX(-50%);
    /* Counter-scale so the badge reads at UI size whatever the zoom. */
    padding: calc(4px / var(--os-z)) calc(12px / var(--os-z));
    border-radius: 999px;
    background: var(--studio-3d);
    color: var(--studio-glyph);
    font-family: var(--font-ui);
    font-size: calc(var(--fs-s) / var(--os-z));
    font-weight: 600;
    line-height: 1.3;
    white-space: nowrap;
    box-shadow: var(--shadow);
  }
  .os-edit :global(.os-edit-badge--warn) {
    background: var(--warning);
  }
  @container os-site (min-width: 1024px) {
    .os-edit :global(.os-hide-desktop) {
      display: block;
      opacity: 0.35;
    }
    .os-edit :global(.os-hide-desktop.os-minh-80),
    .os-edit :global(.os-hide-desktop.os-minh-100) {
      display: grid;
    }
  }
  @container os-site (min-width: 640px) and (max-width: 1023px) {
    .os-edit :global(.os-hide-tablet) {
      display: block;
      opacity: 0.35;
    }
    .os-edit :global(.os-hide-tablet.os-minh-80),
    .os-edit :global(.os-hide-tablet.os-minh-100) {
      display: grid;
    }
  }
  @container os-site (max-width: 639px) {
    .os-edit :global(.os-hide-mobile) {
      display: block;
      opacity: 0.35;
    }
    .os-edit :global(.os-hide-mobile.os-minh-80),
    .os-edit :global(.os-hide-mobile.os-minh-100) {
      display: grid;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .stage {
      transition: none;
    }
  }
</style>
