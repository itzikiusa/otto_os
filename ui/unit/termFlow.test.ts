import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import type { WsTermFlowFrame } from '../src/lib/api/types.ts';
import {
  CREDIT_ACK_STEP,
  CREDIT_WINDOW,
  EMBED_SCROLLBACK,
  FLOW_HIGH,
  FLOW_LOW,
  FLOW_PAUSE_KEEPALIVE_MS,
  PRIMARY_SCROLLBACK,
  QuietRepaint,
  RESYNC_ON_INPUT,
  TUI_CLEANUP_MAX_WAIT_MS,
  TUI_CLEANUP_QUIET_MS,
  TermFlow,
  WRITE_INFLIGHT,
  WRITE_SLICE,
  WriteQueue,
  hasCursorOrErase,
  withInOrderReset,
} from '../src/lib/components/termFlow.ts';

const enc = (s: string) => new TextEncoder().encode(s);

function harness() {
  const sent: string[] = [];
  let clock = 0;
  const flow = new TermFlow((f) => sent.push(f.type), () => clock);
  return { flow, sent, tick: (ms: number) => (clock += ms) };
}

test('pauses once above HIGH and resumes once below LOW', () => {
  const { flow, sent } = harness();
  const chunk = 64 * 1024;
  let written = 0;
  while (written <= FLOW_HIGH) {
    flow.add(chunk, true);
    written += chunk;
  }
  assert.deepEqual(sent, ['pause']);
  assert.equal(flow.paused, true);
  // More frames already in flight don't re-send pause (no keep-alive due yet).
  flow.add(chunk, true);
  written += chunk;
  assert.deepEqual(sent, ['pause']);
  // Drain down: nothing until pending < LOW, then exactly one resume.
  while (flow.pending >= FLOW_LOW + chunk) {
    flow.done(chunk);
    written -= chunk;
  }
  assert.deepEqual(sent, ['pause']);
  flow.done(chunk);
  flow.done(chunk);
  assert.deepEqual(sent, ['pause', 'resume']);
  assert.equal(flow.paused, false);
});

test('normal output below HIGH never sends flow frames', () => {
  const { flow, sent } = harness();
  for (let i = 0; i < 1000; i++) {
    flow.add(4096, true);
    flow.done(4096);
  }
  assert.deepEqual(sent, []);
  assert.equal(flow.pending, 0);
});

test('a slow drain re-sends pause as a keep-alive (server auto-resumes after 2 s)', () => {
  const { flow, sent, tick } = harness();
  flow.add(FLOW_HIGH + 1, true);
  assert.deepEqual(sent, ['pause']);
  tick(FLOW_PAUSE_KEEPALIVE_MS + 1);
  flow.done(1024); // still far above LOW
  assert.deepEqual(sent, ['pause', 'pause']);
  tick(10);
  flow.done(1024);
  assert.deepEqual(sent, ['pause', 'pause'], 'keep-alive is throttled');
});

test('no pause is sent while the socket is closed; a new stream starts unpaused', () => {
  const { flow, sent } = harness();
  flow.add(FLOW_HIGH + 1, false);
  assert.deepEqual(sent, []);
  flow.add(1, true);
  assert.deepEqual(sent, ['pause']);
  flow.resetStream(); // reconnect
  assert.equal(flow.paused, false);
  assert.ok(flow.pending > FLOW_HIGH, 'the local xterm backlog survives a reconnect');
  flow.add(1, true);
  assert.deepEqual(sent, ['pause', 'pause'], 'the new stream is paused again');
  flow.done(flow.pending);
  assert.equal(flow.pending, 0);
  assert.deepEqual(sent, ['pause', 'pause', 'resume']);
});

test('hasCursorOrErase: redraws yes, plain appended output no', () => {
  // Plain output, typing echo, SGR colours, CR/LF: dirty-row paint suffices.
  assert.equal(hasCursorOrErase(enc('hello world\r\n')), false);
  assert.equal(hasCursorOrErase(enc('\x1b[31mred\x1b[0m\r\n')), false);
  assert.equal(hasCursorOrErase(enc('\x1b[1;32mok\x1b[m')), false);
  assert.equal(hasCursorOrErase(enc('\x1b]0;title\x07')), false);
  // Readline ↑-history / prompt rewrites, TUIs, progress bars.
  assert.equal(hasCursorOrErase(enc('\r\x1b[Kls -la')), true);
  assert.equal(hasCursorOrErase(enc('\x1b[2J\x1b[H')), true);
  assert.equal(hasCursorOrErase(enc('\x1b[12;1H')), true);
  assert.equal(hasCursorOrErase(enc('\x1b[3A')), true);
  assert.equal(hasCursorOrErase(enc('\x1b[?25l\x1b[5G')), true);
  assert.equal(hasCursorOrErase(enc('\x1bM')), true);
  assert.equal(hasCursorOrErase(enc('\x1b8')), true);
  // Truncated / bare ESC at the end is not a redraw.
  assert.equal(hasCursorOrErase(enc('abc\x1b')), false);
  assert.equal(hasCursorOrErase(enc('abc\x1b[12')), false);
});

test('withInOrderReset prefixes RIS and keeps the snapshot bytes', () => {
  const out = withInOrderReset(enc('snap'));
  assert.deepEqual([...out.slice(0, 2)], [0x1b, 0x63]);
  assert.equal(new TextDecoder().decode(out.slice(2)), 'snap');
});

// ── A3: write queue in front of xterm ─────────────────────────────────────

/** A fake emulator: records handed slices; `parse()` completes the oldest. */
function queueHarness() {
  const frames: string[] = [];
  const flow = new TermFlow((f) => frames.push(f.type), () => 0);
  const inside: { bytes: Uint8Array; done: () => void }[] = [];
  const handed: number[] = [];
  const q = new WriteQueue((bytes, done) => {
    handed.push(bytes.byteLength);
    inside.push({ bytes, done });
  }, flow);
  const parse = (): boolean => {
    const next = inside.shift();
    next?.done();
    return !!next;
  };
  return { q, flow, frames, inside, handed, parse };
}

test('WriteQueue slices ≤ WRITE_SLICE, keeps ≤ WRITE_INFLIGHT inside xterm, and loses nothing', () => {
  const { q, flow, inside, handed, parse } = queueHarness();
  const big = new Uint8Array(1_000_000).map((_, i) => i % 251);
  let parsedCalls = 0;
  q.push(big, () => parsedCalls++);
  q.push(enc('tail'), () => parsedCalls++);
  assert.equal(flow.pending, big.byteLength + 4, 'every byte counted on arrival');
  assert.ok(q.inflight <= WRITE_INFLIGHT);
  const out: number[] = [];
  let maxInside = 0;
  while (inside.length) {
    maxInside = Math.max(maxInside, q.inflight);
    for (const b of inside[0].bytes) out.push(b);
    parse();
  }
  assert.ok(handed.every((n) => n <= WRITE_SLICE), 'no slice above 64 KB');
  assert.ok(maxInside <= WRITE_INFLIGHT, 'never more than 128 KB inside xterm');
  assert.equal(out.length, big.byteLength + 4, 'all bytes delivered, in order');
  assert.deepEqual(out.slice(0, 5), [0, 1, 2, 3, 4]);
  assert.equal(new TextDecoder().decode(new Uint8Array(out.slice(-4))), 'tail');
  assert.equal(parsedCalls, 2, 'each frame hook fires once, after its last slice');
  assert.equal(flow.pending, 0);
  assert.equal(q.backlog, 0);
});

test('WriteQueue: xterm throwing un-counts the slice and keeps draining', () => {
  const frames: string[] = [];
  const flow = new TermFlow((f) => frames.push(f.type), () => 0);
  let calls = 0;
  const q = new WriteQueue(() => {
    calls++;
    throw new Error('discard watermark');
  }, flow);
  q.push(new Uint8Array(3 * WRITE_SLICE));
  assert.equal(calls, 3);
  assert.equal(flow.pending, 0);
  assert.equal(q.backlog, 0);
});

test('input over a big queue sends resync BEFORE the resume its drop triggers, and drops only the queue', () => {
  const { q, flow, frames, parse } = queueHarness();
  // Flood past HIGH so the stream is paused, like a real `cat`.
  while (flow.pending <= FLOW_HIGH) q.push(new Uint8Array(64 * 1024));
  assert.deepEqual(frames, ['pause']);
  const inside = q.inflight;
  assert.ok(q.queued > RESYNC_ON_INPUT);
  const resynced = q.resyncOnInput(() => frames.push('resync'));
  assert.equal(resynced, true);
  assert.deepEqual(frames, ['pause', 'resync', 'resume'], 'resync first, then the un-pause');
  assert.equal(q.queued, 0, 'queue dropped');
  assert.equal(q.inflight, inside, 'bytes inside xterm are untouched (they still parse)');
  assert.equal(flow.pending, inside);
  while (parse());
  assert.equal(flow.pending, 0);
  // A small queue is left alone: it drains in a few ms anyway.
  q.push(new Uint8Array(WRITE_INFLIGHT + 1024));
  assert.equal(q.resyncOnInput(() => frames.push('resync')), false);
  assert.equal(q.queued, 1024);
});

test('scrollback depths: 2k default for embeds, 4000 (the daemon depth) only where a primary pane asks for it', () => {
  assert.equal(EMBED_SCROLLBACK, 2000);
  // perf 01 N2: never more than the daemon emulator keeps (otto-pty
  // EMULATOR_SCROLLBACK_LINES) — rows past it cannot survive a snapshot.
  assert.equal(PRIMARY_SCROLLBACK, 4000);
  assert.match(
    readFileSync(new URL('../../crates/otto-pty/src/lib.rs', import.meta.url), 'utf8'),
    /pub const EMULATOR_SCROLLBACK_LINES: usize = 4000;/,
  );
  const src = (p: string) => readFileSync(new URL(p, import.meta.url), 'utf8');
  // The default in the props destructure (later props may follow it).
  assert.match(src('../src/lib/components/Terminal.svelte'), /scrollback = EMBED_SCROLLBACK(, \w+ = [^,}]+)* \}: Props = \$props\(\)/);
  assert.match(src('../src/modules/agents/SessionView.svelte'), /scrollback = PRIMARY_SCROLLBACK(, \w+ = [^,}]+)* \}: Props = \$props\(\)/);
  const tiled = src('../src/modules/agents/TiledView.svelte');
  assert.match(tiled, /const TILE_SCROLLBACK = EMBED_SCROLLBACK;/);
  assert.match(tiled, /scrollback=\{TILE_SCROLLBACK\}/);
});

// ── Credit flow control (docs/contracts/ws.md §1 "Credit") ────────────────

function creditHarness() {
  const sent: WsTermFlowFrame[] = [];
  const flow = new TermFlow((f) => sent.push(f), () => 0);
  const inside: { n: number; done: () => void }[] = [];
  const q = new WriteQueue((bytes, done) => inside.push({ n: bytes.byteLength, done }), flow);
  const parse = (): number => {
    const next = inside.shift();
    next?.done();
    return next?.n ?? 0;
  };
  const acks = () => sent.filter((f) => f.type === 'ack').map((f) => (f as { bytes: number }).bytes);
  return { flow, q, sent, parse, acks, inside };
}

test('credit: offer, grant, then no pause frames however big the backlog', () => {
  const { flow, q, sent } = creditHarness();
  flow.offer();
  assert.deepEqual(sent, [{ type: 'credit', window: CREDIT_WINDOW }]);
  flow.granted();
  assert.ok(flow.credit > 0);
  for (let i = 0; i < 64; i++) q.push(new Uint8Array(64 * 1024), undefined, flow.credit);
  assert.ok(flow.pending > FLOW_HIGH);
  assert.deepEqual(sent.map((f) => f.type), ['credit'], 'the daemon bounds the stream; no pause/resume');
});

test('credit: binary snapshots are offered only on request (perf 01 N3: never to the room relay)', () => {
  const direct = creditHarness();
  direct.flow.offer(true);
  assert.deepEqual(direct.sent, [{ type: 'credit', window: CREDIT_WINDOW, binary_snapshots: true }]);
  const relay = creditHarness();
  relay.flow.offer(false);
  assert.deepEqual(relay.sent, [{ type: 'credit', window: CREDIT_WINDOW }], 'no unknown field for deny_unknown_fields');
  // Terminal.svelte: only a direct socket (no socketFactory) asks for it, and a
  // binary payload after a header is applied as the snapshot, never credited.
  const src = readFileSync(new URL('../src/lib/components/Terminal.svelte', import.meta.url), 'utf8');
  assert.match(src, /flow\.offer\(!socketFactory\)/);
  assert.match(src, /const hdr = binarySnapHeaders\.get\(s\);\s*if \(hdr\) \{\s*binarySnapHeaders\.delete\(s\);\s*applySnapshot\(/);
});

test('credit: acks are cumulative, every CREDIT_ACK_STEP consumed, credited bytes only', () => {
  const { flow, q, parse, acks } = creditHarness();
  flow.granted();
  // Small echo frames: nothing until a whole step was consumed (the daemon
  // stalls only at a full window, so an unreported remainder never blocks).
  for (let i = 0; i < 100; i++) {
    q.push(new Uint8Array(100), undefined, flow.credit);
    parse();
  }
  assert.deepEqual(acks(), []);
  // Snapshots / local writes (stream 0) are never acked.
  q.push(new Uint8Array(3 * CREDIT_ACK_STEP));
  while (parse());
  assert.deepEqual(acks(), []);
  q.push(new Uint8Array(CREDIT_ACK_STEP), undefined, flow.credit);
  while (parse());
  assert.deepEqual(acks(), [100 * 100 + CREDIT_ACK_STEP]);
  q.push(new Uint8Array(2 * CREDIT_ACK_STEP), undefined, flow.credit);
  while (parse());
  assert.deepEqual(acks(), [10_000 + CREDIT_ACK_STEP, 10_000 + 2 * CREDIT_ACK_STEP, 10_000 + 3 * CREDIT_ACK_STEP]);
  assert.equal(flow.pending, 0);
});

test('credit: a small granted window acks every window/4 (never waits on a remainder)', () => {
  const { flow, q, parse, acks } = creditHarness();
  flow.granted(64 * 1024);
  q.push(new Uint8Array(40 * 1024), undefined, flow.credit);
  while (parse());
  assert.deepEqual(acks(), [40 * 1024], 'step is 16 KB for a 64 KB window');
});

test('credit: dropped bytes are acked (resync on input sends resync first)', () => {
  const log: string[] = [];
  const flow = new TermFlow((f) => log.push(f.type === 'ack' ? `ack:${f.bytes}` : f.type), () => 0);
  const q = new WriteQueue(() => {}, flow); // xterm never finishes: the queue builds
  flow.granted();
  for (let i = 0; i < 12; i++) q.push(new Uint8Array(64 * 1024), undefined, flow.credit);
  const queued = q.queued;
  assert.ok(queued > RESYNC_ON_INPUT);
  assert.equal(q.resyncOnInput(() => log.push('resync')), true);
  assert.deepEqual(log, ['resync', `ack:${queued}`], 'resync first; the whole dropped queue is released at once');
  assert.equal(flow.pending, q.inflight);
});

test('credit: a new socket reverts to pause mode and bytes from the old stream never ack', () => {
  const { flow, q, parse, acks, sent } = creditHarness();
  flow.granted();
  const old = flow.credit;
  q.push(new Uint8Array(2 * WRITE_SLICE), undefined, old);
  flow.resetStream(); // reconnect: still inside xterm, old tag
  assert.equal(flow.credit, 0);
  while (parse());
  assert.deepEqual(acks(), []);
  // Uncredited again: the watermark fallback is live (older daemon).
  flow.add(FLOW_HIGH + 1, true);
  assert.equal(sent.at(-1)?.type, 'pause');
  // A grant that lands while pause-mode paused un-pauses the daemon too.
  flow.granted();
  assert.notEqual(flow.credit, old, 'every grant is a new stream tag');
  assert.equal(flow.paused, false);
  assert.equal(sent.at(-1)?.type, 'resume');
});

/** The daemon's side, CreditGate-style: never more than `window` bytes
 *  unacknowledged. Whatever the producer's rate, the client backlog stays
 *  ≤ window and every byte arrives in order. */
test('credit: client backlog is bounded by the window independent of send rate', () => {
  for (const burst of [1, 2, 8, 64]) {
    const sent: WsTermFlowFrame[] = [];
    let acked = 0;
    const flow = new TermFlow((f) => {
      sent.push(f);
      if (f.type === 'ack') acked = f.bytes;
    }, () => 0);
    const inside: { bytes: Uint8Array; done: () => void }[] = [];
    const q = new WriteQueue((bytes, done) => inside.push({ bytes, done }), flow);
    flow.granted();
    const total = 8 * 1024 * 1024;
    let produced = 0;
    let serverSent = 0;
    let got = 0;
    let peak = 0;
    let order = 0;
    let inOrder = true;
    while (got < total) {
      // Producer: `burst` 64 KB frames per parse step, as far as credit allows.
      for (let i = 0; i < burst && produced < total; i++) produced += 64 * 1024;
      while (serverSent < produced && serverSent - acked < CREDIT_WINDOW) {
        const n = Math.min(produced - serverSent, CREDIT_WINDOW - (serverSent - acked));
        const frame = new Uint8Array(n).map((_, k) => (serverSent + k) % 251);
        serverSent += n;
        q.push(frame, undefined, flow.credit);
      }
      peak = Math.max(peak, flow.pending);
      const next = inside.shift();
      if (next) {
        for (const b of next.bytes) if (b !== order++ % 251) inOrder = false;
        got += next.bytes.byteLength;
        next.done();
      }
    }
    assert.ok(peak <= CREDIT_WINDOW, `burst ${burst}: peak ${peak}`);
    assert.ok(inOrder, `burst ${burst}: in order`);
    assert.ok(!sent.some((f) => f.type === 'pause'), `burst ${burst}: no pause`);
  }
});

// ── Agent-TUI ghost clean-up (r3-12-01) ─────────────────────────────────

/** Fake clock + timers: `advance(ms)` fires due timers in order. */
function fakeTimers() {
  let now = 0;
  let seq = 0;
  const timers = new Map<number, { at: number; fn: () => void }>();
  const setT = (fn: () => void, ms: number) => {
    const id = ++seq;
    timers.set(id, { at: now + ms, fn });
    return id as unknown as ReturnType<typeof setTimeout>;
  };
  const clearT = (id: ReturnType<typeof setTimeout>) => void timers.delete(id as unknown as number);
  const advance = (ms: number) => {
    const end = now + ms;
    for (;;) {
      let next: [number, { at: number; fn: () => void }] | null = null;
      for (const e of timers) if (e[1].at <= end && (!next || e[1].at < next[1].at)) next = e;
      if (!next) break;
      timers.delete(next[0]);
      now = next[1].at;
      next[1].fn();
    }
    now = end;
  };
  return { setT, clearT, advance, now: () => now, live: () => timers.size };
}

test('tui clean-up: one repaint after a burst goes quiet, not one per frame', () => {
  const t = fakeTimers();
  let runs = 0;
  const r = new QuietRepaint(() => runs++, TUI_CLEANUP_QUIET_MS, TUI_CLEANUP_MAX_WAIT_MS, t.now, t.setT, t.clearT);
  // A 10-frame burst 20 ms apart, then silence.
  for (let i = 0; i < 10; i++) {
    r.poke();
    t.advance(20);
  }
  assert.equal(runs, 0, 'nothing while the burst is still arriving');
  t.advance(TUI_CLEANUP_QUIET_MS);
  assert.equal(runs, 1, 'exactly one repaint once it went quiet');
  assert.equal(r.pending, false);
  assert.equal(t.live(), 0, 'no timer left behind');
  t.advance(5_000);
  assert.equal(runs, 1, 'idle panes never repaint');
});

test('tui clean-up: a never-quiet spinner is cleaned ~1×/s (was 5×/s)', () => {
  const t = fakeTimers();
  let runs = 0;
  const r = new QuietRepaint(() => runs++, TUI_CLEANUP_QUIET_MS, TUI_CLEANUP_MAX_WAIT_MS, t.now, t.setT, t.clearT);
  // A 10 Hz spinner for 10 s.
  for (let ms = 0; ms < 10_000; ms += 100) {
    r.poke();
    t.advance(100);
  }
  assert.ok(runs >= 9 && runs <= 11, `≈1 repaint per max-wait window, got ${runs}`);
  r.cancel();
  assert.equal(t.live(), 0);
});

// ── perf F9: parked / hidden panes withhold acks ───────────────────────────

test('hold: consumed bytes are not acked while held; release acks the total once', () => {
  const { flow, q, parse, acks } = creditHarness();
  flow.offer();
  flow.granted();
  flow.hold(true);
  for (let i = 0; i < 8; i++) q.push(new Uint8Array(CREDIT_ACK_STEP), undefined, flow.credit);
  while (parse() > 0) { /* drain */ }
  assert.deepEqual(acks(), [], 'a parked engine parses but reports nothing');
  flow.hold(false);
  assert.deepEqual(acks(), [8 * CREDIT_ACK_STEP], 'one cumulative ack on return');
  flow.hold(false);
  assert.deepEqual(acks(), [8 * CREDIT_ACK_STEP], 'releasing twice sends nothing more');
  // Normal stepping resumes.
  q.push(new Uint8Array(CREDIT_ACK_STEP), undefined, flow.credit);
  parse();
  assert.deepEqual(acks(), [8 * CREDIT_ACK_STEP, 9 * CREDIT_ACK_STEP]);
});

test('hold: release with nothing consumed, or before a grant, sends no ack', () => {
  const { flow, acks } = creditHarness();
  flow.hold(true);
  flow.hold(false);
  flow.offer();
  flow.granted();
  flow.hold(true);
  flow.hold(false);
  assert.deepEqual(acks(), []);
});

test('hold: a dropped queue while held is acked on release (the daemon owes one snapshot)', () => {
  const { flow, q, acks } = creditHarness();
  flow.offer();
  flow.granted();
  flow.hold(true);
  for (let i = 0; i < 8; i++) q.push(new Uint8Array(CREDIT_ACK_STEP), undefined, flow.credit);
  q.dropQueued();
  assert.deepEqual(acks(), []);
  flow.hold(false);
  assert.equal(acks().length, 1);
  assert.ok(acks()[0] >= 6 * CREDIT_ACK_STEP);
});
