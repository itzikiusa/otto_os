import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import {
  EMBED_SCROLLBACK,
  FLOW_HIGH,
  FLOW_LOW,
  FLOW_PAUSE_KEEPALIVE_MS,
  PRIMARY_SCROLLBACK,
  RESYNC_ON_INPUT,
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

test('scrollback depths: 2k default for embeds, 10k only where a primary pane asks for it', () => {
  assert.equal(EMBED_SCROLLBACK, 2000);
  assert.equal(PRIMARY_SCROLLBACK, 10_000);
  const src = (p: string) => readFileSync(new URL(p, import.meta.url), 'utf8');
  assert.match(src('../src/lib/components/Terminal.svelte'), /scrollback = EMBED_SCROLLBACK \}: Props = \$props\(\)/);
  assert.match(src('../src/modules/agents/SessionView.svelte'), /scrollback = PRIMARY_SCROLLBACK \}: Props = \$props\(\)/);
  const tiled = src('../src/modules/agents/TiledView.svelte');
  assert.match(tiled, /const TILE_SCROLLBACK = EMBED_SCROLLBACK;/);
  assert.match(tiled, /scrollback=\{TILE_SCROLLBACK\}/);
});
