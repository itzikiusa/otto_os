// Fixtures: the raw → clean table in review 04-db-editor-errors.md, in both the
// shape a current daemon sends (cleaned text + tagged trailer lines) and the
// legacy shape older daemons send.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  applySuggestion,
  nearest,
  normalizeDbError,
  offsetToPosition,
  parseTrailers,
} from '../src/modules/database/error-normalize.ts';

test('trailer lines split from the message', () => {
  const { message, tags } = parseTrailers('column "nme" does not exist\nHINT: try x\nSQLSTATE: 42703\nPOSITION: 8');
  assert.equal(message, 'column "nme" does not exist');
  assert.deepEqual(tags, { HINT: 'try x', SQLSTATE: '42703', POSITION: '8' });
});

test('ClickHouse native unknown table: title, server suggestion, code chip', () => {
  const e = normalizeDbError(
    'clickhouse',
    'upstream: Code: 60. DB::Exception: Table default.userz does not exist. Maybe you meant default.users?. (UNKNOWN_TABLE)',
  );
  assert.equal(e.title, 'Table `default.userz` doesn\'t exist.');
  assert.deepEqual(e.suggestions, ['default.users']);
  assert.equal(e.code, 'Code 60 · UNKNOWN_TABLE');
  assert.equal(e.kind, 'unknown_table');
});

test('ClickHouse legacy native text with stack trace still reads cleanly', () => {
  const e = normalizeDbError(
    'clickhouse',
    'upstream: Code: 60. DB::Exception: DB::Exception: Table default.userz does not exist. (UNKNOWN_TABLE)\n0. DB::Exception::Exception(DB::Exception::MessageMasked&&, int, bool) @ 0x000000000c6f4a3b in /usr/bin/clickhouse',
  );
  assert.equal(e.title, 'Table `default.userz` doesn\'t exist.');
  assert.equal(e.code, 'Code 60 · UNKNOWN_TABLE');
});

test('ClickHouse HTTP unknown column with Maybe you meant list', () => {
  const e = normalizeDbError(
    'clickhouse',
    "upstream: Code: 47. DB::Exception: Unknown expression identifier `nme` in scope SELECT nme FROM users. Maybe you meant: ['name']. (UNKNOWN_IDENTIFIER) (version 24.8.4.13 (official build))",
  );
  assert.equal(e.title, 'Unknown column `nme`.');
  assert.deepEqual(e.suggestions, ['name']);
  assert.equal(e.token, 'nme');
  assert.equal(e.code, 'Code 47 · UNKNOWN_IDENTIFIER');
});

test('ClickHouse syntax error: near token, line/col caret, expected list', () => {
  const e = normalizeDbError(
    'clickhouse',
    "upstream: Code: 62. DB::Exception: Syntax error: failed at position 8 ('FORM') (line 1, col 8): FORM users. Expected one of: token, Comma, AS, FROM, PREWHERE, WHERE, …. (SYNTAX_ERROR)",
    'SELECT FORM users',
  );
  assert.equal(e.title, 'Syntax error near `FORM` (line 1, col 8).');
  assert.deepEqual(e.position, { line: 1, col: 8 });
  assert.match(e.cause ?? '', /^Expected one of: token, Comma, AS, FROM/);
  assert.equal(e.kind, 'syntax');
});

test('ClickHouse memory, timeout, auth and readonly', () => {
  const mem = normalizeDbError(
    'clickhouse',
    'Code: 241. DB::Exception: Memory limit (for query) exceeded: would use 9.31 GiB (attempt to allocate chunk of 4194304 bytes), maximum: 9.31 GiB.: While executing AggregatingTransform. (MEMORY_LIMIT_EXCEEDED)',
  );
  assert.equal(mem.title, 'Query ran out of memory (limit 9.31 GiB).');
  assert.match(mem.hint ?? '', /uniq instead of uniqExact/);
  const to = normalizeDbError(
    'clickhouse',
    'Code: 159. DB::Exception: Timeout exceeded: elapsed 30.0001 seconds, maximum: 30. (TIMEOUT_EXCEEDED)',
  );
  assert.equal(to.title, 'Stopped after 30 s (server max_execution_time).');
  assert.equal(to.kind, 'timeout');
  const auth = normalizeDbError(
    'clickhouse',
    'Code: 516. DB::Exception: default: Authentication failed: password is incorrect, or there is no user with such name.. (AUTHENTICATION_FAILED)',
  );
  assert.equal(auth.title, 'Sign-in to ClickHouse failed for `default`.');
  assert.equal(auth.hint, 'Check the profile\'s user and password.');
  const ro = normalizeDbError('clickhouse', 'Code: 164. DB::Exception: Cannot execute query in readonly mode. (READONLY)');
  assert.equal(ro.title, 'This user is read-only on the server.');
});

test('ClickHouse mid-stream failure says rows had streamed (E7)', () => {
  const e = normalizeDbError(
    'clickhouse',
    'upstream: Code: 241. DB::Exception: Memory limit (for query) exceeded: would use 9.31 GiB, maximum: 9.31 GiB. (MEMORY_LIMIT_EXCEEDED)\nSTREAMED_ROWS: 12400',
  );
  assert.equal(e.title, 'Query ran out of memory (limit 9.31 GiB).');
  assert.equal(e.streamedRows, 12400);
  assert.match(e.cause ?? '', /after 12,400 rows had streamed/);
  const unknown = normalizeDbError('clickhouse', 'Code: 159. DB::Exception: x, maximum: 30. (TIMEOUT_EXCEEDED)\nSTREAMED_ROWS: unknown');
  assert.equal(unknown.streamedRows, 'unknown');
});

test('ClickHouse network error drops the request URL', () => {
  const e = normalizeDbError(
    'clickhouse',
    'upstream: error sending request for url (http://10.0.3.4:8123/?database=prod&query_id=otto-1): client error (Connect): tcp connect error: Connection refused (os error 61)',
  );
  assert.equal(e.title, 'Can\'t reach ClickHouse at 10.0.3.4:8123 (connection refused).');
  assert.equal(e.kind, 'network');
  assert.ok(!e.title.includes('query_id'));
});

test('MySQL unknown column: tagged and legacy shapes, daemon suggestions', () => {
  const tagged = normalizeDbError(
    'mysql',
    "upstream: Unknown column 'nme' in 'field list'\nERRNO: 1054\nSQLSTATE: 42S22\nSUGGEST: name, nm",
  );
  assert.equal(tagged.title, 'Unknown column `nme`.');
  assert.equal(tagged.code, 'Error 1054 · 42S22');
  assert.deepEqual(tagged.suggestions, ['name', 'nm']);
  const legacy = normalizeDbError('mysql', "upstream: error returned from database: 1054 (42S22): Unknown column 'nme' in 'field list'");
  assert.equal(legacy.title, 'Unknown column `nme`.');
  assert.equal(legacy.code, 'Error 1054 · 42S22');
});

test('MySQL syntax, auth, network, pool, timeout', () => {
  const syn = normalizeDbError(
    'mysql',
    "You have an error in your SQL syntax; check the manual that corresponds to your MySQL server version for the right syntax to use near 'FORM users' at line 1\nERRNO: 1064\nSQLSTATE: 42000",
  );
  assert.equal(syn.title, 'Syntax error near `FORM users` (line 1).');
  assert.deepEqual(syn.position, { line: 1 });
  const auth = normalizeDbError('mysql', "error returned from database: 1045 (28000): Access denied for user 'app'@'10.0.0.5' (using password: YES)");
  assert.equal(auth.title, 'MySQL rejected user `app` from 10.0.0.5.');
  const net = normalizeDbError('mysql', 'error communicating with database: Connection refused (os error 61)');
  assert.equal(net.title, 'Can\'t reach MySQL (connection refused).');
  const pool = normalizeDbError('mysql', 'pool timed out while waiting for an open connection');
  assert.equal(pool.title, 'All connections are busy.');
  const to = normalizeDbError('mysql', 'Query execution was interrupted, maximum statement execution time exceeded\nERRNO: 3024\nSQLSTATE: HY000');
  assert.equal(to.title, 'Stopped by the tab timeout.');
});

test('Postgres unknown column: server HINT becomes a chip, POSITION a caret', () => {
  const e = normalizeDbError(
    'postgres',
    'upstream: column "nme" does not exist\nHINT: Perhaps you meant to reference the column "users.name".\nSQLSTATE: 42703\nPOSITION: 8',
    'SELECT nme FROM users',
  );
  assert.equal(e.title, 'Unknown column `nme`.');
  assert.deepEqual(e.suggestions, ['name']);
  assert.equal(e.code, '42703');
  assert.deepEqual(e.position, { line: 1, col: 8 });
});

test('Postgres unique violation keeps DETAIL; auth headline', () => {
  const dup = normalizeDbError(
    'postgres',
    'duplicate key value violates unique constraint "users_email_key"\nDETAIL: Key (email)=(a@b.c) already exists.\nSQLSTATE: 23505',
  );
  assert.equal(dup.title, 'Duplicate value for `users_email_key`.');
  assert.equal(dup.cause, 'Key (email)=(a@b.c) already exists.');
  assert.equal(dup.code, '23505');
  const auth = normalizeDbError('postgres', 'password authentication failed for user "app"\nSQLSTATE: 28P01');
  assert.equal(auth.title, 'Sign-in to Postgres failed for `app`.');
});

test('Mongo command, write, unreachable — tagged and legacy', () => {
  const op = normalizeDbError('mongodb', 'upstream: unknown operator: $regx\nCODE: 2 BadValue');
  assert.equal(op.title, 'Unknown operator `$regx`.');
  assert.ok(op.suggestions.includes('$regex'));
  assert.equal(op.code, '2 BadValue');
  const legacy = normalizeDbError(
    'mongodb',
    'upstream: Kind: Command failed: Error code 2 (BadValue): unknown operator: $regx, labels: {}, source: None, server response: Some(Document({"ok": Double(0.0)}))',
  );
  assert.equal(legacy.title, 'Unknown operator `$regx`.');
  assert.equal(legacy.code, '2 BadValue');
  const dup = normalizeDbError(
    'mongodb',
    'E11000 duplicate key error collection: app.users index: email_1 dup key: { email: "a@b.c" }\nCODE: 11000',
  );
  assert.equal(dup.title, 'Duplicate key on index `email_1`.');
  assert.equal(dup.cause, '{ email: "a@b.c" } already exists in app.users.');
  const down = normalizeDbError('mongodb', "Can't reach MongoDB at db:27017: I/O error: Connection refused (os error 61)");
  assert.equal(down.title, 'Can\'t reach MongoDB at db:27017 (connection refused).');
});

test('mongosh script failure: exit code + last error line', () => {
  const e = normalizeDbError(
    'mongodb',
    'mongosh exited 1\nLoading file...\nMongoServerError: not authorized on app to execute command { find: "x" }\n\n',
  );
  assert.equal(e.title, 'Script failed (exit 1).');
  assert.match(e.cause ?? '', /^MongoServerError: not authorized/);
});

test('Redis wrong type, unknown command, auth', () => {
  assert.equal(
    normalizeDbError('redis', 'WRONGTYPE: Operation against a key holding the wrong kind of value').title,
    'Key holds a different type.',
  );
  const cmd = normalizeDbError('redis', "An error was signalled by the server - ResponseError: unknown command 'HGETX', with args beginning with:");
  assert.equal(cmd.title, 'Unknown command `HGETX`.');
  assert.equal(cmd.suggestions[0], 'HGET');
  assert.equal(normalizeDbError('redis', 'WRONGPASS invalid username-password pair').title, 'Redis sign-in failed.');
});

test('unrecognised text falls back to its first line', () => {
  const e = normalizeDbError('mysql', 'something odd happened\nmore');
  assert.equal(e.title, 'Something odd happened');
  assert.equal(e.raw, 'something odd happened\nmore');
});

test('offset → line/col and out-of-range positions', () => {
  assert.deepEqual(offsetToPosition('SELECT 1\nFROM x', 10), { line: 2, col: 1 });
  assert.equal(offsetToPosition('SELECT', 99), undefined);
  const e = normalizeDbError('mysql', 'bad near \'x\' at line 9\nERRNO: 1064', 'SELECT 1');
  assert.equal(e.position, undefined);
});

test('nearest + applySuggestion', () => {
  assert.deepEqual(nearest('nme', ['name', 'email', 'id']), ['name']);
  assert.equal(applySuggestion('SELECT nme, u.nme FROM users', 'nme', 'name'), 'SELECT name, u.name FROM users');
  assert.equal(applySuggestion('SELECT * FROM userz', 'default.userz', 'default.users'), 'SELECT * FROM users');
  assert.equal(applySuggestion('SELECT * FROM t', 'nme', 'name'), null);
  assert.equal(applySuggestion('db.c.find({a: {$regx: 1}})', '$regx', '$regex'), 'db.c.find({a: {$regex: 1}})');
  // Whole-word only: `nmes` is untouched.
  assert.equal(applySuggestion('SELECT nmes, nme FROM t', 'nme', 'name'), 'SELECT nmes, name FROM t');
});
