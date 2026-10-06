// Large-note reading view off the main thread (F10) — the pure half: which
// notes leave the main thread, and what the pane shows while they render.
// No runtime imports, so the node unit tests (unit/vaultNoteRender.test.ts)
// load it directly.

/** At or below this many UTF-16 units a note renders on the main thread (no
 *  flash, no worker round trip); above it the parse moves to a worker. */
export const OFF_THREAD_MIN_BYTES = 64 * 1024;

export function rendersOffThread(raw: string): boolean {
  return raw.length > OFF_THREAD_MIN_BYTES;
}

function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => `&#${c.charCodeAt(0)};`);
}

/** Shown while a large note renders: its body (frontmatter already stripped)
 *  as escaped plain text — readable at once, never a blank pane. */
export function plainPreview(body: string): string {
  return `<pre class="note-plain" aria-busy="true">${esc(body)}</pre>`;
}

/** Why a worker render didn't answer. Only `load` (the worker never came up)
 *  falls back to the main-thread renderer: a note that HUNG the worker would
 *  hang the main thread the same way (S18-305). `recycled` = queued behind
 *  the hung render when its worker was killed. */
export type RenderFailure = 'load' | 'timeout' | 'recycled';

/** What a caller does with a failed worker render: re-run it on the main
 *  thread, retry it once on a fresh worker, or show the plain preview with a
 *  "timed out · Retry" notice. */
export function renderFailurePlan(kind: RenderFailure, retried: boolean): 'main-thread' | 'retry' | 'timed-out' {
  if (kind === 'load') return 'main-thread';
  if (kind === 'recycled' && !retried) return 'retry';
  return 'timed-out';
}
