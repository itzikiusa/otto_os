import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  analyzeStatement,
  countStatements,
  extractVars,
  looksLikeMongoshScript,
  stmtPreview,
  statementAtCursor,
  type SplitMode,
} from '../src/modules/database/sql-util.ts';

test('MySQL delimiter scripts run intact from inside a routine', () => {
  const sql = "DELIMITER $$\nCREATE PROCEDURE p() BEGIN SELECT :x; SELECT 2; END$$\nDELIMITER ;\nCALL p();";
  assert.equal(statementAtCursor(sql, sql.indexOf('SELECT 2')), sql);
  assert.equal(countStatements(sql), 1);
  assert.deepEqual(analyzeStatement(sql), { count: 1, vars: ['x'] });
});

test('quoted MySQL delimiter scripts retain their whole buffer', () => {
  const sql = "DELIMITER 'x y'\nSELECT 1x y\nDELIMITER ;\nSELECT 2;";
  assert.equal(statementAtCursor(sql, sql.indexOf('SELECT 2')), sql);
  assert.equal(countStatements(sql), 1);
});

test('delimiter text inside a string or comment does not change execution selection', () => {
  for (const sql of ["SELECT '\nDELIMITER $$\n'; SELECT 2;", "/*\nDELIMITER $$\n*/ SELECT 1; SELECT 2;"]) {
    assert.equal(statementAtCursor(sql, sql.indexOf('SELECT 2')), 'SELECT 2');
  }
});

// The editor derives "Run all N" and the Variables bar from `analyzeStatement`
// (one pass, no mask array) instead of `countStatements` + `extractVars` (two
// full-buffer scans per keystroke). Both must agree EXACTLY — this pins it.

function parity(sql: string, mode: SplitMode): void {
  const want = { count: countStatements(sql, mode), vars: extractVars(sql, mode) };
  assert.deepEqual(analyzeStatement(sql, mode), want, `${mode}: ${JSON.stringify(sql)}`);
}

const FIXTURES: string[] = [
  '',
  '   \n\t ',
  ';',
  ';;; ;',
  'SELECT 1',
  'SELECT 1;',
  'SELECT 1; SELECT 2;\n',
  'SELECT 1; -- trailing comment',
  'SELECT 1; /* block */',
  '/* unterminated block ; SELECT 2',
  "SELECT 'a;b', \"c;d\", `e;f`; SELECT 2",
  "SELECT 'it''s; fine'; SELECT \"q\"\"x;\"",
  "SELECT 'back\\'slash;'; SELECT 3",
  "SELECT 'unterminated ; string",
  "SELECT 'ends with backslash \\",
  '# hash comment ; still comment\nSELECT 1;',
  '-- dash comment ; x\nSELECT 1;\n-- only a comment',
  // Variables: colon (not ::casts / key:value), brace, mustache, in and out of strings.
  "SELECT * FROM t WHERE a = :id AND b = {{x}} AND c = {y} AND d::int = 1 AND e = 'x:y';",
  "SELECT ':in_string', '{also}', \"{{nope}}\" FROM t WHERE k = :real -- :comment {c}",
  'SELECT {{{a}}}, {{b}, {c}}, x{d}, }{e}, :_u1, 1:no, a:b, }:after, ::cast',
  'SELECT :a, :a, {a}, {{a}}; SELECT :b',
  // Backtick identifiers (a string in SQL mode, NOT in line mode).
  'SELECT `weird;col`, `x``y:z` FROM `t:1`;',
  // Postgres dollar-quoting: code in `sql` mode, a string in `pg` mode — the
  // scanner must match codeMask in both.
  'CREATE FUNCTION f() RETURNS int AS $$ SELECT 1; $$ LANGUAGE sql; SELECT :v',
  // Mongo buffers (sql mode) and a mongosh script.
  'db.users.find({ name: "a;b", age: { $gt: 1 } }); db.orders.find({})',
  'const x = 1;\nfunction f() { return db.c.find({ k: "{v}" }); }\nprint(f());',
  // Redis (line mode): one command per line, `{tag}` is a hash tag, `;` lines.
  'SET user:1 "a;b"\nGET {user}:1\n;\n  ;  \n\nHGETALL :key\nSET k `v:x`',
  // Unicode whitespace that trim() strips.
  ' ;﻿; ;SELECT　:u',
];

test('analyzeStatement matches countStatements + extractVars on fixtures', () => {
  for (const sql of FIXTURES) for (const mode of ['sql', 'line', 'pg'] as const) parity(sql, mode);
});

test('analyzeStatement matches on a 200-INSERT paste (the lag report)', () => {
  const rows: string[] = [];
  for (let i = 0; i < 200; i++) {
    rows.push(
      `INSERT INTO players (id, login, note) VALUES (${i}, 'user_${i}', 'it''s row ${i}; -- not a comment :nope');`,
    );
  }
  const sql = rows.join('\n') + '\nSELECT * FROM players WHERE id = :id;';
  parity(sql, 'sql');
  assert.deepEqual(analyzeStatement(sql), { count: 201, vars: ['id'] });
});

test('analyzeStatement matches on randomized buffers (differential)', () => {
  const alphabet = [
    "'", '"', '`', '\\', ';', ';', ':', ':', '{', '{', '}', '}', '-', '#', '/', '*',
    '\n', '\n', ' ', '\t', '\r', 'a', 'b', '_', 'x', '1', '$', ' ', '﻿', 'Z', '$', 'E',
  ];
  let seed = 20260927;
  const rnd = () => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff;
  for (let t = 0; t < 20_000; t++) {
    const len = 1 + Math.floor(rnd() * 60);
    let s = '';
    for (let k = 0; k < len; k++) s += alphabet[Math.floor(rnd() * alphabet.length)];
    parity(s, 'sql');
    parity(s, 'line');
    parity(s, 'pg');
  }
});

test('stmtPreview bounds list rows and tooltips', () => {
  assert.equal(stmtPreview('SELECT  1\n  FROM t'), 'SELECT 1 FROM t');
  const huge = 'INSERT INTO t VALUES (1);\n'.repeat(10_000);
  const p = stmtPreview(huge, 300);
  assert.ok(p.length <= 301, `preview is bounded (${p.length})`);
  assert.ok(p.endsWith('…'));
  assert.ok(!p.includes('\n'));
  assert.equal(stmtPreview('x'.repeat(300), 300), 'x'.repeat(300));
});


test('Mongo script keywords tolerate whitespace and leading comments without matching document keys', () => {
  for (const script of [
    "do { print('42'); } while(false)",
    "switch(1) { case 1: print('42'); break; }",
    "for await (const x of [42]) { print(x); }",
    "if\t(true) { print('42'); }",
    "/* leading */ const x=42; print(x);",
    "/* multi\nline */ /* second */ do { print('42'); } while(false)",
    "async function answer() { return 42; }\nprint(await answer());",
  ]) assert.equal(looksLikeMongoshScript(script), true, script);
  for (const key of ['if', 'do', 'switch', 'const', 'async', 'print']) {
    const query = `db.c.find({\n ${key} /* key */ : 1\n})`;
    assert.equal(looksLikeMongoshScript(query), false, query);
  }
});


test('Mongo shell marker text in native literals and trailing comments stays native', () => {
  for (const query of [
    "db.c.find({note:'process.env'})",
    "db.c.find({note:'db.getSiblingDB('})",
    "db.c.find({}) // process.env db.getSiblingDB(",
  ]) assert.equal(looksLikeMongoshScript(query), false, query);
});
