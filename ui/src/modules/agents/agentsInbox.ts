import type { Session } from '../../lib/api/types';
import type { InboxClass } from '../../lib/statusWords';

export const SESSION_GROUPING_LABELS = { workspace: 'Workspace', status: 'Status', none: 'None' } as const;
export type SessionGrouping = keyof typeof SESSION_GROUPING_LABELS;

export const SESSION_LIST_STATUS_LABELS = { needs: 'Needs you', working: 'Working', idle: 'Idle', done: 'Done' } as const;
export type SessionListStatus = keyof typeof SESSION_LIST_STATUS_LABELS;
const STATUS_ORDER = Object.keys(SESSION_LIST_STATUS_LABELS) as SessionListStatus[];
type StatusBuckets = Record<SessionListStatus, Session[]>;

export type InboxGroup =
  | { kind: 'workspace'; key: string; sessions: readonly Session[] }
  | { kind: 'status'; key: SessionListStatus; sessions: readonly Session[] }
  | { kind: 'flat'; key: 'flat'; sessions: readonly Session[] };

/** Keep the row's specific outcome word (e.g. Failed) in the Done group. */
export function sessionListStatus(key: string, cls: InboxClass): SessionListStatus {
  if (cls === 'needs' || cls === 'working') return cls;
  if (cls === 'done' || cls === 'failed' || key === 'stopped' || key === 'ended') return 'done';
  return 'idle';
}

/** Compare activity timestamps once per row. Break ties by id so a refresh
 *  that brings the same rows in a different order doesn't move them. */
export function byRecent<T extends Pick<Session, 'id' | 'last_active_at'>>(list: readonly T[]): T[] {
  return list
    .map((row) => {
      // Date.parse truncates the daemon's fractional seconds to milliseconds.
      const fraction = /\.\d{3}(\d{1,6})/.exec(row.last_active_at)?.[1];
      return { row, at: Date.parse(row.last_active_at) || 0, nanos: Number(fraction?.padEnd(6, '0') ?? 0) };
    })
    .sort((a, b) => b.at - a.at || b.nanos - a.nanos || a.row.id.localeCompare(b.row.id))
    .map(({ row }) => row);
}

function statusBuckets(): StatusBuckets {
  return { needs: [], working: [], idle: [], done: [] };
}

/** One recency sort followed by a stable bucket pass. In Workspace mode,
 *  map insertion order records each workspace's newest session, even when
 *  that session sorts below older ones inside the workspace. */
export function groupAgentSessions(
  list: readonly Session[],
  statusOf: (s: Session) => SessionListStatus,
  groupBy: SessionGrouping | 'single',
  currentWorkspaceId: string | null,
  keepInputOrder = false,
): InboxGroup[] {
  if (list.length === 0) return [];
  const rows = groupBy === 'single' && keepInputOrder ? list : byRecent(list);
  if (groupBy === 'none') return [{ kind: 'flat', key: 'flat', sessions: rows }];

  if (groupBy === 'workspace') {
    const workspaces = new Map<string, StatusBuckets>();
    for (const row of rows) {
      let buckets = workspaces.get(row.workspace_id);
      if (!buckets) {
        buckets = statusBuckets();
        workspaces.set(row.workspace_id, buckets);
      }
      buckets[statusOf(row)].push(row);
    }
    const groups: InboxGroup[] = [];
    for (const [key, buckets] of workspaces) {
      const group = { kind: 'workspace' as const, key, sessions: STATUS_ORDER.flatMap((status) => buckets[status]) };
      if (key === currentWorkspaceId) groups.unshift(group);
      else groups.push(group);
    }
    return groups;
  }

  const buckets = statusBuckets();
  for (const row of rows) buckets[statusOf(row)].push(row);
  if (groupBy === 'single') return [{ kind: 'flat', key: 'flat', sessions: STATUS_ORDER.flatMap((status) => buckets[status]) }];
  return STATUS_ORDER.filter((key) => buckets[key].length > 0).map((key) => ({ kind: 'status', key, sessions: buckets[key] }));
}

/** Search: title, provider, folder, branch or workspace name. */
export function matchesQuery(s: Session, q: string, extra: readonly (string | null | undefined)[] = []): boolean {
  const n = q.trim().toLowerCase();
  if (!n) return true;
  return [s.title, s.provider, s.cwd, ...extra].some((v) => (v ?? '').toLowerCase().includes(n));
}
