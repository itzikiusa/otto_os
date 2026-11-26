// Insights hands off to the Goal form; nothing starts until Start goal (GL13.1) —
// insights/actionHandoff.ts stages, workflows/goal/goalDraft.ts takes.
import { test, beforeEach } from 'node:test';
import assert from 'node:assert/strict';
import { buildAutomationDraft, stageAutomationDraft } from '../src/modules/insights/actionHandoff.ts';
import { takeStagedGoalDraft } from '../src/modules/workflows/goal/goalDraft.ts';

const store = new Map<string, string>();
(globalThis as { sessionStorage?: unknown }).sessionStorage = {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
  removeItem: (k: string) => void store.delete(k),
};
beforeEach(() => store.clear());

const action = { title: 'Cut review wait', metric: 'wait', current: '40m', target: '10m', effort: 'S' as const, ids: ['A1'] };

test('a goal draft is taken once', () => {
  stageAutomationDraft(buildAutomationDraft('goal', action, 'this week'));
  const d = takeStagedGoalDraft();
  assert.equal(d?.title, 'Cut review wait');
  assert.match(d!.prompt, /Metric: wait \(40m → 10m\)/);
  assert.equal(takeStagedGoalDraft(), null);
});

test('a Scheduled Task draft is left for its own page', () => {
  stageAutomationDraft(buildAutomationDraft('scheduled_task', action));
  assert.equal(takeStagedGoalDraft(), null);
  assert.ok(store.size === 1);
});
