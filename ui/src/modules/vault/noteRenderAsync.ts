// Large-note reading view off the main thread (F10): the worker client.
// Notes over OFF_THREAD_MIN_BYTES (noteRenderPlan.ts) paint `plainPreview` at
// once and swap in this module's html when it lands; one lazily-created
// module worker serves every note.

import type { VaultOutgoingLink } from '../../lib/api/types';
import { sanitizeHtml } from '../../lib/sanitize';
import { renderNote, resolverFrom, stripFrontmatter } from './mdRender';
import type { NoteRenderIn, NoteRenderOut } from './noteRender.worker';

let worker: Worker | null = null;
let seq = 0;
const pending = new Map<number, { resolve: (html: string) => void; reject: (e: Error) => void }>();

/** A render that hasn't answered by then is treated as a hung worker: base
 *  budget plus 2 s per MB of note (S18-26). */
export function renderTimeoutMs(bytes: number): number {
  return 10_000 + Math.ceil(bytes / 1_048_576) * 2_000;
}

/** Kill the worker and fail every waiter (each falls back to the main-thread
 *  render); the next call starts a fresh worker. */
function resetWorker(reason: string): void {
  for (const p of pending.values()) p.reject(new Error(reason));
  pending.clear();
  worker?.terminate();
  worker = null;
}

function getWorker(): Worker | null {
  if (worker) return worker;
  if (typeof Worker === 'undefined') return null;
  try {
    worker = new Worker(new URL('./noteRender.worker.ts', import.meta.url), { type: 'module' });
  } catch {
    return null;
  }
  worker.onmessage = (e: MessageEvent<NoteRenderOut>) => {
    const p = pending.get(e.data.id);
    if (!p) return;
    pending.delete(e.data.id);
    if ('html' in e.data) p.resolve(sanitizeHtml(e.data.html));
    else p.reject(new Error(e.data.error));
  };
  // A worker that failed to load: fail every waiter (callers fall back to
  // the main-thread render) and let the next call try a fresh one.
  worker.onerror = () => resetWorker('note render worker failed');
  return worker;
}

/** Render a large note off the main thread; falls back to the synchronous
 *  renderer when workers are unavailable or the worker fails. Resolves with
 *  SANITIZED html. */
export function renderNoteOffThread(raw: string, outgoing: VaultOutgoingLink[]): Promise<string> {
  const fallback = (): string =>
    renderNote(stripFrontmatter(raw), { resolve: resolverFrom(outgoing), assetUrl: () => null, lazyAssets: true });
  const w = getWorker();
  if (!w) return Promise.resolve(fallback());
  const id = ++seq;
  return new Promise<string>((resolve, reject) => {
    // A hung worker used to leave the note on its plain preview forever (and
    // the caller's in-flight key pinned): time out, recycle, fall back.
    const timer = setTimeout(() => {
      if (pending.has(id)) resetWorker('note render worker timed out');
    }, renderTimeoutMs(raw.length));
    pending.set(id, {
      resolve: (html) => { clearTimeout(timer); resolve(html); },
      reject: (e) => { clearTimeout(timer); reject(e); },
    });
    w.postMessage({ id, raw, outgoing: plainLinks(outgoing) } satisfies NoteRenderIn);
  }).catch(() => fallback());
}

/** Svelte state proxies can't be structured-cloned; copy the fields we use. */
function plainLinks(outgoing: VaultOutgoingLink[]): VaultOutgoingLink[] {
  return outgoing.map((o) => ({ ...o }));
}
