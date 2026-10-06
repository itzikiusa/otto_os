// Pure helpers for ConflictHunk's hand-edit mode. The conflict parser keeps a
// CRLF file's `\r` on every line, but CodeMirror normalises line breaks — so an
// edited block came back LF-only and the resolved file had mixed endings.

/** A block is CRLF when most of its non-empty lines end in `\r`. */
export function isCrlfBlock(...sides: string[][]): boolean {
  let cr = 0;
  let total = 0;
  for (const side of sides)
    for (const l of side) {
      if (l.length === 0) continue;
      total++;
      if (l.endsWith('\r')) cr++;
    }
  return total > 0 && cr * 2 > total;
}

/** Editor seed: the lines joined with `\n`, trailing `\r` dropped (CodeMirror
 *  would strip them anyway, and a stray `\r` renders as a glyph). */
export function editSeed(lines: string[]): string {
  return lines.map((l) => (l.endsWith('\r') ? l.slice(0, -1) : l)).join('\n');
}

/** The edited text back as resolved lines, re-terminated with `\r` for a CRLF
 *  block. An empty editor is a deliberate empty resolution. */
export function editLines(text: string, crlf: boolean): string[] {
  if (text.length === 0) return [];
  const lines = text.split('\n').map((l) => (l.endsWith('\r') ? l.slice(0, -1) : l));
  return crlf ? lines.map((l) => l + '\r') : lines;
}
