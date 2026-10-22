// Pure ref helpers split out of GraphView.svelte (modules/git/ref-menus.ts):
// branch folder grouping, the collapsed-row ref menu ordering and the
// middle-ellipsis of long branch names.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  groupBranches,
  midParts,
  primaryChip,
  shouldCollapseRow,
  splitChips,
  withoutRemoteHeads,
  type RefChip,
} from '../src/modules/git/ref-menus.ts';
import type { RefBranch, RefsResp } from '../src/lib/api/types';

const br = (name: string, remote = false): RefBranch => ({ name, remote, is_current: false, sha: 'abc' }) as RefBranch;

test('branches group by first path segment; remote refs drop the remote name', () => {
  const g = groupBranches([br('main'), br('feature/b'), br('feature/a'), br('origin/release/1', true)]);
  assert.deepEqual(g.loose.map((l) => l.label), ['main']);
  assert.deepEqual(
    g.folders.map((f) => [f.name, f.leaves.map((l) => l.label)]),
    [['feature', ['b', 'a']], ['release', ['1']]],
  );
  assert.equal(g.folders[0].leaves[0].b.name, 'feature/b', 'the full ref stays for actions');
});

test('the origin/HEAD symref (shortened to a bare remote name) is dropped', () => {
  const r = { local: [], remote: [br('origin', true), br('origin/main', true)], tags: [] } as unknown as RefsResp;
  assert.deepEqual(withoutRemoteHeads(r).remote.map((b) => b.name), ['origin/main']);
});

test('a collapsed row keeps the checked-out branch visible and lists branches before tags', () => {
  const chips: RefChip[] = [
    { kind: 'tag', label: 'v1', current: false },
    { kind: 'remote', label: 'origin/x', current: false },
    { kind: 'local', label: 'x', current: false },
    { kind: 'head', label: 'main', current: true },
  ];
  assert.equal(shouldCollapseRow(chips), true);
  assert.equal(primaryChip(chips).label, 'main');
  const { branches, tags } = splitChips(chips);
  assert.deepEqual(branches.map((c) => c.label), ['main', 'x', 'origin/x']);
  assert.deepEqual(tags.map((c) => c.label), ['v1']);
  assert.equal(shouldCollapseRow([...chips, { kind: 'stash', label: 'stash', current: false }]), false);
});

test('long names keep a whole-word tail; short names render plain', () => {
  assert.equal(midParts('main'), null);
  const mp = midParts('PROJ-1234-really-long-branch-retry-fix')!;
  assert.equal(mp.head + mp.tail, 'PROJ-1234-really-long-branch-retry-fix');
  assert.ok('-_./'.includes(mp.head.at(-1)!), `tail starts at a word boundary: ${mp.tail}`);
});
