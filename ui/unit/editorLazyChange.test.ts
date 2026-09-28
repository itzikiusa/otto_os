// Shared CodeEditor lazy-onchange contract (perf SB-08 / DB5-06) and the
// API builder's split variable scan.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createChangeEmitter } from '../src/lib/components/changeEmitter.ts';
import { requestTexts, requestVarNames, varNames } from '../src/lib/api/apiVars.ts';

function harness(threshold = 100) {
  let doc = '';
  let reads = 0;
  const emitted: string[] = [];
  const timers = new Map<number, () => void>();
  let nextTimer = 1;
  const e = createChangeEmitter({
    threshold,
    delayMs: 150,
    read: () => {
      reads++;
      return doc;
    },
    emit: (v) => emitted.push(v),
    setTimer: (fn) => {
      const id = nextTimer++;
      timers.set(id, fn);
      return id;
    },
    clearTimer: (h) => timers.delete(h as number),
  });
  const edit = (next: string) => {
    doc = next;
    e.changed(next.length);
  };
  const fire = () => {
    const due = [...timers.values()];
    timers.clear();
    for (const fn of due) fn();
  };
  return { e, edit, fire, emitted, timers, reads: () => reads };
}

test('small docs emit synchronously on every edit (unchanged contract)', () => {
  const h = harness();
  h.edit('a');
  h.edit('ab');
  assert.deepEqual(h.emitted, ['a', 'ab']);
  assert.equal(h.e.pending, false);
  assert.equal(h.timers.size, 0);
});

test('a burst of edits on a large doc reads and emits the text once', () => {
  const h = harness();
  const big = 'x'.repeat(200);
  for (let i = 0; i < 50; i++) h.edit(big + i);
  assert.deepEqual(h.emitted, [], 'nothing emitted mid-burst');
  assert.equal(h.reads(), 0, 'the doc is not copied per keystroke');
  assert.equal(h.e.pending, true);
  assert.equal(h.timers.size, 1, 'one live timer, re-armed per edit');
  h.fire();
  assert.deepEqual(h.emitted, [big + 49]);
  assert.equal(h.reads(), 1);
  assert.equal(h.e.pending, false);
});

test('flush delivers the latest text now; a later timer is a no-op', () => {
  const h = harness();
  h.edit('y'.repeat(150));
  h.edit('y'.repeat(151));
  h.e.flush();
  assert.deepEqual(h.emitted, ['y'.repeat(151)]);
  assert.equal(h.timers.size, 0);
  h.fire();
  h.e.flush();
  assert.equal(h.emitted.length, 1, 'no duplicate emit');
});

test('cancel drops a pending emit (doc switched under a stale binding)', () => {
  const h = harness();
  h.edit('z'.repeat(150));
  h.e.cancel();
  h.fire();
  h.e.flush();
  assert.deepEqual(h.emitted, []);
});

test('shrinking below the threshold emits synchronously and supersedes the pending one', () => {
  const h = harness();
  h.edit('q'.repeat(150));
  h.edit('q');
  assert.deepEqual(h.emitted, ['q']);
  h.fire();
  assert.deepEqual(h.emitted, ['q']);
});

test('requestVarNames keeps varNames(requestTexts) order with body names supplied separately', () => {
  const d = {
    url: 'https://{{host}}/x',
    headers: [{ key: 'X-A', value: '{{tok}}', enabled: true }, { key: 'Off', value: '{{off}}', enabled: false }],
    query: [{ key: 'q', value: '{{q}}', enabled: true }],
    body: '{"a":"{{bodyVar}}","b":"{{host}}"}',
    auth: { type: 'bearer', token: '{{authTok}}' },
  };
  const full = varNames(...requestTexts(d));
  assert.deepEqual(requestVarNames(d, varNames(d.body)), full);
  assert.deepEqual(full, ['host', 'q', 'tok', 'bodyVar', 'authTok']);
  // A deferred (stale/empty) body scan only drops the body's own names.
  assert.deepEqual(requestVarNames(d, []), ['host', 'q', 'tok', 'authTok']);
});
