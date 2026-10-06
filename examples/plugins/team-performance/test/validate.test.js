const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs'); const os = require('os'); const path = require('path');
const { Readable } = require('stream');
const V = require('../lib/validate.js');

test('projectKey / accountId', () => {
  assert.equal(V.projectKey('ABC'), 'ABC');
  for (const b of ['abc', 'A', '1AB', 'AB C', 'AB"', '', null]) assert.throws(() => V.projectKey(b));
  assert.equal(V.accountId('01ABC_x-9'), '01ABC_x-9');
  for (const b of ['a/b', '', 'x'.repeat(65), '../x']) assert.throws(() => V.accountId(b));
});

test('jqlString escapes quotes and backslashes', () => {
  assert.equal(V.jqlString('a"b\\c'), '"a\\"b\\\\c"');
  assert.throws(() => V.jqlString('a\nb'));
});

test('repoPath', () => {
  const d = fs.mkdtempSync(path.join(os.tmpdir(), 'rp-'));
  assert.throws(() => V.repoPath(d)); fs.mkdirSync(path.join(d, '.git'));
  assert.equal(V.repoPath(d), d);
  for (const b of ['rel/path', '-x', '/nonexistent/zz', '']) assert.throws(() => V.repoPath(b));
});

test('readBodyCapped caps size', async () => {
  const mk = (s) => Object.assign(Readable.from([Buffer.from(s)]), { headers: {} });
  assert.equal(await V.readBodyCapped(mk('hello'), 10), 'hello');
  await assert.rejects(V.readBodyCapped(mk('x'.repeat(20)), 10), (e) => e.status === 413);
});
