import { test } from 'node:test';
import assert from 'node:assert/strict';
import type { DbColumnDef } from '../src/lib/api/types.ts';
import {
  buildFormInsert,
  fieldBadges,
  fieldError,
  fieldKind,
  fieldsFromResult,
  fieldsFromSchema,
  type InsertField,
} from '../src/modules/database/insert-form.ts';

// The results grid's "Add row" form: fields come from the table's SCHEMA (so
// an empty table — no rows to clone — can still be filled) and become one
// INSERT that the review modal shows before anything runs.

const col = (c: Partial<DbColumnDef> & { name: string; data_type: string }): DbColumnDef => ({ nullable: true, ...c });
const by = (fields: InsertField[], name: string): InsertField => {
  const f = fields.find((x) => x.name === name);
  assert.ok(f, name);
  return f;
};
function sqlOf(r: ReturnType<typeof buildFormInsert>): string {
  assert.ok('sql' in r, 'error' in r ? r.error : 'no sql');
  return r.sql;
}

const mysqlUsers: DbColumnDef[] = [
  col({ name: 'id', data_type: 'bigint unsigned', nullable: false, key: 'PRI', extra: 'auto_increment' }),
  col({ name: 'email', data_type: 'varchar(255)', nullable: false }),
  col({ name: 'nick', data_type: 'varchar(40)' }),
  col({ name: 'active', data_type: 'tinyint(1)', nullable: false, default: '1' }),
  col({ name: 'score', data_type: 'decimal(10,2)', nullable: false }),
  col({ name: 'created_at', data_type: 'timestamp', nullable: false, default: 'CURRENT_TIMESTAMP', extra: 'DEFAULT_GENERATED' }),
  col({ name: 'email_lc', data_type: 'varchar(255)', extra: 'STORED GENERATED' }),
];

test('initial modes follow the schema: required → value, nullable → NULL, defaulted / auto / generated → default', () => {
  const f = fieldsFromSchema(mysqlUsers, ['id']);
  assert.equal(by(f, 'id').mode, 'default');
  assert.equal(by(f, 'id').auto, true);
  assert.equal(by(f, 'id').pk, true);
  assert.equal(by(f, 'email').mode, 'value');
  assert.equal(by(f, 'nick').mode, 'null');
  assert.equal(by(f, 'active').mode, 'default');
  assert.equal(by(f, 'created_at').mode, 'default');
  assert.equal(by(f, 'email_lc').generated, true);
  assert.deepEqual(fieldBadges(by(f, 'id')), ['PK', 'auto']);
  assert.deepEqual(fieldBadges(by(f, 'email')), ['required']);
  assert.deepEqual(fieldBadges(by(f, 'active')), ['default 1']);
});

test('MySQL: only set columns are listed; numbers bare, text escaped, booleans as 1/0', () => {
  const f = fieldsFromSchema(mysqlUsers, ['id']);
  Object.assign(by(f, 'email'), { mode: 'value', raw: "o'neil\\x@example.com" });
  Object.assign(by(f, 'score'), { mode: 'value', raw: '12.50' });
  Object.assign(by(f, 'active'), { mode: 'value', raw: 'false' });
  const r = buildFormInsert('mysql', '`shop`.`users`', f);
  assert.equal(
    sqlOf(r),
    "INSERT INTO `shop`.`users` (`email`, `nick`, `active`, `score`) VALUES ('o''neil\\\\x@example.com', NULL, 0, 12.50);",
  );
  assert.ok('diff' in r);
  assert.deepEqual(
    r.diff.map((d) => [d.path, d.after]),
    [['email', "'o''neil\\\\x@example.com'"], ['nick', 'NULL'], ['active', '0'], ['score', '12.50']],
  );
  assert.ok(r.diff.every((d) => d.row === -1 && d.before === '∅'));
});

test('validation: NOT NULL as NULL, non-numeric number, bad boolean', () => {
  const f = fieldsFromSchema(mysqlUsers, ['id']);
  // email is required and starts empty in value mode — '' is a legal string.
  Object.assign(by(f, 'score'), { mode: 'value', raw: 'abc' });
  const r = buildFormInsert('mysql', '`users`', f);
  assert.ok('error' in r);
  assert.equal(r.field, 'score');
  assert.match(r.error, /Not a number/);

  const email = by(f, 'email');
  email.mode = 'null';
  assert.match(fieldError(email) ?? '', /NOT NULL/);
  const active = by(f, 'active');
  Object.assign(active, { mode: 'value', raw: 'yes' });
  assert.equal(fieldError(active), 'Use true or false');
});

test('every column defaulted: MySQL () VALUES (), Postgres DEFAULT VALUES, ClickHouse refuses', () => {
  const cols = [col({ name: 'id', data_type: 'int', nullable: false, extra: 'auto_increment' })];
  assert.equal(sqlOf(buildFormInsert('mysql', '`t`', fieldsFromSchema(cols))), 'INSERT INTO `t` () VALUES ();');
  const pg = [col({ name: 'id', data_type: 'integer', nullable: false, default: "nextval('t_id_seq'::regclass)" })];
  const pgFields = fieldsFromSchema(pg);
  assert.equal(by(pgFields, 'id').auto, true);
  assert.equal(sqlOf(buildFormInsert('postgres', '"public"."t"', pgFields)), 'INSERT INTO "public"."t" DEFAULT VALUES;');
  const ch = [col({ name: 'ts', data_type: 'DateTime', nullable: false, default: 'DEFAULT now()' })];
  const r = buildFormInsert('clickhouse', '`logs`.`t`', fieldsFromSchema(ch));
  assert.ok('error' in r);
});

test('Postgres: double-quoted identifiers, TRUE/FALSE, backslashes kept literal', () => {
  const cols = [
    col({ name: 'Name', data_type: 'text', nullable: false }),
    col({ name: 'ok', data_type: 'boolean', nullable: false }),
    col({ name: 'n', data_type: 'double precision' }),
  ];
  const f = fieldsFromSchema(cols);
  Object.assign(by(f, 'Name'), { raw: 'a\\b' });
  Object.assign(by(f, 'ok'), { mode: 'value', raw: 'true' });
  Object.assign(by(f, 'n'), { mode: 'value', raw: '-1.5e3' });
  assert.equal(
    sqlOf(buildFormInsert('postgres', '"public"."t"', f)),
    'INSERT INTO "public"."t" ("Name", "ok", "n") VALUES (\'a\\b\', TRUE, -1.5e3);',
  );
});

test('ClickHouse: MATERIALIZED / ALIAS columns are locked out; Nullable(Int) is a number', () => {
  const cols = [
    col({ name: 'id', data_type: 'UInt64', nullable: false, key: 'PRI' }),
    col({ name: 'v', data_type: 'Nullable(Int32)', nullable: true }),
    col({ name: 'day', data_type: 'Date', nullable: false, default: 'MATERIALIZED toDate(ts)' }),
    col({ name: 'a', data_type: 'String', nullable: false, default: 'ALIAS upper(s)' }),
  ];
  const f = fieldsFromSchema(cols);
  assert.equal(by(f, 'day').generated, true);
  assert.equal(by(f, 'a').generated, true);
  // A generated field is never written, whatever its mode says.
  Object.assign(by(f, 'day'), { mode: 'value', raw: 'x' });
  Object.assign(by(f, 'id'), { raw: '7' });
  Object.assign(by(f, 'v'), { mode: 'value', raw: '3' });
  assert.equal(sqlOf(buildFormInsert('clickhouse', '`logs`.`t`', f)), 'INSERT INTO `logs`.`t` (`id`, `v`) VALUES (7, 3);');
});

test('fieldKind across engines', () => {
  for (const t of ['int', 'BIGINT UNSIGNED', 'int8', 'UInt64', 'Nullable(Int32)', 'decimal(10,2)', 'Decimal64(2)', 'Float64', 'numeric', 'real', 'serial']) {
    assert.equal(fieldKind(t), 'number', t);
  }
  for (const t of ['boolean', 'bool', 'Bool', 'tinyint(1)', 'Nullable(Bool)']) assert.equal(fieldKind(t), 'bool', t);
  for (const t of ['varchar(20)', 'interval', 'point', 'json', 'DateTime64(3)', 'uuid', "enum('a','b')", '']) {
    assert.equal(fieldKind(t), 'text', t);
  }
});

test('schema unavailable: result-column fallback starts every field as default (opt-in by typing)', () => {
  const f = fieldsFromResult([{ name: 'id', type_hint: 'INT' }, { name: 'note' }]);
  assert.ok(f.every((x) => x.mode === 'default'));
  Object.assign(by(f, 'note'), { mode: 'value', raw: 'hi' });
  assert.equal(sqlOf(buildFormInsert('mysql', '`t`', f)), "INSERT INTO `t` (`note`) VALUES ('hi');");
});
