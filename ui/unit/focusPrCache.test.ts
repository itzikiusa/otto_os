// Focus's one-minute PR cache: a different set of linked repos is a miss, a
// read where every repo failed is never kept, and opening or merging a PR
// drops it (R1-030).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { freshPrs, invalidateFocusPrs, savePrs } from '../src/modules/git/focusPrHelpers.ts';
import type { Repo } from '../src/lib/api/types';

const repo = (id: string, acc: string | null): Repo => ({ id, git_account_id: acc, provider: 'github' }) as unknown as Repo;
const a = [repo('a', 'acc1')];

test('a kept read answers only for the same linked repos', () => {
  savePrs(a, [], 0, new Set());
  assert.ok(freshPrs(a), 'same candidates: a hit');
  assert.equal(freshPrs([...a, repo('b', 'acc1')]), null, 'an account linked to another repo: a miss');
  assert.equal(freshPrs([repo('a', 'acc2')]), null, 'the repo relinked to another account: a miss');
  assert.equal(freshPrs([]), null, 'the repo removed: a miss');
});

test('a read where every repo failed is not kept', () => {
  savePrs(a, [], 0, new Set());
  savePrs(a, [], 2, new Set());
  assert.equal(freshPrs(a), null);
});

test('opening or merging a PR drops the kept read', () => {
  savePrs(a, [], 0, new Set());
  invalidateFocusPrs();
  assert.equal(freshPrs(a), null);
});
