// Site Studio — deterministic checks the inspector and the publish sheet show:
// WCAG contrast against the brand tokens, missing text alternatives, heading
// structure, off-brand colours and placeholder links. Nothing here calls an
// agent; "Check accessibility" in the Otto panel can build on these.

import { sectionLabel } from './catalog';
import { lines, links, str } from './render';
import { resolveBackground, sectionContrast, type Theme } from './theme';
import type { SiteDoc, SitePage } from './types';

export type AuditLevel = 'error' | 'warn' | 'info';

export interface AuditFinding {
  level: AuditLevel;
  /** Stable key (tests, dedupe): `<rule>:<section>[:<block>]`. */
  key: string;
  sectionId: string | null;
  blockId: string | null;
  message: string;
}

const MIN_TEXT = 4.5;

export function auditPage(doc: SiteDoc, page: SitePage, t: Theme): AuditFinding[] {
  const out: AuditFinding[] = [];
  const add = (level: AuditLevel, rule: string, sectionId: string | null, blockId: string | null, message: string) =>
    out.push({ level, key: `${rule}:${sectionId ?? ''}${blockId ? ':' + blockId : ''}`, sectionId, blockId, message });

  const visible = page.sections.filter((s) => !s.hidden);
  const heroes = visible.filter((s) => s.block.startsWith('hero/'));
  if (visible.length && heroes.length === 0) add('warn', 'no-h1', null, null, `“${page.title || 'This page'}” has no hero, so it has no main heading (h1).`);
  if (heroes.length > 1) add('warn', 'many-h1', heroes[1].id, null, `${heroes.length} heroes on one page — screen readers expect a single main heading.`);

  let placeholders = 0;
  for (const s of visible) {
    const name = sectionLabel(s);
    const c = sectionContrast(s, t);
    if (c.text != null && c.text < MIN_TEXT) {
      add('error', 'contrast-text', s.id, null, `${name}: text contrast is ${c.text}:1 on this background (needs ${MIN_TEXT}:1).`);
    }
    const hasButton = ['primary_label', 'cta_label', 'button_label'].some((k) => str(s.props, k).trim()) || (s.blocks ?? []).some((b) => str(b.props, 'cta_label').trim());
    if (hasButton && c.button != null && c.button < MIN_TEXT) {
      add('error', 'contrast-button', s.id, null, `${name}: button label contrast is ${c.button}:1 (needs ${MIN_TEXT}:1).`);
    }
    const bg = resolveBackground(s.style?.background, t);
    if (bg.offBrand) add('warn', 'off-brand', s.id, null, `${name}: background ${s.style?.background} is not a brand colour.`);
    if (bg.unknownToken) add('warn', 'unknown-token', s.id, null, `${name}: the brand kit has no token “${bg.unknownToken}”.`);
    if (s.block.startsWith('hero/') && !str(s.props, 'headline').trim()) add('error', 'empty-headline', s.id, null, `${name}: the headline is empty.`);
    for (const b of s.blocks ?? []) {
      if (b.block === 'embed/image' && str(b.props, 'src').trim() && !str(b.props, 'alt').trim()) {
        add('error', 'alt', s.id, b.id, `${name}: an image has no text alternative.`);
      }
      if (b.block === 'embed/3d') {
        if (!str(b.props, 'alt').trim()) add('error', 'alt-3d', s.id, b.id, `${name}: the 3D embed has no text alternative.`);
        if (!str(b.props, 'src').trim()) add('info', 'no-3d', s.id, b.id, `${name}: no 3D artifact picked yet — a stand-in card is shown.`);
      }
      for (const k of ['href', 'cta_href']) if (str(b.props, k).trim() === '#') placeholders++;
      placeholders += links(b.props, 'links').filter((l) => l.href.trim() === '#').length;
      if (b.block === 'item/tier' && lines(b.props, 'features').length === 0) add('info', 'tier-empty', s.id, b.id, `${name}: a tier lists no perks.`);
    }
    for (const k of ['primary_href', 'secondary_href', 'cta_href']) {
      if (str(s.props, k).trim() === '#' && str(s.props, k.replace('_href', '_label')).trim()) placeholders++;
    }
    placeholders += links(s.props, 'links').filter((l) => l.href.trim() === '#').length;
  }
  if (placeholders) add('info', 'placeholder-links', null, null, `${placeholders} link${placeholders === 1 ? '' : 's'} still point to “#”.`);
  void doc;
  return out;
}

/** Findings for every page, errors first. */
export function auditSite(doc: SiteDoc, t: Theme): (AuditFinding & { pageId: string })[] {
  const rank: Record<AuditLevel, number> = { error: 0, warn: 1, info: 2 };
  return doc.pages
    .flatMap((p) => auditPage(doc, p, t).map((f) => ({ ...f, pageId: p.id })))
    .sort((a, b) => rank[a.level] - rank[b.level]);
}
