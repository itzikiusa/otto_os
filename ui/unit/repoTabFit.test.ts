// Git page repo tabs (modules/git/repoTabFit.ts): branches show inline only
// while the measured strip has room; the active tab keeps its branch when that
// alone fits; the branch is always in the label.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { branchMode, repoTabLabel } from '../src/modules/git/repoTabFit.ts';

const tab = (full: number, branch: number) => ({ full, branch });

test('everything fits: every tab shows its branch', () => {
  assert.equal(branchMode([tab(200, 90), tab(180, 70)], 400, 0), 'all');
  assert.equal(branchMode([tab(200, 90), tab(180, 70)], 380, 1), 'all');
});

test('overflowing: only the active tab keeps its branch when that alone fits', () => {
  // 3 tabs = 600 with branches, 330 without; room 450 → bare + active branch (100) = 430 fits.
  const tabs = [tab(200, 90), tab(220, 100), tab(180, 80)];
  assert.equal(branchMode(tabs, 450, 1), 'active');
  // Room for the bare names but not the active branch too.
  assert.equal(branchMode(tabs, 400, 1), 'none');
  // No active tab, or the active one has no branch: name-only.
  assert.equal(branchMode(tabs, 450, -1), 'none');
  assert.equal(branchMode([tab(200, 0), tab(300, 120)], 400, 0), 'none');
});

test('it is measured, not counted: many short tabs keep branches, two long ones do not', () => {
  const many = Array.from({ length: 10 }, () => tab(60, 20));
  assert.equal(branchMode(many, 700, 3), 'all');
  assert.equal(branchMode([tab(500, 300), tab(500, 300)], 700, 0), 'active');
});

test('label: repo — branch, or the name alone', () => {
  assert.equal(repoTabLabel('promotions', 'release/5.02.46'), 'promotions — release/5.02.46');
  assert.equal(repoTabLabel('promotions', null), 'promotions');
});
