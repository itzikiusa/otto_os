import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  PG_HIDDEN,
  PROCESS_SQL,
  killChoices,
  killNotDone,
  killSql,
  parseProcesses,
  processEngine,
  processNotices,
  queryPreview,
} from '../src/modules/database/process-list.ts';

// Processes panel: a read-only listing per engine, and a Kill statement that
// stops the query (not the connection) with its id quoted safely.

test('MySQL, PostgreSQL, ClickHouse and MongoDB have a process list', () => {
  assert.equal(processEngine('mysql'), 'mysql');
  assert.equal(processEngine('clickhouse'), 'clickhouse');
  assert.equal(processEngine('mongodb'), 'mongodb');
  assert.equal(processEngine('postgres'), 'postgres');
  assert.equal(processEngine('redis'), null);
  assert.equal(processEngine(undefined), null);
});

test('the listing statements match the daemon’s history exemption exactly', () => {
  const rust = readFileSync(new URL('../../crates/otto-dbviewer/src/service/process_list.rs', import.meta.url), 'utf8');
  assert.ok(rust.includes(`"${PROCESS_SQL.mysql}"`), 'MySQL text in process_list.rs');
  // The Rust constant is split with a `\` continuation; join it the way rustc does.
  const joined = rust.replace(/\\\n\s*/g, '');
  assert.ok(joined.includes(`"${PROCESS_SQL.clickhouse}"`), 'ClickHouse text in process_list.rs');
  assert.ok(joined.includes(`"${PROCESS_SQL.postgres}"`), 'PostgreSQL text in process_list.rs');
  assert.ok(rust.includes(`r#"${PROCESS_SQL.mongodb}"#`), 'MongoDB text in process_list.rs');
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

test('PostgreSQL rows: pid, user@client without the host mask, state + wait, own row dropped', () => {
  const cols = ['pid', 'usename', 'datname', 'client_addr', 'application_name', 'state', 'wait_event_type', 'wait_event', 'backend_type', 'seconds', 'query'].map(
    (name) => ({ name }),
  );
  const rows = parseProcesses('postgres', cols, [
    [4242, 'app', 'shop', '10.0.0.5/32', 'psql', 'active', 'Lock', 'relation', 'client backend', '12.3', 'UPDATE t SET a = 1'],
    [7, 'otto', 'shop', '::1/128', 'otto', 'active', null, null, 'client backend', '0.0', `${PROCESS_SQL.postgres} LIMIT 501`],
    [99, 'other', 'shop', null, '', null, null, null, null, null, PG_HIDDEN],
    [12, null, null, null, '', 'active', 'Activity', 'AutoVacuumMain', 'autovacuum launcher', 3, ''],
  ]);
  assert.equal(rows.length, 3, 'the listing session is not shown');
  assert.deepEqual(JSON.parse(JSON.stringify(rows[0])), {
    id: '4242',
    user: 'app@10.0.0.5',
    database: 'shop',
    seconds: 12.3,
    state: 'active · waiting on Lock: relation',
    query: 'UPDATE t SET a = 1',
  });
  assert.equal(rows[1].hidden, true);
  assert.equal(rows[1].query, '', 'the placeholder is not shown as query text');
  assert.equal(rows[1].user, 'other');
  assert.equal(rows[2].state, 'active · waiting on Activity: AutoVacuumMain · autovacuum launcher');
  assert.equal(rows[0].hidden, undefined);
});

test('PostgreSQL: a notice says why other roles’ sessions are hidden', () => {
  const [n] = processNotices('postgres', [{ id: '1', user: '', seconds: null, state: '', query: '', hidden: true }]);
  assert.match(n, /pg_read_all_stats/);
  assert.deepEqual(processNotices('postgres', [{ id: '1', user: '', seconds: null, state: '', query: 'x' }]), []);
  assert.deepEqual(processNotices('mysql', [], 'anything'), []);
});

test('MongoDB rows: opid, user · client, namespace, op · desc · lock, command text', () => {
  const cols = ['opid', 'user', 'client', 'ns', 'op', 'secs_running', 'waiting_for_lock', 'desc', 'command'].map((name) => ({ name }));
  const [a, b] = parseProcesses('mongodb', cols, [
    [4242, 'app@admin', '10.0.0.5:51234', 'shop.orders', 'query', 3.5, true, 'conn17', '{"find":"orders"}'],
    ['shard01:77', null, null, null, null, null, null, null, null],
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(a)), {
    id: '4242',
    user: 'app@admin · 10.0.0.5:51234',
    database: 'shop.orders',
    seconds: 3.5,
    state: 'query · conn17 · waiting for a lock',
    query: '{"find":"orders"}',
  });
  assert.deepEqual(JSON.parse(JSON.stringify(b)), { id: 'shard01:77', user: '', database: '', seconds: null, state: '', query: '' });
});

test('MongoDB: the limited-list message becomes a notice', () => {
  assert.deepEqual(processNotices('mongodb', [], 'only its own'), ['only its own']);
  assert.deepEqual(processNotices('mongodb', [], null), []);
});

test('PostgreSQL offers Cancel (default) then Terminate; the pid must be a positive int4', () => {
  assert.deepEqual(
    killChoices('postgres').map((c) => [c.action, c.label]),
    [
      ['query', 'Cancel'],
      ['connection', 'Terminate'],
    ],
  );
  assert.equal(killSql('postgres', { id: '4242' }), 'SELECT pg_cancel_backend(4242)');
  assert.equal(killSql('postgres', { id: '4242' }, 'connection'), 'SELECT pg_terminate_backend(4242)');
  for (const id of ['', '0', '-1', '1.5', '2147483648', '42); DROP TABLE t; --', ' 42', '0x10']) {
    assert.equal(killSql('postgres', { id }), null, id);
  }
  assert.equal(killSql('postgres', { id: '2147483647' }), 'SELECT pg_cancel_backend(2147483647)');
});

test('MongoDB kill is a typed killOp; other engines have no Terminate', () => {
  assert.deepEqual(killChoices('mongodb').map((c) => c.action), ['query']);
  assert.deepEqual(killChoices('mysql').map((c) => c.action), ['query']);
  assert.equal(killSql('mongodb', { id: '4242' }), '{"killOp":1,"op":4242}');
  assert.equal(killSql('mongodb', { id: 'shard-01:77' }), '{"killOp":1,"op":"shard-01:77"}');
  for (const id of ['', '-1', '1.5', '9007199254740993', 'a"b:1', 'shard:1; x']) {
    assert.equal(killSql('mongodb', { id }), null, id);
  }
  assert.equal(killSql('mongodb', { id: '1' }, 'connection'), null);
  assert.equal(killSql('mysql', { id: '1' }, 'connection'), null);
});

test('a PostgreSQL signal that found no process is reported, not called a success', () => {
  assert.match(killNotDone('postgres', { id: '9' }, [[false]]) ?? '', /did not signal pid 9/);
  assert.equal(killNotDone('postgres', { id: '9' }, [[true]]), null);
  assert.equal(killNotDone('mongodb', { id: '9' }, [['attempting to kill op']]), null);
  assert.equal(killNotDone('postgres', { id: '9' }, undefined), null);
});
