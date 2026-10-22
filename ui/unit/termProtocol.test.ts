// architecture-2-01: the `/ws/term` protocol is decoded ONCE (termProtocol.ts)
// for the live Terminal and its parked engine, and both rebuild snapshots by
// one rule. These pin the decoder and the rule without mounting xterm.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import * as b64 from '../src/lib/b64.ts';
import * as termFlow from '../src/lib/components/termFlow.ts';
import { loadSource } from './sourceHarness.ts';

const { decodeTermFrame, applySnapshot } = loadSource(new URL('../src/lib/components/termProtocol.ts', import.meta.url), {
  '../b64': b64,
  './termFlow': termFlow,
}, { ArrayBuffer, Uint8Array, JSON });

const text = (o: unknown) => ({ data: JSON.stringify(o) });
/** The decoder runs in a vm realm: compare its bytes as plain arrays. */
const dec = (ev: { data: unknown }, sock: object) => {
  const f = decodeTermFrame(ev, sock);
  return f && f.bytes ? { ...f, bytes: [...f.bytes] } : f && { ...f };
};
const bin = (...b: number[]) => ({ data: new Uint8Array(b).buffer });

test('a binary frame is live output unless a snapshot header is waiting for it', () => {
  const sock = {};
  assert.deepEqual(dec(bin(1, 2), sock), { type: 'output', bytes: [1, 2] });
  // perf 01 N3: the header records, then the NEXT binary frame is its payload…
  assert.equal(decodeTermFrame(text({ type: 'scrollback', binary: true, epoch: 7, len: 2 }), sock), null);
  assert.deepEqual(dec(bin(3, 4), sock), { type: 'snapshot', epoch: 7, bytes: [3, 4] });
  // …only that one: the frame after it is output again.
  assert.equal(decodeTermFrame(bin(5), sock).type, 'output');
});

test('a header is keyed by socket, so a park/adopt between header and payload still pairs them', () => {
  const a = {};
  const b = {};
  decodeTermFrame(text({ type: 'scrollback', binary: true, epoch: 1 }), a);
  assert.equal(decodeTermFrame(bin(9), b).type, 'output', 'another socket is unaffected');
  assert.equal(decodeTermFrame(bin(9), a).type, 'snapshot');
});

test('control frames decode to typed frames; malformed and unknown ones to null', () => {
  const s = {};
  assert.deepEqual(dec(text({ type: 'scrollback', epoch: 3, data: b64.bytesToBase64(new Uint8Array([65])) }), s), {
    type: 'snapshot', epoch: 3, bytes: [65],
  });
  assert.deepEqual(dec(text({ type: 'scrollback' }), s), { type: 'snapshot', epoch: null, bytes: null });
  assert.deepEqual(dec(text({ type: 'credit', window: 4096 }), s), { type: 'credit', window: 4096 });
  assert.deepEqual(dec(text({ type: 'credit', window: 'x' }), s), { type: 'credit', window: undefined });
  assert.deepEqual(dec(text({ type: 'exit' }), s), { type: 'exit', code: 0 });
  assert.deepEqual(dec(text({ type: 'error', code: 'gone' }), s), { type: 'error', code: 'gone', message: '' });
  assert.deepEqual(dec(text({ type: 'status', status: 'idle' }), s), { type: 'status', status: 'idle' });
  const grid = { type: 'grid', cols: 80, rows: 24, owner: 'other' };
  assert.deepEqual(dec(text(grid), s), grid);
  assert.equal(decodeTermFrame({ data: '{not json' }, s), null);
  assert.equal(decodeTermFrame(text({ type: 'pong' }), s), null);
  assert.equal(decodeTermFrame(text(null), s), null);
  assert.equal(decodeTermFrame({ data: 42 }, s), null);
});

function target(inflight: number) {
  const log: string[] = [];
  const t = {
    begin: (epoch: number | null) => log.push(`begin ${epoch}`),
    writes: { dropQueued: () => log.push('drop'), get inflight() { return inflight; } },
    clearResync: () => log.push('resync-cleared'),
    reset: () => log.push('reset'),
    paint: (b: Uint8Array) => log.push(`paint ${[...b].join(',')}`),
  };
  return { t, log };
}

test('applySnapshot: an empty snapshot only moves the epoch', () => {
  const { t, log } = target(0);
  applySnapshot(t, 5, null);
  applySnapshot(t, 6, new Uint8Array());
  assert.deepEqual(log, ['begin 5', 'begin 6']);
});

test('applySnapshot: idle writer resets then paints; a busy one resets in order (RIS)', () => {
  const idle = target(0);
  applySnapshot(idle.t, 1, new Uint8Array([65]));
  assert.deepEqual(idle.log, ['begin 1', 'drop', 'resync-cleared', 'reset', 'paint 65']);
  const busy = target(2);
  applySnapshot(busy.t, 2, new Uint8Array([65]));
  assert.deepEqual(busy.log, ['begin 2', 'drop', 'resync-cleared', `paint ${[...termFlow.withInOrderReset(new Uint8Array([65]))].join(',')}`]);
});

test('Terminal.svelte: both engines decode through termProtocol, not a private copy', () => {
  const src = readFileSync(new URL('../src/lib/components/Terminal.svelte', import.meta.url), 'utf8');
  assert.equal((src.match(/decodeTermFrame\(ev, /g) ?? []).length, 2, 'the live and the parked handler');
  assert.doesNotMatch(src, /JSON\.parse\(ev\.data\)/, 'no hand-rolled frame decode');
  assert.doesNotMatch(src, /binarySnapHeaders/, 'the header handshake lives in termProtocol');
  assert.equal((src.match(/applySnapshot\(snapTarget, /g) ?? []).length, 2, 'one snapshot rule for both');
});
