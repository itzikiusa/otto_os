import { test } from 'node:test';
import assert from 'node:assert/strict';
import { computeRunStats, EMPTY_AGENT_STATS } from '../src/modules/swarm/runStats.ts';
import type { SwarmRun } from '../src/modules/swarm/types.ts';

const run = (id: string, agent: string, status: string, at: string, task: string | null = null): SwarmRun =>
  ({ id, agent_id: agent, status, enqueued_at: at, task_id: task }) as unknown as SwarmRun;

test('one pass yields per-agent live/done/active and swarm totals', () => {
  const s = computeRunStats([
    run('r1', 'a', 'running', '2026-09-01T10:00:00Z', 't1'),
    run('r2', 'a', 'queued', '2026-09-01T11:00:00Z', 't2'),
    run('r3', 'a', 'done', '2026-09-01T09:00:00Z', 't0'),
    run('r4', 'b', 'waiting', '2026-09-01T08:00:00Z'),
    run('r5', 'b', 'error', '2026-09-01T12:00:00Z', 't9'),
  ]);
  assert.equal(s.queued, 1);
  assert.equal(s.running, 2, 'running + waiting');
  const a = s.byAgent.get('a')!;
  assert.equal(a.live, 1);
  assert.equal(a.done, 1);
  assert.equal(a.active?.id, 'r2', 'newest live run (queued counts as live)');
  assert.equal(s.byAgent.get('b')!.active?.id, 'r4');
  assert.deepEqual([...s.liveTaskIds].sort(), ['t1', 't2']);
});

test('no runs → empty stats', () => {
  const s = computeRunStats([]);
  assert.equal(s.byAgent.size, 0);
  assert.equal(s.queued + s.running, 0);
  assert.deepEqual(EMPTY_AGENT_STATS, { live: 0, active: null, done: 0 });
});
