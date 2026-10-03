import {test} from 'node:test';
import assert from 'node:assert/strict';
import {mergeTouched, touches} from '../src/lib/gitLivePaths.ts';

test('events union until a refresh takes them', () => {
  let t = mergeTouched(undefined, ['a.rs']);
  t = mergeTouched(t, ['b/c.rs', 'a.rs']);
  assert.deepEqual([...(t ?? [])].sort(), ['a.rs', 'b/c.rs']);
});

test('an event without paths makes the set unknown, and it stays unknown', () => {
  let t = mergeTouched(undefined, ['a.rs']);
  t = mergeTouched(t, undefined);
  assert.equal(t, null);
  assert.equal(mergeTouched(t, ['x']), null);
});

test('an empty list (remote refs moved) touches no file', () => {
  const t = mergeTouched(undefined, []);
  assert.equal(touches([...(t ?? [])], 'src/a.rs'), false);
});

test('a burst past the cap degrades to unknown', () => {
  const many = Array.from({length: 300}, (_, i) => `f${i}`);
  assert.equal(mergeTouched(undefined, many), null);
});

test('touches: exact file, parent directory, unknown', () => {
  assert.equal(touches(['src/a.rs'], 'src/a.rs'), true);
  assert.equal(touches(['src'], 'src/a.rs'), true);
  assert.equal(touches(['src/a'], 'src/ab.rs'), false);
  assert.equal(touches(['other.rs'], 'src/a.rs'), false);
  assert.equal(touches(null, 'src/a.rs'), true);
  assert.equal(touches(undefined, 'src/a.rs'), true);
});
