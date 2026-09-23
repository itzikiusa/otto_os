<script lang="ts" module>
  /** A section offered from another site ("From your library"). */
  export interface LibraryBlock {
    key: string;
    siteId: string;
    siteTitle: string;
    seq: number;
    section: import('./engine/types').SiteSection;
  }
</script>

<script lang="ts">
  // Site Studio left panel: Pages · Layers · Blocks.
  //   Pages  — the site's pages (home first), add / rename / delete.
  //   Layers — the current page's sections (drag to reorder, eye = hide
  //            everywhere, a glyph when hidden at a breakpoint); the selected
  //            section expands to its child blocks (embeds, cards, tiers…).
  //   Blocks — the searchable library, grouped by family, each tile a LIVE
  //            miniature rendered with the site's brand theme; plus "From your
  //            library": sections of other sites, with provenance. Click a tile
  //            to insert after the selection, or drag it onto the canvas.
  import Icon from '../../../lib/components/Icon.svelte';
  import type { IconName } from '../../../lib/components/Icon.svelte';
  import { FAMILIES, itemDef, searchBlocks, sectionLabel, type SectionDef } from './engine/catalog';
  import { renderSection, str } from './engine/render';
  import { themeStyle, type Theme } from './engine/theme';
  import type { SiteBlock, SiteDoc, SiteSection } from './engine/types';
  import { BLOCK_MIME } from './SiteCanvas.svelte';

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
  }
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
  }: Props = $props();

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
</script>

<div class="panel">
  <div class="tabs segmented" role="tablist" aria-label="Site panels">
    <button role="tab" aria-selected={tab === 'pages'} class:active={tab === 'pages'} onclick={() => (tab = 'pages')} data-testid="site-tab-pages">
      <Icon name="file" size={12} /> Pages
    </button>
    <button role="tab" aria-selected={tab === 'layers'} class:active={tab === 'layers'} onclick={() => (tab = 'layers')} data-testid="site-tab-layers">
      <Icon name="layers" size={12} /> Layers
    </button>
    <button role="tab" aria-selected={tab === 'blocks'} class:active={tab === 'blocks'} onclick={() => (tab = 'blocks')} data-testid="site-tab-blocks">
      <Icon name="grid" size={12} /> Blocks
    </button>
  </div>

  <div class="body" role="tabpanel">
    {#if tab === 'pages'}
      <ul class="list" aria-label="Pages">
        {#each doc.pages as p, i (p.id)}
          <li class="row page-row" class:active={p.id === page?.id}>
            <button class="row-main" onclick={() => onpage(p.id)} aria-current={p.id === page?.id ? 'page' : undefined}>
              <Icon name={i === 0 ? 'home' : 'file'} size={12} />
              <span class="name">{p.title || 'Untitled page'}</span>
              <span class="dim mono">/{p.slug}</span>
            </button>
            {#if !readonly}
              <button class="icon-btn" onclick={(e) => onpagemenu(p.id, e)} aria-label={`Page actions for ${p.title}`} title="Page actions" aria-haspopup="menu">
                <Icon name="more" size={12} />
              </button>
            {/if}
          </li>
        {/each}
      </ul>
      {#if !readonly}
        <button class="btn small ghost add" onclick={onaddpage} data-testid="site-add-page"><Icon name="plus" size={12} /> Add page</button>
      {/if}
    {:else if tab === 'layers'}
      <p class="section-title">{page?.title || 'Page'} · {page?.sections.length ?? 0} sections</p>
      {#if page && page.sections.length}
        <ul class="list" aria-label="Sections" ondragleave={() => (dropAt = null)}>
          {#each page.sections as s, i (s.id)}
            {@const sel = s.id === selectedSection}
            {@const off = hiddenAt(s)}
            <li
              class="layer"
              class:drop-before={dropAt === i}
              class:drop-after={dropAt === i + 1 && i === page.sections.length - 1}
              draggable={!readonly}
              ondragstart={(e) => rowDragStart(e, s.id)}
              ondragover={(e) => rowDragOver(e, i)}
              ondrop={rowDrop}
              ondragend={() => {
                dragId = null;
                dropAt = null;
              }}
            >
              <div class="row" class:active={sel && !selectedBlock} class:muted={s.hidden}>
                {#if !readonly}<span class="grip" aria-hidden="true"><Icon name="grip" size={12} /></span>{/if}
                <button class="row-main" onclick={() => onselect(s.id, null)} aria-pressed={sel} data-testid="site-layer">
                  <Icon name={familyIcon(s.block)} size={12} />
                  <span class="name">{sectionLabel(s)}</span>
                  {#if s.derived_from}<span class="glyph" title="From your library"><Icon name="link" size={12} /></span>{/if}
                  {#if off}<span class="glyph warn" title={off}><Icon name="eyeOff" size={12} /></span>{/if}
                </button>
                {#if !readonly}
                  <button class="icon-btn eye" onclick={() => ontogglehidden(s.id)} aria-label={s.hidden ? `Show ${sectionLabel(s)}` : `Hide ${sectionLabel(s)}`} title={s.hidden ? 'Show section' : 'Hide section everywhere'}>
                    <Icon name={s.hidden ? 'eyeOff' : 'eye'} size={12} />
                  </button>
                {/if}
              </div>
              {#if (s.blocks ?? []).length && (sel || (s.blocks ?? []).some((b) => b.block.startsWith('embed/3d')))}
                {@const kids = sel ? (s.blocks ?? []) : (s.blocks ?? []).filter((b) => b.block === 'embed/3d')}
                <ul class="kids">
                  {#each kids.slice(0, 12) as b (b.id)}
                    <li class="row kid" class:active={b.id === selectedBlock}>
                      <button class="row-main" onclick={() => onselect(s.id, b.id)} aria-pressed={b.id === selectedBlock}>
                        <Icon name={b.block === 'embed/3d' ? 'box' : b.block === 'embed/image' ? 'image' : 'square'} size={12} />
                        <span class="name">{childLabel(b)}</span>
                        {#if b.block === 'embed/3d' && str(b.props, 'src')}<span class="glyph link" title="Linked 3D artifact"><Icon name="link" size={12} /></span>{/if}
                      </button>
                    </li>
                  {/each}
                  {#if sel && (s.blocks ?? []).length > 12}<li class="dim more">+{(s.blocks ?? []).length - 12} more</li>{/if}
                </ul>
              {/if}
            </li>
          {/each}
        </ul>
      {:else}
        <p class="dim hint">No sections yet. Open <button class="linkbtn" onclick={() => (tab = 'blocks')}>Blocks</button> to add one.</p>
      {/if}
    {:else}
      <label class="search">
        <Icon name="search" size={12} />
        <input class="input" placeholder="Search blocks" bind:value={query} aria-label="Search blocks" data-testid="site-block-search" />
      </label>
      {#if readonly}<p class="dim hint">Read-only — blocks can’t be added.</p>{/if}
      {#each groups as g (g.id)}
        <section class="group" aria-label={g.label}>
          <h3 class="section-title">{g.label} <span class="count">{g.blocks.length}</span></h3>
          <div class="tiles">
            {#each g.blocks as def (def.block)}
              <button
                class="tile"
                disabled={readonly}
                draggable={!readonly}
                ondragstart={(e) => tileDrag(e, def.block)}
                onclick={() => oninsert(def.block)}
                title={`${def.label} — ${def.description}. Click to insert, or drag onto the page.`}
                data-testid="site-block-tile"
                data-block={def.block}
              >
                <span class="thumb" aria-hidden="true" style:background={theme.surface}>
                  <span class="mini os-site os-still" {style}>{@html preview(def)}</span>
                </span>
                <span class="tile-label">{def.label.replace(/^[^·]+·\s*/, '') || def.label}</span>
              </button>
            {/each}
          </div>
        </section>
      {:else}
        <p class="dim hint">No block matches “{query}”.</p>
      {/each}
      <section class="group" aria-label="From your library">
        <h3 class="section-title">From your library {#if libFound.length}<span class="count">{libFound.length}</span>{/if}</h3>
        {#if libraryState === 'loading'}
          <p class="dim hint">Looking through your other sites…</p>
        {:else if libraryState === 'error'}
          <p class="dim hint">Couldn’t load your other sites. <button class="linkbtn" onclick={onloadlibrary}>Retry</button></p>
        {:else if !libFound.length}
          <p class="dim hint">Sections from your other sites show up here, with where they came from.</p>
        {:else}
          <div class="tiles">
            {#each libFound.slice(0, 24) as l (l.key)}
              <button
                class="tile"
                disabled={readonly}
                draggable={!readonly}
                ondragstart={(e) => tileDrag(e, `lib:${l.key}`)}
                onclick={() => oninsert(`lib:${l.key}`)}
                title={`${sectionLabel(l.section)} · from ${l.siteTitle} v${l.seq}`}
                data-testid="site-library-tile"
              >
                <span class="thumb" aria-hidden="true" style:background={theme.surface}>
                  <span class="mini os-site os-still" {style}>{@html renderSection(l.section, tileCtx)}</span>
                </span>
                <span class="tile-label">{sectionLabel(l.section)}</span>
                <span class="prov"><Icon name="link" size={12} /> {l.siteTitle} v{l.seq}</span>
              </button>
            {/each}
          </div>
        {/if}
      </section>
    {/if}
  </div>
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    min-height: 0;
    height: 100%;
  }
  .tabs {
    margin: 10px 12px 0;
    align-self: flex-start;
  }
  .tabs button {
    display: inline-flex;
    align-items: center;
    gap: 5px;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 10px 8px 16px;
  }
  .section-title {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 6px 8px 6px;
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .count {
    font-weight: 500;
    letter-spacing: 0;
  }
  .list,
  .kids {
    margin: 0;
    padding: 0;
    list-style: none;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: var(--radius-s);
  }
  .row:hover {
    background: var(--hover);
  }
  .row.active {
    background: var(--accent-soft);
  }
  .row.active .name {
    color: var(--accent-text);
    font-weight: 600;
  }
  .row.muted .name {
    color: var(--text-dim);
    text-decoration: line-through;
  }
  .row-main {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 8px;
    height: 28px;
    padding: 0 8px;
    border: 0;
    background: transparent;
    color: var(--text);
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
    border-radius: var(--radius-s);
  }
  .row-main :global(svg) {
    flex: none;
    color: var(--text-dim);
  }
  .row-main:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: -2px;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .grip {
    display: inline-flex;
    padding-inline-start: 4px;
    color: var(--text-dim);
    cursor: grab;
    opacity: 0;
  }
  .layer:hover .grip {
    opacity: 1;
  }
  .glyph {
    display: inline-flex;
    color: var(--text-dim);
  }
  .glyph.link :global(svg),
  .glyph :global(svg) {
    color: var(--info);
  }
  .glyph.warn :global(svg) {
    color: var(--warning);
  }
  .eye {
    opacity: 0;
  }
  .row:hover .eye,
  .row.muted .eye,
  .eye:focus-visible {
    opacity: 1;
  }
  .kids {
    margin-inline-start: 22px;
    padding-inline-start: 6px;
    border-inline-start: 1px solid var(--border);
  }
  .kid .row-main {
    height: 26px;
    font-size: var(--fs-s);
  }
  .more {
    padding: 2px 8px;
    font-size: var(--fs-xs);
  }
  .layer.drop-before {
    box-shadow: inset 0 2px 0 var(--accent-solid);
  }
  .layer.drop-after {
    box-shadow: inset 0 -2px 0 var(--accent-solid);
  }
  .page-row .dim {
    font-size: var(--fs-xs);
  }
  .add {
    margin: 8px;
  }
  .hint {
    margin: 8px;
    font-size: var(--fs-s);
  }
  .linkbtn {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    cursor: pointer;
  }
  .search {
    position: sticky;
    top: -10px;
    z-index: 1;
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 0 4px 6px;
    padding: 4px 0;
    background: var(--surface);
    color: var(--text-dim);
  }
  .search .input {
    flex: 1;
    min-width: 0;
  }
  .group {
    margin-block-end: 10px;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 8px;
    padding: 0 4px;
  }
  .tile {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 0;
    padding: 4px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface);
    color: var(--text);
    text-align: start;
    cursor: grab;
    transition: border-color 130ms ease-out, box-shadow 130ms ease-out;
  }
  .tile:hover {
    border-color: var(--accent);
    box-shadow: var(--shadow);
  }
  .tile:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .tile:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .thumb {
    position: relative;
    display: block;
    height: 64px;
    overflow: hidden;
    border-radius: var(--radius-s);
    background: var(--surface-2);
    pointer-events: none;
  }
  .mini {
    position: absolute;
    inset-block-start: 0;
    inset-inline-start: 0;
    width: 1280px;
    transform: scale(0.092);
    transform-origin: 0 0;
  }
  :global([dir='rtl']) .mini {
    transform-origin: 100% 0;
    inset-inline-start: auto;
    inset-inline-end: 0;
  }
  .tile-label {
    padding: 0 2px;
    font-size: var(--fs-xs);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .prov {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 0 2px;
    color: var(--text-dim);
    font-size: var(--fs-xs);
    overflow: hidden;
    white-space: nowrap;
  }
  .prov :global(svg) {
    color: var(--info);
    flex: none;
  }
</style>
