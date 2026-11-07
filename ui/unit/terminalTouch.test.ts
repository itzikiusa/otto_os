/// <reference lib="dom" />
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { TerminalTouch } from '../src/lib/components/termTouch.ts';

function setup(coast = false) {
  const moves: number[][] = [];
  let taps = 0;
  let frame: FrameRequestCallback | null = null;
  const touch = new TerminalTouch({
    scroll: (dx, dy) => moves.push([dx, dy]), tap: () => taps++, canCoast: () => coast,
    frame: cb => { frame = cb; return 1; }, cancelFrame: () => { frame = null; },
  });
  const point = (x: number, y: number, timeStamp = 0, pointerId = 1) => ({ clientX: x, clientY: y, timeStamp, pointerId });
  return { touch, moves, point, taps: () => taps, frame: () => frame };
}

test('a small completed tap focuses once, never while the finger is down', () => {
  const s = setup();
  s.touch.start(s.point(30, 30));
  assert.equal(s.taps(), 0);
  s.touch.move(s.point(32, 33, 10));
  assert.deepEqual(s.moves, []);
  s.touch.end(s.point(32, 33, 20));
  assert.equal(s.taps(), 1);
});

test('a downward swipe reads older rows; horizontal travel is preserved without focusing', () => {
  const s = setup();
  s.touch.start(s.point(50, 50));
  s.touch.move(s.point(40, 80, 20));
  s.touch.move(s.point(30, 100, 40));
  s.touch.end(s.point(30, 100, 50));
  assert.deepEqual(s.moves, [[10, -30], [10, -20]]);
  assert.equal(s.taps(), 0);
  assert.equal(s.frame(), null, 'TUI/reduced-motion gestures never coast');
});

test('another pointer and a cancelled gesture never type or focus', () => {
  const s = setup();
  s.touch.start(s.point(20, 20));
  s.touch.move(s.point(20, 100, 10, 2));
  s.touch.end(s.point(20, 100, 20, 2));
  assert.deepEqual(s.moves, []);
  s.touch.cancel();
  s.touch.end(s.point(20, 20, 30));
  assert.equal(s.taps(), 0);
});

test('scrollback coasts briefly; touching again cancels the old momentum', () => {
  const s = setup(true);
  s.touch.start(s.point(20, 20));
  s.touch.move(s.point(20, 60, 20));
  s.touch.end(s.point(20, 60, 25));
  s.frame()!(30);
  assert(s.moves[1][1] < 0);
  s.touch.start(s.point(30, 30, 40));
  assert.equal(s.frame(), null);
  s.touch.cancel();
});

test('holding the finger after scrolling does not launch momentum on release', () => {
  const s = setup(true);
  s.touch.start(s.point(20, 20));
  s.touch.move(s.point(20, 60, 20));
  s.touch.end(s.point(20, 60, 500));
  assert.equal(s.frame(), null);
});

// DOM event coordinates are inherited getters, not enumerable own fields.
test('pointer event prototype properties are captured and updated', () => {
  const s = setup();
  s.touch.start(Object.create(s.point(20, 20)));
  s.touch.move(Object.create(s.point(20, 50, 20)));
  s.touch.move(Object.create(s.point(20, 80, 40)));
  assert.deepEqual(s.moves, [[0, -30], [0, -30]]);
  s.touch.end(Object.create(s.point(20, 80, 45)));
  assert.equal(s.taps(), 0);
});

test('terminal adapter uses actual row height and pans the attached grid', async () => {
  const { loadSource } = await import('./sourceHarness.ts');
  const { createTerminalTouch } = loadSource(new URL('../src/lib/components/termTouch.ts', import.meta.url), {}, {
    window: { matchMedia: () => ({ matches: true }) },
  });
  const lines: number[] = [];
  const host = { scrollLeft: 0 };
  const term = { rows: 10, buffer: { active: { type: 'normal' } }, modes: { mouseTrackingMode: 'none' },
    element: { parentElement: host, querySelector: () => ({ getBoundingClientRect: () => ({ height: 300 }) }) },
    scrollLines: (n: number) => lines.push(n), focus: () => assert.fail('drag focused keyboard'),
  };
  const touch = createTerminalTouch(() => term, () => false);
  touch.start({ pointerId: 1, clientX: 100, clientY: 100, timeStamp: 0 });
  touch.move({ pointerId: 1, clientX: 80, clientY: 160, timeStamp: 20 });
  touch.end({ pointerId: 1, clientX: 80, clientY: 160, timeStamp: 30 });
  assert.deepEqual(lines, [-2]);
  assert.equal(host.scrollLeft, 20);
});

test('full-screen application gets xterm wheel input, but read-only viewers never send it', async () => {
  const { loadSource } = await import('./sourceHarness.ts');
  const wheels: unknown[] = [];
  const { createTerminalTouch } = loadSource(new URL('../src/lib/components/termTouch.ts', import.meta.url), {}, {
    WheelEvent: class { type: string; init: unknown; constructor(type: string, init: unknown) { this.type = type; this.init = init; } },
  });
  const term = { rows: 10, buffer: { active: { type: 'alternate' } }, modes: { mouseTrackingMode: 'none' },
    element: { dispatchEvent: (event: unknown) => wheels.push(event) },
    scrollLines: () => assert.fail('alternate buffer has no scrollback'), focus: () => assert.fail('drag focused keyboard'),
  };
  for (const readOnly of [false, true]) {
    const touch = createTerminalTouch(() => term, () => readOnly);
    touch.start({ pointerId: 1, clientX: 100, clientY: 100, timeStamp: 0 });
    touch.move({ pointerId: 1, clientX: 100, clientY: 160, timeStamp: 20 });
    touch.end({ pointerId: 1, clientX: 100, clientY: 160, timeStamp: 30 });
  }
  assert.equal(wheels.length, 1);
  assert.match(JSON.stringify(wheels[0]), /"deltaY":-60/);
});
