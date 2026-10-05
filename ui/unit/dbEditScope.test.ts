import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import ts from 'typescript';
import * as dialect from '../src/modules/database/sql-dialect.ts';

// Grid edits, "Copy as INSERT" and the post-apply refresh resolve the database
// a result RAN in from the tab's `ran_node` through this helper. It must mirror
// the daemon's `Scope` parse: tagged paths are paths, anything else a name.
function scopeDatabase(): (node: string | null | undefined) => string | null {
  const text = readFileSync(new URL('../src/modules/database/edit-sql.ts', import.meta.url), 'utf8');
  const start = text.indexOf('export function scopeDatabase(');
  const end = text.indexOf('\n}\n', start) + 3;
  const ctx: Record<string, any> = {};
  const source = text.slice(start, end).replace('export function', 'function');
  runInNewContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, ctx);
  return ctx.scopeDatabase;
}

test('scope database: plain names, db: paths, Redis keyspaces', () => {
  const f = scopeDatabase();
  assert.equal(f(null), null);
  assert.equal(f(undefined), null);
  assert.equal(f('  '), null);
  assert.equal(f('shop'), 'shop');
  assert.equal(f('db:shop/table:orders'), 'shop');
  assert.equal(f('db:'), null);
  // A Redis keyspace is not a database — Redis edits carry their own scope.
  assert.equal(f('kdb:3'), null);
  // Databases literally named like a path tag keep their scope.
  assert.equal(f('db'), 'db');
  assert.equal(f('kdb'), 'kdb');
  assert.equal(f('games:v2'), 'games:v2');
});

// 64-bit integers beyond 2^53 arrive as exact digit strings; edits must emit
// them BARE (a quoted value is compared as a double by MySQL) and verbatim.
function literals(): Record<string, any> {
  const text = readFileSync(new URL('../src/modules/database/edit-sql.ts', import.meta.url), 'utf8');
  const start = text.indexOf('export function isIntegerType(');
  const end = text.indexOf('/** The cell DRAFT');
  const ctx: Record<string, any> = {
    escapeSqlText: (s: string) => s.replace(/'/g, "''"),
    backslashEscapes: () => false,
    boolLiteral: dialect.boolLiteral,
    isComplex: () => false,
    compactJson: (v: unknown) => JSON.stringify(v),
  };
  const source = text.slice(start, end).replace(/export function/g, 'function');
  runInNewContext(ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText, ctx);
  return ctx;
}

test('big integer keys keep every digit and stay unquoted', () => {
  const c = literals();
  for (const t of ['BIGINT', 'BIGINT UNSIGNED', 'INT8', 'int4', 'Nullable(UInt64)', 'Int64', 'INTEGER']) {
    assert.equal(c.isIntegerType(t), true, t);
  }
  for (const t of ['POINT', 'INTERVAL', 'VARCHAR', 'TEXT', null, undefined]) {
    assert.equal(c.isIntegerType(t), false, String(t));
  }
  assert.equal(c.valueLiteral('mysql', '9007199254740993', 'BIGINT'), '9007199254740993');
  assert.equal(c.valueLiteral('postgres', '-9007199254740993', 'INT8'), '-9007199254740993');
  // A digit string in a text column is still a quoted string.
  assert.equal(c.valueLiteral('mysql', '9007199254740993', 'VARCHAR'), "'9007199254740993'");
  assert.equal(c.valueLiteral('mysql', 42, 'BIGINT'), '42');
});

function sqlModule(): Record<string, any> {
  const ctx = { exports: {} as Record<string, any>, require: () => ({isComplex:()=>false,cellStr:String,compactJson:JSON.stringify,...dialect}) };
  runInNewContext(ts.transpileModule(readFileSync(new URL('../src/modules/database/edit-sql.ts', import.meta.url), 'utf8'),
    { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2022 } }).outputText, ctx);
  return ctx.exports;
}

test('row mutations require direct, unique column provenance and a complete single FROM', () => {
  const { parseSimpleSelect, sqlAdapter } = sqlModule();
  for (const sql of ['SELECT other_id AS id, name FROM app.users', 'SELECT id + 1 AS id FROM app.users',
    'SELECT name AS other_id, id FROM app.users', 'SELECT id, id FROM app.users',
    'SELECT * FROM app.users, app.other', 'SELECT * FROM app.users u',
    'SELECT * FROM app.users CROSS JOIN app.other', 'SELECT * FROM app.users FINAL']) {
    assert.equal(parseSimpleSelect(sql), null, sql);
  }
  assert.equal(JSON.stringify(parseSimpleSelect('SELECT id, name FROM app.users WHERE id = 1')), '{"db":"app","table":"users"}');
  assert.equal(JSON.stringify(parseSimpleSelect('SELECT * FROM "app"."users" ORDER BY id LIMIT 5')), '{"db":"app","table":"users"}');
  assert.equal(sqlAdapter.target('SELECT * FROM app.users', ['id', 'id']).target, null);
  assert.equal(sqlAdapter.target('SELECT "id", name FROM app.users', ['id','name'], {engine:'mysql',activeDb:'app'}).target, null);
  assert.notEqual(sqlAdapter.target('SELECT "id", name FROM app.users', ['id','name'], {engine:'postgres',activeDb:'app'}).target, null);
});

test('ClickHouse mutation builders reject nonunique keys while inserts remain available', () => {
  const { sqlAdapter } = sqlModule();
  const ctx = {engine:'clickhouse',target:{db:'app',table:'events',pkCols:['account_id']},
    columns:[{name:'account_id'},{name:'note'}],liveRows:[[7,'a']],qid:(s: string)=>s};
  assert.equal(sqlAdapter.buildDelete([0],ctx),null);
  assert.equal(sqlAdapter.buildUpdate([{rowIdx:0,patch:{cells:new Map([[1,'changed']])}}],ctx),null);
  assert.equal(sqlAdapter.buildReplace(0,{note:'changed'},ctx),null);
  assert.equal(sqlAdapter.buildInsert([0],ctx),"INSERT INTO app.events (account_id, note) VALUES (7, 'a');");
});

test('Postgres table and schema names fold only unquoted identifiers', () => {
  const {parseSimpleSelect} = sqlModule();
  for(const [sql,db,table] of [
    ['SELECT id FROM app.Users','app','users'],['SELECT id FROM APP.users','app','users'],
    ['SELECT id FROM app."Users"','app','Users'],['SELECT id FROM "APP".users','APP','users'],
  ]) assert.equal(JSON.stringify(parseSimpleSelect(sql,'postgres')),JSON.stringify({db,table}));
  assert.equal(parseSimpleSelect('SELECT id FROM `app`.`users`','postgres'),null);
});

test('Postgres mutation builder quotes the resolved source table, not its unquoted spelling', () => {
  const {parseSimpleSelect,sqlAdapter,qid}=sqlModule();
  for(const [source,expected] of [['app.Users','"app"."users"'],['app."Users"','"app"."Users"'],['APP.users','"app"."users"'],['"APP".users','"APP"."users"']]) {
    const target={...parseSimpleSelect(`SELECT id, name FROM ${source}`,'postgres'),pkCols:['id']};
    const ctx={engine:'postgres',target,columns:[{name:'id'},{name:'name'}],liveRows:[[1,'source']],qid:(s:string)=>qid('postgres',s)};
    const update=sqlAdapter.buildUpdate([{rowIdx:0,patch:{cells:new Map([[1,'changed']])}}],ctx);
    assert.equal(update.sql,`UPDATE ${expected} SET "name" = 'changed' WHERE "id" = 1;`);
  }
});
