// The approval buttons (runtime/approvalOptions.ts): one vocabulary, and the
// "always" decisions beside their once/deny pair only when offered.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { approvalButtons, approvalWord } from '../src/modules/threads/runtime/approvalOptions.ts';

const o = (decision: string, label = decision) => ({ decision, label }) as never;

test('ACP reject_always is offered as "Always deny" beside Deny', () => {
  const { primary, more } = approvalButtons([o('accept', 'Allow'), o('accept_always', 'Allow always'), o('decline', 'Reject'), o('decline_always', 'Reject always')]);
  assert.deepEqual(primary.map(approvalWord), ['Allow once', 'Always allow', 'Deny', 'Always deny']);
  assert.deepEqual(more, []);
});

test('without the always decisions there are no Always buttons; cancel goes under More', () => {
  const { primary, more } = approvalButtons([o('decline'), o('accept'), o('cancel')]);
  assert.deepEqual(primary.map(approvalWord), ['Allow once', 'Deny']);
  assert.deepEqual(more.map(approvalWord), ['Deny and stop']);
  assert.equal(approvalWord(o('accept_for_session')), 'Always allow');
  assert.equal(approvalWord(o('weird', 'Provider words')), 'Provider words');
});
