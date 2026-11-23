// Cached per-file line-number index for PR review comment diff snippets,
// split out of ReviewPanel.svelte.
import type { DiffLine, FileDiff, ReviewComment } from '../../lib/api/types';

const CONTEXT_LINES = 3;

interface SnippetIndex {
  lines: DiffLine[];
  byNew: Map<number, number>;
  byOld: Map<number, number>;
}

const snippetIndex = new WeakMap<FileDiff, SnippetIndex>();

function indexOf(f: FileDiff): SnippetIndex {
  let ix = snippetIndex.get(f);
  if (ix) return ix;
  ix = { lines: [], byNew: new Map(), byOld: new Map() };
  for (const hunk of f.hunks) {
    for (const line of hunk.lines) {
      const i = ix.lines.push(line) - 1;
      if (line.new_line !== null && !ix.byNew.has(line.new_line)) ix.byNew.set(line.new_line, i);
      if (line.old_line !== null && !ix.byOld.has(line.old_line)) ix.byOld.set(line.old_line, i);
    }
  }
  snippetIndex.set(f, ix);
  return ix;
}

/** Return the ±CONTEXT_LINES slice around a comment's target line, or null. */
export function snippetLinesFor(c: ReviewComment, files: Map<string, FileDiff>): DiffLine[] | null {
  if (c.path === null || c.line === null) return null;
  const fileDiff = files.get(c.path);
  if (!fileDiff) return null;
  const ix = indexOf(fileDiff);
  const targetIdx = ix.byNew.get(c.line) ?? ix.byOld.get(c.line);
  if (targetIdx === undefined) return null;
  const start = Math.max(0, targetIdx - CONTEXT_LINES);
  const end = Math.min(ix.lines.length - 1, targetIdx + CONTEXT_LINES);
  return ix.lines.slice(start, end + 1);
}
