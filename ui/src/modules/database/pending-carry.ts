// Keep un-applied edits when the grid is handed a NEW result for the same tab
// and column shape (a re-run, a refresh after a partial apply, a page there and
// back). Pending drafts are keyed by liveRows INDEX, so a new result used to
// drop them all; an open inline editor kept pointing at an index that now
// holds another row. Here they are re-keyed by the row's primary-key values:
// a draft follows its row to the new index, and only a draft whose row is no
// longer in the result is dropped (and counted, so the grid can say so).
// Pure apart from the `flow` it is handed, so node tests drive it directly.
import type { RowPatch } from './edit-types';

/** The slice of EditFlow this needs (structural, so tests pass a plain object). */
export interface CarryFlow {
  result: { columns: { name: string }[] } | null;
  liveRows: unknown[][];
  editPkCols: string[];
  pending: Map<number, RowPatch>;
  editing: { rowIdx: number; colIdx: number; value: string } | null;
  viewer: { edit: { rowIdx: number; colIdx: number } | null } | null;
  viewerEditing: boolean;
  parkDraft(): Map<number, RowPatch>;
  saveViewerEdit(): void;
}

/** Park every open editor's draft (the inline cell input, and the cell
 *  viewer's editor when its draft is valid) as a pending change, and return
 *  the pending map — what the grid stashes when its view state is parked. */
export function parkOpenEditors(flow: CarryFlow): Map<number, RowPatch> {
  if (flow.viewerEditing && flow.viewer?.edit) flow.saveViewerEdit();
  return flow.parkDraft();
}

/** Column indexes of the primary key in `columns`, or null when one is missing
 *  (no key → no way to follow a row across results). */
export function pkIndexes(columns: readonly { name: string }[] | undefined, pkCols: readonly string[]): number[] | null {
  if (!columns || pkCols.length === 0) return null;
  const idx = pkCols.map((n) => columns.findIndex((c) => c.name === n));
  return idx.includes(-1) ? null : idx;
}

/** A row's identity: its key values, serialized exactly (64-bit ids arrive as strings). */
export function rowKey(row: readonly unknown[], idx: readonly number[]): string {
  return JSON.stringify(idx.map((i) => row[i] ?? null));
}

/** Before the grid re-seeds `rows` (the OUTGOING result's rows): park the open
 *  editors, then key the pending drafts by row. Null when there is nothing to
 *  carry or no key to carry it by. An inline editor open on rows the flow no
 *  longer holds is closed, never committed onto another row. */
export function carryPending(flow: CarryFlow, rows: unknown[][]): Map<string, RowPatch> | null {
  if (flow.liveRows === rows) parkOpenEditors(flow);
  else flow.editing = null;
  const idx = pkIndexes(flow.result?.columns, flow.editPkCols);
  if (!idx || flow.pending.size === 0) return null;
  const out = new Map<string, RowPatch>();
  for (const [r, patch] of flow.pending) if (rows[r]) out.set(rowKey(rows[r], idx), patch);
  return out.size ? out : null;
}

/** The statement text as "the same query" compares it: outer whitespace and a
 *  trailing semicolon dropped, whitespace runs OUTSIDE quotes collapsed to one
 *  space. Identifiers and literals (and the spaces inside a literal) are kept
 *  exactly — `users_stg` and `users` are different queries. */
export function sameQueryText(statement: string | null | undefined): string {
  const src = (statement ?? '').trim().replace(/;\s*$/, '');
  let out = '';
  let quote: string | null = null;
  let esc = false;
  let ws = false;
  for (const ch of src) {
    if (quote) {
      out += ch;
      if (esc) esc = false;
      else if (ch === '\\') esc = true;
      else if (ch === quote) quote = null;
      continue;
    }
    if (/\s/.test(ch)) {
      ws = true;
      continue;
    }
    if (ws) out += ' ';
    ws = false;
    if (ch === "'" || ch === '"' || ch === '`') quote = ch;
    out += ch;
  }
  return out;
}

/** What a result was produced BY: the statement (see `sameQueryText`), the
 *  scope node it ran on and the connection. Drafts carry to a new result only
 *  when this is unchanged — matching rows by key alone would move an edit made
 *  on `users_stg` row 5 onto `users` row 5 after the FROM was changed, because
 *  the edit's target table is resolved from the NEW statement. */
export function carryScope(statement: string | null | undefined, node: string | null | undefined, connectionId: string | null | undefined): string {
  return JSON.stringify([sameQueryText(statement), node ?? null, connectionId ?? null]);
}

/** A new result arrived in the same tab. `same` = same column shape AND same
 *  `carryScope`: then the drafts are keyed for `restorePending`. Otherwise every
 *  draft (open editors parked first, so a half-typed cell is counted too) is
 *  dropped and counted — the grid tells the person instead of losing them
 *  silently or re-targeting them. */
export function carryOrDrop(flow: CarryFlow, rows: unknown[][], same: boolean): { carry: Map<string, RowPatch> | null; dropped: number } {
  if (same) return { carry: carryPending(flow, rows), dropped: 0 };
  if (flow.liveRows === rows) parkOpenEditors(flow);
  else flow.editing = null;
  return { carry: null, dropped: flow.pending.size };
}

/** After the re-seed: re-attach each carried draft to the row with the same
 *  key in the NEW `rows` (first match; a non-unique key never doubles a draft).
 *  Returns how many drafts found no row and were dropped. */
export function restorePending(flow: CarryFlow, carried: Map<string, RowPatch>, rows: unknown[][]): number {
  const idx = pkIndexes(flow.result?.columns, flow.editPkCols);
  const left = new Map(carried);
  const next = new Map<number, RowPatch>();
  if (idx)
    rows.forEach((row, i) => {
      const k = rowKey(row, idx);
      const patch = left.get(k);
      if (patch) {
        next.set(i, patch);
        left.delete(k);
      }
    });
  flow.pending = next;
  return left.size;
}
