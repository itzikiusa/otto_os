// S17-308: whole-list settings writes must be sent one at a time, the newest
// carrying the latest state — ordering only the RESPONSES let the server apply
// an older list last.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { serialLatest } from '../src/lib/latest.ts';

test('writes never overlap and the last one sends the latest state', async () => {
  let state = ['a'];
  let inFlight = 0;
  let maxInFlight = 0;
  const sent: string[][] = [];
  const gates: Array<() => void> = [];
  const save = serialLatest(async () => {
    inFlight++;
    maxInFlight = Math.max(maxInFlight, inFlight);
    sent.push([...state]);
    await new Promise<void>((r) => gates.push(r));
    inFlight--;
    return [...state];
  });
  state = ['a', 'b'];
  const first = save();
  await new Promise((r) => setImmediate(r)); // the first PUT is in flight
  state = ['a', 'b', 'c'];
  const second = save();
  state = ['a', 'b', 'c', 'd'];
  const third = save();
  gates.shift()!();
  await new Promise((r) => setImmediate(r));
  gates.shift()?.();
  const results = await Promise.all([first, second, third]);
  assert.equal(maxInFlight, 1, 'never two writes at once');
  assert.deepEqual(sent, [['a', 'b'], ['a', 'b', 'c', 'd']], 'the queued middle write is skipped');
  assert.deepEqual(results.map((r) => r.ok), [true, false, true]);
});

test('a failed write does not wedge the queue', async () => {
  let fail = true;
  const save = serialLatest(async () => {
    if (fail) throw new Error('boom');
    return 'ok';
  });
  await assert.rejects(save(), /boom/);
  fail = false;
  assert.deepEqual(await save(), { ok: true, value: 'ok' });
});
