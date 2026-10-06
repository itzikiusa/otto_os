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

/** The whole file's line-ending verdict, over every context line and every
 *  conflict side. A hunk whose own sides are empty (or one LF-less last line)
 *  can't tell, so ConflictHunk takes this instead of guessing LF. */
export function fileIsCrlf(
  segments: readonly ({ kind: 'context'; lines: string[] } | { kind: 'conflict'; ours: string[]; theirs: string[]; base: string[] })[],
): boolean {
  const sides: string[][] = [];
  for (const s of segments) {
    if (s.kind === 'context') sides.push(s.lines);
    else sides.push(s.ours, s.theirs, s.base);
  }
  return isCrlfBlock(...sides);
}

/** What "Mark file resolved" may do for a loaded conflict file:
 *  - `compose` — every marker decided: POST the recomposed text;
 *  - `keep` — a readable text file with NO markers left (fixed by hand):
 *    stage the working file's bytes as they are (`side: 'keep'`), never a
 *    re-composition (which would rewrite line endings / the trailing newline);
 *  - `null` — not markable. A BINARY (or unreadable/symlink) conflict and a
 *    file ABSENT from the working tree also have zero segments, and
 *    composing them posted `""`: the blob was truncated to 0 bytes, the
 *    deleted file came back empty. Those resolve by taking a side only. */
export function markAction(
  file: { is_binary: boolean; worktree_present: boolean } | null,
  conflictCount: number,
  decidedCount: number,
): 'compose' | 'keep' | null {
  if (!file || file.is_binary || !file.worktree_present) return null;
  if (conflictCount === 0) return 'keep';
  return decidedCount === conflictCount ? 'compose' : null;
}
