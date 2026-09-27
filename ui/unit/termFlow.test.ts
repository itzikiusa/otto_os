import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  FLOW_HIGH,
  FLOW_LOW,
  FLOW_PAUSE_KEEPALIVE_MS,
  TermFlow,
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
