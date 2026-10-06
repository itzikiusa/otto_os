import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync, readdirSync } from 'node:fs';
import { join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  quoteIdent,
  stringLiteral,
  boolLiteral,
  splitModeFor,
  backslashEscapes,
  qualifiedName,
  tableRefFromNodeId,
} from '../src/modules/database/sql-dialect.ts';
import { condToSql, toFilterVal, filterValMatches, parseFilterValText, type FilterCond } from '../src/modules/database/filter-chips.ts';
import {
  countStatements,
  statementAtCursor,
  extractVars,
  renderVar,
  defaultVarSpec,
} from '../src/modules/database/sql-util.ts';

// One dialect module drives identifiers, literals, booleans and the splitter
// for every engine. Postgres drifted in three separate places before it.

const eq = (column: string, value: unknown): FilterCond => ({ kind: 'col', column, op: 'in', values: [toFilterVal(value)] });

test('identifiers, strings and booleans per engine', () => {
  assert.equal(quoteIdent('postgres', 'a"b'), '"a""b"');
  assert.equal(quoteIdent('mysql', 'a`b'), '`a``b`');
  assert.equal(quoteIdent('clickhouse', 'c'), '`c`');
  assert.equal(stringLiteral('postgres', "C:\\it's"), "'C:\\it''s'");
  assert.equal(stringLiteral('mysql', "C:\\it's"), "'C:\\\\it''s'");
  assert.equal(stringLiteral('clickhouse', 'a\\'), "'a\\\\'");
  assert.equal(boolLiteral('postgres', true), 'TRUE');
  assert.equal(boolLiteral('mysql', false), '0');
  assert.equal(backslashEscapes('postgres'), false);
  assert.deepEqual((['postgres', 'mysql', 'clickhouse', 'mongodb', 'redis', null] as const).map((e) => splitModeFor(e)), ['pg', 'sql', 'sql', 'sql', 'line', 'sql']);
});

test('quick-filter chips render per engine (S16-05)', () => {
  assert.equal(condToSql(eq('status', 'paid'), 'postgres'), `"status" = 'paid'`);
  assert.equal(condToSql(eq('status', 'paid'), 'mysql'), "`status` = 'paid'");
  assert.equal(condToSql(eq('active', true), 'postgres'), '"active" = TRUE');
  assert.equal(condToSql(eq('active', false), 'mysql'), '`active` = 0');
  assert.equal(condToSql(eq('path', 'C:\\tmp'), 'postgres'), `"path" = 'C:\\tmp'`);
  assert.equal(condToSql(eq('path', 'C:\\tmp'), 'clickhouse'), "`path` = 'C:\\\\tmp'");
  const multi: FilterCond = {
    kind: 'col',
    column: 'n',
    op: 'not_in',
    values: [toFilterVal(1), toFilterVal(null), parseFilterValText('x')],
  };
  assert.equal(condToSql(multi, 'postgres'), `"n" NOT IN (1, 'x') AND "n" IS NOT NULL`);
  // Chip label (no engine): MySQL style, unchanged.
  assert.equal(condToSql(eq('a', 'b')), "`a` = 'b'");
});

test('boolean chips match boolean cells by value (S16-06)', () => {
  const text = (v: unknown) => String(v);
  const t = toFilterVal(true);
  assert.equal(filterValMatches(true, t, text), true);
  assert.equal(filterValMatches(false, t, text), false);
  assert.equal(filterValMatches(null, t, text), false);
  assert.equal(filterValMatches(true, toFilterVal(false), text), false);
  // Numbers / strings keep text matching; a 1 cell is not `true`.
  assert.equal(filterValMatches(1, t, text), false);
  assert.equal(filterValMatches(1, toFilterVal(1), text), true);
  assert.equal(filterValMatches('a', toFilterVal('a'), text), true);
  assert.equal(filterValMatches(null, toFilterVal(null), text), true);
});

test('string variables keep backslashes on Postgres (S16-07)', () => {
  const spec = defaultVarSpec('C:\\tmp');
  assert.equal(renderVar(spec, 'pg'), "'C:\\tmp'");
  assert.equal(renderVar(spec, 'sql'), "'C:\\\\tmp'");
  assert.equal(renderVar(defaultVarSpec("it's"), 'pg'), "'it''s'");
});

test('Postgres splitter: dollar quotes, # operator, standard strings (S16-14)', () => {
  const fn = 'CREATE FUNCTION f() RETURNS int AS $$ BEGIN x := 1; RETURN 2; END $$ LANGUAGE plpgsql;\nSELECT 2;';
  assert.equal(countStatements(fn, 'pg'), 2);
  assert.equal(countStatements(fn, 'sql'), 4);
  assert.equal(statementAtCursor(fn, fn.indexOf('RETURN'), 'pg'), fn.slice(0, fn.indexOf(';\n')));
  const tagged = 'DO $body$ BEGIN PERFORM 1; END $body$; SELECT $1, a$b$c';
  assert.equal(countStatements(tagged, 'pg'), 2);
  // `#` is XOR, not a comment: the `;` after it splits.
  assert.equal(countStatements('SELECT 5 # 3; SELECT 2', 'pg'), 2);
  assert.equal(countStatements('SELECT 5 # 3; SELECT 2', 'sql'), 1);
  // A standard string ending in `\` closes; E'…' honours the escape.
  assert.equal(countStatements("SELECT 'C:\\'; SELECT 2", 'pg'), 2);
  assert.equal(countStatements("SELECT E'it\\'s; x'; SELECT 2", 'pg'), 2);
  // Nested block comments.
  assert.equal(countStatements('/* a /* b; */ c; */ SELECT 1; SELECT 2', 'pg'), 2);
  // Variables inside a dollar-quoted body are not variables.
  assert.deepEqual(extractVars('SELECT $$ :nope $$, :yes', 'pg'), ['yes']);
});

test('widgets: obvious writes are flagged at save time (S16-08 UI hint)', async () => {
  const { obviousWriteVerb } = await import('../src/modules/database/sql-dialect.ts');
  assert.equal(obviousWriteVerb('UPDATE counters SET n=n+1'), 'UPDATE');
  assert.equal(obviousWriteVerb('-- tick\n  delete from sessions'), 'DELETE');
  assert.equal(obviousWriteVerb('/* x */ DROP TABLE t'), 'DROP');
  assert.equal(obviousWriteVerb('WITH d AS (DELETE FROM t RETURNING *) SELECT * FROM d'), 'DELETE');
  assert.equal(obviousWriteVerb('DEL k'), 'DEL');
  assert.equal(obviousWriteVerb('SELECT count(*) FROM orders'), null);
  assert.equal(obviousWriteVerb('GET k'), null);
  assert.equal(obviousWriteVerb('db.c.find({})'), null);
});

test('schema-tree table refs are quoted per engine (S16-301)', () => {
  // Postgres trees use `db:` for the schema — the ref must be `"public"."orders"`.
  assert.deepEqual(tableRefFromNodeId('postgres', 'db:public/table:orders'), {
    ref: '"public"."orders"',
    db: 'public',
    table: 'orders',
  });
  assert.equal(tableRefFromNodeId('mysql', 'db:shop/table:orders')?.ref, '`shop`.`orders`');
  assert.equal(tableRefFromNodeId('clickhouse', 'db:logs/view:hits')?.ref, '`logs`.`hits`');
  assert.equal(tableRefFromNodeId('postgres', 'table:we"ird')?.ref, '"we""ird"');
  assert.equal(tableRefFromNodeId('postgres', 'db:public'), null);
  assert.equal(qualifiedName('mysql', null, 't'), '`t`');
});

test('ClickHouse identifiers escape backslashes like the daemon (S16-305)', () => {
  // A name ending in `\` must not swallow the closing backtick.
  assert.equal(quoteIdent('clickhouse', 'a\\'), '`a\\\\`');
  assert.equal(quoteIdent('clickhouse', 'a`b'), '`a``b`');
  // MySQL backtick identifiers do NOT honour `\` escapes.
  assert.equal(quoteIdent('mysql', 'a\\'), '`a\\`');
  assert.equal(condToSql(eq('dir\\', 'x'), 'clickhouse'), "`dir\\\\` = 'x'");
  // NUL: dropped on Postgres, `\0` on the backslash dialects.
  assert.equal(stringLiteral('postgres', 'a\0b'), "'ab'");
  assert.equal(stringLiteral('mysql', 'a\0b'), "'a\\0b'");
});

// Guard: identifier / literal quoting has ONE home. A local quoter anywhere
// else under modules/database (or in the database store) is how Postgres
// drifted in S16-05 and S16-301 — import from `sql-dialect.ts` instead.
test('no local identifier/literal quoters outside sql-dialect.ts (S16-305)', () => {
  const root = fileURLToPath(new URL('../src/', import.meta.url));
  const files = [
    join(root, 'lib/stores/database.svelte.ts'),
    ...walk(join(root, 'modules/database')),
  ].filter((f) => !f.endsWith('sql-dialect.ts'));
  const QUOTERS = [
    // name.replace(/`/g, '``') — a hand-rolled backtick quoter
    /replace\(\s*\/`\/g\s*,\s*['"]``['"]\s*\)/,
    // name.replace(/"/g, '""') — a hand-rolled double-quote quoter
    /replace\(\s*\/"\/g\s*,\s*'""'\s*\)/,
    // s.replace(/'/g, "''") — a hand-rolled string-literal escaper
    /replace\(\s*\/'\/g\s*,\s*"''"\s*\)/,
  ];
  const hits: string[] = [];
  for (const f of files) {
    const lines = readFileSync(f, 'utf8').split('\n');
    lines.forEach((line, i) => {
      // `// not-sql:` marks a non-SQL quoter (CSV fields…) on purpose.
      if (QUOTERS.some((re) => re.test(line)) && !line.includes('not-sql:')) hits.push(`${relative(root, f)}:${i + 1}: ${line.trim()}`);
    });
  }
  assert.deepEqual(hits, [], `use quoteIdent / stringLiteral from sql-dialect.ts:\n${hits.join('\n')}`);
});

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) =>
    e.isDirectory() ? walk(join(dir, e.name)) : /\.(ts|svelte)$/.test(e.name) ? [join(dir, e.name)] : [],
  );
}
