// Redis edit adapter: a result produced by ONE read command (`GET` / `HGET` /
// `HGETALL` / `LRANGE` / `SMEMBERS` / `ZRANGE`) is editable through the same
// review flow as SQL/Mongo — each change becomes one Redis command per line
// (`HSET k f "v"`, `LSET k 3 "v"`, `ZADD k 1.5 "m"`, `HDEL k f`, …) that the
// driver runs line by line. The Redis key is the row identity: the driver
// returns no column for it, so `EditTarget.db` carries the SOURCE COMMAND LINE
// (the adapter has no other channel back to the statement) and `table` the key.
//
// Result shapes (the driver speaks RESP2): every reply is a single `value`
// column — `GET`/`HGET` one row; `LRANGE`/`SMEMBERS`/`ZRANGE` one row per
// element; `HGETALL` and `ZRANGE … WITHSCORES` a FLAT alternating list
// (field, value, field, value, …). A RESP3 `HGETALL` map arrives as `key` /
// `value` columns — handled as well. Values are always emitted double-quoted
// (the driver's `split_args` rules: `\"` and `\\` escapes inside quotes) so an
// edit reads unmistakably in the review; keys / fields are quoted only when
// they need it.
import { cellStr, SET_EMPTY, SET_NULL } from './results-format';
import type { DiffLine, EditAdapter, EditCtx, RowPatch } from './edit-types';

export type RedisReadCmd = 'GET' | 'HGETALL' | 'HGET' | 'LRANGE' | 'SMEMBERS' | 'ZRANGE';

export interface RedisTarget {
  cmd: RedisReadCmd;
  key: string;
  /** `HGET key field`. */
  field?: string;
  /** `ZRANGE … WITHSCORES` — rows alternate member / score. */
  withScores?: boolean;
  /** `LRANGE key start stop` — row i is list index `start + i` (editable only
   *  when non-negative: a negative start can't be mapped without `LLEN`). */
  start?: number;
}

/** How the rows of a recognised command are laid out (see the header). */
type Layout =
  | 'single' // GET / HGET: one `value` row
  | 'kv' // HGETALL (RESP3): `key` + `value` columns
  | 'pairs' // HGETALL / ZRANGE WITHSCORES (RESP2): flat field,value,… rows
  | 'list' // LRANGE: one element per row, index = start + row
  | 'members'; // SMEMBERS / ZRANGE: one member per row — delete only

/** Placeholder a list delete writes over the element before `LREM`ing it —
 *  Redis has no delete-by-index. */
const LIST_TOMBSTONE = '__otto_deleted__';

/** JS port of the driver's `split_args` (`redis.rs`): whitespace-separated
 *  tokens, a double-quoted run keeps its spaces, and `\"` / `\\` are the only
 *  escapes (inside quotes). Mirrors the Rust byte for byte so a statement the
 *  adapter parses here is the one the daemon runs. */
export function splitArgs(line: string): string[] {
  const args: string[] = [];
  let cur = '';
  let inQuotes = false;
  let hasToken = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === '"') {
      inQuotes = !inQuotes;
      hasToken = true;
    } else if (c === '\\' && inQuotes) {
      const n = line[i + 1];
      if (n === '"' || n === '\\') {
        cur += n;
        i++;
      } else {
        cur += '\\';
      }
    } else if (/\s/.test(c) && !inQuotes) {
      if (hasToken) {
        args.push(cur);
        cur = '';
        hasToken = false;
      }
    } else {
      cur += c;
      hasToken = true;
    }
  }
  if (hasToken) args.push(cur);
  return args;
}

/** A key / field / member token: bare when the splitter would read it back
 *  unchanged (no whitespace, quotes or backslashes, non-empty), else quoted. */
export function redisQuote(s: string): string {
  return s !== '' && !/[\s"\\]/.test(s) ? s : redisString(s);
}
/** A value token: ALWAYS `"…"` with `\` and `"` escaped. */
export function redisString(s: string): string {
  return `"${s.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}

/** Parse a statement into the read command its rows came from. Null unless the
 *  statement is exactly ONE non-comment line naming a supported command with
 *  the argument count that command takes. */
export function redisTarget(statement: string): RedisTarget | null {
  const lines = statement
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith('#'));
  if (lines.length !== 1) return null;
  const args = splitArgs(lines[0]);
  const cmd = (args[0] ?? '').toUpperCase();
  const key = args[1];
  if (!key) return null;
  switch (cmd) {
    case 'GET':
    case 'HGETALL':
    case 'SMEMBERS':
      return args.length === 2 ? { cmd, key } : null;
    case 'HGET':
      return args.length === 3 ? { cmd, key, field: args[2] } : null;
    case 'LRANGE': {
      if (args.length !== 4 || !/^-?\d+$/.test(args[2])) return null;
      return { cmd, key, start: Number(args[2]) };
    }
    case 'ZRANGE': {
      if (args.length < 4) return null;
      const withScores = args.slice(4).some((a) => a.toUpperCase() === 'WITHSCORES');
      return { cmd, key, withScores };
    }
    default:
      return null;
  }
}

/** The layout the result columns imply for the command, or null when the
 *  shape isn't the one the command produces (a re-run of a different
 *  statement into the same tab, a `(nil)` message, …). */
function layoutOf(t: RedisTarget, columns: string[]): Layout | null {
  const single = columns.length === 1 && columns[0] === 'value';
  switch (t.cmd) {
    case 'GET':
    case 'HGET':
      return single ? 'single' : null;
    case 'HGETALL':
      if (columns.length === 2 && columns[0] === 'key' && columns[1] === 'value') return 'kv';
      return single ? 'pairs' : null;
    case 'LRANGE':
      return single ? 'list' : null;
    case 'SMEMBERS':
      return single ? 'members' : null;
    case 'ZRANGE':
      return single ? (t.withScores ? 'pairs' : 'members') : null;
  }
}

/** Recover the parsed command + layout from an adapter ctx (`target.db` holds
 *  the source line — see the header). */
function resolve(ctx: EditCtx): { t: RedisTarget; layout: Layout } | null {
  const t = ctx.target.db ? redisTarget(ctx.target.db) : null;
  if (!t) return null;
  const layout = layoutOf(t, ctx.columns.map((c) => c.name));
  return layout ? { t, layout } : null;
}

/** Text of a cell (`null` → empty — Redis has no null). */
function text(v: unknown): string {
  return v === null || v === undefined ? '' : cellStr(v);
}
/** The draft typed for a cell: the Set-NULL / Set-empty sentinels both mean
 *  the empty string here. */
function draft(raw: string): string {
  return raw === SET_NULL || raw === SET_EMPTY ? '' : raw;
}
/** `ZADD` needs a numeric score (`inf` / `-inf` allowed). */
function isScore(s: string): boolean {
  return /^[+-]?(\d+(\.\d+)?([eE][+-]?\d+)?|inf)$/i.test(s.trim());
}
/** A `#` line the driver skips — surfaces an unrepresentable edit in the
 *  review instead of silently dropping it. */
function note(rowIdx: number, why: string): string {
  return `# row ${rowIdx + 1}: ${why} — skipped`;
}

/** The commands that apply ONE edited cell (`colIdx`, new text `after`) of a
 *  row, plus its diff line. `verbs` collects the command names for the title. */
function cellCommands(
  r: { t: RedisTarget; layout: Layout },
  ctx: EditCtx,
  rowIdx: number,
  colIdx: number,
  after: string,
  verbs: Set<string>,
): { stmts: string[]; diff: DiffLine } {
  const { t, layout } = r;
  const key = redisQuote(t.key);
  const rows = ctx.liveRows;
  const before = text(rows[rowIdx]?.[colIdx]);
  const line = (path: string, op: DiffLine['op'] = 'cell'): DiffLine => ({ row: rowIdx, path, op, before, after });
  if (after.includes('\n')) {
    return { stmts: [note(rowIdx, 'the value contains a line break, which a one-line Redis command cannot carry')], diff: line('value') };
  }
  const add = (s: string): string => {
    verbs.add(s.split(' ')[0]);
    return s;
  };
  switch (layout) {
    case 'single':
      return t.cmd === 'GET'
        ? { stmts: [add(`SET ${key} ${redisString(after)} KEEPTTL`)], diff: line('value') }
        : { stmts: [add(`HSET ${key} ${redisQuote(t.field ?? '')} ${redisString(after)}`)], diff: line(t.field ?? 'value') };
    case 'kv': {
      const field = text(rows[rowIdx]?.[0]);
      if (colIdx === 0) {
        // Renaming a field: no HRENAME in Redis — drop the old, set the new.
        const value = text(rows[rowIdx]?.[1]);
        return {
          stmts: [add(`HDEL ${key} ${redisQuote(before)}`), add(`HSET ${key} ${redisQuote(after)} ${redisString(value)}`)],
          diff: line(before, 'rename'),
        };
      }
      return { stmts: [add(`HSET ${key} ${redisQuote(field)} ${redisString(after)}`)], diff: line(field) };
    }
    case 'list': {
      const index = (t.start ?? 0) + rowIdx;
      return { stmts: [add(`LSET ${key} ${index} ${redisString(after)}`)], diff: line(`[${index}]`) };
    }
    case 'pairs': {
      const hash = t.cmd === 'HGETALL';
      if (rowIdx % 2 === 1) {
        // A value / score row — its field / member is the row above.
        const name = text(rows[rowIdx - 1]?.[0]);
        if (hash) return { stmts: [add(`HSET ${key} ${redisQuote(name)} ${redisString(after)}`)], diff: line(name) };
        if (!isScore(after)) return { stmts: [note(rowIdx, `"${after}" is not a numeric score`)], diff: line(name) };
        return { stmts: [add(`ZADD ${key} ${after.trim()} ${redisQuote(name)}`)], diff: line(name) };
      }
      // A field / member row — a rename: remove the old name, re-add with the
      // value / score from the row below (absent only when the row cap cut the
      // reply between the two — never write an empty value in that case).
      if (rows[rowIdx + 1] === undefined) {
        return { stmts: [note(rowIdx, 'its value was not loaded (the result is capped)')], diff: line(before, 'rename') };
      }
      const value = text(rows[rowIdx + 1][0]);
      return {
        stmts: hash
          ? [add(`HDEL ${key} ${redisQuote(before)}`), add(`HSET ${key} ${redisQuote(after)} ${redisString(value)}`)]
          : [add(`ZREM ${key} ${redisQuote(before)}`), add(`ZADD ${key} ${value || 0} ${redisQuote(after)}`)],
        diff: line(before, 'rename'),
      };
    }
    case 'members':
      // Unreachable through the UI: `value` is the identity column there.
      return { stmts: [note(rowIdx, 'set members are not editable in place (delete + re-add)')], diff: line('value') };
  }
}

/** Members / fields the selected rows name (a value row in the flat layout
 *  selects its field's row), deduplicated in selection order. */
function namesOf(layout: Layout, rows: unknown[][], idxs: number[]): string[] {
  const out: string[] = [];
  for (const i of idxs) {
    const name = layout === 'pairs' ? text(rows[i % 2 === 0 ? i : i - 1]?.[0]) : text(rows[i]?.[0]);
    if (!out.includes(name)) out.push(name);
  }
  return out;
}

export const redisAdapter: EditAdapter = {
  target(statement, columns) {
    const t = redisTarget(statement);
    if (!t) {
      return { target: null, reason: 'Editing needs a single GET / HGET / HGETALL / LRANGE / SMEMBERS / ZRANGE command.' };
    }
    if (t.cmd === 'LRANGE' && (t.start ?? -1) < 0) {
      return { target: null, reason: 'Editing a list needs an LRANGE with a non-negative start index.' };
    }
    const layout = layoutOf(t, columns);
    if (!layout) return { target: null, reason: 'The result shape does not match the command, so rows cannot be targeted.' };
    // `db` = the source command line (re-parsed by the builders); `pkCols`
    // names the identity: `key` (the field column of a RESP3 hash; absent —
    // i.e. every cell editable — in the flat layouts) or, for plain member
    // lists, the only column, which makes them delete-only.
    return {
      target: { db: statement.trim(), table: t.key, pkCols: layout === 'members' ? ['value'] : ['key'] },
      reason: null,
    };
  },

  /** One command per edited cell (a rename is two), all in one batch. */
  buildUpdate(rows, ctx) {
    const r = resolve(ctx);
    if (!r) return null;
    const stmts: string[] = [];
    const diff: DiffLine[] = [];
    const verbs = new Set<string>();
    for (const { rowIdx, patch } of rows) {
      for (const [ci, raw] of [...patch.cells.entries()].sort((a, b) => a[0] - b[0])) {
        const built = cellCommands(r, ctx, rowIdx, ci, draft(raw), verbs);
        stmts.push(...built.stmts);
        diff.push(built.diff);
      }
    }
    if (stmts.length === 0) return null;
    const title = verbs.size > 0 ? `Review ${[...verbs].join(' + ')}` : 'Review Redis update';
    return { title, sql: stmts.join('\n'), diff };
  },

  buildDelete(idxs, ctx) {
    const r = resolve(ctx);
    if (!r || idxs.length === 0) return null;
    const { t, layout } = r;
    const key = redisQuote(t.key);
    const rows = ctx.liveRows;
    const n = (k: number, noun: string) => `${k} ${noun}${k === 1 ? '' : 's'}`;
    switch (layout) {
      case 'single':
        return t.cmd === 'GET'
          ? { title: 'Review DEL', sql: `DEL ${key}` }
          : { title: 'Review HDEL', sql: `HDEL ${key} ${redisQuote(t.field ?? '')}` };
      case 'kv':
      case 'pairs': {
        const names = namesOf(layout, rows, idxs).map(redisQuote).join(' ');
        return t.cmd === 'HGETALL'
          ? { title: `Review HDEL (${n(idxs.length, 'field')})`, sql: `HDEL ${key} ${names}` }
          : { title: `Review ZREM (${n(idxs.length, 'member')})`, sql: `ZREM ${key} ${names}` };
      }
      case 'members': {
        const names = namesOf(layout, rows, idxs).map(redisQuote).join(' ');
        return t.cmd === 'SMEMBERS'
          ? { title: `Review SREM (${n(idxs.length, 'member')})`, sql: `SREM ${key} ${names}` }
          : { title: `Review ZREM (${n(idxs.length, 'member')})`, sql: `ZREM ${key} ${names}` };
      }
      case 'list': {
        // No delete-by-index: overwrite each element with a tombstone (every
        // index is still valid at that point), then remove them all at once.
        const sets = idxs.map((i) => `LSET ${key} ${(t.start ?? 0) + i} ${redisString(LIST_TOMBSTONE)}`);
        return {
          title: `Review LSET + LREM (${n(idxs.length, 'element')})`,
          sql: [...sets, `LREM ${key} 0 ${redisString(LIST_TOMBSTONE)}`].join('\n'),
        };
      }
    }
  },

  /** No INSERT equivalent: keys are written by the read command's write twin. */
  buildInsert() {
    return null;
  },
  buildDuplicate() {
    return null;
  },

  /** "Insert from JSON" on a hash: every entry becomes `HSET key field "value"`. */
  buildInsertDoc(doc, ctx) {
    const r = resolve(ctx);
    if (!r || r.t.cmd !== 'HGETALL') return null;
    const key = redisQuote(r.t.key);
    const lines = Object.entries(doc).map(
      ([f, v]) => `HSET ${key} ${redisQuote(f)} ${redisString(typeof v === 'string' ? v : text(v))}`,
    );
    return lines.length > 0 ? lines.join('\n') : null;
  },

  /** Whole-record save (Vertical / JSON pencil): every changed column of the
   *  row goes through the same per-cell builder as an inline edit. */
  buildReplace(rowIdx, doc, ctx) {
    const cells = new Map<number, string>();
    ctx.columns.forEach((c, ci) => {
      const next = doc[c.name];
      const after = next === null || next === undefined ? '' : typeof next === 'string' ? next : text(next);
      if (after !== text(ctx.liveRows[rowIdx]?.[ci])) cells.set(ci, after);
    });
    if (cells.size === 0) return null;
    const patch: RowPatch = { cells, set: new Map(), unset: new Set(), rename: new Map() };
    return redisAdapter.buildUpdate([{ rowIdx, patch }], ctx);
  },
};
