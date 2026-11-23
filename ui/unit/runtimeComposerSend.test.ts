// The composer's one primary action and the mid-turn delivery (composerSend.ts).
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { isAltSend, midTurnDelivery, primaryAction, primaryLabel } from '../src/modules/threads/runtime/composerSend.ts';

test('one primary action: Send idle, Stop while working with nothing typed, the mid-turn delivery with a draft', () => {
  const base = { pref: 'queue' as const, canSteer: true };
  assert.equal(primaryAction({ ...base, running: false, canSend: false }), 'send');
  assert.equal(primaryAction({ ...base, running: false, canSend: true }), 'send');
  assert.equal(primaryAction({ ...base, running: true, canSend: false }), 'stop');
  assert.equal(primaryAction({ ...base, running: true, canSend: true }), 'queue');
  assert.equal(primaryAction({ ...base, pref: 'steer', running: true, canSend: true }), 'steer');
  // An agent that can't steer only ever queues.
  assert.equal(primaryAction({ pref: 'steer', canSteer: false, running: true, canSend: true }), 'queue');
});

test('⌘↵ picks the other delivery; steering an agent that cannot is refused into a queue', () => {
  assert.deepEqual(midTurnDelivery('queue', false, true), { delivery: 'queue', refused: false });
  assert.deepEqual(midTurnDelivery('queue', true, true), { delivery: 'steer', refused: false });
  assert.deepEqual(midTurnDelivery('steer', false, true), { delivery: 'steer', refused: false });
  assert.deepEqual(midTurnDelivery('steer', true, true), { delivery: 'queue', refused: false });
  assert.deepEqual(midTurnDelivery('queue', true, false), { delivery: 'queue', refused: true });
  assert.deepEqual(midTurnDelivery('steer', false, false), { delivery: 'queue', refused: true });
});

test('⌘↵ on a Mac, Ctrl+↵ elsewhere; never on a touch-first device, never with Shift', () => {
  const k = (o: Partial<{ metaKey: boolean; ctrlKey: boolean; shiftKey: boolean }>) => ({ key: 'Enter', metaKey: false, ctrlKey: false, shiftKey: false, ...o });
  assert.equal(isAltSend(k({ metaKey: true }), true, false), true);
  assert.equal(isAltSend(k({ ctrlKey: true }), true, false), false);
  assert.equal(isAltSend(k({ ctrlKey: true }), false, false), true);
  assert.equal(isAltSend(k({}), true, false), false);
  assert.equal(isAltSend(k({ metaKey: true, shiftKey: true }), true, false), false);
  assert.equal(isAltSend(k({ metaKey: true }), true, true), false);
});

test('the button names what it will do, and why an agent only queues', () => {
  assert.equal(primaryLabel('send', { agent: 'Gemini', canSteer: false, mac: true }).label, 'Send message');
  const q = primaryLabel('queue', { agent: 'Claude', canSteer: true, mac: true });
  assert.equal(q.label, 'Queue — sends when this turn ends');
  assert.match(q.title, /⌘↵ steers/);
  assert.match(primaryLabel('queue', { agent: 'Gemini', canSteer: false, mac: false }).title, /Gemini can’t take a message mid-turn/);
  const s = primaryLabel('steer', { agent: 'Codex', canSteer: true, mac: false });
  assert.match(s.label, /^Steer/);
  assert.match(s.title, /Ctrl\+↵ queues/);
});
