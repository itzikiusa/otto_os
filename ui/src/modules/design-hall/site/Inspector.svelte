<script lang="ts">
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
  // Every edit goes up as a new document through `onchange` (the studio owns
  // undo + saving); nothing here talks to an agent.
  import Icon from '../../../lib/components/Icon.svelte';
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
  import type { SiteDoc, SiteLink, SiteMotion, SiteProps, SiteSection } from './engine/types';
  import { parseRef } from './siteApi';

  interface Props {
    doc: SiteDoc;
    pageId: string;
    theme: Theme;
    kitLabel: string | null;
    brandNote: string | null;
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
  }
  let {
    doc,
    pageId,
    theme,
    kitLabel,
    brandNote,
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
      confirmLabel: 'Delete section',
    });
    if (ok) {
      onchange(ops.removeSection(doc, section.id));
      onselect(null, null);
    }
  }

  // ── Style ──────────────────────────────────────────────────────────────────
  const bg = $derived(section ? resolveBackground(section.style?.background, theme) : null);
  const contrastNow = $derived(section ? sectionContrast(section, theme) : null);
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
    const anchor = anchorOf(e);
    let list: DesignArtifact[] = [];
    try {
      list = await pickArtifacts(kind);
    } catch {
      list = [];
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
        onselect(section!.id, r.id);
      });
    } else {
      const r = ops.setMedia(doc, section.id, 'embed/image', { src: '', alt: '' });
      onchange(r.doc);
      onselect(section.id, r.id);
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
    const anchor = anchorOf(e);
    let kits: DesignArtifact[] = [];
    try {
      kits = await pickArtifacts('brand');
    } catch {
      kits = [];
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
  function linkRows(key: string): SiteLink[] {
    return links(cur(), key);
  }
  function setLink(key: string, i: number, part: 'label' | 'href', value: string): void {
    const rows = linkRows(key).map((l) => ({ ...l }));
    rows[i] = { ...rows[i], [part]: value };
    setProp(key, rows);
  }
  function addLink(key: string): void {
    setProp(key, [...linkRows(key), { label: 'New link', href: '#' }]);
  }
  function removeLink(key: string, i: number): void {
    setProp(key, linkRows(key).filter((_, j) => j !== i));
  }
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
</script>

{#snippet field(f: FieldDef)}
  {@const v = str(cur(), f.key)}
  {#if f.kind === 'toggle'}
    <label class="checkbox-row toggle"><input type="checkbox" checked={flag(cur(), f.key)} disabled={readonly} onchange={(e) => setProp(f.key, e.currentTarget.checked)} /> {f.label}</label>
  {:else if f.kind === 'links'}
    <div class="field wide">
      <span class="lbl">{f.label}</span>
      {#each linkRows(f.key) as l, i (i)}
        <div class="linkrow">
          <input class="input" value={l.label} aria-label="Link label" disabled={readonly} oninput={(e) => setLink(f.key, i, 'label', e.currentTarget.value)} />
          <input class="input" value={l.href} aria-label="Link URL" disabled={readonly} oninput={(e) => setLink(f.key, i, 'href', e.currentTarget.value)} />
          <button class="icon-btn" onclick={() => removeLink(f.key, i)} disabled={readonly} aria-label="Remove link" title="Remove link"><Icon name="x" size={12} /></button>
        </div>
      {/each}
      {#if !readonly}<button class="btn small ghost" onclick={() => addLink(f.key)}><Icon name="plus" size={12} /> Add link</button>{/if}
    </div>
  {:else}
    {@const warn = urlWarning(f, v)}
    <div class="field" class:half={f.half}>
      <label for={`si-${f.key}`}>{f.label}</label>
      {#if f.kind === 'textarea'}
        <textarea id={`si-${f.key}`} class="input" rows={f.key === 'body' || f.key === 'quote' ? 4 : 2} value={v} disabled={readonly} oninput={(e) => setProp(f.key, e.currentTarget.value)}></textarea>
      {:else if f.kind === 'lines'}
        <textarea id={`si-${f.key}`} class="input" rows="4" value={rawLines(f.key)} disabled={readonly} oninput={(e) => setProp(f.key, e.currentTarget.value.split('\n'))}></textarea>
      {:else if f.kind === 'icon'}
        <input id={`si-${f.key}`} class="input" list="site-icon-names" value={v} disabled={readonly} placeholder="bolt, star, 2×…" oninput={(e) => setProp(f.key, e.currentTarget.value)} />
      {:else if f.kind === 'media'}
        <div class="with-btn">
          <input id={`si-${f.key}`} class="input" value={v} disabled={readonly} placeholder="https://… or pick from the library" oninput={(e) => setProp(f.key, e.currentTarget.value)} />
          {#if !readonly}<button class="btn small" onclick={(e) => void choose('image', e, (uri) => setProp(f.key, uri))}>Library…</button>{/if}
        </div>
      {:else}
        <input id={`si-${f.key}`} class="input" value={v} disabled={readonly} placeholder={f.placeholder ?? ''} oninput={(e) => setProp(f.key, e.currentTarget.value)} />
      {/if}
      {#if warn}<span class="hint bad"><Icon name="warning" size={12} /> {warn}</span>{:else if f.hint}<span class="hint">{f.hint}</span>{/if}
    </div>
  {/if}
{/snippet}

<div class="insp" data-testid="site-inspector">
  <datalist id="site-icon-names">
    {#each SITE_ICON_NAMES as n (n)}<option value={n}></option>{/each}
  </datalist>

  {#if block && bdef && section}
    <!-- ── A child block ── -->
    <div class="group">
      <div class="gh"><span class="k">Block</span>
        <button class="linkbtn" onclick={() => onselect(section.id, null)}><Icon name="chevronLeft" size={12} /> {sectionLabel(section)}</button>
      </div>
      <div class="ident">
        <span class="ident-icon"><Icon name={block.block === 'embed/3d' ? 'box' : block.block === 'embed/image' ? 'image' : 'square'} size={14} /></span>
        <div class="ident-text"><strong>{bdef.label}</strong><span class="dim">{itemTitle(block)}</span></div>
        {#if !readonly}
          <div class="ident-actions">
            <button class="icon-btn" onclick={() => onchange(ops.moveBlock(doc, block!.id, -1))} aria-label="Move up" title="Move up"><Icon name="arrowUp" size={12} /></button>
            <button class="icon-btn" onclick={() => onchange(ops.moveBlock(doc, block!.id, 1))} aria-label="Move down" title="Move down"><Icon name="arrowDown" size={12} /></button>
            <button class="icon-btn" onclick={() => { onchange(ops.removeBlock(doc, block!.id)); onselect(section!.id, null); }} aria-label="Remove block" title="Remove"><Icon name="trash" size={12} /></button>
          </div>
        {/if}
      </div>
    </div>

    {#if block.block === 'embed/3d'}
      <div class="group">
        <div class="gh"><span class="k">Source</span></div>
        {#if embedRef && embedInfo}
          <div class="src" class:broken={embedInfo.broken}>
            <span class="src-icon"><Icon name="box" size={14} /></span>
            <div class="ident-text">
              <strong>{embedInfo.title}</strong>
              <span class="dim">{embedInfo.seq != null ? `v${embedInfo.seq}` : 'no version yet'} · {policy === 'pinned' ? 'pinned' : policy === 'latest' ? 'follows latest' : 'follows approved'}</span>
            </div>
            {#if !embedInfo.broken}<button class="btn small" onclick={() => onopen(embedRef!.id)}>Open in 3D Studio</button>{/if}
          </div>
        {:else}
          <p class="dim note">No 3D artifact yet — the page shows a stand-in card.</p>
        {/if}
        {#if !readonly}<button class="btn small" onclick={(e) => void choose('3d', e, (uri) => setProp('src', uri))} data-testid="site-pick-3d"><Icon name="box" size={12} /> {embedRef ? 'Replace…' : 'Choose a 3D artifact…'}</button>{/if}
      </div>
      {#if embedRef}
        <div class="group">
          <div class="gh"><span class="k">Version policy</span></div>
          <div class="radios" role="radiogroup" aria-label="Version policy">
            <label><input type="radio" name="policy" checked={policy === 'approved'} disabled={readonly} onchange={() => setEmbedPolicy('approved')} /> Follow approved</label>
            <label><input type="radio" name="policy" checked={policy === 'latest'} disabled={readonly} onchange={() => setEmbedPolicy('latest')} /> Follow latest</label>
            <label><input type="radio" name="policy" checked={policy === 'pinned'} disabled={readonly} onchange={() => setEmbedPolicy('pinned')} /> Pin {embedInfo?.seq != null ? `v${embedInfo.seq}` : 'this version'}</label>
          </div>
          <p class="note">
            {policy === 'approved'
              ? 'This page updates when a new 3D version is approved. Drafts never leak in.'
              : policy === 'latest'
                ? 'Live co-design: every saved 3D version shows up here.'
                : 'Frozen at this version; newer ones show as “available”.'}
            Published sites always pin the versions they shipped with.
          </p>
        </div>
      {/if}
      <div class="group">
        <div class="gh"><span class="k">Interaction & text</span></div>
        {#each bdef.fields.filter((f) => f.key !== 'src') as f (f.key)}{@render field(f)}{/each}
      </div>
    {:else}
      <div class="group">
        <div class="gh"><span class="k">Content</span><span class="dim small">bound to the canvas</span></div>
        <div class="fields">
          {#each bdef.fields as f (f.key)}{@render field(f)}{/each}
        </div>
      </div>
    {/if}
  {:else if section && def}
    <!-- ── A section ── -->
    <div class="group">
      <div class="gh"><span class="k">Block</span></div>
      <div class="ident">
        <span class="ident-icon"><Icon name={section.block === 'media/3d-embed' ? 'box' : 'layout'} size={14} /></span>
        <div class="ident-text"><strong>{sectionLabel(section)}</strong><span class="dim">{def.label.split(' · ')[1] ?? def.description}</span></div>
        {#if layoutsOf(section.block).length > 1}
          <button class="btn small" onclick={swapMenu} disabled={readonly} aria-haspopup="menu" data-testid="site-swap-layout">Swap layout <Icon name="chevronDown" size={12} /></button>
        {/if}
      </div>
      {#if section.derived_from}
        {@const d = ops.parseDerived(section.derived_from)}
        <p class="prov"><Icon name="link" size={12} /> From your library{d?.seq ? ` · v${d.seq}` : ''}
          {#if d}<button class="linkbtn" onclick={() => onopen(d.artifactId)}>Open source</button>{/if}
        </p>
      {/if}
    </div>

    {#if def.fields.length}
      <div class="group">
        <div class="gh"><span class="k">Content</span><span class="dim small">bound to the canvas</span></div>
        <div class="fields">
          {#each def.fields as f (f.key)}{@render field(f)}{/each}
        </div>
      </div>
    {/if}

    {#if def.item}
      {@const items = (section.blocks ?? []).filter((b) => b.block === def!.item)}
      {@const idef = itemDef(def.item)}
      <div class="group">
        <div class="gh"><span class="k">{idef?.label ?? 'Items'}s · {items.length}</span>
          {#if !readonly}<button class="btn small ghost" onclick={() => { const r = ops.addItem(doc, section!.id); onchange(r.doc); if (r.id) onselect(section!.id, r.id); }} data-testid="site-add-item"><Icon name="plus" size={12} /> Add</button>{/if}
        </div>
        <ul class="items">
          {#each items as b (b.id)}
            <li><button class="item" onclick={() => onselect(section!.id, b.id)}><span class="name">{itemTitle(b)}</span><Icon name="chevronRight" size={12} /></button></li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if def.media}
      <div class="group">
        <div class="gh"><span class="k">Media</span></div>
        {#if mediaChild}
          <button class="item" onclick={() => onselect(section!.id, mediaChild!.id)}>
            <Icon name={mediaChild.block === 'embed/3d' ? 'box' : 'image'} size={12} />
            <span class="name">{mediaChild.block === 'embed/3d' ? `3D · ${embed(str(mediaChild.props, 'src'))?.title ?? (str(mediaChild.props, 'label') || 'stand-in card')}` : `Image · ${str(mediaChild.props, 'alt') || 'untitled'}`}</span>
            <Icon name="chevronRight" size={12} />
          </button>
        {/if}
        {#if !readonly}
          <div class="row-btns">
            {#if def.media.includes('embed/3d')}<button class="btn small" onclick={(e) => addMedia('embed/3d', e)}><Icon name="box" size={12} /> {mediaChild?.block === 'embed/3d' ? 'Replace 3D…' : 'Use a 3D artifact…'}</button>{/if}
            {#if def.media.includes('embed/image')}<button class="btn small" onclick={(e) => addMedia('embed/image', e)}><Icon name="image" size={12} /> Use an image</button>{/if}
          </div>
        {/if}
      </div>
    {/if}

    <div class="group">
      <div class="gh"><span class="k">Style</span><span class="dim small from"><Icon name="palette" size={12} /> {kitLabel ? `from ${kitLabel}` : 'default palette'}</span></div>
      <div class="field">
        <span class="lbl">Background</span>
        <div class="swatches" role="group" aria-label="Background">
          <button class="sw none" class:on={!section.style?.background} onclick={() => setBg('')} disabled={readonly} aria-label="Default background" title="Default (surface)"></button>
          {#each brandSwatches as s (s.value)}
            <button class="sw" class:on={section.style?.background === s.value} style:background={s.color} onclick={() => setBg(s.value)} disabled={readonly} aria-label={`Brand ${s.label}`} title={`${s.label} (${s.color})`}></button>
          {/each}
          {#each GRADIENTS as g (g)}
            <button class="sw grad" data-g={g} class:on={section.style?.background === `gradient:${g}`} style:--sw-a={g === 'ink' ? theme.night : g === 'soft' ? theme.surfaceAlt : theme.primary} style:--sw-b={g === 'soft' ? theme.primary : theme.accent} onclick={() => setBg(`gradient:${g}`)} disabled={readonly} aria-label={`Gradient ${g}`} title={`Gradient · ${g}`}></button>
          {/each}
          <input class="input hex" placeholder="# hex" bind:value={hexDraft} disabled={readonly} aria-label="Custom colour (hex)" onchange={commitHex} />
        </div>
        {#if bg?.offBrand}
          <p class="warnchip" role="status"><Icon name="warning" size={12} /> Off-brand colour — not in {kitLabel ?? 'the palette'}.
            {#if nearest && !readonly}<button class="linkbtn" onclick={() => setBg(nearest!.value)}>Use {nearest.label}</button>{/if}
          </p>
        {:else if bg?.unknownToken}
          <p class="warnchip" role="status"><Icon name="warning" size={12} /> The brand kit has no “{bg.unknownToken}” — showing the default.</p>
        {/if}
      </div>
      <div class="field">
        <span class="lbl">Spacing</span>
        <div class="segmented full" role="group" aria-label="Spacing">
          {#each ['s', 'm', 'l', 'xl'] as sp (sp)}
            <button class:active={(section.style?.spacing || 'm') === sp} aria-pressed={(section.style?.spacing || 'm') === sp} disabled={readonly} onclick={() => onchange(ops.setStyle(doc, section!.id, 'spacing', sp as 's'))}>{sp.toUpperCase()}</button>
          {/each}
        </div>
      </div>
      <div class="field">
        <span class="lbl">Alignment</span>
        <div class="segmented full" role="group" aria-label="Alignment">
          {#each ['left', 'center'] as al (al)}
            <button class:active={(section.style?.align || 'left') === al} aria-pressed={(section.style?.align || 'left') === al} disabled={readonly} onclick={() => onchange(ops.setStyle(doc, section!.id, 'align', al as 'left'))}>{al === 'left' ? 'Left' : 'Center'}</button>
          {/each}
        </div>
      </div>
      <div class="field">
        <label for="si-motion">Motion</label>
        <select id="si-motion" class="input" value={section.style?.motion || 'none'} disabled={readonly} onchange={(e) => onchange(ops.setStyle(doc, section!.id, 'motion', e.currentTarget.value as SiteMotion))} data-testid="site-motion">
          {#each MOTIONS as m (m.value)}<option value={m.value}>{m.label}</option>{/each}
        </select>
        <span class="hint">{motionHint} Off for people who prefer reduced motion.</span>
      </div>
      {#if section.block.startsWith('hero/')}
        <div class="field">
          <span class="lbl">Min height</span>
          <div class="segmented full" role="group" aria-label="Min height">
            {#each ['auto', '80vh', '100vh'] as mh (mh)}
              <button class:active={(section.style?.min_height || 'auto') === mh} aria-pressed={(section.style?.min_height || 'auto') === mh} disabled={readonly} onclick={() => onchange(ops.setStyle(doc, section!.id, 'min_height', mh as 'auto'))}>{mh === 'auto' ? 'Auto' : mh}</button>
            {/each}
          </div>
        </div>
      {/if}
    </div>

    <div class="group">
      <div class="gh"><span class="k">Responsive</span></div>
      <div class="field">
        <span class="lbl">Show on</span>
        <div class="pills">
          {#each ['desktop', 'tablet', 'mobile'] as bp (bp)}
            {@const hidden = (section.responsive?.hide ?? []).includes(bp as 'mobile')}
            <button class="pill-toggle" class:on={!hidden} aria-pressed={!hidden} disabled={readonly} onclick={() => onchange(ops.toggleHide(doc, section!.id, bp as 'mobile'))}>
              <Icon name={hidden ? 'eyeOff' : 'eye'} size={12} /> {bp[0].toUpperCase() + bp.slice(1)}
            </button>
          {/each}
        </div>
      </div>
      {#if def.media || section.block === 'features/alternating'}
        <label class="checkbox-row toggle">
          <input type="checkbox" checked={section.responsive?.stack === 'media-first'} disabled={readonly} onchange={(e) => onchange(ops.setResponsive(doc, section!.id, 'stack', e.currentTarget.checked ? 'media-first' : undefined))} />
          On mobile, put the media above the text
        </label>
      {/if}
      <label class="checkbox-row toggle">
        <input type="checkbox" checked={section.responsive?.mobile_align === 'center'} disabled={readonly} onchange={(e) => onchange(ops.setResponsive(doc, section!.id, 'mobile_align', e.currentTarget.checked ? 'center' : undefined))} />
        Center the text on mobile
      </label>
    </div>

    {#if contrastNow}
      <div class="group">
        <div class="gh"><span class="k">Accessibility</span></div>
        <div class="pills">
          <span class="chip" class:ok={(contrastNow.text ?? 0) >= 4.5} class:bad={(contrastNow.text ?? 0) < 4.5} title="Body text on this background (WCAG AA needs 4.5:1)">
            <Icon name={(contrastNow.text ?? 0) >= 4.5 ? 'check' : 'warning'} size={12} /> Text {contrastNow.text ?? '–'}:1
          </span>
          <span class="chip" class:ok={(contrastNow.button ?? 0) >= 4.5} class:bad={(contrastNow.button ?? 0) < 4.5} title="Button label on its fill (WCAG AA needs 4.5:1)">
            <Icon name={(contrastNow.button ?? 0) >= 4.5 ? 'check' : 'warning'} size={12} /> Button {contrastNow.button ?? '–'}:1
          </span>
        </div>
      </div>
    {/if}

    {#if !readonly}
      <div class="group end">
        <button class="btn small danger" onclick={() => void deleteSection()} data-testid="site-delete-section"><Icon name="trash" size={12} /> Delete section</button>
      </div>
    {/if}
  {:else if page}
    <!-- ── Nothing selected: page + site ── -->
    <div class="group">
      <div class="gh"><span class="k">Page</span></div>
      <div class="field"><label for="sp-title">Title</label><input id="sp-title" class="input" value={page.title} disabled={readonly} oninput={(e) => setPage('title', e.currentTarget.value)} /></div>
      <div class="field">
        <label for="sp-slug">URL</label>
        {#if page === doc.pages[0]}
          <input id="sp-slug" class="input" value="/ (home)" disabled />
        {:else}
          <input id="sp-slug" class="input" value={page.slug} disabled={readonly} onchange={(e) => setPage('slug', e.currentTarget.value)} />
        {/if}
      </div>
      <div class="field"><label for="sp-desc">Description (search engines)</label><textarea id="sp-desc" class="input" rows="2" value={page.description ?? ''} disabled={readonly} oninput={(e) => setPage('description', e.currentTarget.value)}></textarea></div>
    </div>
    <div class="group">
      <div class="gh"><span class="k">Site</span></div>
      <div class="field"><label for="ss-title">Site name</label><input id="ss-title" class="input" value={doc.title ?? ''} disabled={readonly} oninput={(e) => setSite('title', e.currentTarget.value)} /></div>
      <div class="field half"><label for="ss-domain">Domain</label><input id="ss-domain" class="input" value={doc.settings?.domain ?? ''} placeholder="example.com" disabled={readonly} oninput={(e) => setSite('domain', e.currentTarget.value)} /></div>
      <div class="field half"><label for="ss-lang">Language</label><input id="ss-lang" class="input" value={doc.settings?.lang ?? 'en'} disabled={readonly} oninput={(e) => setSite('lang', e.currentTarget.value)} /></div>
    </div>
    <div class="group">
      <div class="gh"><span class="k">Brand</span>
        {#if !readonly}<button class="btn small ghost" onclick={(e) => void linkKit(e)} data-testid="site-link-kit"><Icon name="palette" size={12} /> {kitLabel ? 'Change…' : 'Link a brand kit…'}</button>{/if}
      </div>
      <p class="kit"><strong>{kitLabel ?? 'Default palette'}</strong>{#if brandNote}<span class="dim"> · {brandNote}</span>{/if}</p>
      <div class="kit-sw" aria-hidden="true">
        {#each [theme.primary, theme.accent, theme.ink, theme.surface, theme.surfaceAlt] as c, i (i)}<span style:background={c}></span>{/each}
      </div>
    </div>
    <div class="group">
      <div class="gh"><span class="k">Checks</span>
        <span class="dim small">{errors ? `${errors} to fix` : 'No blockers'}{warns ? ` · ${warns} warning${warns === 1 ? '' : 's'}` : ''}</span>
      </div>
      {#if findings.length}
        <ul class="findings">
          {#each findings as f (f.key)}
            <li>
              <button class="finding" data-level={f.level} onclick={() => f.sectionId && onselect(f.sectionId, f.blockId)} disabled={!f.sectionId}>
                <Icon name={f.level === 'error' ? 'warning' : 'info'} size={12} />
                <span>{f.message}</span>
              </button>
            </li>
          {/each}
        </ul>
      {:else}
        <p class="dim note"><Icon name="check" size={12} /> Contrast, text alternatives and headings look good.</p>
      {/if}
    </div>
  {/if}
</div>

<style>
  .insp {
    display: flex;
    flex-direction: column;
    min-height: 0;
  }
  .group {
    padding: 12px 14px;
    border-block-end: 1px solid var(--border);
  }
  .group.end {
    border-block-end: 0;
  }
  .gh {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    margin-block-end: 10px;
    min-height: 22px;
  }
  .k {
    font-size: var(--fs-xs);
    font-weight: 600;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-dim);
  }
  .small {
    font-size: var(--fs-xs);
  }
  .from {
    display: inline-flex;
    align-items: center;
    gap: 4px;
  }
  .dim {
    color: var(--text-dim);
  }
  .ident,
  .src {
    display: flex;
    align-items: center;
    gap: 10px;
  }
  .src {
    margin-block-end: 10px;
    padding: 8px;
    border: 1px solid var(--border);
    border-radius: var(--radius-m);
    background: var(--surface-2);
  }
  .src.broken {
    border-color: color-mix(in srgb, var(--warning) 45%, transparent);
  }
  .ident-icon,
  .src-icon {
    display: grid;
    place-items: center;
    flex: none;
    width: 32px;
    height: 32px;
    border-radius: var(--radius-s);
    background: color-mix(in srgb, var(--studio-site) 18%, transparent);
    color: var(--studio-site);
  }
  .src-icon {
    background: color-mix(in srgb, var(--studio-3d) 18%, transparent);
    color: var(--studio-3d);
  }
  .ident-text {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    font-size: var(--fs-m);
  }
  .ident-text .dim {
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .ident-actions {
    display: flex;
    gap: 2px;
  }
  .prov {
    display: flex;
    align-items: center;
    gap: 6px;
    margin: 10px 0 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .prov :global(svg) {
    color: var(--info);
  }
  .fields {
    display: flex;
    flex-wrap: wrap;
    column-gap: 10px;
  }
  .fields > :global(.field) {
    flex: 1 1 100%;
  }
  .fields > :global(.field.half) {
    flex: 1 1 calc(50% - 5px);
    min-width: 0;
  }
  .field {
    margin-block-end: 10px;
  }
  .field.half {
    display: inline-flex;
    width: calc(50% - 5px);
  }
  .field.half + .field.half {
    margin-inline-start: 6px;
  }
  .lbl {
    font-size: var(--fs-s);
    font-weight: 500;
    color: var(--text-dim);
  }
  .field .input {
    width: 100%;
    min-width: 0;
  }
  .hint.bad {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    color: var(--danger);
  }
  .with-btn {
    display: flex;
    gap: 6px;
  }
  .toggle {
    margin-block-end: 8px;
    font-size: var(--fs-s);
  }
  .linkrow {
    display: grid;
    grid-template-columns: 1fr 1.2fr auto;
    gap: 4px;
    margin-block-end: 4px;
  }
  .swatches {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
  }
  .sw {
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius-s);
    cursor: pointer;
  }
  .sw.on {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .sw:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 2px;
  }
  .sw.none {
    background: repeating-linear-gradient(45deg, var(--surface) 0 5px, var(--surface-2) 5px 10px);
  }
  .sw.grad {
    --sw-a: var(--surface-2);
    --sw-b: var(--surface-3);
    background: linear-gradient(135deg, var(--sw-a), var(--sw-b));
  }
  .sw.grad[data-g='soft'] {
    background: radial-gradient(circle at 80% 20%, color-mix(in srgb, var(--sw-b) 35%, transparent), transparent 70%), var(--sw-a);
  }
  .hex {
    width: 84px;
  }
  .warnchip {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    margin: 8px 0 0;
    padding: 6px 8px;
    border-radius: var(--radius-s);
    background: var(--warning-soft);
    color: var(--text);
    font-size: var(--fs-s);
  }
  .warnchip :global(svg) {
    color: var(--warning);
  }
  .segmented.full {
    display: flex;
  }
  .segmented.full > button {
    flex: 1;
  }
  .pills,
  .row-btns {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }
  .row-btns {
    margin-block-start: 8px;
  }
  .radios {
    display: flex;
    flex-direction: column;
    gap: 6px;
    font-size: var(--fs-m);
  }
  .radios label {
    display: flex;
    align-items: center;
    gap: 7px;
  }
  .note {
    margin: 8px 0 0;
    font-size: var(--fs-s);
    color: var(--text-dim);
  }
  .items,
  .findings {
    margin: 0;
    padding: 0;
    list-style: none;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .item,
  .finding {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    min-height: 28px;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--radius-s);
    background: transparent;
    color: var(--text);
    font-size: var(--fs-m);
    text-align: start;
    cursor: pointer;
  }
  .item:hover,
  .finding:hover:not(:disabled) {
    background: var(--hover);
  }
  .item .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .finding {
    align-items: flex-start;
    font-size: var(--fs-s);
  }
  .finding:disabled {
    cursor: default;
  }
  .finding :global(svg) {
    flex: none;
    margin-block-start: 2px;
    color: var(--text-dim);
  }
  .finding[data-level='error'] :global(svg) {
    color: var(--danger);
  }
  .finding[data-level='warn'] :global(svg) {
    color: var(--warning);
  }
  .kit {
    margin: 0 0 8px;
    font-size: var(--fs-m);
  }
  .kit-sw {
    display: flex;
    gap: 4px;
  }
  .kit-sw span {
    flex: 1;
    height: 22px;
    border-radius: var(--radius-s);
    border: 1px solid var(--border);
  }
  .linkbtn {
    display: inline-flex;
    align-items: center;
    gap: 2px;
    border: 0;
    background: none;
    padding: 0;
    color: var(--accent-text);
    font: inherit;
    font-size: var(--fs-s);
    cursor: pointer;
  }
  .linkbtn:hover {
    text-decoration: underline;
  }
</style>
