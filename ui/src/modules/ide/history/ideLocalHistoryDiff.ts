// Pure client-side line-diff builder converting a Local History snapshot +
// current on-disk file content into Otto's standard `DiffResp` shape so
// `DiffViewer.svelte` renders unified/split syntax-highlighted + word-diff
// comparisons with zero custom diff UI.

import type { DiffLine, DiffResp, FileChangeStatus, Hunk } from '../../../lib/api/types';

const MAX_LCS_CELLS = 1_600_000;
const CONTEXT_LINES = 3;

function splitLines(text: string): string[] {
  if (!text) return [];
  const normalized = text.replace(/\r\n/g, '\n');
  const parts = normalized.split('\n');
  if (parts.length > 0 && parts[parts.length - 1] === '') parts.pop();
  return parts;
}

/**
 * Builds a `DiffResp` comparing `currentContent` (OLD side = what is currently
 * on disk) → `snapshotContent` (NEW side = what restoring this Local History
 * entry will write to disk).
 *
 * When `currentExists` is false (a deleted file), every line in the snapshot
 * is rendered as `'add'` so the user sees the complete recoverable file!
 */
export function buildLocalHistoryDiff(
  relPath: string,
  snapshotContent: string,
  currentContent: string | null | undefined,
  currentExists: boolean,
): DiffResp {
  const path = relPath || 'untitled';
  const snapLines = splitLines(snapshotContent);

  if (!currentExists || currentContent == null) {
    const lines: DiffLine[] = snapLines.map((content, idx) => ({
      origin: 'add',
      content,
      old_line: null,
      new_line: idx + 1,
    }));
    const hunks: Hunk[] =
      lines.length > 0
        ? [{ header: `@@ -0,0 +1,${lines.length} @@ Recoverable snapshot`, lines }]
        : [];
    return {
      files: [
        {
          path,
          old_path: null,
          is_binary: false,
          hunks,
          added: lines.length,
          deleted: 0,
          status: 'added',
        },
      ],
      total_added: lines.length,
      total_deleted: 0,
    };
  }

  const diskLines = splitLines(currentContent);
  if (snapshotContent === currentContent) {
    // Identical to disk: show up to first 60 lines as context so user still sees the file.
    const previewCount = Math.min(snapLines.length, 60);
    const lines: DiffLine[] = snapLines.slice(0, previewCount).map((content, idx) => ({
      origin: 'context',
      content,
      old_line: idx + 1,
      new_line: idx + 1,
    }));
    return {
      files: [
        {
          path,
          old_path: null,
          is_binary: false,
          hunks:
            lines.length > 0
              ? [{ header: `@@ -1,${lines.length} +1,${lines.length} @@ Matches current file on disk`, lines }]
              : [],
          added: 0,
          deleted: 0,
          status: 'modified',
        },
      ],
      total_added: 0,
      total_deleted: 0,
    };
  }

  const rawOps = computeLineOps(diskLines, snapLines);
  const { hunks, added, deleted } = groupIntoHunks(rawOps, CONTEXT_LINES);
  const status: FileChangeStatus = 'modified';

  return {
    files: [
      {
        path,
        old_path: null,
        is_binary: false,
        hunks,
        added,
        deleted,
        status,
      },
    ],
    total_added: added,
    total_deleted: deleted,
  };
}

function computeLineOps(oldLines: string[], newLines: string[]): DiffLine[] {
  let prefix = 0;
  const minLen = Math.min(oldLines.length, newLines.length);
  while (prefix < minLen && oldLines[prefix] === newLines[prefix]) {
    prefix += 1;
  }

  let oldEnd = oldLines.length;
  let newEnd = newLines.length;
  while (oldEnd > prefix && newEnd > prefix && oldLines[oldEnd - 1] === newLines[newEnd - 1]) {
    oldEnd -= 1;
    newEnd -= 1;
  }

  const out: DiffLine[] = [];
  for (let i = 0; i < prefix; i++) {
    out.push({ origin: 'context', content: oldLines[i], old_line: i + 1, new_line: i + 1 });
  }

  const m = oldEnd - prefix;
  const n = newEnd - prefix;

  if (m === 0) {
    for (let j = 0; j < n; j++) {
      out.push({
        origin: 'add',
        content: newLines[prefix + j],
        old_line: null,
        new_line: prefix + j + 1,
      });
    }
  } else if (n === 0) {
    for (let i = 0; i < m; i++) {
      out.push({
        origin: 'del',
        content: oldLines[prefix + i],
        old_line: prefix + i + 1,
        new_line: null,
      });
    }
  } else if (m * n <= MAX_LCS_CELLS) {
    const dp = new Int32Array((m + 1) * (n + 1));
    const stride = n + 1;
    for (let i = m - 1; i >= 0; i--) {
      const oLine = oldLines[prefix + i];
      for (let j = n - 1; j >= 0; j--) {
        if (oLine === newLines[prefix + j]) {
          dp[i * stride + j] = dp[(i + 1) * stride + (j + 1)] + 1;
        } else {
          const down = dp[(i + 1) * stride + j];
          const across = dp[i * stride + (j + 1)];
          dp[i * stride + j] = down >= across ? down : across;
        }
      }
    }
    let i = 0;
    let j = 0;
    while (i < m && j < n) {
      if (oldLines[prefix + i] === newLines[prefix + j]) {
        out.push({
          origin: 'context',
          content: oldLines[prefix + i],
          old_line: prefix + i + 1,
          new_line: prefix + j + 1,
        });
        i += 1;
        j += 1;
      } else if (dp[(i + 1) * stride + j] >= dp[i * stride + (j + 1)]) {
        out.push({
          origin: 'del',
          content: oldLines[prefix + i],
          old_line: prefix + i + 1,
          new_line: null,
        });
        i += 1;
      } else {
        out.push({
          origin: 'add',
          content: newLines[prefix + j],
          old_line: null,
          new_line: prefix + j + 1,
        });
        j += 1;
      }
    }
    while (i < m) {
      out.push({
        origin: 'del',
        content: oldLines[prefix + i],
        old_line: prefix + i + 1,
        new_line: null,
      });
      i += 1;
    }
    while (j < n) {
      out.push({
        origin: 'add',
        content: newLines[prefix + j],
        old_line: null,
        new_line: prefix + j + 1,
      });
      j += 1;
    }
  } else {
    // Huge file fallback: linear middle replacement without O(m*n) allocation.
    for (let i = 0; i < m; i++) {
      out.push({
        origin: 'del',
        content: oldLines[prefix + i],
        old_line: prefix + i + 1,
        new_line: null,
      });
    }
    for (let j = 0; j < n; j++) {
      out.push({
        origin: 'add',
        content: newLines[prefix + j],
        old_line: null,
        new_line: prefix + j + 1,
      });
    }
  }

  const tailCount = oldLines.length - oldEnd;
  for (let k = 0; k < tailCount; k++) {
    out.push({
      origin: 'context',
      content: oldLines[oldEnd + k],
      old_line: oldEnd + k + 1,
      new_line: newEnd + k + 1,
    });
  }

  return out;
}

function groupIntoHunks(
  ops: DiffLine[],
  ctx: number,
): { hunks: Hunk[]; added: number; deleted: number } {
  let added = 0;
  let deleted = 0;
  const changeIdx: number[] = [];
  for (let i = 0; i < ops.length; i++) {
    if (ops[i].origin === 'add') {
      added += 1;
      changeIdx.push(i);
    } else if (ops[i].origin === 'del') {
      deleted += 1;
      changeIdx.push(i);
    }
  }
  if (changeIdx.length === 0) return { hunks: [], added: 0, deleted: 0 };

  const ranges: Array<[number, number]> = [];
  for (let k = 0; k < changeIdx.length; k++) {
    const start = Math.max(0, changeIdx[k] - ctx);
    const end = Math.min(ops.length - 1, changeIdx[k] + ctx);
    const prev = ranges[ranges.length - 1];
    if (prev && start <= prev[1] + 1) {
      prev[1] = Math.max(prev[1], end);
    } else {
      ranges.push([start, end]);
    }
  }

  const hunks: Hunk[] = ranges.map(([start, end]) => {
    const slice = ops.slice(start, end + 1);
    let oldStart = 1;
    let newStart = 1;
    let oldCount = 0;
    let newCount = 0;
    let foundOld = false;
    let foundNew = false;
    for (let i = 0; i < slice.length; i++) {
      const l = slice[i];
      if (l.old_line != null) {
        if (!foundOld) {
          oldStart = l.old_line;
          foundOld = true;
        }
        oldCount += 1;
      }
      if (l.new_line != null) {
        if (!foundNew) {
          newStart = l.new_line;
          foundNew = true;
        }
        newCount += 1;
      }
    }
    return {
      header: `@@ -${oldStart},${oldCount} +${newStart},${newCount} @@`,
      lines: slice,
    };
  });

  return { hunks, added, deleted };
}
