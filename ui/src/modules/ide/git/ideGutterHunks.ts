// Pure Git gutter hunk extraction, unified diff formatting, and single-hunk revert
// for the IDE editor (`E-10a`). Zero Svelte runes or CodeMirror imports so
// `node --test` verifies every edge case directly.

import { diffLineArrays, splitCompareLines } from '../../../lib/lineDiff.ts';
import { plural } from '../../../lib/plural.ts';

export type IdeGutterHunkKind = 'add' | 'mod' | 'del';

export interface IdeGutterHunk {
  index: number;
  kind: IdeGutterHunkKind;
  /** 1-based inclusive start line in current buffer (or insertion line for pure deletions). */
  startLine: number;
  /** 1-based inclusive end line in current buffer (`< startLine` for pure deletions). */
  endLine: number;
  /** 1-based gutter marker anchor line in current buffer. */
  anchorLine: number;
  /** 1-based inclusive start line in base text (`baseEnd < baseStart` for pure additions). */
  baseStart: number;
  /** 1-based inclusive end line in base text. */
  baseEnd: number;
  addedLines: string[];
  removedLines: string[];
  unifiedDiff: string;
}

const MAX_GUTTER_HUNK_LINES = 25_000;

function formatUnifiedHunk(
  oStart0: number,
  delCount: number,
  nStart0: number,
  insCount: number,
  removedLines: readonly string[],
  addedLines: readonly string[],
  filePath?: string,
): string {
  const oldStartHdr = delCount === 0 ? oStart0 : oStart0 + 1;
  const newStartHdr = insCount === 0 ? nStart0 : nStart0 + 1;
  const header = `@@ -${oldStartHdr},${delCount} +${newStartHdr},${insCount} @@`;
  const lines: string[] = [];
  if (filePath) {
    const clean = filePath.replace(/^\/+/, '');
    lines.push(`--- a/${clean}`, `+++ b/${clean}`);
  }
  lines.push(header);
  for (let i = 0; i < removedLines.length; i++) lines.push(`-${removedLines[i]}`);
  for (let i = 0; i < addedLines.length; i++) lines.push(`+${addedLines[i]}`);
  return lines.join('\n');
}

/** Diff `baseText` (Git HEAD or saved file) against `currentText` into structured hunks. */
export function extractGutterHunks(
  baseText: string | null | undefined,
  currentText: string,
  filePath?: string,
): IdeGutterHunk[] {
  if (baseText == null) return [];
  const baseSplit = splitCompareLines(baseText);
  const curSplit = splitCompareLines(currentText);
  const oldLines = baseSplit.lines;
  const newLines = curSplit.lines;
  if (oldLines.length > MAX_GUTTER_HUNK_LINES || newLines.length > MAX_GUTTER_HUNK_LINES) return [];

  if (oldLines.length === 0 && newLines.length === 0) return [];

  const hunks: IdeGutterHunk[] = [];
  const pushHunk = (oStart0: number, oEnd0: number, nStart0: number, nEnd0: number): void => {
    const delCount = oEnd0 - oStart0;
    const insCount = nEnd0 - nStart0;
    if (delCount === 0 && insCount === 0) return;

    const removedLines = oldLines.slice(oStart0, oEnd0);
    const addedLines = newLines.slice(nStart0, nEnd0);
    const kind: IdeGutterHunkKind = delCount === 0 ? 'add' : insCount === 0 ? 'del' : 'mod';
    const startLine = nStart0 + 1;
    const endLine = insCount === 0 ? nStart0 : nEnd0;
    const anchorLine = insCount === 0
      ? Math.min(Math.max(1, newLines.length), Math.max(1, nStart0))
      : startLine;
    const baseStart = oStart0 + 1;
    const baseEnd = delCount === 0 ? oStart0 : oEnd0;

    hunks.push({
      index: hunks.length,
      kind,
      startLine,
      endLine,
      anchorLine,
      baseStart,
      baseEnd,
      addedLines,
      removedLines,
      unifiedDiff: formatUnifiedHunk(oStart0, delCount, nStart0, insCount, removedLines, addedLines, filePath),
    });
  };

  if (oldLines.length === 0 || newLines.length === 0) {
    pushHunk(0, oldLines.length, 0, newLines.length);
    return hunks;
  }

  let prefix = 0;
  while (prefix < oldLines.length && prefix < newLines.length && oldLines[prefix] === newLines[prefix]) {
    prefix++;
  }
  if (prefix === oldLines.length && prefix === newLines.length) return [];

  let suffix = 0;
  while (
    suffix < oldLines.length - prefix &&
    suffix < newLines.length - prefix &&
    oldLines[oldLines.length - 1 - suffix] === newLines[newLines.length - 1 - suffix]
  ) {
    suffix++;
  }

  const oldMid = oldLines.length - prefix - suffix;
  const newMid = newLines.length - prefix - suffix;
  if (oldMid === 0 || newMid === 0) {
    pushHunk(prefix, prefix + oldMid, prefix, prefix + newMid);
    return hunks;
  }

  const ops = diffLineArrays(
    oldLines.slice(prefix, prefix + oldMid),
    newLines.slice(prefix, prefix + newMid),
  );
  let curO = 0;
  let curN = 0;
  for (let k = 0; k < ops.length; k++) {
    const op = ops[k];
    if (op.op !== 'equal') continue;
    const oIdx = op.oldLineNo - 1;
    const nIdx = op.newLineNo - 1;
    pushHunk(prefix + curO, prefix + oIdx, prefix + curN, prefix + nIdx);
    curO = oIdx + 1;
    curN = nIdx + 1;
  }
  pushHunk(prefix + curO, prefix + oldMid, prefix + curN, prefix + newMid);
  return hunks;
}

/** Find the gutter hunk covering a 1-based cursor or clicked line (exact range or pure-deletion anchor). */
export function findGutterHunkAtLine(hunks: readonly IdeGutterHunk[], line: number): IdeGutterHunk | null {
  if (!Number.isFinite(line) || line < 1 || hunks.length === 0) return null;
  for (let i = 0; i < hunks.length; i++) {
    const h = hunks[i];
    if (h.kind === 'del') {
      if (line === h.anchorLine) return h;
    } else if (line >= h.startLine && line <= h.endLine) {
      return h;
    }
  }
  return null;
}

/** Revert a single hunk (`hunkIndex`) in `currentText` back to its `baseText` lines. */
export function revertHunkInText(baseText: string, currentText: string, hunkIndex: number): string {
  const hunks = extractGutterHunks(baseText, currentText);
  if (hunkIndex < 0 || hunkIndex >= hunks.length) return currentText;
  const h = hunks[hunkIndex];

  const baseSplit = splitCompareLines(baseText);
  const curSplit = splitCompareLines(currentText);
  const nextLines = [...curSplit.lines];
  const insertAt = Math.max(0, Math.min(nextLines.length, h.startLine - 1));
  const deleteCount = h.addedLines.length;
  nextLines.splice(insertAt, deleteCount, ...h.removedLines);

  if (nextLines.length === 0) return '';
  const preserveNewline = curSplit.trailingNewline || (curSplit.lines.length === 0 && baseSplit.trailingNewline);
  return preserveNewline ? `${nextLines.join('\n')}\n` : nextLines.join('\n');
}

/** Human-readable hunk summary header (e.g. `Modified L12–15`, `Added 2 lines at L4`). */
export function formatHunkTitle(hunk: Pick<IdeGutterHunk, 'kind' | 'startLine' | 'endLine' | 'anchorLine' | 'addedLines' | 'removedLines'>): string {
  if (hunk.kind === 'add') {
    const range = hunk.endLine > hunk.startLine ? `L${hunk.startLine}–${hunk.endLine}` : `L${hunk.startLine}`;
    return `Added ${plural(hunk.addedLines.length, 'line')} at ${range}`;
  }
  if (hunk.kind === 'del') {
    return `Deleted ${plural(hunk.removedLines.length, 'line')} near L${hunk.anchorLine}`;
  }
  const range = hunk.endLine > hunk.startLine ? `L${hunk.startLine}–${hunk.endLine}` : `L${hunk.startLine}`;
  return `Modified ${range}`;
}

/** Resolve the 1-based editor line number from a click inside CodeMirror's `.cm-ide-diff-gutter`. */
export function resolveClickedDiffGutterLine(event: MouseEvent): number | null {
  if (event.button !== 0 || !(event.target instanceof Element)) return null;
  const bar = event.target.closest('.cm-ide-diff-bar');
  const gutterEl = event.target.closest('.cm-ide-diff-gutter .cm-gutterElement');
  if (!bar && !gutterEl?.querySelector('.cm-ide-diff-bar')) return null;
  const guttersRoot = event.target.closest('.cm-gutters');
  if (!guttersRoot) return null;

  const targetRect = (gutterEl ?? bar)!.getBoundingClientRect();
  const targetMidY = targetRect.top + targetRect.height / 2;
  const lineEls = guttersRoot.querySelectorAll('.cm-lineNumbers .cm-gutterElement');
  let bestLine: number | null = null;
  let bestDist = Infinity;
  for (let i = 0; i < lineEls.length; i++) {
    const el = lineEls[i];
    const text = el.textContent?.trim() ?? '';
    if (!/^\d+$/.test(text)) continue;
    const r = el.getBoundingClientRect();
    if (r.height <= 0) continue;
    const dist = Math.abs(r.top + r.height / 2 - targetMidY);
    if (dist < bestDist) {
      bestDist = dist;
      bestLine = Number(text);
    }
  }
  return bestLine != null && bestDist <= Math.max(24, targetRect.height) ? bestLine : null;
}
