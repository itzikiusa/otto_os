// Site Studio — the live selection (page · section · block) of the site that
// is open, shared with the Otto co-design panel: the chat reads `payload`
// (the `selection` of `POST /design/artifacts/{id}/assist`) and `label`
// ("Section: Hero") for its context chip, and watches `request` — bumped by the
// section toolbar's "Ask Otto" / "Variants" buttons — to focus its composer.
// Site Studio is the only writer.

import { assistSelection, selectionLabel, type SiteAssistSelection, type SiteSelectionIds } from './engine/assist';
import type { SiteDoc } from './engine/types';

export interface SiteAskRequest {
  mode: 'refine' | 'variants';
  /** Increments on every click, so repeated asks are distinct. */
  seq: number;
  artifactId: string;
  label: string | null;
}

class SiteSelectionStore {
  /** The site artifact the selection belongs to (null = no site open). */
  artifactId = $state<string | null>(null);
  pageId = $state<string | null>(null);
  sectionId = $state<string | null>(null);
  blockId = $state<string | null>(null);
  /** "Section: Hero" / "Block: Gold · Tiers" / "Page: Home". */
  label = $state<string | null>(null);
  /** The assist `selection` payload (≤ 4 KB), or null. */
  payload = $state<SiteAssistSelection | null>(null);
  request = $state<SiteAskRequest | null>(null);

  get ids(): SiteSelectionIds {
    return { pageId: this.pageId, sectionId: this.sectionId, blockId: this.blockId };
  }

  set(artifactId: string, ids: SiteSelectionIds, doc: SiteDoc | null): void {
    this.artifactId = artifactId;
    this.pageId = ids.pageId;
    this.sectionId = ids.sectionId;
    this.blockId = ids.blockId;
    this.label = selectionLabel(doc, ids);
    this.payload = assistSelection(doc, ids);
  }

  /** Forget the selection when its site closes (only if it is still ours). */
  clear(artifactId: string): void {
    if (this.artifactId !== artifactId) return;
    this.artifactId = null;
    this.pageId = this.sectionId = this.blockId = null;
    this.label = null;
    this.payload = null;
  }

  ask(mode: SiteAskRequest['mode']): void {
    if (!this.artifactId) return;
    this.request = { mode, seq: (this.request?.seq ?? 0) + 1, artifactId: this.artifactId, label: this.label };
  }
}

export const siteSelection = new SiteSelectionStore();
