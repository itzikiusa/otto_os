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
