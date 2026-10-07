import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';
import * as plural from '../src/lib/plural.ts';

// The daemon runs a Redis "statement" one command per LINE, splitting on line
// breaks before it tokenizes quotes. A key / field / member / value that holds a
// `\n` must therefore never reach a built command — a set member `x\nFLUSHALL`
// deleted through SREM would otherwise run FLUSHALL as the next command.
const SET_NULL = '\u0000<null>';
const SET_EMPTY = '\u0000<empty>';
const mod = loadSource(new URL('../src/modules/database/edit-redis.ts', import.meta.url), {
  '../../lib/plural': plural,
  './results-format': { cellStr: (v: unknown) => (typeof v === 'string' ? v : JSON.stringify(v)), SET_NULL, SET_EMPTY },
});
const adapter = mod.redisAdapter;

for (const fixture of [
  { statement: 'LRANGE k 0 -1', rows: [['__otto_deleted__'], ['chosen'], ['__otto_deleted__']], indices: [1] },
  { statement: 'LRANGE k 40 42', rows: [['duplicate'], ['duplicate'], ['__otto_deleted__']], indices: [0, 1] },
  { statement: 'LRANGE k 4 4', rows: [['chosen']], indices: [0] },
]) test(`list index deletion is refused without an executable mutation: ${fixture.statement}`, () => {
  assert.equal(adapter.buildDelete(fixture.indices, ctx(fixture.statement, fixture.rows)), null);
});

test('refusing list deletion retains safe edits at the actual LRANGE offset', () => {
  const patch = { cells: new Map([[0, 'replacement']]), set: new Map(), unset: new Set(), rename: new Map() };
  const built = adapter.buildUpdate([{ rowIdx: 1, patch }], ctx('LRANGE k 40 42', [['same'], ['same']]));
  assert.equal(built.sql, 'LSET k 41 "replacement"');
});

test('the grid delete flow cannot open or execute a Redis list mutation', async () => {
  let writes = 0;
  const explanations: string[][] = [];
  const { EditFlow } = loadSource(new URL('../src/modules/database/EditFlow.svelte.ts', import.meta.url), {
    '../../lib/toast.svelte': { toasts: { info: (...message: string[]) => explanations.push(message) } },
    '../../lib/stores/database.svelte': { database: { runManagedStatement: () => { writes++; } } },
    './results-format': { SET_NULL, SET_EMPTY },
    './edit-types': { adapterFor: () => adapter },
    './edit-sql': {}, './edit-mongo': {}, './expansion-plan': {},
    '../../lib/toastError': {}, '../../lib/plural': plural,
  });
  const flow = new EditFlow();
  // The source harness runs plain runes; explicitly supply the resolved
  // editable target that the real grid's reactive target resolution produces.
  Object.assign(flow, {
    editable: true, result: { columns: [{ name: 'value' }] }, engine: 'redis',
    editDb: 'LRANGE k 40 42', editTable: 'k', editPkCols: ['key'],
    connectionId: 'fixture', liveRows: [['__otto_deleted__'], ['chosen'], ['chosen']],
    selected: new Set([1, 2]),
  });
  flow.deleteSelected();
  assert.equal(flow.reviewSql, null);
  await flow.runReview();
  assert.equal(writes, 0);
  flow.deleteRows([1]);
  assert.equal(flow.reviewSql, null);
  assert.equal(explanations.length, 2, 'both delete entry points explain the refusal');
  assert.match(explanations[0].join(' '), /cannot be deleted safely/i);
});

function ctx(statement: string, rows: unknown[][], columns = ['value']) {
  const t = adapter.target(statement, columns);
  assert.ok(t.target, t.reason ?? 'no target');
  return { engine: 'redis', columns: columns.map((name) => ({ name })), liveRows: rows, target: t.target, qid: (s: string) => s };
}

/** Every non-comment line the daemon would execute, as its command word. */
function commands(sql: string): string[] {
  return sql
    .split('\n')
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith('#'))
    .map((l) => l.split(/\s+/)[0]);
}

test('SREM: a member with a line break is skipped, never split into a second command', () => {
  const c = ctx('SMEMBERS k', [['ok'], ['x\nFLUSHALL']]);
  const built = adapter.buildDelete([0, 1], c);
  assert.deepEqual(commands(built.sql), ['SREM']);
  assert.ok(!built.sql.includes('FLUSHALL'));
  assert.match(built.sql, /^# 1 selected name with a line break/m);
  assert.match(built.sql, /^SREM k ok$/m);
  // Only unsafe rows selected: nothing executable at all.
  assert.deepEqual(commands(adapter.buildDelete([1], c).sql), []);
});

test('HDEL (flat HGETALL) and ZREM (WITHSCORES) refuse line-broken names', () => {
  const h = ctx('HGETALL k', [['f\nFLUSHALL'], ['v'], ['g'], ['w']]);
  const hd = adapter.buildDelete([0, 2], h);
  assert.deepEqual(commands(hd.sql), ['HDEL']);
  assert.match(hd.sql, /^HDEL k g$/m);
  const z = ctx('ZRANGE z 0 -1 WITHSCORES', [['m\r\nDEL other'], ['1']]);
  const zd = adapter.buildDelete([0], z);
  assert.deepEqual(commands(zd.sql), []);
  assert.ok(!zd.sql.includes('DEL other'));
  const zm = ctx('ZRANGE z 0 -1', [['a\nb'], ['c']]);
  assert.deepEqual(commands(adapter.buildDelete([0, 1], zm).sql), ['ZREM']);
});

test('Insert from JSON: a line-broken field or value refuses the whole document', () => {
  const c = ctx('HGETALL k', [['f'], ['v']]);
  const bad = adapter.buildInsertDoc({ ok: 'fine', bio: 'a\nb' }, c);
  assert.deepEqual(commands(bad), []);
  assert.match(bad, /"bio"/);
  assert.deepEqual(commands(adapter.buildInsertDoc({ 'x\nFLUSHALL': 'v' }, c)), []);
  assert.equal(adapter.buildInsertDoc({ ok: 'fine' }, c), 'HSET k ok "fine"');
});

test('a rename that re-emits a line-broken value from the data is refused as a whole', () => {
  // Renaming field `f` re-sets its value — which holds a line break — under
  // the new name: neither the HDEL nor the HSET may run.
  const c = ctx('HGETALL k', [['f'], ['v\nFLUSHALL']]);
  const patch = { cells: new Map([[0, 'g']]), set: new Map(), unset: new Set(), rename: new Map() };
  const built = adapter.buildUpdate([{ rowIdx: 0, patch }], c);
  assert.deepEqual(commands(built.sql), []);
  // A typed value with a line break is refused too; a clean one goes through.
  const typed = { cells: new Map([[0, 'a\nb']]), set: new Map(), unset: new Set(), rename: new Map() };
  assert.deepEqual(commands(adapter.buildUpdate([{ rowIdx: 1, patch: typed }], c).sql), []);
  const ok = { cells: new Map([[0, 'clean']]), set: new Map(), unset: new Set(), rename: new Map() };
  assert.deepEqual(commands(adapter.buildUpdate([{ rowIdx: 1, patch: ok }], ctx('HGETALL k', [['f'], ['v']])).sql), ['HSET']);
});
