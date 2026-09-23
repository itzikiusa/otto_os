// Site Studio — pure document operations. Every function returns a NEW
// document (the input is never mutated), so the editor's undo stack is just a
// list of documents and Svelte sees a fresh object on every change.

import { itemDef, sectionDef } from './catalog';
import { validateSite } from './validate';
import type { SiteBlock, SiteDoc, SitePage, SiteProps, SiteResponsive, SiteSection, SiteStyle, SiteIssue } from './types';

export function clone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}

export function emptySite(title = ''): SiteDoc {
  return { type: 'otto-site', version: 1, title, settings: {}, pages: [] };
}

export interface ParsedSite {
  doc: SiteDoc | null;
  issues: SiteIssue[];
}

/** Parse + validate the stored JSON; a doc without `pages` reads as empty. */
export function parseSite(source: string | null): ParsedSite {
  if (source == null || !source.trim()) return { doc: emptySite(), issues: [] };
  let raw: unknown;
  try {
    raw = JSON.parse(source);
  } catch (e) {
    return { doc: null, issues: [{ path: '', message: `not valid JSON: ${e instanceof Error ? e.message : String(e)}` }] };
  }
  const issues = validateSite(raw);
  if (issues.length) return { doc: null, issues };
  const doc = raw as SiteDoc;
  if (!Array.isArray(doc.pages)) doc.pages = [];
  for (const p of doc.pages) {
    p.title ??= '';
    p.slug ??= '';
    p.sections ??= [];
    for (const s of p.sections) {
      s.block ??= '';
      s.props ??= {};
      for (const b of s.blocks ?? []) {
        b.block ??= '';
        b.props ??= {};
      }
    }
  }
  return { doc, issues: [] };
}

export function serializeSite(doc: SiteDoc): string {
  return JSON.stringify(doc, null, 2) + '\n';
}

export function isEmptySite(doc: SiteDoc): boolean {
  return doc.pages.length === 0 || doc.pages.every((p) => p.sections.length === 0);
}

// ── Ids ──────────────────────────────────────────────────────────────────────

export function allIds(doc: SiteDoc): Set<string> {
  const ids = new Set<string>();
  for (const p of doc.pages) {
    ids.add(p.id);
    for (const s of p.sections) {
      ids.add(s.id);
      for (const b of s.blocks ?? []) ids.add(b.id);
    }
  }
  return ids;
}

/** A short readable id (`hero-k3f9`) unique within `taken`. */
export function freshId(prefix: string, taken: Set<string>): string {
  const base = prefix.replace(/[^A-Za-z0-9_-]+/g, '-').replace(/^-+|-+$/g, '').slice(0, 40) || 'node';
  for (;;) {
    const id = `${base}-${Math.random().toString(36).slice(2, 6)}`;
    if (!taken.has(id)) {
      taken.add(id);
      return id;
    }
  }
}

/** Give a section and its blocks fresh ids (duplicate / insert from library). */
export function reId(s: SiteSection, taken: Set<string>): SiteSection {
  const out = clone(s);
  out.id = freshId(s.block.split('/')[0], taken);
  out.blocks = (out.blocks ?? []).map((b) => ({ ...b, id: freshId(b.block.split('/')[1] ?? 'block', taken) }));
  return out;
}

// ── Lookup ───────────────────────────────────────────────────────────────────

export interface SectionAt {
  page: SitePage;
  pageIndex: number;
  section: SiteSection;
  index: number;
}

export function findSection(doc: SiteDoc, id: string | null | undefined): SectionAt | null {
  if (!id) return null;
  for (let pi = 0; pi < doc.pages.length; pi++) {
    const page = doc.pages[pi];
    const index = page.sections.findIndex((s) => s.id === id);
    if (index >= 0) return { page, pageIndex: pi, section: page.sections[index], index };
  }
  return null;
}

/** The section that holds block `blockId`. */
export function findBlock(doc: SiteDoc, blockId: string | null | undefined): (SectionAt & { block: SiteBlock; blockIndex: number }) | null {
  if (!blockId) return null;
  for (let pi = 0; pi < doc.pages.length; pi++) {
    const page = doc.pages[pi];
    for (let i = 0; i < page.sections.length; i++) {
      const s = page.sections[i];
      const bi = (s.blocks ?? []).findIndex((b) => b.id === blockId);
      if (bi >= 0) return { page, pageIndex: pi, section: s, index: i, block: s.blocks![bi], blockIndex: bi };
    }
  }
  return null;
}

// ── Sections ─────────────────────────────────────────────────────────────────

/** A new section of `block` with the library's starter content. */
export function makeSection(block: string, taken: Set<string>): SiteSection {
  const def = sectionDef(block);
  if (!def) throw new Error(`unknown block ${block}`);
  const s: SiteSection = {
    id: freshId(def.family, taken),
    block,
    props: clone(def.props),
    style: clone(def.style),
    blocks: def.blocks.map((b) => ({ id: freshId(b.block.split('/')[1] ?? 'block', taken), block: b.block, props: clone(b.props) })),
  };
  return s;
}

function mapPage(doc: SiteDoc, pageId: string, f: (p: SitePage) => SitePage): SiteDoc {
  const next = clone(doc);
  next.pages = next.pages.map((p) => (p.id === pageId ? f(p) : p));
  return next;
}

function mapSection(doc: SiteDoc, sectionId: string, f: (s: SiteSection) => SiteSection): SiteDoc {
  const next = clone(doc);
  for (const p of next.pages) p.sections = p.sections.map((s) => (s.id === sectionId ? f(s) : s));
  return next;
}

/** Insert `section` into page `pageId` at `index` (clamped; -1 / past the end = append). */
export function insertSection(doc: SiteDoc, pageId: string, section: SiteSection, index: number): SiteDoc {
  return mapPage(doc, pageId, (p) => {
    const at = index < 0 || index > p.sections.length ? p.sections.length : index;
    p.sections.splice(at, 0, section);
    return p;
  });
}

export function removeSection(doc: SiteDoc, sectionId: string): SiteDoc {
  const next = clone(doc);
  for (const p of next.pages) p.sections = p.sections.filter((s) => s.id !== sectionId);
  return next;
}

/** Move a section within its page to `to` (0-based, clamped). */
export function moveSectionTo(doc: SiteDoc, sectionId: string, to: number): SiteDoc {
  const at = findSection(doc, sectionId);
  if (!at) return doc;
  return mapPage(doc, at.page.id, (p) => {
    const [s] = p.sections.splice(at.index, 1);
    const dest = Math.max(0, Math.min(p.sections.length, to));
    p.sections.splice(dest, 0, s);
    return p;
  });
}

export function moveSection(doc: SiteDoc, sectionId: string, delta: number): SiteDoc {
  const at = findSection(doc, sectionId);
  return at ? moveSectionTo(doc, sectionId, at.index + delta) : doc;
}

export function duplicateSection(doc: SiteDoc, sectionId: string): { doc: SiteDoc; id: string | null } {
  const at = findSection(doc, sectionId);
  if (!at) return { doc, id: null };
  const copy = reId(at.section, allIds(doc));
  delete copy.derived_from;
  return { doc: insertSection(doc, at.page.id, copy, at.index + 1), id: copy.id };
}

export function updateSection(doc: SiteDoc, sectionId: string, f: (s: SiteSection) => void): SiteDoc {
  return mapSection(doc, sectionId, (s) => {
    f(s);
    return s;
  });
}

export function setSectionProp(doc: SiteDoc, sectionId: string, key: string, value: SiteProps[string]): SiteDoc {
  return updateSection(doc, sectionId, (s) => {
    s.props = { ...(s.props ?? {}), [key]: value };
  });
}

export function setStyle<K extends keyof SiteStyle>(doc: SiteDoc, sectionId: string, key: K, value: SiteStyle[K] | undefined): SiteDoc {
  return updateSection(doc, sectionId, (s) => {
    const st: SiteStyle = { ...(s.style ?? {}) };
    if (value === undefined || value === '') delete st[key];
    else st[key] = value;
    s.style = st;
  });
}

export function setResponsive<K extends keyof SiteResponsive>(doc: SiteDoc, sectionId: string, key: K, value: SiteResponsive[K] | undefined): SiteDoc {
  return updateSection(doc, sectionId, (s) => {
    const r: SiteResponsive = { ...(s.responsive ?? {}) };
    if (value === undefined || value === '' || (Array.isArray(value) && value.length === 0)) delete r[key];
    else r[key] = value;
    if (Object.keys(r).length) s.responsive = r;
    else delete s.responsive;
  });
}

/** Toggle "hidden at breakpoint". */
export function toggleHide(doc: SiteDoc, sectionId: string, bp: 'desktop' | 'tablet' | 'mobile'): SiteDoc {
  const at = findSection(doc, sectionId);
  if (!at) return doc;
  const cur = at.section.responsive?.hide ?? [];
  const next = cur.includes(bp) ? cur.filter((x) => x !== bp) : [...cur, bp];
  return setResponsive(doc, sectionId, 'hide', next);
}

/**
 * Swap a section to another layout of the same family ("Swap layout ▾"):
 * copy survives (props are shared per family), missing props get the new
 * layout's defaults, and child blocks carry over when the new layout repeats
 * the same item type or has a media slot for them.
 */
export function swapLayout(doc: SiteDoc, sectionId: string, block: string): SiteDoc {
  const def = sectionDef(block);
  if (!def) return doc;
  const taken = allIds(doc);
  return updateSection(doc, sectionId, (s) => {
    const props: SiteProps = { ...clone(def.props) };
    for (const [k, v] of Object.entries(s.props ?? {})) props[k] = v;
    const keep = (s.blocks ?? []).filter((b) => b.block === def.item || (def.media ?? []).includes(b.block));
    s.block = block;
    s.props = props;
    s.blocks = keep.length
      ? keep
      : def.blocks.map((b) => ({ id: freshId(b.block.split('/')[1] ?? 'block', taken), block: b.block, props: clone(b.props) }));
  });
}

// ── Child blocks ─────────────────────────────────────────────────────────────

export function setBlockProp(doc: SiteDoc, blockId: string, key: string, value: SiteProps[string]): SiteDoc {
  const at = findBlock(doc, blockId);
  if (!at) return doc;
  return updateSection(doc, at.section.id, (s) => {
    s.blocks = (s.blocks ?? []).map((b) => (b.id === blockId ? { ...b, props: { ...b.props, [key]: value } } : b));
  });
}

/** Append a new item of the section's repeat type (after `afterId` when given). */
export function addItem(doc: SiteDoc, sectionId: string, afterId?: string | null): { doc: SiteDoc; id: string | null } {
  const at = findSection(doc, sectionId);
  const type = at ? sectionDef(at.section.block)?.item : undefined;
  const def = type ? itemDef(type) : undefined;
  if (!at || !def) return { doc, id: null };
  const id = freshId(def.block.split('/')[1] ?? 'item', allIds(doc));
  const next = updateSection(doc, sectionId, (s) => {
    const list = [...(s.blocks ?? [])];
    const after = afterId ? list.findIndex((b) => b.id === afterId) : -1;
    const copyFrom = after >= 0 ? list[after] : [...list].reverse().find((b) => b.block === def.block);
    const block: SiteBlock = { id, block: def.block, props: clone(copyFrom?.block === def.block ? copyFrom.props : def.defaults) };
    list.splice(after >= 0 ? after + 1 : list.length, 0, block);
    s.blocks = list;
  });
  return { doc: next, id };
}

export function removeBlock(doc: SiteDoc, blockId: string): SiteDoc {
  const at = findBlock(doc, blockId);
  if (!at) return doc;
  return updateSection(doc, at.section.id, (s) => {
    s.blocks = (s.blocks ?? []).filter((b) => b.id !== blockId);
  });
}

export function moveBlock(doc: SiteDoc, blockId: string, delta: number): SiteDoc {
  const at = findBlock(doc, blockId);
  if (!at) return doc;
  return updateSection(doc, at.section.id, (s) => {
    const list = [...(s.blocks ?? [])];
    const to = Math.max(0, Math.min(list.length - 1, at.blockIndex + delta));
    const [b] = list.splice(at.blockIndex, 1);
    list.splice(to, 0, b);
    s.blocks = list;
  });
}

/** Put a media child (3D / image) into a section's media slot, replacing the old one. */
export function setMedia(doc: SiteDoc, sectionId: string, block: 'embed/3d' | 'embed/image', props: SiteProps): { doc: SiteDoc; id: string } {
  const taken = allIds(doc);
  const id = freshId(block.split('/')[1], taken);
  const next = updateSection(doc, sectionId, (s) => {
    const media = sectionDef(s.block)?.media ?? [];
    const rest = (s.blocks ?? []).filter((b) => !media.includes(b.block));
    s.blocks = [{ id, block, props: { ...clone(itemDef(block)?.defaults ?? {}), ...props } }, ...rest];
  });
  return { doc: next, id };
}

// ── Pages ────────────────────────────────────────────────────────────────────

export function slugify(s: string): string {
  return s
    .toLowerCase()
    .normalize('NFKD')
    .replace(/[̀-ͯ]/g, '')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .slice(0, 64);
}

export function addPage(doc: SiteDoc, title: string): { doc: SiteDoc; id: string } {
  const taken = allIds(doc);
  const id = freshId('page', taken);
  const slugs = new Set(doc.pages.map((p) => p.slug));
  let slug = slugify(title) || 'page';
  if (slug === 'index') slug = 'index-page';
  for (let n = 2; slugs.has(slug); n++) slug = `${slugify(title) || 'page'}-${n}`;
  const next = clone(doc);
  next.pages.push({ id, title: title.trim() || 'Untitled page', slug: next.pages.length === 0 ? '' : slug, sections: [] });
  return { doc: next, id };
}

export function renamePage(doc: SiteDoc, pageId: string, title: string): SiteDoc {
  return mapPage(doc, pageId, (p) => ({ ...p, title }));
}

export function removePage(doc: SiteDoc, pageId: string): SiteDoc {
  const next = clone(doc);
  next.pages = next.pages.filter((p) => p.id !== pageId);
  if (next.pages[0]) next.pages[0].slug = next.pages[0].slug === 'index' ? '' : next.pages[0].slug;
  return next;
}

// ── From your library ────────────────────────────────────────────────────────

/**
 * Copy a section out of ANOTHER site (with fresh ids) and stamp its
 * provenance, so the save extracts a pinned `derived_from` link to
 * `otto://design/<site>@v<seq>#<section>`.
 */
export function fromLibrary(section: SiteSection, siteId: string, seq: number, taken: Set<string>): SiteSection {
  const copy = reId(section, taken);
  copy.derived_from = `otto://design/${siteId}@v${seq}#${section.id}`;
  return copy;
}

/** `otto://design/<id>@v<seq>#<node>` → its parts (null when not that shape). */
export function parseDerived(uri: string | undefined): { artifactId: string; seq: number | null; node: string | null } | null {
  if (!uri) return null;
  const m = /^otto:\/\/design\/([A-Za-z0-9_-]{1,64})(?:@v([1-9][0-9]*))?(?:#([A-Za-z0-9_:.-]{1,128}))?$/.exec(uri);
  return m ? { artifactId: m[1], seq: m[2] ? Number(m[2]) : null, node: m[3] ?? null } : null;
}
