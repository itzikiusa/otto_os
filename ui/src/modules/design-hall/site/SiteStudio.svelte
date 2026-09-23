<script lang="ts">
  // Site Studio — the editor ArtifactStage mounts for `otto-site` documents.
  //
  //   toolbar: breakpoint (1280 / 834 / 390) · zoom · motion · undo/redo   Preview · Publish ▾
  //   ┌ Pages · Layers · Blocks ┬ canvas (real DOM in a device frame) ┬ Design inspector ┐
  //   (≤ 1100 px wide the inspector stacks under the left panel; ≤ 700 px it all stacks)
  //
  // The studio owns the parsed document, an undo stack and the selection; every
  // change is serialized and handed UP through `onchange` — the artifact view
  // saves it with `PUT …/content` + `base_version` (409 → the person chooses),
  // exactly like every other format. The selection is mirrored into
  // `siteSelection` for the Otto co-design panel. Brand tokens come from the
  // project's brand kit (else the document's `brand` reference, else a
  // `uses_tokens` link); 3D embeds and library images are resolved lazily.
  import { onDestroy, untrack } from 'svelte';
  import Icon from '../../../lib/components/Icon.svelte';
  import { ctxMenu, type MenuItem } from '../../../lib/contextmenu.svelte';
  import { confirmer } from '../../../lib/confirm.svelte';
  import { toasts } from '../../../lib/toast.svelte';
  import { router } from '../../../lib/router.svelte';
  import { designBus } from '../../../lib/events.svelte';
  import * as design from '../../../lib/api/design';
  import type { DesignArtifact } from '../../../lib/api/types';
  import { library as hallLibrary } from '../library.svelte';
  import { auditPage } from './engine/audit';
  import * as ops from './engine/ops';
  import { str, type EmbedInfo } from './engine/render';
  import { starterSite } from './engine/templates';
  import { buildTheme, type Theme } from './engine/theme';
  import type { SiteDoc, SiteIssue } from './engine/types';
  import SITE_CSS from './engine/site.css?raw';
  import SiteCanvas, { type CanvasAction, type SiteDevice } from './SiteCanvas.svelte';
  import LeftPanel, { type LibraryBlock } from './LeftPanel.svelte';
  import Inspector from './Inspector.svelte';
  import TemplatePicker from './TemplatePicker.svelte';
  import PreviewModal from './PreviewModal.svelte';
  import PublishModal from './PublishModal.svelte';
  import { siteSelection } from './selection.svelte';
  import { librarySites, resolveBrand, resolveEmbed, resolveImage, siteContent } from './siteApi';

  interface Props {
    artifact: DesignArtifact;
    source: string | null;
    readonly?: boolean;
    /** Compare panes: the page only, fit to the pane, no panels. */
    compact?: boolean;
    onchange?: (source: string) => void;
  }
  let { artifact, source, readonly = false, compact = false, onchange }: Props = $props();

  // The page stylesheet is injected once into the document (every rule is
  // scoped under .os-*, so it can't touch the app).
  $effect(() => {
    if (document.getElementById('otto-site-css')) return;
    const el = document.createElement('style');
    el.id = 'otto-site-css';
    el.textContent = SITE_CSS;
    document.head.appendChild(el);
  });

  // ── Document + undo ────────────────────────────────────────────────────────
  let doc = $state<SiteDoc | null>(null);
  let issues = $state<SiteIssue[]>([]);
  let undoStack: SiteDoc[] = [];
  let redoStack: SiteDoc[] = [];
  let canUndo = $state(false);
  let canRedo = $state(false);
  let lastKey: string | null = null;
  let lastAt = 0;
  /** The text we last handed up — its echo through `source` is not a reload. */
  let emitted: string | null = null;

  $effect(() => {
    const s = source;
    untrack(() => {
      if (s !== null && s === emitted) return;
      const r = ops.parseSite(s);
      doc = r.doc;
      issues = r.issues;
      undoStack = [];
      redoStack = [];
      syncUndo();
    });
  });

  function syncUndo(): void {
    canUndo = undoStack.length > 0;
    canRedo = redoStack.length > 0;
  }
  function emit(next: SiteDoc): void {
    const text = ops.serializeSite(next);
    emitted = text;
    onchange?.(text);
  }
  /** Apply an edit. Rapid edits of the same field (typing) coalesce into one undo step. */
  function commit(next: SiteDoc, coalesce?: string): void {
    if (readonly || !doc) return;
    const now = Date.now();
    if (!(coalesce && coalesce === lastKey && now - lastAt < 1200)) {
      undoStack.push(doc);
      if (undoStack.length > 100) undoStack.shift();
    }
    lastKey = coalesce ?? null;
    lastAt = now;
    redoStack = [];
    doc = next;
    syncUndo();
    emit(next);
  }
  function undo(): void {
    const prev = undoStack.pop();
    if (!prev || !doc) return;
    redoStack.push(doc);
    doc = prev;
    lastKey = null;
    syncUndo();
    emit(prev);
  }
  function redo(): void {
    const next = redoStack.pop();
    if (!next || !doc) return;
    undoStack.push(doc);
    doc = next;
    lastKey = null;
    syncUndo();
    emit(next);
  }

  // ── Selection (mirrored for the Otto panel) ─────────────────────────────────
  let pageId = $state('');
  let sectionId = $state<string | null>(null);
  let blockId = $state<string | null>(null);
  const page = $derived(doc?.pages.find((p) => p.id === pageId) ?? doc?.pages[0] ?? null);

  $effect(() => {
    const d = doc;
    if (!d) return;
    untrack(() => {
      if (!d.pages.some((p) => p.id === pageId)) pageId = d.pages[0]?.id ?? '';
      if (sectionId && !ops.findSection(d, sectionId)) sectionId = null;
      if (blockId && !ops.findBlock(d, blockId)) blockId = null;
    });
  });
  $effect(() => {
    if (compact) return;
    siteSelection.set(artifact.id, { pageId: page?.id ?? null, sectionId, blockId }, doc);
  });
  onDestroy(() => siteSelection.clear(artifact.id));

  function select(s: string | null, b: string | null): void {
    sectionId = s;
    blockId = s ? b : null;
    if (s && doc) {
      const at = ops.findSection(doc, s);
      if (at && at.page.id !== pageId) pageId = at.page.id;
    }
  }

  // ── Viewer prefs (per viewer, best effort) ──────────────────────────────────
  const PREFS = 'otto.site-studio.prefs';
  let device = $state<SiteDevice>('desktop');
  let zoom = $state<'fit' | number>('fit');
  let playMotion = $state(false);
  let leftTab = $state<'pages' | 'layers' | 'blocks'>('layers');
  let scale = $state(1);
  try {
    const p = JSON.parse(localStorage.getItem(PREFS) ?? '{}');
    if (p.device === 'desktop' || p.device === 'tablet' || p.device === 'mobile') device = p.device;
    if (p.zoom === 'fit' || [50, 75, 100].includes(p.zoom)) zoom = p.zoom;
    if (p.leftTab === 'pages' || p.leftTab === 'layers' || p.leftTab === 'blocks') leftTab = p.leftTab;
  } catch {
    /* private mode / blocked storage: defaults */
  }
  $effect(() => {
    const v = JSON.stringify({ device, zoom, leftTab });
    try {
      localStorage.setItem(PREFS, v);
    } catch {
      /* best effort */
    }
  });

  // ── Brand theme ────────────────────────────────────────────────────────────
  let theme = $state<Theme>(buildTheme(null));
  let kitLabel = $state<string | null>(null);
  let kitId = $state<string | null>(null);
  let brandNote = $state<string | null>(null);
  let brandSeq = 0;
  const projectKit = $derived(hallLibrary.projectOf(artifact.project_id)?.brand_kit_id ?? null);
  const docBrand = $derived(doc?.brand);
  function loadBrand(): void {
    const my = ++brandSeq;
    void resolveBrand({ projectKitId: projectKit, docBrand, artifactId: artifact.id }).then((r) => {
      if (my !== brandSeq) return;
      theme = r.theme;
      kitId = r.kit?.id ?? null;
      kitLabel = r.kit ? `${r.kit.title}${r.seq != null ? ` v${r.seq}` : ''}` : null;
      brandNote = r.note;
    });
  }
  $effect(() => {
    void projectKit;
    void docBrand;
    untrack(loadBrand);
  });
  if (!hallLibrary.loaded) void hallLibrary.load();

  // ── Embeds + images (resolved lazily, cached per reference) ──────────────────
  let embeds = $state<Record<string, EmbedInfo>>({});
  let images = $state<Record<string, string | null>>({});
  const pending = new Set<string>();
  const blobUrls: string[] = [];
  $effect(() => {
    const d = doc;
    if (!d) return;
    untrack(() => {
      for (const p of d.pages)
        for (const s of p.sections) {
          for (const b of s.blocks ?? []) {
            const src = str(b.props, 'src');
            if (b.block === 'embed/3d' && src && !(src in embeds) && !pending.has(src)) {
              pending.add(src);
              void resolveEmbed(src).then((info) => {
                pending.delete(src);
                if (info.poster) blobUrls.push(info.poster);
                embeds = { ...embeds, [src]: info };
              });
            }
            for (const k of ['src', 'image']) {
              const v = str(b.props, k);
              if (v.startsWith('otto://') && b.block !== 'embed/3d') wantImage(v);
            }
          }
          for (const k of ['poster', 'image']) {
            const v = str(s.props, k);
            if (v.startsWith('otto://')) wantImage(v);
          }
        }
    });
  });
  function wantImage(uri: string): void {
    if (uri in images || pending.has(uri)) return;
    pending.add(uri);
    void resolveImage(uri).then((u) => {
      pending.delete(uri);
      if (u) blobUrls.push(u);
      images = { ...images, [uri]: u };
    });
  }
  onDestroy(() => {
    for (const u of blobUrls) URL.revokeObjectURL(u);
  });
  const embedFn = $derived.by(() => {
    const m = embeds;
    return (uri: string): EmbedInfo | null => m[uri] ?? null;
  });
  const assetFn = $derived.by(() => {
    const m = images;
    return (uri: string): string | null => m[uri] ?? null;
  });

  // Live updates: a linked brand kit or 3D artifact changed → refresh it.
  let seen = designBus.seq;
  $effect(() => {
    const now = designBus.seq;
    untrack(() => {
      for (const ev of designBus.since(seen)) {
        if (ev.type !== 'design_artifact_updated') continue;
        if (ev.artifact_id === kitId) loadBrand();
        const stale = Object.keys(embeds).filter((u) => u.startsWith(`otto://design/${ev.artifact_id}`));
        if (stale.length) {
          const next = { ...embeds };
          for (const u of stale) delete next[u];
          embeds = next;
        }
      }
      seen = now;
    });
  });

  // ── Library ("From your library") ──────────────────────────────────────────
  let libBlocks = $state<LibraryBlock[]>([]);
  let libState = $state<'idle' | 'loading' | 'ready' | 'error'>('idle');
  async function loadLibrary(): Promise<void> {
    libState = 'loading';
    try {
      const sites = (await librarySites(artifact.workspace_id, artifact.id)).slice(0, 6);
      const out: LibraryBlock[] = [];
      for (const a of sites) {
        const c = await siteContent(a).catch(() => null);
        if (!c) continue;
        const parsed = ops.parseSite(c.text).doc;
        for (const p of parsed?.pages ?? [])
          for (const s of p.sections) {
            if (s.block.startsWith('nav/')) continue;
            out.push({ key: `${a.id}:${s.id}`, siteId: a.id, siteTitle: a.title, seq: c.seq, section: s });
          }
      }
      libBlocks = out.slice(0, 48);
      libState = 'ready';
    } catch {
      libState = 'error';
    }
  }

  // ── Inserting blocks ───────────────────────────────────────────────────────
  function insert(payload: string, index?: number): void {
    if (!doc || !page || readonly) return;
    const taken = ops.allIds(doc);
    let section;
    if (payload.startsWith('lib:')) {
      const l = libBlocks.find((x) => x.key === payload.slice(4));
      if (!l) return;
      section = ops.fromLibrary(l.section, l.siteId, l.seq, taken);
    } else {
      try {
        section = ops.makeSection(payload, taken);
      } catch {
        return;
      }
    }
    const at = index ?? (sectionId ? (ops.findSection(doc, sectionId)?.index ?? page.sections.length - 1) + 1 : page.sections.length);
    commit(ops.insertSection(doc, page.id, section, at));
    select(section.id, null);
  }

  function pickTemplate(id: string | null): void {
    if (!doc) return;
    const base = starterSite(id ?? undefined, doc.title || artifact.title);
    // Name the project's kit in the document too: the save then carries a
    // `uses_tokens` link, so the Brand Kit's impact preview counts this site.
    const brand = doc.brand ?? (projectKit ? `otto://design/${projectKit}` : undefined);
    const withBrand = (d: SiteDoc): SiteDoc => (brand ? { ...d, brand } : d);
    if (!id) {
      commit(withBrand(ops.addPage(base, 'Home').doc));
      return;
    }
    commit(withBrand(base));
    toasts.success('Template applied', 'Save (⌘S) to keep it as a new version.');
  }

  // ── Canvas actions ─────────────────────────────────────────────────────────
  function onEdit(sid: string, bid: string | null, key: string, value: string): void {
    if (!doc) return;
    commit(bid ? ops.setBlockProp(doc, bid, key, value) : ops.setSectionProp(doc, sid, key, value));
  }

  function onAction(action: CanvasAction, sid: string, e: MouseEvent): void {
    if (!doc) return;
    switch (action) {
      case 'up':
        commit(ops.moveSection(doc, sid, -1));
        break;
      case 'down':
        commit(ops.moveSection(doc, sid, 1));
        break;
      case 'duplicate': {
        const r = ops.duplicateSection(doc, sid);
        commit(r.doc);
        if (r.id) select(r.id, null);
        break;
      }
      case 'hide-mobile':
        commit(ops.toggleHide(doc, sid, 'mobile'));
        break;
      case 'ask':
      case 'variants': {
        select(sid, blockId);
        siteSelection.ask(action === 'ask' ? 'refine' : 'variants');
        window.dispatchEvent(new CustomEvent('otto:site-ask', { detail: { artifactId: artifact.id, mode: action === 'ask' ? 'refine' : 'variants', selection: siteSelection.payload } }));
        toasts.info(action === 'ask' ? 'Selection shared with Otto' : 'Asking Otto for 3 directions', `${siteSelection.label ?? 'This section'} — continue in the Otto panel.`);
        break;
      }
      case 'more':
        sectionMenu(sid, e);
        break;
    }
  }

  function sectionMenu(sid: string, e: MouseEvent): void {
    if (!doc) return;
    const at = ops.findSection(doc, sid);
    if (!at) return;
    const s = at.section;
    const items: MenuItem[] = [
      { label: 'Rename…', icon: 'edit', action: () => void renameSection(sid) },
      { label: 'Duplicate', icon: 'copy', action: () => onAction('duplicate', sid, e) },
      { label: s.hidden ? 'Show everywhere' : 'Hide everywhere', icon: s.hidden ? 'eye' : 'eyeOff', action: () => commit(ops.updateSection(doc!, sid, (x) => (x.hidden = !x.hidden || undefined))) },
      { label: 'Move to top', icon: 'arrowUp', disabled: at.index === 0, action: () => commit(ops.moveSectionTo(doc!, sid, 0)) },
      { label: 'Move to bottom', icon: 'arrowDown', disabled: at.index === at.page.sections.length - 1, action: () => commit(ops.moveSectionTo(doc!, sid, at.page.sections.length)) },
      { separator: true },
      { label: 'Delete section…', icon: 'trash', danger: true, action: () => void deleteSection(sid) },
    ];
    ctxMenu.show(e, items);
  }
  async function renameSection(sid: string): Promise<void> {
    if (!doc) return;
    const at = ops.findSection(doc, sid);
    const name = await confirmer.promptText('Layer name (shown in Layers and to Otto)', {
      title: 'Rename section',
      confirmLabel: 'Rename',
      initial: at?.section.name ?? '',
    });
    if (name == null || !doc) return;
    commit(ops.updateSection(doc, sid, (s) => (name.trim() ? (s.name = name.trim().slice(0, 120)) : delete s.name)));
  }
  async function deleteSection(sid: string): Promise<void> {
    if (!doc) return;
    const ok = await confirmer.ask('Delete this section? Undo (⌘Z) or any earlier version brings it back.', { title: 'Delete section', confirmLabel: 'Delete section' });
    if (!ok || !doc) return;
    commit(ops.removeSection(doc, sid));
    select(null, null);
  }

  // ── Pages ──────────────────────────────────────────────────────────────────
  async function addPage(): Promise<void> {
    if (!doc) return;
    const title = await confirmer.promptText('Page title', { title: 'Add page', confirmLabel: 'Add page', placeholder: 'Pricing' });
    if (!title?.trim() || !doc) return;
    const r = ops.addPage(doc, title.trim());
    commit(r.doc);
    pageId = r.id;
    select(null, null);
  }
  function pageMenu(id: string, e: MouseEvent): void {
    if (!doc) return;
    const idx = doc.pages.findIndex((p) => p.id === id);
    const items: MenuItem[] = [
      {
        label: 'Rename…',
        icon: 'edit',
        action: () =>
          void confirmer.promptText('Page title', { title: 'Rename page', confirmLabel: 'Rename', initial: doc!.pages[idx]?.title ?? '' }).then((t) => {
            if (t?.trim() && doc) commit(ops.renamePage(doc, id, t.trim()));
          }),
      },
      {
        label: 'Make home page',
        icon: 'home',
        disabled: idx === 0,
        action: () => {
          if (!doc) return;
          const next = ops.clone(doc);
          const [p] = next.pages.splice(idx, 1);
          const oldHome = next.pages[0];
          if (oldHome) oldHome.slug = ops.slugify(oldHome.title) || 'page';
          p.slug = '';
          next.pages.unshift(p);
          commit(next);
        },
      },
      { separator: true },
      { label: 'Delete page…', icon: 'trash', danger: true, disabled: doc.pages.length <= 1, action: () => void deletePage(id) },
    ];
    ctxMenu.show(e, items);
  }
  async function deletePage(id: string): Promise<void> {
    if (!doc) return;
    const p = doc.pages.find((x) => x.id === id);
    const ok = await confirmer.ask(`Delete the page “${p?.title}” and its ${p?.sections.length ?? 0} sections? Undo (⌘Z) or any earlier version brings it back.`, {
      title: 'Delete page',
      confirmLabel: 'Delete page',
    });
    if (!ok || !doc) return;
    commit(ops.removePage(doc, id));
  }

  // ── Checks, publish, preview ───────────────────────────────────────────────
  const findings = $derived(doc && page ? auditPage(doc, page, theme) : []);
  const embedCount = $derived(doc ? doc.pages.reduce((n, p) => n + p.sections.reduce((m, s) => m + (s.blocks ?? []).filter((b) => b.block === 'embed/3d').length, 0), 0) : 0);
  let previewOpen = $state(false);
  let served = $state<{ pages: Record<string, string>; url: string } | null>(null);
  let publishMode = $state<'zip' | 'local' | null>(null);

  function publishMenu(e: MouseEvent): void {
    const items: MenuItem[] = [
      { label: 'Export static site (.zip): HTML + CSS', icon: 'download', disabled: !artifact.head_version_id, action: () => (publishMode = 'zip') },
      { label: 'Export as Svelte project — v2', icon: 'download', disabled: true },
      { label: 'Publish preview (local)', icon: 'globe', disabled: !artifact.head_version_id, action: () => (publishMode = 'local') },
      { separator: true },
      { label: 'Publish as claude.ai artifact — coming soon', icon: 'share', disabled: true },
    ];
    ctxMenu.show(e, items);
  }

  // ⌘Z / ⇧⌘Z inside the studio (not while typing in a field or editing text).
  let rootEl = $state<HTMLDivElement | null>(null);
  $effect(() => {
    const el = rootEl;
    if (!el) return;
    const onKey = (e: KeyboardEvent) => {
      const t = e.target as HTMLElement | null;
      const typing = !!t && (t.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/.test(t.tagName));
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === 'z' && !typing) {
        e.preventDefault();
        if (e.shiftKey) redo();
        else undo();
      } else if (e.key === 'Escape' && !typing && (sectionId || blockId)) {
        select(blockId ? sectionId : null, null);
      }
    };
    el.addEventListener('keydown', onKey);
    return () => el.removeEventListener('keydown', onKey);
  });

  const domain = $derived((doc?.settings?.domain || '').trim() || `${ops.slugify(doc?.title || artifact.title) || 'site'}.example`);
  const ZOOMS: ('fit' | number)[] = ['fit', 50, 75, 100];
  function zoomMenu(e: MouseEvent): void {
    ctxMenu.show(
      e,
      ZOOMS.map((z) => ({ label: z === 'fit' ? 'Fit' : `${z}%`, icon: zoom === z ? 'check' : undefined, action: () => (zoom = z) })),
    );
  }
</script>

{#if compact}
  <div class="compact">
    {#if doc && page}
      <SiteCanvas
        {doc}
        {page}
        {theme}
        device="desktop"
        zoom="fit"
        still={true}
        selectedSection={null}
        selectedBlock={null}
        readonly={true}
        live3d={false}
        {domain}
        embed={embedFn}
        asset={assetFn}
        onselect={() => {}}
        onedit={() => {}}
        onaction={() => {}}
        ondrop={() => {}}
      />
    {:else}
      <p class="msg">{issues.length ? 'This version isn’t a valid site document.' : 'Empty site.'}</p>
    {/if}
  </div>
{:else}
  <div class="studio" bind:this={rootEl} data-testid="site-studio">
    <div class="bar" role="toolbar" aria-label="Site Studio">
      <div class="segmented" role="group" aria-label="Breakpoint">
        <button class:active={device === 'desktop'} aria-pressed={device === 'desktop'} onclick={() => (device = 'desktop')} title="Desktop · 1280 px" data-testid="site-bp-desktop">Desktop</button>
        <button class:active={device === 'tablet'} aria-pressed={device === 'tablet'} onclick={() => (device = 'tablet')} title="Tablet · 834 px" data-testid="site-bp-tablet">Tablet</button>
        <button class:active={device === 'mobile'} aria-pressed={device === 'mobile'} onclick={() => (device = 'mobile')} title="Mobile · 390 px" data-testid="site-bp-mobile">Mobile</button>
      </div>
      <button class="btn small ghost" onclick={zoomMenu} aria-haspopup="menu" title="Zoom">
        {zoom === 'fit' ? 'Fit' : `${zoom}%`} <Icon name="chevronDown" size={12} />
      </button>
      <button class="btn small ghost" class:on={playMotion} aria-pressed={playMotion} onclick={() => (playMotion = !playMotion)} title={playMotion ? 'Stop motion presets' : 'Play motion presets on the canvas'}>
        <Icon name={playMotion ? 'square' : 'play'} size={12} /> Motion
      </button>
      <span class="sep"></span>
      <button class="icon-btn" onclick={undo} disabled={!canUndo || readonly} aria-label="Undo" title="Undo (⌘Z)"><Icon name="chevronLeft" size={14} /></button>
      <button class="icon-btn" onclick={redo} disabled={!canRedo || readonly} aria-label="Redo" title="Redo (⇧⌘Z)"><Icon name="chevronRight" size={14} /></button>
      {#if findings.some((f) => f.level === 'error')}
        <button class="chip bad" onclick={() => select(null, null)} title="Open the checks in the Design panel">
          <Icon name="warning" size={12} /> {findings.filter((f) => f.level === 'error').length} to fix
        </button>
      {/if}
      <span class="grow"></span>
      <button class="btn small" onclick={() => (previewOpen = true)} disabled={!doc || !doc.pages.length} data-testid="site-preview"><Icon name="play" size={12} /> Preview</button>
      <button class="btn small primary" onclick={publishMenu} disabled={!doc || !doc.pages.length} aria-haspopup="menu" data-testid="site-publish">
        <Icon name="share" size={12} /> Publish <Icon name="chevronDown" size={12} />
      </button>
    </div>

    {#if !doc}
      <div class="invalid" role="alert">
        <Icon name="warning" size={16} />
        <div>
          <strong>This isn’t a valid Site Studio document.</strong>
          <p class="dim">Open <em>Source</em> to fix it, or restore an earlier version.</p>
          <ul>
            {#each issues.slice(0, 8) as i (i.path + i.message)}<li><span class="mono">{i.path || '(root)'}</span> — {i.message}</li>{/each}
          </ul>
        </div>
      </div>
    {:else if doc.pages.length === 0}
      <TemplatePicker {theme} title={doc.title || artifact.title} {readonly} onpick={pickTemplate} />
    {:else if page}
      <div class="work" class:tab-blocks={leftTab === 'blocks'}>
        <aside class="left" aria-label="Pages, layers and blocks">
          <LeftPanel
            {doc}
            pageId={page.id}
            {theme}
            selectedSection={sectionId}
            selectedBlock={blockId}
            {readonly}
            bind:tab={leftTab}
            library={libBlocks}
            libraryState={libState}
            onpage={(id) => {
              pageId = id;
              select(null, null);
            }}
            onaddpage={() => void addPage()}
            onpagemenu={pageMenu}
            onselect={select}
            onreorder={(sid, to) => commit(ops.moveSectionTo(doc!, sid, to))}
            ontogglehidden={(sid) => commit(ops.updateSection(doc!, sid, (s) => (s.hidden = !s.hidden || undefined)))}
            oninsert={(p) => insert(p)}
            onloadlibrary={() => void loadLibrary()}
          />
        </aside>
        <section class="canvas" aria-label="Page canvas">
          <SiteCanvas
            {doc}
            {page}
            {theme}
            {device}
            {zoom}
            still={!playMotion}
            selectedSection={sectionId}
            selectedBlock={blockId}
            {readonly}
            {domain}
            embed={embedFn}
            asset={assetFn}
            bind:scale
            onselect={select}
            onedit={onEdit}
            onaction={onAction}
            ondrop={(p, i) => insert(p, i)}
          />
        </section>
        <aside class="insp" aria-label="Design">
          <div class="insp-head"><span class="insp-title">Design</span>{#if sectionId || blockId}<button class="linkbtn" onclick={() => select(null, null)}>Page settings</button>{/if}</div>
          <div class="insp-body">
            <Inspector
              {doc}
              pageId={page.id}
              {theme}
              {kitLabel}
              {brandNote}
              {sectionId}
              {blockId}
              {readonly}
              {findings}
              embed={embedFn}
              pickArtifacts={(kind) =>
                design.listArtifacts(
                  kind === 'brand'
                    ? { workspace_id: artifact.workspace_id, format: 'otto-brand', limit: 50 }
                    : kind === '3d'
                      ? { workspace_id: artifact.workspace_id, studio: '3d', limit: 100 }
                      : { workspace_id: artifact.workspace_id, studio: 'graphics', limit: 100 },
                ).then((l) => (kind === 'image' ? l.filter((a) => ['png', 'jpeg', 'gif', 'webp'].includes(a.format)) : l))}
              onchange={commit}
              onselect={select}
              onopen={(id) => router.go(`design/a/${encodeURIComponent(id)}`)}
              onbrandcorrection={(payload) => design.captureSignal({ artifact_id: artifact.id, kind: 'brand_correction', payload })}
            />
          </div>
        </aside>
      </div>
    {/if}
  </div>

  {#if previewOpen && doc}
    <PreviewModal {doc} {theme} pageId={page?.id ?? ''} embed={embedFn} asset={assetFn} served={served?.pages ?? null} servedUrl={served?.url ?? null} onclose={() => {
      previewOpen = false;
      served = null;
    }} />
  {/if}
  {#if publishMode && doc}
    <PublishModal
      mode={publishMode}
      {artifact}
      {doc}
      source={ops.serializeSite(doc)}
      {findings}
      {embedCount}
      onclose={() => (publishMode = null)}
      onpreview={(pages, url) => {
        served = { pages, url };
        publishMode = null;
        previewOpen = true;
      }}
    />
  {/if}
{/if}

<style>
  .studio {
    flex: 1;
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    container: site-studio / inline-size;
  }
  .bar {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    min-height: 40px;
    padding: 6px 12px;
    border-block-end: 1px solid var(--border);
    background: var(--bg);
  }
  .bar .on {
    background: var(--accent-soft);
    color: var(--accent-text);
  }
  .bar .chip {
    cursor: pointer;
  }
  .sep {
    width: 1px;
    height: 18px;
    margin: 0 4px;
    background: var(--border);
  }
  .grow {
    flex: 1;
  }
  .work {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: 248px minmax(0, 1fr) 300px;
    grid-template-rows: minmax(0, 1fr);
    grid-template-areas: 'left canvas insp';
  }
  .left {
    grid-area: left;
    min-height: 0;
    border-inline-end: 1px solid var(--border);
    background: var(--surface);
  }
  .canvas {
    grid-area: canvas;
    min-width: 0;
    min-height: 0;
  }
  .insp {
    grid-area: insp;
    min-height: 0;
    display: flex;
    flex-direction: column;
    border-inline-start: 1px solid var(--border);
    background: var(--surface);
  }
  .insp-head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 14px 8px;
    border-block-end: 1px solid var(--border);
  }
  .insp-title {
    font-size: var(--fs-m);
    font-weight: 600;
  }
  .insp-body {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .linkbtn {
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .invalid {
    display: flex;
    gap: 10px;
    margin: 20px;
    color: var(--text);
  }
  .invalid > :global(svg) {
    color: var(--danger);
    margin-block-start: 2px;
  }
  .invalid ul {
    margin: 6px 0 0;
    padding-inline-start: 18px;
    font-size: var(--fs-s);
  }
  .dim {
    color: var(--text-dim);
    margin: 2px 0 0;
    font-size: var(--fs-s);
  }
  .compact {
    flex: 1;
    height: 100%;
    min-height: 240px;
  }
  .msg {
    margin: 16px;
    color: var(--text-dim);
    font-size: var(--fs-s);
  }
  /* Medium: the inspector stacks under the left panel so the canvas keeps its width. */
  @container site-studio (max-width: 1099px) {
    .work {
      grid-template-columns: 272px minmax(0, 1fr);
      grid-template-rows: minmax(0, 0.9fr) minmax(0, 1.1fr);
      grid-template-areas: 'left canvas' 'insp canvas';
    }
    /* Browsing the block library: give the library most of the column. */
    .work.tab-blocks {
      grid-template-rows: minmax(0, 1.8fr) minmax(0, 1fr);
    }
    .insp {
      border-inline-start: 0;
      border-inline-end: 1px solid var(--border);
      border-block-start: 1px solid var(--border);
    }
  }
  /* Narrow: everything stacks; the page scrolls. */
  @container site-studio (max-width: 699px) {
    .work {
      grid-template-columns: minmax(0, 1fr);
      grid-template-rows: 460px auto auto;
      grid-template-areas: 'canvas' 'insp' 'left';
      overflow-y: auto;
    }
    .left,
    .insp {
      border: 0;
      border-block-start: 1px solid var(--border);
      max-height: none;
    }
    .insp-body {
      overflow: visible;
    }
  }
</style>
