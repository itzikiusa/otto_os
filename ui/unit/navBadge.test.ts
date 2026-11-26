// The sidebar / bottom-nav badge (lib/navBadge.ts) for Workflows: runs that
// wait on a person (needs you, blocked, out of budget) show as a 'needs' count;
// otherwise the running count as 'working'; a paused run counts as neither.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

function badgeFor(states: (string | null)[]) {
  const activeWorkflowRuns = states.map((s, i) => ({ run_id: `r${i}`, ...(s ? { overview: { state: s } } : {}) }));
  const { navBadge } = loadSource(new URL('../src/lib/navBadge.ts', import.meta.url), {
    './stores/workspace.svelte': { ws: { activeWorkflowRuns } },
  });
  return navBadge('workflows');
}

test('a run waiting on you outranks the running ones: count of needs-you runs, warning tone', () => {
  const b = badgeFor(['running', 'needs_you', 'blocked', 'exhausted', 'waiting']);
  assert.equal(b.count, 3);
  assert.equal(b.tone, 'needs');
  assert.equal(b.title, '3 waiting on you');
});

test('nothing waits on you: the running count, calm; a paused run is not running', () => {
  const b = badgeFor(['running', 'queued', 'paused', null]);
  assert.equal(b.count, 3, 'an older daemon without overview still counts as running');
  assert.equal(b.tone, 'working');
});

test('no active runs, or only paused ones: no badge', () => {
  assert.equal(badgeFor([]), null);
  assert.equal(badgeFor(['paused']), null);
});
