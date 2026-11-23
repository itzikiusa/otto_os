// The Threads sidebar as an inbox (T3 Code's thread inbox, re-done in
// Otto's idiom): what each row says in one word, which threads leave the
// main list for the "Working" fold, and the order the main list keeps —
// needs you (Approval, Question), then Failed, then finished-unread (Done),
// then the time the thread last came back to you, newest first. A thread's
// family (a lead and the threads opened from it) always moves together.
// Pure (unit/threadsInbox.test.ts); states come in from lib/status.ts.
import type { Session } from '../../lib/api/types';
import { inboxOrder, type FamilyRow } from './threadFamilies';

/** Where a row belongs, from its derived state key and unread flag. */
export type InboxClass = 'needs' | 'failed' | 'done' | 'working' | 'rest';

export function inboxClass(key: string | null | undefined, unread: boolean): InboxClass {
  if (key === 'needs-you') return 'needs';
  if (key === 'failed') return 'failed';
  if (key === 'working' || key === 'running' || key === 'stale') return 'working';
  return unread ? 'done' : 'rest';
}

const RANK: Record<InboxClass, number> = { needs: 0, failed: 1, done: 2, working: 3, rest: 3 };

/** The main list's sort key (lower first; ties go to the most recent). */
export function inboxScore(cls: InboxClass): number {
  return RANK[cls];
}

/** The main list and the Working fold, each in inbox order with families
 *  nested (threadFamilies.ts). A family goes to Working when one of its
 *  members works and none needs you, failed or finished unread. */
export function inboxSections(
  list: readonly Session[],
  classOf: (s: Session) => InboxClass,
  parentOf: (s: Session) => string | null,
  folded: (id: string) => boolean = () => false,
  /** When the thread last came back to the person (ISO, newest first). */
  at?: (s: Session) => string,
): { main: FamilyRow[]; working: FamilyRow[] } {
  const byId = new Map(list.map((s) => [s.id, s] as const));
  const rootOf = (s: Session): string => {
    let cur = s;
    const seen = new Set<string>();
    for (;;) {
      const p = parentOf(cur);
      const next = p && p !== cur.id && !seen.has(p) ? byId.get(p) : undefined;
      if (!next) return cur.id;
      seen.add(cur.id);
      cur = next;
    }
  };
  const fam = new Map<string, InboxClass[]>();
  for (const s of list) {
    const r = rootOf(s);
    fam.set(r, [...(fam.get(r) ?? []), classOf(s)]);
  }
  const working = (root: string): boolean => {
    const cls = fam.get(root) ?? [];
    return cls.includes('working') && cls.every((c) => c === 'working' || c === 'rest');
  };
  const score = (s: Session): number => RANK[classOf(s)];
  const mainList: Session[] = [];
  const workList: Session[] = [];
  for (const s of list) (working(rootOf(s)) ? workList : mainList).push(s);
  return {
    main: inboxOrder(mainList, score, parentOf, folded, at),
    working: inboxOrder(workList, () => 0, parentOf, folded, at),
  };
}

/** The pending requests' kinds → the word a waiting row shows: Question when
 *  every one is a question (or an MCP form), else Approval. */
export function waitingWord(kinds: readonly string[]): 'Approval' | 'Question' {
  return kinds.length > 0 && kinds.every((k) => k === 'question' || k === 'mcp_elicitation') ? 'Question' : 'Approval';
}

/** The status word of a subagent row (a thread working under another one):
 *  exactly Working · Needs you · Done · Failed · Stopped. */
export function subagentWord(key: string | null | undefined): 'Working' | 'Needs you' | 'Done' | 'Failed' | 'Stopped' {
  if (key === 'needs-you') return 'Needs you';
  if (key === 'working' || key === 'running' || key === 'stale') return 'Working';
  if (key === 'failed') return 'Failed';
  if (key === 'stopped') return 'Stopped';
  return 'Done';
}

/** Finished unread ("Done") across reloads and devices: the last turn ended
 *  after the person last had the thread in front of them (`meta.thread_seen_at`).
 *  A thread never seen on any device counts as read (no flood of "Done"). */
export function unseen(endedIso: string | null | undefined, seenIso: unknown): boolean {
  if (!endedIso || typeof seenIso !== 'string') return false;
  const ended = Date.parse(endedIso);
  const seen = Date.parse(seenIso);
  return Number.isFinite(ended) && Number.isFinite(seen) && ended > seen;
}

/** The meta writes that move this browser's old local pins / History onto
 *  the threads themselves — only for threads that still exist. */
export function migrationPatches(
  pins: readonly string[],
  history: readonly string[],
  exists: (id: string) => boolean,
): [string, { thread_pinned?: true; thread_history?: true }][] {
  const out = new Map<string, { thread_pinned?: true; thread_history?: true }>();
  for (const id of pins) if (exists(id)) out.set(id, { ...out.get(id), thread_pinned: true });
  for (const id of history) if (exists(id)) out.set(id, { ...out.get(id), thread_history: true });
  return [...out];
}

/** "18s", "4m", "1h 5m" for a positive span. */
export function elapsed(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  const m = Math.floor((s % 3600) / 60);
  return `${Math.floor(s / 3600)}h${m ? ` ${m}m` : ''}`;
}

/** The one status word a row shows, or null (it shows the relative time).
 *  The vocabulary is fixed: Approval · Question · Failed · Done · Working. */
export function statusWord(
  cls: InboxClass,
  opts: { kinds?: readonly string[]; workingSince?: number | null; now?: number } = {},
): string | null {
  switch (cls) {
    case 'needs':
      return waitingWord(opts.kinds ?? []);
    case 'failed':
      return 'Failed';
    case 'done':
      return 'Done';
    case 'working':
      return opts.workingSince && opts.now ? `Working ${elapsed(opts.now - opts.workingSince)}` : 'Working';
    default:
      return null;
  }
}
