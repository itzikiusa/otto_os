// Side-by-side comparison engine and per-hunk `>>` (`Revert ⌃⌘→`) patcher
// for `IdeComparisonModal.svelte`.

import { diffLineArrays, splitCompareLines } from '../../../lib/lineDiff.ts';
import { wordDiff, type WordRange } from '../../../lib/diff/diff-words.ts';

export { splitCompareLines };

export interface CompareHunk {
  index: number;
  kind: 'modify' | 'add' | 'delete';
  leftStart: number;
  leftCount: number;
  rightStart: number;
  rightCount: number;
  leftLines: string[];
  rightLines: string[];
  firstRowIndex: number;
}

export interface AlignedCompareRow {
  index: number;
  kind: 'equal' | 'modify' | 'add' | 'delete';
  hunkIndex: number | null;
  isHunkStart: boolean;
  leftLineNo: number | null;
  leftText: string | null;
  leftWords: WordRange[] | null;
  rightLineNo: number | null;
  rightText: string | null;
  rightWords: WordRange[] | null;
}

export interface SideBySideComparison {
  identical: boolean;
  leftLineCount: number;
  rightLineCount: number;
  rows: AlignedCompareRow[];
  hunks: CompareHunk[];
}

/** Build aligned side-by-side rows, word-level marks, and addressable hunks. */
export function computeSideBySideComparison(
  leftText: string,
  rightText: string,
): SideBySideComparison {
  const normL = leftText.replace(/\r\n?/g, '\n');
  const normR = rightText.replace(/\r\n?/g, '\n');
  const left = splitCompareLines(normL).lines;
  const right = splitCompareLines(normR).lines;

  if (normL === normR) {
    const rows: AlignedCompareRow[] = left.map((line, idx) => ({
      index: idx,
      kind: 'equal',
      hunkIndex: null,
      isHunkStart: false,
      leftLineNo: idx + 1,
      leftText: line,
      leftWords: null,
      rightLineNo: idx + 1,
      rightText: line,
      rightWords: null,
    }));
    return {
      identical: true,
      leftLineCount: left.length,
      rightLineCount: right.length,
      rows,
      hunks: [],
    };
  }

  const ops = diffLineArrays(left, right);
  const rows: AlignedCompareRow[] = [];
  const hunks: CompareHunk[] = [];

  const emitGap = (l0: number, l1: number, r0: number, r1: number): void => {
    const lCount = l1 - l0;
    const rCount = r1 - r0;
    if (lCount <= 0 && rCount <= 0) return;
    const hunkIndex = hunks.length;
    const kind: CompareHunk['kind'] =
      lCount > 0 && rCount > 0 ? 'modify' : lCount > 0 ? 'delete' : 'add';
    const firstRowIndex = rows.length;
    hunks.push({
      index: hunkIndex,
      kind,
      leftStart: l0,
      leftCount: lCount,
      rightStart: r0,
      rightCount: rCount,
      leftLines: left.slice(l0, l1),
      rightLines: right.slice(r0, r1),
      firstRowIndex,
    });

    const span = Math.max(lCount, rCount);
    const paired = Math.min(lCount, rCount);
    for (let x = 0; x < span; x++) {
      const hasL = x < lCount;
      const hasR = x < rCount;
      const lLine = hasL ? left[l0 + x] : null;
      const rLine = hasR ? right[r0 + x] : null;
      const wd = x < paired && lLine !== null && rLine !== null ? wordDiff(lLine, rLine) : null;
      rows.push({
        index: rows.length,
        kind: hasL && hasR ? 'modify' : hasL ? 'delete' : 'add',
        hunkIndex,
        isHunkStart: x === 0,
        leftLineNo: hasL ? l0 + x + 1 : null,
        leftText: lLine,
        leftWords: wd?.a?.length ? wd.a : null,
        rightLineNo: hasR ? r0 + x + 1 : null,
        rightText: rLine,
        rightWords: wd?.b?.length ? wd.b : null,
      });
    }
  };

  let curL = 0;
  let curR = 0;
  for (let k = 0; k < ops.length; k++) {
    const op = ops[k];
    if (op.op !== 'equal') continue;
    const leftIdx = op.oldLineNo - 1;
    const rightIdx = op.newLineNo - 1;
    emitGap(curL, leftIdx, curR, rightIdx);
    rows.push({
      index: rows.length,
      kind: 'equal',
      hunkIndex: null,
      isHunkStart: false,
      leftLineNo: leftIdx + 1,
      leftText: left[leftIdx],
      leftWords: null,
      rightLineNo: rightIdx + 1,
      rightText: right[rightIdx],
      rightWords: null,
    });
    curL = leftIdx + 1;
    curR = rightIdx + 1;
  }
  emitGap(curL, left.length, curR, right.length);

  // Trailing-newline-only difference still produces an actionable 1-hunk update.
  if (hunks.length === 0 && normL !== normR) {
    const lastIdx = Math.max(0, rows.length - 1);
    hunks.push({
      index: 0,
      kind: 'modify',
      leftStart: 0,
      leftCount: left.length,
      rightStart: 0,
      rightCount: right.length,
      leftLines: left,
      rightLines: right,
      firstRowIndex: lastIdx,
    });
    if (rows[lastIdx]) {
      rows[lastIdx] = { ...rows[lastIdx], kind: 'modify', hunkIndex: 0, isHunkStart: true };
    }
  }

  return {
    identical: false,
    leftLineCount: left.length,
    rightLineCount: right.length,
    rows,
    hunks,
  };
}

/** Apply a single left-side hunk (`>>` Revert button) onto `rightText`. */
export function applyLeftHunkToRight(
  leftText: string,
  rightText: string,
  hunkIndex: number,
): string {
  const cmp = computeSideBySideComparison(leftText, rightText);
  const hunk = cmp.hunks[hunkIndex];
  if (!hunk) return rightText;
  if (cmp.hunks.length === 1 && hunk.leftCount === cmp.leftLineCount && hunk.rightCount === cmp.rightLineCount) {
    return leftText.replace(/\r\n?/g, '\n');
  }
  const rightParsed = splitCompareLines(rightText);
  const leftParsed = splitCompareLines(leftText);
  const nextLines = [...rightParsed.lines];
  nextLines.splice(hunk.rightStart, hunk.rightCount, ...hunk.leftLines);
  if (nextLines.length === 0) return '';
  const keepTrailing = rightText ? rightParsed.trailingNewline : leftParsed.trailingNewline;
  return keepTrailing ? `${nextLines.join('\n')}\n` : nextLines.join('\n');
}

/** Debounces inline edits in `IdeCompareDiffPane`, binding each scheduled write to the
 *  exact `destPath` that was being edited so switching files or closing the modal can
 *  never overwrite another file or leak a dangling timer. */
export class CompareEditDebouncer {
  private timer: ReturnType<typeof setTimeout> | null = null;
  private pending: { destPath: string; text: string } | null = null;
  private readonly onApply: (text: string, reason: string, destPath: string) => void | Promise<void>;
  private readonly delayMs: number;

  constructor(
    onApply: (text: string, reason: string, destPath: string) => void | Promise<void>,
    delayMs = 320,
  ) {
    this.onApply = onApply;
    this.delayMs = delayMs;
  }

  schedule(destPath: string, text: string): void {
    if (this.pending && this.pending.destPath !== destPath) {
      this.flush();
    }
    if (this.timer) clearTimeout(this.timer);
    this.pending = { destPath, text };
    this.timer = setTimeout(() => {
      this.timer = null;
      const item = this.pending;
      this.pending = null;
      if (item) void this.onApply(item.text, 'Edited Current', item.destPath);
    }, this.delayMs);
  }

  flush(): void {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    const item = this.pending;
    this.pending = null;
    if (item) void this.onApply(item.text, 'Edited Current', item.destPath);
  }

  cancel(): void {
    if (this.timer) {
      clearTimeout(this.timer);
      this.timer = null;
    }
    this.pending = null;
  }

  hasPending(): boolean {
    return this.pending !== null;
  }
}
