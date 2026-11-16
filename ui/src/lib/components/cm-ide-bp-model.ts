// Pure breakpoint/debug decisions behind the IDE gutter (no DOM, no view): how breakpoints
// follow edits, how a breakpoint's binding state renders, and which lines carry the
// "paused here" and selected-frame marks. The CodeMirror wiring lives in cm-ide-extensions.ts.

import { Transaction, type ChangeDesc, type Text } from '@codemirror/state';
import { outsideText } from './cm-outside-text.ts';
import type { IdeBreakpoint } from '../api/types.ts';

/** A breakpoint that an edit carried from line `from` (the line the store knows) to `to`;
 *  `to: null` = it was dropped: its line was deleted, or it met another breakpoint (`into`). */
export interface IdeBreakpointMove { from: number; to: number | null; reason?: 'deleted' | 'merged'; into?: number }

/** The gutter's breakpoints by current line, plus each one's line as the store knows it. */
export interface BpLines<T> { map: Map<number, T>; origin: Map<number, number>; dropped: IdeBreakpointMove[] }

/** Whether a transaction's edits carry breakpoints along. Text the editor received from
 *  outside (a reload from disk, a collab peer, the store) is tagged `outsideText` (or
 *  `Transaction.remote`) and keeps breakpoints on their line numbers, as does replacing the
 *  whole document. */
export function shouldMapBreakpoints(tr: Transaction): boolean {
  if (!tr.docChanged || tr.annotation(outsideText) || tr.annotation(Transaction.remote)) return false;
  const len = tr.startState.doc.length;
  let whole = false;
  tr.changes.iterChangedRanges((fromA, toA) => { if (len > 0 && fromA === 0 && toA === len) whole = true; });
  return !whole;
}

const hasDetails = (bp: object): boolean => {
  const b = bp as Partial<IdeBreakpoint>;
  return Boolean(b.condition || b.hit_condition || b.log_message);
};

/** Maps breakpoint lines through `changes`. A line whose text AND line break were deleted
 *  drops its breakpoint; two breakpoints meeting on one line keep one (the one that stayed,
 *  unless only the other carries a condition/log) and report the other as merged.
 *  Returns null when nothing moved. */
export function mapBreakpointLines<T extends { line: number }>(
  cur: BpLines<T>, changes: ChangeDesc, startDoc: Text, doc: Text,
): BpLines<T> | null {
  if (!cur.map.size) return null;
  const deleted: Array<[number, number]> = [];
  changes.iterChangedRanges((fromA, toA) => { if (toA > fromA) deleted.push([fromA, toA]); });
  const map = new Map<number, T>(), origin = new Map<number, number>(), dropped = [...cur.dropped];
  let changed = false;
  for (const ln of [...cur.map.keys()].sort((a, b) => a - b)) {
    const bp = cur.map.get(ln) as T, base = cur.origin.get(ln) ?? ln;
    if (ln < 1 || ln > startDoc.lines) { map.set(ln, bp); origin.set(ln, base); continue; }
    const line = startDoc.line(ln);
    if (deleted.some(([a, b]) => a <= line.from && b >= line.to && (b > line.to || a < line.from))) {
      dropped.push({ from: base, to: null, reason: 'deleted' }); changed = true; continue;
    }
    const to = doc.lineAt(Math.min(changes.mapPos(line.from, 1), doc.length)).number;
    if (to !== ln) changed = true;
    const held = map.get(to);
    if (held !== undefined) {
      const keepIncoming = !hasDetails(held) && hasDetails(bp);
      dropped.push({ from: keepIncoming ? (origin.get(to) ?? to) : base, to: null, reason: 'merged', into: to });
      changed = true;
      if (!keepIncoming) continue;
    }
    map.set(to, to === ln ? bp : { ...bp, line: to }); origin.set(to, base);
  }
  return changed ? { map, origin, dropped } : null;
}

/** What the gutter has not told the store yet: every carried line plus every drop. */
export function pendingBreakpointMoves(st: Pick<BpLines<unknown>, 'origin' | 'dropped'>): IdeBreakpointMove[] {
  const out = [...st.dropped];
  for (const [cur, base] of st.origin) if (cur !== base) out.push({ from: base, to: cur });
  return out;
}

/** Applies gutter moves to the store's list for one file: lines rewritten once, drops
 *  removed, conditions/hit counts/log messages kept, binding state reset (the session
 *  answers again for the new line), and never two breakpoints on one line. */
export function applyBreakpointMoves(list: IdeBreakpoint[], moves: IdeBreakpointMove[]): IdeBreakpoint[] {
  const byFrom = new Map(moves.map((m) => [m.from, m]));
  const out: IdeBreakpoint[] = [];
  const taken = new Set<number>();
  const moved = list.map((bp) => {
    const m = byFrom.get(bp.line);
    if (!m) return bp;
    if (m.to == null) return null;
    const { verified: _v, message: _m, ...rest } = bp;
    return { ...rest, line: m.to };
  });
  // Unmoved breakpoints claim their line first, so a moved one never displaces them.
  for (const pass of [false, true]) {
    for (let i = 0; i < list.length; i++) {
      const bp = moved[i];
      if (!bp || (bp !== list[i]) !== pass || taken.has(bp.line)) continue;
      taken.add(bp.line); out.push(bp);
    }
  }
  return out.sort((a, b) => a.line - b.line);
}

/** Whether `list` is what the gutter already shows (the store echoing a move back). */
export function sameBreakpointLines(map: Map<number, IdeBreakpoint>, list: IdeBreakpoint[]): boolean {
  if (map.size !== list.length) return false;
  for (const bp of list) {
    const m = map.get(bp.line);
    if (!m || m.enabled !== bp.enabled || (m.condition ?? null) !== (bp.condition ?? null) || (m.hit_condition ?? null) !== (bp.hit_condition ?? null)
      || (m.log_message ?? null) !== (bp.log_message ?? null) || m.verified !== bp.verified || (m.message ?? null) !== (bp.message ?? null)) return false;
  }
  return true;
}

export type BpVisualKind = 'enabled' | 'disabled' | 'muted' | 'conditional' | 'logpoint' | 'unbound' | 'paused' | 'preview' | 'frame';

/** How one breakpoint renders. `verified` (the check mark) comes only from the debugger's
 *  own answer for this breakpoint; `verified === false` is a hollow muted circle saying why;
 *  no answer yet (no live session) is the normal pending dot. */
export function breakpointAppearance(bp: IdeBreakpoint, ln: number, muted: boolean): { kind: BpVisualKind; tooltip: string; verified: boolean } {
  if (muted) return { kind: 'muted', tooltip: `Muted breakpoint at line ${ln}`, verified: false };
  if (!bp.enabled) return { kind: 'disabled', tooltip: `Disabled breakpoint at line ${ln}`, verified: false };
  if (bp.verified === false) {
    return { kind: 'unbound', tooltip: `Not bound: ${bp.message?.trim() || 'the debugger could not place a breakpoint on this line'}`, verified: false };
  }
  const verified = bp.verified === true;
  const bound = verified ? ' (bound)' : '';
  if (bp.log_message) return { kind: 'logpoint', tooltip: `Logpoint at line ${ln}${bound}: ${bp.log_message}`, verified };
  if (bp.condition) return { kind: 'conditional', tooltip: `Conditional breakpoint at line ${ln}${bound}: ${bp.condition}`, verified };
  if (bp.hit_condition) return { kind: 'conditional', tooltip: `Breakpoint at line ${ln}${bound}, hit count ${bp.hit_condition}`, verified };
  return { kind: 'enabled', tooltip: `Breakpoint at line ${ln}${bound}`, verified };
}

/** The muted selected-frame line: only when a caller frame is selected, never on the stop. */
export function selectedFrameMark(pausedLine: number | null, selectedLine: number | null | undefined): number | null {
  return selectedLine != null && selectedLine >= 1 && selectedLine !== pausedLine ? selectedLine : null;
}
