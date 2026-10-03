import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseNoticeRoute } from '../src/lib/noticeRoute.ts';

test('automation notice routes resolve to the item to open', () => {
  assert.deepEqual(parseNoticeRoute('workflows/wf1/runs/r1'), { kind: 'workflow_run', workflowId: 'wf1', runId: 'r1' });
  assert.deepEqual(parseNoticeRoute('scheduled-tasks/t1/runs/r9'), { kind: 'scheduled_task', taskId: 't1', runId: 'r9' });
  assert.deepEqual(parseNoticeRoute('scheduled-tasks/t1'), { kind: 'scheduled_task', taskId: 't1', runId: null });
  assert.deepEqual(parseNoticeRoute('#/loops/l1'), { kind: 'goal_loop', loopId: 'l1' });
  assert.deepEqual(parseNoticeRoute('personal-agents/a1/runs'), { kind: 'route', route: 'personal-agents/a1/runs' });
  // A workflow route without a run just navigates.
  assert.deepEqual(parseNoticeRoute('workflows/wf1'), { kind: 'route', route: 'workflows/wf1' });
});
