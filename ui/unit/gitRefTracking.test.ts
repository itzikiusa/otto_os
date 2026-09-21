import { test } from 'node:test';
import assert from 'node:assert/strict';
import { branchTracking } from '../src/modules/git/refTracking.ts';

test('inactive branches use their own upstream counts; current branch uses fresh status', () => {
  const branch = { name: 'feature', is_current: false, upstream: 'origin/feature', remote: false, ahead: 2, behind: 7 };
  const status = { branch: 'main', upstream: 'origin/main', ahead: 1, behind: 3, changes: [] };
  assert.deepEqual(branchTracking(branch, status), { ahead: 2, behind: 7 });
  assert.deepEqual(branchTracking({ ...branch, name: 'main', is_current: true }, status), { ahead: 1, behind: 3 });
  assert.deepEqual(branchTracking(undefined, status), { ahead: 0, behind: 0 });
  assert.deepEqual(branchTracking({ ...branch, ahead: undefined, behind: undefined }, status), { ahead: 0, behind: 0 });
});
