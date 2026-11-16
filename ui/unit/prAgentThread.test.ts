// One Threads thread per pull request (pr-agent-thread.ts): the origin id and
// label, and how a PR finds its thread through the Threads session port —
// by origin first, then the id this browser remembered for an older daemon.
// Creating and sending go through the embed (EmbeddedRuntimeThread's draft
// state and handle), not this file.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadSource } from './sourceHarness.ts';

const load = () => loadSource(new URL('../src/modules/git/pr-agent-thread.ts', import.meta.url), {});

const row = (id: string) => ({ id, workspace_id: 'w', status: 'idle', last_active_at: '2026-11-12T00:00:00Z' });

function port(hit: unknown, rows: Record<string, unknown> = {}) {
  const calls: unknown[][] = [];
  return {
    calls,
    findByOrigin: async (...args: unknown[]) => { calls.push(['find', ...args]); return hit; },
    loadOne: async (id: string) => {
      calls.push(['loadOne', id]);
      if (!rows[id]) throw new Error('404');
      return rows[id];
    },
  };
}

test('origin id and label (≤ 200 characters)', () => {
  const mod = load();
  assert.equal(mod.prAgentOriginId('rep_otto', 51), 'rep_otto#51');
  assert.equal(mod.prAgentOriginLabel('otto', 51, 'Add  the VIP'), 'otto #51 Add the VIP');
  assert.equal(mod.prAgentOriginLabel('otto', 1, 'x'.repeat(400)).length, 200);
});

test('find asks the store for this PR’s origin, with the remembered id for an older daemon', async () => {
  const mod = load();
  const p = port(row('t1'));
  assert.equal((await mod.findPrThread(p, 'w', 'rep#5')).id, 't1');
  assert.deepEqual(p.calls[0], ['find', 'w', 'pr-agent', 'rep#5', null]);

  mod.rememberPrThread('rep#5', 'old');
  const q = port(null, { old: row('old') });
  assert.equal((await mod.findPrThread(q, 'w', 'rep#5')).id, 'old');
  assert.deepEqual(q.calls[0], ['find', 'w', 'pr-agent', 'rep#5', 'old']);
  assert.deepEqual(q.calls[1], ['loadOne', 'old']); // not listed (another workspace): read it directly
});

test('a remembered thread that is gone (404) is no thread, not an error', async () => {
  const mod = load();
  mod.rememberPrThread('rep#9', 'gone');
  assert.equal(await mod.findPrThread(port(null), 'w', 'rep#9'), null);
  // Nothing remembered and nothing found: no direct read at all.
  const p = port(null);
  assert.equal(await mod.findPrThread(p, 'w', 'rep#10'), null);
  assert.equal(p.calls.length, 1);
});

test('a new thread for the PR replaces the remembered one', async () => {
  const mod = load();
  mod.rememberPrThread('rep#7', 'first');
  mod.rememberPrThread('rep#7', 'second');
  const p = port(null, { second: row('second') });
  assert.equal((await mod.findPrThread(p, 'w', 'rep#7')).id, 'second');
});

test('a folded PR’s dot comes from its thread’s row: background work is calm, not working', () => {
  const mod = load();
  assert.equal(mod.prThreadStatus(null), null);
  assert.equal(mod.prThreadStatus({ status: 'working' }), 'working');
  assert.equal(mod.prThreadStatus({ status: 'starting' }), 'working');
  assert.equal(mod.prThreadStatus({ status: 'awaiting_permission', background_tasks: [{}] }), 'needs-you');
  assert.equal(mod.prThreadStatus({ status: 'idle', background_tasks: [{}] }), 'background');
  assert.equal(mod.prThreadStatus({ status: 'closed', background_tasks: [{}] }), 'closed');
  assert.equal(mod.prThreadStatus({ status: 'dormant' }), 'idle');
});
