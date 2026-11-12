// The work log's quiet presentation, after T3 Code's (MIT, Copyright (c) 2026
// T3 Tools Inc.) apps/web/src/components/chat/TimelineWorkGroupRows.tsx and
// session-logic.ts: a finished stretch of tool calls folds into one "Worked
// for 2m 14s" line, a running group names its live call "+N more", and each
// row reads like what was done — no `cd <dir> &&` preamble, paths relative to
// the thread's folder, the call's duration, a failed command as "exit 1".
// Pure (unit/runtimeWorkLog.test.ts).
import type { ProviderTurn, TurnItem } from '../../../lib/api/types';
import { threadCall } from './agentRows';
import { workLabel, type WorkEntry } from './timeline';

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

/** "820ms", "14s", "2m 14s", "1h 3m". */
export function formatDuration(ms: number): string {
  if (!Number.isFinite(ms) || ms < 0) return '';
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** A finished call's run time, or null when it has no start / end. */
export function itemDuration(i: TurnItem): string | null {
  if (!i.started_at || !i.completed_at || isLive(i)) return null;
  const ms = Date.parse(i.completed_at) - Date.parse(i.started_at);
  return Number.isFinite(ms) && ms >= 0 ? formatDuration(ms) : null;
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
      // A gated call keeps its decision ("Declined", "Withdrawn by Codex") in sight.
      !!approval ||
      ('error' in item && typeof item.error === 'string' && item.error !== ''),
  );
}

/** The stretch is over: nothing in it runs and its turn(s) ended. */
export function stretchDone(entries: readonly WorkEntry[], turns: readonly ProviderTurn[]): boolean {
  if (entries.some((e) => isLive(e.item))) return false;
  const byId = new Map(turns.map((t) => [t.id, t] as const));
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
): boolean {
  return (
    entries.length > 1 &&
    entries.every((e) => FOLDABLE.has(e.item.type) && threadCall(e.item)?.verb !== 'Opened') &&
    stretchDone(entries, turns) &&
    !needsEyes(entries, pendingRequestIds)
  );
}

/** A running group's line: its live call by name, then "+N more". Null
 *  when nothing in it runs (the group keeps its summary sentence). */
export function liveSummary(entries: readonly WorkEntry[], cwd: string | null | undefined): string | null {
  const live = entries.filter((e) => isLive(e.item));
  if (live.length === 0) return null;
  const rest = entries.length - 1;
  return `${rowLabel(live[live.length - 1].item, cwd)}${rest > 0 ? ` +${rest} more` : ''}`;
}
