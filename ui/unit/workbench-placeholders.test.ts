import { test } from 'node:test';
import assert from 'node:assert/strict';
import { detectPlaceholders, fillPlaceholders } from '../src/modules/workbench/lib/placeholders.ts';
import { extractVars } from '../src/modules/database/sql-util.ts';

test('sql: all three syntaxes, counts and first index', () => {
  const sql = 'SELECT * FROM t WHERE brand = :brand AND env = {env} AND x = {{x}} OR brand2 = :brand';
  const ps = detectPlaceholders(sql, 'sql');
  assert.deepEqual(
    ps.map((p) => [p.name, p.syntax, p.count]),
    [
      ['brand', ':name', 2],
      ['env', '{name}', 1],
      ['x', '{{name}}', 1],
    ],
  );
  assert.equal(ps[0].firstIndex, sql.indexOf(':brand'));
});

test('sql: skips literals, comments, casts and keys — same as the DB editor', () => {
  const sql = "SELECT ':no', '{no}' -- :nope {nope}\n/* {{nada}} */ , a::int, '12:30', b FROM t WHERE id = :id";
  assert.deepEqual(detectPlaceholders(sql, 'sql').map((p) => p.name), ['id']);
  assert.deepEqual(detectPlaceholders(sql, 'sql').map((p) => p.name), extractVars(sql));
});

test('non-sql: only {name} / {{name}}, including inside JSON strings', () => {
  const json = '{"url": "http://x:8080/a", "brand": "{{brand}}", "id": "{id}", "time": "12:30"}';
  assert.deepEqual(
    detectPlaceholders(json, 'json').map((p) => [p.name, p.syntax]),
    [
      ['brand', '{{name}}'],
      ['id', '{name}'],
    ],
  );
  assert.deepEqual(detectPlaceholders('key: :value', 'yaml'), []);
});

test('fill: substitutes known non-empty values, leaves the rest', () => {
  const sql = "SELECT * FROM t WHERE b = :brand AND e = {{env}} AND z = {zone} AND s = ':brand'";
  assert.equal(
    fillPlaceholders(sql, { brand: '7', env: "'prod'", zone: '' }, 'sql'),
    "SELECT * FROM t WHERE b = 7 AND e = 'prod' AND z = {zone} AND s = ':brand'",
  );
  assert.equal(fillPlaceholders('{"b":"{{b}}","c":"{c}"}', { b: 'x', c: 'y' }, 'json'), '{"b":"x","c":"y"}');
  assert.equal(fillPlaceholders('no vars', { a: '1' }), 'no vars');
});
