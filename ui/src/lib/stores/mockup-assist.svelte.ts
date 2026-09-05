// Design Assistant store — drives the in-place "Create with AI" / "Refine" design
// agent on the Product → Design tab (the arena). Mirrors the Canvas store's
// live-session model: a turn POSTs to /product/stories/{sid}/mockups/assist; the
// backend surfaces the agent session at turn START (mockup_session_started →
// sessionId, so the embedded Terminal attaches live) and streams the source as
// it's written (mockup_updated → liveContent, for a live preview). The committed
// artifact is a kind:'mockup'|'design' attachment; the arena reloads to show it.
//
// It also acts as the LIVE-UPDATE BUS for the arena: every `mockup_updated` (from
// the assistant, a swarm agent's `otto-mockup`, a Blender render output, or a
// content PUT from another client) lands in `lastUpdate` so the open artifact can
// reload — or, when the user has unsaved local edits, ask before replacing.

import { api, authedText } from '../api/client';
import {
  DESIGN_FORMATS,
  type DesignFormat,
  type DesignMeta,
  type ProductAttachment,
  type ProductMockupAssistReq,
} from '../../modules/product/types';

/** Back-compat alias — the old two-value union is now the full design enum. */
export type MockupFormat = DesignFormat;

/** One live `mockup_updated` as seen by the arena. `content === null` means the
 *  payload was binary/oversized and the bytes must be re-fetched. */
export interface LiveUpdate {
  attachmentId: string;
  storyId: string;
  format: string;
  content: string | null;
  /** Monotonic — lets an `$effect` react to two identical payloads in a row. */
  tick: number;
}

class MockupAssistStore {
  /** The Assistant panel is open (docked in the arena's inspector column). */
  active = $state(false);
  /** The story we're authoring an artifact for. */
  storyId = $state<string | null>(null);
  /** The attachment being edited — null until the first turn mints it. */
  attachmentId = $state<string | null>(null);
  /** Format of the current artifact (locked once it exists). */
  format = $state<DesignFormat>('html');
  /** The live agent session id (set at turn start) — drives the embedded shell. */
  sessionId = $state<string | null>(null);
  /** The live source being written by the agent — drives the live preview. */
  liveContent = $state<string>('');
  /** A turn is in flight. */
  busy = $state(false);
  /** The committed attachment from the last turn (so the arena can select it). */
  lastResult = $state<ProductAttachment | null>(null);
  /** The most recent live update for ANY artifact (the arena filters by id). */
  lastUpdate = $state<LiveUpdate | null>(null);
  private tick = 0;

  /** Open the panel to CREATE a brand-new artifact of `format`. */
  openNew(storyId: string, format: DesignFormat): void {
    this.active = true;
    this.storyId = storyId;
    this.attachmentId = null;
    this.format = format;
    this.sessionId = null;
    this.liveContent = '';
    this.lastResult = null;
  }

  /** Open the panel to REFINE an existing agent artifact. */
  async openRefine(att: ProductAttachment): Promise<void> {
    this.active = true;
    this.storyId = att.story_id;
    this.attachmentId = att.id;
    this.format = formatOf(att);
    this.sessionId = null;
    this.liveContent = '';
    this.lastResult = att;
    // Seed the preview with the artifact's current bytes (best-effort).
    try {
      this.liveContent = await authedText(`/product/attachments/${att.id}`);
    } catch {
      /* preview will fill in on the first mockup_updated */
    }
  }

  /** Run one agent turn. Returns the committed attachment (or throws). The
   *  `provider` is honored only on the FIRST turn (new artifact → new session); a
   *  refine resumes the existing session regardless. */
  async ask(prompt: string, provider?: string, model?: string): Promise<ProductAttachment> {
    if (!this.storyId) throw new Error('No story selected');
    this.busy = true;
    try {
      const body: ProductMockupAssistReq = {
        prompt,
        ...(this.attachmentId ? { mockup_id: this.attachmentId } : { format: this.format }),
        provider: provider || undefined,
        model: model || undefined,
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

  /** mockup_session_started → attach the live shell. For a brand-new artifact the
   *  attachment id is minted server-side, so we don't know it until the POST
   *  returns — adopt the id from the event when ours is still null and the story
   *  matches (otherwise the live shell/preview would miss the mid-POST events). */
  setSession(attId: string, storyId: string, sid: string): void {
    if (this.adopt(attId, storyId)) this.sessionId = sid;
  }

  /** mockup_updated → live preview (same adoption rule as setSession) + the
   *  arena bus. A `null` content (binary / oversized payload) is re-fetched as
   *  text for the preview only when the format is textual; the bus always
   *  carries the raw `null` so the arena knows to reload from the server. */
  ingestLive(attId: string, storyId: string, format: string, content: string | null): void {
    this.lastUpdate = { attachmentId: attId, storyId, format, content, tick: ++this.tick };
    if (!this.adopt(attId, storyId)) return;
    if (isDesignFormat(format)) this.format = format;
    if (content !== null) {
      this.liveContent = content;
    } else if (isDesignFormat(format)) {
      void authedText(`/product/attachments/${attId}`)
        .then((text) => {
          if (this.attachmentId === attId) this.liveContent = text;
        })
        .catch(() => {
          /* the committed attachment still reloads via lastUpdate */
        });
    }
  }

  /** True when an `attachment_id` from a live event belongs to this panel — either
   *  it already IS our artifact, or we're mid-create (no id yet, a turn IN FLIGHT)
   *  for this story and adopt it. Gating on `busy` means a late event from a
   *  PREVIOUS artifact of the same story can't be mis-adopted into a fresh create. */
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

export function isDesignFormat(f: string | null | undefined): f is DesignFormat {
  return !!f && (DESIGN_FORMATS as readonly string[]).includes(f);
}

/** Parse an attachment's `meta_json` (never throws). */
export function metaOf(att: ProductAttachment): DesignMeta {
  try {
    return att.meta_json ? (JSON.parse(att.meta_json) as DesignMeta) : {};
  } catch {
    return {};
  }
}

/** An attachment's design format, from meta_json first, then mime / filename.
 *  Returns `null` for non-design bytes (images, glb, arbitrary files). */
export function designFormatOf(att: ProductAttachment): DesignFormat | null {
  const meta = metaOf(att);
  if (isDesignFormat(meta.format)) return meta.format;
  const mime = (att.mime || '').toLowerCase();
  const name = (att.filename || '').toLowerCase();
  if (mime === 'text/vnd.mermaid' || name.endsWith('.mmd')) return 'mermaid';
  if (mime === 'application/vnd.excalidraw+json' || name.endsWith('.excalidraw')) return 'excalidraw';
  if (mime === 'application/vnd.otto.scene3d+json' || name.endsWith('.scene3d.json')) return 'scene3d';
  if (mime === 'text/html' || name.endsWith('.html') || name.endsWith('.htm')) return 'html';
  return null;
}

/** Like {@link designFormatOf} but defaults to `html` (the assist store's
 *  historical fallback for an agent artifact of unknown shape). */
function formatOf(att: ProductAttachment): DesignFormat {
  return designFormatOf(att) ?? 'html';
}

export const mockupAssist = new MockupAssistStore();
