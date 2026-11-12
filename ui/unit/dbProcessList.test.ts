import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { PROCESS_SQL, killSql, parseProcesses, processEngine, queryPreview } from '../src/modules/database/process-list.ts';

// Processes panel: a read-only listing per engine, and a Kill statement that
// stops the query (not the connection) with its id quoted safely.

test('only MySQL and ClickHouse have a process list', () => {
  assert.equal(processEngine('mysql'), 'mysql');
  assert.equal(processEngine('clickhouse'), 'clickhouse');
  assert.equal(processEngine('mongodb'), null);
  assert.equal(processEngine('postgres'), null);
  assert.equal(processEngine(undefined), null);
});

test('the listing statements match the daemon’s history exemption exactly', () => {
  const rust = readFileSync(new URL('../../crates/otto-dbviewer/src/service/process_list.rs', import.meta.url), 'utf8');
  assert.ok(rust.includes(`"${PROCESS_SQL.mysql}"`), 'MySQL text in process_list.rs');
  // The Rust constant is split with a `\` continuation; join it the way rustc does.
  const joined = rust.replace(/\\\n\s*/g, '');
  assert.ok(joined.includes(`"${PROCESS_SQL.clickhouse}"`), 'ClickHouse text in process_list.rs');
});

test('MySQL rows: user@host, Command · State, Info as the query', () => {
  const cols = ['Id', 'User', 'Host', 'db', 'Command', 'Time', 'State', 'Info'].map((name) => ({ name }));
  const [p] = parseProcesses('mysql', cols, [[42, 'app', '10.0.0.5:5123', 'shop', 'Query', 17, 'Sending data', 'SELECT * FROM orders']]);
  assert.deepEqual(JSON.parse(JSON.stringify(p)), {
    id: '42',
    user: 'app@10.0.0.5:5123',
    seconds: 17,
    state: 'Query · Sending data',
    query: 'SELECT * FROM orders',
  });
});

test('ClickHouse rows: query_id, elapsed seconds, rows read', () => {
  const cols = ['query_id', 'user', 'elapsed', 'read_rows', 'memory_usage', 'query'].map((name) => ({ name }));
  const [p] = parseProcesses('clickhouse', cols, [['ab-1', 'default', '2.5', '1000', '2048', 'SELECT count() FROM t']]);
  assert.equal(p.id, 'ab-1');
  assert.equal(p.seconds, 2.5);
  assert.equal(p.state, '1000 rows read');
});

test('Kill stops the query, never the connection, and quotes the id', () => {
  assert.equal(killSql('mysql', { id: '42' }), 'KILL QUERY 42');
  assert.equal(killSql('mysql', { id: '42; DROP TABLE t' }), null);
  assert.equal(killSql('clickhouse', { id: "ab'1\\" }), "KILL QUERY WHERE query_id = 'ab\\'1\\\\' ASYNC");
  assert.equal(killSql('clickhouse', { id: '' }), null);
});

test('the confirm names the query on one capped line', () => {
  assert.equal(queryPreview('SELECT\n  *\nFROM t'), 'SELECT * FROM t');
  assert.equal(queryPreview('x'.repeat(300), 10), `${'x'.repeat(9)}…`);
  assert.match(queryPreview('  '), /no statement/);
});
