// Mockup Assistant store — drives the in-place "Create with AI" / "Refine" mockup
// agent on the Product → Mockups tab. Mirrors the Canvas store's live-session
// model: a turn POSTs to /product/stories/{sid}/mockups/assist; the backend
// surfaces the agent session at turn START (mockup_session_started → sessionId, so
// the embedded Terminal attaches live) and streams the source as it's written
// (mockup_updated → liveContent, for a live preview). The committed mockup is a
// kind:'mockup' attachment; MockupsTab reloads to show it.

import { api, authedBlobUrl } from '../api/client';
import type { ProductAttachment, ProductMockupAssistReq } from '../../modules/product/types';

export type MockupFormat = 'html' | 'mermaid';

class MockupAssistStore {
  /** The Assistant panel is open (covers the Mockups stage). */
  active = $state(false);
  /** The story we're authoring a mockup for. */
  storyId = $state<string | null>(null);
  /** The mockup attachment being edited — null until the first turn mints it. */
  attachmentId = $state<string | null>(null);
  /** Format of the current mockup (locked once it exists). */
  format = $state<MockupFormat>('html');
  /** The live agent session id (set at turn start) — drives the embedded shell. */
  sessionId = $state<string | null>(null);
  /** The live source being written by the agent — drives the live preview. */
  liveContent = $state<string>('');
  /** A turn is in flight. */
  busy = $state(false);
  /** The committed attachment from the last turn (so MockupsTab can select it). */
  lastResult = $state<ProductAttachment | null>(null);

  /** Open the panel to CREATE a brand-new mockup of `format`. */
  openNew(storyId: string, format: MockupFormat): void {
    this.active = true;
    this.storyId = storyId;
    this.attachmentId = null;
    this.format = format;
    this.sessionId = null;
    this.liveContent = '';
    this.lastResult = null;
  }

  /** Open the panel to REFINE an existing agent mockup attachment. */
  async openRefine(att: ProductAttachment): Promise<void> {
    this.active = true;
    this.storyId = att.story_id;
    this.attachmentId = att.id;
    this.format = formatOf(att);
    this.sessionId = null;
    this.liveContent = '';
    this.lastResult = att;
    // Seed the preview with the mockup's current bytes (best-effort).
    try {
      const url = await authedBlobUrl(`/product/attachments/${att.id}`);
      const res = await fetch(url);
      this.liveContent = await res.text();
      URL.revokeObjectURL(url);
    } catch {
      /* preview will fill in on the first mockup_updated */
    }
  }

  /** Run one agent turn. Returns the committed attachment (or throws). */
  async ask(prompt: string): Promise<ProductAttachment> {
    if (!this.storyId) throw new Error('No story selected');
    this.busy = true;
    try {
      const body: ProductMockupAssistReq = {
        prompt,
        ...(this.attachmentId ? { mockup_id: this.attachmentId } : { format: this.format }),
      };
      const att = await api.post<ProductAttachment>(
        `/product/stories/${this.storyId}/mockups/assist`,
        body,
      );
      this.attachmentId = att.id;
      this.format = formatOf(att);
      this.lastResult = att;
      return att;
    } finally {
      this.busy = false;
    }
  }

  /** mockup_session_started → attach the live shell. For a brand-new mockup the
   *  attachment id is minted server-side, so we don't know it until the POST
   *  returns — adopt the id from the event when ours is still null and the story
   *  matches (otherwise the live shell/preview would miss the mid-POST events). */
  setSession(attId: string, storyId: string, sid: string): void {
    if (this.adopt(attId, storyId)) this.sessionId = sid;
  }

  /** mockup_updated → live preview (same adoption rule as setSession). */
  ingestLive(attId: string, storyId: string, format: string, content: string): void {
    if (this.adopt(attId, storyId)) {
      if (format === 'html' || format === 'mermaid') this.format = format;
      this.liveContent = content;
    }
  }

  /** True when an `attachment_id` from a live event belongs to this panel — either
   *  it already IS our mockup, or we're mid-create (no id yet, a turn IN FLIGHT)
   *  for this story and adopt it. Gating on `busy` means a late event from a
   *  PREVIOUS mockup of the same story can't be mis-adopted into a fresh create. */
  private adopt(attId: string, storyId: string): boolean {
    if (attId === this.attachmentId) return true;
    if (this.active && this.busy && this.attachmentId === null && storyId === this.storyId) {
      this.attachmentId = attId;
      return true;
    }
    return false;
  }

  close(): void {
    this.active = false;
    this.sessionId = null;
  }
}

/** A mockup attachment's format, from its meta_json (default html). */
function formatOf(att: ProductAttachment): MockupFormat {
  try {
    const meta = att.meta_json ? (JSON.parse(att.meta_json) as { format?: string }) : null;
    if (meta?.format === 'mermaid') return 'mermaid';
  } catch {
    /* fall through */
  }
  // Fall back to the mime / filename.
  const mime = (att.mime || '').toLowerCase();
  if (mime === 'text/vnd.mermaid' || (att.filename || '').toLowerCase().endsWith('.mmd')) {
    return 'mermaid';
  }
  return 'html';
}

export const mockupAssist = new MockupAssistStore();
