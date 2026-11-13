import { test } from 'node:test';
import assert from 'node:assert/strict';
import * as cadence from '../src/lib/cadence.ts';
import { loadSource } from './sourceHarness.ts';

// "Also started by scheduled task X": the second way a workflow is scheduled.
const helper = loadSource(new URL('../src/modules/workflows/scheduledBy.ts', import.meta.url), {
  '../../lib/cadence': cadence,
});
const plain = <T>(x: T): T => JSON.parse(JSON.stringify(x));

const task = (over: Record<string, unknown>) => ({
  id: 't1',
  workspace_id: 'w',
  name: 'Morning digest',
  kind: 'workflow',
  prompt: '',
  provider: 'claude',
  model: '',
  cwd: '',
  schedule: { cadence: 'daily', at: '09:00' },
  destination: { type: 'none' },
  enabled: true,
  timezone: 'Europe/London',
  workflow_id: 'wf-a',
  sandbox: 'none',
  max_retries: 0,
  notify_on_change: false,
  created_at: '',
  updated_at: '',
  ...over,
});

test('lists only the workflow-kind tasks that launch THIS workflow, in name order', () => {
  const rows = plain(
    helper.scheduledByTasks(
      [
        task({ id: 'b', name: 'Zeta nightly', schedule: { cadence: 'interval', every_min: 1440 }, timezone: 'UTC' }),
        task({ id: 'a' }),
        task({ id: 'other', workflow_id: 'wf-b' }),
        task({ id: 'prompt', kind: 'agent_prompt', workflow_id: 'wf-a' }),
        task({ id: 'none', workflow_id: null }),
      ],
      'wf-a',
    ),
  );
  assert.deepEqual(rows, [
    { id: 'a', name: 'Morning digest', cadence: 'Daily at 09:00 · Europe/London', route: 'scheduled-tasks/a' },
    { id: 'b', name: 'Zeta nightly', cadence: 'Every day', route: 'scheduled-tasks/b' },
  ]);
});

test('a paused task says so, and an empty workflow id matches nothing', () => {
  const [row] = plain(helper.scheduledByTasks([task({ enabled: false })], 'wf-a'));
  assert.equal(row.cadence, 'Daily at 09:00 · Europe/London · paused');
  assert.deepEqual(plain(helper.scheduledByTasks([task({ workflow_id: '' })], '')), []);
  assert.deepEqual(plain(helper.scheduledByTasks([], 'wf-a')), []);
});

test('only a person who can view scheduled tasks is told about them', () => {
  const seen: string[] = [];
  assert.equal(helper.canSeeScheduledBy((f: string, c: string) => { seen.push(`${f}:${c}`); return true; }), true);
  assert.deepEqual(seen, ['scheduled_tasks:view']);
  assert.equal(helper.canSeeScheduledBy(() => false), false);
});
