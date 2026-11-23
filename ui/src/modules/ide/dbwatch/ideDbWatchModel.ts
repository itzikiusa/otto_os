// Pure target-selection and value-formatting helpers for the IDE Live DB Watch
// & Component-Test Watcher panel. Kept rune-free so `node --test` unit tests
// can exercise target prioritization directly.

import type {
  IdeDbEngine,
  IdeDbWatchTarget,
  IdeEphemeralDbTarget,
  IdeSavedDbTarget,
} from '../../../lib/api/types/ide-db-watch';

export const IDE_DB_ENGINES: readonly { id: IdeDbEngine; label: string }[] = [
  { id: 'mysql', label: 'MySQL' },
  { id: 'postgres', label: 'PostgreSQL' },
  { id: 'mongodb', label: 'MongoDB' },
  { id: 'redis', label: 'Redis' },
  { id: 'clickhouse', label: 'ClickHouse' },
];

export function targetKeyOf(target: IdeDbWatchTarget): string {
  return target.target_kind === 'saved'
    ? `saved:${target.connection_id}`
    : `ephemeral:${target.ephemeral_id}`;
}

export function pickPreferredDbTarget(
  engine: IdeDbEngine,
  saved: readonly IdeSavedDbTarget[],
  ephemeral: readonly IdeEphemeralDbTarget[],
  currentKey?: string | null,
): IdeDbWatchTarget | null {
  const all: IdeDbWatchTarget[] = [...ephemeral, ...saved];
  if (currentKey) {
    const existingSameEngine = all.find(
      (t) => targetKeyOf(t) === currentKey && t.engine === engine,
    );
    if (existingSameEngine) return existingSameEngine;
  }
  // This run's Testcontainers / miniredis first, then the user's saved
  // connection; any other DB container may belong to another project.
  const liveMatch = ephemeral.find((t) => t.engine === engine && t.source !== 'container');
  if (liveMatch) return liveMatch;
  const savedMatch = saved.find((t) => t.engine === engine);
  if (savedMatch) return savedMatch;
  const containerMatch = ephemeral.find((t) => t.engine === engine);
  if (containerMatch) return containerMatch;
  if (currentKey) {
    const existingAny = all.find((t) => targetKeyOf(t) === currentKey);
    if (existingAny) return existingAny;
  }
  return ephemeral[0] ?? saved[0] ?? null;
}

export function formatDbCellValue(val: unknown): string {
  if (val === null) return 'NULL';
  if (val === undefined) return '—';
  if (typeof val === 'string') return val;
  if (typeof val === 'number' || typeof val === 'boolean') return String(val);
  try {
    return JSON.stringify(val);
  } catch {
    return String(val);
  }
}

/** True when the DB Watch panel is currently shown in either desktop dock or phone stacked layout. */
export function isDbWatchPanelVisible(opts: {
  stack: boolean;
  stackPane: string;
  bottomDockOpen: boolean;
  activeBottomTab: string;
}): boolean {
  if (opts.activeBottomTab !== 'dbwatch') return false;
  return opts.stack ? opts.stackPane === 'tools' : opts.bottomDockOpen;
}
