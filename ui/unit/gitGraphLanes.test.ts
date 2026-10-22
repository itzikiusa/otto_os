// The commit-graph lane layout (modules/git/graph-lanes.ts), split out of
// GraphView.svelte: columns, gutter segments, lane names, and the resumable
// memo a "load more" page continues from instead of re-laying every row.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { layoutLanes, MAX_LANES, PALETTE } from '../src/modules/git/graph-lanes.ts';
import type { CommitInfo } from '../src/lib/api/types';

const c = (sha: string, parents: string[] = []): CommitInfo => ({
  sha,
  short_sha: sha.slice(0, 7),
  author: 'dev',
  date: '2026-10-01T00:00:00Z',
  subject: sha,
  parents,
  refs: [],
});

test('a linear history stays in column 0 with no stub above the tip or below the root', () => {
  const out = layoutLanes([c('c', ['b']), c('b', ['a']), c('a')], new Map(), null);
  assert.deepEqual(out.rows.map((r) => r.col), [0, 0, 0]);
  assert.deepEqual(out.rows.map((r) => [r.hasAbove, r.hasBelow]), [[false, true], [true, true], [true, false]]);
  assert.equal(out.widest, 1);
  assert.equal(out.laidOut, 3);
});

test('a merge opens a second lane that converges back onto the shared parent', () => {
  // m merges f (feature) into the main line; both descend from a.
  const out = layoutLanes([c('m', ['a', 'f']), c('f', ['a']), c('a')], new Map([['f', 'feature']]), null);
  const [m, f, a] = out.rows;
  assert.equal(m.col, 0);
  assert.ok(m.lines.some((l) => l.kind === 'merge-in' && l.fromCol === 0 && l.toCol === 1));
  assert.equal(f.col, 1);
  assert.equal(f.branch, 'feature', 'the lane is named from its tip ref');
  assert.equal(a.col, 0);
  assert.ok(a.lines.some((l) => l.kind === 'converge' && l.fromCol === 1 && l.toCol === 0));
  assert.equal(out.widest, 2);
  assert.notEqual(m.color, f.color, 'two live lanes never share a colour');
  assert.ok(PALETTE.includes(m.color) && PALETTE.includes(f.color));
});

test('an appended page resumes from the memo and matches a from-scratch pass', () => {
  const page1 = [c('e', ['d', 'x']), c('x', ['d']), c('d', ['b'])];
  const page2 = [c('b', ['a']), c('a')];
  const all = page1.concat(page2);
  const names = new Map<string, string>();
  const first = layoutLanes(page1, names, null);
  // `concat` shares the objects, exactly as GraphView appends pages.
  const resumed = layoutLanes(all, names, first.memo);
  assert.equal(resumed.laidOut, page2.length, 'only the new rows are laid out');
  const fresh = layoutLanes(all, names, null);
  assert.deepEqual(
    resumed.rows.map((r) => [r.commit.sha, r.col, r.hasAbove, r.hasBelow, r.lines.length]),
    fresh.rows.map((r) => [r.commit.sha, r.col, r.hasAbove, r.hasBelow, r.lines.length]),
  );
  assert.equal(resumed.widest, fresh.widest);
});

test('a changed name map lays out from scratch', () => {
  const list = [c('b', ['a']), c('a')];
  const first = layoutLanes(list, new Map(), null);
  const renamed = layoutLanes(list, new Map([['b', 'main']]), first.memo);
  assert.equal(renamed.laidOut, 2);
  assert.equal(renamed.rows[1].branch, 'main', 'the name carries down the lane');
});

test('an octopus fan-out past MAX_LANES stays capped', () => {
  const parents = Array.from({ length: MAX_LANES + 10 }, (_, i) => `p${i}`);
  const out = layoutLanes([c('octo', parents), ...parents.map((p) => c(p))], new Map(), null);
  assert.ok(out.widest <= MAX_LANES, `widest ${out.widest}`);
});

test('no commits lays out nothing and keeps no memo', () => {
  const out = layoutLanes([], new Map(), null);
  assert.deepEqual(out, { rows: [], widest: 1, memo: null, laidOut: 0 });
});
