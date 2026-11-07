import { test } from 'node:test';
import assert from 'node:assert/strict';
import { reviewFixError } from '../src/modules/git/reviewFix.ts';
import type { Review } from '../src/lib/api/types/review.ts';
const reviewed = { status: 'done', pr_number: 4, scope_plan: { head_sha: 'abc' } } as Review;
test('only a completed PR with a recorded reviewed revision can start a fix goal', () => {
  assert.equal(reviewFixError(reviewed, 9.5), null);
  for (const status of ['running', 'error', 'cancelled'] as const) assert.ok(reviewFixError({ ...reviewed, status }, 9.5));
  assert.ok(reviewFixError({ ...reviewed, pr_number: 0 }, 9.5));
  assert.ok(reviewFixError({ ...reviewed, scope_plan: undefined }, 9.5));
});
test('target is finite and explicitly out of ten', () => {
  for (const target of [NaN, Infinity, -1, 0, 10.1, 95]) assert.ok(reviewFixError(reviewed, target));
  for (const target of [1, 9.5, 10]) assert.equal(reviewFixError(reviewed, target), null);
});
