// Pure helpers for the IDE DB Watch panel, its store and the Debug-panel strip:
// target choice (never a saved or prod connection nobody picked), the
// per-project memory, and the row projections that keep a 40-column table
// readable (a changed row shows its key and ONLY the changed cells; nested
// documents diff leaf by leaf). Rune-free so `node --test` can run it.

import type {
  IdeDbEngine,
  IdeDbFieldChange,
  IdeDbRowDiff,
  IdeDbWatch,
  IdeDbWatchFilter,
  IdeDbWatchItemDiff,
  IdeDbWatchItemSpec,
  IdeDbWatchSnapshotItem,
  IdeDbWatchTarget,
  IdeDbWatchTargetInfo,
  IdeDbWatchTargetRef,
  IdeDetectedDbAccess,
  IdeEphemeralDbTarget,
} from '../../../lib/api/types/ide-db-watch';
import { bsonScalar } from '../../../lib/jsonTree/bson.ts';
import { flattenPaths } from '../../../lib/jsonTree/expansion-plan.ts';
import { envTone } from '../../../lib/status.ts';

export const IDE_DB_ENGINES: readonly { id: IdeDbEngine; label: string }[] = [
  { id: 'mysql', label: 'MySQL' },
  { id: 'postgres', label: 'PostgreSQL' },
  { id: 'mongodb', label: 'MongoDB' },
  { id: 'redis', label: 'Redis' },
  { id: 'clickhouse', label: 'ClickHouse' },
];

/** Daemon cap per watch. */
export const MAX_WATCH_ITEMS = 8;
/** Tables auto-picked on a remote target (local targets get MAX_WATCH_ITEMS). */
export const REMOTE_AUTO_ITEMS = 3;
/** Diff rows a table section shows before "N more · Show". */
export const ROWS_SHOWN = 20;

export function engineLabel(engine: IdeDbEngine): string {
  return IDE_DB_ENGINES.find((e) => e.id === engine)?.label ?? engine;
}

export function targetKeyOf(target: IdeDbWatchTarget): string {
  return target.target_kind === 'saved' ? `saved:${target.connection_id}` : `ephemeral:${target.ephemeral_id}`;
}

export function targetRefOf(target: IdeDbWatchTarget): IdeDbWatchTargetRef {
  return target.target_kind === 'saved'
    ? { kind: 'saved', connection_id: target.connection_id }
    : { kind: 'ephemeral', ephemeral_id: target.ephemeral_id };
}

/** A saved target tagged prod (ephemeral targets carry no environment). */
export function isProdTarget(t: object | null | undefined): boolean {
  const env = t && 'environment' in t ? (t.environment as string | null | undefined) : null;
  return envTone(env).key === 'prod';
}

/** Where a watched target lives, for the Target ▾ tooltip. */
export function targetPlace(t: IdeDbWatchTargetInfo): string {
  if (t.kind === 'ephemeral') return 'This run';
  return t.local ? 'Saved · local' : 'Saved · remote';
}

// ── Per-project memory ({target hint, tables}; no secrets) ─────────────────

/** A saved connection the person picked for this project, or "the run's own
 *  container of this engine". Never a prod target, never an ephemeral id (it
 *  is gone with the run). */
export type DbWatchTargetHint = { kind: 'saved'; connection_id: string } | { kind: 'auto'; engine: IdeDbEngine };

export interface DbWatchMemory {
  target: DbWatchTargetHint;
  database?: string | null;
  items: IdeDbWatchItemSpec[];
}

export const DB_WATCH_MEMORY_PREFIX = 'otto_ide_dbwatch:';

export function memoryKey(root: string): string {
  return DB_WATCH_MEMORY_PREFIX + root;
}

/** What to remember of a watch the person set up; null for a prod target. */
export function memoryOf(watch: IdeDbWatch): DbWatchMemory | null {
  const t = watch.target;
  if (isProdTarget(t)) return null;
  const target: DbWatchTargetHint =
    t.kind === 'saved' && t.connection_id ? { kind: 'saved', connection_id: t.connection_id } : { kind: 'auto', engine: t.engine };
  return {
    target,
    database: watch.database ?? null,
    items: watch.items.map((i) => (i.filter ? { object: i.object, filter: i.filter } : { object: i.object })),
  };
}

/** Written by "Stop watching": this project does not re-arm on its own. */
export const DB_WATCH_OFF = '{"off":true}';

export function parseMemory(raw: string | null): DbWatchMemory | null {
  if (!raw) return null;
  try {
    const m = JSON.parse(raw) as DbWatchMemory;
    const t = m?.target;
    const okTarget = (t?.kind === 'saved' && typeof t.connection_id === 'string') || (t?.kind === 'auto' && typeof t.engine === 'string');
    if (!okTarget || !Array.isArray(m.items)) return null;
    const items = m.items.filter((i) => typeof i?.object === 'string' && i.object).slice(0, MAX_WATCH_ITEMS);
    return items.length > 0 ? { target: t, database: m.database ?? null, items } : null;
  } catch {
    return null;
  }
}

// ── Automatic choice (store-level auto-arm) ─────────────────────────────────

/** The run's own database: a Testcontainers / local-process target of the
 *  best-ranked engine in code. Never a saved connection, never another
 *  project's plain container. */
export function pickAutoTarget(
  detected: readonly IdeDetectedDbAccess[],
  ephemeral: readonly IdeEphemeralDbTarget[],
): IdeEphemeralDbTarget | null {
  const own = ephemeral.filter((t) => t.source !== 'container');
  if (own.length === 0) return null;
  for (const d of detected) {
    const hit = own.find((t) => t.engine === d.engine);
    if (hit) return hit;
  }
  return own[0];
}

const SQL_ENGINES: ReadonlySet<IdeDbEngine> = new Set(['mysql', 'postgres', 'clickhouse']);

/** Detection names any SQL string `mysql`; on a SQL target the target's own
 *  verification (`verified` set) decides, whatever SQL engine it is. */
export function matchesEngine(d: IdeDetectedDbAccess, engine: IdeDbEngine): boolean {
  return d.engine === engine || (SQL_ENGINES.has(d.engine) && SQL_ENGINES.has(engine) && d.verified != null);
}

/** Ranked objects of one engine that exist on the target (`verified` false =
 *  a name in code the target does not have), deduplicated, capped. */
export function pickAutoItems(detected: readonly IdeDetectedDbAccess[], engine: IdeDbEngine, local: boolean): IdeDbWatchItemSpec[] {
  const seen = new Set<string>();
  const out: IdeDbWatchItemSpec[] = [];
  for (const d of detected) {
    if (!matchesEngine(d, engine) || d.verified === false || seen.has(d.object_name)) continue;
    seen.add(d.object_name);
    out.push({ object: d.object_name });
  }
  return out.slice(0, local ? MAX_WATCH_ITEMS : REMOTE_AUTO_ITEMS);
}

/** Names in code the target does not have (the Tables ▾ footer). */
export function unverifiedNames(detected: readonly IdeDetectedDbAccess[], engine: IdeDbEngine): string[] {
  return [...new Set(detected.filter((d) => matchesEngine(d, engine) && d.verified === false).map((d) => d.object_name))];
}

// ── Filters: `column = value` only (no free text in v1) ──────────────────

/** The daemon writes the value into the statement itself, so text is limited
 *  to this charset (anything else is a 400 there). */
const FILTER_TEXT = /^[A-Za-z0-9_.:@-]{1,128}$/;

/** "status = open", "id = 42", "deleted_at = NULL", "code = '007'". */
export function parseFilter(text: string): IdeDbWatchFilter | null {
  const m = /^\s*([A-Za-z_][\w.]*)\s*=\s*(.+?)\s*$/.exec(text);
  if (!m) return null;
  const raw = m[2];
  const quoted = /^(['"])(.*)\1$/.exec(raw);
  let value: IdeDbWatchFilter['value'];
  if (quoted) value = quoted[2];
  else if (/^null$/i.test(raw)) value = null;
  else if (/^(true|false)$/i.test(raw)) value = raw.toLowerCase() === 'true';
  else if (/^-?\d+(\.\d+)?$/.test(raw) && raw.length < 16) value = Number(raw);
  else value = raw;
  if (typeof value === 'string' && !FILTER_TEXT.test(value)) return null;
  return { column: m[1], value };
}

export function filterText(f: IdeDbWatchFilter | null | undefined): string {
  if (!f) return '';
  const v = f.value === null ? 'NULL' : typeof f.value === 'string' ? `'${f.value}'` : String(f.value);
  return `${f.column} = ${v}`;
}

// ── Values and rows ─────────────────────────────────────────────────────────

/** One cell as text: NULL, a BSON scalar as ObjectId("…") / ISODate("…"),
 *  JSON for a nested value. */
export function formatDbCellValue(val: unknown): string {
  if (val === null) return 'NULL';
  if (val === undefined) return '—';
  if (typeof val === 'string') return val;
  if (typeof val === 'number' || typeof val === 'boolean') return String(val);
  const b = bsonScalar(val);
  if (b !== null) return b;
  try {
    return JSON.stringify(val);
  } catch {
    return String(val);
  }
}

export function isNested(v: unknown): boolean {
  return v !== null && typeof v === 'object' && bsonScalar(v) === null;
}

export interface CellChange {
  /** Column, or column.leaf.path for a nested document. */
  path: string;
  before: string;
  after: string;
  /** "changed · hidden": the values are masked. */
  hidden: boolean;
}

function leafText(v: unknown, present: boolean): string {
  return present ? formatDbCellValue(v) : '—';
}

/** A modified row's changed cells, nested documents expanded to the leaves
 *  that changed (`context.geo.city: Valletta → Sliema`), so a Mongo update reads
 *  like a SQL one. */
export function cellChanges(changes: readonly IdeDbFieldChange[]): CellChange[] {
  const out: CellChange[] = [];
  for (const c of changes) {
    if (c.masked) {
      out.push({ path: c.column, before: '', after: '', hidden: true });
      continue;
    }
    if (!isNested(c.before) && !isNested(c.after)) {
      out.push({ path: c.column, before: formatDbCellValue(c.before), after: formatDbCellValue(c.after), hidden: false });
      continue;
    }
    const L = flattenPaths(c.before, c.column);
    const R = flattenPaths(c.after, c.column);
    for (const [p, l] of L) {
      const has = R.has(p);
      const r = R.get(p);
      if (!has || formatDbCellValue(l) !== formatDbCellValue(r) || typeof l !== typeof r) {
        out.push({ path: p, before: leafText(l, true), after: leafText(r, has), hidden: false });
      }
    }
    for (const [p, r] of R) if (!L.has(p)) out.push({ path: p, before: '—', after: formatDbCellValue(r), hidden: false });
  }
  return out;
}

/** "Show current rows" as plain rows keyed by the WHOLE primary key (a
 *  composite one too); no key column → the row number. */
export function currentAsDiffs(current: IdeDbWatchSnapshotItem, primaryKey: readonly string[]): IdeDbRowDiff[] {
  const idx = primaryKey.map((c) => current.columns.indexOf(c)).filter((i) => i >= 0);
  return current.rows.map((r, i) => ({
    key: idx.length > 0 ? idx.map((j) => formatDbCellValue(r[j])).join(', ') : String(i + 1),
    kind: 'added',
    row: r,
    changes: [],
  }));
}

/** Unique {#each} keys: `kind:key`, a repeat gets `#n` (rows of a table with
 *  no primary key can be identical). */
export function rowKeys(list: readonly Pick<IdeDbRowDiff, 'kind' | 'key'>[]): string[] {
  const seen = new Map<string, number>();
  return list.map((d) => {
    const k = `${d.kind}:${d.key}`;
    const n = seen.get(k) ?? 0;
    seen.set(k, n + 1);
    return n === 0 ? k : `${k}#${n}`;
  });
}

/** An added / removed row in one line: its first non-empty cells after the
 *  key, `col=value`, short. The full record is one click away. */
export function rowPreview(columns: readonly string[], row: readonly unknown[], max = 6): string {
  const parts: string[] = [];
  for (let i = 0; i < columns.length && parts.length < max; i++) {
    const v = row[i];
    if (v === null || v === undefined || v === '') continue;
    const s = formatDbCellValue(v);
    parts.push(`${columns[i]}=${s.length > 40 ? `${s.slice(0, 39)}…` : s}`);
  }
  return parts.join('  ');
}

export interface RecordField {
  path: string;
  value: string;
  /** The modified row's previous value of this field (changed fields only). */
  before?: string;
  changed: boolean;
  isNull: boolean;
}

/** The vertical record an expanded row shows: one field per line, nested
 *  documents as dotted leaves, a modified row's changed fields first. */
export function recordFields(columns: readonly string[], d: IdeDbRowDiff): RecordField[] {
  const row = d.kind === 'removed' ? (d.previous_row ?? d.row) : d.row;
  const fields: RecordField[] = [];
  const changed = new Map(d.kind === 'modified' ? cellChanges(d.changes).map((c) => [c.path, c] as const) : []);
  columns.forEach((col, i) => {
    const leaves = isNested(row[i]) ? flattenPaths(row[i], col) : new Map([[col, row[i]]]);
    for (const [path, v] of leaves) {
      const c = changed.get(path);
      fields.push({
        path,
        value: c?.hidden ? 'changed · hidden' : formatDbCellValue(v),
        before: c && !c.hidden ? c.before : undefined,
        changed: Boolean(c),
        isNull: v === null || v === undefined,
      });
    }
  });
  return [...fields.filter((f) => f.changed), ...fields.filter((f) => !f.changed)];
}

/** "Copy changes as text": one block per table, a line per changed row. */
export function changesAsText(items: readonly IdeDbWatchItemDiff[]): string {
  const out: string[] = [];
  for (const it of items) {
    out.push(`${it.object}: ${countsShort(it) || 'no changes'}${it.error ? ` (error: ${it.error})` : ''}`);
    for (const d of it.diffs) {
      if (d.kind === 'modified') {
        const cells = cellChanges(d.changes).map((c) => (c.hidden ? `${c.path}: changed · hidden` : `${c.path}: ${c.before} → ${c.after}`));
        out.push(`  ~ ${d.key}  ${cells.join('; ')}`);
      } else {
        const row = d.kind === 'removed' ? (d.previous_row ?? d.row) : d.row;
        out.push(`  ${d.kind === 'added' ? '+' : '−'} ${d.key}  ${d.masked ? 'hidden' : rowPreview(it.columns, row, 40)}`);
      }
    }
    if (it.more_count > 0) out.push(`  … ${it.more_count} more`);
  }
  return out.join('\n');
}

// ── Counts (badge, strip, headers) ──────────────────────────────────────────

export function changedRowsOf(item: Pick<IdeDbWatchItemDiff, 'added_count' | 'modified_count' | 'removed_count'>): number {
  return item.added_count + item.modified_count + item.removed_count;
}

/** "+1 ~2 −1" for the strip; '' when nothing changed. */
export function countsShort(item: Pick<IdeDbWatchItemDiff, 'added_count' | 'modified_count' | 'removed_count'>): string {
  const parts: string[] = [];
  if (item.added_count) parts.push(`+${item.added_count}`);
  if (item.modified_count) parts.push(`~${item.modified_count}`);
  if (item.removed_count) parts.push(`−${item.removed_count}`);
  return parts.join(' ');
}

/** "Since last stop | Since start" only differ once a stop follows the first one. */
export function canCompareStart(watch: Pick<IdeDbWatch, 'checkpoint_count'> | null | undefined): boolean {
  return (watch?.checkpoint_count ?? 0) >= 3;
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
