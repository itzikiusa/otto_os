import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  bulkResultText,
  rejectAllMessage,
  turnOffAllMessage,
} from '../src/modules/settings/selfImprovementBulk.ts';

const summary = { workspaces: 18, enabled: 3, pending: 23, pending_workspaces: 5 };

test('the reject dialog names counts and leaves applied edits alone', () => {
  const m = rejectAllMessage(summary);
  assert.match(m, /^Reject 23 pending suggestions in 5 workspaces\?/);
  assert.match(m, /already applied stay applied/);
  assert.match(rejectAllMessage({ ...summary, pending: 1, pending_workspaces: 1 }), /1 pending suggestion in 1 workspace\?/);
});

test('the turn-off dialog counts only enabled workspaces', () => {
  assert.match(turnOffAllMessage(summary), /^Turn off self-improvement in 3 workspaces\?/);
});

test('result text mentions skipped workspaces and the role they need', () => {
  assert.equal(
    bulkResultText({ changed: 4, workspaces: 2, skipped: 0 }, 'reject'),
    '4 suggestions rejected in 2 workspaces.',
  );
  assert.equal(
    bulkResultText({ changed: 1, workspaces: 1, skipped: 2 }, 'off'),
    'Turned off in 1 workspace. 2 workspaces skipped (needs Admin).',
  );
  assert.match(bulkResultText({ changed: 0, workspaces: 0, skipped: 1 }, 'reject'), /needs Editor/);
});
