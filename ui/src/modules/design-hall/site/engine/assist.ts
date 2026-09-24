// Site Studio — what the co-design chat sends as `selection` with
// `POST /design/artifacts/{id}/assist` (≤ 4 KB JSON, see docs/contracts/api.md
// § Design assist). Pure, so it is unit-tested; the live store that tracks the
// selection is ../selection.svelte.ts.

import { sectionLabel } from './catalog';
import { findBlock, findSection } from './ops';
import type { SiteDoc } from './types';

export interface SiteSelectionIds {
  pageId: string | null;
  sectionId: string | null;
  blockId: string | null;
}

export interface SiteAssistSelection {
  format: 'otto-site';
  page: { id: string; title: string } | null;
  section: { id: string; block: string; name: string } | null;
  block: { id: string; block: string } | null;
  /** JSON-ish path of the selected node inside the document. */
  path: string | null;
  /** The selected node's JSON (dropped when it would push past the cap). */
  node?: unknown;
}

/** Bytes the assist route accepts for `selection` (keep a margin under 4 KB). */
export const ASSIST_SELECTION_CAP = 3800;

export function assistSelection(doc: SiteDoc | null, sel: SiteSelectionIds): SiteAssistSelection | null {
  if (!doc) return null;
  const inBlock = findBlock(doc, sel.blockId);
  const inSection = inBlock ?? findSection(doc, sel.sectionId);
  const page = inSection?.page ?? doc.pages.find((p) => p.id === sel.pageId) ?? null;
  if (!page) return null;
  const out: SiteAssistSelection = {
    format: 'otto-site',
    page: { id: page.id, title: page.title },
    section: inSection ? { id: inSection.section.id, block: inSection.section.block, name: sectionLabel(inSection.section) } : null,
    block: inBlock ? { id: inBlock.block.id, block: inBlock.block.block } : null,
    path: inBlock
      ? `pages[${inBlock.pageIndex}].sections[${inBlock.index}].blocks[${inBlock.blockIndex}]`
      : inSection
        ? `pages[${inSection.pageIndex}].sections[${inSection.index}]`
        : `pages[${doc.pages.indexOf(page)}]`,
  };
  const node = inBlock ? inBlock.block : inSection ? inSection.section : null;
  if (node) {
    const withNode = { ...out, node };
    if (JSON.stringify(withNode).length <= ASSIST_SELECTION_CAP) return withNode;
  }
  return out;
}

/** "Section: Hero" / "Block: Gold · Tiers" — the chat's context chip. */
export function selectionLabel(doc: SiteDoc | null, sel: SiteSelectionIds): string | null {
  if (!doc) return null;
  const b = findBlock(doc, sel.blockId);
  if (b) {
    const p = b.block.props;
    const title = typeof p.title === 'string' ? p.title : typeof p.name === 'string' ? p.name : typeof p.question === 'string' ? p.question : b.block.block;
    return `Block: ${title} · ${sectionLabel(b.section)}`;
  }
  const s = findSection(doc, sel.sectionId);
  if (s) return `Section: ${sectionLabel(s.section)}`;
  const page = doc.pages.find((p) => p.id === sel.pageId);
  return page ? `Page: ${page.title || 'Untitled'}` : null;
}
