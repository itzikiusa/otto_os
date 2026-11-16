// The editor's path crumbs row (pure, unit-tested in node).
// - A deep path keeps its first folder, the parent folder and the file name; the folders
//   between collapse into ONE "…" crumb whose menu lists them (VS Code's collapse), so the
//   file name is never ellipsised away by a row of "se… pay… int…" stubs.
// - The symbol trail under the cursor (outermost first), each level opening its siblings.

export type IdeCrumb =
  | { kind: 'seg'; label: string; index: number; last: boolean }
  | { kind: 'more'; hidden: { label: string; index: number }[] };

/** Crumbs for `segments` (relative path parts): every part when the path is short, else
 *  the first `head`, a "…" holding the middle, and the last `tail` (parent folder + file). */
export function collapseIdeCrumbs(segments: readonly string[], head = 1, tail = 2): IdeCrumb[] {
  const n = segments.length;
  const seg = (i: number): IdeCrumb => ({ kind: 'seg', label: segments[i], index: i, last: i === n - 1 });
  if (n <= head + tail + 1) return segments.map((_, i) => seg(i));
  const out: IdeCrumb[] = [];
  for (let i = 0; i < head; i++) out.push(seg(i));
  const hidden: { label: string; index: number }[] = [];
  for (let i = head; i < n - tail; i++) hidden.push({ label: segments[i], index: i });
  out.push({ kind: 'more', hidden });
  for (let i = n - tail; i < n; i++) out.push(seg(i));
  return out;
}

/** Absolute path of crumb `index` of a file whose relative path has `segCount` parts. */
export function ideCrumbAbsPath(absFilePath: string, segCount: number, index: number): string {
  const drop = segCount - 1 - index;
  if (drop <= 0) return absFilePath;
  const parts = absFilePath.split('/');
  return parts.slice(0, Math.max(1, parts.length - drop)).join('/') || '/';
}

/** The parent folder of a relative path, as a short menu hint (`notes/a.md` → `notes`). */
export function ideParentFolderHint(relPath: string): string | undefined {
  const parts = relPath.split('/').filter(Boolean);
  return parts.length > 1 ? parts[parts.length - 2] : undefined;
}

interface SymbolLike { name: string; line: number; end_line: number; children?: SymbolLike[] }

/** The symbols enclosing `line`, outermost first. */
export function ideSymbolChainAt<T extends SymbolLike>(symbols: readonly T[], line: number): T[] {
  const chain: T[] = [];
  let level: readonly T[] = symbols;
  for (;;) {
    const hit = level.find((s) => line >= s.line && line <= Math.max(s.line, s.end_line));
    if (!hit) return chain;
    chain.push(hit);
    if (!hit.children?.length) return chain;
    level = hit.children as T[];
  }
}

/** The siblings of chain level `levelIdx` (the whole top level for 0). */
export function ideSymbolSiblings<T extends SymbolLike>(symbols: readonly T[], chain: readonly T[], levelIdx: number): readonly T[] {
  if (levelIdx === 0) return symbols;
  return (chain[levelIdx - 1]?.children as T[] | undefined) ?? symbols;
}

/** How far to scroll a strip (positive = towards its end) so an item spanning
 *  [start, end] is fully inside [viewStart, viewEnd]; 0 when it already is. */
export function ideRevealDelta(start: number, end: number, viewStart: number, viewEnd: number): number {
  if (start < viewStart) return start - viewStart;
  if (end > viewEnd) return Math.min(end - viewEnd, start - viewStart);
  return 0;
}
