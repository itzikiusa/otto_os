// The PR agent thread's messages (pr-agent-context.ts): every ask carries the
// PR's context, file / line asks inline their diff within the named caps, and
// only Fix may edit files.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const ctx = loadSource(new URL('../src/modules/git/pr-agent-context.ts', import.meta.url), {}, {}, { relative: true });
const pick = loadSource(new URL('../src/modules/git/pr-agent-diff-pick.ts', import.meta.url), {}, {}, { relative: true });

const pr = {
  number: 51,
  title: 'Add the VIP program',
  url: 'https://github.com/dev-otto/otto/pull/51',
  source_branch: 'feat/vip',
  target_branch: 'main',
  head_sha: 'abc1234',
};
const repo = { name: 'otto', slug: 'dev-otto/otto', path: '/Users/dev/otto' };
const file = (path: string, n: number) => ({
  path,
  old_path: null,
  is_binary: false,
  hunks: [{ header: '@@ -1,1 +1,1 @@', lines: Array.from({ length: n }, (_, i) => ({ origin: 'add', content: `line ${i} ${'x'.repeat(40)}`, old_line: null, new_line: i + 1 })) }],
});

test('access: questions run in the most restrictive mode, Fix may edit', () => {
  for (const k of ['explain', 'question', 'files', 'lines']) assert.equal(ctx.accessModeFor(k), 'approval-required');
  assert.equal(ctx.accessModeFor('fix'), 'auto-accept-edits');
});

test('every message names the repo folder, the PR, its branches and head', () => {
  for (const ask of [{ kind: 'explain' }, { kind: 'question', text: 'Why?' }, { kind: 'files', paths: ['a.ts'] }]) {
    const m = ctx.buildAskMessage(ask, pr, repo, []);
    assert.match(m.text, /cloned at \/Users\/dev\/otto/);
    assert.match(m.text, /#51 “Add the VIP program”, feat\/vip → main/);
    assert.match(m.text, /git show abc1234:<path>/);
    assert.match(m.text, /read-only request/);
    assert.equal(m.title, 'PR #51 · Add the VIP program');
  }
  const q = ctx.buildAskMessage({ kind: 'question', text: 'Is the cache safe?' }, pr, repo, []);
  assert.match(q.text, /answer my question:\n\nIs the cache safe\?/);
});

test('Fix says it edits only the clone and never commits, pushes or posts', () => {
  const m = ctx.buildAskMessage({ kind: 'fix' }, pr, repo, []);
  assert.equal(m.access, 'auto-accept-edits');
  assert.match(m.text, /You may edit files in \/Users\/dev\/otto only/);
  assert.match(m.text, /Do not commit, push, or post/);
  assert.doesNotMatch(m.text, /read-only request/);
});

test('files: diffs inline up to the per-file and total caps; the rest are named', () => {
  const paths = Array.from({ length: 10 }, (_, i) => `src/f${i}.ts`);
  const diffs = paths.map((p) => file(p, 200)); // ~10 KB each
  const m = ctx.buildAskMessage({ kind: 'files', paths }, pr, repo, diffs);
  const blocks = m.text.match(/```diff/g) ?? [];
  assert.ok(blocks.length >= 1 && blocks.length <= ctx.INLINE_CAP.files);
  assert.ok(m.text.length < ctx.INLINE_CAP.totalChars + 6_000, `message too long: ${m.text.length}`);
  assert.match(m.text, /more characters cut/);
  assert.match(m.text, /Not inlined/);
  for (const p of paths) assert.ok(m.text.includes(`- ${p}`), p);
  // A file without loaded hunks is named, never pretended.
  const bare = ctx.buildAskMessage({ kind: 'files', paths: ['x.ts'] }, pr, repo, []);
  assert.match(bare.text, /Not inlined .*every file/);
});

test('lines: the label, range and capped rows', () => {
  const p = ctx.linePickFrom('src/a.ts', [
    { sign: ' ', oldLine: 9, newLine: 10, code: 'keep()' },
    { sign: '-', oldLine: 10, newLine: null, code: 'old()' },
    { sign: '+', oldLine: null, newLine: 11, code: 'new()' },
  ]);
  assert.deepEqual(JSON.parse(JSON.stringify(p)), { path: 'src/a.ts', start: 10, end: 11, side: 'new', text: ' keep()\n-old()\n+new()' });
  assert.equal(ctx.lineLabel(p), 'lines 10–11');
  assert.equal(ctx.lineLabel({ start: 3, end: 3, side: 'old' }), 'old line 3');
  const deleted = ctx.linePickFrom('b.ts', [{ sign: '-', oldLine: 4, newLine: null, code: 'x' }]);
  assert.equal(deleted.side, 'old');
  const big = { ...p, text: 'y'.repeat(ctx.INLINE_CAP.lineChars * 2) };
  const m = ctx.buildAskMessage({ kind: 'lines', pick: big }, pr, repo, []);
  assert.match(m.text, /explain lines 10–11 of src\/a\.ts/);
  assert.match(m.text, /on disk at \/Users\/dev\/otto\/src\/a\.ts/);
  assert.match(m.text, /more characters cut/);
  assert.ok(m.text.length < ctx.INLINE_CAP.lineChars + 3_000);
});

test('capText leaves short text alone and names the cut', () => {
  assert.equal(ctx.capText('abc', 10), 'abc');
  const cut = ctx.capText('a\n'.repeat(100), 50);
  // Cut at the last line break before the cap (index 49), not mid-line.
  assert.ok(cut.startsWith('a\n'.repeat(24) + 'a\n…'));
  assert.match(cut, /151 more characters cut/);
});

test('repo slug from the remote, and the diff row keys the line picker reads', () => {
  assert.equal(ctx.repoSlugOf({ name: 'otto', remote_url: 'git@github.com:dev-otto/otto.git' }), 'dev-otto/otto');
  assert.equal(ctx.repoSlugOf({ name: 'local', remote_url: null }), 'local');
  assert.deepEqual(JSON.parse(JSON.stringify(pick.parseRowKey('src/a b.ts\u001fl0:3'))), { path: 'src/a b.ts', kind: 'l' });
  assert.equal(pick.parseRowKey('no-separator'), null);
  assert.equal(pick.diffSign('−'), '-');
  assert.equal(pick.diffSign('+'), '+');
  assert.equal(pick.diffSign(''), ' ');
});
