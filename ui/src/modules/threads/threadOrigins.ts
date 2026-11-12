// Where a thread came from, for the Threads sidebar: the person's own threads
// (and a personal agent's CHAT, grouped under the agent's name) fill the
// projects and History; a personal agent's routine RUNS fold into one
// "Agent runs" group (a 15-minute routine starts ~96 a day); threads another
// module started for itself (a pull request's agent, a connection's
// diagnosis, a DB assist…) fold into their own group. Pure — unit-tested in
// unit/threadsOrigins.test.ts.
import type { Session } from '../../lib/api/types';
import { BACKGROUND_SOURCES } from '../../lib/stores/sessionBuckets';

/** `meta.source` of threads embedded in another module (lib/runtimeThreadPort.ts):
 *  they belong to that module, not the Threads list. Together with
 *  BACKGROUND_SOURCES (background-spawned helpers the sidebars never list
 *  flat — `db_assist`, `review`, …) they are the "helper" threads. */
export const EMBEDDED_THREAD_SOURCES: ReadonlySet<string> = new Set(['pr-agent', 'connection-diagnose']);

/** The group title of a helper source. */
const HELPER_LABELS: Readonly<Record<string, string>> = {
  'pr-agent': 'Pull requests',
  review: 'Pull requests',
  'pr-draft': 'Pull requests',
  'connection-diagnose': 'Connections',
  db_assist: 'Connections',
};

export type ThreadOrigin = 'main' | 'run' | 'helper';

const sourceOf = (s: Session): string | null => {
  const v = s.meta?.source;
  return typeof v === 'string' && v !== '' ? v : null;
};

export function threadOrigin(s: Session): ThreadOrigin {
  const src = sourceOf(s);
  if (src && (EMBEDDED_THREAD_SOURCES.has(src) || BACKGROUND_SOURCES.has(src))) return 'helper';
  // A personal agent's thread that is not its chat: a routine / manual /
  // proactive run, or work another agent delegated to it.
  if (src === 'personal_agent' && s.meta?.personal_agent_chat !== true) return 'run';
  return 'main';
}

export interface OriginGroup {
  key: string;
  label: string;
  /** Newest activity first. */
  items: Session[];
}

const byRecent = (a: Session, b: Session): number => b.last_active_at.localeCompare(a.last_active_at);

/** The folded groups for the non-main threads of `list`: "Agent runs" first,
 *  then one group per helper label. Empty groups are left out. */
export function originGroups(list: readonly Session[]): OriginGroup[] {
  const runs: Session[] = [];
  const helpers = new Map<string, Session[]>();
  const seen = new Set<string>();
  for (const s of list) {
    if (seen.has(s.id)) continue;
    seen.add(s.id);
    const o = threadOrigin(s);
    if (o === 'run') runs.push(s);
    else if (o === 'helper') {
      const label = HELPER_LABELS[sourceOf(s) ?? ''] ?? 'Helpers';
      helpers.set(label, [...(helpers.get(label) ?? []), s]);
    }
  }
  const out: OriginGroup[] = runs.length ? [{ key: 'agent-runs', label: 'Agent runs', items: runs.sort(byRecent) }] : [];
  for (const [label, items] of helpers) out.push({ key: `helpers-${label.toLowerCase().replace(/\s+/g, '-')}`, label, items: items.sort(byRecent) });
  return out;
}
