// Side-by-side comparison engine, per-hunk `>>` (`Revert ⌃⌘→`) patcher, and Git
// revision file-content reconstructor for `IdeComparisonModal.svelte`.

import type { DiffResp, Hunk } from '../../../lib/api/types';
import { wordDiff, type WordRange } from '../../git/diff-words'; // ui-guards: allow — reuse Otto's shared Git word diff

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

const MAX_LCS_CELLS = 1_600_000;

export function splitCompareLines(text: string): { lines: string[]; trailingNewline: boolean } {
  if (!text) return { lines: [], trailingNewline: false };
  const normalized = text.replace(/\r\n/g, '\n');
  const trailingNewline = normalized.endsWith('\n');
  const raw = trailingNewline ? normalized.slice(0, -1) : normalized;
  return { lines: raw.length === 0 && trailingNewline ? [''] : raw.split('\n'), trailingNewline };
}

interface RawPairOp {
  equal: boolean;
  leftIdx: number;
  rightIdx: number;
}

function matchMiddleLines(a: string[], b: string[]): Array<[number, number]> {
  const n = a.length;
  const m = b.length;
  if (n === 0 || m === 0) return [];

  if (n * m <= MAX_LCS_CELLS) {
    const w = m + 1;
    const dp = new Uint32Array((n + 1) * w);
    for (let i = n - 1; i >= 0; i--) {
      const row = i * w;
      const nextRow = (i + 1) * w;
      const ai = a[i];
      for (let j = m - 1; j >= 0; j--) {
        dp[row + j] =
          ai === b[j]
            ? dp[nextRow + j + 1] + 1
            : dp[nextRow + j] >= dp[row + j + 1]
              ? dp[nextRow + j]
              : dp[row + j + 1];
      }
    }
    const pairs: Array<[number, number]> = [];
    let i = 0;
    let j = 0;
    while (i < n && j < m) {
      if (a[i] === b[j]) {
        pairs.push([i, j]);
        i += 1;
        j += 1;
      } else if (dp[(i + 1) * w + j] >= dp[i * w + j + 1]) {
        i += 1;
      } else {
        j += 1;
      }
    }
    return pairs;
  }

  // Linear lookahead anchor fallback for very large files (>1.6M LCS cells).
  const pairs: Array<[number, number]> = [];
  let i = 0;
  let j = 0;
  const lookahead = 48;
  while (i < n && j < m) {
    if (a[i] === b[j]) {
      pairs.push([i, j]);
      i += 1;
      j += 1;
      continue;
    }
    let matched = false;
    for (let step = 1; step <= lookahead && (i + step < n || j + step < m); step++) {
      if (j + step < m && a[i] === b[j + step]) {
        j += step;
        matched = true;
        break;
      }
      if (i + step < n && a[i + step] === b[j]) {
        i += step;
        matched = true;
        break;
      }
    }
    if (!matched) {
      i += 1;
      j += 1;
    }
  }
  return pairs;
}

/** Build aligned side-by-side rows, word-level marks, and addressable hunks. */
export function computeSideBySideComparison(
  leftText: string,
  rightText: string,
): SideBySideComparison {
  const normL = leftText.replace(/\r\n/g, '\n');
  const normR = rightText.replace(/\r\n/g, '\n');
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

  let pre = 0;
  while (pre < left.length && pre < right.length && left[pre] === right[pre]) pre += 1;
  let suf = 0;
  while (
    suf < left.length - pre &&
    suf < right.length - pre &&
    left[left.length - 1 - suf] === right[right.length - 1 - suf]
  ) {
    suf += 1;
  }

  const midL = left.slice(pre, left.length - suf);
  const midR = right.slice(pre, right.length - suf);
  const midMatches = matchMiddleLines(midL, midR);

  const anchors: RawPairOp[] = [];
  for (let i = 0; i < pre; i++) anchors.push({ equal: true, leftIdx: i, rightIdx: i });
  for (let k = 0; k < midMatches.length; k++) {
    anchors.push({
      equal: true,
      leftIdx: pre + midMatches[k][0],
      rightIdx: pre + midMatches[k][1],
    });
  }
  for (let i = 0; i < suf; i++) {
    anchors.push({
      equal: true,
      leftIdx: left.length - suf + i,
      rightIdx: right.length - suf + i,
    });
  }

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
  for (let a = 0; a < anchors.length; a++) {
    const anc = anchors[a];
    emitGap(curL, anc.leftIdx, curR, anc.rightIdx);
    rows.push({
      index: rows.length,
      kind: 'equal',
      hunkIndex: null,
      isHunkStart: false,
      leftLineNo: anc.leftIdx + 1,
      leftText: left[anc.leftIdx],
      leftWords: null,
      rightLineNo: anc.rightIdx + 1,
      rightText: right[anc.rightIdx],
      rightWords: null,
    });
    curL = anc.leftIdx + 1;
    curR = anc.rightIdx + 1;
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
    return leftText.replace(/\r\n/g, '\n');
  }
  const rightParsed = splitCompareLines(rightText);
  const leftParsed = splitCompareLines(leftText);
  const nextLines = [...rightParsed.lines];
  nextLines.splice(hunk.rightStart, hunk.rightCount, ...hunk.leftLines);
  if (nextLines.length === 0) return '';
  const keepTrailing = rightText ? rightParsed.trailingNewline : leftParsed.trailingNewline;
  return keepTrailing ? `${nextLines.join('\n')}\n` : nextLines.join('\n');
}

const HUNK_HDR_RE = /^@@\s+-(\d+)(?:,\d+)?\s+\+(\d+)(?:,\d+)?\s+@@/;

function parseHunkStarts(hunk: Hunk): { oldStart: number; newStart: number } {
  const m = HUNK_HDR_RE.exec(hunk.header);
  if (m) return { oldStart: Number(m[1]) || 1, newStart: Number(m[2]) || 1 };
  let oldStart = 1;
  let newStart = 1;
  for (let i = 0; i < hunk.lines.length; i++) {
    if (hunk.lines[i].old_line != null) {
      oldStart = hunk.lines[i].old_line!;
      break;
    }
  }
  for (let i = 0; i < hunk.lines.length; i++) {
    if (hunk.lines[i].new_line != null) {
      newStart = hunk.lines[i].new_line!;
      break;
    }
  }
  return { oldStart, newStart };
}

/** Reconstruct file content by applying a `DiffResp` forward (`old -> new`) or in reverse (`new -> old`). */
export function reconstructFileFromDiff(
  sourceText: string,
  diff: DiffResp | null,
  direction: 'forward' | 'reverse',
): string {
  const fileDiff = diff?.files?.[0];
  if (!fileDiff) return sourceText;
  if (direction === 'reverse' && fileDiff.status === 'added') return '';
  if (direction === 'forward' && fileDiff.status === 'deleted') return '';
  if (fileDiff.hunks.length === 0) return sourceText;

  const parsed = splitCompareLines(sourceText);
  const lines = [...parsed.lines];
  const entries = fileDiff.hunks.map((h) => ({ hunk: h, starts: parseHunkStarts(h) }));
  entries.sort((a, b) =>
    direction === 'reverse' ? b.starts.newStart - a.starts.newStart : b.starts.oldStart - a.starts.oldStart,
  );

  for (let h = 0; h < entries.length; h++) {
    const { hunk, starts } = entries[h];
    const startOneBased = direction === 'reverse' ? starts.newStart : starts.oldStart;
    const removeOrigin = direction === 'reverse' ? 'add' : 'del';
    const keepOrigin = direction === 'reverse' ? 'del' : 'add';
    let removeCount = 0;
    const replacement: string[] = [];
    for (let i = 0; i < hunk.lines.length; i++) {
      const ln = hunk.lines[i];
      if (ln.origin === 'context') {
        removeCount += 1;
        replacement.push(ln.content);
      } else if (ln.origin === removeOrigin) {
        removeCount += 1;
      } else if (ln.origin === keepOrigin) {
        replacement.push(ln.content);
      }
    }
    const startIdx = Math.max(0, startOneBased - 1);
    lines.splice(startIdx, removeCount, ...replacement);
  }

  if (lines.length === 0) return '';
  return parsed.trailingNewline || sourceText === '' ? `${lines.join('\n')}\n` : lines.join('\n');
}
