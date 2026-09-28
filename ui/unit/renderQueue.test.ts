// SD-23: the mermaid/D2 render queue stays serial but skips jobs whose caller
// superseded them before they reached the head (latest-wins at dequeue).

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createRenderQueue, isStaleResult, STALE } from '../src/modules/canvas/renderQueue.ts';

function deferred<T>() {
  let resolve!: (v: T) => void;
  const promise = new Promise<T>((r) => (resolve = r));
  return { promise, resolve };
}

test('a burst of edits renders only the first (running) and the newest job', async () => {
  const q = createRenderQueue();
  const gate = deferred<void>();
  const ran: number[] = [];
  let token = 0;
  const submit = () => {
    const mine = ++token;
    return q.run(async () => {
      ran.push(mine);
      if (mine === 1) await gate.promise;
      return `svg-${mine}`;
    }, () => mine !== token);
  };
  const first = submit();
  await new Promise((r) => setImmediate(r)); // job 1 reaches the head and starts
  const rest = Array.from({ length: 9 }, submit);
  gate.resolve();
  const out = await Promise.all([first, ...rest]);
  assert.deepEqual(ran, [1, 10], 'superseded jobs 2..9 never touch the renderer');
  // Job 1 was already running when it went stale: it finishes (callers
  // discard by token); 2..9 resolve STALE; 10 renders.
  assert.equal(out[0], 'svg-1');
  for (const r of out.slice(1, 9)) assert.ok(isStaleResult(r));
  assert.equal(out[9], 'svg-10');
});

test('jobs without isStale keep strict FIFO order', async () => {
  const q = createRenderQueue();
  const order: string[] = [];
  await Promise.all(['a', 'b', 'c'].map((k) => q.run(async () => { order.push(k); return k; })));
  assert.deepEqual(order, ['a', 'b', 'c']);
});

test('a failing job or a throwing isStale never wedges the chain', async () => {
  const q = createRenderQueue();
  const bad = q.run(async () => { throw new Error('boom'); });
  const weird = q.run(async () => 'ran', () => { throw new Error('predicate'); });
  const after = q.run(async () => 'ok');
  await assert.rejects(bad, /boom/);
  assert.equal(await weird, 'ran', 'a throwing predicate counts as not stale');
  assert.equal(await after, 'ok');
  assert.equal(STALE.stale, true);
});
