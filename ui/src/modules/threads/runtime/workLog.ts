// The work log's quiet presentation, after T3 Code's (MIT, Copyright (c) 2026
// T3 Tools Inc.) apps/web/src/components/chat/TimelineWorkGroupRows.tsx and
// session-logic.ts: a finished stretch of tool calls folds into one "Worked
// for 2m 14s" line, a running group names its live call "+N more", and each
// row reads like what was done — no `cd <dir> &&` preamble, paths relative to
// the thread's folder, the call's duration, a failed command as "exit 1".
// Pure (unit/runtimeWorkLog.test.ts).
import type { ProviderTurn, TurnItem } from '../../../lib/api/types';
import { threadCall } from './agentRows';
import { formatDuration, groupSummary, toolError, workLabel, type TimelineRow, type WorkEntry, type WorkPart } from './timeline';
import { perfCount } from '../../../lib/perfCount.ts';

export { formatDuration };

const LIVE: ReadonlySet<string> = new Set(['running', 'pending', 'waiting']);
export const isLive = (i: TurnItem): boolean => LIVE.has(i.status);

/** `cd /repo && npm test` → `npm test` (also `cd x; …`, quoted dirs, `pushd`). */
export function stripCdPrefix(cmd: string): string {
  const out = cmd.replace(/^\s*(?:cd|pushd)\s+(?:"[^"]*"|'[^']*'|[^\s;&|]+)\s*(?:&&|;)\s*/, '');
  return out.trim() ? out : cmd;
}

/** Every `<cwd>/` in `text` dropped, so `/Users/me/repo/src/a.ts` reads `src/a.ts`. */
export function relativeToCwd(text: string, cwd: string | null | undefined): string {
  const root = cwd?.replace(/\/+$/, '');
  if (!root || root === '' || root === '/') return text;
  return text.split(`${root}/`).join('');
}

/** A finished call's run time, or null when it has no start / end.
 *  Legacy persisted reasoning rows had identical start/end (`0ms`); hide `<50ms` on reasoning. */
export function itemDuration(i: TurnItem): string | null {
  if (!i.started_at || !i.completed_at || isLive(i)) return null;
  const ms = Date.parse(i.completed_at) - Date.parse(i.started_at);
  if (!Number.isFinite(ms) || ms < 0) return null;
  if (i.type === 'reasoning' && ms < 50) return null;
  return formatDuration(ms);
}

/** "Error: Exit code 2" / "Exit code 2" → "exit 2"; anything else unchanged. */
export function exitText(error: string): string {
  const m = /^\s*(?:error:\s*)?exit(?:ed with)? code:?\s*(-?\d+)\s*$/i.exec(error);
  return m ? `exit ${m[1]}` : error;
}

/** A work row's one-line label, cleaned for reading. */
export function rowLabel(item: TurnItem, cwd: string | null | undefined): string {
  const raw = workLabel(item);
  return relativeToCwd(item.type === 'command_execution' ? stripCdPrefix(raw) : raw, cwd);
}

/** Something in the stretch a person must see: a failure, a pending
 *  approval or a gated call's outcome. Such a stretch never folds. */
export function needsEyes(entries: readonly WorkEntry[], pendingRequestIds: ReadonlySet<string>): boolean {
  return entries.some(
    ({ item, approval }) =>
      item.status === 'failed' ||
      item.type === 'error' ||
      (item.type === 'approval_request' && pendingRequestIds.has(item.request_id)) ||
      // A gated call keeps its decision ("Denied", "Withdrawn by Codex") in sight.
      !!approval ||
      ('error' in item && typeof item.error === 'string' && item.error !== ''),
  );
}

/** The stretch is over: nothing in it runs and its turn(s) ended. */
export function stretchDone(
  entries: readonly WorkEntry[],
  turns: readonly ProviderTurn[],
  turnById?: ReadonlyMap<string, ProviderTurn>,
): boolean {
  if (entries.some((e) => isLive(e.item))) return false;
  const byId = turnById ?? new Map(turns.map((t) => [t.id, t] as const));
  return entries.every(({ item }) => {
    const t = item.turn_id ? byId.get(item.turn_id) : undefined;
    return !t || (t.status !== 'running' && t.status !== 'pending');
  });
}

/** "Worked for 2m 14s" over the stretch's wall time (first start → last
 *  end), "Worked" when the times are missing. */
export function workedFor(entries: readonly WorkEntry[]): string {
  let start = Infinity;
  let end = -Infinity;
  for (const { item } of entries) {
    const s = Date.parse(item.started_at ?? '');
    const e = Date.parse(item.completed_at ?? item.updated_at);
    if (Number.isFinite(s)) start = Math.min(start, s);
    if (Number.isFinite(e)) end = Math.max(end, e);
  }
  const d = end - start;
  return Number.isFinite(d) && d >= 1000 ? `Worked for ${formatDuration(d)}` : 'Worked';
}

/** What folds: tool calls and thinking. A notice, a subagent, a thread the
 *  agent opened (its live card), a task list, an approval or an error is
 *  news, so a stretch holding one stays open. */
const FOLDABLE: ReadonlySet<TurnItem['type']> = new Set(['command_execution', 'file_change', 'web_search', 'dynamic_tool', 'reasoning']);

/** A finished stretch folds when it holds more than one step, only tool
 *  calls / thinking, and nothing in it needs a person. */
export function shouldFold(
  entries: readonly WorkEntry[],
  turns: readonly ProviderTurn[],
  pendingRequestIds: ReadonlySet<string>,
  turnById?: ReadonlyMap<string, ProviderTurn>,
): boolean {
  return (
    entries.length > 1 &&
    entries.every((e) => FOLDABLE.has(e.item.type) && threadCall(e.item)?.verb !== 'Opened') &&
    stretchDone(entries, turns, turnById) &&
    !needsEyes(entries, pendingRequestIds)
  );
}

/** A folded stretch's line: how long, then what it did in words (T3's
 *  summarizeToolGroup) — "Worked for 2m 14s · ran 2 commands and edited 3
 *  files"; thinking alone adds nothing. */
export function foldLine(entries: readonly WorkEntry[]): string {
  perfCount('foldLine');
  const tools = entries.filter((e) => e.item.type !== 'reasoning');
  const did = tools.length ? groupSummary(tools) : '';
  return did ? `${workedFor(entries)} · ${did.charAt(0).toLowerCase()}${did.slice(1)}` : workedFor(entries);
}

/** How much tool-call noise the timeline shows (threadPrefs, ⋯ › View),
 *  after T3 Code's toolCallVisibility (packages/contracts/src/settings.ts). */
export type ToolVisibility = 'all' | 'hide-ok' | 'none';

const HIDEABLE: ReadonlySet<TurnItem['type']> = new Set(['command_execution', 'file_change', 'web_search', 'dynamic_tool']);

/** A tool call this choice hides. A failure, a gated call (its approval and
 *  outcome), a thread the agent opened, and everything that is not a tool
 *  call — messages, questions, approvals, notices — always show. */
export function hiddenTool({ item, approval }: WorkEntry, v: ToolVisibility): boolean {
  if (v === 'all' || !HIDEABLE.has(item.type) || approval || item.status === 'failed' || toolError(item)) return false;
  if (item.type === 'dynamic_tool' && threadCall(item)) return false;
  return v === 'none' || item.status === 'completed';
}

/** The rows with the hidden tool calls left out (a stretch left empty goes too). */
export function visibleTools(rows: readonly TimelineRow[], v: ToolVisibility): TimelineRow[] {
  if (v === 'all') return rows as TimelineRow[];
  const out: TimelineRow[] = [];
  for (const r of rows) {
    if (r.kind !== 'work') {
      out.push(r);
      continue;
    }
    const parts: WorkPart[] = [];
    for (const p of r.parts) {
      if (p.kind === 'item') {
        if (!hiddenTool(p.entry, v)) parts.push(p);
        continue;
      }
      const kept = p.entries.filter((e) => !hiddenTool(e, v));
      if (kept.length === p.entries.length) parts.push(p);
      else if (kept.length === 1) parts.push({ kind: 'item', entry: kept[0] });
      else if (kept.length > 1) parts.push({ ...p, entries: kept });
    }
    if (parts.length === r.parts.length && parts.every((p, i) => p === r.parts[i])) out.push(r);
    else if (parts.length) out.push({ ...r, parts });
  }
  return out;
}

/** While a stretch runs, how many of its leading steps fold into "N
 *  earlier steps" (T3's live work log): the finished ones before the live
 *  step, up to the first that needs a person (a failure, an approval) —
 *  that one and everything after it stays in sight. Fewer than two: none. */
export function earlierSteps(parts: readonly WorkPart[], pendingRequestIds: ReadonlySet<string>): number {
  const entriesOf = (p: WorkPart): WorkEntry[] => (p.kind === 'item' ? [p.entry] : p.entries);
  if (!parts.some((p) => entriesOf(p).some((e) => isLive(e.item)))) return 0;
  let n = 0;
  while (n < parts.length - 1) {
    const es = entriesOf(parts[n]);
    if (es.some((e) => isLive(e.item)) || needsEyes(es, pendingRequestIds) || !es.every((e) => FOLDABLE.has(e.item.type))) break;
    n++;
  }
  return n >= 2 ? n : 0;
}

/** A running group's line: its live call by name, then "+N more". Null
 *  when nothing in it runs (the group keeps its summary sentence). */
export function liveSummary(entries: readonly WorkEntry[], cwd: string | null | undefined): string | null {
  const live = entries.filter((e) => isLive(e.item));
  if (live.length === 0) return null;
  const rest = entries.length - 1;
  return `${rowLabel(live[live.length - 1].item, cwd)}${rest > 0 ? ` +${rest} more` : ''}`;
}
