import { test } from 'node:test';
import assert from 'node:assert/strict';
import { canCompleteOperation } from '../src/modules/git/operationState.ts';

test('a reopened operation with no unmerged files can continue', () => {
  assert.equal(canCompleteOperation('rebase', [], new Set()), true);
  assert.equal(canCompleteOperation('merge', [], new Set()), true);
  assert.equal(canCompleteOperation(null, [], new Set()), false);
  assert.equal(canCompleteOperation('merge', ['a'], new Set()), false);
  assert.equal(canCompleteOperation(null, ['a'], new Set(['a'])), true);
});
