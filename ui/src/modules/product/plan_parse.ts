// Pure parser + targeted writer for the implementation-plan markdown produced by
// the `story-task-breakdown` skill. The plan uses `### Task N: <title>` headings
// followed by GitHub-style checkbox lines. The PO toggles checkboxes in the UI;
// `setItemStatus` rewrites a single line's marker, avoiding lossy round-tripping.
//
// Marker convention (must match the backend skill + plan-format.md reference):
//   `[ ]`              → todo
//   `[x]` / `[X]`      → done
//   `[~]` / `[>]` / `[-]` → in_progress

export type Status = 'todo' | 'in_progress' | 'done';

export interface Item {
  text: string;
  status: Status;
  /** Absolute 0-based line index in the source markdown. */
  lineIndex: number;
  /** The original marker character found between the brackets (e.g. ' ', 'x', '~'). */
  marker: string;
}

export interface Task {
  title: string;
  /** Absolute 0-based line index of the heading line. */
  lineIndex: number;
  items: Item[];
  status: Status;
}

// A heading is any 2–4 level ATX heading. The skill emits `### Task N: ...`.
const HEADING_RE = /^#{2,4}\s+(.+?)\s*$/;
// A checkbox line: optional indent, - or *, a space, [marker], a space, text.
const CHECKBOX_RE = /^\s*[-*]\s*\[( |x|X|~|>|-)\]\s+(.+?)\s*$/;

/** Map a raw marker character to a status. */
function markerToStatus(marker: string): Status {
  if (marker === 'x' || marker === 'X') return 'done';
  if (marker === '~' || marker === '>' || marker === '-') return 'in_progress';
  return 'todo';
}

/** Map a status to the canonical marker the UI writes back. */
export function statusToMarker(status: Status): string {
  if (status === 'done') return 'x';
  if (status === 'in_progress') return '~';
  return ' ';
}

/** Derive a task's rollup status from its items. */
function deriveTaskStatus(items: Item[]): Status {
  if (items.length === 0) return 'todo';
  const done = items.filter((i) => i.status === 'done').length;
  const inProgress = items.filter((i) => i.status === 'in_progress').length;
  if (done === items.length) return 'done';
  // Any explicit in-progress item, or a partially-done set, reads as in_progress.
  if (inProgress > 0 || done > 0) return 'in_progress';
  return 'todo';
}

/**
 * Parse plan markdown into a task tree. Each heading starts a task; the checkbox
 * lines that follow (until the next heading) become that task's items. Lines
 * before the first heading are ignored.
 */
export function parsePlan(md: string): { tasks: Task[] } {
  const lines = md.split('\n');
  const tasks: Task[] = [];
  let current: Task | null = null;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const heading = HEADING_RE.exec(line);
    if (heading) {
      current = { title: heading[1], lineIndex: i, items: [], status: 'todo' };
      tasks.push(current);
      continue;
    }
    if (!current) continue;
    const cb = CHECKBOX_RE.exec(line);
    if (cb) {
      const marker = cb[1];
      current.items.push({
        text: cb[2],
        status: markerToStatus(marker),
        lineIndex: i,
        marker,
      });
    }
  }

  for (const t of tasks) t.status = deriveTaskStatus(t.items);

  return { tasks };
}

/**
 * Rewrite the marker on a single checkbox line. Replaces only the `[x]`/`[ ]`/etc.
 * token on `lineIndex`, preserving indentation, bullet char, and step text — so
 * the rest of the document is untouched. Returns the new markdown (input
 * unchanged if the line is not a recognizable checkbox).
 */
export function setItemStatus(md: string, lineIndex: number, status: Status): string {
  const lines = md.split('\n');
  if (lineIndex < 0 || lineIndex >= lines.length) return md;
  const line = lines[lineIndex];
  const marker = statusToMarker(status);
  // Replace the first `[<single char>]` on the line with `[marker]`.
  const replaced = line.replace(/\[( |x|X|~|>|-)\]/, `[${marker}]`);
  if (replaced === line) return md;
  lines[lineIndex] = replaced;
  return lines.join('\n');
}
