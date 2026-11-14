// Shared patience-anchored typed-array LCS line-diff engine (`DUP-7`).
// Used by `ideDiffApplier`, `ideCompareEngine`, `ideLocalHistoryDiff`, and
// `cm-ide-git-and-agent` so Unified DiffViewer, Side-by-Side comparison, and
// CodeMirror VCS gutter markers always agree on exact edit scripts.
// Pure TypeScript (zero Svelte runes or external imports) for `node --test`.

export type LineDiffOpKind = 'equal' | 'delete' | 'insert';

export interface LineDiffOp {
  op: LineDiffOpKind;
  line: string;
  oldLineNo: number; // 1-based when op !== 'insert', else 0
  newLineNo: number; // 1-based when op !== 'delete', else 0
}

export function splitDiffLines(text: string): string[] {
  if (!text) return [];
  const norm = text.replace(/\r\n/g, '\n');
  const lines = norm.split('\n');
  if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop();
  return lines;
}

interface RawEdit {
  op: LineDiffOpKind;
  line: string;
}

const MAX_EXACT_DP_CELLS = 1_600_000;

function exactSliceLcs(a: readonly string[], b: readonly string[], out: RawEdit[]): void {
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) {
    out.push({ op: 'equal', line: a[start] });
    start++;
  }
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }

  const m = endA - start;
  const n = endB - start;
  if (m === 0 && n === 0) {
    for (let k = endA; k < a.length; k++) out.push({ op: 'equal', line: a[k] });
    return;
  }
  if (m === 0) {
    for (let j = start; j < endB; j++) out.push({ op: 'insert', line: b[j] });
    for (let k = endA; k < a.length; k++) out.push({ op: 'equal', line: a[k] });
    return;
  }
  if (n === 0) {
    for (let i = start; i < endA; i++) out.push({ op: 'delete', line: a[i] });
    for (let k = endA; k < a.length; k++) out.push({ op: 'equal', line: a[k] });
    return;
  }

  if (m * n <= MAX_EXACT_DP_CELLS) {
    const cols = n + 1;
    const dp = new Int32Array((m + 1) * cols);
    for (let i = m - 1; i >= 0; i--) {
      const rowOff = i * cols;
      const nextRowOff = (i + 1) * cols;
      const ai = a[start + i];
      for (let j = n - 1; j >= 0; j--) {
        if (ai === b[start + j]) {
          dp[rowOff + j] = dp[nextRowOff + j + 1] + 1;
        } else {
          const down = dp[nextRowOff + j];
          const right = dp[rowOff + j + 1];
          dp[rowOff + j] = down >= right ? down : right;
        }
      }
    }
    let i = 0;
    let j = 0;
    while (i < m && j < n) {
      if (a[start + i] === b[start + j]) {
        out.push({ op: 'equal', line: a[start + i] });
        i++;
        j++;
      } else if (dp[(i + 1) * cols + j] >= dp[i * cols + j + 1]) {
        out.push({ op: 'delete', line: a[start + i] });
        i++;
      } else {
        out.push({ op: 'insert', line: b[start + j] });
        j++;
      }
    }
    while (i < m) {
      out.push({ op: 'delete', line: a[start + i] });
      i++;
    }
    while (j < n) {
      out.push({ op: 'insert', line: b[start + j] });
      j++;
    }
  } else {
    // Bounded linear-memory lookahead for huge gaps exceeding DP budget
    let i = start;
    let j = start;
    const windowSize = 96;
    while (i < endA && j < endB) {
      if (a[i] === b[j]) {
        out.push({ op: 'equal', line: a[i] });
        i++;
        j++;
        continue;
      }
      let matchI = -1;
      let matchJ = -1;
      const maxStep = Math.min(windowSize, Math.max(endA - i, endB - j));
      for (let step = 1; step <= maxStep; step++) {
        if (j + step < endB && a[i] === b[j + step]) {
          matchI = i;
          matchJ = j + step;
          break;
        }
        if (i + step < endA && a[i + step] === b[j]) {
          matchI = i + step;
          matchJ = j;
          break;
        }
      }
      if (matchI >= 0 && matchJ >= 0) {
        while (i < matchI) out.push({ op: 'delete', line: a[i++] });
        while (j < matchJ) out.push({ op: 'insert', line: b[j++] });
      } else {
        out.push({ op: 'delete', line: a[i++] });
        out.push({ op: 'insert', line: b[j++] });
      }
    }
    while (i < endA) out.push({ op: 'delete', line: a[i++] });
    while (j < endB) out.push({ op: 'insert', line: b[j++] });
  }

  for (let k = endA; k < a.length; k++) {
    out.push({ op: 'equal', line: a[k] });
  }
}

/** Find unique non-blank anchor pairs and extract their Longest Increasing Subsequence. */
function findUniqueAnchors(
  a: readonly string[],
  b: readonly string[],
): Array<{ i: number; j: number }> {
  const countA = new Map<string, number>();
  const idxA = new Map<string, number>();
  for (let i = 0; i < a.length; i++) {
    const s = a[i];
    if (!s.trim()) continue;
    countA.set(s, (countA.get(s) ?? 0) + 1);
    idxA.set(s, i);
  }
  const countB = new Map<string, number>();
  const idxB = new Map<string, number>();
  for (let j = 0; j < b.length; j++) {
    const s = b[j];
    if (countA.get(s) !== 1) continue;
    countB.set(s, (countB.get(s) ?? 0) + 1);
    idxB.set(s, j);
  }

  const pairs: Array<{ i: number; j: number }> = [];
  for (const [s, cb] of countB) {
    if (cb === 1) {
      pairs.push({ i: idxA.get(s)!, j: idxB.get(s)! });
    }
  }
  if (pairs.length === 0) return [];
  pairs.sort((x, y) => x.i - y.i);

  // O(K log K) LIS over `j`
  const tails = new Int32Array(pairs.length);
  const prev = new Int32Array(pairs.length).fill(-1);
  let len = 0;
  for (let k = 0; k < pairs.length; k++) {
    const val = pairs[k].j;
    let lo = 0;
    let hi = len;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (pairs[tails[mid]].j < val) lo = mid + 1;
      else hi = mid;
    }
    if (lo > 0) prev[k] = tails[lo - 1];
    tails[lo] = k;
    if (lo === len) len++;
  }

  const anchors: Array<{ i: number; j: number }> = new Array(len);
  let cur = tails[len - 1];
  for (let pos = len - 1; pos >= 0; pos--) {
    anchors[pos] = pairs[cur];
    cur = prev[cur];
  }
  return anchors;
}

export function diffLineArrays(
  oldLines: readonly string[],
  newLines: readonly string[],
): LineDiffOp[] {
  let prefix = 0;
  while (
    prefix < oldLines.length &&
    prefix < newLines.length &&
    oldLines[prefix] === newLines[prefix]
  ) {
    prefix++;
  }
  let endOld = oldLines.length;
  let endNew = newLines.length;
  while (endOld > prefix && endNew > prefix && oldLines[endOld - 1] === newLines[endNew - 1]) {
    endOld--;
    endNew--;
  }

  const raw: RawEdit[] = [];
  for (let k = 0; k < prefix; k++) {
    raw.push({ op: 'equal', line: oldLines[k] });
  }

  const midOld = oldLines.slice(prefix, endOld);
  const midNew = newLines.slice(prefix, endNew);
  if (midOld.length * midNew.length > MAX_EXACT_DP_CELLS) {
    const anchors = findUniqueAnchors(midOld, midNew);
    let prevI = 0;
    let prevJ = 0;
    for (let idx = 0; idx < anchors.length; idx++) {
      const anc = anchors[idx];
      exactSliceLcs(midOld.slice(prevI, anc.i), midNew.slice(prevJ, anc.j), raw);
      raw.push({ op: 'equal', line: midOld[anc.i] });
      prevI = anc.i + 1;
      prevJ = anc.j + 1;
    }
    exactSliceLcs(midOld.slice(prevI), midNew.slice(prevJ), raw);
  } else if (midOld.length > 0 || midNew.length > 0) {
    exactSliceLcs(midOld, midNew, raw);
  }

  for (let k = endOld; k < oldLines.length; k++) {
    raw.push({ op: 'equal', line: oldLines[k] });
  }

  const ops: LineDiffOp[] = new Array(raw.length);
  let oldLn = 0;
  let newLn = 0;
  for (let idx = 0; idx < raw.length; idx++) {
    const r = raw[idx];
    if (r.op === 'equal') {
      oldLn++;
      newLn++;
      ops[idx] = { op: 'equal', line: r.line, oldLineNo: oldLn, newLineNo: newLn };
    } else if (r.op === 'delete') {
      oldLn++;
      ops[idx] = { op: 'delete', line: r.line, oldLineNo: oldLn, newLineNo: 0 };
    } else {
      newLn++;
      ops[idx] = { op: 'insert', line: r.line, oldLineNo: 0, newLineNo: newLn };
    }
  }
  return ops;
}

export function diffTextLines(oldText: string, newText: string): LineDiffOp[] {
  if (oldText === newText) {
    const lines = splitDiffLines(oldText);
    return lines.map((line, idx) => ({
      op: 'equal',
      line,
      oldLineNo: idx + 1,
      newLineNo: idx + 1,
    }));
  }
  return diffLineArrays(splitDiffLines(oldText), splitDiffLines(newText));
}
