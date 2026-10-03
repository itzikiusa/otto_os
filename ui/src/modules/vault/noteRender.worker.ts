// noteRender.worker.ts — large-note reading view, parsed off the main thread
// (F10). NoteView sends the raw note + its outgoing links for any note over
// `OFF_THREAD_MIN_BYTES`; the worker strips frontmatter/comments, runs marked
// and highlight.js (awaiting the lazy hljs load first, so the one render is
// already highlighted) and posts the UNSANITIZED html back. Sanitizing needs
// `DOMParser`, which a worker lacks, so the main thread runs `sanitizeHtml` on
// the reply (noteRenderAsync.ts) — cheap next to parse + highlight.

import type { VaultOutgoingLink } from '../../lib/api/types';
import { ensureHljs } from '../../lib/hl';
import { renderNoteUnsanitized, resolverFrom, stripFrontmatter } from './mdRender';

export type NoteRenderIn = { id: number; raw: string; outgoing: VaultOutgoingLink[] };
export type NoteRenderOut = { id: number; html: string } | { id: number; error: string };

self.onmessage = async (e: MessageEvent<NoteRenderIn>) => {
  const { id, raw, outgoing } = e.data;
  try {
    await ensureHljs().catch(() => {}); // unhighlighted beats no render
    const html = renderNoteUnsanitized(stripFrontmatter(raw), {
      resolve: resolverFrom(outgoing),
      assetUrl: () => null,
      lazyAssets: true,
    });
    (self as unknown as Worker).postMessage({ id, html } satisfies NoteRenderOut);
  } catch (err) {
    (self as unknown as Worker).postMessage({ id, error: String(err) } satisfies NoteRenderOut);
  }
};
