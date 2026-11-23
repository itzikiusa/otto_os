// The details card's "Subagents" section: everything working under a thread
// — its subagents (a turn's own, and threads an agent opened from this one)
// and the terminal threads it delegated to — with ONE set of status words
// (Working · Needs you · Done · Failed · Stopped), plus the thread it works
// under when it is itself a subagent. Background commands and timers are not
// subagents: they stay in Agents & tasks and the strip. This is the one
// adapter the card reads (built on runtime/agentTasks.ts); a richer helper
// replaces it with a single import. Pure (unit/threadsSubagents.test.ts).
import type { AgentTask, AgentTaskStatus } from './runtime/agentTasks';
import { perfCount } from '../../lib/perfCount.ts';

export type SubagentWord = 'Working' | 'Needs you' | 'Done' | 'Failed' | 'Stopped';

export interface SubagentRow {
  id: string;
  title: string;
  /** Provider slug for the glyph (null for a turn's own subagent). */
  provider: string | null;
  word: SubagentWord;
  startedAt: string | null;
  endedAt: string | null;
  /** The subagent's own thread, when it has one (opens it). */
  threadId: string | null;
  /** Its Agents & tasks row (per-row Stop), when it has one. */
  task: AgentTask | null;
}

export interface SubagentSection {
  /** The thread this one works under. */
  parent: { id: string; title: string } | null;
  running: SubagentRow[];
  /** Finished, newest first (folded under "Previous"). */
  previous: SubagentRow[];
}

/** A delegated terminal thread (or any thread known only by its status). */
export interface SubagentThread {
  id: string;
  title: string;
  provider: string;
  status: string;
}

const WORD: Record<AgentTaskStatus, SubagentWord> = {
  running: 'Working',
  'needs-you': 'Needs you',
  done: 'Done',
  failed: 'Failed',
  stopped: 'Stopped',
};

/** A thread's raw status (runtime or terminal) in the subagent words. */
export function subagentWord(status: string): SubagentWord {
  switch (status) {
    case 'working':
    case 'starting':
    case 'running':
      return 'Working';
    case 'awaiting_permission':
    case 'needs_input':
      return 'Needs you';
    case 'failed':
      return 'Failed';
    case 'closed':
    case 'exited':
      return 'Stopped';
    default:
      return 'Done';
  }
}

const isSubagent = (t: AgentTask): boolean => t.kind === 'agent' || t.childSessionId !== null;
const live = (w: SubagentWord): boolean => w === 'Working' || w === 'Needs you';

export function subagentSection(
  tasks: { running: readonly AgentTask[]; finished: readonly AgentTask[] },
  more: { parent?: { id: string; title: string } | null; delegated?: readonly SubagentThread[]; providerOf?: (threadId: string) => string | null },
): SubagentSection {
  perfCount('detailsSubagents');
  const row = (t: AgentTask): SubagentRow => ({
    id: t.id,
    title: t.label,
    provider: t.childSessionId ? (more.providerOf?.(t.childSessionId) ?? null) : null,
    word: WORD[t.status],
    startedAt: t.startedAt,
    endedAt: t.endedAt,
    threadId: t.childSessionId,
    task: t,
  });
  const seen = new Set<string>();
  const running: SubagentRow[] = [];
  const previous: SubagentRow[] = [];
  const add = (r: SubagentRow): void => {
    const key = r.threadId ?? r.id;
    if (seen.has(key)) return;
    seen.add(key);
    (live(r.word) ? running : previous).push(r);
  };
  for (const t of tasks.running) if (isSubagent(t)) add(row(t));
  for (const t of tasks.finished) if (isSubagent(t)) add(row(t));
  for (const d of more.delegated ?? []) {
    add({ id: d.id, title: d.title, provider: d.provider, word: subagentWord(d.status), startedAt: null, endedAt: null, threadId: d.id, task: null });
  }
  return { parent: more.parent ?? null, running, previous };
}
