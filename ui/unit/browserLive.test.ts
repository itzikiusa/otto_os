import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
  clientToPage,
  fitContain,
  pageToBox,
  viewportChanged,
  viewportFor,
} from '../src/modules/browser/live/geometry.ts';
import {
  editingCommands,
  keyPayload,
  modifierMask,
  routeKey,
  virtualKeyCode,
  type KeyLike,
} from '../src/modules/browser/live/keys.ts';
import { coalesce, mergeWheel, wheelPixels, type Clock } from '../src/modules/browser/live/throttle.ts';
import {
  BACKOFF,
  INITIAL,
  STALE_MS,
  backoffDelay,
  isStale,
  isTerminalClose,
  meterFps,
  meterFrame,
  meterRtt,
  EMPTY_METER,
  reduce,
  type ConnState,
} from '../src/modules/browser/live/connection.ts';
import { buttonName, parseBinaryFrame, parseServer, safeCursor } from '../src/modules/browser/live/protocol.ts';

// ── geometry ────────────────────────────────────────────────────────────

test('fitContain letterboxes a wide frame top/bottom and centres it', () => {
  const f = fitContain({ width: 1600, height: 900 }, { width: 800, height: 600 });
  assert.equal(f.scale, 0.5);
  assert.deepEqual([f.x, f.y, f.width, f.height], [0, 75, 800, 450]);
});

test('fitContain pillarboxes a tall frame left/right', () => {
  const f = fitContain({ width: 400, height: 800 }, { width: 800, height: 400 });
  assert.equal(f.scale, 0.5);
  assert.deepEqual([f.x, f.y, f.width, f.height], [300, 0, 200, 400]);
});

test('fitContain with a degenerate size draws nothing', () => {
  assert.equal(fitContain({ width: 0, height: 10 }, { width: 100, height: 100 }).scale, 0);
  assert.equal(fitContain({ width: 10, height: 10 }, { width: 0, height: 100 }).scale, 0);
});

test('clientToPage maps through a 2x (retina) frame to CSS px', () => {
  // Panel 800×500 at client (100, 50). Remote viewport 800×500 CSS px, frame
  // encoded at dpr 2 → 1600×1000 image. Exact fit, no bars.
  const box = { left: 100, top: 50, width: 800, height: 500 };
  const image = { width: 1600, height: 1000 };
  const frame = { device_width: 800, device_height: 500 };
  assert.deepEqual(clientToPage({ x: 100, y: 50 }, box, image, frame), { x: 0, y: 0 });
  assert.deepEqual(clientToPage({ x: 500, y: 300 }, box, image, frame), { x: 400, y: 250 });
  assert.deepEqual(clientToPage({ x: 900, y: 550 }, box, image, frame), { x: 800, y: 500 });
});

test('clientToPage handles a downscaled frame (image smaller than the viewport)', () => {
  // Remote viewport 1280×800 CSS, frame capped to 640×400 image, shown in a
  // 640×400 panel at the origin: 1 client px = 2 remote CSS px.
  const box = { left: 0, top: 0, width: 640, height: 400 };
  const p = clientToPage({ x: 320, y: 100 }, box, { width: 640, height: 400 }, { device_width: 1280, device_height: 800 });
  assert.deepEqual(p, { x: 640, y: 200 });
});

test('clientToPage ignores clicks on the letterbox bars', () => {
  // 16:9 frame in a 4:3 panel → 75 px bars top and bottom.
  const box = { left: 0, top: 0, width: 800, height: 600 };
  const image = { width: 1600, height: 900 };
  const frame = { device_width: 1600, device_height: 900 };
  assert.equal(clientToPage({ x: 400, y: 10 }, box, image, frame), null);
  assert.equal(clientToPage({ x: 400, y: 590 }, box, image, frame), null);
  assert.deepEqual(clientToPage({ x: 400, y: 75 }, box, image, frame), { x: 800, y: 0 });
  assert.deepEqual(clientToPage({ x: 0, y: 300 }, box, image, frame), { x: 0, y: 450 });
});

test('clientToPage subtracts the frame offset_top and rejects the bar above it', () => {
  const box = { left: 0, top: 0, width: 400, height: 440 };
  const image = { width: 400, height: 440 };
  const frame = { device_width: 400, device_height: 400, offset_top: 40 };
  assert.equal(clientToPage({ x: 10, y: 20 }, box, image, frame), null);
  assert.deepEqual(clientToPage({ x: 10, y: 60 }, box, image, frame), { x: 10, y: 20 });
});

test('pageToBox inverts clientToPage (ghost cursor placement)', () => {
  const box = { left: 0, top: 0, width: 800, height: 600 };
  const image = { width: 1600, height: 900 };
  const frame = { device_width: 800, device_height: 450 };
  const page = clientToPage({ x: 250, y: 200 }, box, image, frame)!;
  const back = pageToBox(page, box, image, frame)!;
  assert.ok(Math.abs(back.x - 250) < 0.02 && Math.abs(back.y - 200) < 0.02);
});

test('viewportFor clamps size and quantises the device scale factor', () => {
  assert.deepEqual(viewportFor({ width: 1023.6, height: 700.2 }, 2), { width: 1024, height: 700, device_scale_factor: 2 });
  assert.equal(viewportFor({ width: 800, height: 600 }, 1.1).device_scale_factor, 1);
  assert.equal(viewportFor({ width: 800, height: 600 }, 1.33).device_scale_factor, 1.25);
  assert.equal(viewportFor({ width: 800, height: 600 }, 5).device_scale_factor, 3);
  assert.equal(viewportFor({ width: 800, height: 600 }, NaN).device_scale_factor, 1);
  assert.deepEqual(viewportFor({ width: 50, height: 20 }, 1), { width: 240, height: 160, device_scale_factor: 1 });
});

test('viewportFor lowers the scale factor to stay inside the pixel budget', () => {
  const v = viewportFor({ width: 2400, height: 1600 }, 2);
  assert.ok(v.width * v.height * v.device_scale_factor ** 2 <= 3840 * 2400);
  assert.equal(v.device_scale_factor, 1.5);
  assert.equal(viewportFor({ width: 3840, height: 2400 }, 2).device_scale_factor, 1);
});

test('viewportChanged ignores 1px wobble but not a dpr change', () => {
  const a = { width: 800, height: 600, device_scale_factor: 2 };
  assert.equal(viewportChanged(null, a), true);
  assert.equal(viewportChanged(a, { ...a, width: 801 }), false);
  assert.equal(viewportChanged(a, { ...a, height: 603 }), true);
  assert.equal(viewportChanged(a, { ...a, device_scale_factor: 1 }), true);
});

// ── keys ────────────────────────────────────────────────────────────────

const key = (k: string, code: string, mods: Partial<KeyLike> = {}): KeyLike => ({
  key: k, code, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...mods,
});

test('app chords already handled by the global key map pass through', () => {
  // lib/keys.ts runs on window capture and preventDefaults ⌘K/⌘T/⌘W/…
  assert.equal(routeKey(key('k', 'KeyK', { metaKey: true, defaultPrevented: true }), true), 'app');
  assert.equal(routeKey(key('t', 'KeyT', { metaKey: true, defaultPrevented: true }), true), 'app');
  // …and system chords are never sent to the page.
  assert.equal(routeKey(key('q', 'KeyQ', { metaKey: true }), true), 'app');
  assert.equal(routeKey(key('`', 'Backquote', { metaKey: true }), true), 'app');
});

test('browser-convention chords are handled locally', () => {
  assert.equal(routeKey(key('l', 'KeyL', { metaKey: true }), true), 'url');
  assert.equal(routeKey(key('r', 'KeyR', { metaKey: true }), true), 'reload');
  assert.equal(routeKey(key('v', 'KeyV', { metaKey: true }), true), 'paste');
  // non-Mac viewer: Ctrl is the primary modifier
  assert.equal(routeKey(key('v', 'KeyV', { ctrlKey: true }), false), 'paste');
  // …but Ctrl+V on a Mac is not paste (emacs-style page down in text fields)
  assert.equal(routeKey(key('v', 'KeyV', { ctrlKey: true }), true), 'forward');
  // ⌘⇧R is the app's hard reload (defaultPrevented by the key map)
  assert.equal(routeKey(key('R', 'KeyR', { metaKey: true, shiftKey: true, defaultPrevented: true }), true), 'app');
});

test('Esc releases the keyboard; ⇧Esc is sent to the page as Escape', () => {
  assert.equal(routeKey(key('Escape', 'Escape'), true), 'release');
  assert.equal(routeKey(key('Escape', 'Escape', { shiftKey: true }), true), 'forward');
  const p = keyPayload(key('Escape', 'Escape', { shiftKey: true }), true, 'down');
  assert.equal(p.modifiers, 0, 'shift used for routing is stripped');
  assert.equal(p.key_code, 27);
});

test('IME composition keystrokes are ignored (text arrives via compositionend)', () => {
  assert.equal(routeKey(key('a', 'KeyA', { isComposing: true }), true), 'ignore');
  assert.equal(routeKey(key('Process', 'KeyA'), true), 'ignore');
  assert.equal(routeKey(key('Dead', 'Quote'), true), 'ignore');
  assert.equal(routeKey(key('Unidentified', ''), true), 'ignore');
});

test('plain typing, editing and copy chords are forwarded', () => {
  assert.equal(routeKey(key('a', 'KeyA'), true), 'forward');
  assert.equal(routeKey(key('c', 'KeyC', { metaKey: true }), true), 'forward');
  assert.equal(routeKey(key('ArrowLeft', 'ArrowLeft', { metaKey: true, shiftKey: true }), true), 'forward');
});

test('modifierMask uses CDP bits', () => {
  assert.equal(modifierMask({ altKey: true, ctrlKey: false, metaKey: false, shiftKey: false }), 1);
  assert.equal(modifierMask({ altKey: false, ctrlKey: true, metaKey: true, shiftKey: true }), 14);
});

test('virtualKeyCode covers letters, digits, punctuation and named keys', () => {
  assert.equal(virtualKeyCode({ key: 'a', code: 'KeyA' }), 65);
  assert.equal(virtualKeyCode({ key: '!', code: 'Digit1' }), 49);
  assert.equal(virtualKeyCode({ key: 'Enter', code: 'Enter' }), 13);
  assert.equal(virtualKeyCode({ key: 'ArrowDown', code: 'ArrowDown' }), 40);
  assert.equal(virtualKeyCode({ key: ';', code: 'Semicolon' }), 186);
  assert.equal(virtualKeyCode({ key: 'F5', code: 'F5' }), 116);
});

test('keyPayload: text on keydown only, Enter types \\r, chords type nothing', () => {
  assert.equal(keyPayload(key('a', 'KeyA'), true, 'down').text, 'a');
  assert.equal(keyPayload(key('a', 'KeyA'), true, 'up').text, undefined);
  assert.equal(keyPayload(key('Enter', 'Enter'), true, 'down').text, '\r');
  assert.equal(keyPayload(key('a', 'KeyA', { metaKey: true }), true, 'down').text, undefined);
  assert.equal(keyPayload(key('ArrowLeft', 'ArrowLeft'), true, 'down').text, undefined);
});

test('editingCommands names macOS commands for chords, mapped from Ctrl off-Mac', () => {
  assert.deepEqual(editingCommands(key('a', 'KeyA', { metaKey: true }), true), ['selectAll']);
  assert.deepEqual(editingCommands(key('a', 'KeyA', { ctrlKey: true }), false), ['selectAll']);
  assert.deepEqual(editingCommands(key('a', 'KeyA', { ctrlKey: true }), true), []);
  assert.deepEqual(editingCommands(key('Z', 'KeyZ', { metaKey: true, shiftKey: true }), true), ['redo']);
  assert.deepEqual(editingCommands(key('ArrowLeft', 'ArrowLeft', { metaKey: true, shiftKey: true }), true), ['moveToBeginningOfLineAndModifySelection']);
  assert.deepEqual(editingCommands(key('Backspace', 'Backspace', { altKey: true }), true), ['deleteWordBackward']);
  assert.deepEqual(keyPayload(key('c', 'KeyC', { metaKey: true }), true, 'down').commands, ['copy']);
  assert.equal(keyPayload(key('c', 'KeyC', { metaKey: true }), true, 'up').commands, undefined);
});

// ── throttling ──────────────────────────────────────────────────────────

function fakeClock() {
  let t = 0;
  const timers: { at: number; fn: () => void; id: number }[] = [];
  let nextId = 1;
  const clock: Clock = {
    now: () => t,
    setTimeout: (fn, ms) => {
      const id = nextId++;
      timers.push({ at: t + ms, fn, id });
      return id;
    },
    clearTimeout: (h) => {
      const i = timers.findIndex((x) => x.id === h);
      if (i >= 0) timers.splice(i, 1);
    },
  };
  const advance = (ms: number) => {
    t += ms;
    for (;;) {
      timers.sort((a, b) => a.at - b.at);
      const due = timers[0];
      if (!due || due.at > t) break;
      timers.shift();
      due.fn();
    }
  };
  return { clock, advance, timers };
}

test('coalesce sends the first move at once and the LATEST one per interval', () => {
  const { clock, advance } = fakeClock();
  const sent: number[] = [];
  const c = coalesce<number>((v) => sent.push(v), 33, clock);
  c.push(1);
  assert.deepEqual(sent, [1]);
  c.push(2); c.push(3); c.push(4);
  assert.deepEqual(sent, [1], 'inside the interval nothing more goes out');
  advance(33);
  assert.deepEqual(sent, [1, 4], 'trailing send carries the latest value');
  advance(100);
  assert.deepEqual(sent, [1, 4], 'no duplicate');
});

test('coalesce.flush sends the pending move before a click; cancel drops it', () => {
  const { clock, advance } = fakeClock();
  const sent: number[] = [];
  const c = coalesce<number>((v) => sent.push(v), 33, clock);
  c.push(1);
  c.push(2);
  c.flush();
  assert.deepEqual(sent, [1, 2]);
  advance(50);
  assert.deepEqual(sent, [1, 2], 'the cancelled timer does not re-send');
  advance(10);
  c.push(3); // interval elapsed → immediate
  c.push(4);
  c.cancel();
  advance(100);
  assert.deepEqual(sent, [1, 2, 3]);
  assert.equal(c.pending, false);
});

test('mergeWheel sums deltas at the latest position; modifier change restarts', () => {
  let w = mergeWheel(null, { x: 1, y: 1, dx: 0, dy: 10, modifiers: 0 });
  w = mergeWheel(w, { x: 2, y: 3, dx: 1, dy: 5, modifiers: 0 });
  assert.deepEqual(w, { x: 2, y: 3, dx: 1, dy: 15, modifiers: 0 });
  w = mergeWheel(w, { x: 2, y: 3, dx: 0, dy: 7, modifiers: 2 });
  assert.deepEqual(w, { x: 2, y: 3, dx: 0, dy: 7, modifiers: 2 });
});

test('wheelPixels normalises line and page deltas', () => {
  assert.equal(wheelPixels(3, 0, 800), 3);
  assert.equal(wheelPixels(3, 1, 800), 48);
  assert.equal(wheelPixels(1, 2, 600), 600);
});

// ── connection reducer / backoff ────────────────────────────────────────

test('backoff grows exponentially with a cap and bounded jitter', () => {
  assert.equal(backoffDelay(1, 0.5), 500);
  assert.equal(backoffDelay(2, 0.5), 1000);
  assert.equal(backoffDelay(3, 0.5), 2000);
  assert.equal(backoffDelay(20, 0.5), BACKOFF.maxMs);
  assert.equal(backoffDelay(1, 0), 400);
  assert.ok(backoffDelay(1, 0.999) <= 600);
});

test('reducer walks connect → live → reconnecting → connecting → live', () => {
  let s: ConnState = reduce(INITIAL, { type: 'connect' });
  assert.equal(s.status, 'connecting');
  s = reduce(s, { type: 'open' });
  assert.equal(s.status, 'live');
  s = reduce(s, { type: 'frame', at: 1000 });
  assert.equal(s.hasFrame, true);
  s = reduce(s, { type: 'close', code: 1006 });
  assert.equal(s.status, 'reconnecting');
  assert.equal(s.attempt, 1);
  assert.equal(s.retryInMs, 500);
  s = reduce(s, { type: 'retry' });
  assert.equal(s.status, 'connecting');
  s = reduce(s, { type: 'close', code: 1006 });
  assert.equal(s.attempt, 2);
  assert.equal(s.retryInMs, 1000);
  s = reduce(reduce(s, { type: 'retry' }), { type: 'open' });
  assert.equal(s.status, 'live');
  assert.equal(s.attempt, 0, 'a successful open resets the backoff');
});

test('terminal close codes and ended messages end the session without retry', () => {
  assert.equal(isTerminalClose(1000), true);
  assert.equal(isTerminalClose(4404), true);
  assert.equal(isTerminalClose(1006), false);
  assert.equal(isTerminalClose(1012), false);
  assert.equal(isTerminalClose(4500), false);
  const live = reduce(reduce(INITIAL, { type: 'connect' }), { type: 'open' });
  const s = reduce(live, { type: 'close', code: 4404, reason: 'Tab closed' });
  assert.equal(s.status, 'ended');
  assert.equal(s.reason, 'Tab closed');
  assert.equal(reduce(s, { type: 'retry' }).status, 'ended');
  assert.equal(reduce(live, { type: 'ended', reason: 'Engine stopped' }).status, 'ended');
});

test('the reducer gives up after maxAttempts', () => {
  let s = reduce(INITIAL, { type: 'connect' });
  for (let i = 0; i < BACKOFF.maxAttempts; i++) {
    s = reduce(s, { type: 'close', code: 1006 });
    assert.equal(s.status, 'reconnecting');
    s = reduce(s, { type: 'retry' });
  }
  s = reduce(s, { type: 'close', code: 1006 });
  assert.equal(s.status, 'ended');
});

test('duplicate connect / stray open / frame-while-not-live are no-ops', () => {
  const c = reduce(INITIAL, { type: 'connect' });
  assert.equal(reduce(c, { type: 'connect' }), c);
  assert.equal(reduce(INITIAL, { type: 'open' }), INITIAL);
  assert.equal(reduce(c, { type: 'frame', at: 5 }), c);
});

test('isStale: a quiet page is not stale; a silent pipe and any reconnect are', () => {
  let s = reduce(reduce(INITIAL, { type: 'connect' }), { type: 'open' });
  assert.equal(isStale(s, 10_000), false, 'nothing seen yet: nothing to dim');
  s = reduce(s, { type: 'frame', at: 10_000 });
  assert.equal(isStale(s, 10_000 + STALE_MS - 1), false);
  // no new frames (static page) but the ping keeps answering → still fresh
  s = reduce(s, { type: 'heartbeat', at: 10_000 + STALE_MS });
  assert.equal(isStale(s, 10_000 + STALE_MS + 1), false);
  assert.equal(s.hasFrame, true);
  assert.equal(isStale(s, 10_000 + 2 * STALE_MS + 1), true, 'neither frames nor pongs → wedged');
  s = reduce(s, { type: 'close', code: 1006 });
  assert.equal(isStale(s, 10_001), true);
});

test('meter reports frames in the last second and smooths the rtt', () => {
  let m = EMPTY_METER;
  for (let t = 0; t <= 1000; t += 100) m = meterFrame(m, t);
  assert.equal(meterFps(m, 1000), 10);
  assert.equal(meterFps(m, 2500), 0);
  m = meterRtt(m, 100);
  assert.equal(m.rttMs, 100);
  m = meterRtt(m, 200);
  assert.equal(m.rttMs, 130);
});

// ── protocol helpers ────────────────────────────────────────────────────

test('parseServer ignores junk; buttonName maps DOM buttons', () => {
  assert.equal(parseServer('nope'), null);
  assert.equal(parseServer('{"no":"type"}'), null);
  assert.deepEqual(parseServer('{"type":"pong","t":1}'), { type: 'pong', t: 1 });
  assert.deepEqual([0, 1, 2, 3].map(buttonName), ['left', 'middle', 'right', 'none']);
});

test('safeCursor only lets known CSS cursor keywords through', () => {
  assert.equal(safeCursor('pointer'), 'pointer');
  assert.equal(safeCursor('url(https://evil/x.png), auto'), 'default');
  assert.equal(safeCursor(undefined), 'default');
});

test('parseBinaryFrame splits the JSON header from the image bytes', () => {
  const head = new TextEncoder().encode(JSON.stringify({ seq: 7, meta: { device_width: 10, device_height: 5 }, mime: 'image/jpeg' }));
  const buf = new ArrayBuffer(4 + head.length + 3);
  new DataView(buf).setUint32(0, head.length);
  new Uint8Array(buf, 4).set(head);
  new Uint8Array(buf, 4 + head.length).set([0xff, 0xd8, 0xff]);
  const f = parseBinaryFrame(buf)!;
  assert.equal(f.seq, 7);
  assert.equal(f.meta.device_width, 10);
  assert.deepEqual([...f.bytes], [0xff, 0xd8, 0xff]);
  assert.equal(parseBinaryFrame(new ArrayBuffer(2)), null);
  const bad = new ArrayBuffer(8);
  new DataView(bad).setUint32(0, 100);
  assert.equal(parseBinaryFrame(bad), null);
});
