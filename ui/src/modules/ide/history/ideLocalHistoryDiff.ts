// Pure client-side line-diff builder converting a Local History snapshot +
// current on-disk file content into Otto's standard `DiffResp` shape so
// `DiffViewer.svelte` renders unified/split syntax-highlighted + word-diff
// comparisons with zero custom diff UI.

import type { DiffLine, DiffResp, FileChangeStatus, Hunk } from '../../../lib/api/types';
import { diffLineArrays, splitDiffLines } from '../../../lib/lineDiff.ts';

const CONTEXT_LINES = 3;

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
  const snapLines = splitDiffLines(snapshotContent);

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

  const diskLines = splitDiffLines(currentContent);
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

  const rawOps: DiffLine[] = diffLineArrays(diskLines, snapLines).map((op) =>
    op.op === 'equal'
      ? { origin: 'context', content: op.line, old_line: op.oldLineNo, new_line: op.newLineNo }
      : op.op === 'delete'
        ? { origin: 'del', content: op.line, old_line: op.oldLineNo, new_line: null }
        : { origin: 'add', content: op.line, old_line: null, new_line: op.newLineNo },
  );
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
