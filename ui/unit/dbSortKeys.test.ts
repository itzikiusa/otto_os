import { test } from 'node:test';
import assert from 'node:assert/strict';
import { compareIntStrings, exactIntTie } from '../src/modules/database/sort-keys.ts';

// Ids beyond 2^53 arrive as exact digit strings; Number() makes neighbours
// equal, so the grid sort breaks such ties on the digits.
test('integer digit strings order by sign, length, then digits', () => {
  assert.equal(Number('9007199254740993'), Number('9007199254740992'), 'the doubles tie');
  assert.equal(compareIntStrings('9007199254740993', '9007199254740992'), 1);
  assert.equal(compareIntStrings('18446744073709551615', '9223372036854775807'), 1);
  assert.equal(compareIntStrings('-9223372036854775808', '-9223372036854775807'), -1);
  assert.equal(compareIntStrings('-1', '1'), -1);
  assert.equal(compareIntStrings('007', '7'), 0);
  assert.equal(compareIntStrings('12', '12'), 0);
});

test('exactIntTie only orders two exact integers', () => {
  assert.equal(exactIntTie('9007199254740993', '9007199254740992'), 1);
  assert.equal(exactIntTie(9007199254740993n, '9007199254740994'), -1);
  assert.equal(exactIntTie('1.5', '1.5'), 0);
  assert.equal(exactIntTie(5, 5), 0, 'plain numbers are already ordered by value');
  assert.equal(exactIntTie('abc', '12'), 0);
});
