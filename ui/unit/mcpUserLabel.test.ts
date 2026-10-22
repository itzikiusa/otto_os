import { test } from 'node:test';
import assert from 'node:assert/strict';
import { shortUserId, userLabel } from '../src/lib/userLabel.ts';

const ME = '01J9ZZZZZZZZZZZZZZZZZZME00';
const DANA = '01J9AAAAAAAAAAAAAAAAAADANA';
const members = [
  { user_id: DANA, display_name: 'Dana', username: 'dana' },
  { user_id: 'u-bare', display_name: '  ', username: 'bob' },
];

test('MCP approvals never show a raw id as the decider or requester', () => {
  assert.equal(userLabel(ME, ME, members), 'you');
  assert.equal(userLabel(DANA, ME, members), 'Dana');
  // A blank display name falls back to the username, not the id.
  assert.equal(userLabel('u-bare', ME, members), 'bob');
  // Not a member we can see: the id shortened, never the full ULID.
  const stranger = '01J9BBBBBBBBBBBBBBBBBBXYZW';
  assert.equal(userLabel(stranger, ME, members), '01J9…XYZW');
  assert.equal(shortUserId('short'), 'short');
  assert.equal(userLabel(null, ME, members), 'unknown');
  assert.equal(userLabel(undefined, null, [], '—'), '—');
});
