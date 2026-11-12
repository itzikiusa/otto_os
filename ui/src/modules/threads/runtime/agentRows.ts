// One compact row shape for "another agent" in the work log: a subagent
// card's head and the agent's own `otto.thread_open` / `otto.thread_message`
// calls (T3 Code's ProviderSubagentBar / V2LifecycleRow): provider · model ·
// effort · status · elapsed · one-line result · Open. Pure helpers
// (unit/runtimeWorkLog.test.ts).
import type { TurnItem } from '../../../lib/api/types';
import { openedTarget } from './timeline';

export interface ThreadCall {
  verb: 'Opened' | 'Messaged';
  /** The thread it opened or spoke to (null until the call returned an id). */
  targetId: string | null;
  provider: string | null;
  title: string | null;
  model: string | null;
  effort: string | null;
  /** Its opening prompt / the message, first line. */
  text: string | null;
}

const firstLine = (v: unknown): string | null => {
  if (typeof v !== 'string') return null;
  const line = v.trim().split('\n').find((l) => l.trim() !== '');
  return line ? line.trim() : null;
};
const str = (v: unknown): string | null => (typeof v === 'string' && v.trim() ? v.trim() : null);

/** A thread_open / thread_message call, read for its row; null for any other item. */
export function threadCall(item: TurnItem): ThreadCall | null {
  if (item.type !== 'dynamic_tool') return null;
  const tool = item.tool_name.replace(/^.*otto[._]/, '');
  if (tool !== 'thread_open' && tool !== 'thread_message') return null;
  const input = (item.input && typeof item.input === 'object' ? item.input : {}) as Record<string, unknown>;
  const opened = tool === 'thread_open';
  return {
    verb: opened ? 'Opened' : 'Messaged',
    targetId: openedTarget(item)?.id ?? null,
    provider: opened ? str(input.provider) : null,
    title: opened ? str(input.title) : null,
    model: opened ? str(input.model) : null,
    effort: opened ? str(input.effort) : null,
    text: firstLine(opened ? input.prompt : input.text),
  };
}

/** "12s", "2m 5s" from a start to an end (or now). */
export function elapsedBetween(startIso: string | null, endMs: number): string | null {
  if (!startIso) return null;
  const s = Math.max(0, Math.floor((endMs - Date.parse(startIso)) / 1000));
  if (!Number.isFinite(s)) return null;
  return s < 60 ? `${s}s` : `${Math.floor(s / 60)}m ${s % 60}s`;
}
