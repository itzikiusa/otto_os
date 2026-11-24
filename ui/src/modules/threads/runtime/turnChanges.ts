// The end-of-turn changes card's numbers: the files a finished turn changed,
// from its own file-change items — one shape for every provider's driver
// (`file_name` plus a unified `diff_str` and / or `old_str` / `new_str`;
// Claude's Edit / Write, Codex's fileChange, ACP and pi edits). A file whose
// items carry no text shows its name only. Pure (unit/runtimeTurnChanges.test.ts).
import type { TurnItem } from '../../../lib/api/types';
import { sameItems } from './stableMap.ts';
import { relativeToCwd } from './workLog';
import { perfCount } from '../../../lib/perfCount.ts';

export interface FileDelta {
  path: string;
  /** Lines added / removed; null when the items carry no diff text. */
  added: number | null;
  removed: number | null;
}

const lines = (s: string): string[] => (s === '' ? [] : s.replace(/\n$/, '').split('\n'));

/** One file-change item's +/− lines, or null when it carries no text. */
export function diffCounts(item: { diff_str?: string; old_str?: string; new_str?: string }): { added: number; removed: number } | null {
  if (item.diff_str) {
    let added = 0;
    let removed = 0;
    for (const l of item.diff_str.split('\n')) {
      if (l.startsWith('+') && !l.startsWith('+++')) added++;
      else if (l.startsWith('-') && !l.startsWith('---')) removed++;
    }
    return { added, removed };
  }
  if (item.old_str === undefined && item.new_str === undefined) return null;
  const a = lines(item.old_str ?? '');
  const b = lines(item.new_str ?? '');
  let head = 0;
  while (head < a.length && head < b.length && a[head] === b[head]) head++;
  let tail = 0;
  while (tail < a.length - head && tail < b.length - head && a[a.length - 1 - tail] === b[b.length - 1 - tail]) tail++;
  return { added: b.length - head - tail, removed: a.length - head - tail };
}

/** The files turn `turnId` changed (completed, top-level file changes), in
 *  first-touched order, paths relative to the thread's folder. */
export function turnChanges(items: readonly TurnItem[], turnId: string, cwd: string | null | undefined): FileDelta[] {
  perfCount('turnChanges');
  const byPath = new Map<string, FileDelta>();
  for (const it of items) {
    if (it.type !== 'file_change' || it.turn_id !== turnId || it.parent_item_id || it.status !== 'completed' || it.error) continue;
    const path = relativeToCwd(it.file_name, cwd);
    const c = diffCounts(it);
    const d = byPath.get(path) ?? { path, added: null, removed: null };
    if (c) {
      d.added = (d.added ?? 0) + c.added;
      d.removed = (d.removed ?? 0) + c.removed;
    }
    byPath.set(path, d);
  }
  return [...byPath.values()];
}

/** Every finished turn's changes in ONE pass over `items` (the timeline's
 *  changes cards): `turnChanges` per turn rescanned every item for every
 *  turn on each stream event. */
let lastChanges: { cwd: string | null | undefined; edits: TurnItem[]; out: Map<string, FileDelta[]> } | null = null;

export function turnChangesByTurn(items: readonly TurnItem[], cwd: string | null | undefined): Map<string, FileDelta[]> {
  // A streamed chunk of the answer changes no file edit: the same edits in,
  // the same Map (and arrays) out — what reads it stays put.
  const edits = items.filter((it) => it.type === 'file_change' && !!it.turn_id && !it.parent_item_id && it.status === 'completed' && !it.error);
  if (lastChanges && lastChanges.cwd === cwd && sameItems(lastChanges.edits, edits)) return lastChanges.out;
  perfCount('turnChangesByTurn');
  const byTurn = new Map<string, Map<string, FileDelta>>();
  for (const it of edits) {
    if (it.type !== 'file_change' || !it.turn_id) continue;
    const path = relativeToCwd(it.file_name, cwd);
    const c = diffCounts(it);
    let m = byTurn.get(it.turn_id);
    if (!m) byTurn.set(it.turn_id, (m = new Map()));
    const d = m.get(path) ?? { path, added: null, removed: null };
    if (c) {
      d.added = (d.added ?? 0) + c.added;
      d.removed = (d.removed ?? 0) + c.removed;
    }
    m.set(path, d);
  }
  const out = new Map([...byTurn].map(([t, m]) => [t, [...m.values()]]));
  lastChanges = { cwd, edits, out };
  return out;
}

/** The card's totals; null counts when no file had numbers. */
export function changeTotals(files: readonly FileDelta[]): { added: number | null; removed: number | null } {
  const known = files.filter((f) => f.added !== null);
  if (!known.length) return { added: null, removed: null };
  return { added: known.reduce((n, f) => n + (f.added ?? 0), 0), removed: known.reduce((n, f) => n + (f.removed ?? 0), 0) };
}
