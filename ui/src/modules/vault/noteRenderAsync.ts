// Large-note reading view off the main thread (F10): the worker client.
// Notes over OFF_THREAD_MIN_BYTES (noteRenderPlan.ts) paint `plainPreview` at
// once and swap in this module's html when it lands; one lazily-created
// module worker serves every note.

import type { VaultOutgoingLink } from '../../lib/api/types';
import { sanitizeHtml } from '../../lib/sanitize';
import { renderNote, resolverFrom, stripFrontmatter } from './mdRender';
import { plainPreview, renderFailurePlan, type RenderFailure } from './noteRenderPlan';
import type { NoteRenderIn, NoteRenderOut } from './noteRender.worker';

let worker: Worker | null = null;
let seq = 0;
const pending = new Map<number, { resolve: (html: string) => void; reject: (e: Error) => void }>();

/** A render that hasn't answered by then is treated as a hung worker: base
 *  budget plus 2 s per MB of note (S18-26). */
export function renderTimeoutMs(bytes: number): number {
  return 10_000 + Math.ceil(bytes / 1_048_576) * 2_000;
}

class NoteRenderError extends Error {
  readonly kind: RenderFailure;
  constructor(kind: RenderFailure) {
    super(`note render worker: ${kind}`);
    this.kind = kind;
  }
}

/** Kill the worker and fail every waiter; the waiter whose budget ran out
 *  gets `timeout`, the others (queued behind it) `recycled`, a worker that
 *  failed to load fails everyone with `load`. The next call starts a fresh
 *  worker. */
function resetWorker(kind: 'load' | 'timeout', hungId?: number): void {
  for (const [id, p] of pending) p.reject(new NoteRenderError(kind === 'load' ? 'load' : id === hungId ? 'timeout' : 'recycled'));
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
  worker.onerror = () => resetWorker('load');
  return worker;
}

/** The result of an off-thread render: SANITIZED html, or — when the note
 *  hung the worker — the escaped plain preview plus `timedOut` so the view
 *  can offer Retry (and must not cache it). */
export interface NoteRenderResult {
  html: string;
  timedOut: boolean;
}

/** Render a large note off the main thread. Falls back to the synchronous
 *  renderer only when workers are unavailable or fail to LOAD; a render that
 *  times out resolves with the plain preview (`timedOut: true`) instead of
 *  re-running the hanging parse on the main thread. */
export function renderNoteOffThread(raw: string, outgoing: VaultOutgoingLink[], retried = false): Promise<NoteRenderResult> {
  const fallback = (): string =>
    renderNote(stripFrontmatter(raw), { resolve: resolverFrom(outgoing), assetUrl: () => null, lazyAssets: true });
  const w = getWorker();
  if (!w) return Promise.resolve({ html: fallback(), timedOut: false });
  const id = ++seq;
  return new Promise<string>((resolve, reject) => {
    // A hung worker used to leave the note on its plain preview forever (and
    // the caller's in-flight key pinned): time out and recycle the worker.
    const timer = setTimeout(() => {
      if (pending.has(id)) resetWorker('timeout', id);
    }, renderTimeoutMs(raw.length));
    pending.set(id, {
      resolve: (html) => { clearTimeout(timer); resolve(html); },
      reject: (e) => { clearTimeout(timer); reject(e); },
    });
    w.postMessage({ id, raw, outgoing: plainLinks(outgoing) } satisfies NoteRenderIn);
  }).then(
    (html) => ({ html, timedOut: false }),
    (e: unknown) => {
      // A worker-side parse error (`{error}` reply) is not a hang: render here.
      const kind: RenderFailure = e instanceof NoteRenderError ? e.kind : 'load';
      switch (renderFailurePlan(kind, retried)) {
        case 'main-thread':
          return { html: fallback(), timedOut: false };
        case 'retry':
          return renderNoteOffThread(raw, outgoing, true);
        default:
          return { html: plainPreview(stripFrontmatter(raw)), timedOut: true };
      }
    },
  );
}

/** Svelte state proxies can't be structured-cloned; copy the fields we use. */
function plainLinks(outgoing: VaultOutgoingLink[]): VaultOutgoingLink[] {
  return outgoing.map((o) => ({ ...o }));
}
