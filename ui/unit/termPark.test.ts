import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { PARK_BUDGET_BYTES, PARK_CAP, PARK_CELL_BYTES, PARK_SCROLLBACK, PARK_TTL_MS, TermPark } from '../src/lib/components/termPark.ts';
import { TermFlow, WriteQueue } from '../src/lib/components/termFlow.ts';
import type { WsTermFlowFrame } from '../src/lib/api/types.ts';

/** Fake timers: `advance(ms)` fires what came due. */
function timers() {
  let now = 0;
  let seq = 0;
  const live = new Map<number, { at: number; fn: () => void }>();
  const setT = (fn: () => void, ms: number) => {
    live.set(++seq, { at: now + ms, fn });
    return seq as unknown as ReturnType<typeof setTimeout>;
  };
  const clearT = (id: ReturnType<typeof setTimeout>) => void live.delete(id as unknown as number);
  const advance = (ms: number) => {
    now += ms;
    for (const [id, t] of [...live]) if (t.at <= now) {
      live.delete(id);
      t.fn();
    }
  };
  return { setT, clearT, advance, live: () => live.size };
}

test('park: take returns the parked value once and cancels its TTL', () => {
  const t = timers();
  const disposed: string[] = [];
  const park = new TermPark<string>((v) => disposed.push(v), 3, 1000, t.setT, t.clearT);
  park.put('s1', 'engine-1');
  assert.equal(park.size, 1);
  assert.equal(park.take('s1'), 'engine-1');
  assert.equal(park.take('s1'), null, 'the adopter owns it now');
  assert.equal(t.live(), 0, 'TTL timer cleared on adopt');
  t.advance(5000);
  assert.deepEqual(disposed, [], 'an adopted engine is never disposed by the lot');
});

test('park: LRU cap evicts (and disposes) the oldest parked engine', () => {
  const t = timers();
  const disposed: string[] = [];
  const park = new TermPark<string>((v) => disposed.push(v), 3, 60_000, t.setT, t.clearT);
  for (const k of ['a', 'b', 'c', 'd', 'e']) park.put(k, `eng-${k}`);
  assert.equal(park.size, 3);
  assert.deepEqual(disposed, ['eng-a', 'eng-b']);
  assert.equal(park.has('a'), false);
  assert.equal(park.take('e'), 'eng-e');
  assert.equal(t.live(), 2, 'one TTL timer per parked entry, none leaked');
});

test('park: TTL disposes an engine nobody came back for', () => {
  const t = timers();
  const disposed: string[] = [];
  const park = new TermPark<string>((v) => disposed.push(v), 3, 1000, t.setT, t.clearT);
  park.put('s', 'eng');
  t.advance(999);
  assert.equal(park.has('s'), true);
  t.advance(1);
  assert.equal(park.has('s'), false);
  assert.deepEqual(disposed, ['eng']);
});

test('park: re-parking a session disposes the older engine; a stale close cannot evict the newer one', () => {
  const t = timers();
  const disposed: string[] = [];
  const park = new TermPark<string>((v) => disposed.push(v), 3, 60_000, t.setT, t.clearT);
  park.put('s', 'old');
  park.put('s', 'new');
  assert.deepEqual(disposed, ['old']);
  park.evict('s', 'old'); // the old socket's late onclose
  assert.equal(park.take('s'), 'new');
});

test('park: bounds are the documented ones', () => {
  assert.equal(PARK_CAP, 12);
  assert.equal(PARK_TTL_MS, 5 * 60 * 1000, 'no longer than the daemon idle-suspend grace');
  assert.equal(PARK_SCROLLBACK, 4000, 'the daemon emulator depth (otto-pty EMULATOR_SCROLLBACK_LINES)');
  const pty = readFileSync(new URL('../../crates/otto-pty/src/lib.rs', import.meta.url), 'utf8');
  assert.match(pty, /pub const EMULATOR_SCROLLBACK_LINES: usize = 4000;/);
});

test('park → adopt keeps the credit stream: acks continue on the same tag through re-pointed sinks', () => {
  const lot: WsTermFlowFrame[] = [];
  const adopter: WsTermFlowFrame[] = [];
  const flow = new TermFlow((f) => adopter.push(f), () => 0);
  const inside: { n: number; done: () => void }[] = [];
  const q = new WriteQueue((b, done) => inside.push({ n: b.byteLength, done }), flow);
  flow.granted(1024 * 1024);
  const tag = flow.credit;
  q.push(new Uint8Array(64 * 1024), undefined, tag);
  // Park: the lot takes over the sinks while a slice is still inside xterm.
  flow.setSink((f) => lot.push(f));
  const parked: { n: number; done: () => void }[] = [];
  q.rebind((b, done) => parked.push({ n: b.byteLength, done }), () => true);
  inside.shift()!.done(); // the in-flight slice settles under the lot
  q.push(new Uint8Array(64 * 1024), undefined, tag);
  parked.shift()!.done();
  // Adopt: sinks move again; nothing restarts.
  flow.setSink((f) => adopter.push(f));
  q.rebind((b, done) => inside.push({ n: b.byteLength, done }), () => true);
  q.push(new Uint8Array(64 * 1024), undefined, tag);
  inside.shift()!.done();
  const acks = (fs: WsTermFlowFrame[]) => fs.filter((f) => f.type === 'ack').map((f) => (f as { bytes: number }).bytes);
  assert.deepEqual(acks(lot), [64 * 1024, 128 * 1024], 'acks while parked go through the lot');
  assert.deepEqual(acks(adopter), [192 * 1024], 'the adopter continues the cumulative count');
  assert.equal(flow.credit, tag, 'same credit stream');
  assert.equal(flow.pending, 0);
});

test('park: a byte budget evicts the oldest engines past it, keeping the newest (perf 01 N2)', () => {
  const t = timers();
  const disposed: string[] = [];
  const size = new Map<string, number>();
  const park = new TermPark<string>((v) => disposed.push(v), 12, 60_000, t.setT, t.clearT, (v) => size.get(v) ?? 0, 100);
  for (const [k, n] of [['a', 30], ['b', 30], ['c', 30]] as const) {
    size.set(k, n);
    park.put(k, k);
  }
  assert.equal(park.bytes, 90);
  assert.deepEqual(disposed, [], 'within budget: nothing goes');
  // A parked engine grows while it keeps parsing; the next put re-measures.
  size.set('b', 50);
  size.set('d', 30);
  park.put('d', 'd');
  // 30 + 50 + 30 + 30 = 140: `a` goes (110), still over → `b` goes (60).
  assert.deepEqual(disposed, ['a', 'b'], 'over budget → the least recently parked go first');
  assert.equal(park.bytes, 60);
  // One engine over the whole budget still parks (alone).
  size.set('huge', 500);
  park.put('huge', 'huge');
  assert.equal(park.size, 1);
  assert.equal(park.has('huge'), true);
});

test('park: the default budget holds ~5 worst-case (4000 × 200) engines, all PARK_CAP typical ones', () => {
  const worst = PARK_SCROLLBACK * 200 * PARK_CELL_BYTES;
  assert.equal(Math.floor(PARK_BUDGET_BYTES / worst), 5);
  const typical = 1500 * 120 * PARK_CELL_BYTES;
  assert.ok(typical * PARK_CAP <= PARK_BUDGET_BYTES, 'typical sessions fill every slot');
  const t = timers();
  const disposed: string[] = [];
  const park = new TermPark<string>((v) => disposed.push(v), PARK_CAP, PARK_TTL_MS, t.setT, t.clearT, () => worst);
  for (let i = 0; i < PARK_CAP; i++) park.put(`s${i}`, `s${i}`);
  assert.equal(park.size, 5);
  assert.ok(park.bytes <= PARK_BUDGET_BYTES);
  assert.deepEqual(disposed, ['s0', 's1', 's2', 's3', 's4', 's5', 's6']);
});

